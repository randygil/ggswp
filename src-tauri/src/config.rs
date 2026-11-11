use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StartAt {
    /// Al empezar la partida (isla de calentamiento).
    Warmup,
    /// Al subir al autobús.
    Bus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopAt {
    /// Al volver al lobby (incluye el tiempo de espectador).
    Lobby,
    /// Unos segundos después de que te eliminan o ganas.
    PlayerFinished,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DriveSettings {
    pub client_id: String,
    pub client_secret: String,
    pub refresh_token: Option<String>,
    pub account_email: Option<String>,
    pub folder_id: Option<String>,
    pub auto_upload: bool,
    pub upload_full_match: bool,
    pub upload_clips: bool,
    /// Borra la copia local apenas el archivo queda verificado en Drive (si no, se usa
    /// `Settings::local_days`).
    pub delete_local_after_upload: bool,
    /// Formato de la copia que se sube: "hevc" | "av1" | "h264" | "original".
    #[serde(default = "default_drive_codec")]
    pub codec: String,
}

fn default_drive_codec() -> String {
    "hevc".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Guarda los últimos `clip_seconds` segundos.
    pub save_clip: String,
    /// Marca el momento actual en la línea de tiempo.
    pub bookmark: String,
    pub toggle_recording: String,
    /// Abre el menú de highlights sobre el juego.
    pub highlights_menu: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self { save_clip: "Alt+F8".into(), bookmark: "Alt+F7".into(), toggle_recording: "Alt+F11".into(), highlights_menu: "Alt+Shift+KeyG".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub library_dir: PathBuf,
    pub auto_record: bool,
    pub start_at: StartAt,
    pub stop_at: StopAt,
    pub post_roll_s: u32,
    pub encoder: String,
    pub quality_cq: u32,
    pub max_bitrate_mbps: u32,
    pub fps: u32,
    /// `None` = el monitor donde está Fortnite.
    pub monitor: Option<u32>,
    pub system_audio: bool,
    pub microphone: bool,
    pub mic_volume: f32,
    pub auto_clips: bool,
    pub clip_before_s: f64,
    pub clip_after_s: f64,
    pub clip_deaths: bool,
    /// Formato de los clips que se crean: "share" (1080p liviano, listo para compartir) u
    /// "original" (resolución y calidad de la grabación).
    pub clip_quality: String,
    /// Zonas del HUD y encuadre del modo móvil del editor.
    pub mobile_layout: crate::export::MobileLayout,
    pub ffmpeg_path: Option<PathBuf>,
    pub share_port: u16,
    pub launch_at_startup: bool,
    pub start_minimized: bool,
    pub close_to_tray: bool,
    pub drive: DriveSettings,
    pub hotkeys: Hotkeys,
    pub clip_seconds: u32,
    /// Graba en segundo plano mientras Fortnite está abierto, para poder guardar los
    /// últimos segundos también fuera de una partida.
    pub replay_buffer: bool,
    pub overlay: bool,
    pub overlay_moments: bool,
    /// Las partidas no conservadas se borran del todo (PC y Drive) después de estos días
    /// (0 = nunca).
    pub retention_days: u32,
    /// Lo que ya está en Drive se borra de la PC después de estos días (0 = nunca).
    pub local_days: u32,
    /// Lee los avisos "+PE COFRE" del video para contar los cofres abiertos.
    pub detect_chests: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let videos = std::env::var_os("USERPROFILE").map(|p| PathBuf::from(p).join("Videos")).unwrap_or_else(|| PathBuf::from("."));
        Self {
            library_dir: videos.join("ggswp"),
            auto_record: true,
            start_at: StartAt::Warmup,
            stop_at: StopAt::PlayerFinished,
            post_roll_s: 8,
            encoder: "h264_nvenc".into(),
            quality_cq: 19,
            max_bitrate_mbps: 80,
            fps: 60,
            monitor: None,
            system_audio: true,
            microphone: false,
            mic_volume: 1.0,
            auto_clips: true,
            clip_before_s: 12.0,
            clip_after_s: 4.0,
            clip_deaths: true,
            clip_quality: "share".into(),
            mobile_layout: Default::default(),
            ffmpeg_path: None,
            share_port: 8765,
            launch_at_startup: false,
            start_minimized: false,
            close_to_tray: true,
            drive: DriveSettings { upload_clips: true, codec: default_drive_codec(), ..Default::default() },
            hotkeys: Hotkeys::default(),
            clip_seconds: 30,
            replay_buffer: true,
            overlay: true,
            overlay_moments: true,
            retention_days: 15,
            local_days: 7,
            detect_chests: true,
        }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, path)?;
        Ok(())
    }
}
