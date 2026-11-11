//! Formatos de salida de los clips. La grabación va a ~80 Mbps para no perder nada, pero un
//! clip a esa calidad pesa 10 MB por segundo: los clips se guardan y se comparten en 1080p
//! H.264 (~1 MB por segundo), que es lo que aceptan WhatsApp, Instagram y cualquier teléfono.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::capture::video::{Family, VideoOptions, encoder_args, encoder_for, family};
use crate::capture::{TRACK_GAME, TRACK_MIC, TRACK_MIX, TRACK_OTHER, TRACK_SYSTEM};
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

/// Cómo se controla el peso del video: calidad constante con un tope de bitrate, o un
/// bitrate fijo.
#[derive(Clone, Copy)]
enum Rate {
    Quality(u32, f64),
    Bitrate(f64),
}

pub struct Plan {
    /// Lado corto del video de salida, en píxeles.
    pub short: u32,
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

/// Ancho y alto (pares) con que sale un video de `dims` en ese formato.
pub fn output_dims(q: Quality, (w, h): (u32, u32)) -> (u32, u32) {
    let limit = match q {
        Quality::Original => u32::MAX,
        Quality::Share => 1080,
        Quality::Small => 720,
    };
    if w == 0 || h == 0 || w.min(h) <= limit {
        return (w, h);
    }
    let k = limit as f64 / w.min(h) as f64;
    (even(w as f64 * k), even(h as f64 * k))
}

/// Lado corto del video después de `scale_to`.
fn scaled_short(limit: u32, (w, h): (u32, u32)) -> u32 {
    if w == 0 || h == 0 { limit } else { w.min(h).min(limit) }
}

/// Bitrate de video del formato liviano para un clip de `duration` segundos.
fn small_video_bps(duration: f64, audio_bps: f64) -> f64 {
    ((SMALL_TARGET_BYTES * 8.0 / duration.max(1.0) - audio_bps) * 0.94).clamp(500e3, 6.0e6)
}

/// Cómo codificar un video de `dims` píxeles y `fps` cuadros que va a durar `duration`.
pub fn plan(q: Quality, dims: (u32, u32), fps: f64, duration: f64, rec: &VideoOptions) -> Plan {
    // Los clips siempre van en H.264, con la tarjeta (o el software) que graba.
    let fam = family(&rec.encoder);
    let h264 = |rate: Rate, out_fps: f64| {
        let mut v = strs(&["-c:v", &encoder_for(fam, "h264")]);
        let (b, max, buf) = match rate {
            Rate::Quality(_, max) => (String::new(), format!("{max:.0}"), format!("{:.0}", max * 2.0)),
            Rate::Bitrate(bps) => (format!("{bps:.0}"), format!("{:.0}", bps * 1.2), format!("{:.0}", bps * 2.0)),
        };
        let limits = ["-maxrate", &max, "-bufsize", &buf];
        v.extend(strs(match fam {
            Family::Nvenc => &["-preset", "p6", "-tune", "hq", "-rc", "vbr"],
            Family::Amf => &["-usage", "transcoding", "-quality", "quality", "-rc", "vbr_peak"],
            Family::Qsv => &["-preset", "slow"],
            Family::Software => &["-preset", "veryfast"],
        }));
        match (rate, fam) {
            (Rate::Quality(cq, _), Family::Nvenc) => v.extend(strs(&["-cq", &cq.to_string(), "-b:v", "0"])),
            // AMF no tiene calidad constante con tope: se le da un bitrate medio bajo el tope.
            (Rate::Quality(_, max), Family::Amf) => v.extend(strs(&["-b:v", &format!("{:.0}", max * 0.75)])),
            (Rate::Quality(cq, _), Family::Qsv) => v.extend(strs(&["-global_quality", &cq.to_string()])),
            (Rate::Quality(cq, _), Family::Software) => v.extend(strs(&["-crf", &(cq - 2).to_string()])),
            (Rate::Bitrate(_), _) => v.extend(strs(&["-b:v", &b])),
        }
        v.extend(strs(&limits));
        let pix = if fam == Family::Qsv { "nv12" } else { "yuv420p" };
        v.extend(strs(&["-profile:v", "high", "-pix_fmt", pix, "-bf", "2", "-g", &format!("{:.0}", out_fps * 2.0)]));
        v
    };
    let known_fps = if fps > 0.0 { fps } else { 60.0 };
    match q {
        Quality::Original => Plan { short: scaled_short(u32::MAX, dims).min(4320), filters: vec![], video: encoder_args(rec), audio: strs(&["-c:a", "aac", "-b:a", "192k"]) },
        Quality::Share => {
            let mut filters: Vec<String> = scale_to(1080, dims).into_iter().collect();
            let out_fps = if known_fps > 61.0 { 60.0 } else { known_fps };
            if out_fps < known_fps {
                filters.push("fps=60".into());
            }
            Plan { short: scaled_short(1080, dims), filters, video: h264(Rate::Quality(25, SHARE_MAX_BPS), out_fps), audio: strs(&["-c:a", "aac", "-b:a", "160k"]) }
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
            Plan {
                short: scaled_short(720, dims),
                filters,
                video: h264(Rate::Bitrate(bps), out_fps),
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
/// Pieza suelta del modo móvil (minimapa, contador…): un recorte de la grabación puesto
/// donde se quiera sobre el cuadro vertical.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Extra {
    pub label: String,
    /// Recorte en la grabación.
    pub src: Region,
    /// Esquina superior izquierda en el cuadro de salida (0–1).
    pub x: f64,
    pub y: f64,
    /// Ancho en el cuadro de salida (0–1); el alto sale de la proporción del recorte.
    pub w: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
    /// Piezas sueltas además de la franja.
    pub extras: Vec<Extra>,
}

/// Disposición del modo móvil guardada con nombre.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MobilePreset {
    pub name: String,
    pub layout: MobileLayout,
}

impl Default for MobileLayout {
    fn default() -> Self {
        // HUD de Fortnite a escala 100 % en 16:9.
        Self { health: Region { x: 0.015, y: 0.872, w: 0.29, h: 0.085 }, weapons: Region { x: 0.685, y: 0.855, w: 0.302, h: 0.092 }, focus: 0.5, two_rows: false, strip: 0.93, extras: Vec::new() }
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

#[derive(Debug, Clone, PartialEq)]
pub struct MobileGeometry {
    /// Recorte 9:16 del juego: ancho, alto, x, y.
    pub game: (u32, u32, u32, u32),
    /// Franja oscura detrás del HUD: y, alto.
    pub band: (u32, u32),
    pub health: Placed,
    pub weapons: Placed,
    pub extras: Vec<Placed>,
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
    let extras = l
        .extras
        .iter()
        .map(|e| {
            let c = crop(&e.src);
            let ew = even(e.w.clamp(0.05, 1.0) * MOBILE_W).min(MOBILE_W as u32);
            let eh = even(ew as f64 / aspect(c)).min(MOBILE_H as u32);
            let ex = (e.x.max(0.0) * MOBILE_W).min(MOBILE_W - ew as f64).round() as u32;
            let ey = (e.y.max(0.0) * MOBILE_H).min(MOBILE_H - eh as f64).round() as u32;
            Placed { crop: c, size: (ew, eh), pos: (ex, ey) }
        })
        .collect();
    MobileGeometry {
        extras,
        game: (gw, h, gx.round() as u32, 0),
        band: ((y - MOBILE_BAND_PAD) as u32, (band_h + 2.0 * MOBILE_BAND_PAD) as u32),
        health: Placed { crop: hc, size: hs, pos: (hx as u32, y as u32) },
        weapons: Placed { crop: wc, size: ws, pos: (wx as u32, (y + wdy) as u32) },
    }
}

/// Recorte central que deja 1/`zoom` del cuadro (acercamiento), o nada si no hay zoom.
fn zoom_crop(zoom: f64) -> String {
    if zoom > 1.01 { format!("crop=trunc(iw/{zoom:.3}/2)*2:trunc(ih/{zoom:.3}/2)*2,") } else { String::new() }
}

/// Grafo de filtros del modo móvil: de `input` a `[f{i}]`, en 1080x1920. El zoom acerca
/// solo el juego; la salud, las armas y las piezas sueltas quedan igual.
pub fn mobile_filter(g: &MobileGeometry, input: &str, i: usize, zoom: f64) -> String {
    let piece = |p: &Placed| format!("crop={}:{}:{}:{},scale={}:{}:flags=lanczos", p.crop.0, p.crop.1, p.crop.2, p.crop.3, p.size.0, p.size.1);
    let pieces: Vec<&Placed> = [&g.health, &g.weapons].into_iter().chain(g.extras.iter()).collect();
    let n = pieces.len();
    let labels: String = (0..n).map(|k| format!("[p{i}_{k}]")).collect();
    let mut f = format!(
        "{input}split={}[g{i}]{labels};[g{i}]crop={}:{}:{}:{},{}scale=1080:1920:flags=lanczos,setsar=1,drawbox=x=0:y={}:w=iw:h={}:color=black@0.45:t=fill[o{i}_0]",
        n + 1,
        g.game.0,
        g.game.1,
        g.game.2,
        g.game.3,
        zoom_crop(zoom),
        g.band.0,
        g.band.1,
    );
    for (k, p) in pieces.iter().enumerate() {
        let out = if k + 1 == n { format!("[f{i}]") } else { format!("[o{i}_{}]", k + 1) };
        f.push_str(&format!(";[p{i}_{k}]{}[q{i}_{k}];[o{i}_{k}][q{i}_{k}]overlay={}:{}{out}", piece(p), p.pos.0, p.pos.1));
    }
    f
}

// ---------- efectos del editor ----------

/// Texto sobre el clip, centrado a lo ancho.
#[derive(Debug, Clone, Deserialize)]
pub struct Text {
    pub text: String,
    /// Altura del centro del texto en el cuadro (0 = arriba, 1 = abajo).
    pub y: f64,
    /// Tamaño de la letra como fracción del lado corto del cuadro.
    pub size: f64,
    /// white | accent | yellow
    #[serde(default)]
    pub color: String,
    /// Tramo en que se ve, en segundos del clip ya exportado; `to` en 0 = hasta el final.
    #[serde(default)]
    pub from: f64,
    #[serde(default)]
    pub to: f64,
}

/// Tramo del clip con cámara lenta y/o acercamiento al centro (el punto de mira).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Moment {
    pub start: f64,
    pub end: f64,
    /// 1 = normal, 0,5 = mitad de velocidad.
    pub speed: f64,
    /// 1 = sin acercar.
    pub zoom: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Music {
    pub path: String,
    pub volume: f64,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Fx {
    pub texts: Vec<Text>,
    pub moment: Option<Moment>,
    pub music: Option<Music>,
    /// Fuentes de audio de la grabación a incluir (`game`, `other`, `mic`); `None` = todas.
    pub tracks: Option<Vec<String>>,
    /// Volumen del audio de la grabación (1 = igual).
    pub volume: Option<f64>,
}

/// Fuentes de audio que llevan los clips según los ajustes.
pub fn default_tracks(s: &crate::config::Settings) -> Vec<String> {
    let mut t = vec![TRACK_GAME.to_string()];
    if s.clip_other_audio {
        t.push(TRACK_OTHER.into());
    }
    if s.clip_mic_audio {
        t.push(TRACK_MIC.into());
    }
    t
}

/// De dónde sale el audio de un video con las pistas `names`, si solo se quieren las fuentes
/// `wanted`: `None` = sin audio; si no, (filtros previos, etiqueta de entrada). Los videos
/// de una sola pista se usan tal cual.
pub fn audio_source(names: &[String], wanted: Option<&[String]>) -> Option<(String, String)> {
    if names.is_empty() {
        return None;
    }
    let whole = Some((String::new(), "[0:a:0]".to_string()));
    let Some(wanted) = wanted.filter(|_| names[0] == TRACK_MIX) else { return whole };
    // El audio del juego se llama `system` cuando no se pudo separar del resto.
    let wants = |name: &str| wanted.iter().any(|w| w == name || (w == TRACK_GAME && name == TRACK_SYSTEM));
    let sources = names.len() - 1;
    let picked: Vec<usize> = (1..names.len()).filter(|&i| wants(&names[i])).collect();
    match picked.as_slice() {
        [] => None,
        p if p.len() == sources => whole,
        [one] => Some((String::new(), format!("[0:a:{one}]"))),
        many => {
            let ins: String = many.iter().map(|i| format!("[0:a:{i}]")).collect();
            Some((format!("{ins}amix=inputs={}:duration=first:normalize=0[asrc];", many.len()), "[asrc]".into()))
        }
    }
}

/// `atempo` solo acepta de 0,5 a 100: las velocidades menores se encadenan.
fn atempo(speed: f64) -> String {
    let mut s = speed;
    let mut f = String::new();
    while s < 0.5 {
        f.push_str(",atempo=0.5");
        s /= 0.5;
    }
    if (s - 1.0).abs() > 0.001 {
        f.push_str(&format!(",atempo={s:.4}"));
    }
    f
}

/// Ruta de archivo dentro de un filtro de ffmpeg (entre comillas simples).
fn filter_path(p: &Path) -> String {
    p.display().to_string().replace('\\', "/").replace(':', "\\:").replace('\'', "'\\''")
}

/// `short` es el lado corto del video de salida, en píxeles: de ahí sale el tamaño de la letra.
fn text_filter(t: &Text, file: &Path, short: u32) -> String {
    let color = match t.color.as_str() {
        "accent" => "0x39FF88",
        "yellow" => "0xFFD23F",
        _ => "white",
    };
    let px = (short as f64 * t.size.clamp(0.02, 0.25)).round();
    let mut f = format!(
        "drawtext=fontfile='{}':textfile='{}':expansion=none:fontsize={px:.0}:fontcolor={color}:borderw={}:bordercolor=black@0.9:x=(w-text_w)/2:y=h*{:.4}-text_h/2",
        filter_path(Path::new(montage::FONT_FILE)),
        filter_path(file),
        ((px * 0.07).round() as u32).max(2),
        t.y.clamp(0.0, 1.0),
    );
    if t.from > 0.0 || t.to > 0.0 {
        let to = if t.to > t.from { t.to } else { 1e9 };
        f.push_str(&format!(":enable='between(t\\,{:.3}\\,{to:.3})'", t.from.max(0.0)));
    }
    f
}

/// Encuadre de la salida.
enum Frame<'a> {
    Plain,
    Vertical(Option<&'a (String, String)>),
    Mobile(MobileGeometry),
}

/// Grafo que arma el cuadro de un tramo: de `input` a `[f{i}]`.
fn frame_graph(frame: &Frame, input: &str, i: usize, zoom: f64, (w, h): (u32, u32)) -> String {
    match frame {
        Frame::Mobile(g) => mobile_filter(g, input, i, zoom),
        Frame::Vertical(badge) => format!("{input}{}{}[f{i}]", zoom_crop(zoom), montage::vertical_filter(*badge, montage::FONT_FILE)),
        Frame::Plain if zoom > 1.01 && w > 0 => format!("{input}{}scale={w}:{h}:flags=lanczos,setsar=1[f{i}]", zoom_crop(zoom)),
        Frame::Plain => format!("{input}setsar=1[f{i}]"),
    }
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
    /// Texto, cámara lenta, música y fuentes de audio.
    pub fx: Fx,
}

/// Tramos del clip con su velocidad y zoom: (inicio, fin, velocidad, zoom).
fn parts(moment: Option<Moment>, duration: f64) -> Vec<(f64, f64, f64, f64)> {
    let m = moment.filter(|m| (m.speed - 1.0).abs() > 0.01 || m.zoom > 1.01).map(|m| {
        let start = m.start.clamp(0.0, duration);
        Moment { start, end: m.end.clamp(start, duration), speed: m.speed.clamp(0.1, 4.0), zoom: m.zoom.clamp(1.0, 4.0) }
    });
    match m.filter(|m| m.end - m.start >= 0.1) {
        None => vec![(0.0, duration, 1.0, 1.0)],
        Some(m) => [(0.0, m.start, 1.0, 1.0), (m.start, m.end, m.speed, m.zoom), (m.end, duration, 1.0, 1.0)].into_iter().filter(|p| p.1 - p.0 >= 0.05).collect(),
    }
}

/// Corta (exacto al cuadro) y codifica según el formato pedido, con los efectos del editor.
pub fn render(ffmpeg: &Path, job: &Job, rec: &VideoOptions, out: &Path, on_pct: &dyn Fn(f32)) -> Result<()> {
    let src = library::probe(ffmpeg, job.src).unwrap_or_default();
    let duration = job.cut.map(|c| c.1).unwrap_or(src.duration).max(0.1);
    let parts = parts(job.fx.moment, duration);
    let out_duration: f64 = parts.iter().map(|p| (p.1 - p.0) / p.2).sum();
    let mut dims = (src.width, src.height);
    let frame = if let Some(layout) = &job.mobile {
        if src.width == 0 || src.width <= src.height {
            bail!("El modo móvil necesita un video horizontal");
        }
        dims = (1080, 1920);
        Frame::Mobile(mobile_geometry(layout, src.width, src.height))
    } else if job.vertical {
        dims = (1080, 1920);
        // Sin la fuente de Windows, el video sale igual pero sin marcador.
        Frame::Vertical(job.badge.as_ref().filter(|_| Path::new(montage::FONT_FILE).exists()))
    } else {
        Frame::Plain
    };
    let plan = plan(job.quality, dims, src.fps, out_duration, rec);

    // Video: cada tramo se encuadra por separado (el del efecto, con su zoom) y se unen.
    let whole = parts.len() == 1;
    let mut graph = String::new();
    if !whole {
        let labels: String = (0..parts.len()).map(|i| format!("[s{i}]")).collect();
        graph.push_str(&format!("[0:v:0]split={}{labels};", parts.len()));
    }
    for (i, &(a, b, speed, zoom)) in parts.iter().enumerate() {
        let mut input = if whole { "[0:v:0]".to_string() } else { format!("[s{i}]") };
        if !whole || (speed - 1.0).abs() > 0.001 {
            let trim = if whole { String::new() } else { format!("trim=start={a:.3}:end={b:.3},") };
            graph.push_str(&format!("{input}{trim}setpts=(PTS-STARTPTS)/{speed:.4}[t{i}];"));
            input = format!("[t{i}]");
        }
        graph.push_str(&frame_graph(&frame, &input, i, zoom, (src.width, src.height)));
        graph.push(';');
    }
    let mut last = "[f0]".to_string();
    if !whole {
        let labels: String = (0..parts.len()).map(|i| format!("[f{i}]")).collect();
        graph.push_str(&format!("{labels}concat=n={}:v=1:a=0[cat];", parts.len()));
        last = "[cat]".into();
    }
    let mut post = plan.filters.clone();
    let retimed = parts.iter().any(|p| (p.2 - 1.0).abs() > 0.001);
    if retimed && !post.iter().any(|f| f.starts_with("fps=")) {
        // La cámara lenta deja huecos entre cuadros: se rellena a una cadencia fija.
        post.push(format!("fps={:.0}", if src.fps > 1.0 { src.fps.min(60.0) } else { 60.0 }));
    }
    // Cada texto va en un archivo: así no hay que escapar lo que escriba el usuario.
    let texts: Vec<&Text> = job.fx.texts.iter().filter(|t| !t.text.trim().is_empty()).collect();
    let text_dir = std::env::temp_dir().join(library::new_id("ggswp-text"));
    if !texts.is_empty() && Path::new(montage::FONT_FILE).exists() {
        std::fs::create_dir_all(&text_dir)?;
        for (i, t) in texts.iter().enumerate() {
            let file = text_dir.join(format!("{i}.txt"));
            std::fs::write(&file, t.text.trim())?;
            post.push(text_filter(t, &file, plan.short));
        }
    }
    graph.push_str(&format!("{last}{}[v]", if post.is_empty() { "null".to_string() } else { post.join(",") }));

    // Audio: las fuentes elegidas, con el mismo cambio de velocidad, más la música.
    let source = audio_source(&src.audio, job.fx.tracks.as_deref());
    let mut audio = None;
    if let Some((pre, label)) = source {
        graph.push(';');
        graph.push_str(&pre);
        let volume = job.fx.volume.filter(|v| (v - 1.0).abs() > 0.01).map(|v| format!("volume={:.3},", v.clamp(0.0, 4.0))).unwrap_or_default();
        if whole && !retimed {
            graph.push_str(&format!("{label}{volume}anull[a0]"));
        } else {
            let labels: String = (0..parts.len()).map(|i| format!("[as{i}]")).collect();
            graph.push_str(&format!("{label}{volume}asplit={}{labels}", parts.len()));
            for (i, &(a, b, speed, _)) in parts.iter().enumerate() {
                graph.push_str(&format!(";[as{i}]atrim=start={a:.3}:end={b:.3},asetpts=PTS-STARTPTS{}[at{i}]", atempo(speed)));
            }
            let labels: String = (0..parts.len()).map(|i| format!("[at{i}]")).collect();
            graph.push_str(&format!(";{labels}concat=n={}:v=0:a=1[a0]", parts.len()));
        }
        audio = Some("[a0]");
    }
    let music = job.fx.music.as_ref().filter(|m| m.volume > 0.0 && Path::new(&m.path).exists());
    if let Some(m) = music {
        let fade = (out_duration - 1.5).max(0.0);
        graph.push_str(&format!(
            ";[1:a]aresample=48000,volume={:.3},atrim=0:{out_duration:.3},afade=t=out:st={fade:.3}:d=1.5[mus]",
            m.volume.clamp(0.0, 4.0)
        ));
        match audio {
            Some(a) => graph.push_str(&format!(";{a}[mus]amix=inputs=2:duration=first:normalize=0[a1]")),
            None => graph.push_str(";[mus]anull[a1]"),
        }
        audio = Some("[a1]");
    }

    let mut cmd = montage::ffmpeg_cmd(ffmpeg);
    if let Some((start, _)) = job.cut {
        cmd.args(["-ss", &format!("{start:.3}"), "-t", &format!("{duration:.3}")]);
    }
    cmd.arg("-i").arg(job.src);
    if let Some(m) = music {
        cmd.args(["-stream_loop", "-1", "-i", &m.path]);
    }
    cmd.args(["-filter_complex", &graph, "-map", "[v]"]);
    match audio {
        Some(a) => cmd.args(["-map", a]).args(&plan.audio),
        None => cmd.arg("-an"),
    };
    cmd.args(plan.video).args(["-t", &format!("{:.3}", out_duration + 0.05), "-movflags", "+faststart"]).arg(out);
    let result = montage::run_progress(&mut cmd, &mut |t| on_pct((t / out_duration * 100.0) as f32));
    let _ = std::fs::remove_dir_all(&text_dir);
    result
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
                let job = Job { src, cut: None, quality: q, vertical: false, badge: None, mobile: None, fx: Default::default() };
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
            let job = Job { src: &src, cut: None, quality: Quality::Share, vertical: false, badge: None, mobile: None, fx: Default::default() };
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
        let f = mobile_filter(&g, "[0:v]", 0, 1.0);
        assert!(f.starts_with("[0:v]split=3[g0][p0_0][p0_1];[g0]crop=810:1440:875:0,scale=1080:1920") && f.ends_with("[f0]"), "{f}");
        // El zoom acerca solo el juego.
        let z = mobile_filter(&g, "[t1]", 1, 2.0);
        assert!(z.contains("[g1]crop=810:1440:875:0,crop=trunc(iw/2.000/2)*2:trunc(ih/2.000/2)*2,scale=1080:1920"), "{z}");
        assert_eq!(z.matches("crop=trunc").count(), 1);
    }

    #[test]
    fn mobile_extras_keep_their_shape_and_stay_inside() {
        let minimap = Extra { label: "Minimapa".into(), src: Region { x: 0.85, y: 0.02, w: 0.14, h: 0.25 }, x: 0.9, y: 0.02, w: 0.3 };
        let g = mobile_geometry(&MobileLayout { extras: vec![minimap], ..Default::default() }, 2560, 1440);
        let e = g.extras[0];
        assert_eq!(e.size.0, 324);
        // Misma proporción que el recorte (358x360) y sin salirse por la derecha.
        assert!((e.size.1 as f64 / e.size.0 as f64 - e.crop.1 as f64 / e.crop.0 as f64).abs() < 0.02, "{e:?}");
        assert_eq!(e.pos.0 + e.size.0, 1080);
        let f = mobile_filter(&g, "[0:v]", 0, 1.0);
        assert!(f.starts_with("[0:v]split=4[g0][p0_0][p0_1][p0_2];") && f.ends_with(&format!("overlay={}:{}[f0]", e.pos.0, e.pos.1)), "{f}");
    }

    /// Video de prueba de 10 s con tres pistas (mezcla, juego y resto), cada una con su tono.
    #[cfg(test)]
    fn synthetic(dir: &Path) -> PathBuf {
        let out = dir.join("src.mp4");
        let ok = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "testsrc2=s=1920x1080:r=60", "-f", "lavfi", "-i", "sine=f=440", "-f", "lavfi", "-i", "sine=f=880", "-f", "lavfi", "-i", "sine=f=220"])
            .args(["-t", "10", "-map", "0:v", "-map", "1:a", "-map", "2:a", "-map", "3:a", "-c:v", "libx264", "-preset", "ultrafast", "-pix_fmt", "yuv420p", "-c:a", "aac"])
            .args(["-metadata:s:a:0", "handler_name=mix", "-metadata:s:a:1", "handler_name=game", "-metadata:s:a:2", "handler_name=other"])
            .arg(&out)
            .status()
            .unwrap()
            .success();
        assert!(ok);
        out
    }

    /// Tono dominante (Hz) de la única pista de audio de un video.
    #[cfg(test)]
    fn dominant_tone(video: &Path) -> f64 {
        let o = std::process::Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-i"]).arg(video).args(["-t", "2", "-ac", "1", "-ar", "8000", "-f", "s16le", "-"]).output().unwrap();
        let x: Vec<f64> = o.stdout.chunks_exact(2).map(|b| i16::from_le_bytes([b[0], b[1]]) as f64).collect();
        // Cruces por cero: suficiente para distinguir tonos puros.
        let crossings = x.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        crossings as f64 / (x.len() as f64 / 8000.0)
    }

    /// Exporta con todos los efectos: `cargo test --lib renders_effects -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn renders_effects() {
        let ffmpeg = Path::new("ffmpeg");
        let dir = std::env::temp_dir().join(library::new_id("ggswp-fx"));
        std::fs::create_dir_all(&dir).unwrap();
        let src = synthetic(&dir);
        assert_eq!(library::probe(ffmpeg, &src).unwrap().audio, ["mix", "game", "other"]);
        let text = |s: &str, y: f64| Text { text: s.into(), y, size: 0.07, color: "accent".into(), from: 0.0, to: 0.0 };
        let moment = Some(Moment { start: 2.0, end: 4.0, speed: 0.5, zoom: 2.0 });
        let keep = std::env::var("GGSWP_KEEP").ok();
        for (name, vertical, mobile) in [("horizontal", false, false), ("vertical", true, false), ("movil", true, true)] {
            let fx = Fx { texts: vec![text("DOBLE: eliminación, 'ok' 100%", 0.12), Text { from: 1.0, to: 3.0, ..text("solo 2 s", 0.5) }], moment, tracks: Some(vec!["game".into()]), ..Default::default() };
            let layout = MobileLayout { extras: vec![Extra { label: "Minimapa".into(), src: Region { x: 0.85, y: 0.02, w: 0.14, h: 0.25 }, x: 0.68, y: 0.03, w: 0.3 }], ..Default::default() };
            let job = Job { src: &src, cut: Some((1.0, 8.0)), quality: Quality::Share, vertical, badge: montage::vertical_badge(3, false), mobile: mobile.then_some(layout), fx };
            let out = dir.join(format!("{name}.mp4"));
            render(ffmpeg, &job, &rec(), &out, &|_| {}).unwrap();
            let p = library::probe(ffmpeg, &out).unwrap();
            let tone = dominant_tone(&out);
            println!("{name}: {}x{}, {:.2} s, {} pista(s), tono {tone:.0} Hz", p.width, p.height, p.duration, p.audio.len());
            // 8 s con 2 s a mitad de velocidad = 10 s; una sola pista, la del juego (880 Hz).
            assert!((p.duration - 10.0).abs() < 0.15, "{}", p.duration);
            assert_eq!(p.audio.len(), 1);
            assert!((tone - 880.0).abs() < 30.0, "{tone}");
            assert_eq!((p.width, p.height), if vertical { (1080, 1920) } else { (1920, 1080) });
            if let Some(k) = &keep {
                std::fs::copy(&out, PathBuf::from(k).join(format!("fx-{name}.mp4"))).unwrap();
            }
        }
        // Sin fuentes elegidas y con música: el audio es solo la música (220 Hz, la pista 3).
        let music = dir.join("music.wav");
        assert!(std::process::Command::new("ffmpeg").args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=f=330", "-t", "3"]).arg(&music).status().unwrap().success());
        let fx = Fx { tracks: Some(vec![]), music: Some(Music { path: music.display().to_string(), volume: 1.0 }), ..Default::default() };
        let out = dir.join("music.mp4");
        render(ffmpeg, &Job { src: &src, cut: Some((0.0, 7.0)), quality: Quality::Small, vertical: false, badge: None, mobile: None, fx }, &rec(), &out, &|_| {}).unwrap();
        let p = library::probe(ffmpeg, &out).unwrap();
        println!("música: {:.2} s, tono {:.0} Hz", p.duration, dominant_tone(&out));
        // La música (3 s) se repite hasta cubrir el clip.
        assert!((p.duration - 7.0).abs() < 0.15 && (dominant_tone(&out) - 330.0).abs() < 20.0);
        // Sin audio pedido ni música: sale sin pista de audio.
        let out = dir.join("mute.mp4");
        let fx = Fx { tracks: Some(vec![]), ..Default::default() };
        render(ffmpeg, &Job { src: &src, cut: None, quality: Quality::Small, vertical: false, badge: None, mobile: None, fx }, &rec(), &out, &|_| {}).unwrap();
        assert!(library::probe(ffmpeg, &out).unwrap().audio.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn clips_use_the_h264_of_the_recording_card() {
        let with = |enc: &str, q: Quality| plan(q, (2560, 1440), 60.0, 20.0, &VideoOptions { encoder: enc.into(), ..rec() }).video.join(" ");
        assert!(with("hevc_nvenc", Quality::Share).starts_with("-c:v h264_nvenc -preset p6 -tune hq -rc vbr -cq 25 -b:v 0 -maxrate 8000000 -bufsize 16000000"));
        assert!(with("av1_amf", Quality::Share).starts_with("-c:v h264_amf -usage transcoding -quality quality -rc vbr_peak -b:v 6000000 -maxrate 8000000"));
        assert!(with("h264_qsv", Quality::Share).contains("-global_quality 25") && with("h264_qsv", Quality::Share).contains("-pix_fmt nv12"));
        assert!(with("libx264", Quality::Share).starts_with("-c:v libx264 -preset veryfast -crf 23 -maxrate"));
        assert!(with("h264_amf", Quality::Small).contains("-rc vbr_peak -b:v 5"));
    }

    #[test]
    fn slow_motion_splits_the_clip_in_parts() {
        assert_eq!(parts(None, 10.0), vec![(0.0, 10.0, 1.0, 1.0)]);
        let m = Moment { start: 4.0, end: 6.0, speed: 0.5, zoom: 2.0 };
        assert_eq!(parts(Some(m), 10.0), vec![(0.0, 4.0, 1.0, 1.0), (4.0, 6.0, 0.5, 2.0), (6.0, 10.0, 1.0, 1.0)]);
        // Pegado al inicio o al final no deja tramos vacíos; sin efecto real, el clip va entero.
        assert_eq!(parts(Some(Moment { start: 0.0, end: 3.0, ..m }), 10.0).len(), 2);
        assert_eq!(parts(Some(Moment { start: 8.0, end: 30.0, ..m }), 10.0), vec![(0.0, 8.0, 1.0, 1.0), (8.0, 10.0, 0.5, 2.0)]);
        assert_eq!(parts(Some(Moment { speed: 1.0, zoom: 1.0, ..m }), 10.0).len(), 1);
        assert_eq!(atempo(1.0), "");
        assert_eq!(atempo(0.5), ",atempo=0.5000");
        assert_eq!(atempo(0.25), ",atempo=0.5,atempo=0.5000");
    }

    #[test]
    fn picks_the_audio_sources_that_were_asked_for() {
        let names = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let all = names(&["mix", "game", "other", "mic"]);
        let label = |wanted: &[&str]| audio_source(&all, Some(&names(wanted)));
        assert_eq!(label(&["game", "other", "mic"]), Some((String::new(), "[0:a:0]".into())), "todo = la mezcla");
        assert_eq!(label(&["game"]), Some((String::new(), "[0:a:1]".into())));
        assert_eq!(label(&["game", "mic"]), Some(("[0:a:1][0:a:3]amix=inputs=2:duration=first:normalize=0[asrc];".into(), "[asrc]".into())));
        assert_eq!(label(&[]), None);
        // Sin pedido, o en un video de una sola pista, se usa lo que hay.
        assert_eq!(audio_source(&all, None).unwrap().1, "[0:a:0]");
        assert_eq!(audio_source(&names(&["SoundHandler"]), Some(&names(&["game"]))).unwrap().1, "[0:a:0]");
        assert_eq!(audio_source(&[], None), None);
        // Cuando el juego no se pudo separar, su audio es el del sistema.
        assert_eq!(audio_source(&names(&["mix", "system", "mic"]), Some(&names(&["game"]))).unwrap().1, "[0:a:1]");
    }

    #[test]
    fn text_goes_through_a_file_and_can_be_timed() {
        let t = Text { text: "x".into(), y: 0.1, size: 0.06, color: "accent".into(), from: 0.0, to: 0.0 };
        let f = text_filter(&t, Path::new("C:\\tmp\\o'k\\0.txt"), 1080);
        assert!(f.contains("textfile='C\\:/tmp/o'\\''k/0.txt'") && f.contains(":fontsize=65:fontcolor=0x39FF88:borderw=5:") && !f.contains("enable"), "{f}");
        let timed = text_filter(&Text { from: 1.0, to: 3.5, ..t }, Path::new("a.txt"), 720);
        assert!(timed.ends_with(":enable='between(t\\,1.000\\,3.500)'"), "{timed}");
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
        let p = Probe { duration: 16.0, width: 1920, height: 1080, fps: 60.0, audio: vec![] };
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
            let job = Job { src: &src, cut: Some((1.0, dur - 2.0)), quality: q, vertical, badge: montage::vertical_badge(3, false), mobile: mobile.then(MobileLayout::default), fx: Default::default() };
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
