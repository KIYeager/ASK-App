// Ohne Konsolenfenster unter Windows (nur im Release-Build)
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    ask_lib::run();
}
