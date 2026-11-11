//! Overlay sobre el juego: avisos ("Clip guardado") y el menú de highlights.
//!
//! Es una ventana transparente, siempre encima, del tamaño del monitor de Fortnite. Para
//! los avisos se muestra sin activarse (no le quita el foco al juego) y deja pasar los
//! clics; el menú sí toma el foco hasta que se cierra.
//! Nota: con Fortnite en "Pantalla completa" exclusiva Windows no dibuja ventanas encima;
//! en "Ventana sin bordes" (recomendado) sí.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{HWND_TOPMOST, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SetWindowPos, ShowWindow};

use crate::AppState;
use crate::capture::win;

const LABEL: &str = "overlay";
static MENU_OPEN: AtomicBool = AtomicBool::new(false);

#[derive(Serialize, Clone)]
struct Toast<'a> {
    text: &'a str,
    kind: &'a str,
}

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
        .title("ggswp overlay")
        .transparent(true)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focused(false)
        .visible(false)
        .build()?;
    Ok(())
}

fn window(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LABEL)
}

fn enabled(app: &AppHandle) -> bool {
    app.try_state::<AppState>().is_some_and(|s| s.shared.settings.read().unwrap().overlay)
}

/// Cubre el monitor donde está Fortnite (o el principal).
fn place(w: &WebviewWindow) {
    let outputs = win::list_outputs();
    let idx = win::game_window().and_then(win::output_index_of).unwrap_or(0);
    if let Some(o) = outputs.iter().find(|o| o.index == idx).or(outputs.first()) {
        let _ = w.set_position(PhysicalPosition::new(o.x, o.y));
        let _ = w.set_size(PhysicalSize::new(o.width as u32, o.height as u32));
    }
}

fn show_no_activate(w: &WebviewWindow) {
    if let Ok(h) = w.hwnd() {
        let hwnd = HWND(h.0);
        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    } else {
        let _ = w.show();
    }
}

pub fn toast(app: &AppHandle, text: &str, kind: &str) {
    let _ = app.emit("toast", Toast { text, kind });
    if !enabled(app) || win::game_window().is_none() {
        return;
    }
    let Some(w) = window(app) else { return };
    if !MENU_OPEN.load(Ordering::SeqCst) {
        place(&w);
        let _ = w.set_ignore_cursor_events(true);
        show_no_activate(&w);
    }
    let _ = w.emit("overlay-toast", Toast { text, kind });
}

pub fn toggle_menu(app: &AppHandle) {
    let Some(w) = window(app) else { return };
    if MENU_OPEN.swap(false, Ordering::SeqCst) {
        let _ = w.emit("overlay-menu", false);
        let _ = w.hide();
        return;
    }
    MENU_OPEN.store(true, Ordering::SeqCst);
    place(&w);
    let _ = w.set_ignore_cursor_events(false);
    let _ = w.emit("overlay-menu", true);
    let _ = w.show();
    let _ = w.set_focus();
}

pub fn close_menu(app: &AppHandle) {
    if MENU_OPEN.swap(false, Ordering::SeqCst)
        && let Some(w) = window(app)
    {
        let _ = w.emit("overlay-menu", false);
        let _ = w.hide();
    }
}

/// El overlay avisa que ya no tiene nada que mostrar.
pub fn idle(app: &AppHandle) {
    if !MENU_OPEN.load(Ordering::SeqCst)
        && let Some(w) = window(app)
    {
        let _ = w.hide();
    }
}
