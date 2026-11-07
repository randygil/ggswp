//! Atajos de teclado globales (funcionan con el juego en primer plano).

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::AppState;
use crate::config::Hotkeys;
use crate::engine::Cmd;

#[derive(Clone, Copy)]
enum Action {
    SaveClip,
    Bookmark,
    ToggleRecording,
    Menu,
}

fn run(app: &AppHandle, a: Action) {
    let send = |c: Cmd| {
        let _ = app.state::<AppState>().shared.cmd.lock().unwrap().send(c);
    };
    match a {
        Action::SaveClip => send(Cmd::SaveClip),
        Action::Bookmark => send(Cmd::Bookmark),
        Action::ToggleRecording => send(Cmd::ToggleRecording),
        Action::Menu => crate::overlay::toggle_menu(app),
    }
}

/// Registra los atajos. Devuelve los que no se pudieron registrar (formato inválido o
/// ya usados por otra aplicación).
pub fn apply(app: &AppHandle, keys: &Hotkeys) -> Vec<String> {
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut failed = Vec::new();
    for (combo, action) in [
        (&keys.save_clip, Action::SaveClip),
        (&keys.bookmark, Action::Bookmark),
        (&keys.toggle_recording, Action::ToggleRecording),
        (&keys.highlights_menu, Action::Menu),
    ] {
        if combo.trim().is_empty() {
            continue;
        }
        let r = gs.on_shortcut(combo.as_str(), move |app, _sc, ev| {
            if ev.state == ShortcutState::Pressed {
                run(app, action);
            }
        });
        if let Err(e) = r {
            log::warn!("atajo {combo}: {e}");
            failed.push(combo.clone());
        }
    }
    failed
}
