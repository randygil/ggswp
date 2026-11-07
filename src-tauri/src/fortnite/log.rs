//! Seguimiento de `FortniteGame.log`: convierte líneas del log en eventos de partida.
//!
//! Marcadores verificados contra logs reales (build 43.00):
//! - `LogLoad: LoadMap: <ip>:<puerto>/...?EncryptionToken=<accountId>:...` → carga de partida
//! - `LogLoad: LoadMap: /Game/Maps/Frontend` → vuelta al lobby
//! - `MatchState changed previous=WaitingToStart current=InProgress` → arranca la partida
//! - `HandleGamePhaseChanged ... NewPhase = EAthenaGamePhase::Aircraft` → autobús
//! - `Match finished for <nombre>, end game` → el jugador terminó (muerte o victoria)
//! - `Posted Request HLSetVideo` → Fortnite pidió un highlight a NVIDIA (≈4,5 s después de una eliminación)

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::SystemTime;

use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;

#[derive(Debug, Clone, PartialEq)]
pub enum LogEvent {
    /// El log se reinició (Fortnite se abrió de nuevo).
    SessionStart,
    MatchLoading { account_id: Option<String>, map: String },
    Lobby,
    MatchState(String),
    GamePhase(String),
    Playlist(String),
    Presence(String),
    NvHighlight,
    PlayerFinished,
}

#[derive(Debug, Clone)]
pub struct Timed {
    pub at: DateTime<Utc>,
    pub event: LogEvent,
}

static TS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\[(\d{4}\.\d{2}\.\d{2}-\d{2}\.\d{2}\.\d{2}):(\d{3})\]").unwrap());
static LOADMAP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"LogLoad: LoadMap: (\S+)").unwrap());
static TOKEN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"EncryptionToken=([0-9a-fA-F]{32}):").unwrap());
static MAPNAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"/([A-Za-z0-9_]+)/Maps/([A-Za-z0-9_]+)").unwrap());
static MATCHSTATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"MatchState changed previous=\w+ current=(\w+)").unwrap());
static PHASE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"HandleGamePhaseChanged\. OldPhase = EAthenaGamePhase::\w+\. NewPhase = EAthenaGamePhase::(\w+)").unwrap());
static PLAYLIST: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"PlaylistName is (\S+)").unwrap());
static PRESENCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"RichText=\[([^\]]+)\]").unwrap());
static FINISHED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Match finished for .+, end game").unwrap());

/// Última cuenta de Epic que entró a una partida, buscando en el log actual y en los respaldos.
pub fn last_account_id() -> Option<String> {
    let dir = PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("FortniteGame").join("Saved").join("Logs");
    let mut files: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "log") && p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("FortniteGame"))).collect();
    files.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    files.iter().find_map(|p| {
        let bytes = std::fs::read(p).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        TOKEN.captures_iter(&text).last().map(|c| c[1].to_ascii_lowercase())
    })
}

fn parse_ts(line: &str) -> Option<DateTime<Utc>> {
    let c = TS.captures(line)?;
    let base = NaiveDateTime::parse_from_str(&c[1], "%Y.%m.%d-%H.%M.%S").ok()?;
    let ms: i64 = c[2].parse().ok()?;
    Some((base + chrono::Duration::milliseconds(ms)).and_utc())
}

pub fn parse_line(line: &str) -> Option<Timed> {
    if line.starts_with("Log file open") {
        return Some(Timed { at: Utc::now(), event: LogEvent::SessionStart });
    }
    // Filtro barato antes de las regex: el log escribe miles de líneas por minuto.
    let event = if line.contains("LogLoad: LoadMap: ") {
        let url = LOADMAP.captures(line)?.get(1)?.as_str();
        if url.contains("/Game/Maps/Frontend") {
            LogEvent::Lobby
        } else {
            let map = MAPNAME.captures(url).map(|c| c[2].to_string()).unwrap_or_default();
            let account_id = TOKEN.captures(url).map(|c| c[1].to_ascii_lowercase());
            LogEvent::MatchLoading { account_id, map }
        }
    } else if line.contains("MatchState changed") {
        LogEvent::MatchState(MATCHSTATE.captures(line)?[1].to_string())
    } else if line.contains("HandleGamePhaseChanged") {
        LogEvent::GamePhase(PHASE.captures(line)?[1].to_string())
    } else if line.contains("PlaylistName is ") {
        LogEvent::Playlist(PLAYLIST.captures(line)?[1].to_string())
    } else if line.contains("LogEOSPresence: Updating Presence") {
        LogEvent::Presence(PRESENCE.captures(line)?[1].to_string())
    } else if line.contains("Posted Request HLSetVideo") {
        LogEvent::NvHighlight
    } else if line.contains("Match finished for ") && FINISHED.is_match(line) {
        LogEvent::PlayerFinished
    } else {
        return None;
    };
    Some(Timed { at: parse_ts(line).unwrap_or_else(Utc::now), event })
}

pub fn log_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("FortniteGame\\Saved\\Logs\\FortniteGame.log"))
}

/// Lee el log de forma incremental. Abre y cierra el archivo en cada lectura para no
/// interferir con la rotación que hace Fortnite al arrancar.
pub struct LogTailer {
    path: PathBuf,
    pos: u64,
    created: Option<SystemTime>,
    partial: Vec<u8>,
}

impl LogTailer {
    pub fn new(path: PathBuf) -> Self {
        Self { path, pos: 0, created: None, partial: Vec::new() }
    }

    /// Lee todo el log actual (para reconstruir el estado al abrir la app) y deja el
    /// cursor al final.
    pub fn catch_up(&mut self) -> Vec<Timed> {
        self.pos = 0;
        self.created = None;
        self.partial.clear();
        self.poll()
            .into_iter()
            .filter(|t| t.event != LogEvent::SessionStart)
            .collect()
    }

    pub fn poll(&mut self) -> Vec<Timed> {
        let mut out = Vec::new();
        let Ok(meta) = std::fs::metadata(&self.path) else { return out };
        let created = meta.created().ok();
        if (self.created.is_some() && created != self.created) || meta.len() < self.pos {
            self.pos = 0;
            self.partial.clear();
        }
        self.created = created;
        if meta.len() == self.pos {
            return out;
        }
        let Ok(mut f) = File::open(&self.path) else { return out };
        if f.seek(SeekFrom::Start(self.pos)).is_err() {
            return out;
        }
        let mut buf = Vec::with_capacity((meta.len() - self.pos) as usize);
        if f.read_to_end(&mut buf).is_err() {
            return out;
        }
        self.pos += buf.len() as u64;

        let mut data = std::mem::take(&mut self.partial);
        data.extend_from_slice(&buf);
        let last_nl = data.iter().rposition(|&b| b == b'\n');
        let (complete, rest) = match last_nl {
            Some(i) => data.split_at(i + 1),
            None => (&data[..0], &data[..]),
        };
        for raw in complete.split(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(raw);
            let line = line.trim_start_matches('\u{feff}').trim_end();
            if let Some(t) = parse_line(line) {
                out.push(t);
            }
        }
        self.partial = rest.to_vec();
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_lines() {
        let l = "[2026.11.06-21.13.57:606][964]LogLoad: LoadMap: 3.237.227.99:9053/Delilah/Maps/Delilah_Terrain?EncryptionToken=0123456789abcdef0123456789abcdef:4b63?VerseURI=/x";
        let t = parse_line(l).unwrap();
        assert_eq!(t.event, LogEvent::MatchLoading { account_id: Some("0123456789abcdef0123456789abcdef".into()), map: "Delilah_Terrain".into() });
        assert_eq!(t.at.to_rfc3339(), "2026-11-06T21:13:57.606+00:00");
        let l = "[2026.11.06-21.26.58:861][285]LogLoad: LoadMap: /Game/Maps/Frontend?closed";
        assert_eq!(parse_line(l).unwrap().event, LogEvent::Lobby);
        let l = "[2026.11.06-21.14.02:387][ 24]LogFort: MatchState changed previous=WaitingToStart current=InProgress";
        assert_eq!(parse_line(l).unwrap().event, LogEvent::MatchState("InProgress".into()));
        let l = "[2026.11.06-21.14.20:711][100]LogBattleRoyaleGamePhaseLogic: HandleGamePhaseChanged. OldPhase = EAthenaGamePhase::Warmup. NewPhase = EAthenaGamePhase::Aircraft";
        assert_eq!(parse_line(l).unwrap().event, LogEvent::GamePhase("Aircraft".into()));
        let l = "[2026.11.06-21.26.49:926][510]MatchmakingLog: [358a] Match finished for geaclash, end game, not leaving session";
        assert_eq!(parse_line(l).unwrap().event, LogEvent::PlayerFinished);
        let l = "[2026.11.06-21.26.58:454][261]MatchmakingLog: [358a] Match finished for geaclash, leaving session";
        assert!(parse_line(l).is_none());
    }

    #[test]
    #[ignore]
    fn scan_real_log() {
        let mut t = LogTailer::new(log_path().unwrap());
        for e in t.catch_up() {
            if !matches!(e.event, LogEvent::Presence(_) | LogEvent::Playlist(_)) {
                println!("{} {:?}", e.at.format("%H:%M:%S%.3f"), e.event);
            }
        }
    }
}
