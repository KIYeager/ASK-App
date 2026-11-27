//! Frühere Chats eines Ordners für das Modell (ENTSCHEIDE E28), nur wenn die Person den Schalter
//! «Chats» einschaltet: eine Liste im Systemprompt und die Werkzeuge chat_search und chat_read.
//! Das Modell bekommt Eingaben und Antworten als Text, Werkzeugaufrufe nur als kurze Zeile.
//!
//! Öffentliche Schnittstelle (Port von src/main/history.js):
//! - `history_prompt(folder, current) -> String`
//! - `search(folder, query, current) -> Result<Vec<Hit>>`
//! - `read(folder, id, ab) -> Result<ReadResult>`
//! - `chat_text(&Value) -> String`, `date(ms) -> String`
//!
//! Die Chats sind untypisiertes JSON (`.ask/chats/<id>.json`, geschrieben von der Oberfläche).

use crate::memory::is_js_ws;
use crate::util::{char_len, js_string, num_of, str_of, take_chars, truthy};
use anyhow::{bail, Result};
use chrono::{Local, TimeZone};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const LIST_MAX: usize = 15;
const READ_CHARS: usize = 15000;
const RESULTS: usize = 8;

/// Datum und Uhrzeit wie `dd.mm.yyyy hh:mm` (Ortszeit)
pub fn date(ms: f64) -> String {
    match Local.timestamp_millis_opt(ms as i64) {
        chrono::LocalResult::Single(d) | chrono::LocalResult::Ambiguous(d, _) => d.format("%d.%m.%Y %H:%M").to_string(),
        chrono::LocalResult::None => String::new(),
    }
}

/// Zwischenspeicher der gelesenen Chats: Datei → (Änderungszeit, Grösse der Datei, Inhalt). Begrenzt nach Anzahl und
/// nach Grösse: Chats mit Spielen oder langen Ausgaben sind mehrere MB gross, und als `Value` belegen sie ein Vielfaches.
type Cache = Mutex<HashMap<PathBuf, (std::time::SystemTime, u64, Value)>>;
const CACHE_MAX_FILES: usize = 500;
const CACHE_MAX_BYTES: u64 = 16 * 1024 * 1024;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn load_chats(folder: &Path) -> Vec<Value> {
    let dir = folder.join(".ask").join("chats");
    let Ok(rd) = std::fs::read_dir(&dir) else { return Vec::new() };
    let names: Vec<String> = rd.filter_map(Result::ok).map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| n.ends_with(".json")).collect();
    // Gelöschte Chats dieses Ordners nicht weiter im Speicher halten
    {
        let present: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
        cache().lock().retain(|file, _| file.parent() != Some(dir.as_path()) || file.file_name().and_then(|n| n.to_str()).is_some_and(|n| present.contains(n)));
    }
    // Bei eingeschaltetem Schalter «Chats» liest jeder Arbeitsschritt die Liste: unveränderte Dateien nicht neu lesen
    let mut chats: Vec<Value> = names
        .iter()
        .filter_map(|n| {
            let file = dir.join(n);
            let mtime = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
            if let Some((t, _, chat)) = cache().lock().get(&file) {
                if *t == mtime {
                    return Some(chat.clone());
                }
            }
            let bytes = std::fs::read(&file).ok()?;
            let size = bytes.len() as u64;
            let chat: Value = serde_json::from_str(&String::from_utf8_lossy(&bytes)).ok()?;
            let mut c = cache().lock();
            c.remove(&file);
            let full = |c: &HashMap<PathBuf, (std::time::SystemTime, u64, Value)>| c.len() >= CACHE_MAX_FILES || c.values().map(|v| v.1).sum::<u64>() + size > CACHE_MAX_BYTES;
            // Voll: zuerst die Chats anderer Ordner hinaus. Reicht das nicht, bleibt der Speicher, wie er ist, und
            // dieser Chat wird jedes Mal neu gelesen. Alles zu leeren hiesse, dass danach jeder Arbeitsschritt wieder
            // sämtliche Chats liest, weil die Liste immer in derselben Reihenfolge durchlaufen wird.
            if full(&c) {
                c.retain(|f, _| f.parent() == Some(dir.as_path()));
            }
            // Sehr grosse Chats gar nicht behalten
            if !full(&c) && size <= CACHE_MAX_BYTES / 4 {
                c.insert(file, (mtime, size, chat.clone()));
            }
            Some(chat)
        })
        .filter(|c| c.get("id").is_some_and(truthy) && c.get("messages").is_some_and(Value::is_array))
        .collect();
    chats.sort_by(|a, b| num_of(b, "updated").total_cmp(&num_of(a, "updated")));
    chats
}

fn messages(chat: &Value) -> &[Value] {
    chat.get("messages").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// Verlauf als Text: Eingaben, Antworten und Werkzeugaufrufe als Kurzzeile
pub fn chat_text(chat: &Value) -> String {
    let mut lines: Vec<String> = Vec::new();
    for m in messages(chat) {
        match m.get("role").and_then(Value::as_str) {
            Some("user") => {
                if str_of(m, "note") == "view" || m.get("auto").is_some_and(truthy) {
                    continue;
                }
                let files = m.get("attachments").and_then(Value::as_array).map(|a| a.iter().map(|x| str_of(x, "name")).collect::<Vec<_>>().join(", ")).unwrap_or_default();
                let content = m.get("content").filter(|c| truthy(c)).map(js_string).unwrap_or_default();
                let tail = if files.is_empty() { String::new() } else { format!(" [Anhänge: {files}]") };
                lines.push(format!("Person: {content}{tail}"));
            }
            Some("assistant") => {
                for c in m.get("tool_calls").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
                    let func = c.get("function").cloned().unwrap_or(Value::Null);
                    let raw = str_of(&func, "arguments");
                    let args: Value = serde_json::from_str(if raw.is_empty() { "{}" } else { &raw }).unwrap_or(Value::Null); // roh
                    let detail = ["path", "command", "anfrage", "url", "nachricht", "auftrag"]
                        .iter()
                        .filter_map(|k| args.get(*k))
                        .find(|v| truthy(v))
                        .map(js_string)
                        .unwrap_or_default();
                    let name = str_of(&func, "name");
                    if detail.is_empty() {
                        lines.push(format!("  [{name}]"));
                    } else {
                        let flat = collapse(&detail);
                        lines.push(format!("  [{name} {}]", take_chars(&flat, 120)));
                    }
                }
                if let Some(Value::String(content)) = m.get("content") {
                    let t = content.trim_matches(is_js_ws);
                    if !t.is_empty() {
                        lines.push(format!("Modell: {t}"));
                    }
                }
            }
            _ => {}
        }
    }
    lines.join("\n")
}

/// `s.replace(/\s+/g, ' ')` (ohne trim)
fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_ws = false;
    for c in s.chars() {
        if is_js_ws(c) {
            if !in_ws {
                out.push(' ');
            }
            in_ws = true;
        } else {
            in_ws = false;
            out.push(c);
        }
    }
    out
}

fn title_of(c: &Value) -> String {
    let t = c.get("title").filter(|t| truthy(t)).map(js_string).unwrap_or_default();
    if t.is_empty() { "(ohne Titel)".to_string() } else { t }
}

fn has_user_message(c: &Value) -> bool {
    messages(c).iter().any(|m| str_of(m, "role") == "user")
}

/// Liste für den Systemprompt (ohne den laufenden Chat)
pub fn history_prompt(folder: &Path, current: &str) -> String {
    let chats: Vec<Value> = load_chats(folder).into_iter().filter(|c| str_of(c, "id") != current && has_user_message(c)).collect();
    if chats.is_empty() {
        return "\n\n# Frühere Chats in diesem Ordner\nKeine.".to_string();
    }
    let list: Vec<String> = chats
        .iter()
        .take(LIST_MAX)
        .map(|c| format!("- {} «{}» (id {})", date(num_of(c, "updated")), take_chars(&title_of(c), 80), str_of(c, "id")))
        .collect();
    let more = if chats.len() > LIST_MAX { format!(", {LIST_MAX} von {}", chats.len()) } else { String::new() };
    format!(
        "\n\n# Frühere Chats in diesem Ordner (neueste zuerst{more})\n{}\n\
Bezieht sich die Person auf frühere Arbeit («wie letztes Mal», «der Bericht von gestern»), suche mit chat_search oder lies den Chat mit chat_read.",
        list.join("\n")
    )
}

/// Treffer der Suche
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub updated: f64,
    pub score: usize,
    pub snippets: Vec<String>,
}

pub fn search(folder: &Path, query: &str, current: &str) -> Result<Vec<Hit>> {
    let words: Vec<String> = query.to_lowercase().split(|c: char| is_js_ws(c)).filter(|w| char_len(w) > 1).map(str::to_string).collect();
    if words.is_empty() {
        bail!("Leere Suchanfrage.");
    }
    let mut hits: Vec<Hit> = Vec::new();
    for c in load_chats(folder) {
        if str_of(&c, "id") == current {
            continue;
        }
        let text = chat_text(&c);
        let title = c.get("title").filter(|t| truthy(t)).map(js_string).unwrap_or_default();
        let lower = format!("{}\n{}", title.to_lowercase(), text.to_lowercase());
        let score: usize = words.iter().map(|w| lower.matches(w.as_str()).count()).sum();
        if score == 0 || !words.iter().all(|w| lower.contains(w.as_str())) {
            continue;
        }
        let snippets: Vec<String> = text
            .split('\n')
            .filter(|l| {
                let l = l.to_lowercase();
                words.iter().any(|w| l.contains(w.as_str()))
            })
            .take(3)
            .map(|l| if char_len(l) > 220 { format!("{} …", take_chars(l, 220)) } else { l.to_string() })
            .collect();
        hits.push(Hit { id: str_of(&c, "id"), title: title_of(&c), updated: num_of(&c, "updated"), score, snippets });
    }
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(b.updated.total_cmp(&a.updated)));
    hits.truncate(RESULTS);
    Ok(hits)
}

/// Ausschnitt eines Chats (`chat_read`)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ReadResult {
    pub title: String,
    pub updated: f64,
    pub text: String,
    pub from: usize,
    pub to: usize,
    pub total: usize,
}

/// Liest einen Chat ab Zeichen `ab` (höchstens 15000 Zeichen)
pub fn read(folder: &Path, id: &str, ab: i64) -> Result<ReadResult> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        bail!("Ungültige Chat-id.");
    }
    let Some(chat) = load_chats(folder).into_iter().find(|c| str_of(c, "id") == id) else {
        bail!("Kein Chat mit id {id} in diesem Ordner.");
    };
    let text = chat_text(&chat);
    let total = char_len(&text);
    let from = (ab.max(0) as usize).min(total);
    let slice: String = text.chars().skip(from).take(READ_CHARS).collect();
    Ok(ReadResult { title: title_of(&chat), updated: num_of(&chat, "updated"), text: slice, from, to: total.min(from + READ_CHARS), total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn folder_with_chats() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(".ask").join("chats");
        std::fs::create_dir_all(&dir).unwrap();
        let call = |name: &str, args: Value| json!({ "id": "c", "type": "function", "function": { "name": name, "arguments": args.to_string() } });
        let chats = [
            json!({ "id": "alt", "title": "Bericht Proxmox", "updated": 1000, "messages": [
                { "role": "user", "content": "Schreib einen Bericht über Proxmox Backups", "attachments": [{ "name": "notizen.docx" }] },
                { "role": "assistant", "content": "", "tool_calls": [call("write_file", json!({ "path": "bericht.docx" }))] },
                { "role": "tool", "content": "Geschrieben" },
                { "role": "user", "auto": true, "note": "view", "content": "[Ansicht] Bilder" },
                { "role": "assistant", "content": "Der Bericht liegt in bericht.docx. Backups laufen täglich." }] }),
            json!({ "id": "neu", "title": "Spiel", "updated": 3000, "messages": [{ "role": "user", "content": "Mach ein Spiel" }, { "role": "assistant", "content": "Fertig." }] }),
            json!({ "id": "jetzt", "title": "Aktuell", "updated": 5000, "messages": [{ "role": "user", "content": "Proxmox Backups nochmals" }] }),
        ];
        for c in chats {
            std::fs::write(dir.join(format!("{}.json", c["id"].as_str().unwrap())), c.to_string()).unwrap();
        }
        root
    }

    #[test]
    fn fruehere_chats_liste_ohne_den_laufenden_chat_text_ohne_ansichten() {
        let f = folder_with_chats();
        let prompt = history_prompt(f.path(), "jetzt");
        assert!(prompt.contains("# Frühere Chats in diesem Ordner"));
        assert!(prompt.find("«Spiel» (id neu)").unwrap() < prompt.find("«Bericht Proxmox» (id alt)").unwrap(), "neueste zuerst");
        assert!(!prompt.contains("Aktuell"));
        let chat: Value = serde_json::from_str(&std::fs::read_to_string(f.path().join(".ask").join("chats").join("alt.json")).unwrap()).unwrap();
        assert_eq!(
            chat_text(&chat),
            "Person: Schreib einen Bericht über Proxmox Backups [Anhänge: notizen.docx]\n  [write_file bericht.docx]\nModell: Der Bericht liegt in bericht.docx. Backups laufen täglich."
        );
        let leer = tempfile::tempdir().unwrap();
        assert!(history_prompt(leer.path(), "x").contains("Keine."));
    }

    #[test]
    fn suchen_und_lesen() {
        let f = folder_with_chats();
        let hits = search(f.path(), "proxmox backups", "jetzt").unwrap();
        assert_eq!(hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(), ["alt"], "laufender Chat nicht, alle Wörter nötig");
        assert!(hits[0].snippets[0].contains("Proxmox Backups"));
        let r = read(f.path(), "alt", 0).unwrap();
        assert_eq!(r.title, "Bericht Proxmox");
        assert!(r.text.contains("Backups laufen täglich"));
        assert!(read(f.path(), "../x", 0).unwrap_err().to_string().contains("Ungültige"));
        assert!(read(f.path(), "fehlt", 0).unwrap_err().to_string().contains("Kein Chat"));
        assert!(search(f.path(), "a", "").unwrap_err().to_string().contains("Leere"));
    }
}
