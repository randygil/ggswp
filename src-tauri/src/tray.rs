use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};

use crate::AppState;

pub struct TrayItems {
    record: MenuItem<Wry>,
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Abrir ggswp", true, None::<&str>)?;
    let record = MenuItem::with_id(app, "record", "Grabar ahora", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Salir", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &record, &PredefinedMenuItem::separator(app)?, &quit])?;
    app.manage(TrayItems { record });

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("icono"))
        .tooltip("ggswp · esperando partida")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "open" => show_main(app),
            "record" => {
                let state = app.state::<AppState>();
                let recording = state.shared.status.lock().unwrap().recording.is_some();
                let cmd = if recording { crate::engine::Cmd::StopManual } else { crate::engine::Cmd::StartManual };
                let _ = state.shared.cmd.lock().unwrap().send(cmd);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn refresh(app: &AppHandle, recording: bool) {
    if let Some(items) = app.try_state::<TrayItems>() {
        let _ = items.record.set_text(if recording { "Detener grabación" } else { "Grabar ahora" });
    }
    if let Some(t) = app.tray_by_id("main") {
        let _ = t.set_tooltip(Some(if recording { "ggswp · grabando" } else { "ggswp · esperando partida" }));
    }
}
