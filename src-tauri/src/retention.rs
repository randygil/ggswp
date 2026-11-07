//! Vencimiento de partidas no conservadas:
//! - a los `local_days` días se borra de la PC lo que ya está (verificado) en Drive;
//! - a los `retention_days` días se borra del todo: PC y Drive (a la papelera, recuperable
//!   30 días). Lo que nunca llegó a Drive se queda en la PC hasta este momento.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tauri::{AppHandle, Emitter};

use crate::engine::Shared;
use crate::library::{MatchMeta, MatchStatus};

/// Revisa al iniciar y luego cada hora.
pub fn spawn(app: AppHandle, shared: Arc<Shared>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            let n = expire(&shared).await + free_local(&shared).await;
            if n > 0 {
                let _ = app.emit("library", ());
            }
            tokio::time::sleep(Duration::from_secs(60 * 60)).await;
        }
    });
}

fn is_expired(m: &MatchMeta, days: u32, recording_id: Option<&str>) -> bool {
    days > 0
        && !m.keep
        && matches!(m.status, MatchStatus::Ready | MatchStatus::Error)
        && recording_id != Some(m.id.as_str())
        && Utc::now() - m.created_at > chrono::Duration::days(days as i64)
}

/// Borra de la PC lo que ya está en Drive de las partidas con más de `local_days` días.
pub async fn free_local(shared: &Shared) -> usize {
    let days = shared.settings.read().unwrap().local_days;
    let recording = shared.status.lock().unwrap().recording.as_ref().map(|r| r.match_id.clone());
    let mut changed = 0;
    for m in shared.library().list().into_iter().filter(|m| is_expired(m, days, recording.as_deref()) && !m.drive_files().is_empty()) {
        match crate::uploads::offload_verified(shared, &m.id).await {
            Ok(0) => {}
            Ok(n) => {
                changed += 1;
                log::info!("{}: {n} archivo(s) borrados de la PC tras {days} días (siguen en Drive)", m.id);
            }
            Err(e) => log::warn!("{}: no se pudo liberar la PC: {e:#}", m.id),
        }
    }
    changed
}

/// Borra las partidas vencidas y devuelve cuántas se borraron. Si Drive falla, la partida se
/// deja para el próximo intento, así nunca queda una copia huérfana en Drive.
pub async fn expire(shared: &Shared) -> usize {
    let days = shared.settings.read().unwrap().retention_days;
    if days == 0 {
        return 0;
    }
    let recording = shared.status.lock().unwrap().recording.as_ref().map(|r| r.match_id.clone());
    let lib = shared.library();
    let mut removed = 0;
    for m in lib.list().into_iter().filter(|m| is_expired(m, days, recording.as_deref())) {
        if let Err(e) = crate::uploads::trash(shared, &m.drive_files()).await {
            log::warn!("{} venció pero no se pudo borrar de Drive: {e:#}", m.id);
            continue;
        }
        match lib.delete(&m.id) {
            Ok(()) => {
                removed += 1;
                log::info!("{} venció ({days} días) y se borró", m.id);
            }
            Err(e) => log::warn!("{} venció pero no se pudo borrar: {e:#}", m.id),
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_old_unkept_ready_matches_expire() {
        let old = MatchMeta { id: "a".into(), created_at: Utc::now() - chrono::Duration::days(8), status: MatchStatus::Ready, ..Default::default() };
        assert!(is_expired(&old, 7, None));
        assert!(!is_expired(&old, 0, None), "0 días = nunca");
        assert!(!is_expired(&old, 7, Some("a")), "se está grabando");
        assert!(!is_expired(&MatchMeta { keep: true, ..old.clone() }, 7, None));
        assert!(!is_expired(&MatchMeta { status: MatchStatus::Processing, ..old.clone() }, 7, None));
        assert!(!is_expired(&MatchMeta { created_at: Utc::now() - chrono::Duration::days(6), ..old }, 7, None));
    }
}
