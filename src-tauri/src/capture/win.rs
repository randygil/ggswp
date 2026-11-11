//! Utilidades Win32: reloj QPC, ventana de Fortnite y monitores (salidas DXGI).

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible};
use windows::core::{BOOL, PWSTR};

pub const FORTNITE_EXE: &str = "FortniteClient-Win64-Shipping.exe";

/// Reloj QPC en unidades de 100 ns: la misma base que usa WASAPI en sus timestamps.
pub fn qpc_100ns() -> u64 {
    let mut c = 0i64;
    let mut f = 1i64;
    unsafe {
        let _ = QueryPerformanceCounter(&mut c);
        let _ = QueryPerformanceFrequency(&mut f);
    }
    ((c as i128 * 10_000_000) / f as i128) as u64
}

fn process_exe(pid: u32) -> Option<String> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// Ventanas visibles con el ejecutable de su proceso (ruta completa).
fn visible_windows() -> Vec<(HWND, String)> {
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        unsafe {
            let found = &mut *(lp.0 as *mut Vec<(HWND, String)>);
            if IsWindowVisible(hwnd).as_bool() {
                let mut pid = 0u32;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if let Some(exe) = process_exe(pid) {
                    found.push((hwnd, exe));
                }
            }
            BOOL(1)
        }
    }
    let mut found: Vec<(HWND, String)> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut found as *mut Vec<(HWND, String)> as isize));
    }
    found
}

fn exe_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Ventana visible que pertenece a ese ejecutable (por nombre de archivo, sin importar mayúsculas).
pub fn find_window(exe: &str) -> Option<HWND> {
    visible_windows().into_iter().find(|(_, path)| exe_name(path).eq_ignore_ascii_case(exe)).map(|(h, _)| h)
}

pub fn find_fortnite_window() -> Option<HWND> {
    find_window(FORTNITE_EXE)
}

pub fn fortnite_running() -> bool {
    find_fortnite_window().is_some()
}

/// Ejecutable del otro juego que el motor está siguiendo (de la lista de Ajustes).
static OTHER_GAME: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn set_other_game(exe: Option<String>) {
    *OTHER_GAME.lock().unwrap() = exe;
}

/// Ventana del juego en curso: Fortnite o el otro juego abierto.
pub fn game_window() -> Option<HWND> {
    find_fortnite_window().or_else(|| OTHER_GAME.lock().unwrap().as_deref().and_then(find_window))
}

/// Programa con una ventana abierta, para elegirlo como juego a grabar.
#[derive(Debug, Clone, Serialize)]
pub struct OpenApp {
    pub title: String,
    pub exe: String,
}

/// Programas con ventana y título, sin repetir ejecutable.
pub fn open_apps() -> Vec<OpenApp> {
    let mut out: Vec<OpenApp> = Vec::new();
    for (hwnd, path) in visible_windows() {
        let mut buf = [0u16; 256];
        let n = unsafe { GetWindowTextW(hwnd, &mut buf) };
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]).trim().to_string();
        let exe = exe_name(&path).to_string();
        let system = path.to_ascii_lowercase().contains("\\windows\\");
        if title.is_empty() || system || out.iter().any(|a| a.exe.eq_ignore_ascii_case(&exe)) {
            continue;
        }
        out.push(OpenApp { title, exe });
    }
    out.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
    out
}

/// Proceso dueño de una ventana.
pub fn window_pid(hwnd: HWND) -> Option<u32> {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    (pid != 0).then_some(pid)
}

#[derive(Debug, Clone, Serialize)]
pub struct Output {
    pub index: u32,
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub x: i32,
    pub y: i32,
    #[serde(skip)]
    monitor: isize,
}

/// Salidas del adaptador 0, en el mismo orden que usa `ddagrab=output_idx=N` de ffmpeg.
pub fn list_outputs() -> Vec<Output> {
    let mut out = Vec::new();
    unsafe {
        let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() else { return out };
        let Ok(adapter) = factory.EnumAdapters1(0) else { return out };
        let mut i = 0;
        while let Ok(o) = adapter.EnumOutputs(i) {
            if let Ok(d) = o.GetDesc() {
                let name = String::from_utf16_lossy(&d.DeviceName).trim_end_matches('\0').to_string();
                let r = d.DesktopCoordinates;
                out.push(Output {
                    index: i,
                    name,
                    width: r.right - r.left,
                    height: r.bottom - r.top,
                    x: r.left,
                    y: r.top,
                    monitor: d.Monitor.0 as isize,
                });
            }
            i += 1;
        }
    }
    out
}

/// Índice de salida (monitor) donde está esa ventana.
pub fn output_index_of(hwnd: HWND) -> Option<u32> {
    let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    list_outputs().into_iter().find(|o| o.monitor == mon.0 as isize).map(|o| o.index)
}
