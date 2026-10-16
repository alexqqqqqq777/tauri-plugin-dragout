const COMMANDS: &[&str] = &["start_drag", "is_supported"];

fn main() {
    tauri_plugin::Builder::new(COMMANDS).build();
}
