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
/// Duración máxima del resumen semanal.
const WEEK_MAX_S: f64 = 90.0;
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
    let score = |s: f64, e: f64| window_score(events, s, e);
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

/// Peso de un tramo: la victoria manda, después las eliminaciones (más si fueron varias
/// seguidas, de lejos, una remontada o la que dio la victoria) y por último los derribos.
pub fn window_score(events: &[MatchEvent], s: f64, e: f64) -> f64 {
    let inside: Vec<&MatchEvent> = events.iter().filter(|ev| ev.t >= s && ev.t <= e).collect();
    let kills = inside.iter().filter(|ev| ev.kind == "kill").count();
    let base: f64 = inside
        .iter()
        .map(|ev| match ev.kind.as_str() {
            "victory" => 100.0,
            "kill" => {
                let far = ev.distance_m.filter(|d| *d >= 100.0).map_or(0.0, |d| 5.0 + d / 50.0);
                let special = if ev.label.starts_with("Remontada") || ev.label.contains("de la victoria") { 8.0 } else { 0.0 };
                let bot = if ev.label.contains("(bot)") { -4.0 } else { 0.0 };
                10.0 + far + special + bot
            }
            "knock" | "highlight" => 3.0,
            "chest" => 0.5,
            _ => 0.0,
        })
        .sum();
    // Varias eliminaciones en el mismo tramo valen más que sueltas.
    base + if kills >= 2 { 6.0 * (kills - 1) as f64 } else { 0.0 }
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

/// Carpeta temporal que se borra al salir.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Un tramo de un video para unir con otros.
#[derive(Debug, Clone, PartialEq)]
pub struct Seg {
    pub src: PathBuf,
    pub start: f64,
    pub dur: f64,
}

/// Filtros de un tramo: fps constante, encajado en `dims` (con bandas si la proporción es
/// otra), los filtros del formato (`extra`) y fundidos de entrada y salida. `audio` es la
/// entrada de audio del tramo (con sus filtros previos).
fn segment_filter(dur: f64, fps: u32, (w, h): (u32, u32), extra: &[String], audio: &(String, String)) -> String {
    let out = (dur - FADE_S).max(0.0);
    let extra: String = extra.iter().map(|f| format!("{f},")).collect();
    let fit = if w > 0 { format!("scale={w}:{h}:force_original_aspect_ratio=decrease:flags=lanczos,pad={w}:{h}:(ow-iw)/2:(oh-ih)/2,setsar=1,") } else { String::new() };
    format!(
        "[0:v:0]setpts=PTS-STARTPTS,fps={fps},{fit}{extra}format=yuv420p,fade=t=in:st=0:d={FADE_S},fade=t=out:st={out:.3}:d={FADE_S}[v];\
         {}{}aresample=48000:async=1:first_pts=0,aformat=channel_layouts=stereo,apad,atrim=0:{dur:.3},asetpts=PTS-STARTPTS,afade=t=in:st=0:d={FADE_S},afade=t=out:st={out:.3}:d={FADE_S}[a]",
        audio.0, audio.1
    )
}

/// Codifica cada tramo por separado (con fundidos) y los une sin recodificar el video. El
/// audio va en PCM en los intermedios y se codifica una sola vez al final, así no se corre
/// en las uniones. Todos salen al tamaño del primero. `tracks` son las fuentes de audio a
/// incluir de las grabaciones con pistas separadas.
pub fn render_segments(ffmpeg: &Path, segs: &[Seg], quality: Quality, opts: &VideoOptions, tracks: Option<&[String]>, out: &Path, on_pct: &dyn Fn(f32)) -> Result<()> {
    let first = segs.first().context("no hay tramos para unir")?;
    let tmp = TempDir(std::env::temp_dir().join(format!("ggswp-{}", library::new_id("reel"))));
    std::fs::create_dir_all(&tmp.0)?;
    let total: f64 = segs.iter().map(|s| s.dur).sum::<f64>().max(0.1);
    let base = library::probe(ffmpeg, &first.src).unwrap_or_default();
    let dims = export::output_dims(quality, (base.width, base.height));
    // Los tramos ya salen a `opts.fps` y a `dims`: del plan se usa el resto (fps del formato
    // liviano) y el codificador.
    let plan = export::plan(quality, (base.width, base.height), opts.fps as f64, total, opts);
    let extra: Vec<String> = plan.filters.iter().filter(|f| !f.starts_with("scale=")).cloned().collect();
    let mut done = 0.0;
    let mut list = String::new();
    for (i, seg) in segs.iter().enumerate() {
        let name = format!("seg{i:02}.mkv");
        let names = library::probe(ffmpeg, &seg.src).map(|p| p.audio).unwrap_or_default();
        let mut cmd = ffmpeg_cmd(ffmpeg);
        cmd.args(["-ss", &format!("{:.3}", seg.start), "-t", &format!("{:.3}", seg.dur), "-i"]).arg(&seg.src);
        // Un tramo sin audio lleva silencio: todos los intermedios tienen las mismas pistas.
        let audio = export::audio_source(&names, tracks).unwrap_or_else(|| {
            cmd.args(["-f", "lavfi", "-t", &format!("{:.3}", seg.dur), "-i", "anullsrc=r=48000:cl=stereo"]);
            (String::new(), "[1:a]".into())
        });
        cmd.args(["-filter_complex", &segment_filter(seg.dur, opts.fps, dims, &extra, &audio), "-map", "[v]", "-map", "[a]", "-c:a", "pcm_s16le"]);
        cmd.args(&plan.video).arg(tmp.0.join(&name));
        run_progress(&mut cmd, &mut |t| on_pct(((done + t.min(seg.dur)) / total * 95.0) as f32))?;
        done += seg.dur;
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

/// Resumen de una partida: sus tramos unidos.
pub fn render_reel(ffmpeg: &Path, video: &Path, windows: &[(f64, f64)], quality: Quality, opts: &VideoOptions, tracks: Option<&[String]>, out: &Path, on_pct: &dyn Fn(f32)) -> Result<()> {
    let segs: Vec<Seg> = windows.iter().map(|&(s, e)| Seg { src: video.to_path_buf(), start: s, dur: e - s }).collect();
    render_segments(ffmpeg, &segs, quality, opts, tracks, out, on_pct)
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
    let result = render_reel(&ffmpeg, &video, &windows, quality, &engine::video_options(&settings, 0), Some(&export::default_tracks(&settings)), &out, &|p| {
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
        best: false,
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

/// Une varios clips de una partida, en el orden dado, en un clip nuevo.
pub fn join_clips(app: &AppHandle, shared: &Shared, id: &str, clip_ids: &[String], title: &str) -> Result<Clip> {
    let m = local_match(shared, id)?;
    if clip_ids.len() < 2 {
        bail!("Elige al menos dos clips para unir");
    }
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = engine::ffmpeg_path(app, &settings);
    let lib = shared.library();
    let mut segs = Vec::new();
    for cid in clip_ids {
        let c = m.clips.iter().find(|c| &c.id == cid).with_context(|| format!("clip {cid} no encontrado"))?;
        if c.offloaded {
            bail!("El clip \"{}\" está solo en Drive: descárgalo primero", c.title);
        }
        let src = lib.file(id, &c.file);
        let dur = library::probe(&ffmpeg, &src)?.duration;
        segs.push(Seg { src, start: 0.0, dur });
    }
    let cid = library::new_id("join");
    let file = format!("clips/{cid}.mp4");
    let out = lib.file(id, &file);
    std::fs::create_dir_all(lib.file(id, "clips"))?;
    let label = "Uniendo clips";
    shared.set_step(app, id, label, Some(0.0));
    // Los clips ya traen el audio elegido al crearlos: no se filtra de nuevo.
    let result = render_segments(&ffmpeg, &segs, Quality::from_setting(&settings.clip_quality), &engine::video_options(&settings, 0), None, &out, &|p| shared.set_step(app, id, label, Some(p)));
    clear_step(app, shared, id);
    if let Err(e) = result {
        let _ = std::fs::remove_file(&out);
        return Err(e);
    }
    let length: f64 = segs.iter().map(|s| s.dur).sum();
    let thumb = format!("clips/{cid}.jpg");
    let thumb_ok = library::thumbnail(&ffmpeg, &out, (segs[0].dur / 2.0).min(3.0), &lib.file(id, &thumb)).is_ok();
    let title = if title.trim().is_empty() { format!("Unión de {} clips", segs.len()) } else { title.trim().to_string() };
    let clip = Clip { id: cid, file, thumb: thumb_ok.then_some(thumb), start: 0.0, end: length, title, kind: "join".into(), created_at: chrono::Utc::now(), ..Default::default() };
    let mut m = lib.load(id)?;
    m.clips.push(clip.clone());
    lib.save(&m)?;
    Ok(clip)
}

/// Mejores tramos de varias partidas, hasta `max` segundos, en orden cronológico.
pub fn weekly_segments(matches: &[(PathBuf, &MatchMeta)], max: f64) -> Vec<Seg> {
    // (puntaje, orden de partida, inicio, fin, video)
    let mut ranked: Vec<(f64, usize, f64, f64, &PathBuf)> = Vec::new();
    for (i, (video, m)) in matches.iter().enumerate() {
        for (s, e, ..) in library::clip_windows(&m.events, 8.0, 3.0, m.duration_s, false) {
            let s = s.max(e - SEGMENT_MAX_S);
            if e - s >= 1.0 {
                ranked.push((window_score(&m.events, s, e), i, s, e, video));
            }
        }
    }
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut picked = Vec::new();
    let mut total = 0.0;
    for r in ranked {
        if total + (r.3 - r.2) <= max {
            total += r.3 - r.2;
            picked.push(r);
        }
    }
    picked.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.total_cmp(&b.2)));
    picked.into_iter().map(|(_, _, s, e, video)| Seg { src: (*video).clone(), start: s, dur: e - s }).collect()
}

/// Clave del paso "resumen semanal" en el estado (no pertenece a ninguna partida).
pub const WEEKLY_STEP: &str = "weekly";

/// Un video con lo mejor de los últimos 7 días. Queda en la galería como una entrada propia.
pub fn create_weekly(app: &AppHandle, shared: &Shared) -> Result<MatchMeta> {
    let settings = shared.settings.read().unwrap().clone();
    let ffmpeg = engine::ffmpeg_path(app, &settings);
    let lib = shared.library();
    let since = chrono::Utc::now() - chrono::Duration::days(7);
    let mut all: Vec<MatchMeta> = lib
        .list()
        .into_iter()
        .filter(|m| m.status == MatchStatus::Ready && !m.offloaded && m.game.is_none() && m.created_at >= since && !m.id.ends_with("_week") && !m.id.ends_with("_clip"))
        .collect();
    all.sort_by_key(|m| m.created_at);
    let sources: Vec<(PathBuf, &MatchMeta)> = all.iter().map(|m| (lib.file(&m.id, m.video.as_deref().unwrap_or("match.mp4")), m)).filter(|(p, _)| p.exists()).collect();
    let segs = weekly_segments(&sources, WEEK_MAX_S);
    if segs.is_empty() {
        bail!("No hay momentos para un resumen: en los últimos 7 días no hubo eliminaciones ni victorias en las partidas que están en la PC");
    }
    let used: Vec<&MatchMeta> = sources.iter().filter(|(p, _)| segs.iter().any(|s| &s.src == p)).map(|(_, m)| *m).collect();
    let id = format!("{}_week", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"));
    std::fs::create_dir_all(lib.dir(&id))?;
    let out = lib.file(&id, "match.mp4");
    let label = "Creando el resumen semanal";
    shared.set_step(app, WEEKLY_STEP, label, Some(0.0));
    let result = render_segments(&ffmpeg, &segs, Quality::from_setting(&settings.clip_quality), &engine::video_options(&settings, 0), Some(&export::default_tracks(&settings)), &out, &|p| {
        shared.set_step(app, WEEKLY_STEP, label, Some(p))
    });
    clear_step(app, shared, WEEKLY_STEP);
    if let Err(e) = result {
        let _ = std::fs::remove_dir_all(lib.dir(&id));
        return Err(e);
    }
    let p = library::probe(&ffmpeg, &out)?;
    let thumb = library::thumbnail(&ffmpeg, &out, (segs[0].dur - 3.5).max(0.0), &lib.file(&id, "thumb.jpg")).ok().map(|_| "thumb.jpg".to_string());
    let day = |m: &MatchMeta| m.created_at.with_timezone(&chrono::Local).format("%d/%m").to_string();
    let title = format!("Lo mejor de la semana · {} – {}", day(used[0]), day(used[used.len() - 1]));
    let now = chrono::Utc::now();
    let meta = MatchMeta {
        id: id.clone(),
        created_at: now,
        title: title.clone(),
        duration_s: p.duration,
        width: p.width,
        height: p.height,
        size_bytes: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
        video: Some("match.mp4".into()),
        thumb: thumb.clone(),
        kills: used.iter().map(|m| m.kills).sum(),
        victory: used.iter().any(|m| m.victory),
        clips: vec![Clip { id: "week".into(), file: "match.mp4".into(), thumb, start: 0.0, end: p.duration, title, kind: "reel".into(), created_at: now, ..Default::default() }],
        status: MatchStatus::Ready,
        ..Default::default()
    };
    lib.save(&meta)?;
    log::info!("{id}: resumen semanal de {} momentos de {} partidas ({:.0} s)", segs.len(), used.len(), p.duration);
    Ok(meta)
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
    let job = export::Job { src: &input, cut: None, quality: Quality::from_setting(&settings.clip_quality), vertical: true, badge: vertical_badge(m.kills, m.victory), mobile: None, fx: Default::default() };
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
        best: false,
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
    fn score_prefers_multi_kills_long_shots_and_real_players() {
        let kill = |t: f64, label: &str, d: Option<f64>| MatchEvent { t, kind: "kill".into(), label: label.into(), distance_m: d };
        let one = window_score(&[kill(10.0, "Eliminación", None)], 0.0, 20.0);
        let bot = window_score(&[kill(10.0, "Eliminación (bot)", None)], 0.0, 20.0);
        let far = window_score(&[kill(10.0, "Tiro largo · 150 m", Some(150.0))], 0.0, 20.0);
        let two = window_score(&[kill(10.0, "Eliminación", None), kill(14.0, "Eliminación", None)], 0.0, 20.0);
        let comeback = window_score(&[kill(10.0, "Remontada · eliminación tras un derribo", None)], 0.0, 20.0);
        assert!(bot < one && one < far && far < two, "{bot} {one} {far} {two}");
        assert!(comeback > one);
        assert!(window_score(&[ev(5.0, "victory")], 0.0, 20.0) > two);
        assert_eq!(window_score(&[kill(30.0, "Eliminación", None)], 0.0, 20.0), 0.0);
    }

    #[test]
    fn weekly_takes_the_best_of_every_match_in_order() {
        let m = |kills: &[f64], victory: Option<f64>| {
            let mut events: Vec<_> = kills.iter().map(|&t| ev(t, "kill")).collect();
            events.extend(victory.map(|t| ev(t, "victory")));
            MatchMeta { duration_s: 2000.0, events, ..Default::default() }
        };
        let (a, b) = (m(&[100.0, 500.0], None), m(&[300.0], Some(900.0)));
        let sources = vec![(PathBuf::from("a.mp4"), &a), (PathBuf::from("b.mp4"), &b)];
        let all = weekly_segments(&sources, 90.0);
        let seg = |src: &str, start: f64| Seg { src: PathBuf::from(src), start, dur: 11.0 };
        assert_eq!(all, vec![seg("a.mp4", 92.0), seg("a.mp4", 492.0), seg("b.mp4", 292.0), seg("b.mp4", 892.0)]);
        // Si no entra todo, queda lo más fuerte (la victoria primero) y sigue en orden.
        let short = weekly_segments(&sources, 25.0);
        assert_eq!(short.len(), 2);
        assert_eq!(short[1], seg("b.mp4", 892.0));
        assert!(weekly_segments(&[], 90.0).is_empty());
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
        render_reel(ffmpeg, &lib.file(&m.id, "match.mp4"), &windows, Quality::Share, &opts, None, &reel, &|_| {}).unwrap();
        println!("{}: resumen {windows:?} ({length:.1} s) en {:.1} s", m.id, t.elapsed().as_secs_f64());
        let p = library::probe(ffmpeg, &reel).unwrap();
        println!("resumen: {}x{}, {:.1} MB", p.width, p.height, std::fs::metadata(&reel).unwrap().len() as f64 / 1e6);
        assert!((p.duration - length).abs() < 0.2, "{} vs {length}", p.duration);
        assert!(export::is_light(&p, std::fs::metadata(&reel).unwrap().len()));
        let vert = tmp.0.join("vert.mp4");
        let t = std::time::Instant::now();
        let job = export::Job { src: &reel, cut: None, quality: Quality::Share, vertical: true, badge: vertical_badge(3, false), mobile: None, fx: Default::default() };
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
