mod adb;
mod emulator;
mod spice;
mod titlebar;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

// Learn more about Tauri commands at https://tauri.app/develop/calling-rust/
#[tauri::command]
fn greet(name: &str) -> String {
    format!(
        "Hola, {}! esperamos tu app en nuestro repositorio, Feliz codigo!",
        name
    )
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(adb::AdbAppState::new())
        .manage(emulator::EmulatorAppState::new())
        .manage(spice::SpiceAppState::new())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            use tauri::Manager;
            #[cfg(target_os = "linux")]
            {
                if let Some(window) = app.get_webview_window("main") {
                    titlebar::setup_gtk_headerbar(window);
                }
            }
            if let Some(window) = app.get_webview_window("main") {
                use tauri::WindowEvent;
                let close_started = Arc::new(AtomicBool::new(false));
                let close_ready = Arc::new(AtomicBool::new(false));
                let window_to_close = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        if close_ready.swap(false, Ordering::SeqCst) {
                            return;
                        }
                        api.prevent_close();
                        if close_started.swap(true, Ordering::SeqCst) {
                            return;
                        }

                        let window = window_to_close.clone();
                        let close_ready = Arc::clone(&close_ready);
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = emulator::shutdown_android_emulator().await {
                                eprintln!("{error}");
                            }
                            close_ready.store(true, Ordering::SeqCst);
                            if let Err(error) = window.close() {
                                eprintln!("No se pudo cerrar la ventana de la aplicación: {error}");
                                window.app_handle().exit(0);
                            }
                        });
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            adb::adb_devices,
            adb::adb_connect,
            adb::adb_disconnect,
            adb::adb_input_tap,
            adb::adb_input_swipe,
            adb::adb_input_key,
            adb::adb_set_dark_mode,
            adb::adb_install_apk,
            adb::scrcpy_input_key,
            adb::scrcpy_input_touch,
            adb::scrcpy_input_scroll,
            spice::spice_connect,
            spice::spice_disconnect,
            emulator::start_android_emulator,
            titlebar::sync_theme
        ])
        .run(tauri::generate_context!())
        .expect("ups! ocurrio un error al ejecutar la app");
}
