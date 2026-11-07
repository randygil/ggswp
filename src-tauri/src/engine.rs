//! Motor: sigue el log de Fortnite, decide cuándo grabar y post-procesa cada partida.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::capture::video::VideoOptions;
use crate::capture::{self, RawRecording, Session, SessionConfig, win};
use crate::config::{Settings, StartAt, StopAt};
use crate::fortnite::log::{LogEvent, LogTailer, log_path};
use crate::fortnite::replay;
use crate::library::{self, Clip, Library, MatchContext, MatchMeta, MatchStatus};

const MAX_RECORDING: Duration = Duration::from_secs(90 * 60);

#[derive(Debug, Clone, Serialize, Default)]
pub struct RecordingInfo {
    pub match_id: String,
    pub started_at: DateTime<Utc>,
    pub manual: bool,
}

/// Marca en vivo de la grabación actual (para el menú de highlights).
#[derive(Debug, Clone, Serialize)]
pub struct LiveMarker {
    /// Segundos desde el inicio de la grabación.
    pub t: f64,
    /// highlight | bookmark | clip
    pub kind: String,
    pub label: String,
}

/// Paso en curso al procesar una partida (para mostrar progreso).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Step {
    pub label: String,
    /// 0–100, si el paso puede medirse.
    pub pct: Option<f32>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Status {
    /// Paso en curso de cada partida en `processing`.
    pub steps: std::collections::BTreeMap<String, Step>,
    pub fortnite_running: bool,
    /// idle | lobby | loading | in_match
    pub game_state: String,
    pub recording: Option<RecordingInfo>,
    pub processing: Vec<String>,
    pub last_error: Option<String>,
    pub account_id: Option<String>,
    pub live: Vec<LiveMarker>,
    /// El búfer de repetición está grabando en segundo plano.
    pub buffer_active: bool,
}

pub enum Cmd {
    StartManual,
    StopManual,
    ToggleRecording,
    SaveClip,
    Bookmark,
}

struct BufferSession {
    session: Session,
    started: Instant,
}

pub struct Shared {
    pub settings: RwLock<Settings>,
    pub settings_path: PathBuf,
    pub status: Mutex<Status>,
    pub cmd: Mutex<Sender<Cmd>>,
    in_match: AtomicBool,
}

impl Shared {
    pub fn library(&self) -> Library {
        Library::new(self.settings.read().unwrap().library_dir.clone())
    }

    /// Estás en una partida (cargando o jugando) o se está grabando.
    pub fn is_playing(&self) -> bool {
        self.in_match.load(Ordering::SeqCst) || self.status.lock().unwrap().recording.is_some()
    }

    /// Publica el paso en curso de una partida. Los porcentajes se mandan solo cuando
    /// cambian al menos un punto, para no saturar la interfaz.
    pub fn set_step(&self, app: &AppHandle, id: &str, label: &str, pct: Option<f32>) {
        let snapshot = {
            let mut st = self.status.lock().unwrap();
            let step = Step { label: label.to_string(), pct: pct.map(|p| p.clamp(0.0, 100.0).floor()) };
            if st.steps.get(id) == Some(&step) {
                return;
            }
            st.steps.insert(id.to_string(), step);
            st.clone()
        };
        let _ = app.emit("status", snapshot);
    }
}

pub fn ffmpeg_path(app: &AppHandle, s: &Settings) -> PathBuf {
    if let Some(p) = &s.ffmpeg_path
        && p.exists()
    {
        return p.clone();
    }
    if let Ok(dir) = app.path().resource_dir() {
        let p = dir.join("bin").join("ffmpeg.exe");
        if p.exists() {
            return p;
        }
    }
    PathBuf::from("ffmpeg")
}

pub fn video_options(s: &Settings, output_index: u32) -> VideoOptions {
    VideoOptions { output_index, fps: s.fps, encoder: s.encoder.clone(), cq: s.quality_cq, max_bitrate_mbps: s.max_bitrate_mbps }
}

pub fn spawn(app: AppHandle, settings: Settings, settings_path: PathBuf) -> Arc<Shared> {
    let (tx, rx) = channel();
    let shared = Arc::new(Shared {
        settings: RwLock::new(settings),
        settings_path,
        status: Mutex::new(Status { game_state: "idle".into(), ..Default::default() }),
        cmd: Mutex::new(tx),
        in_match: AtomicBool::new(false),
    });
    let s = shared.clone();
    std::thread::Builder::new()
        .name("engine".into())
        .spawn(move || Engine::new(app, s, rx).run())
        .expect("hilo del motor");
    shared
}

struct Active {
    session: Session,
    match_id: String,
    manual: bool,
    started: Instant,
    stop_at: Option<Instant>,
}

struct Engine {
    app: AppHandle,
    shared: Arc<Shared>,
    rx: Receiver<Cmd>,
    tailer: Option<LogTailer>,
    ctx: MatchContext,
    in_match: bool,
    match_started: bool,
    active: Option<Active>,
    last_game_check: Instant,
    game_missing_since: Option<Instant>,
    buffer: Vec<BufferSession>,
    buffer_seq: u32,
}

impl Engine {
    fn new(app: AppHandle, shared: Arc<Shared>, rx: Receiver<Cmd>) -> Self {
        Self {
            app,
            shared,
            rx,
            tailer: log_path().map(LogTailer::new),
            ctx: MatchContext::default(),
            in_match: false,
            match_started: false,
            active: None,
            last_game_check: Instant::now() - Duration::from_secs(10),
            game_missing_since: None,
            buffer: Vec::new(),
            buffer_seq: 0,
        }
    }

    fn settings(&self) -> Settings {
        self.shared.settings.read().unwrap().clone()
    }

    fn set_status(&self, f: impl FnOnce(&mut Status)) {
        let snapshot = {
            let mut st = self.shared.status.lock().unwrap();
            f(&mut st);
            st.clone()
        };
        let _ = self.app.emit("status", snapshot);
    }

    fn run(mut self) {
        // Reconstruye el estado actual por si la app se abre con una partida en curso.
        if let Some(t) = self.tailer.as_mut() {
            let past = t.catch_up();
            for e in past {
                self.apply(e, true);
            }
        }
        let running = win::fortnite_running();
        self.set_status(|s| s.fortnite_running = running);
        if running && self.in_match && self.match_started && self.settings().auto_record {
            log::info!("Partida en curso al abrir la app: empiezo a grabar ya");
            self.start_recording(false);
        } else if !running {
            self.in_match = false;
        }
        self.publish_game_state();

        loop {
            while let Ok(cmd) = self.rx.try_recv() {
                match cmd {
                    Cmd::StartManual if self.active.is_none() => self.start_recording(true),
                    Cmd::StopManual => self.stop_recording("manual"),
                    Cmd::ToggleRecording if self.active.is_some() => self.stop_recording("atajo"),
                    Cmd::ToggleRecording => self.start_recording(true),
                    Cmd::SaveClip => self.save_clip(),
                    Cmd::Bookmark => self.bookmark(),
                    _ => {}
                }
            }
            let events = self.tailer.as_mut().map(|t| t.poll()).unwrap_or_default();
            for e in events {
                self.apply(e, false);
            }
            self.tick();
            std::thread::sleep(Duration::from_millis(250));
        }
    }

    fn publish_game_state(&self) {
        let state = if self.in_match { if self.match_started { "in_match" } else { "loading" } } else { "lobby" };
        self.shared.in_match.store(self.in_match, Ordering::SeqCst);
        let account = self.ctx.account_id.clone();
        self.set_status(|s| {
            s.game_state = if s.fortnite_running { state.into() } else { "idle".into() };
            if account.is_some() {
                s.account_id = account;
            }
        });
    }

    fn apply(&mut self, t: crate::fortnite::log::Timed, historic: bool) {
        let settings = self.settings();
        let recording_auto = self.active.as_ref().is_some_and(|a| !a.manual);
        match &t.event {
            LogEvent::SessionStart => {
                self.in_match = false;
                self.match_started = false;
                if recording_auto {
                    self.stop_recording("fortnite se reinició");
                }
            }
            LogEvent::MatchLoading { account_id, map } => {
                if recording_auto {
                    self.stop_recording("nueva partida");
                }
                self.ctx = MatchContext { account_id: account_id.clone(), map: Some(map.clone()), ..Default::default() };
                self.in_match = true;
                self.match_started = false;
            }
            LogEvent::Lobby => {
                self.in_match = false;
                self.match_started = false;
                if recording_auto {
                    self.stop_recording("vuelta al lobby");
                }
            }
            LogEvent::MatchState(s) if s == "InProgress" => {
                self.match_started = true;
                if !historic && self.in_match && settings.auto_record && settings.start_at == StartAt::Warmup && self.active.is_none() {
                    self.ctx.log.push(t.clone());
                    self.start_recording(false);
                    self.publish_game_state();
                    return;
                }
            }
            LogEvent::GamePhase(p) if p == "Aircraft" => {
                if !historic && self.in_match && settings.auto_record && settings.start_at == StartAt::Bus && self.active.is_none() {
                    self.ctx.log.push(t.clone());
                    self.start_recording(false);
                    self.publish_game_state();
                    return;
                }
            }
            LogEvent::PlayerFinished => {
                if settings.stop_at == StopAt::PlayerFinished
                    && let Some(a) = self.active.as_mut().filter(|a| !a.manual)
                {
                    a.stop_at = Some(Instant::now() + Duration::from_secs(settings.post_roll_s as u64));
                }
            }
            LogEvent::NvHighlight if !historic && self.active.is_some() => {
                // Fortnite avisa a NVIDIA ~4,5 s después de una eliminación, muerte o victoria.
                let t_rel = self.live_t() - 4.5;
                self.push_live(t_rel.max(0.0), "highlight", "Momento destacado");
                if settings.overlay_moments {
                    crate::overlay::toast(&self.app, "Momento destacado registrado", "moment");
                }
            }
            LogEvent::Playlist(p) => self.ctx.playlist = Some(p.clone()),
            LogEvent::Presence(p) if p.contains(" - ") => self.ctx.presence = Some(p.clone()),
            _ => {}
        }
        if self.in_match || self.active.is_some() {
            self.ctx.log.push(t);
        }
        if !historic {
            self.publish_game_state();
        }
    }

    fn tick(&mut self) {
        if self.last_game_check.elapsed() > Duration::from_secs(2) {
            self.last_game_check = Instant::now();
            let running = win::fortnite_running();
            let was = self.shared.status.lock().unwrap().fortnite_running;
            if running != was {
                self.set_status(|s| s.fortnite_running = running);
                if !running {
                    self.in_match = false;
                }
                self.publish_game_state();
            }
            self.game_missing_since = if running { None } else { self.game_missing_since.or(Some(Instant::now())) };
        }
        self.tick_buffer();
        let Some(a) = self.active.as_mut() else { return };
        let reason = if !a.session.is_alive() {
            Some("ffmpeg se detuvo")
        } else if a.stop_at.is_some_and(|t| Instant::now() >= t) {
            Some("jugador terminó")
        } else if a.started.elapsed() > MAX_RECORDING {
            Some("duración máxima")
        } else if !a.manual && self.game_missing_since.is_some_and(|t| t.elapsed() > Duration::from_secs(5)) {
            Some("fortnite se cerró")
        } else {
            None
        };
        if let Some(r) = reason {
            self.stop_recording(r);
        }
    }

    fn start_recording(&mut self, manual: bool) {
        let settings = self.settings();
        let output = settings.monitor.or_else(win::fortnite_output_index).unwrap_or(0);
        let cfg = SessionConfig {
            ffmpeg: ffmpeg_path(&self.app, &settings),
            video: video_options(&settings, output),
            system_audio: settings.system_audio,
            microphone: settings.microphone,
            mic_volume: settings.mic_volume,
        };
        self.clear_buffer();
        let match_id = Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
        let lib = Library::new(settings.library_dir.clone());
        let dir = lib.dir(&match_id);
        log::info!("Grabando {match_id} (monitor {output}, {} cq{})", settings.encoder, settings.quality_cq);
        match Session::start(dir, cfg) {
            Ok(session) => {
                let meta = MatchMeta {
                    id: match_id.clone(),
                    created_at: session.started_at(),
                    title: if manual { "Grabación manual".into() } else { "Partida en curso".into() },
                    map: self.ctx.map.clone(),
                    status: MatchStatus::Recording,
                    ..Default::default()
                };
                let _ = lib.save(&meta);
                let info = RecordingInfo { match_id: match_id.clone(), started_at: session.started_at(), manual };
                self.active = Some(Active { session, match_id, manual, started: Instant::now(), stop_at: None });
                if manual {
                    self.ctx = MatchContext::default();
                }
                self.set_status(|s| {
                    s.recording = Some(info);
                    s.last_error = None;
                    s.live.clear();
                });
                let _ = self.app.emit("library", ());
                crate::tray::refresh(&self.app, true);
                crate::overlay::toast(&self.app, if manual { "Grabación iniciada" } else { "Grabando partida" }, "rec");
            }
            Err(e) => {
                log::error!("No se pudo iniciar la grabación: {e:#}");
                self.set_status(|s| s.last_error = Some(format!("No se pudo grabar: {e:#}")));
            }
        }
    }

    fn stop_recording(&mut self, reason: &str) {
        let Some(a) = self.active.take() else { return };
        log::info!("Deteniendo {} ({reason})", a.match_id);
        let ctx = std::mem::take(&mut self.ctx);
        // Si seguimos en la partida (p. ej. modo espectador), conservamos el contexto.
        if self.in_match {
            self.ctx = MatchContext { account_id: ctx.account_id.clone(), map: ctx.map.clone(), ..Default::default() };
        }
        let id = a.match_id.clone();
        self.set_status(|s| {
            s.recording = None;
            s.processing.push(id.clone());
        });
        crate::tray::refresh(&self.app, false);
        crate::overlay::toast(&self.app, "Grabación guardada · procesando", "info");
        let raw = a.session.stop();
        let app = self.app.clone();
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let lib = shared.library();
            let result = raw.and_then(|raw| process(&app, &shared, &lib, &id, raw, ctx, a.manual));
            if let Err(e) = &result {
                log::error!("Procesando {id}: {e:#}");
                if let Ok(mut m) = lib.load(&id) {
                    m.status = MatchStatus::Error;
                    m.error = Some(format!("{e:#}"));
                    let _ = lib.save(&m);
                }
            }
            let snapshot = {
                let mut st = shared.status.lock().unwrap();
                st.processing.retain(|p| p != &id);
                st.steps.remove(&id);
                st.clone()
            };
            let _ = app.emit("status", snapshot);
            let _ = app.emit("library", ());
            if result.is_ok() {
                crate::uploads::maybe_auto_upload(&app, &shared, &id);
            }
        });
    }
}

impl Engine {
    fn live_t(&self) -> f64 {
        self.active.as_ref().map(|a| a.started.elapsed().as_secs_f64()).unwrap_or(0.0)
    }

    fn push_live(&self, t: f64, kind: &str, label: &str) {
        let m = LiveMarker { t, kind: kind.into(), label: label.into() };
        self.set_status(|s| s.live.push(m));
    }

    fn bookmark(&mut self) {
        if self.active.is_some() {
            self.ctx.bookmarks.push(Utc::now());
            self.push_live(self.live_t(), "bookmark", "Momento marcado");
            crate::overlay::toast(&self.app, "Momento marcado", "bookmark");
        } else {
            crate::overlay::toast(&self.app, "No hay una grabación en curso", "warn");
        }
    }

    fn save_clip(&mut self) {
        let secs = self.settings().clip_seconds;
        if self.active.is_some() {
            self.ctx.clip_requests.push(Utc::now());
            self.push_live(self.live_t(), "clip", &format!("Clip de {secs} s"));
            crate::overlay::toast(&self.app, &format!("Clip de {secs} s guardado · se exporta al terminar"), "clip");
            return;
        }
        if self.buffer.is_empty() {
            crate::overlay::toast(&self.app, "No hay nada grabando (activa el búfer en Ajustes)", "warn");
            return;
        }
        // El más antiguo tiene más historia. Antes de detenerlo arrancamos otro para no
        // dejar huecos en el búfer.
        if self.buffer.len() == 1 {
            self.start_buffer_session();
        }
        let old = self.buffer.remove(0);
        crate::overlay::toast(&self.app, &format!("Guardando los últimos {secs} s…"), "clip");
        let raw = old.session.stop();
        let (app, shared) = (self.app.clone(), self.shared.clone());
        std::thread::spawn(move || {
            match raw.and_then(|raw| save_buffer_clip(&app, &shared, raw, secs as f64)) {
                Ok(()) => crate::overlay::toast(&app, &format!("Clip de {secs} s guardado"), "clip"),
                Err(e) => {
                    log::error!("clip del búfer: {e:#}");
                    crate::overlay::toast(&app, "No se pudo guardar el clip", "warn");
                }
            }
            let _ = app.emit("library", ());
        });
    }

    fn buffer_root(&self) -> PathBuf {
        self.app.path().app_local_data_dir().unwrap_or_else(|_| std::env::temp_dir()).join("buffer")
    }

    fn start_buffer_session(&mut self) {
        let settings = self.settings();
        let output = settings.monitor.or_else(win::fortnite_output_index).unwrap_or(0);
        self.buffer_seq += 1;
        let dir = self.buffer_root().join(format!("b{}", self.buffer_seq));
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = SessionConfig {
            ffmpeg: ffmpeg_path(&self.app, &settings),
            video: video_options(&settings, output),
            system_audio: settings.system_audio,
            microphone: settings.microphone,
            mic_volume: settings.mic_volume,
        };
        match Session::start(dir, cfg) {
            Ok(session) => self.buffer.push(BufferSession { session, started: Instant::now() }),
            Err(e) => {
                log::error!("búfer: {e:#}");
                self.set_status(|s| s.last_error = Some(format!("No se pudo iniciar el búfer: {e:#}")));
                // Evita reintentar en bucle: queda apagado hasta que se guarden los ajustes.
                self.shared.settings.write().unwrap().replay_buffer = false;
            }
        }
    }

    fn discard(b: BufferSession) {
        let dir = b.session.dir.clone();
        if let Ok(raw) = b.session.stop() {
            capture::remove_raw(&raw);
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    fn clear_buffer(&mut self) {
        for b in self.buffer.drain(..) {
            Self::discard(b);
        }
        if self.shared.status.lock().unwrap().buffer_active {
            self.set_status(|s| s.buffer_active = false);
        }
    }

    /// Mantiene 1 o 2 sesiones superpuestas para tener siempre al menos `clip_seconds` de historia.
    fn tick_buffer(&mut self) {
        let settings = self.settings();
        let running = self.shared.status.lock().unwrap().fortnite_running;
        if !(settings.replay_buffer && running && self.active.is_none()) {
            if !self.buffer.is_empty() {
                self.clear_buffer();
            }
            return;
        }
        let mut dead = Vec::new();
        let mut i = 0;
        while i < self.buffer.len() {
            if self.buffer[i].session.is_alive() {
                i += 1;
            } else {
                dead.push(self.buffer.remove(i));
            }
        }
        dead.into_iter().for_each(Self::discard);
        let need = Duration::from_secs(settings.clip_seconds as u64 + 5);
        let rotate = Duration::from_secs((settings.clip_seconds as u64 + 60).max(120));
        if self.buffer.last().is_none_or(|b| b.started.elapsed() >= rotate) {
            self.start_buffer_session();
        }
        if self.buffer.len() >= 2 && self.buffer.last().is_some_and(|b| b.started.elapsed() >= need) {
            let old = self.buffer.remove(0);
            Self::discard(old);
        }
        let active = !self.buffer.is_empty();
        if self.shared.status.lock().unwrap().buffer_active != active {
            self.set_status(|s| s.buffer_active = active);
        }
    }
}

/// Recorta los últimos `secs` segundos de una sesión del búfer y los guarda como clip suelto.
fn save_buffer_clip(app: &AppHandle, shared: &Shared, raw: RawRecording, secs: f64) -> anyhow::Result<()> {
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = ffmpeg_path(app, &settings);
    let full = raw.dir.join("full.mp4");
    capture::finalize(&ffmpeg, &raw, &full)?;
    capture::remove_raw(&raw);
    let p = library::probe(&ffmpeg, &full)?;
    let lib = shared.library();
    let id = format!("{}_clip", Local::now().format("%Y-%m-%d_%H-%M-%S"));
    std::fs::create_dir_all(lib.dir(&id))?;
    let start = (p.duration - secs).max(0.0);
    let out = lib.file(&id, "match.mp4");
    library::cut_copy(&ffmpeg, &full, start, p.duration, &out)?;
    let _ = std::fs::remove_dir_all(&raw.dir);
    let cp = library::probe(&ffmpeg, &out)?;
    let thumb = library::thumbnail(&ffmpeg, &out, (cp.duration - 2.0).max(0.0), &lib.file(&id, "thumb.jpg")).ok().map(|_| "thumb.jpg".to_string());
    let created = raw.video_started_at + chrono::Duration::milliseconds((start * 1000.0) as i64);
    let meta = MatchMeta {
        id: id.clone(),
        created_at: created,
        title: "Clip guardado".into(),
        duration_s: cp.duration,
        width: cp.width,
        height: cp.height,
        size_bytes: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
        video: Some("match.mp4".into()),
        thumb: thumb.clone(),
        clips: vec![Clip {
            id: "hk01".into(),
            file: "match.mp4".into(),
            thumb,
            start: 0.0,
            end: cp.duration,
            title: "Clip guardado".into(),
            kind: "hotkey".into(),
            created_at: created,
            drive: None,
            offloaded: false,
        }],
        status: MatchStatus::Ready,
        ..Default::default()
    };
    lib.save(&meta)?;
    Ok(())
}

fn process(app: &AppHandle, shared: &Shared, lib: &Library, id: &str, raw: RawRecording, ctx: MatchContext, manual: bool) -> anyhow::Result<()> {
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = ffmpeg_path(app, &settings);
    let mut meta = lib.load(id).unwrap_or_else(|_| MatchMeta { id: id.into(), created_at: raw.video_started_at, ..Default::default() });
    meta.status = MatchStatus::Processing;
    meta.created_at = raw.video_started_at;
    lib.save(&meta)?;
    let _ = app.emit("library", ());

    let video = lib.file(id, "match.mp4");
    shared.set_step(app, id, "Guardando el video", None);
    capture::finalize(&ffmpeg, &raw, &video)?;
    capture::remove_raw(&raw);
    let probe = library::probe(&ffmpeg, &video)?;
    meta.video = Some("match.mp4".into());
    meta.duration_s = probe.duration;
    meta.width = probe.width;
    meta.height = probe.height;
    meta.size_bytes = std::fs::metadata(&video).map(|m| m.len()).unwrap_or(0);
    meta.mode = ctx.mode_name();
    meta.playlist = ctx.playlist.clone();
    meta.map = ctx.map.clone().or(meta.map);
    meta.account_id = ctx.account_id.clone().or(meta.account_id);

    let rep = if manual { None } else { wait_for_replay(app, shared, id, raw.video_started_at, probe.duration) };
    meta.replay = rep.as_ref().map(|r| r.path.display().to_string());
    let step = |label: &str, pct: Option<f32>| shared.set_step(app, id, label, pct);
    analyze(&ffmpeg, lib, &mut meta, &ctx, rep.as_ref(), raw.video_started_at, &settings, Vec::new(), &step)?;
    meta.status = MatchStatus::Ready;
    lib.save(&meta)?;
    log::info!("{id} listo: {} elim, {} cofres, victoria={}", meta.kills, meta.chests, meta.victory);
    if let Err(e) = crate::stats::record(&crate::stats::path_for(&shared.settings_path), &meta) {
        log::warn!("no se pudieron guardar los totales: {e:#}");
    }
    Ok(())
}

fn secs_between(from: DateTime<Utc>, to: DateTime<Utc>) -> f64 {
    (to - from).num_milliseconds() as f64 / 1000.0
}

/// Calcula eventos, título, miniatura y clips automáticos. También sirve para re-procesar.
pub fn analyze(
    ffmpeg: &std::path::Path,
    lib: &Library,
    meta: &mut MatchMeta,
    ctx: &MatchContext,
    rep: Option<&replay::Replay>,
    video_start: DateTime<Utc>,
    settings: &Settings,
    kept: Vec<library::MatchEvent>,
    step: &dyn Fn(&str, Option<f32>),
) -> anyhow::Result<()> {
    let video = lib.file(&meta.id, "match.mp4");
    let sum = library::build_events(ctx, rep, video_start, meta.duration_s);
    meta.kills = sum.kills;
    meta.knocks = sum.knocks;
    meta.died = sum.died || kept.iter().any(|e| e.kind == "death");
    meta.victory = sum.victory || kept.iter().any(|e| e.kind == "victory");
    meta.apply_stats(rep.and_then(|r| r.stats.as_ref()));
    meta.events = sum.events;
    meta.events.extend(kept);
    meta.chests = 0;
    if settings.detect_chests && !meta.offloaded && video.exists() {
        let started = Instant::now();
        step("Contando cofres", Some(0.0));
        let total = meta.duration_s.max(1.0);
        match crate::fortnite::chests::scan(ffmpeg, &video, meta.width, meta.height, &mut |t| step("Contando cofres", Some((t / total * 100.0) as f32))) {
            Ok(times) => {
                log::info!("{}: {} cofres ({:.0} s)", meta.id, times.len(), started.elapsed().as_secs_f64());
                meta.chests = times.len() as u32;
                meta.events.extend(times.into_iter().map(|t| library::MatchEvent { t, kind: "chest".into(), label: "Cofre abierto".into(), distance_m: None }));
            }
            Err(e) => log::warn!("{}: no se pudieron contar los cofres: {e:#}", meta.id),
        }
    }
    meta.events.sort_by(|a, b| a.t.total_cmp(&b.t));
    if ctx.account_id.is_some() || rep.is_some() {
        let elim = if meta.kills == 1 { "eliminación".to_string() } else { "eliminaciones".to_string() };
        meta.title = if meta.victory { format!("Victoria · {} {elim}", meta.kills) } else { format!("{} {elim}", meta.kills) };
    }

    step("Creando miniatura y clips", None);
    let thumb_at = meta.events.iter().find(|e| e.kind == "kill").map(|e| (e.t - 1.0).max(0.0)).unwrap_or(meta.duration_s * 0.3);
    if library::thumbnail(ffmpeg, &video, thumb_at, &lib.file(&meta.id, "thumb.jpg")).is_ok() {
        meta.thumb = Some("thumb.jpg".into());
    }

    // Los clips que salen iguales se reutilizan: así no se pierde su copia en Drive (ni la
    // única copia, si ya se borró de la PC). Al re-analizar no hay pedidos de atajo: se
    // conservan los clips de atajo que ya existían.
    let previous = std::mem::take(&mut meta.clips);
    let id = meta.id.clone();
    let reuse = |cid: &str, start: f64, end: f64| {
        previous
            .iter()
            .find(|c| c.id == cid && (c.start - start).abs() < 0.05 && (c.end - end).abs() < 0.05 && (c.offloaded || lib.file(&id, &c.file).exists()))
            .cloned()
    };
    meta.clips = previous
        .iter()
        .filter(|c| matches!(c.kind.as_str(), "manual" | "reel" | "vertical") || (c.kind == "hotkey" && ctx.clip_requests.is_empty()))
        .cloned()
        .collect();
    let clips_dir = lib.file(&meta.id, "clips");
    for (i, at) in ctx.clip_requests.iter().enumerate() {
        let end = (secs_between(video_start, *at)).min(meta.duration_s);
        let start = (end - settings.clip_seconds as f64).max(0.0);
        if end - start < 1.0 {
            continue;
        }
        std::fs::create_dir_all(&clips_dir)?;
        let cid = format!("hk{:02}", i + 1);
        if let Some(c) = reuse(&cid, start, end) {
            meta.clips.push(c);
            continue;
        }
        let file = format!("clips/{cid}.mp4");
        if let Err(e) = library::cut_copy(ffmpeg, &video, start, end, &lib.file(&meta.id, &file)) {
            log::warn!("clip {cid}: {e:#}");
            continue;
        }
        let thumb = format!("clips/{cid}.jpg");
        let thumb_ok = library::thumbnail(ffmpeg, &lib.file(&meta.id, &file), ((end - start) - 2.0).max(0.0), &lib.file(&meta.id, &thumb)).is_ok();
        meta.clips.push(Clip { id: cid, file, thumb: thumb_ok.then_some(thumb), start, end, title: "Clip guardado".into(), kind: "hotkey".into(), created_at: Utc::now(), drive: None, offloaded: false });
    }
    if settings.auto_clips {
        std::fs::create_dir_all(&clips_dir)?;
        let windows = library::clip_windows(&meta.events, settings.clip_before_s, settings.clip_after_s, meta.duration_s, settings.clip_deaths);
        let n = windows.len();
        for (i, (start, end, title, kind)) in windows.into_iter().enumerate() {
            step(&format!("Creando clips ({}/{n})", i + 1), Some(i as f32 / n as f32 * 100.0));
            let cid = format!("auto{:02}", i + 1);
            if let Some(c) = reuse(&cid, start, end) {
                meta.clips.push(c);
                continue;
            }
            let file = format!("clips/{cid}.mp4");
            let out = lib.file(&meta.id, &file);
            if let Err(e) = library::cut_copy(ffmpeg, &video, start, end, &out) {
                log::warn!("clip {cid}: {e:#}");
                continue;
            }
            let thumb = format!("clips/{cid}.jpg");
            let thumb_ok = library::thumbnail(ffmpeg, &out, ((end - start) - settings.clip_after_s).max(0.0), &lib.file(&meta.id, &thumb)).is_ok();
            meta.clips.push(Clip { id: cid, file, thumb: thumb_ok.then_some(thumb), start, end, title, kind, created_at: Utc::now(), drive: None, offloaded: false });
        }
    }
    for c in previous.iter().filter(|c| c.drive.is_some() && !meta.clips.iter().any(|n| n.id == c.id && n.drive == c.drive)) {
        log::warn!("{}: el clip {} cambió; su copia anterior sigue en Drive", meta.id, c.id);
    }
    Ok(())
}

/// Espera a que Fortnite termine de escribir el replay (al volver al lobby) y lo lee.
fn wait_for_replay(app: &AppHandle, shared: &Shared, id: &str, video_start: DateTime<Utc>, duration: f64) -> Option<replay::Replay> {
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    let end = video_start + chrono::Duration::seconds(duration as i64 + 5);
    loop {
        let in_match = shared.in_match.load(Ordering::SeqCst);
        // Fortnite termina de escribir el replay cuando vuelves al lobby.
        let label = if in_match { "Esperando a que vuelvas al lobby (replay)" } else { "Leyendo el replay de Fortnite" };
        shared.set_step(app, id, label, None);
        if let Some(path) = replay::find_for_window(video_start, end) {
            let stable = std::fs::metadata(&path).and_then(|m| m.modified()).map(|t| t.elapsed().unwrap_or_default() > Duration::from_secs(4)).unwrap_or(false);
            if (!in_match && stable) || Instant::now() > deadline {
                return replay::parse_file(&path).map_err(|e| log::warn!("replay {}: {e:#}", path.display())).ok();
            }
        } else if !in_match || Instant::now() > deadline {
            return None;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Re-procesa una partida ya grabada (por ejemplo, si el replay apareció después).
pub fn reprocess(app: &AppHandle, shared: &Shared, id: &str) -> anyhow::Result<MatchMeta> {
    let settings = shared.settings.read().unwrap().clone();
    let lib = shared.library();
    let ffmpeg = ffmpeg_path(app, &settings);
    let mut meta = lib.load(id)?;
    if meta.offloaded {
        anyhow::bail!("El video está solo en Drive: descárgalo para volver a analizarlo");
    }
    let rep = match &meta.replay {
        Some(p) => replay::parse_file(std::path::Path::new(p)).ok(),
        None => replay::find_for_window(meta.created_at, meta.created_at + chrono::Duration::seconds(meta.duration_s as i64)).and_then(|p| replay::parse_file(&p).ok()),
    };
    let account_id = meta
        .account_id
        .clone()
        .or_else(|| shared.status.lock().unwrap().account_id.clone())
        .or_else(crate::fortnite::log::last_account_id);
    let ctx = MatchContext { account_id: account_id.clone(), map: meta.map.clone(), playlist: meta.playlist.clone(), ..Default::default() };
    meta.replay = rep.as_ref().map(|r| r.path.display().to_string());
    meta.account_id = account_id;
    let start = meta.created_at;
    // El log de Fortnite ya no está disponible: se conservan los eventos que salieron de él.
    let mut keep: Vec<_> = meta.events.iter().filter(|e| matches!(e.kind.as_str(), "bus" | "bookmark" | "victory")).cloned().collect();
    if rep.is_none() {
        keep.extend(meta.events.iter().filter(|e| matches!(e.kind.as_str(), "highlight" | "death")).cloned());
    }
    let step = |label: &str, pct: Option<f32>| shared.set_step(app, id, label, pct);
    let result = analyze(&ffmpeg, &lib, &mut meta, &ctx, rep.as_ref(), start, &settings, keep, &step);
    let snapshot = {
        let mut st = shared.status.lock().unwrap();
        st.steps.remove(id);
        st.clone()
    };
    let _ = app.emit("status", snapshot);
    result?;
    lib.save(&meta)?;
    if let Err(e) = crate::stats::record(&crate::stats::path_for(&shared.settings_path), &meta) {
        log::warn!("no se pudieron guardar los totales: {e:#}");
    }
    Ok(meta)
}
