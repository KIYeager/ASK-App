// Der eigene Befehl «ipc» ist nur mit der Berechtigung «allow-ipc» erreichbar (capabilities/ui.json),
// also nur aus der eigenen Oberfläche, nie aus der ASK-Seite oder dem Browser der App.
fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(&["ipc"])),
    )
    .expect("tauri-build");
}
