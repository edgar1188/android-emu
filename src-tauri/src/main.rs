// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod gpu;

fn main() {
    gpu::setup_workarounds();
    canaima_android_emu_lib::run()
}
