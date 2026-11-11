//! Piezas para compartir: el resumen automático de una partida (sus mejores momentos en un
//! solo video) y la versión vertical 9:16 de un clip (TikTok, Shorts, Reels).

use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use tauri::{AppHandle, Emitter};

use crate::capture::video::{CREATE_NO_WINDOW, VideoOptions};
use crate::export::{self, Quality};
use crate::engine::{self, Shared};
use crate::library::{self, Clip, MatchEvent, MatchMeta, MatchStatus};

/// Duración máxima del resumen.
const REEL_MAX_S: f64 = 75.0;
/// Un tramo más largo se recorta por el principio: las eliminaciones quedan al final.
const SEGMENT_MAX_S: f64 = 30.0;
/// Fundido a negro al salir de cada momento y desde negro al entrar (0,3 s entre momentos).
const FADE_S: f64 = 0.15;
const ACCENT: &str = "0x39FF88";
pub(crate) const FONT_FILE: &str = "C:/Windows/Fonts/bahnschrift.ttf";

/// Tramos del resumen, en orden cronológico: eliminaciones, derribos, victoria y momentos
/// marcados; si son pocos, también los cofres. Si no entran todos, se quedan los mejores.
pub fn reel_windows(events: &[MatchEvent], duration: f64) -> Vec<(f64, f64)> {
    let mut wins: Vec<(f64, f64)> = library::clip_windows(events, 8.0, 3.0, duration, false).into_iter().map(|(s, e, ..)| (s, e)).collect();
    if wins.len() < 3 {
        wins.extend(events.iter().filter(|e| e.kind == "chest").map(|e| ((e.t - 4.0).max(0.0), (e.t + 3.0).min(duration))));
    }
    wins.retain(|(s, e)| e - s >= 1.0);
    wins.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (s, e) in wins {
        match merged.last_mut() {
            Some(m) if s <= m.1 + 1.0 => m.1 = m.1.max(e),
            _ => merged.push((s, e)),
        }
    }
    for w in &mut merged {
        w.0 = w.0.max(w.1 - SEGMENT_MAX_S);
    }
    let score = |s: f64, e: f64| -> f64 {
        events
            .iter()
            .filter(|ev| ev.t >= s && ev.t <= e)
            .map(|ev| match ev.kind.as_str() {
                "victory" => 100.0,
                "kill" => 10.0,
                "knock" | "highlight" => 3.0,
                "chest" => 0.5,
                _ => 0.0,
            })
            .sum()
    };
    // Primero los de más peso; a igual peso, los más tardíos (el final de la partida).
    let mut ranked: Vec<(f64, f64, f64)> = merged.into_iter().map(|(s, e)| (s, e, score(s, e))).collect();
    ranked.sort_by(|a, b| b.2.total_cmp(&a.2).then(b.0.total_cmp(&a.0)));
    let mut picked = Vec::new();
    let mut total = 0.0;
    for (s, e, _) in ranked {
        if total + (e - s) <= REEL_MAX_S {
            total += e - s;
            picked.push((s, e));
        }
    }
    picked.sort_by(|a, b| a.0.total_cmp(&b.0));
    picked
}

pub fn reel_title(kills: u32, victory: bool) -> String {
    let elim = if kills == 1 { "1 eliminación".to_string() } else { format!("{kills} eliminaciones") };
    match (victory, kills) {
        (true, 0) => "Resumen · Victoria".into(),
        (true, _) => format!("Resumen · Victoria · {elim}"),
        (false, 0) => "Resumen de la partida".into(),
        (false, _) => format!("Resumen · {elim}"),
    }
}

/// Texto grande (en verde) y etiqueta (en blanco) del marcador del video vertical.
pub fn vertical_badge(kills: u32, victory: bool) -> Option<(String, String)> {
    let label = |k: u32| if k == 1 { "ELIMINACIÓN" } else { "ELIMINACIONES" };
    match (victory, kills) {
        (true, 0) => Some(("VICTORIA".into(), "#1".into())),
        (true, k) => Some(("VICTORIA".into(), format!("{k} {}", label(k)))),
        (false, 0) => None,
        (false, k) => Some((k.to_string(), label(k).into())),
    }
}

/// Recorte central 9:16 a 1080x1920 con el marcador arriba, sobre una franja oscura.
pub fn vertical_filter(badge: Option<&(String, String)>, font_file: &str) -> String {
    let mut vf = "crop=trunc(ih*9/16/2)*2:ih:(iw-trunc(ih*9/16/2)*2)/2:0,scale=1080:1920:flags=lanczos,setsar=1".to_string();
    if let Some((big, small)) = badge {
        let font = format!("fontfile='{}'", font_file.replace(':', "\\:"));
        let size = if big.chars().count() > 3 { 170 } else { 230 };
        vf.push_str(&format!(
            ",drawbox=x=0:y=0:w=iw:h=430:color=black@0.7:t=fill\
             ,drawbox=x=(iw-120)/2:y=414:w=120:h=6:color={ACCENT}:t=fill\
             ,drawtext={font}:text='{big}':fontsize={size}:fontcolor={ACCENT}:borderw=5:bordercolor={ACCENT}:x=(w-text_w)/2:y=280-text_h\
             ,drawtext={font}:text='{small}':fontsize=58:fontcolor=white:borderw=1:bordercolor=white:x=(w-text_w)/2:y=322"
        ));
    }
    vf
}

/// Filtros de un tramo del resumen: fps constante, la escala del formato de salida (`scale`)
/// y fundidos de entrada y salida.
fn segment_filter(dur: f64, fps: u32, scale: &[String], audio: bool) -> String {
    let out = (dur - FADE_S).max(0.0);
    let scale: String = scale.iter().map(|f| format!("{f},")).collect();
    let mut f = format!("[0:v]setpts=PTS-STARTPTS,fps={fps},{scale}format=yuv420p,fade=t=in:st=0:d={FADE_S},fade=t=out:st={out:.3}:d={FADE_S}[v]");
    if audio {
        f.push_str(&format!(
            ";[0:a]aresample=async=1:first_pts=0,apad,atrim=0:{dur:.3},asetpts=PTS-STARTPTS,afade=t=in:st=0:d={FADE_S},afade=t=out:st={out:.3}:d={FADE_S}[a]"
        ));
    }
    f
}

pub(crate) fn ffmpeg_cmd(ffmpeg: &Path) -> Command {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1", "-y"]);
    c
}

/// Ejecuta ffmpeg informando los segundos ya escritos (de `-progress`).
pub(crate) fn run_progress(cmd: &mut Command, on_secs: &mut dyn FnMut(f64)) -> Result<()> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("ejecutando ffmpeg")?;
    let mut stderr = child.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    for line in BufReader::new(child.stdout.take().unwrap()).lines().map_while(|l| l.ok()) {
        if let Some(us) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<i64>().ok()) {
            on_secs(us.max(0) as f64 / 1e6);
        }
    }
    let status = child.wait()?;
    let errors = errors.join().unwrap_or_default();
    if !status.success() {
        // drawtext hace que ffmpeg imprima avisos de Fontconfig sin importancia.
        let last = errors.lines().filter(|l| !l.contains("Fontconfig")).last().unwrap_or("");
        bail!("ffmpeg: {last}");
    }
    Ok(())
}

fn has_audio(ffmpeg: &Path, video: &Path) -> bool {
    Command::new(ffmpeg)
        .args(["-hide_banner", "-i"])
        .arg(video)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stderr).contains("Audio:"))
        .unwrap_or(false)
}

/// Carpeta temporal que se borra al salir.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Codifica cada tramo por separado (con fundidos) y los une sin recodificar el video. El
/// audio va en PCM en los intermedios y se codifica una sola vez al final, así no se corre
/// en las uniones.
pub fn render_reel(ffmpeg: &Path, video: &Path, windows: &[(f64, f64)], quality: Quality, opts: &VideoOptions, out: &Path, on_pct: &dyn Fn(f32)) -> Result<()> {
    let audio = has_audio(ffmpeg, video);
    let src = library::probe(ffmpeg, video).unwrap_or_default();
    let tmp = TempDir(std::env::temp_dir().join(format!("ggswp-{}", library::new_id("reel"))));
    std::fs::create_dir_all(&tmp.0)?;
    let total: f64 = windows.iter().map(|(s, e)| e - s).sum::<f64>().max(0.1);
    // Los tramos ya salen a `opts.fps`: el plan solo aporta la escala y el codificador.
    let plan = export::plan(quality, (src.width, src.height), opts.fps as f64, total, opts);
    let mut done = 0.0;
    let mut list = String::new();
    for (i, &(start, end)) in windows.iter().enumerate() {
        let dur = end - start;
        let name = format!("seg{i:02}.mkv");
        let mut cmd = ffmpeg_cmd(ffmpeg);
        cmd.args(["-ss", &format!("{start:.3}"), "-t", &format!("{dur:.3}"), "-i"])
            .arg(video)
            .args(["-filter_complex", &segment_filter(dur, opts.fps, &plan.filters, audio), "-map", "[v]"]);
        if audio {
            cmd.args(["-map", "[a]", "-c:a", "pcm_s16le"]);
        }
        cmd.args(&plan.video).arg(tmp.0.join(&name));
        run_progress(&mut cmd, &mut |t| on_pct(((done + t.min(dur)) / total * 95.0) as f32))?;
        done += dur;
        list.push_str(&format!("file '{name}'\n"));
    }
    let list_path = tmp.0.join("list.txt");
    std::fs::write(&list_path, list)?;
    let mut cmd = ffmpeg_cmd(ffmpeg);
    cmd.args(["-f", "concat", "-safe", "0", "-i"])
        .arg(&list_path)
        .args(["-map", "0", "-c:v", "copy"])
        .args(&plan.audio)
        .args(["-movflags", "+faststart"])
        .arg(out);
    run_progress(&mut cmd, &mut |t| on_pct((95.0 + t / total * 5.0) as f32))?;
    Ok(())
}

pub(crate) fn clear_step(app: &AppHandle, shared: &Shared, id: &str) {
    let snapshot = {
        let mut st = shared.status.lock().unwrap();
        st.steps.remove(id);
        st.clone()
    };
    let _ = app.emit("status", snapshot);
}

fn local_match(shared: &Shared, id: &str) -> Result<MatchMeta> {
    let m = shared.library().load(id)?;
    if m.status != MatchStatus::Ready {
        bail!("La partida todavía se está procesando");
    }
    if m.offloaded {
        bail!("El video está solo en Drive: descárgalo primero");
    }
    Ok(m)
}

/// Crea el resumen de la partida y reemplaza el anterior. Devuelve el clip nuevo y los
/// resúmenes reemplazados (por si hay que mandar su copia de Drive a la papelera).
pub fn create_reel(app: &AppHandle, shared: &Shared, id: &str) -> Result<(Clip, Vec<Clip>)> {
    let m = local_match(shared, id)?;
    let windows = reel_windows(&m.events, m.duration_s);
    if windows.is_empty() {
        bail!("Esta partida no tiene momentos para un resumen: no hubo eliminaciones, derribos ni cofres");
    }
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = engine::ffmpeg_path(app, &settings);
    let lib = shared.library();
    let video = lib.file(id, m.video.as_deref().unwrap_or("match.mp4"));
    let cid = library::new_id("reel");
    let file = format!("clips/{cid}.mp4");
    let out = lib.file(id, &file);
    std::fs::create_dir_all(lib.file(id, "clips"))?;
    shared.set_step(app, id, "Creando resumen", Some(0.0));
    let started = std::time::Instant::now();
    let quality = Quality::from_setting(&settings.clip_quality);
    let result = render_reel(&ffmpeg, &video, &windows, quality, &engine::video_options(&settings, 0), &out, &|p| {
        shared.set_step(app, id, "Creando resumen", Some(p))
    });
    clear_step(app, shared, id);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    let length: f64 = windows.iter().map(|(s, e)| e - s).sum();
    log::info!("{id}: resumen de {} momentos ({length:.0} s) en {:.0} s", windows.len(), started.elapsed().as_secs_f64());
    let thumb = format!("clips/{cid}.jpg");
    let first = windows[0];
    let thumb_ok = library::thumbnail(&ffmpeg, &out, (first.1 - first.0 - 3.5).max(0.0), &lib.file(id, &thumb)).is_ok();
    let clip = Clip {
        id: cid,
        file,
        thumb: thumb_ok.then_some(thumb),
        start: 0.0,
        end: length,
        title: reel_title(m.kills, m.victory),
        kind: "reel".into(),
        created_at: chrono::Utc::now(),
        drive: None,
        offloaded: false,
    };
    // Se vuelve a leer: la partida pudo cambiar mientras se codificaba.
    let mut m = lib.load(id)?;
    let (old, keep): (Vec<Clip>, Vec<Clip>) = std::mem::take(&mut m.clips).into_iter().partition(|c| c.kind == "reel");
    m.clips = keep;
    for c in &old {
        lib.remove_clip_files(&m, c);
    }
    m.clips.push(clip.clone());
    lib.save(&m)?;
    Ok((clip, old))
}

/// Versión vertical 9:16 de un clip (o del resumen), con el marcador de la partida.
pub fn create_vertical(app: &AppHandle, shared: &Shared, id: &str, clip_id: &str) -> Result<Clip> {
    let m = local_match(shared, id)?;
    let src = m.clips.iter().find(|c| c.id == clip_id).context("clip no encontrado")?.clone();
    if src.kind == "vertical" {
        bail!("Este clip ya es vertical");
    }
    if src.offloaded {
        bail!("El clip está solo en Drive: descárgalo primero");
    }
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = engine::ffmpeg_path(app, &settings);
    let lib = shared.library();
    let input = lib.file(id, &src.file);
    let cid = library::new_id("vert");
    let file = format!("clips/{cid}.mp4");
    let out = lib.file(id, &file);
    let job = export::Job { src: &input, cut: None, quality: Quality::from_setting(&settings.clip_quality), vertical: true, badge: vertical_badge(m.kills, m.victory), mobile: None };
    shared.set_step(app, id, "Creando versión vertical", Some(0.0));
    let started = std::time::Instant::now();
    let result = export::render(&ffmpeg, &job, &engine::video_options(&settings, 0), &out, &|p| shared.set_step(app, id, "Creando versión vertical", Some(p)));
    clear_step(app, shared, id);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    log::info!("{id}: versión vertical de {clip_id} en {:.0} s", started.elapsed().as_secs_f64());
    let thumb = format!("clips/{cid}.jpg");
    let thumb_ok = library::thumbnail(&ffmpeg, &out, ((src.end - src.start) / 2.0).min(3.0), &lib.file(id, &thumb)).is_ok();
    let clip = Clip {
        id: cid,
        file,
        thumb: thumb_ok.then_some(thumb),
        start: src.start,
        end: src.end,
        title: format!("{} · vertical", src.title),
        kind: "vertical".into(),
        created_at: chrono::Utc::now(),
        drive: None,
        offloaded: false,
    };
    let mut m = lib.load(id)?;
    m.clips.push(clip.clone());
    lib.save(&m)?;
    Ok(clip)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(t: f64, kind: &str) -> MatchEvent {
        MatchEvent { t, kind: kind.into(), label: kind.into(), distance_m: None }
    }

    #[test]
    fn reel_uses_kills_and_skips_chests_when_enough() {
        let events = vec![ev(30.0, "chest"), ev(100.0, "kill"), ev(200.0, "knock"), ev(300.0, "kill"), ev(301.0, "victory")];
        let w = reel_windows(&events, 400.0);
        assert_eq!(w, vec![(92.0, 103.0), (192.0, 203.0), (292.0, 304.0)]);
    }

    #[test]
    fn reel_adds_chests_when_few_moments() {
        let events = vec![ev(54.5, "chest"), ev(91.5, "chest"), ev(98.5, "chest"), ev(150.0, "kill")];
        let w = reel_windows(&events, 200.0);
        assert_eq!(w, vec![(50.5, 57.5), (87.5, 101.5), (142.0, 153.0)]);
    }

    #[test]
    fn reel_is_capped_and_keeps_the_best() {
        // 12 eliminaciones separadas (11 s cada una) y una victoria al final.
        let mut events: Vec<_> = (0..12).map(|i| ev(100.0 + i as f64 * 60.0, "kill")).collect();
        events.push(ev(900.0, "knock"));
        events.push(ev(1000.0, "victory"));
        let w = reel_windows(&events, 1100.0);
        let total: f64 = w.iter().map(|(s, e)| e - s).sum();
        assert!(total <= REEL_MAX_S, "{total}");
        assert!(w.windows(2).all(|p| p[0].1 <= p[1].0), "orden cronológico");
        assert_eq!(w.last().unwrap().1, 1003.0, "la victoria entra");
        assert!(!w.iter().any(|&(s, _)| s == 892.0), "el derribo suelto queda afuera");
    }

    #[test]
    fn reel_trims_long_fights() {
        let events: Vec<_> = (0..8).map(|i| ev(100.0 + i as f64 * 9.0, "kill")).collect();
        let w = reel_windows(&events, 500.0);
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].1 - w[0].0, SEGMENT_MAX_S);
        assert_eq!(w[0].1, 166.0);
    }

    #[test]
    fn reel_empty_without_moments() {
        assert!(reel_windows(&[ev(10.0, "bus"), ev(90.0, "death")], 100.0).is_empty());
    }

    #[test]
    fn badges() {
        assert_eq!(vertical_badge(3, false), Some(("3".into(), "ELIMINACIONES".into())));
        assert_eq!(vertical_badge(1, false), Some(("1".into(), "ELIMINACIÓN".into())));
        assert_eq!(vertical_badge(2, true), Some(("VICTORIA".into(), "2 ELIMINACIONES".into())));
        assert_eq!(vertical_badge(0, false), None);
        assert_eq!(reel_title(1, false), "Resumen · 1 eliminación");
        assert_eq!(reel_title(4, true), "Resumen · Victoria · 4 eliminaciones");
    }

    /// Con una partida real de la biblioteca: `cargo test --lib montage -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn renders_real_match() {
        let root = PathBuf::from(std::env::var("USERPROFILE").unwrap()).join("Videos").join("ggswp");
        let lib = library::Library::new(root);
        let m = lib
            .list()
            .into_iter()
            .find(|m| !m.offloaded && lib.file(&m.id, "match.mp4").exists() && !reel_windows(&m.events, m.duration_s).is_empty())
            .expect("una partida local con momentos");
        let ffmpeg = Path::new("ffmpeg");
        let opts = VideoOptions { output_index: 0, fps: 60, encoder: "h264_nvenc".into(), cq: 19, max_bitrate_mbps: 80 };
        let tmp = TempDir(std::env::temp_dir().join(library::new_id("ggswp-test")));
        std::fs::create_dir_all(&tmp.0).unwrap();
        let windows = reel_windows(&m.events, m.duration_s);
        let length: f64 = windows.iter().map(|(s, e)| e - s).sum();
        let reel = tmp.0.join("reel.mp4");
        let t = std::time::Instant::now();
        render_reel(ffmpeg, &lib.file(&m.id, "match.mp4"), &windows, Quality::Share, &opts, &reel, &|_| {}).unwrap();
        println!("{}: resumen {windows:?} ({length:.1} s) en {:.1} s", m.id, t.elapsed().as_secs_f64());
        let p = library::probe(ffmpeg, &reel).unwrap();
        println!("resumen: {}x{}, {:.1} MB", p.width, p.height, std::fs::metadata(&reel).unwrap().len() as f64 / 1e6);
        assert!((p.duration - length).abs() < 0.2, "{} vs {length}", p.duration);
        assert!(export::is_light(&p, std::fs::metadata(&reel).unwrap().len()));
        let vert = tmp.0.join("vert.mp4");
        let t = std::time::Instant::now();
        let job = export::Job { src: &reel, cut: None, quality: Quality::Share, vertical: true, badge: vertical_badge(3, false), mobile: None };
        export::render(ffmpeg, &job, &opts, &vert, &|_| {}).unwrap();
        println!("vertical en {:.1} s", t.elapsed().as_secs_f64());
        let p = library::probe(ffmpeg, &vert).unwrap();
        assert_eq!((p.width, p.height), (1080, 1920));
    }

    #[test]
    fn vertical_filter_escapes_font_path() {
        let f = vertical_filter(Some(&("3".into(), "ELIMINACIONES".into())), FONT_FILE);
        assert!(f.contains("fontfile='C\\:/Windows/Fonts/bahnschrift.ttf'"));
        assert!(f.starts_with("crop=trunc(ih*9/16/2)*2:ih"));
        assert!(!vertical_filter(None, FONT_FILE).contains("drawtext"));
    }
}
