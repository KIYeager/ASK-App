//! Kursunterlagen im Ordner «Quellen» eines Projekts (ENTSCHEIDE E84): Text in Teilen mit Ort (Seite, Folie,
//! Abschnitt, Blatt), ein Index in `.ask/quellen/index.json` und eine Suche (BM25) mit Ausschnitt.
//!
//! Läuft im Terminal-Prozess (Low Integrity), weil hier fremde Dateien geparst werden (E33, E61). Der Index liest
//! nur neu, was sich geändert hat (Grösse und Änderungszeit), und vergisst gelöschte Dateien.
//!
//! Öffentliche Schnittstelle:
//! - `FOLDER`: Name des Ordners im Projekt
//! - `extract_parts(pfad) -> Result<Vec<Part>, String>`: Text einer Datei in Teilen mit Ort
//! - `tokens(text)`: Wörter für die Suche (klein, Umlaute vereinheitlicht, einfach gestutzt)
//! - `Sources::new()`, `search(&FileApi, suche, anzahl)`, `read(&FileApi, pfad, seite, ort)`, `list(&FileApi)`

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::documents;
use crate::files::{ApiError, FileApi};

/// Ordner im Projekt, dessen Dateien als Quellen gelten
pub const FOLDER: &str = "Quellen";
const INDEX: &str = ".ask/quellen/index.json";
const VERSION: u32 = 1;
/// Höchstens so viele Unterlagen werden durchsucht
pub const MAX_FILES: usize = 200;
/// Grössere Dateien werden nicht gelesen
pub const MAX_BYTES: u64 = 50 * 1024 * 1024;
/// Grösse der Stücke für die Suche (Zeichen) und ihre Überlappung
const CHUNK: usize = 1500;
const OVERLAP: usize = 200;
/// Teil ohne feste Gliederung (Text, Word ohne Überschriften): so lang werden die Teile (Zeichen)
const BLOCK: usize = 3000;
/// Länge des Ausschnitts eines Treffers und eines gelesenen Teils (Zeichen)
const SNIPPET: usize = 600;
const READ_MAX: usize = 20_000;
/// Seite mit weniger Zeichen gilt als Bild oder Scan
const SCAN_CHARS: usize = 20;

const IMAGE_EXT: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const TEXT_EXT: [&str; 24] = [
    "txt", "md", "markdown", "csv", "tsv", "json", "html", "htm", "xml", "py", "js", "ts", "java", "c", "h", "cpp", "cs", "rs", "css", "sql",
    "tex", "yaml", "yml", "ipynb",
];
const STOPWORDS: [&str; 48] = [
    "der", "die", "das", "den", "dem", "des", "und", "oder", "ein", "eine", "einer", "eines", "einem", "einen", "ist", "sind", "zu", "im", "in", "am",
    "an", "auf", "mit", "von", "fuer", "was", "wie", "wer", "wo", "nicht", "es", "sich", "auch", "als", "bei", "aus", "nach", "so", "dass", "the",
    "of", "and", "to", "is", "are", "for", "on", "with",
];

/// Ein Teil einer Unterlage: Ort für Menschen («S. 14», «Folie 3», «Abschnitt «Zellen»»), Seite für die Vorschau
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub ort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seite: Option<u32>,
    pub text: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Entry {
    size: u64,
    mtime: u64,
    #[serde(default)]
    parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hinweis: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct Index {
    version: u32,
    files: BTreeMap<String, Entry>,
}

fn extension(path: &Path) -> String {
    path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default()
}

/// Text einer Datei in Teilen mit Ort. Fehler als Text für Menschen.
pub fn extract_parts(path: &Path) -> Result<Vec<Part>, String> {
    let p = path.to_string_lossy().to_string();
    let ext = extension(path);
    let parts = match ext.as_str() {
        "pdf" => documents::pdf_pages_safe(&p)
            .map_err(|e| e.message)?
            .into_iter()
            .enumerate()
            .map(|(i, t)| Part { ort: format!("S. {}", i + 1), seite: Some(i as u32 + 1), text: tidy(&t) })
            .collect(),
        "docx" => {
            let data = std::fs::read(path).map_err(|e| e.to_string())?;
            let sections = documents::docx_sections(&data).map_err(|e| e.message)?;
            if sections.iter().all(|(t, _)| t.is_empty()) {
                blocks(&sections.into_iter().map(|(_, t)| t).collect::<Vec<_>>().join("\n"))
            } else {
                sections
                    .into_iter()
                    .map(|(title, text)| {
                        if title.is_empty() {
                            Part { ort: "Anfang".into(), seite: None, text: tidy(&text) }
                        } else {
                            Part { ort: format!("Abschnitt «{title}»"), seite: None, text: tidy(&format!("{title}\n{text}")) }
                        }
                    })
                    .collect()
            }
        }
        "pptx" | "odp" => split_marked(&documents::extract(&p).map_err(|e| e.message)?)
            .into_iter()
            .map(|(mark, text)| {
                let n = mark.strip_prefix("Slide ").and_then(|n| n.trim().parse::<u32>().ok());
                Part { ort: n.map(|n| format!("Folie {n}")).unwrap_or(mark), seite: n, text: tidy(&text) }
            })
            .collect(),
        "xlsx" | "xls" | "ods" => split_marked(&documents::extract(&p).map_err(|e| e.message)?)
            .into_iter()
            .map(|(mark, text)| Part { ort: format!("Blatt «{mark}»"), seite: None, text: tidy(&text) })
            .collect(),
        "md" | "markdown" => markdown_sections(&read_text(path)?),
        e if documents::supports(&p) && !e.is_empty() => blocks(&documents::extract(&p).map_err(|e| e.message)?),
        e if TEXT_EXT.contains(&e) => blocks(&read_text(path)?),
        _ => return Err("kein Textformat".into()),
    };
    Ok(parts)
}

fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
}

/// Zeilenenden vereinheitlichen, Leerraum am Zeilenende weg, höchstens eine Leerzeile am Stück
fn tidy(text: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line);
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

/// Text mit Trennzeilen `--- Name ---` (aus `documents::extract`) in (Name, Text)
fn split_marked(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, Vec<&str>)> = Vec::new();
    for line in text.lines() {
        let mark = line.strip_prefix("--- ").and_then(|l| l.strip_suffix(" ---")).filter(|m| !m.trim().is_empty());
        match (mark, out.last_mut()) {
            (Some(m), _) => out.push((m.to_string(), Vec::new())),
            (None, Some(last)) => last.1.push(line),
            (None, None) => out.push((String::new(), vec![line])),
        }
    }
    out.into_iter().filter(|(m, lines)| !m.is_empty() || lines.iter().any(|l| !l.trim().is_empty())).map(|(m, lines)| (m, lines.join("\n"))).collect()
}

/// Markdown nach Überschriften (`#`); Text davor heisst «Anfang»
fn markdown_sections(text: &str) -> Vec<Part> {
    let mut parts: Vec<(String, Vec<&str>)> = vec![(String::new(), Vec::new())];
    let mut fence = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fence = !fence;
        }
        let heading = if fence { None } else { line.strip_prefix('#').map(|h| h.trim_start_matches('#').trim()).filter(|h| !h.is_empty()) };
        match heading {
            Some(h) => parts.push((h.to_string(), vec![line])),
            None => {
                if let Some(p) = parts.last_mut() {
                    p.1.push(line);
                }
            }
        }
    }
    let parts: Vec<Part> = parts
        .into_iter()
        .map(|(h, lines)| Part { ort: if h.is_empty() { "Anfang".into() } else { format!("Abschnitt «{h}»") }, seite: None, text: tidy(&lines.join("\n")) })
        .filter(|p| !p.text.trim().is_empty())
        .collect();
    if parts.len() <= 1 {
        return blocks(text);
    }
    parts
}

/// Text ohne Gliederung in Teile von etwa `BLOCK` Zeichen an Zeilengrenzen («Teil 1», «Teil 2» …)
fn blocks(text: &str) -> Vec<Part> {
    let text = tidy(text);
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if !current.is_empty() && current.chars().count() + 1 + line.chars().count() > BLOCK {
            parts.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);
    }
    if !current.trim().is_empty() {
        parts.push(current);
    }
    parts.into_iter().enumerate().map(|(i, t)| Part { ort: format!("Teil {}", i + 1), seite: None, text: t }).collect()
}

// ---------- Wörter ----------

/// Wörter für die Suche: klein, ä → ae, ö → oe, ü → ue, ß → ss, Akzente weg, einfach gestutzt
/// (-ungen, -ung, -en, -er, -es, -em, -e, -n, -s), ohne Füllwörter und ohne einzelne Zeichen
pub fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                match l {
                    'ä' => current.push_str("ae"),
                    'ö' => current.push_str("oe"),
                    'ü' => current.push_str("ue"),
                    'ß' => current.push_str("ss"),
                    'à' | 'á' | 'â' => current.push('a'),
                    'è' | 'é' | 'ê' | 'ë' => current.push('e'),
                    'ì' | 'í' | 'î' | 'ï' => current.push('i'),
                    'ò' | 'ó' | 'ô' => current.push('o'),
                    'ù' | 'ú' | 'û' => current.push('u'),
                    'ç' => current.push('c'),
                    other => current.push(other),
                }
            }
        } else {
            flush_word(&mut current, &mut out);
        }
    }
    flush_word(&mut current, &mut out);
    out
}

fn flush_word(word: &mut String, out: &mut Vec<String>) {
    if word.chars().count() >= 2 && !STOPWORDS.contains(&word.as_str()) {
        out.push(stem(word));
    }
    word.clear();
}

fn stem(word: &str) -> String {
    for suffix in ["ungen", "ung", "en", "er", "es", "em", "e", "n", "s"] {
        if let Some(rest) = word.strip_suffix(suffix) {
            if rest.chars().count() >= 4 {
                return rest.to_string();
            }
        }
    }
    word.to_string()
}

// ---------- Stücke und Ausschnitte ----------

/// Byteposition des Zeichens Nummer `n` (oder Textende)
fn char_pos(text: &str, n: usize) -> usize {
    text.char_indices().nth(n).map(|(i, _)| i).unwrap_or(text.len())
}

/// Stücke von etwa `CHUNK` Zeichen mit `OVERLAP` Überlappung, an Leerraum getrennt
fn pieces(text: &str) -> Vec<&str> {
    let total = text.chars().count();
    if total <= CHUNK {
        return vec![text];
    }
    let mut out = Vec::new();
    let mut start = 0; // in Zeichen
    loop {
        let mut end = (start + CHUNK).min(total);
        if end < total {
            // zurück zum letzten Leerraum, höchstens 200 Zeichen
            let window: Vec<char> = text[char_pos(text, end - 200)..char_pos(text, end)].chars().collect();
            if let Some(k) = window.iter().rposition(|c| c.is_whitespace()) {
                end = end - 200 + k;
            }
        }
        out.push(&text[char_pos(text, start)..char_pos(text, end)]);
        if end >= total {
            break;
        }
        start = end.saturating_sub(OVERLAP).max(start + 1);
    }
    out
}

/// Ausschnitt um die Zeile mit den meisten Suchwörtern, höchstens `SNIPPET` Zeichen
fn snippet(text: &str, terms: &HashSet<String>) -> String {
    let mut best = (0usize, 0usize); // (Treffer, Byteposition der Zeile)
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let hits = tokens(line).into_iter().filter(|t| terms.contains(t)).collect::<HashSet<_>>().len();
        if hits > best.0 {
            best = (hits, offset);
        }
        offset += line.len();
    }
    let before = text[..best.1].chars().count();
    let from = before.saturating_sub(150);
    let total = text.chars().count();
    let to = (from + SNIPPET).min(total);
    let mut out = text[char_pos(text, from)..char_pos(text, to)].trim().to_string();
    if from > 0 {
        out = format!("… {out}");
    }
    if to < total {
        out.push_str(" …");
    }
    out
}

fn clip(text: &str, max: usize) -> (String, bool) {
    if text.chars().count() <= max {
        return (text.to_string(), false);
    }
    (format!("{} …", &text[..char_pos(text, max)]), true)
}

// ---------- Index ----------

/// Index und Suche; hält den zuletzt benutzten Index im Speicher
#[derive(Default)]
pub struct Sources {
    cache: Mutex<Option<(PathBuf, Index)>>,
}

fn load(home: &Path) -> Index {
    std::fs::read(home.join(INDEX))
        .ok()
        .and_then(|b| serde_json::from_slice::<Index>(&b).ok())
        .filter(|i| i.version == VERSION)
        .unwrap_or(Index { version: VERSION, files: BTreeMap::new() })
}

fn save(home: &Path, index: &Index) {
    let path = home.join(INDEX);
    let Some(dir) = path.parent() else { return };
    if std::fs::create_dir_all(dir).is_err() {
        return;
    }
    let Ok(data) = serde_json::to_vec(index) else { return };
    let tmp = path.with_extension("json.neu");
    if std::fs::write(&tmp, data).is_ok() && std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

/// Schlüssel einer Datei im Index: Pfad ab dem Projekt mit «/»
fn key_of(home: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(home).ok()?;
    Some(rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"))
}

/// Versteckte Dateien und Ordner, Sperrdateien von Office (`~$…`) und Thumbs.db zählen nicht
fn ignored(key: &str) -> bool {
    key.split('/').any(|c| c.starts_with('.') || c.starts_with("~$")) || key.to_lowercase().ends_with("/thumbs.db") || key.to_lowercase().ends_with("/desktop.ini")
}

fn new_entry(path: &Path, size: u64, mtime: u64, too_many: bool) -> Entry {
    let mut entry = Entry { size, mtime, ..Default::default() };
    if too_many {
        entry.hinweis = Some(format!("Mehr als {MAX_FILES} Unterlagen: diese wird nicht durchsucht."));
    } else if IMAGE_EXT.contains(&extension(path).as_str()) {
        entry.hinweis = Some("Bild: mit view_file ansehen.".into());
    } else if size > MAX_BYTES {
        entry.hinweis = Some("Grösser als 50 MB: wird nicht durchsucht.".into());
    } else {
        match extract_parts(path) {
            Ok(parts) => {
                let empty = parts.iter().filter(|p| p.text.chars().count() < SCAN_CHARS).count();
                if parts.is_empty() || empty == parts.len() {
                    entry.hinweis = Some(if extension(path) == "pdf" { "Ohne Text, wohl ein Scan: Seiten mit view_file ansehen.".into() } else { "Ohne Text.".into() });
                } else if empty > 0 && extension(path) == "pdf" {
                    entry.hinweis = Some(format!("{empty} Seiten ohne Text (Bilder oder Scans): mit view_file ansehen."));
                }
                entry.parts = parts;
            }
            Err(e) => entry.hinweis = Some(format!("Nicht lesbar: {e}")),
        }
    }
    entry
}

impl Sources {
    pub fn new() -> Self {
        Self::default()
    }

    /// Den Index auf den Stand des Ordners bringen und damit arbeiten
    fn with_index<T>(&self, files: &FileApi, f: impl FnOnce(&Index) -> T) -> T {
        let home = files.home().to_path_buf();
        let mut cache = self.cache.lock();
        let mut index = match cache.take() {
            Some((h, i)) if h == home => i,
            _ => load(&home),
        };
        let mut changed = false;
        let mut seen: HashSet<String> = HashSet::new();
        for path in files.files_under(&home.join(FOLDER)) {
            let Some(key) = key_of(&home, &path) else { continue };
            if ignored(&key) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else { continue };
            let size = meta.len();
            let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_millis() as u64).unwrap_or(0);
            let too_many = seen.len() >= MAX_FILES;
            seen.insert(key.clone());
            if index.files.get(&key).is_some_and(|e| e.size == size && e.mtime == mtime && e.hinweis.as_deref().is_none_or(|h| !h.starts_with("Mehr als") || too_many)) {
                continue;
            }
            index.files.insert(key, new_entry(&path, size, mtime, too_many));
            changed = true;
        }
        let before = index.files.len();
        index.files.retain(|k, _| seen.contains(k));
        if changed || index.files.len() != before {
            save(&home, &index);
        }
        let result = f(&index);
        *cache = Some((home, index));
        result
    }

    /// Die besten Treffer für eine Suche, höchstens einer pro Teil
    pub fn search(&self, files: &FileApi, query: &str, limit: usize) -> Value {
        let terms: Vec<String> = {
            let mut seen = HashSet::new();
            tokens(query).into_iter().filter(|t| seen.insert(t.clone())).collect()
        };
        self.with_index(files, |index| {
            let count = index.files.values().filter(|e| !e.parts.is_empty()).count();
            let hinweise = notes(index);
            if terms.is_empty() || count == 0 {
                return json!({ "query": query, "dateien": count, "results": [], "hinweise": hinweise });
            }
            struct Chunk<'a> {
                file: &'a str,
                part: usize,
                piece: &'a str,
                tf: HashMap<String, usize>,
                len: usize,
                label: HashSet<String>,
            }
            let mut chunks: Vec<Chunk> = Vec::new();
            for (file, entry) in &index.files {
                let name = file.rsplit('/').next().unwrap_or(file);
                for (i, part) in entry.parts.iter().enumerate() {
                    if part.text.trim().is_empty() {
                        continue;
                    }
                    let label: HashSet<String> = tokens(&format!("{name} {}", part.ort)).into_iter().collect();
                    for piece in pieces(&part.text) {
                        let words = tokens(piece);
                        let mut tf = HashMap::new();
                        for w in &words {
                            if terms.contains(w) {
                                *tf.entry(w.clone()).or_insert(0) += 1;
                            }
                        }
                        chunks.push(Chunk { file, part: i, piece, tf, len: words.len().max(1), label: label.clone() });
                    }
                }
            }
            let n = chunks.len() as f64;
            let avg = chunks.iter().map(|c| c.len).sum::<usize>() as f64 / n.max(1.0);
            let idf: HashMap<&String, f64> = terms
                .iter()
                .map(|t| {
                    let df = chunks.iter().filter(|c| c.tf.contains_key(t)).count() as f64;
                    (t, (1.0 + (n - df + 0.5) / (df + 0.5)).ln())
                })
                .collect();
            let (k1, b) = (1.2, 0.75);
            let mut best: HashMap<(&str, usize), (f64, &str)> = HashMap::new();
            for c in &chunks {
                let mut score = 0.0;
                for t in &terms {
                    let w = idf[t];
                    if let Some(&tf) = c.tf.get(t) {
                        let tf = tf as f64;
                        score += w * tf * (k1 + 1.0) / (tf + k1 * (1.0 - b + b * c.len as f64 / avg));
                    }
                    if c.label.contains(t) {
                        score += w * 0.5;
                    }
                }
                if c.tf.is_empty() || score <= 0.0 {
                    continue;
                }
                let slot = best.entry((c.file, c.part)).or_insert((0.0, ""));
                if score > slot.0 {
                    *slot = (score, c.piece);
                }
            }
            let mut ranked: Vec<_> = best.into_iter().collect();
            ranked.sort_by(|a, b| b.1 .0.total_cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
            let set: HashSet<String> = terms.iter().cloned().collect();
            let results: Vec<Value> = ranked
                .into_iter()
                .take(limit)
                .map(|((file, i), (score, piece))| {
                    let part = &index.files[file].parts[i];
                    json!({ "datei": file, "ort": part.ort, "seite": part.seite, "text": snippet(piece, &set), "score": (score * 100.0).round() / 100.0 })
                })
                .collect();
            json!({ "query": query, "dateien": count, "results": results, "hinweise": hinweise })
        })
    }

    /// Einen ganzen Teil lesen: nach Seite (PDF, Folie) oder Ort; ohne Angabe den ersten
    pub fn read(&self, files: &FileApi, path: &str, seite: Option<u32>, ort: Option<&str>) -> Result<Value, ApiError> {
        let wanted = resolve_key(files, path)?;
        self.with_index(files, |index| {
            let (key, entry) = index
                .files
                .get_key_value(&wanted)
                .or_else(|| index.files.iter().find(|(k, _)| k.to_lowercase() == wanted.to_lowercase()))
                .ok_or_else(|| ApiError::new(404, format!("Keine Unterlage «{path}» im Ordner {FOLDER}.")))?;
            let orts: Vec<&str> = entry.parts.iter().map(|p| p.ort.as_str()).take(200).collect();
            if entry.parts.is_empty() {
                return Ok(json!({ "datei": key, "teile": orts, "hinweis": entry.hinweis, "text": "" }));
            }
            let ort = ort.map(str::trim).filter(|o| !o.is_empty());
            let part = match (seite, ort) {
                (Some(n), _) => entry.parts.iter().find(|p| p.seite == Some(n)),
                (None, Some(o)) => {
                    let o = o.to_lowercase();
                    entry.parts.iter().find(|p| p.ort.to_lowercase() == o).or_else(|| entry.parts.iter().find(|p| p.ort.to_lowercase().contains(&o)))
                }
                (None, None) => entry.parts.first(),
            };
            let Some(part) = part else {
                return Err(ApiError {
                    status: 404,
                    detail: json!(format!("«{}» hat keinen solchen Teil. Vorhanden: {}", key, orts.iter().take(50).copied().collect::<Vec<_>>().join(", "))),
                });
            };
            let (text, cut) = clip(&part.text, READ_MAX);
            let text = if text.chars().count() < SCAN_CHARS && part.seite.is_some() {
                "(Seite ohne Text, wohl ein Bild oder Scan: mit view_file ansehen.)".to_string()
            } else {
                text
            };
            Ok(json!({ "datei": key, "ort": part.ort, "seite": part.seite, "text": text, "gekuerzt": cut, "teile": orts, "hinweis": entry.hinweis }))
        })
    }

    /// Alle Unterlagen mit Anzahl Teilen und Hinweis
    pub fn list(&self, files: &FileApi) -> Value {
        let exists = files.home().join(FOLDER).is_dir();
        self.with_index(files, |index| {
            let list: Vec<Value> = index
                .files
                .iter()
                .map(|(k, e)| json!({ "datei": k, "teile": e.parts.len(), "seiten": e.parts.iter().filter_map(|p| p.seite).max(), "hinweis": e.hinweis }))
                .collect();
            json!({ "ordner": FOLDER, "vorhanden": exists, "dateien": list })
        })
    }
}

fn notes(index: &Index) -> Vec<Value> {
    index.files.iter().filter_map(|(k, e)| e.hinweis.as_ref().map(|h| json!({ "datei": k, "hinweis": h }))).take(20).collect()
}

/// Pfad einer Unterlage zum Schlüssel im Index: «Skript.pdf», «Quellen\Skript.pdf» und «Quellen/Skript.pdf» gehen,
/// nur innerhalb des Ordners «Quellen» und der Ordnersperre
fn resolve_key(files: &FileApi, path: &str) -> Result<String, ApiError> {
    let mut p = path.trim().replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    let prefix = format!("{}/", FOLDER.to_lowercase());
    if !p.to_lowercase().starts_with(&prefix) && !Path::new(&p).is_absolute() {
        p = format!("{FOLDER}/{p}");
    }
    let target = files.resolve_read(&p)?;
    let key = key_of(files.home(), &target).ok_or_else(|| ApiError::new(403, format!("Nur Dateien im Ordner {FOLDER} sind Quellen.")))?;
    if !key.to_lowercase().starts_with(&prefix) {
        return Err(ApiError::new(403, format!("Nur Dateien im Ordner {FOLDER} sind Quellen.")));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, FileApi) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(FOLDER)).unwrap();
        let api = FileApi::new(dir.path());
        (dir, api)
    }

    fn write(dir: &tempfile::TempDir, name: &str, text: &str) {
        let p = dir.path().join(FOLDER).join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn woerter() {
        assert_eq!(tokens("Die Photosynthesen der Pflanzen"), ["photosynthes", "pflanz"]);
        assert_eq!(tokens("Photosynthese"), ["photosynthes"]);
        assert_eq!(tokens("Grösse Übung Straße"), ["groess", "uebung", "strass"]);
        assert_eq!(tokens("Bedingungen Bedingung"), ["beding", "beding"]);
        assert_eq!(tokens("a, b und 42"), ["42"]);
    }

    #[test]
    fn markdown_und_text_in_teilen() {
        let parts = markdown_sections("Einleitung\n# Zellen\nZellen haben einen Kern.\n```\n# kein Titel\n```\n## Licht\nLicht ist Energie.");
        let orts: Vec<&str> = parts.iter().map(|p| p.ort.as_str()).collect();
        assert_eq!(orts, ["Anfang", "Abschnitt «Zellen»", "Abschnitt «Licht»"]);
        assert!(parts[1].text.contains("# kein Titel"));
        let long = (0..400).map(|i| format!("Zeile {i} mit etwas Text")).collect::<Vec<_>>().join("\n");
        let b = blocks(&long);
        assert!(b.len() >= 3, "{}", b.len());
        assert!(b.iter().all(|p| p.text.chars().count() <= BLOCK));
        assert_eq!(b[1].ort, "Teil 2");
    }

    #[test]
    fn trennzeilen() {
        let s = split_marked("--- Slide 1 ---\nTitel\n--- Slide 2 ---\n\n--- Slide 3 ---\nEnde");
        assert_eq!(s, [("Slide 1".to_string(), "Titel".to_string()), ("Slide 2".to_string(), String::new()), ("Slide 3".to_string(), "Ende".to_string())]);
    }

    #[test]
    fn stuecke_mit_ueberlappung() {
        let text = "wort ".repeat(1000);
        let p = pieces(&text);
        assert!(p.len() >= 3);
        assert!(p.iter().all(|x| x.chars().count() <= CHUNK));
        assert_eq!(pieces("kurz"), ["kurz"]);
        let umlaut = "Grösse ".repeat(600);
        assert!(pieces(&umlaut).len() > 1);
    }

    #[test]
    fn suchen_rangfolge_und_ausschnitt() {
        let (dir, api) = project();
        write(&dir, "biologie.md", "# Zellen\nZellen haben einen Kern und eine Membran.\n# Photosynthese\nDie Photosynthese wandelt Licht in Zucker um. Photosynthese braucht Chlorophyll.");
        write(&dir, "chemie.txt", "Säuren und Basen. Licht spielt hier keine Rolle.");
        let s = Sources::new();
        let r = s.search(&api, "Wie funktioniert die Photosynthesen?", 5);
        let results = r["results"].as_array().unwrap();
        assert_eq!(results.len(), 1, "{r}");
        assert_eq!(results[0]["datei"], "Quellen/biologie.md");
        assert_eq!(results[0]["ort"], "Abschnitt «Photosynthese»");
        assert!(results[0]["text"].as_str().unwrap().contains("Chlorophyll"));
        let r = s.search(&api, "Licht", 5);
        let files: Vec<&str> = r["results"].as_array().unwrap().iter().map(|x| x["datei"].as_str().unwrap()).collect();
        assert_eq!(files.len(), 2);
        assert_eq!(r["dateien"], 2);
        assert_eq!(s.search(&api, "Fotosynthese", 5)["results"].as_array().unwrap().len(), 0);
        assert_eq!(s.search(&api, "", 5)["results"].as_array().unwrap().len(), 0);
        // Umlaut in der Suche und im Text
        assert_eq!(s.search(&api, "saeuren", 5)["results"][0]["datei"], "Quellen/chemie.txt");
    }

    #[test]
    fn index_liest_nur_geaendertes_und_vergisst_geloeschtes() {
        let (dir, api) = project();
        write(&dir, "a.txt", "Erste Fassung Apfel");
        write(&dir, "b.txt", "Birne");
        write(&dir, ".versteckt.txt", "Apfel");
        write(&dir, "~$offen.docx", "x");
        let s = Sources::new();
        assert_eq!(s.list(&api)["dateien"].as_array().unwrap().len(), 2);
        assert!(dir.path().join(INDEX).is_file());
        // Index bleibt nach einem Neustart (neues Sources) gültig
        let s2 = Sources::new();
        assert_eq!(s2.search(&api, "Apfel", 5)["results"].as_array().unwrap().len(), 1);
        // Änderung wird gelesen (andere Grösse), gelöschte Datei fällt weg
        write(&dir, "a.txt", "Zweite Fassung Banane und mehr");
        std::fs::remove_file(dir.path().join(FOLDER).join("b.txt")).unwrap();
        assert_eq!(s2.search(&api, "Apfel", 5)["results"].as_array().unwrap().len(), 0);
        assert_eq!(s2.search(&api, "Banane", 5)["results"].as_array().unwrap().len(), 1);
        assert_eq!(s2.list(&api)["dateien"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn hinweise_fuer_bilder_und_unlesbares() {
        let (dir, api) = project();
        write(&dir, "foto.png", "kein echtes Bild");
        write(&dir, "kaputt.pdf", "kein PDF");
        write(&dir, "unbekannt.xyz", "x");
        let s = Sources::new();
        let l = s.list(&api);
        let hint = |name: &str| l["dateien"].as_array().unwrap().iter().find(|d| d["datei"] == format!("Quellen/{name}")).unwrap()["hinweis"].as_str().unwrap().to_string();
        assert!(hint("foto.png").starts_with("Bild"));
        assert!(hint("kaputt.pdf").starts_with("Nicht lesbar"));
        assert!(hint("unbekannt.xyz").starts_with("Nicht lesbar"));
        assert_eq!(l["vorhanden"], true);
    }

    #[test]
    fn lesen_nach_ort_und_sperre() {
        let (dir, api) = project();
        write(&dir, "skript.md", "# Zellen\nZellen haben einen Kern.\n# Licht\nLicht ist Energie.");
        let s = Sources::new();
        let r = s.read(&api, "skript.md", None, Some("licht")).unwrap();
        assert_eq!(r["ort"], "Abschnitt «Licht»");
        assert!(r["text"].as_str().unwrap().contains("Energie"));
        assert_eq!(s.read(&api, "Quellen\\skript.md", None, None).unwrap()["ort"], "Abschnitt «Zellen»");
        assert_eq!(s.read(&api, "quellen/SKRIPT.md", None, None).unwrap()["datei"], "Quellen/skript.md");
        assert_eq!(s.read(&api, "skript.md", None, Some("Mond")).unwrap_err().status, 404);
        assert_eq!(s.read(&api, "fehlt.md", None, None).unwrap_err().status, 404);
        std::fs::write(dir.path().join("geheim.txt"), "x").unwrap();
        assert_eq!(s.read(&api, "Quellen/../geheim.txt", None, None).unwrap_err().status, 403);
        assert!(s.read(&api, "../../etc/passwd", None, None).is_err());
    }

    #[test]
    fn ohne_ordner_leer() {
        let dir = tempfile::tempdir().unwrap();
        let api = FileApi::new(dir.path());
        let s = Sources::new();
        assert_eq!(s.list(&api)["vorhanden"], false);
        assert_eq!(s.search(&api, "x", 5)["dateien"], 0);
        assert!(!dir.path().join(INDEX).exists(), "ohne Quellen kein Index");
    }
}
