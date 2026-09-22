mod spice;
mod titlebar;

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
            spice::spice_open_viewer,
            spice::spice_probe,
            titlebar::sync_theme
        ])
        .run(tauri::generate_context!())
        .expect("ups! ocurrio un error al ejecutar la app");
}
