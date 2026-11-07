//! Utilidades Win32: reloj QPC, ventana de Fortnite y monitores (salidas DXGI).

use serde::Serialize;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};
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

/// Ventana visible que pertenece al ejecutable de Fortnite.
pub fn find_fortnite_window() -> Option<HWND> {
    struct Search {
        found: Option<HWND>,
    }
    unsafe extern "system" fn cb(hwnd: HWND, lp: LPARAM) -> BOOL {
        unsafe {
            let s = &mut *(lp.0 as *mut Search);
            if !IsWindowVisible(hwnd).as_bool() {
                return BOOL(1);
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if let Some(exe) = process_exe(pid)
                && exe.to_ascii_lowercase().ends_with(&FORTNITE_EXE.to_ascii_lowercase())
            {
                s.found = Some(hwnd);
                return BOOL(0);
            }
            BOOL(1)
        }
    }
    let mut s = Search { found: None };
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut s as *mut Search as isize));
    }
    s.found
}

pub fn fortnite_running() -> bool {
    find_fortnite_window().is_some()
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

/// Índice de salida donde está Fortnite (si está abierto).
pub fn fortnite_output_index() -> Option<u32> {
    let hwnd = find_fortnite_window()?;
    let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    list_outputs().into_iter().find(|o| o.monitor == mon.0 as isize).map(|o| o.index)
}
