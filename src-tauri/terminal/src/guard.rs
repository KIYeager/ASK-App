//! Ordnersperre für die Datei-API (Port von python/ask_terminal/guard.py).
//!
//! Jeder Pfad aus einer Anfrage wird aufgelöst; nur Pfade innerhalb des Arbeitsordners kommen durch.
//! Verknüpfungen (Junctions, Symlinks) werden vorher aufgelöst, damit sie nicht hinausführen.
//!
//! Öffentliche Schnittstelle:
//! - `PathGuard::new(root)`, `inside(path)`, `check(path) -> Result<(), Denied>`
//! - `normpath(path)`: lexikalisch bereinigen wie `os.path.normpath`
//! - `realpath(path)`: Verknüpfungen auflösen wie `os.path.realpath` (auch wenn der Pfad nicht ganz existiert)

use std::path::{Component, Path, PathBuf};

/// Zugriff ausserhalb des Arbeitsordners (wird zu HTTP 403)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Denied(pub String);

impl std::fmt::Display for Denied {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Denied {}

/// Wie `os.path.normpath`: «.» und «..» lexikalisch entfernen, ohne das Dateisystem zu fragen.
pub fn normpath(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    let mut depth = 0usize; // Anzahl normaler Teile hinter Wurzel/Laufwerk
    let mut rooted = false;
    for comp in path.components() {
        match comp {
            Component::Prefix(p) => out.push(p.as_os_str()),
            Component::RootDir => {
                out.push(Component::RootDir.as_os_str());
                rooted = true;
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if depth > 0 {
                    out.pop();
                    depth -= 1;
                } else if !rooted {
                    out.push("..");
                }
                // an der Wurzel: «..» bleibt dort (wie normpath)
            }
            Component::Normal(name) => {
                out.push(name);
                depth += 1;
            }
        }
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}

/// Wie `os.path.realpath` (nicht streng): der längste vorhandene Anfang wird aufgelöst (Symlinks,
/// Junctions, 8.3-Namen), der nicht vorhandene Rest unverändert angehängt.
pub fn realpath(path: &Path) -> PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = path.to_path_buf();
    loop {
        if let Ok(real) = std::fs::canonicalize(&cur) {
            let mut real = strip_verbatim(real);
            for part in tail.iter().rev() {
                real.push(part);
            }
            return normpath(&real);
        }
        match (cur.parent().map(Path::to_path_buf), cur.file_name().map(|n| n.to_os_string())) {
            (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => {
                tail.push(name);
                cur = parent;
            }
            _ => {
                // nichts davon vorhanden: der Pfad selbst
                let mut base = cur;
                for part in tail.iter().rev() {
                    base.push(part);
                }
                return normpath(&base);
            }
        }
    }
}

/// `\\?\C:\x` → `C:\x`, `\\?\UNC\server\share` → `\\server\share`
fn strip_verbatim(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let s = path.to_string_lossy().to_string();
        if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = s.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path
}

/// Wie `os.path.normcase`: unter Windows klein und mit «\», sonst unverändert
fn normcase(path: &Path) -> String {
    let s = path.to_string_lossy().to_string();
    if cfg!(windows) {
        s.replace('/', "\\").to_lowercase()
    } else {
        s
    }
}

pub struct PathGuard {
    display_root: String,
    root: String,
}

impl PathGuard {
    pub fn new(root: &Path) -> Self {
        let real = realpath(root);
        let display_root = real.to_string_lossy().to_string();
        let root = normcase(&real).trim_end_matches(['\\', '/']).to_string();
        PathGuard { display_root, root }
    }

    /// Der aufgelöste Arbeitsordner, wie er in Meldungen steht
    pub fn display_root(&self) -> &str {
        &self.display_root
    }

    pub fn inside(&self, path: &Path) -> bool {
        if path.to_string_lossy().contains('\0') {
            return false;
        }
        let resolved = normcase(&realpath(path));
        let sep = std::path::MAIN_SEPARATOR;
        resolved == self.root || resolved.starts_with(&format!("{}{}", self.root, sep))
    }

    pub fn check(&self, path: &Path) -> Result<(), Denied> {
        if self.inside(path) {
            Ok(())
        } else {
            Err(Denied(format!(
                "Zugriff verweigert: {} liegt ausserhalb des Arbeitsordners {}",
                path.display(),
                self.display_root
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normpath_wie_python() {
        let p = |s: &str| normpath(Path::new(s)).to_string_lossy().replace('\\', "/");
        assert_eq!(p("/a/b/../c/./d"), "/a/c/d");
        assert_eq!(p("/a/../../x"), "/x");
        assert_eq!(p("a/../.."), "..");
        assert_eq!(p(""), ".");
    }

    #[test]
    fn innerhalb_und_ausserhalb() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("Arbeitsordner Prüfung");
        let outside = base.path().join("ausserhalb");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let guard = PathGuard::new(&root);
        assert!(guard.inside(&root));
        assert!(guard.inside(&root.join("neu").join("datei.txt")));
        assert!(!guard.inside(&outside));
        assert!(!guard.inside(&base.path().join("Arbeitsordner Prüfung2")));
        assert!(!guard.inside(&root.join("..").join("ausserhalb")));
        assert!(guard.check(&outside).unwrap_err().0.starts_with("Zugriff verweigert:"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_nach_aussen_wird_aufgeloest() {
        let base = tempfile::tempdir().unwrap();
        let root = base.path().join("ordner");
        let outside = base.path().join("aussen");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("weg")).unwrap();
        let guard = PathGuard::new(&root);
        assert!(!guard.inside(&root.join("weg")));
        assert!(!guard.inside(&root.join("weg").join("neu.txt")));
    }
}
