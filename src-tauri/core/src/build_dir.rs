//! Build-Ordner für Entwicklung und Installer-Build (nicht für die installierte App).
//! Reihenfolge: Umgebungsvariable ASK_BUILD_DIR, dann .ask-build-dir im Projekt (schreibt
//! scripts/setup.ps1), sonst %LOCALAPPDATA%\ASK-Desktop-Build.

use std::path::PathBuf;

pub fn build_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("ASK_BUILD_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    if let Ok(remembered) = std::fs::read_to_string(crate::paths::repo_root().join(".ask-build-dir")) {
        let r = remembered.trim();
        if !r.is_empty() {
            return PathBuf::from(r);
        }
    }
    crate::paths::local_app_data().join("ASK-Desktop-Build")
}
