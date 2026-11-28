//! ASK Docs (ENTSCHEIDE E81): lebende Dokumente wie Claude Docs. ASK schreibt ein Doc Abschnitt für Abschnitt, die
//! Person sieht im Reiter «Docs» rechts zu, bearbeitet es selbst wie in Word, kommentiert Stellen für ASK und speichert
//! es als Word, PDF oder Markdown.
//!
//! Ein Doc ist eine Markdown-Datei im Ordner `docs/` des Projekts (in Chats ohne Projekt im Ordner dieser Chats, E78).
//! So gehört es zum Projekt, steht im Explorer, kommt in die Sicherung für «Rückgängig» und mit auf GitHub. Die
//! Kommentare liegen dagegen in den Daten der App zum Projekt (`paths::project_data_dir`): Sie sind Anweisungen der
//! Person an ASK, und was im Ordner liegt, kann das Modell selbst schreiben (E33).
//!
//! Die Werkzeuge laufen im Hauptprozess, nicht im Terminal: Sie schreiben nur Markdown-Dateien unter `docs/` und prüfen
//! jeden Pfad mit `safe::safe_join` (keine Verknüpfung aus dem Ordner hinaus). Das ist nicht mehr, als das Terminal im
//! Ordner ohnehin darf.
//!
//! Öffentliche Schnittstelle:
//! - `DIR`, `doc_rel(eingabe) -> Result<String>`, `slug(titel)`, `title_of(inhalt, rel)`, `hash(inhalt)`
//! - `list(folder) -> Vec<Value>`, `read(folder, rel) -> Result<Value>`, `save(folder, rel, inhalt, basis) -> Result<Value>`,
//!   `create(folder, titel, inhalt) -> Result<String>`, `remove(folder, rel)`
//! - Kommentare: `threads`, `add_comment`, `reply`, `set_done`, `delete_comment`, `open_count`
//! - Werkzeuge: `DOC_TOOLS`, `doc_tools() -> Vec<Tool>`, `is_doc_tool`, `describe(name, args)`, `run(name, args, ctx)`
//! - `system_prompt(folder)`, `set_open(folder, rel)` (welches Doc der Reiter gerade zeigt)
//! - `App::handle_docs(channel, args)`: die Kanäle `docs:*` der Oberfläche

use crate::app::App;
use crate::safe::{safe_join, safe_rel};
use crate::tools::{Tool, ToolCtx, ToolResult};
use crate::util::{js_string, now_ms, path_key, str_of, take_chars, truthy};
use anyhow::{bail, Result};
use parking_lot::Mutex;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Ordner der Docs im Projekt
pub const DIR: &str = "docs";
/// Grösste Datei, die als Doc gilt (Text)
pub const MAX_BYTES: usize = 2_000_000;
/// Höchstens so viele Docs in der Liste
const MAX_LIST: usize = 200;
/// Höchstens so viele Docs im Systemprompt
const PROMPT_LIST: usize = 15;
/// Kommentare: Länge und Anzahl
const MAX_COMMENT: usize = 4000;
const MAX_QUOTE: usize = 500;
const MAX_THREADS: usize = 200;
const MAX_ENTRIES: usize = 50;

/// Werkzeuge der Docs
pub const DOC_TOOLS: [&str; 5] = ["doc_liste", "doc_lesen", "doc_erstellen", "doc_bearbeiten", "doc_kommentieren"];

pub fn is_doc_tool(name: &str) -> bool {
    DOC_TOOLS.contains(&name)
}

// ---------- Pfade und Namen ----------

/// Pfad eines Docs relativ zum Ordner, immer `docs/….md` mit «/». Nimmt auch «bericht», «bericht.md» oder
/// «docs\bericht.md». Fehler bei allem, was aus `docs/` hinausführen könnte.
pub fn doc_rel(input: &str) -> Result<String> {
    let mut p = input.trim().replace('\\', "/");
    while let Some(rest) = p.strip_prefix("./") {
        p = rest.to_string();
    }
    let p = p.trim_start_matches('/');
    if p.is_empty() {
        bail!("Kein Doc angegeben.");
    }
    let mut rel = if p.get(..5).is_some_and(|h| h.eq_ignore_ascii_case("docs/")) { format!("{DIR}/{}", &p[5..]) } else { format!("{DIR}/{p}") };
    if !rel.to_lowercase().ends_with(".md") {
        rel.push_str(".md");
    }
    if !safe_rel(&rel) || rel.len() <= DIR.len() + 4 {
        bail!("Ungültiger Name für ein Doc: {}", take_chars(input, 120));
    }
    Ok(rel)
}

/// Dateiname aus einem Titel: klein, Umlaute ausgeschrieben, nur a–z, 0–9 und Bindestriche, höchstens 60 Zeichen
pub fn slug(title: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut out = String::new();
    for c in title.to_lowercase().chars() {
        match c {
            'ä' => out.push_str("ae"),
            'ö' => out.push_str("oe"),
            'ü' => out.push_str("ue"),
            'ß' => out.push_str("ss"),
            c if c.is_ascii_alphanumeric() => out.push(c),
            c => {
                let base: String = c.to_string().nfd().filter(|x| x.is_ascii_alphanumeric()).collect();
                if base.is_empty() { out.push('-') } else { out.push_str(&base) }
            }
        }
    }
    let mut s = String::new();
    for part in out.split('-').filter(|p| !p.is_empty()) {
        if !s.is_empty() {
            s.push('-');
        }
        s.push_str(part);
    }
    let s: String = s.chars().take(60).collect();
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "doc".into() } else { s }
}

/// Titel eines Docs: die erste Überschrift «# …», sonst der Dateiname
pub fn title_of(content: &str, rel: &str) -> String {
    for line in content.lines().take(40) {
        let t = line.trim();
        if let Some(h) = t.strip_prefix("# ") {
            let h = h.trim().trim_matches(|c| matches!(c, '*' | '_' | '`')).trim();
            if !h.is_empty() {
                return take_chars(h, 120);
            }
        }
    }
    let name = rel.rsplit('/').next().unwrap_or(rel);
    name.strip_suffix(".md").or_else(|| name.strip_suffix(".MD")).unwrap_or(name).to_string()
}

/// Prüfsumme des Inhalts (erkennt, ob das Doc inzwischen jemand anders geändert hat)
pub fn hash(content: &str) -> String {
    hex::encode(Sha1::digest(content.as_bytes()))
}

fn modified_ms(p: &Path) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn full_path(folder: &str, rel: &str) -> Result<PathBuf> {
    if folder.is_empty() {
        bail!("Kein Projekt offen.");
    }
    safe_join(Path::new(folder), rel)
}

/// Freier Name unter docs/ für einen Titel (nie über ein vorhandenes Doc)
fn free_rel(folder: &str, title: &str) -> Result<String> {
    let base = slug(title);
    for n in 1..1000 {
        let rel = if n == 1 { format!("{DIR}/{base}.md") } else { format!("{DIR}/{base}-{n}.md") };
        if !full_path(folder, &rel)?.exists() {
            return Ok(rel);
        }
    }
    bail!("Kein freier Name für das Doc gefunden.")
}

/// Schreibt über eine Zwischendatei, damit nie ein halbes Doc auf der Platte steht
fn write_atomic(path: &Path, content: &str) -> Result<()> {
    if content.len() > MAX_BYTES {
        bail!("Das Doc ist zu gross (höchstens 2 MB).");
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("md.{}.tmp", hex::encode(rand::random::<[u8; 3]>())));
    std::fs::write(&tmp, content)?;
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

// ---------- Docs ----------

/// Alle Docs des Ordners, zuletzt geänderte zuerst: { path, title, modified, size, comments }
pub fn list(folder: &str) -> Vec<Value> {
    let Ok(dir) = full_path(folder, DIR) else { return vec![] };
    if !dir.is_dir() {
        return vec![];
    }
    let mut out: Vec<Value> = walkdir::WalkDir::new(&dir)
        .max_depth(4)
        .into_iter()
        .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'))
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file() && e.file_name().to_string_lossy().to_lowercase().ends_with(".md"))
        .filter_map(|e| {
            let rel_in = e.path().strip_prefix(&dir).ok()?.to_string_lossy().replace('\\', "/");
            let rel = format!("{DIR}/{rel_in}");
            if !safe_rel(&rel) {
                return None;
            }
            let size = e.metadata().ok()?.len();
            let head = read_head(e.path());
            Some(json!({ "path": rel, "title": title_of(&head, &rel), "modified": modified_ms(e.path()), "size": size,
                "comments": open_count(folder, &rel), "geteilt": crate::docsync::is_shared(folder, &rel) }))
        })
        .collect();
    out.sort_by(|a, b| b["modified"].as_i64().cmp(&a["modified"].as_i64()));
    out.truncate(MAX_LIST);
    out
}

fn read_head(p: &Path) -> String {
    use std::io::Read;
    let mut buf = vec![0u8; 4096];
    let n = std::fs::File::open(p).and_then(|mut f| f.read(&mut buf)).unwrap_or(0);
    String::from_utf8_lossy(&buf[..n]).to_string()
}

/// Ein Doc lesen: { path, title, content, hash, modified }
pub fn read(folder: &str, rel: &str) -> Result<Value> {
    let rel = doc_rel(rel)?;
    let p = full_path(folder, &rel)?;
    let meta = match std::fs::metadata(&p) {
        Ok(m) if m.is_file() => m,
        _ => bail!("Das Doc «{rel}» gibt es nicht."),
    };
    if meta.len() as usize > MAX_BYTES {
        bail!("Das Doc «{rel}» ist zu gross (höchstens 2 MB).");
    }
    let bytes = std::fs::read(&p)?;
    let content = String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("«{rel}» ist kein Text in UTF-8."))?;
    let content = content.strip_prefix('\u{feff}').unwrap_or(&content).replace("\r\n", "\n");
    Ok(json!({ "path": rel, "title": title_of(&content, &rel), "hash": hash(&content), "modified": modified_ms(&p), "content": content }))
}

/// Doc speichern. Mit `base` (Prüfsumme des Stands, den die Person bearbeitet hat): Hat sich das Doc inzwischen
/// geändert (ASK hat geschrieben), wird nichts überschrieben, sondern `{ ok: false, conflict: true, content, hash }`
/// zurückgegeben. Sonst `{ ok: true, hash, modified, title }`.
pub fn save(folder: &str, rel: &str, content: &str, base: Option<&str>) -> Result<Value> {
    let rel = doc_rel(rel)?;
    // Geteiltes Doc (E82): gegen den Stand `base` vergleichen und mit den Änderungen der anderen zusammenführen
    // (ohne die Sperre unten, weil das übers Netz gehen kann)
    if let Some(r) = crate::docsync::save(folder, &rel, &content.replace("\r\n", "\n"), base.filter(|b| !b.is_empty())) {
        return r;
    }
    // Prüfen und Schreiben ohne Lücke: Sonst konnten Speichern der Oberfläche und doc_bearbeiten gleichzeitig
    // die Prüfung bestehen, und der Letzte überschrieb still den anderen
    let _guard = DOCS_LOCK.lock();
    let p = full_path(folder, &rel)?;
    let content = content.replace("\r\n", "\n");
    if let Some(base) = base.filter(|b| !b.is_empty()) {
        if p.is_file() {
            let now = read(folder, &rel)?;
            let current = str_of(&now, "content");
            if hash(&current) != base && current != content {
                return Ok(json!({ "ok": false, "conflict": true, "content": current, "hash": hash(&current), "title": now["title"] }));
            }
        }
    }
    write_atomic(&p, &content)?;
    Ok(json!({ "ok": true, "path": rel, "hash": hash(&content), "modified": modified_ms(&p), "title": title_of(&content, &rel) }))
}

static DOCS_LOCK: Mutex<()> = Mutex::new(());

/// Neues Doc mit Titel; der Inhalt beginnt immer mit «# Titel». Liefert den Pfad.
pub fn create(folder: &str, title: &str, content: &str) -> Result<String> {
    let title = title.trim();
    let title = if title.is_empty() { "Neues Doc" } else { title };
    let title: String = title.lines().next().unwrap_or("").chars().take(120).collect();
    let rel = free_rel(folder, &title)?;
    let body = content.replace("\r\n", "\n");
    let body = body.trim_start();
    let text = if body.starts_with("# ") { format!("{}\n", body.trim_end()) } else if body.is_empty() { format!("# {title}\n\n") } else { format!("# {title}\n\n{}\n", body.trim_end()) };
    write_atomic(&full_path(folder, &rel)?, &text)?;
    Ok(rel)
}

/// Doc löschen, mit seinen Kommentaren
pub fn remove(folder: &str, rel: &str) -> Result<()> {
    let rel = doc_rel(rel)?;
    let p = full_path(folder, &rel)?;
    if p.is_file() {
        std::fs::remove_file(&p)?;
    }
    let _ = with_comments(folder, |all| { all.remove(&rel.to_lowercase()); Ok(()) });
    Ok(())
}

// ---------- Kommentare ----------

static COMMENTS_LOCK: Mutex<()> = Mutex::new(());

#[cfg(test)]
thread_local! {
    /// Tests: Kommentare in einem eigenen Ordner statt in den Daten der App (ohne Umgebungsvariable, die andere Tests setzen)
    static TEST_DIR: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

fn comments_file(folder: &str) -> PathBuf {
    #[cfg(test)]
    if let Some(dir) = TEST_DIR.with(|d| d.borrow().clone()) {
        return dir.join(format!("{}.json", &hash(&path_key(folder))[..12]));
    }
    crate::paths::project_data_dir(folder).join("docs-kommentare.json")
}

fn load_comments(folder: &str) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(comments_file(folder)).ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

/// Kommentare ändern: lesen, `f` anwenden, speichern (unter einer Sperre, damit nichts verloren geht)
fn with_comments<T>(folder: &str, f: impl FnOnce(&mut serde_json::Map<String, Value>) -> Result<T>) -> Result<T> {
    let _guard = COMMENTS_LOCK.lock();
    let mut all = load_comments(folder);
    // Unlesbare Datei: zur Seite legen statt mit einer leeren Liste zu überschreiben (alle Kommentare wären weg)
    let file = comments_file(folder);
    if all.is_empty() && std::fs::read_to_string(&file).is_ok_and(|t| !t.trim().is_empty() && serde_json::from_str::<Value>(&t).map_or(true, |v| !v.is_object())) {
        let aside = file.with_extension(format!("kaputt-{}.json", now_ms()));
        std::fs::rename(&file, &aside)?;
    }
    let out = f(&mut all)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&Value::Object(all))?)?;
    std::fs::rename(&tmp, &file)?;
    Ok(out)
}

fn key(rel: &str) -> Result<String> {
    Ok(doc_rel(rel)?.to_lowercase())
}

fn clean(text: &str, max: usize) -> String {
    take_chars(text.trim(), max)
}

/// Kommentare eines Docs, älteste zuerst: [{ id, zitat, erledigt, zeit, eintraege: [{ von, text, zeit }] }]
pub fn threads(folder: &str, rel: &str) -> Vec<Value> {
    let Ok(k) = key(rel) else { return vec![] };
    load_comments(folder).get(&k).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// Offene Kommentare eines Docs
pub fn open_count(folder: &str, rel: &str) -> usize {
    threads(folder, rel).iter().filter(|t| !truthy(&t["erledigt"])).count()
}

/// Neuer Kommentar der Person zu einer Stelle (`zitat`, darf leer sein: zum ganzen Doc)
pub fn add_comment(folder: &str, rel: &str, quote: &str, text: &str) -> Result<Value> {
    add_comment_with(folder, rel, quote, text, "person", "")
}

/// Farben zum Anstreichen (leer: Kommentar ohne eigene Farbe)
pub const COLORS: [&str; 4] = ["gelb", "gruen", "blau", "rot"];

/// Neuer Kommentar von der Person oder von ASK (`from`), auf Wunsch angestrichen (`color`, siehe `COLORS`).
/// Eine angestrichene Stelle darf ohne Text sein (nur angestrichen).
pub fn add_comment_with(folder: &str, rel: &str, quote: &str, text: &str, from: &str, color: &str) -> Result<Value> {
    let k = key(rel)?;
    let text = clean(text, MAX_COMMENT);
    let quote = clean(quote, MAX_QUOTE);
    let color = if COLORS.contains(&color) { color } else { "" };
    if text.is_empty() && (color.is_empty() || quote.is_empty()) {
        bail!("Der Kommentar ist leer.");
    }
    let from = if from == "ask" { "ask" } else { "person" };
    let entries = if text.is_empty() { json!([]) } else { json!([{ "von": from, "text": text, "zeit": now_ms() }]) };
    let thread = json!({ "id": format!("k{}", hex::encode(rand::random::<[u8; 3]>())), "zitat": quote, "farbe": color, "erledigt": false,
        "zeit": now_ms(), "eintraege": entries });
    with_comments(folder, |all| {
        let list = all.entry(k).or_insert_with(|| json!([]));
        let arr = list.as_array_mut().expect("Liste");
        if arr.len() >= MAX_THREADS {
            bail!("Zu viele Kommentare in diesem Doc (höchstens {MAX_THREADS}). Lösche erledigte.");
        }
        arr.push(thread.clone());
        Ok(())
    })?;
    Ok(thread)
}

fn change_thread(folder: &str, rel: &str, id: &str, f: impl FnOnce(&mut Value) -> Result<()>) -> Result<Value> {
    let k = key(rel)?;
    with_comments(folder, |all| {
        let Some(t) = all.get_mut(&k).and_then(Value::as_array_mut).and_then(|a| a.iter_mut().find(|t| t["id"] == id)) else {
            bail!("Den Kommentar {id} gibt es in diesem Doc nicht.");
        };
        f(t)?;
        Ok(t.clone())
    })
}

/// Antwort in einem Kommentar (von «person» oder «ask»); `done` setzt ihn zugleich erledigt oder offen
pub fn reply(folder: &str, rel: &str, id: &str, from: &str, text: &str, done: Option<bool>) -> Result<Value> {
    let text = clean(text, MAX_COMMENT);
    let from = if from == "ask" { "ask" } else { "person" };
    change_thread(folder, rel, id, |t| {
        if !text.is_empty() {
            let arr = t["eintraege"].as_array_mut().ok_or_else(|| anyhow::anyhow!("Kommentar beschädigt."))?;
            if arr.len() >= MAX_ENTRIES {
                bail!("Dieser Kommentar hat schon zu viele Antworten.");
            }
            arr.push(json!({ "von": from, "text": text, "zeit": now_ms() }));
        }
        if let Some(d) = done {
            t["erledigt"] = json!(d);
        }
        Ok(())
    })
}

pub fn set_done(folder: &str, rel: &str, id: &str, done: bool) -> Result<Value> {
    change_thread(folder, rel, id, |t| { t["erledigt"] = json!(done); Ok(()) })
}

pub fn delete_comment(folder: &str, rel: &str, id: &str) -> Result<()> {
    let k = key(rel)?;
    with_comments(folder, |all| {
        if let Some(a) = all.get_mut(&k).and_then(Value::as_array_mut) {
            a.retain(|t| t["id"] != id);
        }
        Ok(())
    })
}

/// Nachricht an ASK zu einem Kommentar (geht wie eine Eingabe der Person in den Chat)
pub fn comment_message(rel: &str, thread: &Value) -> String {
    let quote = str_of(thread, "zitat");
    let last = thread["eintraege"].as_array().and_then(|a| a.iter().rev().find(|e| e["von"] == "person")).map(|e| str_of(e, "text")).unwrap_or_default();
    let place = if quote.is_empty() { "zum ganzen Doc".to_string() } else { format!("zur Stelle «{}»", take_chars(&quote, 200)) };
    format!("[Kommentar im Doc {rel}, {}] {place}: {last}", str_of(thread, "id"))
}

// ---------- Welches Doc der Reiter zeigt ----------

fn open_docs() -> &'static Mutex<HashMap<String, String>> {
    static OPEN: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    OPEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Die Oberfläche zeigt im Reiter «Docs» dieses Doc (leer: keines). Steht im Systemprompt, damit «dieses Doc» klar ist.
pub fn set_open(folder: &str, rel: &str) {
    let mut m = open_docs().lock();
    match doc_rel(rel) {
        Ok(r) if !rel.trim().is_empty() => { m.insert(path_key(folder), r); }
        _ => { m.remove(&path_key(folder)); }
    }
}

fn open_doc(folder: &str) -> Option<String> {
    open_docs().lock().get(&path_key(folder)).cloned()
}

// ---------- Systemprompt ----------

const PROMPT: &str = "\n\n# ASK Docs\nDocs sind lebende Dokumente: Markdown-Dateien im Ordner docs/, die die Person rechts im Reiter «Docs» sieht, \
selbst wie in Word bearbeitet, kommentiert und als Word, PDF oder Markdown speichert. Lege mit doc_erstellen ein Doc an, wenn die Person einen \
längeren Text will, den sie behalten, weiterbearbeiten oder abgeben möchte (Bericht, Zusammenfassung, Konzept, Brief, Lernzettel, Protokoll, \
Plan), oder wenn sie ausdrücklich ein Doc verlangt. Kurze Antworten und Erklärungen bleiben im Chat.\n\
So gehst du vor: Zuerst mit doc_erstellen das Gerüst anlegen (Titel, alle Überschriften, unter jeder «…»), dann Abschnitt für Abschnitt mit \
doc_bearbeiten füllen (alt «…» unter der Überschrift, neu der Text); die Person sieht dabei zu. Vor jeder Änderung an einem bestehenden Doc liest \
du es mit doc_lesen, denn die Person kann es inzwischen selbst geändert haben, und du änderst nur, was verlangt ist. Im Chat sagst du danach in \
ein, zwei Sätzen, was im Doc steht, ohne den Text zu wiederholen. Formatierung: Markdown mit #, ##, ###, Listen, Tabellen, Zitaten, **fett**, \
*kursiv* und Links; kein HTML und keine Blöcke ask-ui. Bilder und Diagramme: Lege die Datei in docs/bilder/ an (Diagramme mit Python und \
matplotlib als PNG, Bilder mit bild_erstellen und datei «docs/bilder/name.png») und füge sie mit ![kurze Beschreibung](bilder/name.png) ein; \
der Pfad gilt ab dem Ordner docs/. Will die Person ausdrücklich eine Word-Datei nach einer Vorlage oder mit besonderem Layout, \
nimm statt eines Docs die Anleitung für Word.\n\
Kommentare: Eine Nachricht, die mit «[Kommentar im Doc» beginnt, kommt aus dem Reiter «Docs» (Doc, Kennung des Kommentars, Stelle, Text). Lies \
das Doc, ändere die Stelle mit doc_bearbeiten, wenn das verlangt ist, und antworte mit doc_kommentieren kurz im Kommentar (was du getan hast oder \
deine Antwort); setze erledigt, wenn er damit erledigt ist. Im Chat genügt dann ein Satz. Halte dich genau an die Bitte: «Erkläre …» heisst nur \
im Kommentar antworten, das Doc bleibt; «Mach eine Grafik dazu …» heisst Grafik erstellen und direkt nach der Stelle einfügen; «einfacher» \
und «Beispiel» ändern das Doc. Wünscht die Person Feedback oder Vorschläge, ändere das Doc nicht, sondern setze sie mit doc_kommentieren \
ohne id und mit zitat als Kommentare an die passenden Stellen.";

/// Abschnitt «ASK Docs» im Systemprompt: Anleitung, die Docs des Ordners und das offene Doc
pub fn system_prompt(folder: &str) -> String {
    let docs = list(folder);
    let mut out = PROMPT.to_string();
    if !docs.is_empty() {
        let shown: Vec<String> = docs.iter().take(PROMPT_LIST).map(|d| {
            let n = d["comments"].as_u64().unwrap_or(0);
            format!("{} «{}»{}{}", str_of(d, "path"), str_of(d, "title"), if n > 0 { format!(" ({n} offene Kommentare)") } else { String::new() },
                if truthy(&d["geteilt"]) { " (geteilt, andere schreiben live mit)" } else { "" })
        }).collect();
        out.push_str(&format!("\nDocs hier: {}{}.", shown.join("; "), if docs.len() > PROMPT_LIST { format!(" und {} weitere (doc_liste)", docs.len() - PROMPT_LIST) } else { String::new() }));
    }
    if let Some(open) = open_doc(folder).filter(|r| full_path(folder, r).map(|p| p.is_file()).unwrap_or(false)) {
        out.push_str(&format!("\nGerade offen im Reiter «Docs»: {open}. Meint die Person «das Doc» oder «hier», ist dieses gemeint."));
        let note = crate::docsync::prompt_note(folder, &open);
        if !note.is_empty() {
            out.push_str(&format!(" {note}"));
        }
    }
    out.push_str("\nGeteilte Docs: Andere Personen können am selben Doc mitschreiben (live über einen geteilten Ordner). Lies ein geteiltes Doc vor jeder Antwort dazu neu; doc_lesen nennt, wer zuletzt was geändert hat. Gibst du Feedback zu den Änderungen anderer, nenne sie beim Namen.");
    out
}

// ---------- Werkzeuge ----------

fn tool(name: &str, label: &str, description: &str, parameters: Value) -> Tool {
    Tool { name: name.into(), label: label.into(), description: description.into(), parameters }
}

/// Werkzeuge der Docs im Format von tools.rs
pub fn doc_tools() -> Vec<Tool> {
    vec![
        tool("doc_liste", "Docs", "Listet die Docs dieses Ordners (docs/…) mit Titel, Änderungszeit und offenen Kommentaren.",
            json!({ "type": "object", "properties": {} })),
        tool("doc_lesen", "Doc gelesen", "Liest ein Doc ganz (Markdown) mit seinen offenen Kommentaren. Vor jeder Änderung an einem bestehenden Doc aufrufen: Die Person kann es selbst geändert haben.",
            json!({ "type": "object", "properties": { "pfad": { "type": "string", "description": "Pfad des Docs, z. B. docs/bericht.md" } }, "required": ["pfad"] })),
        tool("doc_erstellen", "Doc erstellt", concat!(
            "Legt ein neues Doc unter docs/ an und öffnet es rechts im Reiter «Docs». Lege zuerst das Gerüst an (alle Überschriften, unter jeder «…») ",
            "und fülle es danach mit doc_bearbeiten Abschnitt für Abschnitt. Überschreibt nie ein vorhandenes Doc; der Name kommt aus dem Titel."),
            json!({ "type": "object", "properties": {
                "titel": { "type": "string", "description": "Titel des Docs, wird zur Überschrift # und zum Dateinamen" },
                "inhalt": { "type": "string", "description": "Markdown unter dem Titel, meist das Gerüst mit ## Überschriften" },
            }, "required": ["titel"] })),
        tool("doc_bearbeiten", "Doc geändert", concat!(
            "Ändert ein Doc. Genau eine Art: alt und neu (ersetzt die Stelle alt, die genau einmal vorkommen muss, durch neu; z. B. «…» unter einer Überschrift), ",
            "anhaengen (Text ans Ende) oder inhalt (ganzes Doc neu, nur wenn fast alles anders wird). Vorher mit doc_lesen den aktuellen Stand lesen."),
            json!({ "type": "object", "properties": {
                "pfad": { "type": "string", "description": "Pfad des Docs, z. B. docs/bericht.md" },
                "alt": { "type": "string", "description": "Stelle, die ersetzt wird, wörtlich wie im Doc" },
                "neu": { "type": "string", "description": "Neuer Text für die Stelle alt (leer: Stelle löschen)" },
                "anhaengen": { "type": "string", "description": "Markdown, das ans Ende kommt" },
                "inhalt": { "type": "string", "description": "Das ganze Doc neu (Markdown, beginnt mit # Titel)" },
            }, "required": ["pfad"] })),
        tool("doc_kommentieren", "Kommentar", concat!(
            "Antwortet in einem Kommentar der Person zu einem Doc (Kennung aus «[Kommentar im Doc …, k…]» oder doc_lesen) und setzt ihn auf Wunsch erledigt. ",
            "Ohne id legst du einen neuen Kommentar an einer Stelle an (zitat), z. B. für Feedback und Vorschläge, ohne das Doc selbst zu ändern."),
            json!({ "type": "object", "properties": {
                "pfad": { "type": "string", "description": "Pfad des Docs" },
                "id": { "type": "string", "description": "Kennung des Kommentars, z. B. k3f2a1c; leer für einen neuen Kommentar" },
                "zitat": { "type": "string", "description": "Nur neuer Kommentar: die Stelle im Doc, wörtlich (ein paar Wörter bis ein Satz); leer für das ganze Doc" },
                "antwort": { "type": "string", "description": "Kurzer Text: was du getan hast, deine Antwort oder dein Vorschlag" },
                "erledigt": { "type": "boolean", "description": "true, wenn der Kommentar damit erledigt ist" },
            }, "required": ["pfad", "antwort"] })),
    ]
}

/// Kurztext eines Aufrufs für die Oberfläche
pub fn describe(name: &str, a: &Value) -> String {
    match name {
        "doc_liste" => "Liste".into(),
        "doc_erstellen" => js_string(a.get("titel").unwrap_or(&Value::Null)),
        "doc_kommentieren" => format!("{} {}", js_string(a.get("pfad").unwrap_or(&Value::Null)), js_string(a.get("id").unwrap_or(&Value::Null))).trim().to_string(),
        _ => js_string(a.get("pfad").unwrap_or(&Value::Null)),
    }
}

fn arg(a: &Value, k: &str) -> Option<String> {
    a.get(k).filter(|v| !v.is_null()).map(js_string)
}

/// Meldet der Oberfläche, dass ein Doc geändert wurde (öffnet es im Reiter «Docs»)
fn live(ctx: &ToolCtx, rel: &str, comments: bool) {
    if let Some(f) = &ctx.on_live {
        f(json!({ "phase": "doc", "doc": rel, "comments": comments }));
    }
}

fn thread_text(t: &Value) -> String {
    let entries: Vec<String> = t["eintraege"].as_array().map(|a| a.iter().map(|e| format!("  {}: {}", if e["von"] == "ask" { "ASK" } else { "Person" }, str_of(e, "text"))).collect()).unwrap_or_default();
    let quote = str_of(t, "zitat");
    let entries = if entries.is_empty() { vec!["  (nur angestrichen)".to_string()] } else { entries };
    format!("- {}{}:\n{}", str_of(t, "id"), if quote.is_empty() { " (ganzes Doc)".to_string() } else { format!(" zu «{}»", take_chars(&quote, 200)) }, entries.join("\n"))
}

/// Führt ein Werkzeug der Docs aus; `None`, wenn es keines ist
pub async fn run(name: &str, a: &Value, ctx: &ToolCtx) -> Option<Result<ToolResult>> {
    if !is_doc_tool(name) {
        return None;
    }
    let folder = ctx.folder.clone();
    Some(run_inner(name, a, ctx, &folder))
}

fn run_inner(name: &str, a: &Value, ctx: &ToolCtx, folder: &str) -> Result<ToolResult> {
    match name {
        "doc_liste" => {
            let docs = list(folder);
            if docs.is_empty() {
                return Ok(ToolResult::new(true, "Noch keine Docs in diesem Ordner (docs/ ist leer).", "keine Docs"));
            }
            let lines: Vec<String> = docs.iter().map(|d| {
                let n = d["comments"].as_u64().unwrap_or(0);
                format!("{} «{}», geändert {}{}", str_of(d, "path"), str_of(d, "title"), crate::history::date(d["modified"].as_f64().unwrap_or(0.0)),
                    if n > 0 { format!(", {n} offene Kommentare") } else { String::new() })
            }).collect();
            Ok(ToolResult::new(true, lines.join("\n"), format!("{} Docs", docs.len())))
        }
        "doc_lesen" => {
            let rel = doc_rel(&arg(a, "pfad").unwrap_or_default())?;
            let doc = read(folder, &rel)?;
            let content = str_of(&doc, "content");
            let open: Vec<String> = threads(folder, &rel).iter().filter(|t| !truthy(&t["erledigt"])).map(thread_text).collect();
            let mut out = crate::tools::clip(&content, 60_000);
            let note = crate::docsync::prompt_note(folder, &rel);
            if !note.is_empty() {
                out = format!("[{note}]\n\n{out}");
            }
            if !open.is_empty() {
                out.push_str(&format!("\n\n--- Offene Kommentare ---\n{}", open.join("\n")));
            }
            let lines = content.lines().count();
            Ok(ToolResult::new(true, out, format!("{lines} Zeilen{}", if open.is_empty() { String::new() } else { format!(", {} Kommentare", open.len()) })))
        }
        "doc_erstellen" => {
            let title = arg(a, "titel").unwrap_or_default();
            if title.trim().is_empty() {
                bail!("Gib dem Doc einen Titel (titel).");
            }
            let rel = create(folder, &title, &arg(a, "inhalt").unwrap_or_default())?;
            live(ctx, &rel, false);
            let mut r = ToolResult::new(true, format!("Doc angelegt: {rel}. Es ist rechts im Reiter «Docs» offen. Fülle es jetzt mit doc_bearbeiten."), rel.clone());
            r.made = Some(rel);
            Ok(r)
        }
        "doc_bearbeiten" => {
            let rel = doc_rel(&arg(a, "pfad").unwrap_or_default())?;
            let doc = read(folder, &rel)?;
            let current = str_of(&doc, "content");
            let (old, new, append, whole) = (arg(a, "alt"), arg(a, "neu"), arg(a, "anhaengen"), arg(a, "inhalt"));
            let modes = [old.is_some(), append.is_some(), whole.is_some()].iter().filter(|x| **x).count();
            if modes != 1 {
                bail!("Gib genau eine Art an: alt und neu, anhaengen oder inhalt.");
            }
            let next = if let Some(old) = old.filter(|o| !o.is_empty()) {
                let old = old.replace("\r\n", "\n");
                let n = current.matches(&old).count();
                if n == 0 {
                    bail!("Die Stelle alt steht so nicht im Doc. Lies es mit doc_lesen und nimm den Text wörtlich.");
                }
                if n > 1 {
                    bail!("Die Stelle alt kommt {n}-mal vor. Nimm mehr Text dazu, damit sie eindeutig ist (z. B. mit der Überschrift davor).");
                }
                current.replacen(&old, &new.unwrap_or_default().replace("\r\n", "\n"), 1)
            } else if let Some(text) = append {
                format!("{}\n\n{}\n", current.trim_end(), text.trim_matches('\n'))
            } else if let Some(text) = whole {
                format!("{}\n", text.replace("\r\n", "\n").trim_end())
            } else {
                bail!("alt darf nicht leer sein.");
            };
            if next == current {
                return Ok(ToolResult::new(true, "Keine Änderung: Das Doc steht schon so.", "unverändert"));
            }
            // gegen den gelesenen Stand: Hat die Person inzwischen gespeichert, nichts überschreiben
            let saved = save(folder, &rel, &next, Some(&hash(&current)))?;
            if saved["conflict"] == true {
                bail!("Das Doc hat sich gerade geändert (die Person schreibt). Lies es mit doc_lesen nochmals und ändere dann.");
            }
            live(ctx, &rel, false);
            let (before, after) = (current.lines().count() as i64, next.lines().count() as i64);
            let diff = after - before;
            Ok(ToolResult::new(true, format!("{rel} geändert ({after} Zeilen)."), if diff == 0 { "geändert".to_string() } else { format!("{}{} Zeilen", if diff > 0 { "+" } else { "−" }, diff.abs()) }))
        }
        "doc_kommentieren" => {
            let rel = doc_rel(&arg(a, "pfad").unwrap_or_default())?;
            let id = arg(a, "id").unwrap_or_default();
            let text = arg(a, "antwort").unwrap_or_default();
            if text.trim().is_empty() {
                bail!("Die Antwort ist leer.");
            }
            let done = a.get("erledigt").and_then(Value::as_bool);
            if id.trim().is_empty() {
                // Neuer Kommentar von ASK (Feedback, Vorschlag) an einer Stelle, die es im Doc gibt
                let content = str_of(&read(folder, &rel)?, "content");
                let quote = arg(a, "zitat").unwrap_or_default();
                let quote = if !quote.trim().is_empty() && content.contains(quote.trim()) { quote.trim().to_string() } else { String::new() };
                let t = add_comment_with(folder, &rel, &quote, &text, "ask", "")?;
                live(ctx, &rel, true);
                return Ok(ToolResult::new(true, format!("Kommentar {} angelegt{}.", str_of(&t, "id"), if quote.is_empty() { " (zum ganzen Doc; die Stelle stand so nicht im Doc)" } else { "" }), "Kommentar angelegt"));
            }
            reply(folder, &rel, id.trim(), "ask", &text, done)?;
            live(ctx, &rel, true);
            Ok(ToolResult::new(true, format!("Antwort im Kommentar {id} gespeichert{}.", if done == Some(true) { ", erledigt" } else { "" }), if done == Some(true) { "beantwortet, erledigt" } else { "beantwortet" }))
        }
        _ => bail!("Unbekanntes Werkzeug: {name}"),
    }
}

// ---------- Export ----------

/// Doc als Word-Datei
pub fn export_docx(folder: &str, rel: &str) -> Result<Vec<u8>> {
    let doc = read(folder, rel)?;
    let path = full_path(folder, &str_of(&doc, "path"))?;
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from(folder));
    crate::docx::markdown_to_docx_in(&str_of(&doc, "content"), &str_of(&doc, "title"), &dir, Path::new(folder))
}

// ---------- Oberfläche ----------

fn fail(reason: impl Into<String>) -> Value {
    json!({ "ok": false, "reason": reason.into() })
}

fn s(a: &[Value], i: usize) -> String {
    a.get(i).map(js_string).unwrap_or_default()
}

fn or_fail(r: Result<Value>) -> Value {
    r.unwrap_or_else(|e| fail(e.to_string()))
}

impl App {
    /// Kanäle `docs:*` (ENTSCHEIDE E81); `None`, wenn der Kanal nicht dazugehört. Gilt immer für das sichtbare Projekt.
    pub(crate) async fn handle_docs(&self, channel: &str, a: &[Value]) -> Option<Result<Value, String>> {
        if !channel.starts_with("docs:") {
            return None;
        }
        let folder = self.workspace().root();
        if folder.is_empty() {
            return Some(Ok(fail("Kein Projekt offen.")));
        }
        let f = folder.as_str();
        let v = match channel {
            "docs:list" => json!({ "ok": true, "docs": list(f) }),
            "docs:read" => or_fail(read(f, &s(a, 0)).map(|mut d| {
                d["ok"] = json!(true);
                d["comments"] = json!(threads(f, &s(a, 0)));
                d["share"] = crate::docsync::info(f, &str_of(&d, "path"));
                d
            })),
            "docs:share-info" => crate::docsync::info(f, &doc_rel(&s(a, 0)).unwrap_or_default()),
            "docs:share" => {
                let rel = doc_rel(&s(a, 0)).unwrap_or_default();
                let start = crate::paths::home().to_string_lossy().to_string();
                match self.host.pick_folder("Geteilten Ordner wählen (OneDrive, Teams oder Laufwerk der Schule)", &start).await {
                    None => json!({ "ok": false, "canceled": true }),
                    Some(dir) => or_fail(crate::docsync::share(f, &rel, &dir, &self.share_name()).map(|mut i| { i["ok"] = json!(true); i })),
                }
            }
            "docs:join" => {
                let start = crate::paths::home().to_string_lossy().to_string();
                match self.host.pick_folder("Geteiltes Doc öffnen (Ordner «….askdoc»)", &start).await {
                    None => json!({ "ok": false, "canceled": true }),
                    Some(dir) => or_fail(crate::docsync::join(f, &dir, &self.share_name()).map(|p| json!({ "ok": true, "path": p }))),
                }
            }
            // Teilen per Link über Microsoft (E83)
            "docs:ms-status" => crate::msgraph::status(),
            "docs:ms-login" => {
                let host = self.host.clone();
                match crate::msgraph::login(move |url| host.open_external(url)).await {
                    Ok(v) => { let mut v = v; v["ok"] = json!(true); v }
                    Err(e) => fail(e.to_string()),
                }
            }
            "docs:ms-logout" => { crate::msgraph::logout(); json!({ "ok": true }) }
            "docs:share-link" => {
                let (rel, project, name) = (doc_rel(&s(a, 0)).unwrap_or_default(), f.to_string(), self.share_name());
                let rt = tokio::runtime::Handle::current();
                let r = tokio::task::spawn_blocking(move || {
                    let token = crate::msgraph::app_token(rt.clone());
                    crate::docsync::share_link(&project, &rel, &name, rt, token)
                }).await.unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
                or_fail(r.map(|mut i| { i["ok"] = json!(true); i }))
            }
            "docs:join-link" => {
                let (link, project, name) = (s(a, 0), f.to_string(), self.share_name());
                let rt = tokio::runtime::Handle::current();
                let r = tokio::task::spawn_blocking(move || {
                    let token = crate::msgraph::app_token(rt.clone());
                    crate::docsync::join_link(&project, &link, &name, rt, token)
                }).await.unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
                or_fail(r.map(|p| json!({ "ok": true, "path": p })))
            }
            "docs:unshare" => {
                let rel = doc_rel(&s(a, 0)).unwrap_or_default();
                let ok = self.confirm("Nicht mehr teilen?", "Das Doc bleibt bei dir, bekommt aber keine Änderungen der anderen mehr, und sie keine von dir. Der geteilte Ordner bleibt für die anderen.",
                    crate::host::ConfirmOptions::default()).await.ok;
                if ok { crate::docsync::unshare(f, &rel); json!({ "ok": true }) } else { json!({ "ok": false, "canceled": true }) }
            }
            "docs:show-folder" => {
                let i = crate::docsync::info(f, &doc_rel(&s(a, 0)).unwrap_or_default());
                if truthy(&i["geteilt"]) {
                    if i["link"].is_string() { self.host.open_external(&str_of(&i, "link")); } else { self.host.open_path(&str_of(&i, "ordner")).await; }
                }
                Value::Null
            }
            "docs:save" => {
                let base = s(a, 2);
                or_fail(save(f, &s(a, 0), &s(a, 1), Some(base.as_str())))
            }
            "docs:create" => or_fail(create(f, &s(a, 0), "").map(|p| json!({ "ok": true, "path": p }))),
            "docs:open" => { set_open(f, &s(a, 0)); Value::Null }
            "docs:delete" => {
                let rel = s(a, 0);
                let title = read(f, &rel).map(|d| str_of(&d, "title")).unwrap_or_else(|_| rel.clone());
                let ok = self.confirm("Doc löschen?", &format!("«{title}» und seine Kommentare werden gelöscht. «Rückgängig» unter dem letzten Auftrag holt die Datei nicht zurück."),
                    crate::host::ConfirmOptions { danger: true, ..Default::default() }).await.ok;
                if !ok {
                    json!({ "ok": false, "canceled": true })
                } else {
                    crate::docsync::unshare(f, &doc_rel(&rel).unwrap_or_default());
                    or_fail(remove(f, &rel).map(|_| json!({ "ok": true })))
                }
            }
            "docs:comments" => json!({ "ok": true, "comments": threads(f, &s(a, 0)) }),
            "docs:comment-add" => or_fail(add_comment_with(f, &s(a, 0), &s(a, 1), &s(a, 2), "person", &s(a, 3)).map(|t| json!({ "ok": true, "thread": t, "message": comment_message(&doc_rel(&s(a, 0)).unwrap_or_default(), &t) }))),
            "docs:comment-reply" => or_fail(reply(f, &s(a, 0), &s(a, 1), "person", &s(a, 2), None).map(|t| json!({ "ok": true, "thread": t, "message": comment_message(&doc_rel(&s(a, 0)).unwrap_or_default(), &t) }))),
            "docs:comment-done" => or_fail(set_done(f, &s(a, 0), &s(a, 1), truthy(&a.get(2).cloned().unwrap_or(Value::Null))).map(|t| json!({ "ok": true, "thread": t }))),
            "docs:comment-delete" => or_fail(delete_comment(f, &s(a, 0), &s(a, 1)).map(|_| json!({ "ok": true }))),
            "docs:export" => self.export_doc(f, &s(a, 0), &s(a, 1)).await,
            _ => return Some(Err(format!("Unbekannter Kanal: {channel}"))),
        };
        Some(Ok(v))
    }

    /// Name, unter dem die Person in geteilten Docs erscheint
    fn share_name(&self) -> String {
        crate::docsync::display_name(self.ask.user().map(|u| u.name))
    }

    async fn export_doc(&self, folder: &str, rel: &str, format: &str) -> Value {
        let doc = match read(folder, rel) {
            Ok(d) => d,
            Err(e) => return fail(e.to_string()),
        };
        let name = slug(&str_of(&doc, "title"));
        let (label, ext) = match format {
            "docx" => ("Word-Dokument", "docx"),
            "pdf" => ("PDF", "pdf"),
            _ => ("Markdown", "md"),
        };
        let Some(target) = self.host.pick_save_file("Doc speichern", &format!("{name}.{ext}"), vec![(label.to_string(), vec![ext.to_string()])]).await else {
            return json!({ "ok": false, "canceled": true });
        };
        // Ohne Endung öffnet Windows die Datei nicht mit Word
        let target = if target.to_lowercase().ends_with(&format!(".{ext}")) { target } else { format!("{target}.{ext}") };
        let res: Result<()> = async {
            match ext {
                "md" => std::fs::write(&target, str_of(&doc, "content").as_bytes())?,
                "docx" => std::fs::write(&target, export_docx(folder, rel)?)?,
                _ => {
                    // PDF über Word (dieselbe abgesicherte Umwandlung wie für pruefen.py, ENTSCHEIDE E15, E20)
                    let dir = std::env::temp_dir().join(format!("ask-docs-{}", hex::encode(rand::random::<[u8; 4]>())));
                    std::fs::create_dir_all(&dir)?;
                    let source = dir.join(format!("{name}.docx"));
                    std::fs::write(&source, export_docx(folder, rel)?)?;
                    let conv = crate::office::convert_to_pdf(Path::new(folder), &source, false).await;
                    let _ = std::fs::remove_dir_all(&dir);
                    match conv.target.filter(|_| conv.ok) {
                        Some(pdf) => { std::fs::copy(&pdf, &target)?; let _ = std::fs::remove_file(&pdf); }
                        None => bail!("{} Als Word speichern geht ohne Office.", conv.fehler.unwrap_or_else(|| "PDF braucht Microsoft Word.".into())),
                    }
                }
            }
            Ok(())
        }.await;
        match res {
            Ok(()) => json!({ "ok": true, "path": target }),
            Err(e) if e.downcast_ref::<std::io::Error>().is_some_and(|io| io.kind() == std::io::ErrorKind::PermissionDenied) =>
                fail("Nicht gespeichert: Die Datei ist wohl gerade in Word geöffnet. Schliess sie dort und versuch es nochmals, oder wähle einen anderen Namen."),
            Err(e) => fail(format!("Nicht gespeichert: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(folder: &Path) -> ToolCtx {
        ToolCtx { folder: folder.to_string_lossy().to_string(), ..Default::default() }
    }

    #[test]
    fn pfade_bleiben_in_docs() {
        assert_eq!(doc_rel("bericht").unwrap(), "docs/bericht.md");
        assert_eq!(doc_rel("docs/bericht.md").unwrap(), "docs/bericht.md");
        assert_eq!(doc_rel(".\\Docs\\a\\b.md").unwrap(), "docs/a/b.md");
        assert_eq!(doc_rel("/docs/x").unwrap(), "docs/x.md");
        // Umlaut oder ß über Byte 5: kein Absturz beim Vergleich mit «docs/»
        assert_eq!(doc_rel("Straße").unwrap(), "docs/Straße.md");
        assert_eq!(doc_rel("Größe").unwrap(), "docs/Größe.md");
        for bad in ["", "docs/../geheim.md", "..\\x", "docs/a:b.md", "C:/x.md", "docs/.md"] {
            assert!(doc_rel(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn namen_und_titel() {
        assert_eq!(slug("Bericht über Äpfel & Birnen!"), "bericht-ueber-aepfel-birnen");
        assert_eq!(slug("Café Crème"), "cafe-creme");
        assert_eq!(slug("???"), "doc");
        assert_eq!(slug(&"a".repeat(100)).len(), 60);
        assert_eq!(title_of("Vorwort\n# **Mein Titel**\n## Teil", "docs/x.md"), "Mein Titel");
        assert_eq!(title_of("## nur Unterkapitel", "docs/notizen.md"), "notizen");
    }

    #[test]
    fn anlegen_lesen_speichern_mit_konflikt() {
        let dir = tempfile::tempdir().unwrap();
        TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.path().join("daten")));
        let f = dir.path().to_string_lossy().to_string();
        let rel = create(&f, "Lernzettel Chemie", "## Atome\n\n…").unwrap();
        assert_eq!(rel, "docs/lernzettel-chemie.md");
        assert_eq!(create(&f, "Lernzettel Chemie", "").unwrap(), "docs/lernzettel-chemie-2.md", "nie überschreiben");
        let doc = read(&f, &rel).unwrap();
        assert_eq!(str_of(&doc, "content"), "# Lernzettel Chemie\n\n## Atome\n\n…\n");
        assert_eq!(str_of(&doc, "title"), "Lernzettel Chemie");
        let base = str_of(&doc, "hash");
        // Die Person speichert auf dem Stand, den sie geladen hat
        let saved = save(&f, &rel, "# Lernzettel Chemie\n\nNeu\n", Some(&base)).unwrap();
        assert_eq!(saved["ok"], true);
        // Ein zweites Speichern auf dem alten Stand (ASK hat dazwischen geschrieben) überschreibt nichts
        let conflict = save(&f, &rel, "# alt bearbeitet\n", Some(&base)).unwrap();
        assert_eq!(conflict["conflict"], true);
        assert_eq!(str_of(&conflict, "content"), "# Lernzettel Chemie\n\nNeu\n");
        let names: Vec<String> = list(&f).iter().map(|d| str_of(d, "path")).collect();
        assert_eq!(names.len(), 2);
        remove(&f, &rel).unwrap();
        assert_eq!(list(&f).len(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn keine_verknuepfung_aus_dem_ordner() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir.path().join("docs")).unwrap();
        let f = dir.path().to_string_lossy().to_string();
        assert!(create(&f, "x", "").is_err());
        assert!(save(&f, "docs/x.md", "x", None).is_err());
        assert!(std::fs::read_dir(outside.path()).unwrap().next().is_none());
    }

    #[test]
    fn kommentare_mit_antworten() {
        let dir = tempfile::tempdir().unwrap();
        TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.path().join("daten")));
        let proj = dir.path().join("p");
        std::fs::create_dir_all(&proj).unwrap();
        let f = proj.to_string_lossy().to_string();
        let rel = create(&f, "Brief", "Liebe Lea").unwrap();
        let t = add_comment(&f, &rel, "Liebe Lea", "Förmlicher bitte").unwrap();
        let id = str_of(&t, "id");
        assert_eq!(open_count(&f, &rel), 1);
        assert!(comment_message(&rel, &t).starts_with(&format!("[Kommentar im Doc docs/brief.md, {id}] zur Stelle «Liebe Lea»: Förmlicher bitte")));
        let r = reply(&f, &rel, &id, "ask", "Anrede geändert.", Some(true)).unwrap();
        assert_eq!(r["eintraege"].as_array().unwrap().len(), 2);
        assert_eq!(open_count(&f, &rel), 0);
        assert!(reply(&f, &rel, "k000000", "ask", "x", None).is_err());
        assert!(add_comment(&f, &rel, "", "  ").is_err());
        delete_comment(&f, &rel, &id).unwrap();
        assert!(threads(&f, &rel).is_empty());
        // Kommentare liegen nicht im Projektordner
        assert!(walkdir::WalkDir::new(&proj).into_iter().flatten().all(|e| !e.path().to_string_lossy().ends_with(".json")));
        // unlesbare Kommentardatei: zur Seite gelegt, nicht überschrieben
        let file = comments_file(&f);
        std::fs::write(&file, "{ kaputt").unwrap();
        add_comment(&f, &rel, "Liebe Lea", "Neu").unwrap();
        let aside: Vec<String> = std::fs::read_dir(file.parent().unwrap()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.contains(".kaputt-")).collect();
        assert_eq!(aside.len(), 1, "{aside:?}");
        assert_eq!(std::fs::read_to_string(file.with_file_name(&aside[0])).unwrap(), "{ kaputt");
        assert_eq!(threads(&f, &rel).len(), 1);
    }

    #[tokio::test]
    async fn werkzeuge_erstellen_fuellen_und_kommentieren() {
        let dir = tempfile::tempdir().unwrap();
        TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.path().join("daten")));
        let proj = dir.path().join("p");
        std::fs::create_dir_all(&proj).unwrap();
        let c = ctx(&proj);
        let f = c.folder.clone();
        let r = run("doc_erstellen", &json!({ "titel": "Bericht", "inhalt": "## Einleitung\n\n…\n\n## Schluss\n\n…" }), &c).await.unwrap().unwrap();
        assert!(r.ok);
        assert_eq!(r.made.as_deref(), Some("docs/bericht.md"));
        let twice = run("doc_bearbeiten", &json!({ "pfad": "docs/bericht.md", "alt": "…", "neu": "x" }), &c).await.unwrap();
        assert!(twice.unwrap_err().to_string().contains("2-mal"));
        let ok = run("doc_bearbeiten", &json!({ "pfad": "bericht", "alt": "## Einleitung\n\n…", "neu": "## Einleitung\n\nDas ist die Einleitung." }), &c).await.unwrap().unwrap();
        assert!(ok.ok, "{}", ok.output);
        run("doc_bearbeiten", &json!({ "pfad": "bericht", "anhaengen": "## Quellen\n\n- A" }), &c).await.unwrap().unwrap();
        let none = run("doc_bearbeiten", &json!({ "pfad": "bericht", "alt": "gibt es nicht", "neu": "x" }), &c).await.unwrap();
        assert!(none.is_err());
        let both = run("doc_bearbeiten", &json!({ "pfad": "bericht", "anhaengen": "a", "inhalt": "b" }), &c).await.unwrap();
        assert!(both.is_err());
        let text = str_of(&read(&f, "docs/bericht.md").unwrap(), "content");
        assert_eq!(text, "# Bericht\n\n## Einleitung\n\nDas ist die Einleitung.\n\n## Schluss\n\n…\n\n## Quellen\n\n- A\n");
        let t = add_comment(&f, "docs/bericht.md", "Einleitung", "Länger?").unwrap();
        let read_out = run("doc_lesen", &json!({ "pfad": "docs/bericht.md" }), &c).await.unwrap().unwrap();
        assert!(read_out.output.contains("Offene Kommentare") && read_out.output.contains("Länger?"));
        let answered = run("doc_kommentieren", &json!({ "pfad": "docs/bericht.md", "id": str_of(&t, "id"), "antwort": "Ergänzt.", "erledigt": true }), &c).await.unwrap().unwrap();
        assert!(answered.ok);
        assert_eq!(open_count(&f, "docs/bericht.md"), 0);
        let listed = run("doc_liste", &json!({}), &c).await.unwrap().unwrap();
        assert!(listed.output.contains("docs/bericht.md «Bericht»"));
        assert!(run("read_file", &json!({}), &c).await.is_none());
    }

    #[tokio::test]
    async fn anstreichen_und_kommentare_von_ask() {
        let dir = tempfile::tempdir().unwrap();
        TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.path().join("daten")));
        let proj = dir.path().join("p");
        std::fs::create_dir_all(&proj).unwrap();
        let c = ctx(&proj);
        let f = c.folder.clone();
        let rel = create(&f, "Notizen", "Die Mitochondrien sind die Kraftwerke der Zelle.").unwrap();
        // Nur angestrichen: ohne Text, mit Farbe; ohne Stelle oder ohne Farbe geht das nicht
        let mark = add_comment_with(&f, &rel, "Kraftwerke", "", "person", "gruen").unwrap();
        assert_eq!(mark["farbe"], "gruen");
        assert!(mark["eintraege"].as_array().unwrap().is_empty());
        assert!(add_comment_with(&f, &rel, "", "", "person", "gruen").is_err());
        assert!(add_comment_with(&f, &rel, "Kraftwerke", "", "person", "lila").is_err(), "unbekannte Farbe gilt als keine");
        // ASK legt Vorschläge als eigene Kommentare an; eine Stelle, die es nicht gibt, wird zum ganzen Doc
        let r = run("doc_kommentieren", &json!({ "pfad": rel, "zitat": "Mitochondrien", "antwort": "Hier ein Bild der Zelle?" }), &c).await.unwrap().unwrap();
        assert!(r.ok, "{}", r.output);
        let r2 = run("doc_kommentieren", &json!({ "pfad": rel, "zitat": "gibt es nicht", "antwort": "Quelle fehlt" }), &c).await.unwrap().unwrap();
        assert!(r2.output.contains("ganzen Doc"));
        let all = threads(&f, &rel);
        assert_eq!(all.len(), 3);
        assert_eq!(all[1]["eintraege"][0]["von"], "ask");
        assert_eq!(all[1]["zitat"], "Mitochondrien");
        assert_eq!(all[2]["zitat"], "");
        let shown = run("doc_lesen", &json!({ "pfad": rel }), &c).await.unwrap().unwrap();
        assert!(shown.output.contains("(nur angestrichen)") && shown.output.contains("ASK: Hier ein Bild"));
    }

    #[test]
    fn systemprompt_nennt_docs_und_offenes_doc() {
        let dir = tempfile::tempdir().unwrap();
        TEST_DIR.with(|d| *d.borrow_mut() = Some(dir.path().join("daten")));
        let f = dir.path().to_string_lossy().to_string();
        assert!(system_prompt(&f).contains("# ASK Docs"));
        assert!(!system_prompt(&f).contains("Docs hier"));
        let rel = create(&f, "Plan", "").unwrap();
        set_open(&f, &rel);
        let p = system_prompt(&f);
        assert!(p.contains("Docs hier: docs/plan.md «Plan»"), "{p}");
        assert!(p.contains("Gerade offen im Reiter «Docs»: docs/plan.md"));
        set_open(&f, "");
        assert!(!system_prompt(&f).contains("Gerade offen"));
    }

    #[test]
    fn werkzeuge_haben_beschreibungen() {
        let tools = doc_tools();
        assert_eq!(tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), DOC_TOOLS.to_vec());
        assert_eq!(describe("doc_erstellen", &json!({ "titel": "Bericht" })), "Bericht");
        assert_eq!(describe("doc_bearbeiten", &json!({ "pfad": "docs/a.md" })), "docs/a.md");
    }
}
