//! Einzelne EXE ohne Installation (ENTSCHEIDE E18). Der Starter (scripts/starter.nsi) entpackt die
//! App beim ersten Start nach %LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build> und ruft sie danach
//! direkt von dort auf. Nach einem Update bleiben ältere Stände liegen (je rund 650 MB), dazu
//! abgebrochene Entpackungen (*.neu). Diese räumt die App beim Start weg.
//!
//! Öffentliche Schnittstelle:
//! - `app_cache_root(local_app_data) -> PathBuf`, `stale_app_dirs(root, own_name, now_ms) -> Vec<PathBuf>`
//! - `is_portable(exec_path, local_app_data) -> bool`, `cleanup_old_versions()`
//! - `MIN_AGE_MS`

use std::path::{Path, PathBuf};

/// Ein jüngerer .neu-Ordner kann einem Starter gehören, der gerade entpackt.
pub const MIN_AGE_MS: i64 = 60 * 60 * 1000;

/// %LOCALAPPDATA%\ASK-Desktop\app; leer, wenn LOCALAPPDATA fehlt
pub fn app_cache_root(local_app_data: Option<&str>) -> PathBuf {
    match local_app_data {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("ASK-Desktop").join("app"),
        _ => PathBuf::new(),
    }
}

fn birth_ms(meta: &std::fs::Metadata) -> Option<f64> {
    // wie birthtimeMs: ohne Erstellzeit (manche Dateisysteme) gilt die Änderungszeit
    let t = meta.created().or_else(|_| meta.modified()).ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(d.as_secs() as f64 * 1000.0 + f64::from(d.subsec_nanos()) / 1e6)
}

/// Ältere Stände und abgebrochene Entpackungen in root, ohne den eigenen Stand (own_name).
pub fn stale_app_dirs(root: &Path, own_name: &str, now_ms: i64) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(root) else { return Vec::new() };
    let mut result = Vec::new();
    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else { continue };
        let name = entry.file_name().to_string_lossy().to_string();
        if !file_type.is_dir() || name.to_lowercase() == own_name.to_lowercase() {
            continue;
        }
        let dir = root.join(&name);
        if name.to_lowercase().ends_with(".neu") {
            // verschwunden oder nicht lesbar: überspringen
            let Ok(meta) = std::fs::metadata(&dir) else { continue };
            let Some(born) = birth_ms(&meta) else { continue };
            if (now_ms as f64) - born < MIN_AGE_MS as f64 {
                continue;
            }
        }
        result.push(dir);
    }
    result
}

/// Läuft die App aus der einzelnen EXE (entpackt unter app_cache_root) statt installiert?
pub fn is_portable(exec_path: &Path, local_app_data: Option<&str>) -> bool {
    let root = app_cache_root(local_app_data);
    if root.as_os_str().is_empty() {
        return false;
    }
    let grand = exec_path.parent().and_then(Path::parent);
    grand.is_some_and(|g| g.to_string_lossy().to_lowercase() == root.to_string_lossy().to_lowercase())
}

/// Nur mit der Einzelinstanz-Sperre aufrufen: Dann läuft kein anderer Stand mehr.
pub fn cleanup_old_versions() {
    let local = std::env::var("LOCALAPPDATA").ok();
    let exe = crate::paths::config().exe;
    if !is_portable(&exe, local.as_deref()) {
        return;
    }
    let root = app_cache_root(local.as_deref());
    let own = exe.parent().and_then(Path::file_name).map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    for dir in stale_app_dirs(&root, &own, crate::util::now_ms()) {
        std::thread::spawn(move || {
            let _ = std::fs::remove_dir_all(dir);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root_with(dirs: &[&str], files: &[&str]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        for name in dirs {
            std::fs::create_dir_all(root.path().join(name).join("resources")).unwrap();
        }
        for name in files {
            std::fs::write(root.path().join(name), "").unwrap();
        }
        root
    }

    fn found(root: &Path, own: &str, now: i64) -> Vec<String> {
        let mut v: Vec<String> =
            stale_app_dirs(root, own, now).iter().map(|d| d.file_name().unwrap().to_string_lossy().to_string()).collect();
        v.sort();
        v
    }

    fn later() -> i64 {
        crate::util::now_ms() + MIN_AGE_MS + 60000
    }

    #[test]
    fn ordner_unter_localappdata_ask_desktop_app() {
        let base = "C:\\Users\\lea\\AppData\\Local";
        assert_eq!(app_cache_root(Some(base)), PathBuf::from(base).join("ASK-Desktop").join("app"));
        assert_eq!(app_cache_root(None), PathBuf::new());
    }

    #[test]
    fn findet_aeltere_staende_laesst_den_eigenen_stehen() {
        let root = root_with(&["0.1.0-aaa", "0.1.0-bbb", "0.2.0-ccc"], &[]);
        assert_eq!(found(root.path(), "0.2.0-ccc", crate::util::now_ms()), ["0.1.0-aaa", "0.1.0-bbb"]);
    }

    #[test]
    fn eigener_stand_auch_bei_anderer_schreibweise() {
        let root = root_with(&["0.2.0-CCC"], &[]);
        assert!(found(root.path(), "0.2.0-ccc", crate::util::now_ms()).is_empty());
    }

    #[test]
    fn abgebrochenes_entpacken_neu_erst_nach_einer_stunde() {
        let root = root_with(&["0.2.0-ccc", "0.2.0-ddd.neu"], &[]);
        assert!(found(root.path(), "0.2.0-ccc", crate::util::now_ms()).is_empty());
        assert_eq!(found(root.path(), "0.2.0-ccc", later()), ["0.2.0-ddd.neu"]);
    }

    #[test]
    fn dateien_bleiben_stehen() {
        let root = root_with(&["0.2.0-ccc"], &["notiz.txt"]);
        assert!(found(root.path(), "0.2.0-ccc", later()).is_empty());
    }

    #[test]
    fn fehlender_ordner_ergibt_keine_treffer() {
        assert!(found(&std::env::temp_dir().join("gibt-es-nicht-ask"), "x", later()).is_empty());
    }

    #[test]
    fn einzelne_exe_wird_erkannt() {
        let local = "C:\\Users\\lea\\AppData\\Local";
        let exe = PathBuf::from(local).join("ASK-Desktop").join("app").join("0.3.6-abc").join("ASK.exe");
        assert!(is_portable(&exe, Some(local)));
        assert!(!is_portable(&exe, None));
        assert!(!is_portable(&PathBuf::from(local).join("Programs").join("ASK").join("ASK.exe"), Some(local)));
    }
}
