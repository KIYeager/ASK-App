//! Pfade aus Dateien, die das Modell ändern kann (.ask im Arbeitsordner) oder die von aussen kommen
//! (GitHub), sind nicht vertrauenswürdig: Der Hauptprozess läuft mit normalen Rechten, das Terminal
//! mit niedrigen. Jeder solche Pfad wird hier geprüft, bevor der Hauptprozess damit schreibt.

use anyhow::{bail, Result};
use std::path::{Path, PathBuf};

/// 40 Hex-Zeichen (SHA-1)
pub fn is_sha1(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Relativer Pfad mit / als Trenner, der sicher im Ordner bleibt: keine leeren Teile, kein «..»,
/// kein Laufwerk oder Datenstrom (:), kein Rückstrich, keine Steuerzeichen. Teile, die auf Punkt
/// oder Leerzeichen enden, lehnt Windows nicht ab, sondern kürzt sie («.ask.» wird «.ask»).
pub fn safe_rel(rel: &str) -> bool {
    if rel.is_empty() || rel.encode_utf16().count() > 1000 || rel.starts_with('/') {
        return false;
    }
    if rel.chars().any(|c| matches!(c, '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*') || (c as u32) < 0x20) {
        return false;
    }
    rel.split('/').all(|part| !part.is_empty() && part != "." && part != ".." && !part.ends_with(['.', ' ']))
}

fn real(p: &Path) -> std::io::Result<String> {
    Ok(dunce::canonicalize(p)?.to_string_lossy().to_lowercase())
}

/// Absoluter Pfad zu rel im Ordner root. Fehler, wenn rel nicht sicher ist oder ein vorhandener
/// Teil des Wegs (Verknüpfung, Junction) aus dem Ordner hinausführt.
pub fn safe_join(root: &Path, rel: &str) -> Result<PathBuf> {
    if !safe_rel(rel) {
        bail!("Unsicherer Pfad: {}", crate::util::take_chars(rel, 120));
    }
    let mut full = root.to_path_buf();
    for part in rel.split('/') {
        full.push(part);
    }
    let base = real(root)?;
    let sep = std::path::MAIN_SEPARATOR;
    let mut existing = full.clone();
    loop {
        match real(&existing) {
            Ok(r) => {
                if r != base && !r.starts_with(&format!("{base}{sep}")) {
                    bail!("Der Pfad führt aus dem Ordner hinaus: {}", crate::util::take_chars(rel, 120));
                }
                return Ok(full);
            }
            Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => {
                let Some(parent) = existing.parent().map(Path::to_path_buf) else { return Err(e.into()) };
                if parent == existing {
                    return Err(e.into());
                }
                existing = parent; // noch nicht vorhanden: den nächsten vorhandenen Ordner prüfen
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rel_rules() {
        assert!(safe_rel("a/b.txt"));
        assert!(safe_rel(".ask/x"));
        for bad in ["", "/a", "a//b", "a/../b", "..", "a\\b", "C:x", "a/b.", "a /b", "a\u{1}"] {
            assert!(!safe_rel(bad), "{bad:?}");
        }
    }

    #[test]
    fn join_stays_inside() {
        let dir = tempfile::tempdir().unwrap();
        let p = safe_join(dir.path(), "neu/datei.txt").unwrap();
        assert!(p.ends_with("neu/datei.txt") || p.ends_with("neu\\datei.txt"));
        assert!(safe_join(dir.path(), "../x").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn join_rejects_symlink_out() {
        let dir = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(out.path(), dir.path().join("weg")).unwrap();
        assert!(safe_join(dir.path(), "weg/x.txt").is_err());
    }
}
