//! Totales históricos (cofres, eliminaciones, victorias…). Se guardan aparte de la
//! biblioteca, en `stats.json` junto a los ajustes, para que no bajen cuando una partida
//! vence o se borra. Cada partida guarda su propio aporte: si se vuelve a analizar, su
//! aporte se reemplaza en vez de sumarse dos veces.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::library::{Library, MatchMeta, MatchStatus};

static LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Contribution {
    pub at: DateTime<Utc>,
    pub kills: u32,
    pub knocks: u32,
    pub chests: u32,
    pub victory: bool,
    pub seconds: f64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Store {
    matches: BTreeMap<String, Contribution>,
}

#[derive(Debug, Default, Serialize, PartialEq)]
pub struct Totals {
    pub matches: u32,
    pub kills: u32,
    pub knocks: u32,
    pub chests: u32,
    pub victories: u32,
    pub seconds: f64,
    /// Primera partida contada.
    pub since: Option<DateTime<Utc>>,
}

pub fn path_for(settings_path: &Path) -> PathBuf {
    settings_path.with_file_name("stats.json")
}

fn load(path: &Path) -> Store {
    std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn save(path: &Path, store: &Store) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(store)?)?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

/// Solo cuentan las partidas reales de Fortnite (no las grabaciones manuales ni los clips
/// guardados fuera de partida).
fn counts(m: &MatchMeta) -> bool {
    m.status == MatchStatus::Ready && (m.replay.is_some() || m.account_id.is_some())
}

fn contribution(m: &MatchMeta) -> Contribution {
    Contribution { at: m.created_at, kills: m.kills, knocks: m.knocks, chests: m.chests, victory: m.victory, seconds: m.duration_s }
}

/// Registra (o actualiza) el aporte de una partida recién analizada.
pub fn record(path: &Path, m: &MatchMeta) -> Result<()> {
    if !counts(m) {
        return Ok(());
    }
    let _g = LOCK.lock().unwrap();
    let mut store = load(path);
    let c = contribution(m);
    if store.matches.get(&m.id) != Some(&c) {
        store.matches.insert(m.id.clone(), c);
        save(path, &store)?;
    }
    Ok(())
}

/// Suma las partidas de la biblioteca que todavía no estaban contadas (la primera vez,
/// las que ya existían antes de esta función).
pub fn seed(path: &Path, lib: &Library) -> Result<()> {
    let _g = LOCK.lock().unwrap();
    let mut store = load(path);
    let mut changed = false;
    for m in lib.list().iter().filter(|m| counts(m)) {
        if !store.matches.contains_key(&m.id) {
            store.matches.insert(m.id.clone(), contribution(m));
            changed = true;
        }
    }
    if changed {
        save(path, &store)?;
    }
    Ok(())
}

pub fn totals(path: &Path) -> Totals {
    let _g = LOCK.lock().unwrap();
    sum(&load(path))
}

fn sum(store: &Store) -> Totals {
    let mut t = Totals::default();
    for c in store.matches.values() {
        t.matches += 1;
        t.kills += c.kills;
        t.knocks += c.knocks;
        t.chests += c.chests;
        t.victories += c.victory as u32;
        t.seconds += c.seconds;
        t.since = Some(t.since.map_or(c.at, |s| s.min(c.at)));
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(id: &str, kills: u32, chests: u32) -> MatchMeta {
        MatchMeta { id: id.into(), kills, chests, status: MatchStatus::Ready, account_id: Some("a".into()), duration_s: 60.0, ..Default::default() }
    }

    #[test]
    fn reanalysis_replaces_and_deletion_keeps_totals() {
        let dir = std::env::temp_dir().join(format!("ggswp-stats-{}", std::process::id()));
        let path = dir.join("stats.json");
        let _ = std::fs::remove_dir_all(&dir);

        record(&path, &meta("a", 1, 0)).unwrap();
        record(&path, &meta("b", 2, 3)).unwrap();
        // Re-analizar "a" con los cofres detectados: no se cuenta dos veces.
        record(&path, &meta("a", 1, 5)).unwrap();
        let t = totals(&path);
        assert_eq!((t.matches, t.kills, t.chests), (2, 3, 8));

        // Una grabación manual (sin replay ni cuenta) no cuenta.
        record(&path, &MatchMeta { account_id: None, ..meta("manual", 9, 9) }).unwrap();
        assert_eq!(totals(&path).matches, 2);

        // Que una partida venza o se borre de la biblioteca no cambia el histórico:
        // seed con una biblioteca vacía no quita nada.
        seed(&path, &Library::new(dir.join("vacia"))).unwrap();
        assert_eq!(totals(&path).chests, 8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
