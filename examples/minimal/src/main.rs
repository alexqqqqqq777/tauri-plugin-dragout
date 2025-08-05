#![cfg(target_os = "macos")]

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dragout::init())
        .invoke_handler(tauri::generate_handler![native_drag_out])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
