// En release no abre consola: es una app de ventana.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ggswp_lib::run()
}
