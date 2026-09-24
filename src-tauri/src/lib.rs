mod spice;
mod titlebar;

use std::sync::Arc;
use tokio::sync::Mutex;

pub struct SpiceAppState {
    pub session: Arc<Mutex<Option<spice::SpiceSession>>>,
}

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
        .manage(SpiceAppState {
            session: Arc::new(Mutex::new(None)),
        })
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            #[cfg(target_os = "linux")]
            {
                use tauri::Manager;
                if let Some(window) = app.get_webview_window("main") {
                    titlebar::setup_gtk_headerbar(window);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            spice::spice_connect,
            spice::spice_disconnect,
            spice::spice_frame,
            spice::spice_probe,
            titlebar::sync_theme
        ])
        .run(tauri::generate_context!())
        .expect("ups! ocurrio un error al ejecutar la app");
}
