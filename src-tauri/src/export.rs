//! Formatos de salida de los clips. La grabación va a ~80 Mbps para no perder nada, pero un
//! clip a esa calidad pesa 10 MB por segundo: los clips se guardan y se comparten en 1080p
//! H.264 (~1 MB por segundo), que es lo que aceptan WhatsApp, Instagram y cualquier teléfono.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::capture::video::{VideoOptions, encoder_args};
use crate::engine::{self, Shared};
use crate::library::{self, Library, MatchStatus, Probe};
use crate::montage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// 1080p H.264 con tope de 8 Mbps.
    Share,
    /// 720p con el bitrate calculado para no pasar de 16 MB.
    Small,
    /// Resolución y calidad de la grabación.
    Original,
}

impl Quality {
    /// Valor de `Settings::clip_quality`.
    pub fn from_setting(s: &str) -> Self {
        if s == "original" { Self::Original } else { Self::Share }
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Share => "share",
            Self::Small => "small",
            Self::Original => "original",
        }
    }
}

const SHARE_MAX_BPS: f64 = 8.0e6;
/// Por debajo de esto (y hasta 1080p) un archivo ya cuenta como liviano.
const LIGHT_BPS: f64 = 9.5e6;
/// Tamaño que busca el formato liviano: deja margen bajo los 16 MB.
const SMALL_TARGET_BYTES: f64 = 15.0e6;
const SMALL_LIMIT_BYTES: u64 = 16_000_000;

pub struct Plan {
    /// Filtros de video (escala y fps), en orden.
    pub filters: Vec<String>,
    pub video: Vec<String>,
    pub audio: Vec<String>,
}

fn strs(a: &[&str]) -> Vec<String> {
    a.iter().map(|s| s.to_string()).collect()
}

/// Escala para que el lado corto no pase de `limit`.
fn scale_to(limit: u32, (w, h): (u32, u32)) -> Option<String> {
    match (w, h) {
        (0, _) | (_, 0) => Some(format!("scale=-2:'min({limit},ih)':flags=lanczos")),
        _ if w.min(h) <= limit => None,
        _ if h <= w => Some(format!("scale=-2:{limit}:flags=lanczos")),
        _ => Some(format!("scale={limit}:-2:flags=lanczos")),
    }
}

/// Bitrate de video del formato liviano para un clip de `duration` segundos.
fn small_video_bps(duration: f64, audio_bps: f64) -> f64 {
    ((SMALL_TARGET_BYTES * 8.0 / duration.max(1.0) - audio_bps) * 0.94).clamp(500e3, 6.0e6)
}

/// Cómo codificar un video de `dims` píxeles y `fps` cuadros que va a durar `duration`.
pub fn plan(q: Quality, dims: (u32, u32), fps: f64, duration: f64, rec: &VideoOptions) -> Plan {
    let h264 = |rate: &[&str], out_fps: f64| {
        let mut v = strs(&["-c:v", "h264_nvenc", "-preset", "p6", "-tune", "hq", "-rc", "vbr"]);
        v.extend(strs(rate));
        v.extend(strs(&["-profile:v", "high", "-pix_fmt", "yuv420p", "-bf", "2", "-g", &format!("{:.0}", out_fps * 2.0)]));
        v
    };
    let known_fps = if fps > 0.0 { fps } else { 60.0 };
    match q {
        Quality::Original => Plan { filters: vec![], video: encoder_args(rec), audio: strs(&["-c:a", "aac", "-b:a", "192k"]) },
        Quality::Share => {
            let mut filters: Vec<String> = scale_to(1080, dims).into_iter().collect();
            let out_fps = if known_fps > 61.0 { 60.0 } else { known_fps };
            if out_fps < known_fps {
                filters.push("fps=60".into());
            }
            let max = format!("{:.0}", SHARE_MAX_BPS);
            let buf = format!("{:.0}", SHARE_MAX_BPS * 2.0);
            Plan { filters, video: h264(&["-cq", "25", "-b:v", "0", "-maxrate", &max, "-bufsize", &buf], out_fps), audio: strs(&["-c:a", "aac", "-b:a", "160k"]) }
        }
        Quality::Small => {
            let audio_bps = if duration > 90.0 { 96e3 } else { 128e3 };
            let bps = small_video_bps(duration, audio_bps);
            let mut filters: Vec<String> = scale_to(720, dims).into_iter().collect();
            // Con poco bitrate, 30 cuadros nítidos se ven mejor que 60 borrosos.
            let out_fps = if bps < 2.5e6 && known_fps > 31.0 { 30.0 } else { known_fps.min(60.0) };
            if out_fps < known_fps {
                filters.push(format!("fps={out_fps:.0}"));
            }
            let (b, max, buf) = (format!("{bps:.0}"), format!("{:.0}", bps * 1.2), format!("{:.0}", bps * 2.0));
            Plan {
                filters,
                video: h264(&["-b:v", &b, "-maxrate", &max, "-bufsize", &buf], out_fps),
                audio: strs(&["-c:a", "aac", "-b:a", &format!("{:.0}", audio_bps)]),
            }
        }
    }
}

// ---------- modo móvil ----------

/// Rectángulo dentro del cuadro de la grabación, en fracciones (0–1) del ancho y del alto.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

/// Composición vertical para teléfono: el juego recortado a 9:16 y, encima, una franja con
/// la salud y las armas, que en la grabación quedan en las esquinas y el recorte dejaría
/// afuera. Las zonas dependen de la escala del HUD de cada jugador: se ajustan en el editor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MobileLayout {
    pub health: Region,
    pub weapons: Region,
    /// Centro horizontal del recorte del juego (0–1).
    pub focus: f64,
    /// La salud arriba y las armas abajo, más grandes, en vez de lado a lado.
    pub two_rows: bool,
    /// Altura de la franja del HUD en el cuadro: 0 = arriba del todo, 1 = abajo del todo.
    pub strip: f64,
}

impl Default for MobileLayout {
    fn default() -> Self {
        // HUD de Fortnite a escala 100 % en 16:9.
        Self { health: Region { x: 0.015, y: 0.872, w: 0.29, h: 0.085 }, weapons: Region { x: 0.685, y: 0.855, w: 0.302, h: 0.092 }, focus: 0.5, two_rows: false, strip: 0.93 }
    }
}

/// Un recorte de la grabación y dónde va en el cuadro de salida (todo en píxeles).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placed {
    /// Recorte en la grabación: ancho, alto, x, y.
    pub crop: (u32, u32, u32, u32),
    pub size: (u32, u32),
    pub pos: (u32, u32),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MobileGeometry {
    /// Recorte 9:16 del juego: ancho, alto, x, y.
    pub game: (u32, u32, u32, u32),
    /// Franja oscura detrás del HUD: y, alto.
    pub band: (u32, u32),
    pub health: Placed,
    pub weapons: Placed,
}

const MOBILE_W: f64 = 1080.0;
const MOBILE_MARGIN: f64 = 24.0;
const MOBILE_GAP: f64 = 16.0;
/// Lo más cerca que llega la franja al borde de arriba o de abajo.
const MOBILE_EDGE: f64 = 40.0;
const MOBILE_H: f64 = 1920.0;
const MOBILE_BAND_PAD: f64 = 14.0;
const MOBILE_MAX_ROW_H: f64 = 220.0;
const MOBILE_TWO_ROWS_W: f64 = 760.0;

fn even(v: f64) -> u32 {
    ((v / 2.0).round() as u32 * 2).max(2)
}

/// Dónde va cada pieza. La misma cuenta está en `src/components/MobileFrame.tsx` para la
/// vista previa del editor: si cambia una, cambia la otra.
pub fn mobile_geometry(l: &MobileLayout, w: u32, h: u32) -> MobileGeometry {
    let (fw, fh) = (w as f64, h as f64);
    let gw = even(fh * 9.0 / 16.0).min(w);
    let gx = (l.focus.clamp(0.0, 1.0) * fw - gw as f64 / 2.0).clamp(0.0, fw - gw as f64);
    let crop = |r: &Region| {
        let cw = even(r.w.clamp(0.02, 1.0) * fw).min(w);
        let ch = even(r.h.clamp(0.02, 1.0) * fh).min(h);
        let cx = (r.x.max(0.0) * fw).min(fw - cw as f64).round() as u32;
        let cy = (r.y.max(0.0) * fh).min(fh - ch as f64).round() as u32;
        (cw, ch, cx, cy)
    };
    let (hc, wc) = (crop(&l.health), crop(&l.weapons));
    let aspect = |c: (u32, u32, u32, u32)| c.0 as f64 / c.1 as f64;
    let (ah, aw) = (aspect(hc), aspect(wc));
    // Tamaños y posición horizontal de cada pieza; `dy` es el desplazamiento dentro de la franja.
    let (hs, hx, ws, wx, wdy, band_h) = if l.two_rows {
        let row = |a: f64| {
            let rh = (MOBILE_TWO_ROWS_W / a).min(MOBILE_MAX_ROW_H);
            (even(rh * a), even(rh))
        };
        let (hs, ws) = (row(ah), row(aw));
        let center = |s: (u32, u32)| ((MOBILE_W - s.0 as f64) / 2.0).round();
        (hs, center(hs), ws, center(ws), hs.1 as f64 + MOBILE_GAP, hs.1 as f64 + MOBILE_GAP + ws.1 as f64)
    } else {
        let rh = ((MOBILE_W - 2.0 * MOBILE_MARGIN - MOBILE_GAP) / (ah + aw)).min(MOBILE_MAX_ROW_H);
        let (hs, ws) = ((even(rh * ah), even(rh)), (even(rh * aw), even(rh)));
        let x0 = ((MOBILE_W - (hs.0 as f64 + MOBILE_GAP + ws.0 as f64)) / 2.0).max(0.0).round();
        (hs, x0, ws, x0 + hs.0 as f64 + MOBILE_GAP, 0.0, hs.1 as f64)
    };
    let y = (MOBILE_EDGE + l.strip.clamp(0.0, 1.0) * (MOBILE_H - 2.0 * MOBILE_EDGE - band_h)).round();
    MobileGeometry {
        game: (gw, h, gx.round() as u32, 0),
        band: ((y - MOBILE_BAND_PAD) as u32, (band_h + 2.0 * MOBILE_BAND_PAD) as u32),
        health: Placed { crop: hc, size: hs, pos: (hx as u32, y as u32) },
        weapons: Placed { crop: wc, size: ws, pos: (wx as u32, (y + wdy) as u32) },
    }
}

/// Grafo de filtros del modo móvil: de `[0:v]` a `[m]`, en 1080x1920.
pub fn mobile_filter(g: &MobileGeometry) -> String {
    let piece = |p: &Placed| format!("crop={}:{}:{}:{},scale={}:{}:flags=lanczos", p.crop.0, p.crop.1, p.crop.2, p.crop.3, p.size.0, p.size.1);
    format!(
        "[0:v]split=3[g][h][w];\
         [g]crop={}:{}:{}:{},scale=1080:1920:flags=lanczos,setsar=1,drawbox=x=0:y={}:w=iw:h={}:color=black@0.45:t=fill[bg];\
         [h]{}[hh];[w]{}[ww];\
         [bg][hh]overlay={}:{}[t];[t][ww]overlay={}:{}[m]",
        g.game.0, g.game.1, g.game.2, g.game.3, g.band.0, g.band.1,
        piece(&g.health), piece(&g.weapons),
        g.health.pos.0, g.health.pos.1, g.weapons.pos.0, g.weapons.pos.1,
    )
}

pub struct Job<'a> {
    pub src: &'a Path,
    /// Tramo a cortar (inicio, duración); `None` = el archivo entero.
    pub cut: Option<(f64, f64)>,
    pub quality: Quality,
    /// Recorte central 9:16 a 1080x1920.
    pub vertical: bool,
    /// Marcador de la partida sobre el video vertical.
    pub badge: Option<(String, String)>,
    /// Modo móvil: recorte 9:16 con la salud y las armas en una franja. Le gana a `vertical`.
    pub mobile: Option<MobileLayout>,
}

/// Corta (exacto al cuadro) y codifica según el formato pedido.
pub fn render(ffmpeg: &Path, job: &Job, rec: &VideoOptions, out: &Path, on_pct: &dyn Fn(f32)) -> Result<()> {
    let src = library::probe(ffmpeg, job.src).unwrap_or_default();
    let duration = job.cut.map(|c| c.1).unwrap_or(src.duration).max(0.1);
    let mut filters = Vec::new();
    let mut dims = (src.width, src.height);
    // El modo móvil arma el cuadro con varias piezas: va como grafo, no como cadena simple.
    let mut graph = None;
    if let Some(layout) = &job.mobile {
        if src.width == 0 || src.width <= src.height {
            bail!("El modo móvil necesita un video horizontal");
        }
        graph = Some(mobile_filter(&mobile_geometry(layout, src.width, src.height)));
        dims = (1080, 1920);
    } else if job.vertical {
        // Sin la fuente de Windows, el video sale igual pero sin marcador.
        let badge = job.badge.as_ref().filter(|_| Path::new(montage::FONT_FILE).exists());
        filters.push(montage::vertical_filter(badge, montage::FONT_FILE));
        dims = (1080, 1920);
    }
    let plan = plan(job.quality, dims, src.fps, duration, rec);
    filters.extend(plan.filters);
    let mut cmd = montage::ffmpeg_cmd(ffmpeg);
    if let Some((start, _)) = job.cut {
        cmd.args(["-ss", &format!("{start:.3}")]);
    }
    cmd.arg("-i").arg(job.src);
    if job.cut.is_some() {
        cmd.args(["-t", &format!("{duration:.3}")]);
    }
    match graph {
        Some(g) => {
            let post = if filters.is_empty() { "null".to_string() } else { filters.join(",") };
            cmd.args(["-filter_complex", &format!("{g};[m]{post}[v]"), "-map", "[v]"]);
        }
        None => {
            if !filters.is_empty() {
                cmd.args(["-vf", &filters.join(",")]);
            }
            cmd.args(["-map", "0:v:0"]);
        }
    }
    cmd.args(["-map", "0:a?"]).args(plan.video).args(plan.audio).args(["-movflags", "+faststart"]).arg(out);
    montage::run_progress(&mut cmd, &mut |t| on_pct((t / duration * 100.0) as f32))
}

fn file_size(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

/// Hasta 1080p y con un bitrate razonable: no hace falta recodificarlo para compartirlo.
pub fn is_light(p: &Probe, size: u64) -> bool {
    p.duration > 0.0 && p.width.min(p.height) <= 1080 && size as f64 * 8.0 / p.duration <= LIGHT_BPS
}

// ---------- copia para compartir ----------

#[derive(Serialize)]
pub struct ShareFile {
    pub quality: Quality,
    pub size_bytes: u64,
    /// Tamaño del archivo de la biblioteca.
    pub original_bytes: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize, Clone)]
struct ShareProgress {
    key: String,
    pct: f32,
}

enum Resolved {
    /// El archivo de la biblioteca ya sirve tal cual.
    Source,
    /// Hay una copia lista en `share/`.
    Cached(PathBuf),
    /// Hay que crear la copia en esta ruta.
    Missing(PathBuf),
}

fn resolve(ffmpeg: &Path, lib: &Library, id: &str, clip_id: Option<&str>, src: &Path, q: Quality) -> Resolved {
    let size = file_size(src);
    let direct = match q {
        Quality::Original => true,
        Quality::Share => library::probe(ffmpeg, src).is_ok_and(|p| is_light(&p, size)),
        Quality::Small => size <= SMALL_LIMIT_BYTES,
    };
    if direct {
        return Resolved::Source;
    }
    let cached = lib.file(id, &format!("share/{}-{}.mp4", clip_id.unwrap_or("match"), q.slug()));
    let modified = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let fresh = file_size(&cached) > 0 && modified(&cached) >= modified(src);
    if fresh { Resolved::Cached(cached) } else { Resolved::Missing(cached) }
}

/// Archivo a compartir en el formato pedido, si ya está listo.
pub fn shared_file(ffmpeg: &Path, lib: &Library, id: &str, clip_id: Option<&str>, src: &Path, q: Quality) -> Result<PathBuf> {
    match resolve(ffmpeg, lib, id, clip_id, src, q) {
        Resolved::Source => Ok(src.to_path_buf()),
        Resolved::Cached(p) => Ok(p),
        Resolved::Missing(_) => bail!("El archivo todavía no está listo en ese formato"),
    }
}

static PREPARING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(|| Mutex::new(HashSet::new()));

/// Deja listo el archivo en el formato pedido (lo crea en `share/` si hace falta) e informa
/// el avance con el evento `share-progress`.
pub fn prepare(app: &AppHandle, shared: &Shared, id: &str, clip_id: Option<&str>, src: &Path, q: Quality) -> Result<ShareFile> {
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = engine::ffmpeg_path(app, &settings);
    let lib = shared.library();
    let original_bytes = file_size(src);
    let path = match resolve(&ffmpeg, &lib, id, clip_id, src, q) {
        Resolved::Source => src.to_path_buf(),
        Resolved::Cached(p) => p,
        Resolved::Missing(out) => {
            if !PREPARING.lock().unwrap().insert(out.clone()) {
                bail!("Ya se está preparando este archivo");
            }
            let key = format!("{id}/{}", clip_id.unwrap_or("match"));
            let tmp = out.with_extension("tmp.mp4");
            let result = (|| -> Result<()> {
                std::fs::create_dir_all(out.parent().context("ruta sin carpeta")?)?;
                let job = Job { src, cut: None, quality: q, vertical: false, badge: None, mobile: None };
                render(&ffmpeg, &job, &engine::video_options(&settings, 0), &tmp, &|pct| {
                    let _ = app.emit("share-progress", ShareProgress { key: key.clone(), pct: pct.clamp(0.0, 100.0).floor() });
                })?;
                std::fs::rename(&tmp, &out)?;
                Ok(())
            })();
            PREPARING.lock().unwrap().remove(&out);
            if let Err(e) = result {
                let _ = std::fs::remove_file(&tmp);
                return Err(e);
            }
            log::info!("{key}: copia para compartir ({:.0} MB → {:.0} MB)", original_bytes as f64 / 1e6, file_size(&out) as f64 / 1e6);
            out
        }
    };
    let p = library::probe(&ffmpeg, &path).unwrap_or_default();
    Ok(ShareFile { quality: q, size_bytes: file_size(&path), original_bytes, width: p.width, height: p.height })
}

// ---------- clips que ya existían ----------

#[derive(Serialize, Default)]
pub struct Compressed {
    pub clips: usize,
    pub saved_bytes: u64,
}

/// Pasa al formato liviano los clips que siguen a la calidad de grabación. La partida
/// completa y las copias de Drive no se tocan.
pub fn compress_existing(app: &AppHandle, shared: &Shared) -> Result<Compressed> {
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = engine::ffmpeg_path(app, &settings);
    let rec = engine::video_options(&settings, 0);
    let lib = shared.library();
    let mut total = Compressed::default();
    for m in lib.list().into_iter().filter(|m| m.status == MatchStatus::Ready) {
        let heavy: Vec<_> = m
            .clips
            .iter()
            .filter(|c| !c.offloaded && Some(&c.file) != m.video.as_ref())
            .filter(|c| {
                let path = lib.file(&m.id, &c.file);
                library::probe(&ffmpeg, &path).is_ok_and(|p| !is_light(&p, file_size(&path)))
            })
            .cloned()
            .collect();
        let n = heavy.len();
        for (i, c) in heavy.into_iter().enumerate() {
            if shared.is_playing() {
                montage::clear_step(app, shared, &m.id);
                bail!("Se pausó porque empezó una partida: {} clips comprimidos hasta ahora", total.clips);
            }
            let label = format!("Comprimiendo clips ({}/{n})", i + 1);
            shared.set_step(app, &m.id, &label, Some(0.0));
            let src = lib.file(&m.id, &c.file);
            let before = file_size(&src);
            let tmp = src.with_extension("tmp.mp4");
            let job = Job { src: &src, cut: None, quality: Quality::Share, vertical: false, badge: None, mobile: None };
            let done = render(&ffmpeg, &job, &rec, &tmp, &|p| shared.set_step(app, &m.id, &label, Some((i as f32 * 100.0 + p) / n as f32)));
            let after = file_size(&tmp);
            if let Err(e) = done.and_then(|_| if after > 0 && after < before { Ok(()) } else { bail!("la copia no salió más liviana") }) {
                log::warn!("{}/{}: no se comprimió: {e:#}", m.id, c.id);
                let _ = std::fs::remove_file(&tmp);
                continue;
            }
            // Si el reproductor tiene abierto el original no se puede reemplazar: el clip
            // pasa a un archivo nuevo.
            let file = if std::fs::rename(&tmp, &src).is_ok() {
                c.file.clone()
            } else {
                let alt = format!("clips/{}-s.mp4", c.id);
                std::fs::rename(&tmp, lib.file(&m.id, &alt))?;
                let _ = std::fs::remove_file(&src);
                alt
            };
            let mut fresh = lib.load(&m.id)?;
            if let Some(fc) = fresh.clips.iter_mut().find(|x| x.id == c.id) {
                fc.file = file;
            }
            lib.save(&fresh)?;
            let _ = std::fs::remove_file(lib.file(&m.id, &format!("share/{}-share.mp4", c.id)));
            total.clips += 1;
            total.saved_bytes += before - after;
        }
        if n > 0 {
            montage::clear_step(app, shared, &m.id);
            let _ = app.emit("library", ());
        }
    }
    log::info!("clips comprimidos: {} ({:.0} MB liberados)", total.clips, total.saved_bytes as f64 / 1e6);
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> VideoOptions {
        VideoOptions { output_index: 0, fps: 60, encoder: "h264_nvenc".into(), cq: 19, max_bitrate_mbps: 80 }
    }

    fn arg<'a>(v: &'a [String], key: &str) -> &'a str {
        &v[v.iter().position(|a| a == key).unwrap() + 1]
    }

    #[test]
    fn share_downscales_only_above_1080p() {
        assert_eq!(plan(Quality::Share, (2560, 1440), 60.0, 16.0, &rec()).filters, vec!["scale=-2:1080:flags=lanczos"]);
        assert!(plan(Quality::Share, (1920, 1080), 60.0, 16.0, &rec()).filters.is_empty());
        // Vertical: manda el lado corto.
        assert!(plan(Quality::Share, (1080, 1920), 60.0, 16.0, &rec()).filters.is_empty());
        assert_eq!(plan(Quality::Small, (1080, 1920), 60.0, 16.0, &rec()).filters, vec!["scale=720:-2:flags=lanczos"]);
        assert_eq!(plan(Quality::Share, (3840, 2160), 120.0, 16.0, &rec()).filters, vec!["scale=-2:1080:flags=lanczos", "fps=60"]);
    }

    #[test]
    fn share_is_h264_capped_and_original_keeps_recording_settings() {
        let p = plan(Quality::Share, (2560, 1440), 60.0, 16.0, &rec());
        assert_eq!((arg(&p.video, "-c:v"), arg(&p.video, "-maxrate"), arg(&p.video, "-g")), ("h264_nvenc", "8000000", "120"));
        let o = plan(Quality::Original, (2560, 1440), 60.0, 16.0, &rec());
        assert!(o.filters.is_empty());
        assert_eq!((arg(&o.video, "-cq"), arg(&o.video, "-maxrate")), ("19", "80M"));
    }

    #[test]
    fn small_fits_in_16_mb() {
        for dur in [5.0, 16.0, 30.0, 75.0, 120.0] {
            let p = plan(Quality::Small, (2560, 1440), 60.0, dur, &rec());
            let bytes = (arg(&p.video, "-b:v").parse::<f64>().unwrap() + arg(&p.audio, "-b:a").parse::<f64>().unwrap()) * dur / 8.0;
            assert!(bytes <= 15.0e6, "{dur} s → {bytes}");
        }
        // Un clip largo baja a 30 fps; uno corto conserva los 60.
        assert!(plan(Quality::Small, (2560, 1440), 60.0, 75.0, &rec()).filters.contains(&"fps=30".to_string()));
        assert!(!plan(Quality::Small, (2560, 1440), 60.0, 10.0, &rec()).filters.iter().any(|f| f.starts_with("fps")));
    }

    #[test]
    fn mobile_layout_puts_the_hud_in_one_strip() {
        let top = MobileLayout { strip: 0.0, ..Default::default() };
        let g = mobile_geometry(&top, 2560, 1440);
        assert_eq!(g.game, (810, 1440, 875, 0));
        // Misma altura, lado a lado, centradas y dentro del cuadro.
        assert_eq!(g.health.size.1, g.weapons.size.1);
        assert_eq!(g.health.pos.1, 40);
        assert_eq!(g.weapons.pos.0, g.health.pos.0 + g.health.size.0 + 16);
        assert!(g.weapons.pos.0 + g.weapons.size.0 <= 1080 - g.health.pos.0 + 2);
        assert_eq!(g.band, (26, g.health.size.1 + 28));
        // Los recortes no se salen de la grabación.
        for p in [g.health, g.weapons] {
            assert!(p.crop.2 + p.crop.0 <= 2560 && p.crop.3 + p.crop.1 <= 1440, "{p:?}");
        }
        let f = mobile_filter(&g);
        assert!(f.starts_with("[0:v]split=3[g][h][w];[g]crop=810:1440:875:0,scale=1080:1920") && f.ends_with("[m]"), "{f}");
    }

    #[test]
    fn mobile_layout_in_two_rows_is_bigger_and_stacked() {
        let top = MobileLayout { strip: 0.0, ..Default::default() };
        let one = mobile_geometry(&top, 2560, 1440);
        let two = mobile_geometry(&MobileLayout { two_rows: true, ..top }, 2560, 1440);
        assert!(two.health.size.1 > one.health.size.1);
        assert_eq!(two.weapons.pos.1, 40 + two.health.size.1 + 16);
        assert_eq!(two.band.1, two.health.size.1 + 16 + two.weapons.size.1 + 28);
        // El encuadre del juego se mueve y no se sale.
        let left = mobile_geometry(&MobileLayout { focus: 0.0, ..Default::default() }, 2560, 1440);
        assert_eq!(left.game.2, 0);
        let right = mobile_geometry(&MobileLayout { focus: 1.0, ..Default::default() }, 2560, 1440);
        assert_eq!(right.game.2, 2560 - 810);
    }

    #[test]
    fn mobile_strip_moves_between_the_top_and_the_bottom() {
        for two_rows in [false, true] {
            let at = |strip: f64| mobile_geometry(&MobileLayout { two_rows, strip, ..Default::default() }, 2560, 1440);
            // Abajo del todo termina a 40 px del borde; por defecto queda cerca de abajo.
            let low = at(1.0);
            assert_eq!(low.weapons.pos.1 + low.weapons.size.1, 1920 - 40, "{low:?}");
            assert_eq!(low.band.0 + low.band.1, 1920 - 40 + 14);
            let def = mobile_geometry(&MobileLayout { two_rows, ..Default::default() }, 2560, 1440);
            assert!(def.health.pos.1 > 1400 && def.health.pos.1 < low.health.pos.1, "{def:?}");
            // Fuera de rango no se sale del cuadro.
            assert_eq!(at(7.0), low);
            assert_eq!(at(-1.0), at(0.0));
        }
    }

    #[test]
    fn light_files_are_shared_as_they_are() {
        let p = Probe { duration: 16.0, width: 1920, height: 1080, fps: 60.0 };
        assert!(is_light(&p, 16_000_000));
        assert!(!is_light(&p, 150_000_000));
        assert!(!is_light(&Probe { width: 2560, height: 1440, ..p }, 16_000_000));
    }

    /// Con un clip real de la biblioteca: `cargo test --lib export -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn renders_real_clip() {
        let root = PathBuf::from(std::env::var("USERPROFILE").unwrap()).join("Videos").join("ggswp");
        let lib = Library::new(root);
        let (m, c) = lib
            .list()
            .into_iter()
            .find_map(|m| m.clips.iter().find(|c| c.kind != "vertical" && c.file != "match.mp4" && lib.file(&m.id, &c.file).exists()).cloned().map(|c| (m, c)))
            .expect("un clip local");
        let ffmpeg = Path::new("ffmpeg");
        let src = lib.file(&m.id, &c.file);
        let dir = std::env::temp_dir().join(library::new_id("ggswp-export"));
        std::fs::create_dir_all(&dir).unwrap();
        let before = file_size(&src);
        let dur = library::probe(ffmpeg, &src).unwrap().duration;
        for (q, vertical, mobile) in [(Quality::Share, false, false), (Quality::Small, false, false), (Quality::Share, true, false), (Quality::Small, true, false), (Quality::Share, true, true), (Quality::Small, true, true)] {
            let out = dir.join(format!("{}-{vertical}-{mobile}.mp4", q.slug()));
            let job = Job { src: &src, cut: Some((1.0, dur - 2.0)), quality: q, vertical, badge: montage::vertical_badge(3, false), mobile: mobile.then(MobileLayout::default) };
            let t = std::time::Instant::now();
            render(ffmpeg, &job, &rec(), &out, &|_| {}).unwrap();
            let p = library::probe(ffmpeg, &out).unwrap();
            if let Ok(keep) = std::env::var("GGSWP_KEEP") {
                std::fs::copy(&out, PathBuf::from(keep).join(out.file_name().unwrap())).unwrap();
            }
            println!("{q:?} vertical={vertical} móvil={mobile}: {:.1} MB → {:.1} MB, {}x{} en {:.1} s", before as f64 / 1e6, file_size(&out) as f64 / 1e6, p.width, p.height, t.elapsed().as_secs_f64());
            assert!((p.duration - (dur - 2.0)).abs() < 0.2, "{} vs {}", p.duration, dur - 2.0);
            let short = if q == Quality::Small { 720 } else { 1080 };
            assert_eq!(p.width.min(p.height), short);
            assert_eq!(p.width < p.height, vertical);
            if q == Quality::Small {
                assert!(file_size(&out) <= SMALL_LIMIT_BYTES);
            } else {
                assert!(is_light(&p, file_size(&out)));
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
