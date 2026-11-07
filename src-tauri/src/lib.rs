pub mod capture;
pub mod config;
pub mod engine;
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

use std::os::windows::process::CommandExt;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_opener::OpenerExt as _;

use capture::video::CREATE_NO_WINDOW;
use config::Settings;
use engine::{Cmd, Shared, Status};
use library::{Clip, DriveRef, MatchMeta};
use share::{ShareLink, ShareServer};

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
}

fn view(shared: &Shared, meta: MatchMeta) -> MatchView {
    let dir = shared.library().dir(&meta.id).display().to_string();
    MatchView { meta, dir }
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
    settings.save(&state.shared.settings_path).map_err(err)?;
    let autostart = app.autolaunch();
    let _ = if settings.launch_at_startup { autostart.enable() } else { autostart.disable() };
    if settings.share_port != prev_port {
        *state.share.lock().unwrap() = None;
    }
    let failed = hotkeys::apply(&app, &settings.hotkeys);
    *state.shared.settings.write().unwrap() = settings;
    let _ = app.emit("library", ());
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
    if let Some(c) = m.clips.iter().find(|c| c.id == clip_id && Some(&c.file) != m.video.as_ref()) {
        let _ = std::fs::remove_file(lib.file(&id, &c.file));
        if let Some(t) = &c.thumb {
            let _ = std::fs::remove_file(lib.file(&id, t));
        }
    }
    m.clips.retain(|c| c.id != clip_id);
    lib.save(&m).map_err(err)?;
    let _ = app.emit("library", ());
    Ok(())
}

#[tauri::command]
async fn export_clip(app: AppHandle, state: State<'_, AppState>, id: String, start: f64, end: f64, title: String) -> CmdResult<Clip> {
    if end - start < 0.5 {
        return Err("El clip es demasiado corto".into());
    }
    let shared = state.shared.clone();
    let app2 = app.clone();
    let clip = tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<Clip> {
        let settings = shared.settings.read().unwrap().clone();
        let ffmpeg = engine::ffmpeg_path(&app2, &settings);
        let lib = shared.library();
        let mut m = lib.load(&id)?;
        if m.offloaded {
            anyhow::bail!("El video está solo en Drive: descárgalo para editarlo");
        }
        let video = lib.file(&id, m.video.as_deref().unwrap_or("match.mp4"));
        let cid = library::new_id("edit");
        std::fs::create_dir_all(lib.file(&id, "clips"))?;
        let file = format!("clips/{cid}.mp4");
        library::cut_precise(&ffmpeg, &video, start, end, &engine::video_options(&settings, 0), &lib.file(&id, &file))?;
        let thumb = format!("clips/{cid}.jpg");
        let thumb_ok = library::thumbnail(&ffmpeg, &lib.file(&id, &file), ((end - start) / 2.0).min(3.0), &lib.file(&id, &thumb)).is_ok();
        let title = if title.trim().is_empty() { "Clip".to_string() } else { title.trim().to_string() };
        let clip = Clip { id: cid, file, thumb: thumb_ok.then_some(thumb), start, end, title, kind: "manual".into(), created_at: chrono::Utc::now(), drive: None, offloaded: false };
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

#[tauri::command]
fn share_lan(state: State<AppState>, id: String, clip_id: Option<String>) -> CmdResult<ShareLink> {
    let (path, title) = media_path(&state.shared, &id, clip_id.as_deref()).map_err(err)?;
    let mut guard = state.share.lock().unwrap();
    if guard.is_none() {
        let port = state.shared.settings.read().unwrap().share_port;
        *guard = Some(ShareServer::start(port).map_err(|e| format!("{e:#}"))?);
    }
    guard.as_ref().unwrap().share(path, title).map_err(|e| format!("{e:#}"))
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
    nvenc: bool,
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
        let nvenc = run(&["-hide_banner", "-encoders"]).map(|o| String::from_utf8_lossy(&o.stdout).contains(&settings.encoder)).unwrap_or(false);
        let ini = std::env::var_os("LOCALAPPDATA").map(|p| std::path::PathBuf::from(p).join("FortniteGame\\Saved\\Config\\WindowsClient\\GameUserSettings.ini"));
        let replays_enabled = ini.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| {
            s.lines().find_map(|l| l.trim().strip_prefix("bReplayRecordingEnabled=").map(|v| v.eq_ignore_ascii_case("true")))
        });
        SetupCheck {
            ffmpeg: version,
            ffmpeg_path: ffmpeg.display().to_string(),
            nvenc,
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
