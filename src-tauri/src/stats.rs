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

/// 1: con el resumen de fin de partida (puesto, daño, distancia).
const VERSION: u8 = 1;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Contribution {
    pub version: u8,
    pub at: DateTime<Utc>,
    pub kills: u32,
    pub knocks: u32,
    pub chests: u32,
    pub victory: bool,
    pub seconds: f64,
    pub placement: Option<u32>,
    pub damage_dealt: Option<u32>,
    pub distance_m: Option<f64>,
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
    pub damage_dealt: u64,
    pub distance_m: f64,
    /// Mejor puesto y promedio, solo entre las partidas que tienen puesto.
    pub best_placement: Option<u32>,
    pub avg_placement: Option<f64>,
    pub placed_matches: u32,
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
    Contribution {
        version: VERSION,
        at: m.created_at,
        kills: m.kills,
        knocks: m.knocks,
        chests: m.chests,
        victory: m.victory,
        seconds: m.duration_s,
        placement: m.placement,
        damage_dealt: m.damage_dealt,
        distance_m: m.distance_m,
    }
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
/// las que ya existían antes de esta función). Las contadas con una versión anterior se
/// completan una sola vez con el resumen de fin de partida, si su replay todavía existe.
pub fn seed(path: &Path, lib: &Library) -> Result<()> {
    let _g = LOCK.lock().unwrap();
    let mut store = load(path);
    let mut changed = false;
    for mut m in lib.list().into_iter().filter(counts) {
        if store.matches.get(&m.id).is_some_and(|c| c.version >= VERSION) {
            continue;
        }
        backfill(lib, &mut m);
        store.matches.insert(m.id.clone(), contribution(&m));
        changed = true;
    }
    if changed {
        save(path, &store)?;
    }
    Ok(())
}

/// Lee el resumen de fin de partida de una partida analizada antes de que existiera.
fn backfill(lib: &Library, m: &mut MatchMeta) {
    if m.placement.is_some() || m.accuracy.is_some() {
        return;
    }
    let stats = m.replay.as_deref().map(Path::new).filter(|p| p.exists()).and_then(|p| crate::fortnite::replay::parse_file(p).ok()).and_then(|r| r.stats);
    m.apply_stats(stats.as_ref());
    if m.placement.is_some() || m.accuracy.is_some() {
        if let Err(e) = lib.save(m) {
            log::warn!("{}: no se pudo guardar el resumen de la partida: {e:#}", m.id);
        }
    }
}

pub fn totals(path: &Path) -> Totals {
    let _g = LOCK.lock().unwrap();
    sum(&load(path))
}

fn sum(store: &Store) -> Totals {
    let mut t = Totals::default();
    let mut placement_sum = 0u64;
    for c in store.matches.values() {
        t.matches += 1;
        t.kills += c.kills;
        t.knocks += c.knocks;
        t.chests += c.chests;
        t.victories += c.victory as u32;
        t.seconds += c.seconds;
        t.damage_dealt += c.damage_dealt.unwrap_or(0) as u64;
        t.distance_m += c.distance_m.unwrap_or(0.0);
        if let Some(p) = c.placement {
            t.placed_matches += 1;
            placement_sum += p as u64;
            t.best_placement = Some(t.best_placement.map_or(p, |b| b.min(p)));
        }
        t.since = Some(t.since.map_or(c.at, |s| s.min(c.at)));
    }
    if t.placed_matches > 0 {
        t.avg_placement = Some(placement_sum as f64 / t.placed_matches as f64);
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

        // Puesto, daño y distancia: el puesto se promedia solo entre las partidas que lo tienen.
        let t = totals(&path);
        assert_eq!((t.best_placement, t.avg_placement, t.placed_matches, t.damage_dealt), (None, None, 0, 0));
        record(&path, &MatchMeta { placement: Some(10), damage_dealt: Some(300), distance_m: Some(500.0), ..meta("a", 1, 5) }).unwrap();
        record(&path, &MatchMeta { placement: Some(1), victory: true, damage_dealt: Some(700), distance_m: Some(1500.5), ..meta("c", 4, 2) }).unwrap();
        let t = totals(&path);
        assert_eq!((t.matches, t.victories, t.best_placement, t.placed_matches), (3, 1, Some(1), 2));
        assert_eq!(t.avg_placement, Some(5.5));
        assert_eq!(t.damage_dealt, 1000);
        assert!((t.distance_m - 2000.5).abs() < 1e-9);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
