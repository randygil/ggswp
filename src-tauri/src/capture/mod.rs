pub mod audio;
pub mod video;
pub mod win;

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};

use audio::{AudioCapture, Source};
use video::{CREATE_NO_WINDOW, VideoCapture, VideoOptions};

#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub ffmpeg: PathBuf,
    pub video: VideoOptions,
    pub system_audio: bool,
    pub microphone: bool,
    pub mic_volume: f32,
    /// Proceso del juego: su audio se graba en una pista aparte del resto del sistema.
    pub game_pid: Option<u32>,
}

/// Pistas de audio de una grabación. `mix` es todo junto (lo que reproduce cualquier
/// reproductor); las demás son las fuentes por separado, para elegirlas al exportar.
pub const TRACK_MIX: &str = "mix";
pub const TRACK_GAME: &str = "game";
pub const TRACK_OTHER: &str = "other";
pub const TRACK_SYSTEM: &str = "system";
pub const TRACK_MIC: &str = "mic";

pub struct Session {
    pub dir: PathBuf,
    t0: u64,
    wall0: DateTime<Utc>,
    video: VideoCapture,
    audio: Vec<(&'static str, AudioCapture)>,
    cfg: SessionConfig,
}

#[derive(Debug, Clone)]
pub struct RawTrack {
    /// game | other | system | mic
    pub kind: &'static str,
    pub wav: PathBuf,
}

/// Archivos crudos de una grabación terminada, antes de unirlos en el MP4.
#[derive(Debug, Clone)]
pub struct RawRecording {
    pub dir: PathBuf,
    pub video: PathBuf,
    pub tracks: Vec<RawTrack>,
    /// Hora real (UTC) del primer frame de video.
    pub video_started_at: DateTime<Utc>,
    /// Cuánto audio sobra al inicio respecto del video, en segundos.
    pub audio_lead_s: f64,
    pub mic_volume: f32,
}

/// Audio del sistema: el juego y el resto por separado si se puede; si no, todo junto.
fn start_system_audio(dir: &Path, game_pid: Option<u32>, t0: u64) -> Vec<(&'static str, AudioCapture)> {
    if let Some(pid) = game_pid {
        let game = AudioCapture::start(Source::Process(pid), dir.join("game.wav"), t0);
        let other = AudioCapture::start(Source::AllBut(pid), dir.join("other.wav"), t0);
        match (game, other) {
            (Ok(g), Ok(o)) => return vec![(TRACK_GAME, g), (TRACK_OTHER, o)],
            (g, o) => {
                for r in [g, o] {
                    match r {
                        Ok(a) => drop(a.stop()),
                        Err(e) => log::warn!("sin audio separado del juego: {e:#}; se graba todo junto"),
                    }
                }
            }
        }
    }
    match AudioCapture::start(Source::SystemLoopback, dir.join("system.wav"), t0) {
        Ok(a) => vec![(TRACK_SYSTEM, a)],
        Err(e) => {
            log::error!("sin audio del sistema: {e:#}");
            Vec::new()
        }
    }
}

impl Session {
    pub fn start(dir: PathBuf, cfg: SessionConfig) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let t0 = win::qpc_100ns();
        let wall0 = Utc::now();
        // El audio arranca antes que el video, así el desfase siempre es >= 0.
        let mut audio = if cfg.system_audio { start_system_audio(&dir, cfg.game_pid, t0) } else { Vec::new() };
        if cfg.microphone {
            match AudioCapture::start(Source::Microphone, dir.join("mic.wav"), t0) {
                Ok(a) => audio.push((TRACK_MIC, a)),
                Err(e) => log::error!("sin micrófono: {e:#}"),
            }
        }
        let video = match VideoCapture::start(&cfg.ffmpeg, &cfg.video, dir.join("video.mkv")) {
            Ok(v) => v,
            Err(e) => {
                for (_, a) in audio {
                    let _ = a.stop();
                }
                return Err(e);
            }
        };
        Ok(Self { dir, t0, wall0, video, audio, cfg })
    }

    pub fn is_alive(&mut self) -> bool {
        self.video.is_alive()
    }

    pub fn started_at(&self) -> DateTime<Utc> {
        self.wall0
    }

    pub fn stop(self) -> Result<RawRecording> {
        let video_path = self.video.path.clone();
        let video_t0 = self.video.stop();
        let tracks = self
            .audio
            .into_iter()
            .filter_map(|(kind, a)| {
                let wav = a.path.clone();
                a.stop().map_err(|e| log::error!("audio {kind}: {e:#}")).ok().map(|_| RawTrack { kind, wav })
            })
            .collect();
        let video_t0 = video_t0.context("video")?;
        let lead_100ns = video_t0.saturating_sub(self.t0);
        Ok(RawRecording {
            dir: self.dir,
            video: video_path,
            tracks,
            video_started_at: self.wall0 + chrono::Duration::microseconds((lead_100ns / 10) as i64),
            audio_lead_s: lead_100ns as f64 / 1e7,
            mic_volume: self.cfg.mic_volume,
        })
    }
}

/// Pistas del MP4 final, en orden. Con una sola fuente hay una pista; con varias, la
/// primera es la mezcla de todas y después va cada fuente.
pub fn track_names(raw: &RawRecording) -> Vec<&'static str> {
    match raw.tracks.as_slice() {
        [] => vec![],
        [one] => vec![one.kind],
        many => std::iter::once(TRACK_MIX).chain(many.iter().map(|t| t.kind)).collect(),
    }
}

/// Une video + audio(s) en un MP4 reproducible en cualquier lado (faststart para streaming).
pub fn finalize(ffmpeg: &Path, raw: &RawRecording, out: &Path) -> Result<()> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&raw.video);
    let lead = format!("{:.4}", raw.audio_lead_s.max(0.0));
    for t in &raw.tracks {
        cmd.args(["-ss", &lead, "-i"]).arg(&t.wav);
    }
    cmd.args(["-map", "0:v", "-c:v", "copy"]);
    let n = raw.tracks.len();
    if n == 1 {
        cmd.args(["-map", "1:a"]);
    } else if n > 1 {
        // Cada fuente va a la mezcla y a su propia pista.
        let mut fc = String::new();
        for (i, t) in raw.tracks.iter().enumerate() {
            let vol = if t.kind == TRACK_MIC { format!("volume={:.2},", raw.mic_volume) } else { String::new() };
            fc.push_str(&format!("[{}:a]{vol}asplit=2[m{i}][t{i}];", i + 1));
        }
        let mixed: String = (0..n).map(|i| format!("[m{i}]")).collect();
        fc.push_str(&format!("{mixed}amix=inputs={n}:duration=first:normalize=0[mix]"));
        cmd.args(["-filter_complex", &fc, "-map", "[mix]"]);
        for i in 0..n {
            cmd.args(["-map", &format!("[t{i}]")]);
        }
    }
    if n > 0 {
        cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
        // El nombre de cada pista viaja en el archivo: así se reconocen al exportar.
        for (i, name) in track_names(raw).iter().enumerate() {
            cmd.args([&format!("-metadata:s:a:{i}"), &format!("handler_name={name}")]);
        }
        cmd.args(["-disposition:a:0", "default"]);
    }
    cmd.args(["-movflags", "+faststart"]).arg(out).creation_flags(CREATE_NO_WINDOW);
    let o = cmd.output().context("ejecutando ffmpeg")?;
    if !o.status.success() {
        bail!("ffmpeg mux: {}", String::from_utf8_lossy(&o.stderr));
    }
    Ok(())
}

pub fn remove_raw(raw: &RawRecording) {
    for p in std::iter::once(&raw.video).chain(raw.tracks.iter().map(|t| &t.wav)) {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(kinds: &[&'static str]) -> RawRecording {
        RawRecording {
            dir: PathBuf::new(),
            video: PathBuf::new(),
            tracks: kinds.iter().map(|k| RawTrack { kind: k, wav: PathBuf::from(format!("{k}.wav")) }).collect(),
            video_started_at: Utc::now(),
            audio_lead_s: 0.0,
            mic_volume: 1.0,
        }
    }

    #[test]
    fn several_sources_get_a_mix_and_their_own_tracks() {
        assert!(track_names(&raw(&[])).is_empty());
        assert_eq!(track_names(&raw(&[TRACK_SYSTEM])), [TRACK_SYSTEM]);
        assert_eq!(track_names(&raw(&[TRACK_GAME, TRACK_OTHER, TRACK_MIC])), [TRACK_MIX, TRACK_GAME, TRACK_OTHER, TRACK_MIC]);
    }

    /// Graba 3 s separando el audio de un proceso que hace sonar un tono (suena por los
    /// parlantes): `cargo test --lib separates_one_process -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn separates_one_process() {
        let dir = std::env::temp_dir().join("ggswp-test-audio");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Un tono suave de 5 s, reproducido por un proceso aparte.
        let tone = dir.join("tone.wav");
        let spec = hound::WavSpec { channels: 2, sample_rate: 48_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut w = hound::WavWriter::create(&tone, spec).unwrap();
        for i in 0..48_000 * 5 {
            let v = ((i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 1500.0) as i16;
            w.write_sample(v).unwrap();
            w.write_sample(v).unwrap();
        }
        w.finalize().unwrap();
        let script = format!("(New-Object System.Media.SoundPlayer '{}').PlaySync()", tone.display());
        let mut player = Command::new("powershell").args(["-NoProfile", "-Command", &script]).creation_flags(CREATE_NO_WINDOW).spawn().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        let cfg = SessionConfig {
            ffmpeg: "ffmpeg".into(),
            video: VideoOptions { output_index: 0, fps: 60, encoder: "h264_nvenc".into(), cq: 30, max_bitrate_mbps: 10 },
            system_audio: true,
            microphone: false,
            mic_volume: 1.0,
            game_pid: Some(player.id()),
        };
        let s = Session::start(dir.clone(), cfg.clone()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(3));
        let raw = s.stop().unwrap();
        let _ = player.kill();
        assert_eq!(track_names(&raw), [TRACK_MIX, TRACK_GAME, TRACK_OTHER]);
        let samples = |p: &Path| -> Vec<f64> { hound::WavReader::open(p).unwrap().samples::<i16>().map(|s| s.unwrap() as f64).collect() };
        let game = samples(&raw.tracks[0].wav);
        let peak = game.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        // Solo la parte donde suena (el WAV empieza con silencio hasta el primer paquete).
        let loud: Vec<f64> = game.iter().copied().skip_while(|v| v.abs() < peak / 2.0).collect();
        let rms = (loud.iter().map(|v| v * v).sum::<f64>() / loud.len().max(1) as f64).sqrt();
        let other = samples(&raw.tracks[1].wav).iter().fold(0.0f64, |m, v| m.max(v.abs()));
        println!("juego: pico {peak:.0}, rms/pico {:.2} · resto: pico {other:.0} · lead {:.3}", rms / peak, raw.audio_lead_s);
        assert!(peak > 300.0, "el tono debe estar en la pista del proceso");
        // Un tono puro tiene rms/pico = 0,71: si se colara otro audio, la relación cambiaría.
        assert!((rms / peak - 0.707).abs() < 0.06, "la pista del proceso solo debe tener el tono");
        let out = dir.join("out.mp4");
        finalize(&cfg.ffmpeg, &raw, &out).unwrap();
        let info = Command::new("ffmpeg").args(["-hide_banner", "-i"]).arg(&out).output().unwrap();
        let info = String::from_utf8_lossy(&info.stderr).to_string();
        println!("{info}");
        assert_eq!(info.matches("Audio:").count(), 3);
        assert!(info.contains("handler_name    : game"));
    }

    /// Graba 6 s del monitor principal + audio del sistema: `cargo test capture -- --ignored`
    #[test]
    #[ignore]
    fn record_six_seconds() {
        let dir = std::env::temp_dir().join("ggswp-test");
        let _ = std::fs::remove_dir_all(&dir);
        let cfg = SessionConfig {
            ffmpeg: "ffmpeg".into(),
            video: VideoOptions { output_index: 0, fps: 60, encoder: "h264_nvenc".into(), cq: 19, max_bitrate_mbps: 80 },
            system_audio: true,
            microphone: false,
            mic_volume: 1.0,
            game_pid: None,
        };
        let s = Session::start(dir.clone(), cfg.clone()).unwrap();
        std::thread::sleep(std::time::Duration::from_secs(6));
        let raw = s.stop().unwrap();
        println!("lead={:.3}s video_start={}", raw.audio_lead_s, raw.video_started_at);
        let out = dir.join("out.mp4");
        finalize(&cfg.ffmpeg, &raw, &out).unwrap();
        println!("{}", out.display());
        assert!(out.metadata().unwrap().len() > 10_000);
    }
}
