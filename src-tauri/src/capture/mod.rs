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
}

pub struct Session {
    pub dir: PathBuf,
    t0: u64,
    wall0: DateTime<Utc>,
    video: VideoCapture,
    sys: Option<AudioCapture>,
    mic: Option<AudioCapture>,
    cfg: SessionConfig,
}

/// Archivos crudos de una grabación terminada, antes de unirlos en el MP4.
#[derive(Debug, Clone)]
pub struct RawRecording {
    pub dir: PathBuf,
    pub video: PathBuf,
    pub system_wav: Option<PathBuf>,
    pub mic_wav: Option<PathBuf>,
    /// Hora real (UTC) del primer frame de video.
    pub video_started_at: DateTime<Utc>,
    /// Cuánto audio sobra al inicio respecto del video, en segundos.
    pub audio_lead_s: f64,
    pub mic_volume: f32,
}

impl Session {
    pub fn start(dir: PathBuf, cfg: SessionConfig) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let t0 = win::qpc_100ns();
        let wall0 = Utc::now();
        // El audio arranca antes que el video, así el desfase siempre es >= 0.
        let sys = if cfg.system_audio {
            match AudioCapture::start(Source::SystemLoopback, dir.join("system.wav"), t0) {
                Ok(a) => Some(a),
                Err(e) => {
                    log::error!("sin audio del sistema: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        let mic = if cfg.microphone {
            match AudioCapture::start(Source::Microphone, dir.join("mic.wav"), t0) {
                Ok(a) => Some(a),
                Err(e) => {
                    log::error!("sin micrófono: {e:#}");
                    None
                }
            }
        } else {
            None
        };
        let video = match VideoCapture::start(&cfg.ffmpeg, &cfg.video, dir.join("video.mkv")) {
            Ok(v) => v,
            Err(e) => {
                for a in [sys, mic].into_iter().flatten() {
                    let _ = a.stop();
                }
                return Err(e);
            }
        };
        Ok(Self { dir, t0, wall0, video, sys, mic, cfg })
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
        let sys = self.sys.map(|a| {
            let p = a.path.clone();
            a.stop().map(|_| p)
        });
        let mic = self.mic.map(|a| {
            let p = a.path.clone();
            a.stop().map(|_| p)
        });
        let video_t0 = video_t0.context("video")?;
        let lead_100ns = video_t0.saturating_sub(self.t0);
        Ok(RawRecording {
            dir: self.dir,
            video: video_path,
            system_wav: sys.and_then(|r| r.map_err(|e| log::error!("audio sistema: {e:#}")).ok()),
            mic_wav: mic.and_then(|r| r.map_err(|e| log::error!("micrófono: {e:#}")).ok()),
            video_started_at: self.wall0 + chrono::Duration::microseconds((lead_100ns / 10) as i64),
            audio_lead_s: lead_100ns as f64 / 1e7,
            mic_volume: self.cfg.mic_volume,
        })
    }
}

/// Une video + audio(s) en un MP4 reproducible en cualquier lado (faststart para streaming).
pub fn finalize(ffmpeg: &Path, raw: &RawRecording, out: &Path) -> Result<()> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(["-hide_banner", "-loglevel", "error", "-y", "-i"]).arg(&raw.video);
    let lead = format!("{:.4}", raw.audio_lead_s.max(0.0));
    let mut audio_inputs = 0;
    for wav in [&raw.system_wav, &raw.mic_wav].into_iter().flatten() {
        cmd.args(["-ss", &lead, "-i"]).arg(wav);
        audio_inputs += 1;
    }
    cmd.args(["-map", "0:v", "-c:v", "copy"]);
    match (audio_inputs, raw.system_wav.is_some()) {
        (2, _) => {
            let fc = format!("[2:a]volume={:.2}[m];[1:a][m]amix=inputs=2:duration=first:normalize=0[a]", raw.mic_volume);
            cmd.args(["-filter_complex", &fc, "-map", "[a]"]);
        }
        (1, _) => {
            cmd.args(["-map", "1:a"]);
        }
        _ => {}
    }
    if audio_inputs > 0 {
        cmd.args(["-c:a", "aac", "-b:a", "192k", "-shortest"]);
    }
    cmd.args(["-movflags", "+faststart"]).arg(out).creation_flags(CREATE_NO_WINDOW);
    let o = cmd.output().context("ejecutando ffmpeg")?;
    if !o.status.success() {
        bail!("ffmpeg mux: {}", String::from_utf8_lossy(&o.stderr));
    }
    Ok(())
}

pub fn remove_raw(raw: &RawRecording) {
    for p in [Some(&raw.video), raw.system_wav.as_ref(), raw.mic_wav.as_ref()].into_iter().flatten() {
        let _ = std::fs::remove_file(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
