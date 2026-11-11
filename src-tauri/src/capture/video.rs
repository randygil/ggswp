//! Captura de pantalla: ffmpeg con `ddagrab` (Desktop Duplication, los frames se quedan en
//! la GPU) codificado por la tarjeta gráfica: NVENC (NVIDIA), AMF (AMD) o Quick Sync (Intel);
//! sin ninguna, por software. Se graba exactamente lo que se ve en el monitor del juego.
//!
//! Para sincronizar con el audio se estima el instante QPC del primer frame a partir del
//! reporte `-progress` de ffmpeg: `t0 ≈ min(ahora - out_time)`.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
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

/// Quién codifica: de eso dependen las opciones de ffmpeg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Nvenc,
    Amf,
    Qsv,
    Software,
}

pub fn family(encoder: &str) -> Family {
    match encoder.rsplit('_').next() {
        Some("nvenc") => Family::Nvenc,
        Some("amf") => Family::Amf,
        Some("qsv") => Family::Qsv,
        _ => Family::Software,
    }
}

/// Codificadores que la app sabe usar, del preferido al último recurso.
pub const ENCODERS: [&str; 10] = ["h264_nvenc", "hevc_nvenc", "av1_nvenc", "h264_amf", "hevc_amf", "av1_amf", "h264_qsv", "hevc_qsv", "av1_qsv", "libx264"];

/// Codificador de la misma familia para un códec (`h264`, `hevc` o `av1`).
pub fn encoder_for(fam: Family, codec: &str) -> String {
    match (fam, codec) {
        (Family::Nvenc, c) => format!("{c}_nvenc"),
        (Family::Amf, c) => format!("{c}_amf"),
        (Family::Qsv, c) => format!("{c}_qsv"),
        (Family::Software, "hevc") => "libx265".into(),
        (Family::Software, "av1") => "libsvtav1".into(),
        (Family::Software, _) => "libx264".into(),
    }
}

fn codec_of(encoder: &str) -> &str {
    match encoder {
        e if e.starts_with("hevc") || e == "libx265" => "hevc",
        e if e.starts_with("av1") || e == "libsvtav1" => "av1",
        _ => "h264",
    }
}

/// Codificadores que funcionan en esta PC (se prueban una vez, al iniciar).
static AVAILABLE: OnceLock<Vec<String>> = OnceLock::new();

/// Prueba cada codificador con un video de un instante. Tarda unos segundos: va en un hilo.
pub fn detect_encoders(ffmpeg: &Path) -> &'static [String] {
    AVAILABLE.get_or_init(|| {
        let works = |enc: &str| {
            Command::new(ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-f", "lavfi", "-i", "testsrc2=s=640x360:r=30", "-t", "0.2", "-c:v", enc, "-f", "null", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .is_ok_and(|s| s.success())
        };
        let found: Vec<String> = ENCODERS.iter().filter(|e| works(e)).map(|e| e.to_string()).collect();
        log::info!("codificadores disponibles: {}", found.join(", "));
        found
    })
}

pub fn available_encoders() -> Option<&'static [String]> {
    AVAILABLE.get().map(|v| v.as_slice())
}

/// El codificador elegido si funciona en esta PC; si no, el mismo códec en otra tarjeta, o
/// H.264, o por software. Antes de terminar la detección se respeta lo elegido.
pub fn pick_encoder(wanted: &str, available: Option<&[String]>) -> String {
    let Some(av) = available.filter(|a| !a.is_empty()) else { return wanted.to_string() };
    if av.iter().any(|e| e == wanted) {
        return wanted.to_string();
    }
    let same_codec = av.iter().find(|e| codec_of(e) == codec_of(wanted) && family(e) != Family::Software);
    same_codec.or_else(|| av.iter().find(|e| codec_of(e) == "h264")).unwrap_or(&av[0]).clone()
}

pub fn encoder_args(o: &VideoOptions) -> Vec<String> {
    let maxrate = format!("{}M", o.max_bitrate_mbps);
    let bufsize = format!("{}M", o.max_bitrate_mbps * 2);
    let cq = o.cq.to_string();
    let mut a: Vec<String> = vec!["-c:v".into(), o.encoder.clone()];
    let rate: Vec<&str> = match family(&o.encoder) {
        Family::Nvenc => vec!["-preset", "p5", "-tune", "hq", "-rc", "vbr", "-cq", &cq, "-b:v", "0", "-maxrate", &maxrate, "-bufsize", &bufsize],
        Family::Amf => vec!["-usage", "transcoding", "-quality", "quality", "-rc", "cqp", "-qp_i", &cq, "-qp_p", &cq, "-qp_b", &cq],
        Family::Qsv => vec!["-preset", "medium", "-global_quality", &cq, "-look_ahead", "0", "-maxrate", &maxrate, "-bufsize", &bufsize],
        Family::Software => vec!["-preset", "veryfast", "-crf", &cq, "-maxrate", &maxrate, "-bufsize", &bufsize, "-pix_fmt", "yuv420p"],
    };
    a.extend(rate.into_iter().map(String::from));
    // Sin B-frames: menos latencia en el pipeline y medición de t0 más exacta.
    a.extend(["-bf", "0", "-g", &(o.fps * 2).to_string()].map(String::from));
    if codec_of(&o.encoder) == "h264" && family(&o.encoder) != Family::Amf {
        a.extend(["-profile:v", "high"].map(String::from));
    }
    a
}

/// Entrada de ffmpeg para grabar el monitor. Los cuadros salen de la GPU como texturas
/// D3D11: NVENC y AMF las toman directo; Quick Sync las mapea y el software las descarga.
pub fn capture_input(o: &VideoOptions) -> String {
    let grab = format!("ddagrab=output_idx={}:framerate={}:draw_mouse=0", o.output_index, o.fps);
    match family(&o.encoder) {
        Family::Nvenc | Family::Amf => grab,
        Family::Qsv => format!("{grab},hwmap=derive_device=qsv,format=qsv"),
        Family::Software => format!("{grab},hwdownload,format=bgra"),
    }
}

impl VideoCapture {
    pub fn start(ffmpeg: &Path, opts: &VideoOptions, path: PathBuf) -> Result<Self> {
        let input = capture_input(opts);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(encoder: &str) -> VideoOptions {
        VideoOptions { output_index: 1, fps: 60, encoder: encoder.into(), cq: 19, max_bitrate_mbps: 80 }
    }

    #[test]
    fn each_card_gets_its_own_options() {
        assert_eq!(family("hevc_nvenc"), Family::Nvenc);
        assert_eq!(family("h264_amf"), Family::Amf);
        assert_eq!(family("av1_qsv"), Family::Qsv);
        assert_eq!(family("libx264"), Family::Software);
        let has = |enc: &str, flag: &str| encoder_args(&opts(enc)).iter().any(|a| a == flag);
        assert!(has("h264_nvenc", "-cq") && has("h264_nvenc", "-profile:v"));
        assert!(has("hevc_amf", "-qp_i") && !has("hevc_amf", "-cq") && !has("h264_amf", "-profile:v"));
        assert!(has("h264_qsv", "-global_quality"));
        assert!(has("libx264", "-crf"));
        assert_eq!(capture_input(&opts("h264_amf")), "ddagrab=output_idx=1:framerate=60:draw_mouse=0");
        assert!(capture_input(&opts("h264_qsv")).ends_with(",hwmap=derive_device=qsv,format=qsv"));
        assert!(capture_input(&opts("libx264")).ends_with(",hwdownload,format=bgra"));
        assert_eq!(encoder_for(Family::Amf, "hevc"), "hevc_amf");
        assert_eq!(encoder_for(Family::Software, "h264"), "libx264");
    }

    #[test]
    fn falls_back_to_an_encoder_that_works() {
        let av = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let amd = av(&["h264_amf", "hevc_amf", "libx264"]);
        assert_eq!(pick_encoder("h264_nvenc", Some(&amd)), "h264_amf");
        assert_eq!(pick_encoder("hevc_nvenc", Some(&amd)), "hevc_amf");
        assert_eq!(pick_encoder("av1_nvenc", Some(&amd)), "h264_amf", "sin AV1, H.264");
        assert_eq!(pick_encoder("hevc_amf", Some(&amd)), "hevc_amf");
        assert_eq!(pick_encoder("h264_nvenc", Some(&av(&["libx264"]))), "libx264");
        // Sin detección todavía, se respeta lo elegido.
        assert_eq!(pick_encoder("h264_nvenc", None), "h264_nvenc");
        assert_eq!(pick_encoder("h264_nvenc", Some(&[])), "h264_nvenc");
    }
}
