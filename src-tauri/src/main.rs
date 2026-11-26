// Ohne Konsolenfenster unter Windows (nur im Release-Build)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Terminal-Server: dieselbe ASK.exe, vom Starter in der App mit niedriger Integritätsstufe gestartet.
    // Muss vor jedem Tauri-Aufbau laufen (kein Fenster, keine Einzelinstanz-Sperre, kein Webview).
    if std::env::args().any(|a| a == "--terminal") {
        std::process::exit(ask_terminal::run_from_stdin());
    }
    ask_lib::run();
}
