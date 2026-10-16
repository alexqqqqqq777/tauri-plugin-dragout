use tauri_plugin_dragout::{DragItem, DragOptions};

/// Drag a file that is rendered only when the user drops it.
#[tauri::command]
fn drag_report(window: tauri::WebviewWindow) -> Result<u64, tauri_plugin_dragout::Error> {
    let item = DragItem::promise("report.txt", |dest| {
        let body = format!("Rendered at drop time: {:?}\n", std::time::SystemTime::now());
        std::fs::write(dest, body).map_err(|e| e.to_string())
    });
    tauri_plugin_dragout::start_drag(&window, vec![item], DragOptions::default()).map(|s| s.id)
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dragout::init())
        .invoke_handler(tauri::generate_handler![drag_report])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
