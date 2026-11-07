//! Lector de archivos `.replay` de Fortnite (formato "local file replay" de Unreal).
//!
//! Solo se leen los chunks de eventos: `playerElim` trae cada eliminación de la partida
//! (víctima, autor, posiciones, arma y si fue derribo). Desde v2 los eventos van cifrados
//! con AES-256-ECB y la clave viaja en la cabecera del propio archivo.

use std::path::{Path, PathBuf};

use aes::Aes256;
use aes::cipher::{BlockDecrypt, KeyInit, generic_array::GenericArray};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, NaiveDateTime, TimeZone, Utc};
use serde::Serialize;

const MAGIC: u32 = 0x1CA2_E27F;
const TICKS_AT_UNIX_EPOCH: i64 = 621_355_968_000_000_000;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum PlayerRef {
    Bot,
    NamedBot(String),
    Player(String),
}

impl PlayerRef {
    pub fn is(&self, account_id: &str) -> bool {
        matches!(self, PlayerRef::Player(id) if id.eq_ignore_ascii_case(account_id))
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Elimination {
    /// Milisegundos desde el inicio del replay.
    pub time_ms: u32,
    pub eliminated: PlayerRef,
    pub eliminator: PlayerRef,
    pub knocked: bool,
    pub gun_type: u8,
    pub distance_m: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Replay {
    pub path: PathBuf,
    pub started_at: DateTime<Utc>,
    pub length_ms: u32,
    pub eliminations: Vec<Elimination>,
}

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.p + n > self.b.len() {
            bail!("fin de archivo inesperado en offset {}", self.p);
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.take(4)?.try_into()?))
    }
    fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.take(8)?.try_into()?))
    }
    fn f64(&mut self) -> Result<f64> {
        Ok(f64::from_le_bytes(self.take(8)?.try_into()?))
    }
    fn fstring(&mut self) -> Result<String> {
        let n = self.i32()?;
        let s = if n == 0 {
            String::new()
        } else if n < 0 {
            let raw = self.take((-n as usize) * 2)?;
            let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&units)
        } else {
            String::from_utf8_lossy(self.take(n as usize)?).into_owned()
        };
        Ok(s.trim_end_matches('\0').to_string())
    }
}

pub fn parse_file(path: &Path) -> Result<Replay> {
    let bytes = std::fs::read(path).with_context(|| format!("leyendo {}", path.display()))?;
    parse_bytes(&bytes, path)
}

/// Lee solo la cabecera (rápido) para saber cuándo empezó el replay.
pub fn read_start_time(path: &Path) -> Result<DateTime<Utc>> {
    use std::io::Read;
    let mut buf = vec![0u8; 4096];
    let n = std::fs::File::open(path)?.read(&mut buf)?;
    buf.truncate(n);
    let mut r = Reader { b: &buf, p: 0 };
    let h = read_header(&mut r)?;
    Ok(resolve_start(h.started_naive, path))
}

/// Fortnite escribe la hora de inicio con `FDateTime::Now()` (hora local, sin zona), aunque
/// el formato de Unreal no lo dice. Para no depender de eso, se prueban las dos lecturas
/// (local y UTC) y se elige la que coincide con la fecha de creación real del archivo.
fn resolve_start(naive: Option<NaiveDateTime>, path: &Path) -> DateTime<Utc> {
    let created: Option<DateTime<Utc>> = std::fs::metadata(path).ok().and_then(|m| m.created().or_else(|_| m.modified()).ok()).map(Into::into);
    let Some(naive) = naive else { return created.unwrap_or_else(Utc::now) };
    let as_utc = Utc.from_utc_datetime(&naive);
    let as_local = Local.from_local_datetime(&naive).earliest().map(|t| t.with_timezone(&Utc));
    match (as_local, created) {
        (Some(local), Some(c)) if (local - c).num_seconds().abs() < (as_utc - c).num_seconds().abs() => local,
        (Some(local), None) => local,
        _ => as_utc,
    }
}

struct Header {
    started_naive: Option<NaiveDateTime>,
    length_ms: u32,
    key: Option<Vec<u8>>,
}

fn read_header(r: &mut Reader) -> Result<Header> {
    if r.u32()? != MAGIC {
        bail!("no es un replay de Unreal");
    }
    let version = r.u32()?;
    if version >= 7 {
        let n = r.u32()? as usize;
        r.take(n * 20)?;
    }
    let length_ms = r.u32()?;
    let _network_version = r.u32()?;
    let _changelist = r.u32()?;
    let _friendly_name = r.fstring()?;
    let _is_live = r.u32()?;
    let mut started_naive = None;
    if version >= 3 {
        let ticks = r.i64()?;
        let unix_100ns = ticks - TICKS_AT_UNIX_EPOCH;
        started_naive = DateTime::from_timestamp(unix_100ns.div_euclid(10_000_000), (unix_100ns.rem_euclid(10_000_000) * 100) as u32).map(|t| t.naive_utc());
    }
    if version >= 2 {
        let _compressed = r.u32()?;
    }
    let mut key = None;
    if version >= 6 {
        let encrypted = r.u32()? != 0;
        let n = r.u32()? as usize;
        let k = r.take(n)?.to_vec();
        if encrypted {
            key = Some(k);
        }
    }
    Ok(Header { started_naive, length_ms, key })
}

fn decrypt(key: &[u8], data: &[u8]) -> Result<Vec<u8>> {
    if key.len() != 32 || data.len() % 16 != 0 {
        bail!("bloque cifrado inválido");
    }
    let cipher = Aes256::new(GenericArray::from_slice(key));
    let mut out = data.to_vec();
    for block in out.chunks_exact_mut(16) {
        cipher.decrypt_block(GenericArray::from_mut_slice(block));
    }
    if let Some(&pad) = out.last() {
        let pad = pad as usize;
        if (1..=16).contains(&pad) && out.len() >= pad && out[out.len() - pad..].iter().all(|&b| b as usize == pad) {
            out.truncate(out.len() - pad);
        }
    }
    Ok(out)
}

pub fn parse_bytes(bytes: &[u8], path: &Path) -> Result<Replay> {
    let mut r = Reader { b: bytes, p: 0 };
    let header = read_header(&mut r)?;
    let mut eliminations = Vec::new();

    while r.p + 8 <= bytes.len() {
        let chunk_type = r.u32()?;
        let size = r.i32()?;
        if size < 0 {
            break;
        }
        let end = r.p + size as usize;
        if end > bytes.len() {
            break; // replay todavía escribiéndose
        }
        if chunk_type == 3 {
            let mut c = Reader { b: &bytes[..end], p: r.p };
            let _id = c.fstring()?;
            let group = c.fstring()?;
            let _meta = c.fstring()?;
            let t1 = c.u32()?;
            let _t2 = c.u32()?;
            let dsize = c.i32()?.max(0) as usize;
            if group == "playerElim" {
                let raw = c.take(dsize)?;
                let data = match &header.key {
                    Some(k) => decrypt(k, raw)?,
                    None => raw.to_vec(),
                };
                match parse_elim(&data, t1) {
                    Ok(e) => eliminations.push(e),
                    Err(err) => log::debug!("playerElim ilegible en {t1} ms: {err:#}"),
                }
            }
        }
        r.p = end;
    }

    Ok(Replay {
        path: path.to_path_buf(),
        started_at: resolve_start(header.started_naive, path),
        length_ms: header.length_ms,
        eliminations,
    })
}

fn read_player(r: &mut Reader) -> Result<PlayerRef> {
    Ok(match r.u8()? {
        0x03 => PlayerRef::Bot,
        0x10 => PlayerRef::NamedBot(r.fstring()?),
        0x11 => {
            let n = r.u8()? as usize;
            let id = r.take(n)?;
            PlayerRef::Player(id.iter().map(|b| format!("{b:02x}")).collect())
        }
        other => bail!("tipo de jugador desconocido 0x{other:02x}"),
    })
}

/// Formato actual (versión de evento >= 3): i32 versión, 1 byte, dos FTransform en doubles
/// (quat 4 + posición 3 + escala 3), víctima, autor, tipo de arma (u8), derribado (u32).
fn parse_elim(data: &[u8], time_ms: u32) -> Result<Elimination> {
    let mut r = Reader { b: data, p: 0 };
    let version = r.i32()?;
    if version < 3 {
        bail!("versión de playerElim {version} no soportada");
    }
    r.u8()?;
    let mut locations = [[0f64; 3]; 2];
    for loc in &mut locations {
        for _ in 0..4 {
            r.f64()?;
        }
        for v in loc.iter_mut() {
            *v = r.f64()?;
        }
        for _ in 0..3 {
            r.f64()?;
        }
    }
    let eliminated = read_player(&mut r)?;
    let eliminator = read_player(&mut r)?;
    let gun_type = r.u8()?;
    let knocked = r.u32()? != 0;

    let [a, b] = locations;
    let dist = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt() / 100.0;
    let has_pos = a.iter().chain(b.iter()).any(|v| *v != 0.0);

    Ok(Elimination {
        time_ms,
        eliminated,
        eliminator,
        knocked,
        gun_type,
        distance_m: (has_pos && dist.is_finite() && dist < 5000.0).then_some(dist),
    })
}

/// Carpeta donde Fortnite guarda los replays.
pub fn demos_dir() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join("FortniteGame\\Saved\\Demos"))
}

/// Busca el replay que empezó dentro de la ventana de la grabación.
pub fn find_for_window(from: DateTime<Utc>, to: DateTime<Utc>) -> Option<PathBuf> {
    let dir = demos_dir()?;
    let mut best: Option<(i64, PathBuf)> = None;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("replay") {
            continue;
        }
        let Ok(start) = read_start_time(&path) else { continue };
        if start < from - chrono::Duration::minutes(3) || start > to {
            continue;
        }
        let gap = (start - from).num_milliseconds().abs();
        if best.as_ref().is_none_or(|(g, _)| gap < *g) {
            best = Some((gap, path));
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Usa los replays reales de la máquina: `cargo test -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn parse_latest_local_replay() {
        let dir = demos_dir().unwrap();
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().flatten().map(|e| e.path()).collect();
        files.sort_by_key(|p| std::fs::metadata(p).unwrap().modified().unwrap());
        let path = files.last().unwrap();
        let rep = parse_file(path).unwrap();
        println!("{} start={} len={}ms elims={}", path.display(), rep.started_at, rep.length_ms, rep.eliminations.len());
        for e in rep.eliminations.iter().filter(|e| matches!(&e.eliminator, PlayerRef::Player(_))).take(80) {
            println!("{:>8} {:?} -> {:?} knocked={} gun={} dist={:?}", e.time_ms, e.eliminator, e.eliminated, e.knocked, e.gun_type, e.distance_m.map(|d| d.round()));
        }
        assert!(!rep.eliminations.is_empty());
    }
}
