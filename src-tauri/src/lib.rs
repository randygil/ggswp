pub mod capture;
pub mod config;
pub mod engine;
pub mod export;
pub mod fortnite;
pub mod gdrive;
pub mod hotkeys;
pub mod library;
pub mod montage;
pub mod overlay;
pub mod retention;
pub mod share;
pub mod stats;
pub mod tray;
pub mod uploads;
pub mod winshare;

use std::os::windows::process::CommandExt;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt as _;

use capture::video::CREATE_NO_WINDOW;
use config::Settings;
use engine::{Cmd, Shared, Status};
use export::Quality;
use library::{Clip, DriveRef, MatchMeta};
use share::{Firewall, ShareLink, ShareServer};

pub struct AppState {
    pub shared: Arc<Shared>,
    share: Mutex<Option<ShareServer>>,
}

type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

#[derive(Serialize)]
struct MatchView {
    #[serde(flatten)]
    meta: MatchMeta,
    dir: String,
    /// Tamaño en disco de cada clip que está en la PC, por id.
    clip_sizes: std::collections::HashMap<String, u64>,
}

fn view(shared: &Shared, meta: MatchMeta) -> MatchView {
    let lib = shared.library();
    let dir = lib.dir(&meta.id).display().to_string();
    let clip_sizes = meta
        .clips
        .iter()
        .filter(|c| !c.offloaded)
        .filter_map(|c| Some((c.id.clone(), std::fs::metadata(lib.file(&meta.id, &c.file)).ok()?.len())))
        .collect();
    MatchView { meta, dir, clip_sizes }
}

#[tauri::command]
fn get_status(state: State<AppState>) -> Status {
    state.shared.status.lock().unwrap().clone()
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Settings {
    state.shared.settings.read().unwrap().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: State<AppState>, settings: Settings) -> CmdResult<Vec<String>> {
    let prev_port = state.shared.settings.read().unwrap().share_port;
    let language_changed = state.shared.settings.read().unwrap().language != settings.language;
    settings.save(&state.shared.settings_path).map_err(err)?;
    let autostart = app.autolaunch();
    let _ = if settings.launch_at_startup { autostart.enable() } else { autostart.disable() };
    if settings.share_port != prev_port {
        *state.share.lock().unwrap() = None;
    }
    let failed = hotkeys::apply(&app, &settings.hotkeys);
    *state.shared.settings.write().unwrap() = settings;
    let _ = app.emit("library", ());
    if language_changed {
        // Las ventanas se recargan con el idioma nuevo.
        tray::relabel(&app);
        let _ = app.emit("language", ());
    }
    Ok(failed)
}

#[tauri::command]
fn list_matches(state: State<AppState>) -> Vec<MatchView> {
    state.shared.library().list().into_iter().map(|m| view(&state.shared, m)).collect()
}

#[tauri::command]
fn get_totals(state: State<AppState>) -> stats::Totals {
    stats::totals(&stats::path_for(&state.shared.settings_path))
}

#[tauri::command]
fn get_match(state: State<AppState>, id: String) -> CmdResult<MatchView> {
    Ok(view(&state.shared, state.shared.library().load(&id).map_err(err)?))
}

/// Borra la partida de la PC y manda sus copias de Drive a la papelera.
#[tauri::command]
async fn delete_match(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    if state.shared.status.lock().unwrap().recording.as_ref().is_some_and(|r| r.match_id == id) {
        return Err("No se puede borrar mientras se graba".into());
    }
    let lib = state.shared.library();
    let m = lib.load(&id).map_err(err)?;
    uploads::trash(&state.shared, &m.drive_files()).await.map_err(|e| format!("No se pudo borrar de Drive, la partida sigue aquí: {e:#}"))?;
    lib.delete(&id).map_err(err)?;
    log::info!("{id} borrada por el usuario ({} archivo(s) a la papelera de Drive)", m.drive_files().len());
    let _ = app.emit("library", ());
    Ok(())
}

/// Renombra y/o conserva una partida; el cambio se refleja en Drive.
#[tauri::command]
async fn update_match(app: AppHandle, state: State<'_, AppState>, id: String, title: Option<String>, keep: Option<bool>) -> CmdResult<()> {
    let lib = state.shared.library();
    let mut m = lib.load(&id).map_err(err)?;
    if let Some(t) = title.filter(|t| !t.trim().is_empty()) {
        m.title = t.trim().to_string();
    }
    if let Some(k) = keep {
        m.keep = k;
    }
    lib.save(&m).map_err(err)?;
    let _ = app.emit("library", ());
    uploads::sync_meta(&state.shared, &m).await.map_err(|e| format!("Guardado en la PC, pero no se pudo actualizar en Drive: {e:#}"))
}

#[tauri::command]
async fn delete_clip(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: String) -> CmdResult<()> {
    let lib = state.shared.library();
    let mut m = lib.load(&id).map_err(err)?;
    if let Some(d) = m.clips.iter().find(|c| c.id == clip_id).and_then(|c| c.drive.as_ref()) {
        uploads::trash(&state.shared, &[d]).await.map_err(|e| format!("No se pudo borrar de Drive: {e:#}"))?;
    }
    if let Some(c) = m.clips.iter().find(|c| c.id == clip_id) {
        lib.remove_clip_files(&m, c);
    }
    m.clips.retain(|c| c.id != clip_id);
    lib.save(&m).map_err(err)?;
    let _ = app.emit("library", ());
    Ok(())
}

/// Exporta un tramo desde el editor. `source` es el clip que se está editando (sin él, la
/// partida completa); `start` y `end` son segundos dentro de ese video.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn export_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
    source: Option<String>,
    start: f64,
    end: f64,
    title: String,
    quality: Option<Quality>,
    frame: String,
    fx: Option<export::Fx>,
) -> CmdResult<Clip> {
    if end - start < 0.5 {
        return Err("El clip es demasiado corto".into());
    }
    let shared = state.shared.clone();
    let app2 = app.clone();
    let clip = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Clip> {
        let settings = shared.settings.read().unwrap().clone();
        let ffmpeg = engine::ffmpeg_path(&app2, &settings);
        let lib = shared.library();
        let m = lib.load(&id)?;
        let from = match &source {
            Some(cid) => Some(m.clips.iter().find(|c| &c.id == cid).ok_or_else(|| anyhow::anyhow!("clip no encontrado"))?.clone()),
            None => None,
        };
        if from.as_ref().map_or(m.offloaded, |c| c.offloaded) {
            anyhow::bail!("El video está solo en Drive: descárgalo para editarlo");
        }
        let file = from.as_ref().map(|c| c.file.clone()).or_else(|| m.video.clone()).unwrap_or_else(|| "match.mp4".into());
        // Un clip que ya es vertical se recorta tal cual.
        let from_vertical = from.as_ref().is_some_and(|c| c.kind == "vertical");
        let mobile = (frame == "mobile" && !from_vertical).then(|| settings.mobile_layout.clone());
        let is_mobile = mobile.is_some();
        let vertical = (frame == "vertical" || is_mobile) && !from_vertical;
        // Sin elección en el editor, las fuentes de audio son las de Ajustes.
        let mut fx = fx.unwrap_or_default();
        if fx.tracks.is_none() {
            fx.tracks = Some(export::default_tracks(&settings));
        }
        let cid = library::new_id("edit");
        std::fs::create_dir_all(lib.file(&id, "clips"))?;
        let out_file = format!("clips/{cid}.mp4");
        let out = lib.file(&id, &out_file);
        let job = export::Job {
            src: &lib.file(&id, &file),
            cut: Some((start, end - start)),
            quality: quality.unwrap_or_else(|| Quality::from_setting(&settings.clip_quality)),
            vertical,
            badge: montage::vertical_badge(m.kills, m.victory),
            mobile,
            fx,
        };
        shared.set_step(&app2, &id, "Exportando clip", Some(0.0));
        let result = export::render(&ffmpeg, &job, &engine::video_options(&settings, 0), &out, &|p| shared.set_step(&app2, &id, "Exportando clip", Some(p)));
        montage::clear_step(&app2, &shared, &id);
        if let Err(e) = result {
            let _ = std::fs::remove_file(&out);
            return Err(e);
        }
        let thumb = format!("clips/{cid}.jpg");
        let thumb_ok = library::thumbnail(&ffmpeg, &out, ((end - start) / 2.0).min(3.0), &lib.file(&id, &thumb)).is_ok();
        let title = match (title.trim(), &from) {
            ("", Some(c)) if vertical => format!("{} · {}", c.title, if is_mobile { "móvil" } else { "vertical" }),
            ("", Some(c)) => c.title.clone(),
            ("", None) => "Clip".to_string(),
            (t, _) => t.to_string(),
        };
        // Los tiempos de un clip son los de la partida. Un recorte de un resumen o de un
        // vertical no corresponde a un tramo de la partida: no va en la línea de tiempo.
        let on_timeline = from.as_ref().is_none_or(|c| !matches!(c.kind.as_str(), "reel" | "vertical" | "cut" | "join"));
        let offset = from.as_ref().filter(|_| on_timeline).map_or(0.0, |c| c.start);
        let (kind, clip_start) = match (vertical || from_vertical, on_timeline) {
            (true, _) => ("vertical", offset + start),
            (false, true) => ("manual", offset + start),
            (false, false) => ("cut", 0.0),
        };
        // Con cámara lenta el clip dura más que el tramo de donde salió.
        let length = library::probe(&ffmpeg, &out).map(|p| p.duration).unwrap_or(end - start);
        let clip = Clip {
            id: cid,
            file: out_file,
            thumb: thumb_ok.then_some(thumb),
            start: clip_start,
            end: clip_start + if on_timeline && !vertical { end - start } else { length },
            title,
            kind: kind.into(),
            created_at: chrono::Utc::now(),
            drive: None,
            offloaded: false,
            best: false,
        };
        // Se vuelve a leer: la partida pudo cambiar mientras se codificaba.
        let mut m = lib.load(&id)?;
        m.clips.push(clip.clone());
        lib.save(&m)?;
        Ok(clip)
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))?;
    let _ = app.emit("library", ());
    Ok(clip)
}

/// Une varios clips de una partida en uno, en el orden dado.
#[tauri::command]
async fn join_clips(app: AppHandle, state: State<'_, AppState>, id: String, clip_ids: Vec<String>, title: String) -> CmdResult<Clip> {
    let shared = state.shared.clone();
    let app2 = app.clone();
    let clip = tauri::async_runtime::spawn_blocking(move || montage::join_clips(&app2, &shared, &id, &clip_ids, &title)).await.map_err(err)?.map_err(|e| format!("{e:#}"))?;
    let _ = app.emit("library", ());
    Ok(clip)
}

/// Un video con lo mejor de los últimos 7 días; devuelve el id de la entrada nueva.
#[tauri::command]
async fn create_weekly(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let shared = state.shared.clone();
    let app2 = app.clone();
    let meta = tauri::async_runtime::spawn_blocking(move || montage::create_weekly(&app2, &shared)).await.map_err(err)?.map_err(|e| format!("{e:#}"))?;
    let _ = app.emit("library", ());
    Ok(meta.id)
}

/// Guarda las disposiciones del modo móvil que tienen nombre.
#[tauri::command]
fn set_mobile_presets(state: State<AppState>, presets: Vec<export::MobilePreset>) -> CmdResult<()> {
    let mut s = state.shared.settings.write().unwrap();
    s.mobile_presets = presets;
    s.save(&state.shared.settings_path).map_err(err)
}

/// Programas abiertos, para agregar uno como juego a grabar.
#[tauri::command]
fn list_open_apps() -> Vec<capture::win::OpenApp> {
    let me = std::env::current_exe().ok().and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()));
    capture::win::open_apps().into_iter().filter(|a| Some(&a.exe) != me.as_ref() && !a.exe.eq_ignore_ascii_case(capture::win::FORTNITE_EXE)).collect()
}

/// Guarda las zonas del HUD y el encuadre del modo móvil que se ajustan en el editor.
#[tauri::command]
fn set_mobile_layout(state: State<AppState>, layout: export::MobileLayout) -> CmdResult<()> {
    let mut s = state.shared.settings.write().unwrap();
    s.mobile_layout = layout;
    s.save(&state.shared.settings_path).map_err(err)
}

/// Pasa al formato liviano los clips que ya existían a la calidad de grabación.
#[tauri::command]
async fn compress_clips(app: AppHandle, state: State<'_, AppState>) -> CmdResult<export::Compressed> {
    let shared = state.shared.clone();
    let app2 = app.clone();
    let r = tauri::async_runtime::spawn_blocking(move || export::compress_existing(&app2, &shared)).await.map_err(err)?;
    let _ = app.emit("library", ());
    r.map_err(|e| format!("{e:#}"))
}

/// Resumen de la partida con sus mejores momentos; reemplaza el anterior.
#[tauri::command]
async fn create_reel(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<Clip> {
    let shared = state.shared.clone();
    let (app2, id2) = (app.clone(), id.clone());
    let (clip, old) = tauri::async_runtime::spawn_blocking(move || montage::create_reel(&app2, &shared, &id2))
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))?;
    let _ = app.emit("library", ());
    let stale: Vec<&DriveRef> = old.iter().filter_map(|c| c.drive.as_ref()).collect();
    if let Err(e) = uploads::trash(&state.shared, &stale).await {
        log::warn!("{id}: el resumen anterior sigue en Drive: {e:#}");
    }
    Ok(clip)
}

/// Versión vertical 9:16 de un clip, con el marcador de la partida.
#[tauri::command]
async fn create_vertical(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: String) -> CmdResult<Clip> {
    let shared = state.shared.clone();
    let app2 = app.clone();
    let clip = tauri::async_runtime::spawn_blocking(move || montage::create_vertical(&app2, &shared, &id, &clip_id))
        .await
        .map_err(err)?
        .map_err(|e| format!("{e:#}"))?;
    let _ = app.emit("library", ());
    Ok(clip)
}

#[tauri::command]
async fn drive_download(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: Option<String>) -> CmdResult<()> {
    uploads::download(&app, &state.shared, &id, clip_id.as_deref()).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn drive_open(app: AppHandle, state: State<AppState>, id: String, clip_id: Option<String>) -> CmdResult<()> {
    let m = state.shared.library().load(&id).map_err(err)?;
    let link = match clip_id {
        Some(cid) => m.clips.iter().find(|c| c.id == cid).and_then(|c| c.drive.clone()),
        None => m.drive.clone(),
    }
    .ok_or("Este archivo no está en Drive")?
    .link;
    app.opener().open_url(link, None::<&str>).map_err(err)
}

#[tauri::command]
async fn reprocess_match(app: AppHandle, state: State<'_, AppState>, id: String) -> CmdResult<()> {
    let shared = state.shared.clone();
    let app2 = app.clone();
    tauri::async_runtime::spawn_blocking(move || engine::reprocess(&app2, &shared, &id)).await.map_err(err)?.map_err(|e| format!("{e:#}"))?;
    let _ = app.emit("library", ());
    Ok(())
}

#[tauri::command]
fn start_recording(state: State<AppState>) {
    let _ = state.shared.cmd.lock().unwrap().send(Cmd::StartManual);
}

#[tauri::command]
fn stop_recording(state: State<AppState>) {
    let _ = state.shared.cmd.lock().unwrap().send(Cmd::StopManual);
}

#[tauri::command]
fn save_clip_now(state: State<AppState>) {
    let _ = state.shared.cmd.lock().unwrap().send(Cmd::SaveClip);
}

#[tauri::command]
fn add_bookmark(state: State<AppState>) {
    let _ = state.shared.cmd.lock().unwrap().send(Cmd::Bookmark);
}

#[tauri::command]
fn toggle_recording(state: State<AppState>) {
    let _ = state.shared.cmd.lock().unwrap().send(Cmd::ToggleRecording);
}

#[tauri::command]
fn overlay_close(app: AppHandle) {
    overlay::close_menu(&app);
}

#[tauri::command]
fn overlay_idle(app: AppHandle) {
    overlay::idle(&app);
}

#[derive(Serialize, Clone)]
struct Navigate {
    id: String,
    clip_id: Option<String>,
}

/// Abre la ventana principal en una partida o clip (desde el menú del overlay).
#[tauri::command]
fn open_in_main(app: AppHandle, id: String, clip_id: Option<String>) {
    overlay::close_menu(&app);
    tray::show_main(&app);
    let _ = app.emit_to("main", "navigate", Navigate { id, clip_id });
}

#[tauri::command]
fn list_monitors() -> Vec<capture::win::Output> {
    capture::win::list_outputs()
}

fn media_path(shared: &Shared, id: &str, clip_id: Option<&str>) -> anyhow::Result<(std::path::PathBuf, String)> {
    let lib = shared.library();
    let m = lib.load(id)?;
    Ok(match clip_id {
        Some(cid) => {
            let c = m.clips.iter().find(|c| c.id == cid).ok_or_else(|| anyhow::anyhow!("clip no encontrado"))?;
            (lib.file(id, &c.file), c.title.clone())
        }
        None => (lib.file(id, m.video.as_deref().unwrap_or("match.mp4")), m.title.clone()),
    })
    .and_then(|(path, title)| {
        if !path.exists() {
            anyhow::bail!("Este video está solo en Google Drive: compártelo por Drive o descárgalo de nuevo");
        }
        Ok((path, title))
    })
}

/// Archivo de la partida o del clip en el formato pedido, ya preparado con `share_prepare`.
fn shared_media(app: &AppHandle, shared: &Shared, id: &str, clip_id: Option<&str>, quality: Quality) -> anyhow::Result<(std::path::PathBuf, String)> {
    let (src, title) = media_path(shared, id, clip_id)?;
    let ffmpeg = engine::ffmpeg_path(app, &shared.settings.read().unwrap());
    Ok((export::shared_file(&ffmpeg, &shared.library(), id, clip_id, &src, quality)?, title))
}

/// Deja listo el archivo a compartir en el formato pedido (lo comprime si hace falta).
#[tauri::command]
async fn share_prepare(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: Option<String>, quality: Quality) -> CmdResult<export::ShareFile> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (src, _) = media_path(&shared, &id, clip_id.as_deref())?;
        export::prepare(&app, &shared, &id, clip_id.as_deref(), &src, quality)
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))
}

/// Lleva el archivo a otra app: `copy` (portapapeles), `system` (ventana "Compartir" de
/// Windows) o `reveal` (seleccionarlo en el Explorador).
#[tauri::command]
async fn share_file(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: Option<String>, quality: Quality, action: String) -> CmdResult<()> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        let (path, title) = shared_media(&app, &shared, &id, clip_id.as_deref(), quality)?;
        if action == "reveal" {
            std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display().to_string().replace('/', "\\"))).spawn()?;
            return Ok(());
        }
        let file = winshare::SharedFile::open(&path)?;
        let window = app.get_webview_window("main").ok_or_else(|| anyhow::anyhow!("sin ventana principal"))?;
        let hwnd = windows::Win32::Foundation::HWND(window.hwnd()?.0);
        // El portapapeles y la ventana de compartir solo funcionan desde el hilo de la interfaz.
        let (tx, rx) = std::sync::mpsc::channel();
        let hwnd = hwnd.0 as isize;
        app.run_on_main_thread(move || {
            let r = match action.as_str() {
                "copy" => winshare::copy_to_clipboard(&file),
                _ => winshare::show_share_ui(windows::Win32::Foundation::HWND(hwnd as _), file, &title),
            };
            let _ = tx.send(r);
        })?;
        rx.recv()?
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))
}

/// Guarda una copia del archivo a compartir donde elija el usuario.
#[tauri::command]
async fn share_save(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: Option<String>, quality: Quality, dest: String) -> CmdResult<()> {
    let shared = state.shared.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        let (path, _) = shared_media(&app, &shared, &id, clip_id.as_deref(), quality)?;
        std::fs::copy(path, dest)?;
        Ok(())
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn share_lan(app: AppHandle, state: State<AppState>, id: String, clip_id: Option<String>, quality: Quality, ip: Option<String>) -> CmdResult<ShareLink> {
    let (path, title) = shared_media(&app, &state.shared, &id, clip_id.as_deref(), quality).map_err(|e| format!("{e:#}"))?;
    let mut guard = state.share.lock().unwrap();
    if guard.is_none() {
        let port = state.shared.settings.read().unwrap().share_port;
        *guard = Some(ShareServer::start(port).map_err(|e| format!("{e:#}"))?);
    }
    guard.as_ref().unwrap().share(path, title, ip.as_deref()).map_err(|e| format!("{e:#}"))
}

/// Si el firewall de Windows deja que el teléfono llegue a esta PC por la red de `adapter`.
#[tauri::command]
async fn share_firewall(adapter: Option<String>) -> CmdResult<Firewall> {
    tauri::async_runtime::spawn_blocking(move || share::firewall_state(adapter.as_deref())).await.map_err(err)?.map_err(|e| format!("{e:#}"))
}

/// Permite la app en el firewall (pide permiso de administrador) y devuelve el estado nuevo.
#[tauri::command]
async fn share_firewall_allow(adapter: Option<String>) -> CmdResult<Firewall> {
    tauri::async_runtime::spawn_blocking(move || {
        share::firewall_allow()?;
        share::firewall_state(adapter.as_deref())
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn drive_connect(app: AppHandle, state: State<'_, AppState>) -> CmdResult<String> {
    let d = state.shared.settings.read().unwrap().drive.clone();
    let opener = app.clone();
    let auth = gdrive::authorize(&d.client_id, &d.client_secret, move |url| {
        let _ = opener.opener().open_url(url, None::<&str>);
    })
    .await
    .map_err(|e| format!("{e:#}"))?;
    let email = auth.email.clone().unwrap_or_else(|| "cuenta conectada".into());
    let mut s = state.shared.settings.write().unwrap();
    s.drive.refresh_token = Some(auth.refresh_token);
    s.drive.account_email = auth.email;
    s.save(&state.shared.settings_path).map_err(err)?;
    Ok(email)
}

#[tauri::command]
fn drive_disconnect(state: State<AppState>) -> CmdResult<()> {
    let mut s = state.shared.settings.write().unwrap();
    s.drive.refresh_token = None;
    s.drive.account_email = None;
    s.drive.folder_id = None;
    s.save(&state.shared.settings_path).map_err(err)
}

#[tauri::command]
async fn drive_upload(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: Option<String>) -> CmdResult<DriveRef> {
    uploads::upload(&app, &state.shared, &id, clip_id.as_deref()).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn drive_share(app: AppHandle, state: State<'_, AppState>, id: String, clip_id: Option<String>) -> CmdResult<ShareLink> {
    uploads::drive_link(&app, &state.shared, &id, clip_id.as_deref()).await.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn reveal(state: State<AppState>, id: String, clip_id: Option<String>) -> CmdResult<()> {
    let (path, _) = media_path(&state.shared, &id, clip_id.as_deref()).map_err(err)?;
    std::process::Command::new("explorer").raw_arg(format!("/select,\"{}\"", path.display())).spawn().map_err(err)?;
    Ok(())
}

#[derive(Serialize)]
struct SetupCheck {
    ffmpeg: Option<String>,
    ffmpeg_path: String,
    /// El codificador elegido funciona en esta PC.
    nvenc: bool,
    /// Codificadores que funcionan en esta PC.
    encoders: Vec<String>,
    /// El que se usa de verdad (otro, si el elegido no funciona aquí).
    encoder_in_use: String,
    fortnite_log: bool,
    replays_enabled: Option<bool>,
}

#[tauri::command]
async fn check_setup(app: AppHandle, state: State<'_, AppState>) -> CmdResult<SetupCheck> {
    let settings = state.shared.settings.read().unwrap().clone();
    let ffmpeg = engine::ffmpeg_path(&app, &settings);
    tauri::async_runtime::spawn_blocking(move || {
        let run = |args: &[&str]| std::process::Command::new(&ffmpeg).args(args).creation_flags(CREATE_NO_WINDOW).output().ok();
        let version = run(&["-hide_banner", "-version"]).filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).lines().next().unwrap_or("").to_string());
        let encoders = capture::video::detect_encoders(&ffmpeg).to_vec();
        let nvenc = encoders.contains(&settings.encoder);
        let encoder_in_use = capture::video::pick_encoder(&settings.encoder, Some(&encoders));
        let ini = std::env::var_os("LOCALAPPDATA").map(|p| std::path::PathBuf::from(p).join("FortniteGame\\Saved\\Config\\WindowsClient\\GameUserSettings.ini"));
        let replays_enabled = ini.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| {
            s.lines().find_map(|l| l.trim().strip_prefix("bReplayRecordingEnabled=").map(|v| v.eq_ignore_ascii_case("true")))
        });
        SetupCheck {
            ffmpeg: version,
            ffmpeg_path: ffmpeg.display().to_string(),
            nvenc,
            encoders,
            encoder_in_use,
            fortnite_log: fortnite::log::log_path().is_some_and(|p| p.exists()),
            replays_enabled,
        }
    })
    .await
    .map_err(err)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let minimized_arg = std::env::args().any(|a| a == "--minimized");
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| tray::show_main(app)))
        .plugin(tauri_plugin_log::Builder::new().level(log::LevelFilter::Info).build())
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let settings_path = app.path().app_config_dir()?.join("settings.json");
            let settings = Settings::load(&settings_path);
            let _ = std::fs::create_dir_all(&settings.library_dir);
            let start_hidden = minimized_arg || settings.start_minimized;
            let keys = settings.hotkeys.clone();
            let shared = engine::spawn(app.handle().clone(), settings, settings_path);
            // Las partidas que ya existían entran al histórico la primera vez.
            if let Err(e) = stats::seed(&stats::path_for(&shared.settings_path), &shared.library()) {
                log::warn!("no se pudieron preparar los totales: {e:#}");
            }
            retention::spawn(app.handle().clone(), shared.clone());
            app.manage(AppState { shared, share: Mutex::new(None) });
            tray::build(app.handle())?;
            overlay::create(app.handle())?;
            hotkeys::apply(app.handle(), &keys);
            if let Some(w) = app.get_webview_window("main")
                && !start_hidden
            {
                let _ = w.show();
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                let state = window.state::<AppState>();
                if state.shared.settings.read().unwrap().close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_settings,
            save_settings,
            list_matches,
            get_match,
            get_totals,
            delete_match,
            update_match,
            delete_clip,
            export_clip,
            reprocess_match,
            start_recording,
            stop_recording,
            list_monitors,
            save_clip_now,
            add_bookmark,
            toggle_recording,
            overlay_close,
            overlay_idle,
            open_in_main,
            share_lan,
            share_prepare,
            share_file,
            share_save,
            share_firewall,
            share_firewall_allow,
            compress_clips,
            set_mobile_layout,
            set_mobile_presets,
            join_clips,
            create_weekly,
            list_open_apps,
            drive_connect,
            drive_disconnect,
            drive_upload,
            drive_share,
            drive_download,
            drive_open,
            reveal,
            check_setup,
            create_reel,
            create_vertical,
        ])
        .run(tauri::generate_context!())
        .expect("error al iniciar ggswp");
}
