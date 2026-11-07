//! Biblioteca de partidas en disco: una carpeta por partida con `match.mp4`, `meta.json`,
//! miniatura y clips.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::capture::video::{CREATE_NO_WINDOW, VideoOptions, encoder_args};
use crate::fortnite::log::{LogEvent, Timed};
use crate::fortnite::replay::{PlayerRef, Replay};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MatchEvent {
    /// Segundos desde el inicio del video.
    pub t: f64,
    /// kill | knock | death | knocked | victory | highlight | bus | bookmark
    pub kind: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct DriveRef {
    pub file_id: String,
    pub link: String,
    /// Tamaño del archivo subido (puede ser una copia comprimida, más chica que la local).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: String,
    pub file: String,
    #[serde(default)]
    pub thumb: Option<String>,
    pub start: f64,
    pub end: f64,
    pub title: String,
    pub kind: String,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub drive: Option<DriveRef>,
    /// El archivo solo está en Drive (la copia local se borró después de subirlo).
    #[serde(default)]
    pub offloaded: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MatchStatus {
    #[default]
    Recording,
    Processing,
    Ready,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct MatchMeta {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub title: String,
    pub mode: Option<String>,
    pub playlist: Option<String>,
    pub map: Option<String>,
    pub duration_s: f64,
    pub width: u32,
    pub height: u32,
    pub size_bytes: u64,
    pub video: Option<String>,
    pub thumb: Option<String>,
    pub kills: u32,
    pub knocks: u32,
    /// Cofres abiertos (leídos de los avisos de PE en el video).
    pub chests: u32,
    pub died: bool,
    pub victory: bool,
    /// Resumen de fin de partida del replay (no está si el jugador salió antes).
    pub placement: Option<u32>,
    pub total_players: Option<u32>,
    /// 0–1.
    pub accuracy: Option<f32>,
    pub assists: Option<u32>,
    pub damage_dealt: Option<u32>,
    pub damage_taken: Option<u32>,
    pub damage_to_structures: Option<u32>,
    pub revives: Option<u32>,
    pub materials_gathered: Option<u32>,
    pub materials_used: Option<u32>,
    pub distance_m: Option<f64>,
    pub events: Vec<MatchEvent>,
    pub clips: Vec<Clip>,
    pub drive: Option<DriveRef>,
    pub replay: Option<String>,
    /// Cuenta de Epic del jugador en esa partida (sirve para re-analizar el replay más tarde).
    pub account_id: Option<String>,
    pub status: MatchStatus,
    pub error: Option<String>,
    /// Conservada: no vence. Antes se llamaba "favorita".
    #[serde(alias = "favorite")]
    pub keep: bool,
    /// El video completo solo está en Drive.
    pub offloaded: bool,
}

impl MatchMeta {
    /// Archivos de esta partida que están en Drive.
    pub fn drive_files(&self) -> Vec<&DriveRef> {
        self.drive.iter().chain(self.clips.iter().filter_map(|c| c.drive.as_ref())).collect()
    }

    /// Nombre del archivo en Drive para la partida o uno de sus clips.
    pub fn drive_name(&self, clip: Option<&Clip>) -> String {
        let day = self.created_at.with_timezone(&chrono::Local).format("%Y-%m-%d %H.%M");
        let clean = |s: &str| s.chars().map(|c| if r#"\/:*?"<>|"#.contains(c) { '-' } else { c }).collect::<String>();
        match clip {
            Some(c) => format!("{day} - {} - {}.mp4", clean(&self.title), clean(&c.title)),
            None => format!("{day} - {}.mp4", clean(&self.title)),
        }
    }

    /// Copia el resumen de fin de partida del replay. Sin resumen (p. ej. al re-analizar sin
    /// replay) se conservan los datos que ya había. Puesto 1 y victoria siempre coinciden.
    pub fn apply_stats(&mut self, stats: Option<&crate::fortnite::replay::MatchStats>) {
        if let Some(s) = stats {
            self.placement = s.placement;
            self.total_players = s.total_players;
            self.accuracy = s.accuracy;
            self.assists = s.assists;
            self.damage_dealt = s.damage_dealt;
            self.damage_taken = s.damage_taken;
            self.damage_to_structures = s.damage_to_structures;
            self.revives = s.revives;
            self.materials_gathered = s.materials_gathered;
            self.materials_used = s.materials_used;
            self.distance_m = s.distance_m;
        }
        if self.placement == Some(1) {
            self.victory = true;
        } else if self.victory && self.placement.is_none() {
            self.placement = Some(1);
        }
    }
}

#[derive(Clone)]
pub struct Library {
    pub root: PathBuf,
}

impl Library {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn dir(&self, id: &str) -> PathBuf {
        self.root.join(id)
    }

    pub fn list(&self) -> Vec<MatchMeta> {
        let mut v: Vec<MatchMeta> = std::fs::read_dir(&self.root)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| self.load(&e.file_name().to_string_lossy()).ok())
            .collect();
        v.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        v
    }

    pub fn load(&self, id: &str) -> Result<MatchMeta> {
        if id.contains(['/', '\\']) || id.contains("..") {
            bail!("id inválido");
        }
        let s = std::fs::read_to_string(self.dir(id).join("meta.json"))?;
        Ok(serde_json::from_str(&s)?)
    }

    pub fn save(&self, m: &MatchMeta) -> Result<()> {
        let dir = self.dir(&m.id);
        std::fs::create_dir_all(&dir)?;
        let tmp = dir.join("meta.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(m)?)?;
        std::fs::rename(tmp, dir.join("meta.json"))?;
        Ok(())
    }

    pub fn delete(&self, id: &str) -> Result<()> {
        self.load(id)?;
        std::fs::remove_dir_all(self.dir(id))?;
        Ok(())
    }

    /// Ruta absoluta de un archivo dentro de la carpeta de una partida.
    pub fn file(&self, id: &str, name: &str) -> PathBuf {
        self.dir(id).join(name)
    }
}

/// Lo que el motor recolecta del log mientras dura la grabación.
#[derive(Debug, Clone, Default)]
pub struct MatchContext {
    pub account_id: Option<String>,
    pub map: Option<String>,
    pub playlist: Option<String>,
    pub presence: Option<String>,
    pub log: Vec<Timed>,
    /// Momentos marcados con el atajo de teclado.
    pub bookmarks: Vec<DateTime<Utc>>,
    /// Pedidos de "guardar los últimos N segundos" hechos durante la grabación.
    pub clip_requests: Vec<DateTime<Utc>>,
}

impl MatchContext {
    pub fn mode_name(&self) -> Option<String> {
        static SUFFIX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*-\s*\d+\s+\S+$").unwrap());
        self.presence.as_ref().map(|p| SUFFIX.replace(p, "").trim().to_string()).filter(|s| !s.is_empty())
    }

    pub fn victory_at(&self) -> Option<DateTime<Utc>> {
        // WaitingPostMatch llega cuando termina la partida para todos; si el jugador ya había
        // terminado antes (muerte), no es victoria.
        let end = self.log.iter().find(|t| t.event == LogEvent::MatchState("WaitingPostMatch".into()))?.at;
        let finished_before = self.log.iter().any(|t| t.event == LogEvent::PlayerFinished && t.at < end - chrono::Duration::seconds(2));
        (!finished_before).then_some(end)
    }
}

fn secs_between(from: DateTime<Utc>, to: DateTime<Utc>) -> f64 {
    (to - from).num_milliseconds() as f64 / 1000.0
}

pub struct Summary {
    pub events: Vec<MatchEvent>,
    pub kills: u32,
    pub knocks: u32,
    pub died: bool,
    pub victory: bool,
}

/// Arma la línea de tiempo combinando el replay (eliminaciones exactas) y el log.
pub fn build_events(ctx: &MatchContext, replay: Option<&Replay>, video_start: DateTime<Utc>, duration: f64) -> Summary {
    let mut events = Vec::new();
    let mut kills = 0;
    let mut knocks = 0;
    let mut died = false;
    let in_video = |t: f64| t >= 0.0 && t <= duration + 1.0;

    for t in &ctx.log {
        if t.event == LogEvent::GamePhase("Aircraft".into()) {
            let s = secs_between(video_start, t.at);
            if in_video(s) {
                events.push(MatchEvent { t: s, kind: "bus".into(), label: "Autobús".into(), distance_m: None });
            }
        }
    }

    match (replay, &ctx.account_id) {
        (Some(rep), Some(me)) => {
            for e in &rep.eliminations {
                let at = rep.started_at + chrono::Duration::milliseconds(e.time_ms as i64);
                let s = secs_between(video_start, at);
                if !in_video(s) {
                    continue;
                }
                let self_elim = e.eliminator == e.eliminated;
                if e.eliminator.is(me) && !e.eliminated.is(me) {
                    let target = match &e.eliminated {
                        PlayerRef::Bot | PlayerRef::NamedBot(_) => " (bot)",
                        PlayerRef::Player(_) => "",
                    };
                    let long = e.distance_m.filter(|d| *d >= 100.0);
                    let (kind, label) = if e.knocked {
                        knocks += 1;
                        ("knock", format!("Derribo{target}"))
                    } else {
                        kills += 1;
                        match long {
                            Some(d) => ("kill", format!("Tiro largo · {d:.0} m{target}")),
                            None => ("kill", format!("Eliminación{target}")),
                        }
                    };
                    events.push(MatchEvent { t: s, kind: kind.into(), label, distance_m: e.distance_m });
                } else if e.eliminated.is(me) {
                    if e.knocked {
                        events.push(MatchEvent { t: s, kind: "knocked".into(), label: "Te derribaron".into(), distance_m: None });
                    } else {
                        died = true;
                        let label = if self_elim { "Eliminado (tormenta/caída)" } else { "Te eliminaron" };
                        events.push(MatchEvent { t: s, kind: "death".into(), label: label.into(), distance_m: e.distance_m });
                    }
                }
            }
        }
        _ => {
            // Sin replay: usamos los avisos de highlight que Fortnite manda a NVIDIA.
            for t in &ctx.log {
                if t.event == LogEvent::NvHighlight {
                    let s = secs_between(video_start, t.at) - 4.5;
                    if in_video(s) {
                        events.push(MatchEvent { t: s.max(0.0), kind: "highlight".into(), label: "Momento destacado".into(), distance_m: None });
                    }
                }
            }
        }
    }

    for b in &ctx.bookmarks {
        let s = secs_between(video_start, *b);
        if in_video(s) {
            events.push(MatchEvent { t: s, kind: "bookmark".into(), label: "Momento marcado".into(), distance_m: None });
        }
    }

    let victory_at = ctx.victory_at();
    if let Some(v) = victory_at {
        let s = secs_between(video_start, v);
        if in_video(s) {
            events.push(MatchEvent { t: s, kind: "victory".into(), label: "¡Victoria magistral!".into(), distance_m: None });
        }
    } else if replay.is_none()
        && let Some(f) = ctx.log.iter().find(|t| t.event == LogEvent::PlayerFinished)
    {
        died = true;
        let s = secs_between(video_start, f.at);
        if in_video(s) {
            events.push(MatchEvent { t: s, kind: "death".into(), label: "Te eliminaron".into(), distance_m: None });
        }
    }

    events.sort_by(|a, b| a.t.total_cmp(&b.t));
    Summary { events, kills, knocks, died, victory: victory_at.is_some() }
}

/// Ventanas de clip automáticas: se agrupan eventos cercanos en un solo clip.
pub fn clip_windows(events: &[MatchEvent], before: f64, after: f64, duration: f64, include_deaths: bool) -> Vec<(f64, f64, String, String)> {
    let mut wins: Vec<(f64, f64, Vec<&MatchEvent>)> = Vec::new();
    for e in events {
        let wanted = match e.kind.as_str() {
            "kill" | "knock" | "victory" | "highlight" => true,
            "death" => include_deaths,
            _ => false,
        };
        if !wanted {
            continue;
        }
        let (s, end) = ((e.t - before).max(0.0), (e.t + after).min(duration));
        match wins.last_mut() {
            Some(w) if s <= w.1 => {
                w.1 = w.1.max(end);
                w.2.push(e);
            }
            _ => wins.push((s, end, vec![e])),
        }
    }
    wins.into_iter()
        .map(|(s, e, evs)| {
            let kills = evs.iter().filter(|e| e.kind == "kill").count();
            let kind = if evs.iter().any(|e| e.kind == "victory") {
                "victory"
            } else if kills > 0 {
                "kill"
            } else {
                evs[0].kind.as_str()
            };
            let title = match (kind, kills) {
                ("victory", _) => "Victoria magistral".to_string(),
                (_, 2) => "Doble eliminación".to_string(),
                (_, 3) => "Triple eliminación".to_string(),
                (_, k) if k >= 4 => format!("Multi eliminación ×{k}"),
                _ => evs[0].label.clone(),
            };
            (s, e, title, kind.to_string())
        })
        .collect()
}

// ---------- ffmpeg helpers ----------

fn run(cmd: &mut Command) -> Result<std::process::Output> {
    let o = cmd.creation_flags(CREATE_NO_WINDOW).output().context("ejecutando ffmpeg")?;
    if !o.status.success() {
        bail!("ffmpeg: {}", String::from_utf8_lossy(&o.stderr).lines().last().unwrap_or(""));
    }
    Ok(o)
}

pub struct Probe {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
}

pub fn probe(ffmpeg: &Path, video: &Path) -> Result<Probe> {
    static DUR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Duration: (\d+):(\d+):(\d+\.\d+)").unwrap());
    static RES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Video: .*?, (\d{2,5})x(\d{2,5})").unwrap());
    let o = Command::new(ffmpeg).args(["-hide_banner", "-i"]).arg(video).creation_flags(CREATE_NO_WINDOW).output()?;
    let s = String::from_utf8_lossy(&o.stderr);
    let d = DUR.captures(&s).context("sin duración")?;
    let duration = d[1].parse::<f64>()? * 3600.0 + d[2].parse::<f64>()? * 60.0 + d[3].parse::<f64>()?;
    let (width, height) = RES.captures(&s).map(|c| (c[1].parse().unwrap_or(0), c[2].parse().unwrap_or(0))).unwrap_or((0, 0));
    Ok(Probe { duration, width, height })
}

pub fn thumbnail(ffmpeg: &Path, video: &Path, at: f64, out: &Path) -> Result<()> {
    run(Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{at:.2}"), "-i"])
        .arg(video)
        .args(["-frames:v", "1", "-vf", "scale=640:-2", "-q:v", "4"])
        .arg(out))?;
    Ok(())
}

/// Corte rápido sin recodificar (arranca en el keyframe anterior, cada 2 s).
pub fn cut_copy(ffmpeg: &Path, video: &Path, start: f64, end: f64, out: &Path) -> Result<()> {
    run(Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{start:.3}"), "-i"])
        .arg(video)
        .args(["-t", &format!("{:.3}", end - start), "-c", "copy", "-avoid_negative_ts", "make_zero", "-movflags", "+faststart"])
        .arg(out))?;
    Ok(())
}

/// Corte exacto al frame, recodificando con NVENC (para el editor).
pub fn cut_precise(ffmpeg: &Path, video: &Path, start: f64, end: f64, opts: &VideoOptions, out: &Path) -> Result<()> {
    run(Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-ss", &format!("{start:.3}"), "-i"])
        .arg(video)
        .args(["-t", &format!("{:.3}", end - start)])
        .args(encoder_args(opts))
        .args(["-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart"])
        .arg(out))?;
    Ok(())
}

pub fn new_id(prefix: &str) -> String {
    use rand::Rng;
    let n: u32 = rand::rng().random();
    format!("{prefix}{n:08x}")
}

#[cfg(test)]
mod replay_alignment_tests {
    use super::*;

    /// Usa la última partida real de la biblioteca: `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn latest_match_gets_replay_kills() {
        let root = PathBuf::from(std::env::var("USERPROFILE").unwrap()).join("Videos").join("ggswp");
        let lib = Library::new(root);
        let m = lib.list().into_iter().find(|m| !m.id.ends_with("_clip")).unwrap();
        let end = m.created_at + chrono::Duration::seconds(m.duration_s as i64);
        let path = crate::fortnite::replay::find_for_window(m.created_at, end).expect("replay");
        let rep = crate::fortnite::replay::parse_file(&path).unwrap();
        let ctx = MatchContext { account_id: crate::fortnite::log::last_account_id(), ..Default::default() };
        let sum = build_events(&ctx, Some(&rep), m.created_at, m.duration_s);
        println!("{} -> {} | cuenta {:?}", m.id, path.display(), ctx.account_id);
        for e in &sum.events {
            println!("{:>7.1}s {} {}", e.t, e.kind, e.label);
        }
        assert!(sum.kills > 0);
    }
}
