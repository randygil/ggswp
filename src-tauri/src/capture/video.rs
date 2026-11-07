//! Captura de pantalla: ffmpeg con `ddagrab` (Desktop Duplication, los frames se quedan en
//! la GPU) codificado por NVENC. Se graba exactamente lo que se ve en el monitor de Fortnite.
//!
//! Para sincronizar con el audio se estima el instante QPC del primer frame a partir del
//! reporte `-progress` de ffmpeg: `t0 ≈ min(ahora - out_time)`.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};

use super::win::qpc_100ns;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone)]
pub struct VideoOptions {
    pub output_index: u32,
    pub fps: u32,
    pub encoder: String,
    pub cq: u32,
    pub max_bitrate_mbps: u32,
}

pub struct VideoCapture {
    child: Child,
    stdin: Option<ChildStdin>,
    t0: Arc<Mutex<Option<u64>>>,
    progress: Option<JoinHandle<()>>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    pub path: PathBuf,
}

pub fn encoder_args(o: &VideoOptions) -> Vec<String> {
    let maxrate = format!("{}M", o.max_bitrate_mbps);
    let bufsize = format!("{}M", o.max_bitrate_mbps * 2);
    let mut a: Vec<String> = vec!["-c:v".into(), o.encoder.clone()];
    a.extend(
        ["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", &o.cq.to_string(), "-b:v", "0", "-maxrate", &maxrate, "-bufsize", &bufsize]
            .map(String::from),
    );
    // Sin B-frames: menos latencia en el pipeline y medición de t0 más exacta.
    a.extend(["-bf", "0", "-g", &(o.fps * 2).to_string()].map(String::from));
    if o.encoder == "h264_nvenc" {
        a.extend(["-profile:v", "high"].map(String::from));
    }
    a
}

impl VideoCapture {
    pub fn start(ffmpeg: &Path, opts: &VideoOptions, path: PathBuf) -> Result<Self> {
        let input = format!("ddagrab=output_idx={}:framerate={}:draw_mouse=0", opts.output_index, opts.fps);
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-hide_banner", "-nostats", "-loglevel", "error", "-progress", "pipe:1", "-stats_period", "0.1"])
            .args(["-f", "lavfi", "-i", &input])
            .args(encoder_args(opts))
            .arg("-y")
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .creation_flags(CREATE_NO_WINDOW);
        let mut child = cmd.spawn().with_context(|| format!("no se pudo ejecutar {}", ffmpeg.display()))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        let t0 = Arc::new(Mutex::new(None::<u64>));
        let progress = {
            let t0 = t0.clone();
            std::thread::spawn(move || {
                let mut samples = 0;
                for line in BufReader::new(stdout).lines().map_while(|l| l.ok()) {
                    let Some(v) = line.strip_prefix("out_time_us=") else { continue };
                    let Ok(us) = v.trim().parse::<i64>() else { continue };
                    if us <= 0 || samples > 300 {
                        continue;
                    }
                    samples += 1;
                    let est = qpc_100ns().saturating_sub(us as u64 * 10);
                    let mut g = t0.lock().unwrap();
                    *g = Some(g.map_or(est, |cur| cur.min(est)));
                }
            })
        };
        let stderr_tail = Arc::new(Mutex::new(Vec::new()));
        {
            let tail = stderr_tail.clone();
            std::thread::spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
                    log::warn!("ffmpeg: {line}");
                    let mut t = tail.lock().unwrap();
                    t.push(line);
                    if t.len() > 20 {
                        t.remove(0);
                    }
                }
            });
        }

        let mut cap = Self { child, stdin, t0, progress: Some(progress), stderr_tail, path };
        // Confirmamos que arrancó (ddagrab falla rápido si el índice de monitor no existe).
        for _ in 0..50 {
            std::thread::sleep(Duration::from_millis(100));
            if cap.t0.lock().unwrap().is_some() {
                return Ok(cap);
            }
            if let Ok(Some(status)) = cap.child.try_wait() {
                let err = cap.stderr_tail.lock().unwrap().join("\n");
                bail!("ffmpeg terminó al iniciar ({status}): {err}");
            }
        }
        let _ = cap.child.kill();
        bail!("ffmpeg no empezó a grabar en 5 s")
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Cierra ffmpeg de forma ordenada (`q`) para que el archivo quede bien escrito.
    /// Devuelve el instante QPC estimado del primer frame.
    pub fn stop(mut self) -> Result<u64> {
        if let Some(mut s) = self.stdin.take() {
            let _ = s.write_all(b"q");
            let _ = s.flush();
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            match self.child.try_wait()? {
                Some(_) => break,
                None if std::time::Instant::now() > deadline => {
                    let _ = self.child.kill();
                    break;
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        }
        if let Some(h) = self.progress.take() {
            let _ = h.join();
        }
        let t0 = *self.t0.lock().unwrap();
        t0.ok_or_else(|| anyhow!("no se obtuvo el inicio del video"))
    }
}
