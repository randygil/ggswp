//! Cofres abiertos. Ni el log ni los eventos del replay los registran, pero al abrir uno
//! Fortnite muestra el aviso de experiencia "+600 PE / COFRE" a la derecha del centro de la
//! pantalla. Se recorre el video grabado a 2 cuadros por segundo, se recorta esa zona y se
//! lee con el OCR que trae Windows.

use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use regex::Regex;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;

use crate::capture::video::CREATE_NO_WINDOW;

const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x4000;
const FPS: f64 = 2.0;
/// El aviso aparece ~1 s después de abrir el cofre.
const TOAST_DELAY_S: f64 = 1.0;
/// Un mismo aviso dura ~2 s en pantalla; más separación = otro cofre.
const SAME_TOAST_GAP_S: f64 = 2.0;

/// Si el texto leído en un cuadro es el aviso de cofre, devuelve cuántos cofres indica
/// ("COFRE X2" = 2). Se exige la línea exacta en mayúsculas para no confundirlo con textos
/// como la misión "Abre cofres raros" o cualquier ventana que aparezca en la grabación.
pub fn chest_mark(lines: &[String]) -> Option<u32> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(?:COFRE|COFRE ABIERTO|CHEST|CHEST SEARCHED)(?:\s*[X×]\s*(\d{1,2}))?$").unwrap());
    lines.iter().find_map(|l| {
        let c = RE.captures(l.trim())?;
        Some(c.get(1).and_then(|m| m.as_str().parse().ok()).unwrap_or(1))
    })
}

/// Convierte los cuadros con aviso `(segundo, multiplicador)` en el momento de cada cofre.
pub fn group(hits: &[(f64, u32)]) -> Vec<f64> {
    let mut out = Vec::new();
    let mut last: Option<f64> = None;
    let mut shown = 0;
    for &(t, mult) in hits {
        if last.is_none_or(|l| t - l > SAME_TOAST_GAP_S) {
            shown = 0;
        }
        // Si el aviso pasa de "COFRE" a "COFRE X2", se abrió otro cofre.
        while shown < mult.max(1) {
            out.push((t - TOAST_DELAY_S).max(0.0));
            shown += 1;
        }
        last = Some(t);
    }
    out
}

struct Ocr {
    engine: OcrEngine,
}

impl Ocr {
    fn new() -> Result<Self> {
        let engine = OcrEngine::TryCreateFromUserProfileLanguages().context("Windows no tiene OCR para el idioma del perfil")?;
        Ok(Self { engine })
    }

    fn lines(&self, gray: &[u8], w: u32, h: u32) -> Result<Vec<String>> {
        let writer = DataWriter::new()?;
        writer.WriteBytes(gray)?;
        let buf = writer.DetachBuffer()?;
        let bmp = SoftwareBitmap::CreateCopyFromBuffer(&buf, BitmapPixelFormat::Gray8, w as i32, h as i32)?;
        let bmp = SoftwareBitmap::Convert(&bmp, BitmapPixelFormat::Bgra8)?;
        let res = self.engine.RecognizeAsync(&bmp)?.join()?;
        let mut out = Vec::new();
        for line in res.Lines()? {
            out.push(line.Text()?.to_string());
        }
        Ok(out)
    }
}

/// Recorre el video y devuelve el segundo en que se abrió cada cofre.
/// `progress(segundo)` se llama con el segundo de video ya revisado.
pub fn scan(ffmpeg: &Path, video: &Path, width: u32, height: u32, progress: &mut dyn FnMut(f64)) -> Result<Vec<f64>> {
    if width < 640 || height < 360 {
        bail!("resolución demasiado baja para leer los avisos");
    }
    let even = |v: f64| (v as u32) & !1;
    let (cw, ch) = (even(width as f64 * 0.35), even(height as f64 * 0.25));
    let (cx, cy) = (even(width as f64 * 0.5), even(height as f64 * 0.2));
    // Misma escala con la que se probó en 2560x1440 (x1,5): el texto queda de ~30 px.
    let s = 1.5 * 1440.0 / height as f64;
    let (ow, oh) = (even(cw as f64 * s), even(ch as f64 * s));
    let ocr = Ocr::new()?;

    let mut child = Command::new(ffmpeg)
        .args(["-hide_banner", "-v", "error", "-hwaccel", "auto", "-i"])
        .arg(video)
        .args(["-an", "-vf", &format!("fps={FPS},crop={cw}:{ch}:{cx}:{cy},scale={ow}:{oh},format=gray"), "-f", "rawvideo", "pipe:1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS)
        .spawn()
        .context("ejecutando ffmpeg")?;
    let mut stdout = child.stdout.take().context("sin salida de ffmpeg")?;
    let mut frame = vec![0u8; (ow * oh) as usize];
    let mut hits = Vec::new();
    let mut i = 0u64;
    while stdout.read_exact(&mut frame).is_ok() {
        let t = i as f64 / FPS;
        i += 1;
        if i % 10 == 0 {
            progress(t);
        }
        match ocr.lines(&frame, ow, oh) {
            Ok(lines) => {
                if let Some(mult) = chest_mark(&lines) {
                    hits.push((t, mult));
                }
            }
            Err(e) => log::debug!("OCR en {t} s: {e}"),
        }
    }
    let _ = child.wait();
    if i == 0 {
        bail!("ffmpeg no entregó cuadros");
    }
    Ok(group(&hits))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn reads_the_xp_toast_only() {
        assert_eq!(chest_mark(&l(&["+ 600 PE", "COFRE"])), Some(1));
        assert_eq!(chest_mark(&l(&["RECOGER", "+ 600 PE", "COFRE", "RARO"])), Some(1));
        assert_eq!(chest_mark(&l(&["+ 1200 PE", "COFRE X2"])), Some(2));
        assert_eq!(chest_mark(&l(&["+500 XP", "CHEST"])), Some(1));
        assert_eq!(chest_mark(&l(&["26/50 Abre cofres raros"])), None);
        assert_eq!(chest_mark(&l(&["+ 200 PE", "ESCUDO ROTO"])), None);
        assert_eq!(chest_mark(&l(&["abre el cofre de la izquierda"])), None);
    }

    /// Escanea un video real: `GGSWP_TEST_VIDEO=ruta cargo test scan_real_video -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn scan_real_video() {
        let video = std::env::var("GGSWP_TEST_VIDEO").expect("GGSWP_TEST_VIDEO");
        let started = std::time::Instant::now();
        let times = scan(Path::new("ffmpeg"), Path::new(&video), 2560, 1440, &mut |_| {}).unwrap();
        println!("{} cofres en {:.1} s: {times:?}", times.len(), started.elapsed().as_secs_f64());
    }

    /// Lo que se leyó en una partida real de 3:15 (5 cofres).
    #[test]
    fn groups_frames_of_the_same_toast() {
        let frames = [55.0, 55.5, 56.5, 92.5, 93.0, 93.5, 94.0, 99.5, 100.0, 100.5, 101.0, 101.5, 106.0, 106.5, 107.0, 107.5, 108.0, 135.5, 136.0, 136.5, 137.0];
        let hits: Vec<_> = frames.iter().map(|&t| (t, 1)).collect();
        assert_eq!(group(&hits), vec![54.0, 91.5, 98.5, 105.0, 134.5]);
        // Dos cofres seguidos: el aviso cambia a "COFRE X2" sin desaparecer.
        assert_eq!(group(&[(10.0, 1), (10.5, 1), (11.0, 2), (11.5, 2)]), vec![9.0, 10.0]);
    }
}
