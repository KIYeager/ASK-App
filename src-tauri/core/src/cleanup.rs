// Noch nicht portiert (src/main/cleanup.js).

// TEMP-STUB (wird beim Zusammenführen ersetzt)
// Nur damit workspace.rs übersetzt; die echte Fassung kommt mit dem Port von cleanup.js.
/// Löscht alte Dateien in .ask/skripte, .ask/arbeit, .ask/anhaenge; liefert die Anzahl gelöschter Dateien.
pub fn cleanup_work_dirs(_root: &std::path::Path) -> usize {
    0
}

/// Markiert .ask als versteckt (nur unter Windows)
pub fn hide_folder(_dir: &std::path::Path) {}
// TEMP-STUB Ende
