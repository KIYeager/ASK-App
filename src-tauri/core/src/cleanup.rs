//! Hält den versteckten Ordner .ask klein: Hilfsskripte, Zwischendateien und Anhänge
//! (.ask/skripte/<Chat>, .ask/arbeit/<Chat>, .ask/anhaenge/<Chat>) werden nach einiger Zeit gelöscht.
//! Chats, Anleitungen und Aufgabenliste bleiben. Die Sicherung räumt snapshot.rs selbst auf.
//!
//! Öffentliche Schnittstelle:
//! - `cleanup_work_dirs(root) -> usize`, `cleanup_work_dirs_at(root, now_ms) -> usize`
//! - `hide_folder(dir)`
//! - `KEEP_DAYS`, `WORK_DIRS`

use std::path::Path;

pub const KEEP_DAYS: i64 = 14;
pub const WORK_DIRS: [&str; 3] = ["skripte", "arbeit", "anhaenge"];

/// Löscht Dateien, die seit KEEP_DAYS Tagen unverändert sind, und danach leere Unterordner.
/// Liefert die Anzahl gelöschter Dateien.
pub fn cleanup_work_dirs(root: &Path) -> usize {
    cleanup_work_dirs_at(root, crate::util::now_ms())
}

/// Wie `cleanup_work_dirs`, mit einer vorgegebenen Zeit (Millisekunden seit 1970)
pub fn cleanup_work_dirs_at(root: &Path, now_ms: i64) -> usize {
    let limit = (now_ms - KEEP_DAYS * 24 * 60 * 60 * 1000) as f64;
    let mut removed = 0;
    for sub in WORK_DIRS {
        walk(&root.join(".ask").join(sub), true, limit, &mut removed);
    }
    removed
}

/// Temporäre Dateien der Befehle des Modells (TEMP und TMP zeigen auf laufzeit\tmp, terminal.rs): Windows räumt
/// dort nie auf, darum löscht die App beim Start des Terminals, was älter als `KEEP_DAYS` Tage ist.
pub fn cleanup_temp_dir(dir: &Path) -> usize {
    cleanup_temp_dir_at(dir, crate::util::now_ms())
}

pub fn cleanup_temp_dir_at(dir: &Path, now_ms: i64) -> usize {
    let limit = (now_ms - KEEP_DAYS * 24 * 60 * 60 * 1000) as f64;
    let mut removed = 0;
    walk(dir, true, limit, &mut removed);
    removed
}

fn mtime_ms(meta: &std::fs::Metadata) -> Option<f64> {
    let d = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(d.as_secs() as f64 * 1000.0 + f64::from(d.subsec_nanos()) / 1e6)
}

fn walk(dir: &Path, top: bool, limit: f64, removed: &mut usize) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let full = entry.path();
        // gesperrt oder verschwunden: beim nächsten Mal
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            walk(&full, false, limit, removed);
        } else if let Ok(meta) = std::fs::metadata(&full) {
            if mtime_ms(&meta).is_some_and(|t| t < limit) && std::fs::remove_file(&full).is_ok() {
                *removed += 1;
            }
        }
    }
    if !top {
        // nicht leer oder gesperrt: bleibt
        if std::fs::read_dir(dir).is_ok_and(|mut d| d.next().is_none()) {
            let _ = std::fs::remove_dir(dir);
        }
    }
}

/// Markiert .ask als versteckt (wie Git den Ordner .git), damit Lernende ihn im Explorer nicht sehen.
pub fn hide_folder(dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut cmd = std::process::Command::new("attrib");
        cmd.arg("+h").arg(dir).creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        std::thread::spawn(move || {
            let _ = cmd.status();
        });
    }
    #[cfg(not(windows))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn file(root: &Path, rel: &str, age_days: u64) -> std::path::PathBuf {
        let full = root.join(rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, "x").unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_days * 86400);
        std::fs::OpenOptions::new().write(true).open(&full).unwrap().set_modified(t).unwrap();
        full
    }

    #[test]
    fn alte_skripte_und_zwischendateien_weg_neue_und_chats_bleiben() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let keep = KEEP_DAYS as u64;
        let old_script = file(root, ".ask/skripte/chat-a/alt.py", keep + 1);
        let new_script = file(root, ".ask/skripte/chat-b/neu.py", 1);
        let old_work = file(root, ".ask/arbeit/chat-a/versuch.docx", keep + 5);
        let chat = file(root, ".ask/chats/chat-a.json", 100);
        let result = file(root, "ausgabe/bericht.docx", 100);
        assert_eq!(cleanup_work_dirs(root), 2);
        assert!(!old_script.exists() && !old_work.exists());
        assert!(!root.join(".ask/skripte/chat-a").exists(), "leerer Chat-Ordner entfernt");
        assert!(root.join(".ask/skripte").exists(), "Hauptordner bleibt");
        assert!(new_script.exists() && chat.exists() && result.exists());
    }

    #[test]
    fn temp_ordner_alte_dateien_weg_neue_und_der_ordner_selbst_bleiben() {
        let dir = tempfile::tempdir().unwrap();
        let tmp = dir.path().join("tmp");
        let keep = KEEP_DAYS as u64;
        let old = file(&tmp, "pip-build-1/alt.whl", keep + 3);
        let old_top = file(&tmp, "tmpabc.txt", keep + 1);
        let new = file(&tmp, "lauf/neu.csv", 2);
        assert_eq!(cleanup_temp_dir(&tmp), 2);
        assert!(!old.exists() && !old_top.exists() && new.exists());
        assert!(!tmp.join("pip-build-1").exists(), "leerer Unterordner entfernt");
        assert!(tmp.exists(), "der Temp-Ordner selbst bleibt");
        assert_eq!(cleanup_temp_dir(&dir.path().join("gibt-es-nicht")), 0);
    }

    #[test]
    fn ohne_ask_nichts_zu_tun() {
        assert_eq!(cleanup_work_dirs(&std::env::temp_dir().join("gibt-es-nicht-ask")), 0);
    }
}
