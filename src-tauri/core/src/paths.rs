//! Pfade zu mitgelieferten Teilen, getrennt nach Entwicklung (`npm start`) und installierter App.
//! Die Tauri-Hülle ruft beim Start `init` auf; ohne Aufruf (Unit-Tests) gilt die Entwicklung im
//! Repository.

use parking_lot::RwLock;
use sha1::{Digest, Sha1};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

#[derive(Clone, Debug)]
pub struct PathsConfig {
    /// Installierte App oder einzelne EXE (in Electron `app.isPackaged`)
    pub packaged: bool,
    /// Ordner mit den mitgelieferten Teilen (Tauri: resource_dir). In der Entwicklung `<repo>/resources`.
    pub resources_dir: PathBuf,
    /// Daten der App (%APPDATA%\ASK, wie zuvor bei Electron: einstellungen.json, gedaechtnis.json, skills\)
    pub user_data: PathBuf,
    /// Die laufende EXE (Verknüpfung im Startmenü, einzelne EXE erkennen)
    pub exe: PathBuf,
}

static CONFIG: OnceLock<RwLock<PathsConfig>> = OnceLock::new();

/// Wurzel des Repositorys (Entwicklung und Tests): src-tauri/core/../..
pub fn repo_root() -> PathBuf {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    here.parent().and_then(Path::parent).map(Path::to_path_buf).unwrap_or(here)
}

/// %LOCALAPPDATA% (unter Linux für Tests ~/.local/share)
pub fn local_app_data() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(std::env::temp_dir)
}

/// %USERPROFILE% bzw. das Benutzerverzeichnis
pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
}

fn default_user_data() -> PathBuf {
    if let Some(dir) = std::env::var_os("ASK_TEST_USER_DATA") {
        return PathBuf::from(dir);
    }
    std::env::var_os("APPDATA").map(PathBuf::from)
        .or_else(dirs::config_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join("ASK")
}

fn defaults() -> PathsConfig {
    PathsConfig {
        packaged: false,
        resources_dir: repo_root().join("resources"),
        user_data: default_user_data(),
        exe: std::env::current_exe().unwrap_or_default(),
    }
}

fn cfg() -> &'static RwLock<PathsConfig> {
    CONFIG.get_or_init(|| RwLock::new(defaults()))
}

/// Einmal beim Start (Tauri-Hülle). Später aufgerufen ersetzt es die Angaben (Tests).
pub fn init(config: PathsConfig) {
    *cfg().write() = config;
}

pub fn config() -> PathsConfig {
    cfg().read().clone()
}

pub fn is_packaged() -> bool {
    cfg().read().packaged
}

/// Test-Modus wie in Electron: nicht installiert und ASK_TEST_SCRIPT gesetzt.
pub fn test_mode() -> bool {
    !is_packaged() && std::env::var_os("ASK_TEST_SCRIPT").is_some()
}

pub fn user_data() -> PathBuf {
    cfg().read().user_data.clone()
}

pub fn resources(parts: &[&str]) -> PathBuf {
    let mut p = cfg().read().resources_dir.clone();
    for part in parts {
        p.push(part);
    }
    p
}

/// Python der App. Installiert: einmal entpackt nach %LOCALAPPDATA%\ASK-Desktop\python\<id>
/// (python.rs, ENTSCHEIDE E39); Entwicklung: <Build-Ordner>\python\python.exe.
pub fn python_exe() -> PathBuf {
    if is_packaged() {
        crate::python::shared().exe()
    } else {
        crate::build_dir::build_dir().join("python").join("python.exe")
    }
}

pub fn launcher_py() -> PathBuf {
    if is_packaged() {
        resources(&["ask_terminal", "launcher.py"])
    } else {
        repo_root().join("python").join("ask_terminal").join("launcher.py")
    }
}

pub fn skills_dir() -> PathBuf {
    resources(&["skills"])
}

pub fn instructions_dir() -> PathBuf {
    resources(&["anweisungen"])
}

pub fn app_config_file() -> PathBuf {
    resources(&["app-config.json"])
}

pub fn icon_file() -> PathBuf {
    resources(&["icon.ico"])
}

/// Seite für das Zeichnen von PDF-Seiten (visual.rs)
pub fn render_dir() -> PathBuf {
    if is_packaged() { resources(&["render"]) } else { repo_root().join("src").join("render") }
}

/// pdf.js (src/vendor/pdfjs)
pub fn vendor_dir() -> PathBuf {
    if is_packaged() { resources(&["vendor"]) } else { repo_root().join("src").join("vendor") }
}

/// Daten der App zu einem Arbeitsordner, die das Modell nicht ändern darf (Sicherung für «Rückgängig»,
/// Verbindung mit GitHub). Liegt darum nicht im Arbeitsordner und nicht im Laufzeitordner des
/// Terminals: Beide sind für das Terminal beschreibbar.
pub fn project_data_dir(folder: &str) -> PathBuf {
    let base = match std::env::var_os("ASK_TEST_USER_DATA") {
        Some(dir) if !is_packaged() => PathBuf::from(dir).join("projekte"),
        _ => local_app_data().join("ASK-Desktop").join("projekte"),
    };
    let resolved = std::path::absolute(folder).unwrap_or_else(|_| PathBuf::from(folder));
    let mut h = Sha1::new();
    h.update(resolved.to_string_lossy().to_lowercase().as_bytes());
    let key = hex::encode(h.finalize());
    base.join(&key[..20])
}

/// Laufzeitdaten des Terminals (temporäre Dateien, Logs). Muss lokal sein, nicht im Roaming-Profil.
pub fn runtime_dir() -> PathBuf {
    if !is_packaged() {
        if let Some(dir) = std::env::var_os("ASK_RUNTIME_DIR") {
            return PathBuf::from(dir);
        }
    }
    local_app_data().join("ASK-Desktop").join("laufzeit")
}
