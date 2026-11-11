//! Subidas a Drive de partidas y clips, con progreso hacia la interfaz.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use anyhow::{Result, anyhow, bail};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::engine::Shared;
use crate::gdrive::Drive;
use crate::library::{DriveRef, MatchMeta};
use crate::share::{ShareLink, qr_svg};

static ACTIVE: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

#[derive(Serialize, Clone)]
pub struct UploadProgress {
    pub key: String,
    pub sent: u64,
    pub total: u64,
    pub done: bool,
    pub error: Option<String>,
    /// true = bajando de Drive a la PC.
    pub download: bool,
    /// En pausa porque estás jugando.
    #[serde(default)]
    pub waiting: bool,
    /// Comprimiendo antes de subir (`sent`/`total` = segundos procesados / duración).
    #[serde(default)]
    pub compressing: bool,
}

fn key(match_id: &str, clip_id: Option<&str>) -> String {
    format!("{match_id}/{}", clip_id.unwrap_or("match"))
}

async fn drive_for(shared: &Shared) -> Result<(Drive, String)> {
    let d = shared.settings.read().unwrap().drive.clone();
    let refresh = d.refresh_token.clone().ok_or_else(|| anyhow!("Conecta tu cuenta de Google Drive en Ajustes"))?;
    let drive = Drive::connect(&d.client_id, &d.client_secret, &refresh).await?;
    let folder = drive.ensure_folder(d.folder_id.as_deref(), "ggswp").await?;
    if d.folder_id.as_deref() != Some(folder.as_str()) {
        let mut s = shared.settings.write().unwrap();
        s.drive.folder_id = Some(folder.clone());
        let _ = s.save(&shared.settings_path);
    }
    Ok((drive, folder))
}

pub async fn upload(app: &AppHandle, shared: &Shared, match_id: &str, clip_id: Option<&str>) -> Result<DriveRef> {
    let k = key(match_id, clip_id);
    if !ACTIVE.lock().unwrap().insert(k.clone()) {
        bail!("Ya se está subiendo");
    }
    let r = upload_inner(app, shared, match_id, clip_id, &k).await;
    ACTIVE.lock().unwrap().remove(&k);
    let _ = app.emit("upload", UploadProgress { key: k, sent: 0, total: 0, done: true, error: r.as_ref().err().map(|e| format!("{e:#}")), download: false, waiting: false, compressing: false });
    r
}

async fn upload_inner(app: &AppHandle, shared: &Shared, match_id: &str, clip_id: Option<&str>, k: &str) -> Result<DriveRef> {
    let lib = shared.library();
    let meta = lib.load(match_id)?;
    let (file, name, existing) = match clip_id {
        Some(cid) => {
            let c = meta.clips.iter().find(|c| c.id == cid).ok_or_else(|| anyhow!("clip no encontrado"))?;
            (c.file.clone(), meta.drive_name(Some(c)), c.drive.clone())
        }
        None => (meta.video.clone().ok_or_else(|| anyhow!("la partida no tiene video"))?, meta.drive_name(None), meta.drive.clone()),
    };
    if let Some(d) = existing {
        return Ok(d);
    }
    let path = lib.file(match_id, &file);
    let total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    // No se sube mientras juegas: la subida espera (o se pausa) hasta que vuelvas al lobby.
    let paused = || {
        let p = shared.is_playing();
        if p {
            let _ = app.emit("upload", UploadProgress { key: k.to_string(), sent: 0, total, done: false, error: None, download: false, waiting: true, compressing: false });
        }
        p
    };
    while paused() {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    }
    // Copia comprimida solo para Drive (la de la PC queda intacta).
    let duration = match clip_id {
        Some(cid) => meta.clips.iter().find(|c| c.id == cid).map(|c| c.end - c.start).unwrap_or(0.0),
        None => meta.duration_s,
    };
    let compressed = compress_for_drive(app, shared, &path, duration, k).await;
    let upload_path = compressed.clone().unwrap_or_else(|| path.clone());
    let result = upload_file(app, shared, &upload_path, &name, k, &paused).await;
    if let Some(tmp) = &compressed {
        let _ = std::fs::remove_file(tmp);
    }
    let (drive, file_id, uploaded_size) = result?;
    let r = DriveRef { link: format!("https://drive.google.com/file/d/{file_id}/view"), file_id, size: Some(uploaded_size) };
    let verified = uploaded_size > 0 && drive.file_size(&r.file_id).await.ok().flatten() == Some(uploaded_size);
    if compressed.is_some() {
        log::info!("{name}: subida comprimida ({:.0} MB → {:.0} MB)", total as f64 / 1e6, uploaded_size as f64 / 1e6);
    }
    if meta.keep {
        let _ = drive.patch(&r.file_id, serde_json::json!({ "starred": true })).await;
    }
    let mut meta = lib.load(match_id)?;
    match clip_id {
        Some(cid) => {
            if let Some(c) = meta.clips.iter_mut().find(|c| c.id == cid) {
                c.drive = Some(r.clone());
            }
        }
        None => meta.drive = Some(r.clone()),
    }
    let delete_local = shared.settings.read().unwrap().drive.delete_local_after_upload;
    if delete_local && verified {
        offload(&lib, &mut meta, &file);
    } else if delete_local {
        log::warn!("{name}: el tamaño en Drive no coincide; se conserva la copia local");
    }
    lib.save(&meta)?;
    let _ = app.emit("library", ());
    Ok(r)
}

/// Sube un archivo a la carpeta de ggswp y devuelve (cliente, id, bytes subidos).
async fn upload_file(app: &AppHandle, shared: &Shared, path: &std::path::Path, name: &str, k: &str, paused: &(dyn Fn() -> bool + Sync)) -> Result<(Drive, String, u64)> {
    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let (drive, folder) = drive_for(shared).await?;
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
    let file_id = drive
        .upload(path, name, &folder, |sent, total| {
            if last_emit.elapsed().as_millis() > 300 || sent == total {
                last_emit = std::time::Instant::now();
                let _ = app.emit("upload", UploadProgress { key: k.to_string(), sent, total, done: false, error: None, download: false, waiting: false, compressing: false });
            }
        }, paused)
        .await?;
    Ok((drive, file_id, size))
}

/// Recodifica con NVENC una copia más liviana para Drive (≈ 4 veces menos que la grabación,
/// casi igual a la vista). Devuelve `None` si está desactivado o si algo falla: en ese caso
/// se sube el original.
async fn compress_for_drive(app: &AppHandle, shared: &Shared, src: &std::path::Path, duration: f64, k: &str) -> Option<std::path::PathBuf> {
    let settings = shared.settings.read().unwrap().clone();
    let video = match settings.drive.codec.as_str() {
        "hevc" => vec!["-c:v", "hevc_nvenc", "-cq", "28", "-tag:v", "hvc1"],
        "av1" => vec!["-c:v", "av1_nvenc", "-cq", "34"],
        "h264" => vec!["-c:v", "h264_nvenc", "-cq", "26"],
        _ => return None,
    };
    let ffmpeg = crate::engine::ffmpeg_path(app, &settings);
    let dst = src.with_extension("drive.mp4");
    let original = std::fs::metadata(src).map(|m| m.len()).unwrap_or(0);
    let (app2, k2, src2, dst2) = (app.clone(), k.to_string(), src.to_path_buf(), dst.clone());
    let total = (duration * 1000.0).max(1.0) as u64;
    let ok = tauri::async_runtime::spawn_blocking(move || -> Result<()> {
        use std::io::BufRead;
        use std::os::windows::process::CommandExt;
        let mut child = std::process::Command::new(&ffmpeg)
            .args(["-hide_banner", "-v", "error", "-y", "-hwaccel", "auto", "-i"])
            .arg(&src2)
            .args(["-map", "0"])
            .args(&video)
            .args(["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-b:v", "0", "-maxrate", "25M", "-bufsize", "50M", "-c:a", "copy", "-movflags", "+faststart", "-progress", "pipe:1", "-nostats"])
            .arg(&dst2)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .creation_flags(crate::capture::video::CREATE_NO_WINDOW | 0x4000)
            .spawn()?;
        let mut last = 0u64;
        for line in std::io::BufReader::new(child.stdout.take().unwrap()).lines().map_while(Result::ok) {
            if let Some(ms) = line.strip_prefix("out_time_ms=").and_then(|v| v.parse::<u64>().ok()) {
                let done = (ms / 1000).min(total);
                if done >= last + total / 100 {
                    last = done;
                    let _ = app2.emit("upload", UploadProgress { key: k2.clone(), sent: done, total, done: false, error: None, download: false, waiting: false, compressing: true });
                }
            }
        }
        if !child.wait()?.success() {
            bail!("ffmpeg falló");
        }
        Ok(())
    })
    .await;
    let size = std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
    match ok {
        Ok(Ok(())) if size > 0 && size < original => Some(dst),
        other => {
            if let Ok(Err(e)) = other {
                log::warn!("{}: no se pudo comprimir para Drive, se sube el original: {e:#}", src.display());
            }
            let _ = std::fs::remove_file(&dst);
            None
        }
    }
}

/// Borra el archivo local si todo lo que lo usa (la partida y/o sus clips) ya está en Drive.
/// Un clip guardado con el atajo fuera de partida comparte archivo con su "partida".
fn offload(lib: &crate::library::Library, meta: &mut MatchMeta, file: &str) {
    let match_uses = meta.video.as_deref() == Some(file);
    let all_in_drive = (!match_uses || meta.drive.is_some()) && meta.clips.iter().filter(|c| c.file == file).all(|c| c.drive.is_some());
    if !all_in_drive {
        return;
    }
    if let Err(e) = std::fs::remove_file(lib.file(&meta.id, file)) {
        log::warn!("no se pudo borrar la copia local {file}: {e}");
        return;
    }
    if match_uses {
        meta.offloaded = true;
    }
    for c in meta.clips.iter_mut().filter(|c| c.file == file) {
        c.offloaded = true;
    }
    log::info!("{}: {file} ahora solo está en Drive", meta.id);
}

/// Borra de la PC los archivos de la partida que ya están en Drive, después de comprobar
/// que la copia de Drive sigue ahí y tiene el mismo tamaño. Devuelve cuántos se borraron.
pub async fn offload_verified(shared: &Shared, match_id: &str) -> Result<usize> {
    let lib = shared.library();
    let meta = lib.load(match_id)?;
    let mut candidates: Vec<(String, DriveRef)> = Vec::new();
    if let (Some(v), Some(d)) = (&meta.video, &meta.drive)
        && !meta.offloaded
    {
        candidates.push((v.clone(), d.clone()));
    }
    for c in meta.clips.iter().filter(|c| !c.offloaded) {
        if let Some(d) = &c.drive {
            candidates.push((c.file.clone(), d.clone()));
        }
    }
    candidates.retain(|(file, _)| lib.file(match_id, file).exists());
    if candidates.is_empty() || !drive_connected(shared) {
        return Ok(0);
    }
    let (drive, _) = drive_for(shared).await?;
    let mut verified = Vec::new();
    for (file, d) in candidates {
        // Si se subió una copia comprimida, se compara con el tamaño que se subió.
        let expected = d.size.unwrap_or_else(|| std::fs::metadata(lib.file(match_id, &file)).map(|m| m.len()).unwrap_or(0));
        match drive.file_size(&d.file_id).await? {
            Some(n) if n == expected && expected > 0 => verified.push(file),
            other => log::warn!("{match_id}/{file}: en Drive no coincide ({other:?} vs {expected} bytes); se conserva en la PC"),
        }
    }
    let mut meta = lib.load(match_id)?;
    let mut n = 0;
    for file in &verified {
        let before = lib.file(match_id, file).exists();
        offload(&lib, &mut meta, file);
        if before && !lib.file(match_id, file).exists() {
            n += 1;
        }
    }
    if n > 0 {
        lib.save(&meta)?;
    }
    Ok(n)
}

/// Vuelve a bajar a la PC un archivo que solo estaba en Drive.
pub async fn download(app: &AppHandle, shared: &Shared, match_id: &str, clip_id: Option<&str>) -> Result<()> {
    let k = key(match_id, clip_id);
    if !ACTIVE.lock().unwrap().insert(k.clone()) {
        bail!("Ya se está transfiriendo");
    }
    let r = download_inner(app, shared, match_id, clip_id, &k).await;
    ACTIVE.lock().unwrap().remove(&k);
    let _ = app.emit("upload", UploadProgress { key: k, sent: 0, total: 0, done: true, error: r.as_ref().err().map(|e| format!("{e:#}")), download: true, waiting: false, compressing: false });
    r
}

async fn download_inner(app: &AppHandle, shared: &Shared, match_id: &str, clip_id: Option<&str>, k: &str) -> Result<()> {
    let lib = shared.library();
    let meta = lib.load(match_id)?;
    let (file, drive_ref) = match clip_id {
        Some(cid) => {
            let c = meta.clips.iter().find(|c| c.id == cid).ok_or_else(|| anyhow!("clip no encontrado"))?;
            (c.file.clone(), c.drive.clone())
        }
        None => (meta.video.clone().unwrap_or_else(|| "match.mp4".into()), meta.drive.clone()),
    };
    let drive_ref = drive_ref.ok_or_else(|| anyhow!("este archivo no está en Drive"))?;
    let path = lib.file(match_id, &file);
    if !path.exists() {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let (drive, _) = drive_for(shared).await?;
        let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
        drive
            .download(&drive_ref.file_id, &path, |sent, total| {
                if last_emit.elapsed().as_millis() > 300 || sent == total {
                    last_emit = std::time::Instant::now();
                    let _ = app.emit("upload", UploadProgress { key: k.to_string(), sent, total, done: false, error: None, download: true, waiting: false, compressing: false });
                }
            })
            .await?;
    }
    let mut meta = lib.load(match_id)?;
    if meta.video.as_deref() == Some(file.as_str()) {
        meta.offloaded = false;
        meta.size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(meta.size_bytes);
    }
    for c in meta.clips.iter_mut().filter(|c| c.file == file) {
        c.offloaded = false;
    }
    lib.save(&meta)?;
    let _ = app.emit("library", ());
    Ok(())
}

fn drive_connected(shared: &Shared) -> bool {
    shared.settings.read().unwrap().drive.refresh_token.is_some()
}

/// Lleva a Drive el nombre y el estado "conservada" (estrella) de la partida y sus clips.
pub async fn sync_meta(shared: &Shared, meta: &MatchMeta) -> Result<()> {
    if meta.drive_files().is_empty() || !drive_connected(shared) {
        return Ok(());
    }
    let (drive, _) = drive_for(shared).await?;
    if let Some(d) = &meta.drive {
        drive.patch(&d.file_id, serde_json::json!({ "name": meta.drive_name(None), "starred": meta.keep })).await?;
    }
    for c in &meta.clips {
        if let Some(d) = &c.drive {
            drive.patch(&d.file_id, serde_json::json!({ "name": meta.drive_name(Some(c)), "starred": meta.keep })).await?;
        }
    }
    Ok(())
}

/// Manda a la papelera de Drive (recuperable durante 30 días) los archivos indicados.
pub async fn trash(shared: &Shared, files: &[&DriveRef]) -> Result<()> {
    if files.is_empty() {
        return Ok(());
    }
    if !drive_connected(shared) {
        // Con la cuenta desconectada no hay forma de tocarlos: quedan en Drive.
        log::info!("Drive desconectado: {} archivo(s) quedan en Drive", files.len());
        return Ok(());
    }
    let (drive, _) = drive_for(shared).await?;
    for d in files {
        drive.patch(&d.file_id, serde_json::json!({ "trashed": true })).await?;
    }
    Ok(())
}

/// Sube (si hace falta), activa "cualquiera con el enlace" y devuelve link + QR.
pub async fn drive_link(app: &AppHandle, shared: &Shared, match_id: &str, clip_id: Option<&str>) -> Result<ShareLink> {
    let r = upload(app, shared, match_id, clip_id).await?;
    let (drive, _) = drive_for(shared).await?;
    let url = drive.make_public_link(&r.file_id).await?;
    Ok(ShareLink { qr_svg: qr_svg(&url)?, url, addresses: Vec::new() })
}

pub fn maybe_auto_upload(app: &AppHandle, shared: &std::sync::Arc<Shared>, match_id: &str) {
    let d = shared.settings.read().unwrap().drive.clone();
    if !d.auto_upload || d.refresh_token.is_none() {
        return;
    }
    let (app, shared, id) = (app.clone(), shared.clone(), match_id.to_string());
    tauri::async_runtime::spawn(async move {
        let Ok(meta) = shared.library().load(&id) else { return };
        if d.upload_clips {
            for c in &meta.clips {
                if let Err(e) = upload(&app, &shared, &id, Some(&c.id)).await {
                    log::warn!("subida automática del clip {}: {e:#}", c.id);
                }
            }
        }
        if d.upload_full_match
            && let Err(e) = upload(&app, &shared, &id, None).await
        {
            log::warn!("subida automática de {id}: {e:#}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{Clip, Library};

    fn clip(id: &str, file: &str, drive: bool) -> Clip {
        Clip {
            id: id.into(),
            file: file.into(),
            thumb: None,
            start: 0.0,
            end: 1.0,
            title: id.into(),
            kind: "hotkey".into(),
            created_at: chrono::Utc::now(),
            drive: drive.then(|| DriveRef { file_id: id.into(), link: String::new(), size: None }),
            offloaded: false,
        }
    }

    #[test]
    fn offload_only_when_every_user_of_the_file_is_in_drive() {
        let root = std::env::temp_dir().join(format!("ggswp-offload-{}", std::process::id()));
        let lib = Library::new(root.clone());
        std::fs::create_dir_all(lib.file("m", "clips")).unwrap();
        for f in ["match.mp4", "clips/a.mp4"] {
            std::fs::write(lib.file("m", f), b"x").unwrap();
        }
        // Clip del atajo que comparte archivo con la partida: la partida aún no está en Drive.
        let mut meta = MatchMeta { id: "m".into(), video: Some("match.mp4".into()), clips: vec![clip("hk", "match.mp4", true), clip("a", "clips/a.mp4", true)], ..Default::default() };
        offload(&lib, &mut meta, "match.mp4");
        assert!(lib.file("m", "match.mp4").exists(), "la partida no está en Drive: no se borra");
        assert!(!meta.offloaded && !meta.clips[0].offloaded);

        offload(&lib, &mut meta, "clips/a.mp4");
        assert!(!lib.file("m", "clips/a.mp4").exists());
        assert!(meta.clips[1].offloaded && !meta.offloaded);

        meta.drive = Some(DriveRef { file_id: "m".into(), link: String::new(), size: None });
        offload(&lib, &mut meta, "match.mp4");
        assert!(!lib.file("m", "match.mp4").exists());
        assert!(meta.offloaded && meta.clips[0].offloaded);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn drive_names_are_safe_and_follow_the_title() {
        let meta = MatchMeta { title: "Victoria: 5/6 elims?".into(), ..Default::default() };
        let name = meta.drive_name(None);
        assert!(name.ends_with(" - Victoria- 5-6 elims-.mp4"), "{name}");
    }
}
