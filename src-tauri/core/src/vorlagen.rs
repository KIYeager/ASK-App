//! Vorlagen der TF Bern (ENTSCHEIDE E65): Auf den Laptops der Schule liegen sie in `C:\Vorlagen`, für Lernende
//! nur zum Lesen. Ist der Ordner da, darf das Terminal dort auflisten, lesen und suchen (nie schreiben), das Modell
//! erfährt im Systemprompt davon, und Befehle, die nur `C:\Vorlagen` nennen, gelten nicht als «ausserhalb».

use std::path::{Path, PathBuf};

pub const DIR: &str = r"C:\Vorlagen";
/// Dateien mit diesen Endungen stehen im Systemprompt (höchstens `MAX_LISTED`)
const LISTED: [&str; 12] = ["dotx", "dotm", "docx", "xltx", "xltm", "xlsx", "potx", "potm", "pptx", "pdf", "thmx", "oft"];
const MAX_LISTED: usize = 80;

/// Der Ordner, wenn es ihn gibt. `ASK_VORLAGEN` ersetzt ihn (Tests, Entwicklung).
pub fn dir() -> Option<PathBuf> {
    let p = std::env::var_os("ASK_VORLAGEN").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(DIR));
    p.is_dir().then_some(p)
}

/// Ordner nur zum Lesen für das Terminal
pub fn read_only_dirs() -> Vec<String> {
    dir().map(|d| vec![d.to_string_lossy().to_string()]).unwrap_or_default()
}

/// Vorlagen im Ordner (relativ, mit «/»), sortiert, höchstens bis zur dritten Ebene
pub fn list(dir: &Path) -> (Vec<String>, bool) {
    let mut out = Vec::new();
    let mut more = false;
    let mut stack = vec![(dir.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for e in entries {
            let p = e.path();
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') || name.starts_with("~$") {
                continue;
            }
            if p.is_dir() {
                if depth < 3 {
                    stack.push((p, depth + 1));
                }
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| LISTED.contains(&x.to_lowercase().as_str())) {
                if out.len() >= MAX_LISTED {
                    more = true;
                    continue;
                }
                out.push(p.strip_prefix(dir).unwrap_or(&p).to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    (out, more)
}

/// Abschnitt für den Systemprompt; leer ohne Ordner
pub fn system_prompt() -> String {
    let Some(d) = dir() else { return String::new() };
    let (files, more) = list(&d);
    let shown = d.to_string_lossy();
    let mut s = format!(
        "\n\n# Vorlagen der TF Bern\nAuf diesem Laptop liegen die Vorlagen der TF Bern in `{shown}` (nur lesen). Möchte die Person ein Dokument, \
eine Präsentation, eine Tabelle oder einen Brief der TF Bern, nimm die passende Vorlage: kopiere sie in den Arbeitsordner und arbeite \
in der Kopie. In `{shown}` änderst, löschst und speicherst du nie etwas. Suchen und lesen kannst du dort mit list_files, find_files und \
read_file.\nEine Vorlage (.dotx, .xltx, .potx) öffnen python-docx und python-pptx nicht direkt. So wird daraus eine normale Datei:\n\
```python\nimport zipfile\ndef vorlage_kopieren(quelle, ziel):  # z. B. Brief.dotx -> brief.docx\n    typen = {{b\"wordprocessingml.template.main\": b\"wordprocessingml.document.main\", \
b\"spreadsheetml.template.main\": b\"spreadsheetml.sheet.main\", b\"presentationml.template.main\": b\"presentationml.presentation.main\"}}\n\
    with zipfile.ZipFile(quelle) as zin, zipfile.ZipFile(ziel, \"w\", zipfile.ZIP_DEFLATED) as zout:\n        for item in zin.infolist():\n\
            data = zin.read(item.filename)\n            if item.filename == \"[Content_Types].xml\":\n                for alt, neu in typen.items():\n\
                    data = data.replace(alt, neu)\n            zout.writestr(item, data)\n```"
    );
    if files.is_empty() {
        return s;
    }
    s.push_str("\nVorhanden:\n");
    for f in &files {
        s.push_str(&format!("- {f}\n"));
    }
    if more {
        s.push_str("- … (weitere, mit find_files suchen)\n");
    }
    s
}

/// Vorlagen, deren Pfad alle Wörter der Suche enthält (für @ im Eingabefeld); «vorl…» zeigt alle.
/// Einträge wie bei `Workspace::find`, mit absolutem Pfad und `template: true`.
pub fn find(query: &str) -> Vec<serde_json::Value> {
    let Some(d) = dir() else { return Vec::new() };
    let q = query.to_lowercase();
    let words: Vec<&str> = q.split_whitespace().collect();
    let all = words.len() == 1 && "vorlagen".starts_with(words[0]) && words[0].len() >= 4;
    list(&d)
        .0
        .into_iter()
        .filter(|rel| all || (!words.is_empty() && words.iter().all(|w| rel.to_lowercase().contains(w))))
        .map(|rel| {
            let full = d.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
            let name = rel.rsplit('/').next().unwrap_or(&rel).to_string();
            serde_json::json!({ "name": name, "path": full.to_string_lossy(), "dir": false, "template": true })
        })
        .collect()
}

/// Ist das ein Pfad in den Vorlagen? (auch mit «/», wie das Eingabefeld ihn schreibt)
pub fn is_template_ref(path: &str) -> bool {
    let norm = |s: &str| s.replace('\\', "/").to_lowercase().trim_end_matches('/').to_string();
    let p = norm(path);
    let mut roots = vec![norm(DIR)];
    if let Some(d) = dir() {
        roots.push(norm(&d.to_string_lossy()));
    }
    roots.iter().any(|r| p.starts_with(&format!("{r}/")))
}

/// Gibt es die Vorlage? (Pfad aus dem Eingabefeld)
pub fn template_exists(path: &str) -> bool {
    is_template_ref(path) && Path::new(&path.replace('/', std::path::MAIN_SEPARATOR_STR)).is_file()
}

/// Für die Prüfung von Befehlen: Nennungen von `C:\Vorlagen` entfernen, damit sie nicht als «ausserhalb» gelten
pub fn without_templates(command: &str) -> String {
    static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)c:[\\/]vorlagen\b").expect("Regex"));
    if dir().is_none() {
        return command.to_string();
    }
    RE.replace_all(command, "VORLAGEN").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn liste_und_systemprompt() {
        let base = tempfile::tempdir().unwrap();
        let v = base.path();
        std::fs::create_dir_all(v.join("Word").join("Briefe")).unwrap();
        std::fs::create_dir_all(v.join("PowerPoint")).unwrap();
        std::fs::write(v.join("Word").join("Briefe").join("Brief TF Bern.dotx"), "x").unwrap();
        std::fs::write(v.join("PowerPoint").join("Praesentation.potx"), "x").unwrap();
        std::fs::write(v.join("Word").join("~$Brief.dotx"), "x").unwrap();
        std::fs::write(v.join("liesmich.txt"), "x").unwrap();
        let (files, more) = list(v);
        assert_eq!(files, vec!["PowerPoint/Praesentation.potx", "Word/Briefe/Brief TF Bern.dotx"]);
        assert!(!more);
        assert!(is_template_ref("C:/Vorlagen/Word/Brief.dotx"));
        assert!(is_template_ref(r"c:\vorlagen\Word\Brief.dotx"));
        assert!(!is_template_ref("C:/Vorlagenx/Brief.dotx"));
        assert!(!is_template_ref("bericht.docx"));
    }
}
