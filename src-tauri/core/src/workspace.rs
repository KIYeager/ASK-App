//! Arbeitsordner: Grundstruktur anlegen, Skills kopieren, Änderungen beobachten, Dateien auflisten.
//!
//! Öffentliche Schnittstelle (Port von src/main/workspace.js):
//! - `Workspace::new()`; `events: Emitter` mit den Ereignissen `files` (Nutzlast `{ top, recent }`) und
//!   `auftrag` (Nutzlast: Pfad der Auftragsdatei als Text), wie `on('files')` und `on('auftrag')` in JavaScript
//! - `open(folder) -> Result<()>`, `close()`, `root() -> String`, `files() -> Value`, `list(rel) -> Value`,
//!   `find(query) -> Vec<Value>`, `import(rel_dir, files) -> Result<Vec<String>>`, `resolve(rel) -> Option<PathBuf>`,
//!   `sync_skills() -> Result<()>`
//! - freie Funktionen `recent_files(root) -> Vec<Value>`, `locale_cmp_de(a, b)` (`localeCompare(…, 'de')`)
//!
//! Die Dateiänderungen kommen vom Crate `notify` (in JavaScript `fs.watch`) und werden wie dort
//! entprellt: «files» 250 ms nach der letzten Änderung, «auftrag» 300 ms nach der letzten Änderung
//! der Datei. Dafür läuft pro geöffnetem Ordner ein kleiner Thread.

use crate::util::Emitter;
use anyhow::{bail, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use unicode_normalization::UnicodeNormalization;

pub const ASK_DIR: &str = ".ask";
const MAX_ENTRIES: usize = 300;
const RECENT: usize = 12; // Seitenleiste «Zuletzt geändert»
const FILES_DELAY: Duration = Duration::from_millis(250);
const ORDER_DELAY: Duration = Duration::from_millis(300);

/// Nicht in der Seitenleiste
fn hidden(name: &str) -> bool {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^(\.ask|\.git|node_modules|__pycache__|\.venv)$|^~\$").expect("gültiges Muster")).is_match(name)
}

/// Sortierung wie `a.localeCompare(b, 'de')` (ICU): Satzzeichen vor Ziffern vor Buchstaben,
/// Gross-/Kleinschreibung und Akzente erst bei sonst gleichen Namen, klein vor gross.
pub fn locale_cmp_de(a: &str, b: &str) -> Ordering {
    fn primary(s: &str) -> Vec<(u8, char)> {
        s.nfkd()
            .filter(|c| !('\u{300}'..='\u{36f}').contains(c))
            .flat_map(char::to_lowercase)
            .map(|c| (if c.is_alphabetic() { 2 } else if c.is_numeric() { 1 } else { 0 }, c))
            .collect()
    }
    primary(a).cmp(&primary(b)).then_with(|| {
        // Akzente: ohne vor mit
        let (ka, kb) = (a.to_lowercase(), b.to_lowercase());
        ka.nfkd().collect::<String>().cmp(&kb.nfkd().collect::<String>())
    }).then_with(|| {
        for (x, y) in a.chars().zip(b.chars()) {
            if x != y {
                return match (x.is_lowercase(), y.is_lowercase()) {
                    (true, false) => Ordering::Less,
                    (false, true) => Ordering::Greater,
                    _ => x.cmp(&y),
                };
            }
        }
        a.cmp(b)
    })
}

fn mtime_ms(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

fn entry(path: &str, name: &str, dir: bool, size: u64, modified: f64) -> Value {
    json!({ "path": path, "name": name, "dir": dir, "size": size, "modified": modified })
}

fn rel_of(root: &Path, full: &Path) -> String {
    full.strip_prefix(root).unwrap_or(full).to_string_lossy().to_string()
}

/// `path.resolve` ohne Zugriff auf die Platte: «.» und «..» auflösen
fn clean_lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn is_ask_path(rel: &str) -> bool {
    rel == ASK_DIR || rel.starts_with(&format!("{ASK_DIR}{}", std::path::MAIN_SEPARATOR))
}

fn list_recursive(root: &Path, dir: &Path, depth: i32, out: &mut Vec<Value>) {
    if out.len() >= MAX_ENTRIES || depth < 0 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.filter_map(Result::ok) {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let full = e.path();
        let rel = rel_of(root, &full);
        if is_ask_path(&rel) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&full) else { continue };
        let is_dir = e.file_type().is_ok_and(|t| t.is_dir());
        let name = e.file_name().to_string_lossy().to_string();
        let mut v = entry(&rel, &name, is_dir, meta.len(), mtime_ms(&meta));
        v["depth"] = json!(rel.split(std::path::MAIN_SEPARATOR).count() as i64 - 1);
        out.push(v);
        if is_dir {
            list_recursive(root, &full, depth - 1, out);
        }
    }
}

/// Zuletzt geänderte Dateien im ganzen Ordner (ohne .ask, .git, node_modules usw.), neueste zuerst.
pub fn recent_files(root: &Path) -> Vec<Value> {
    fn walk(root: &Path, dir: &Path, depth: i32, seen: &mut usize, files: &mut Vec<(f64, Value)>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.filter_map(Result::ok) {
            *seen += 1;
            let name = e.file_name().to_string_lossy().to_string();
            if *seen > 5000 || hidden(&name) {
                continue;
            }
            let full = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                if depth > 0 {
                    walk(root, &full, depth - 1, seen, files);
                }
            } else if let Ok(meta) = std::fs::metadata(&full) {
                let m = mtime_ms(&meta);
                files.push((m, entry(&rel_of(root, &full), &name, false, meta.len(), m)));
            } // sonst: gerade gelöscht
        }
    }
    let mut files = Vec::new();
    let mut seen = 0;
    walk(root, root, 6, &mut seen, &mut files);
    files.sort_by(|a, b| b.0.total_cmp(&a.0));
    files.into_iter().take(RECENT).map(|(_, v)| v).collect()
}

fn hash_dir(dir: &Path) -> Result<String> {
    fn walk(base: &Path, d: &Path, hash: &mut Sha1) -> Result<()> {
        let mut entries: Vec<_> = std::fs::read_dir(d)?.filter_map(Result::ok).collect();
        entries.sort_by(|a, b| locale_cmp_de(&a.file_name().to_string_lossy(), &b.file_name().to_string_lossy()));
        for e in entries {
            let full = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                walk(base, &full, hash)?;
            } else {
                hash.update(rel_of(base, &full).as_bytes());
                hash.update(std::fs::read(&full)?);
            }
        }
        Ok(())
    }
    let mut hash = Sha1::new();
    walk(dir, dir, &mut hash)?;
    Ok(hex::encode(hash.finalize()))
}

fn copy_dir_all(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let target = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir_all(&e.path(), &target)?;
        } else {
            std::fs::copy(e.path(), &target)?;
        }
    }
    Ok(())
}

/// Was eine Änderung im Ordner auslöst
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Change {
    /// ignorieren
    None,
    /// Auftragsdatei geändert (nach kurzer Ruhe melden)
    Order,
    /// Dateiliste neu senden (nach kurzer Ruhe)
    Files,
}

/// `name`: Pfad relativ zum Arbeitsordner, mit dem Trenner des Systems
fn classify(name: &str) -> Change {
    static ORDER: OnceLock<Regex> = OnceLock::new();
    let sep = std::path::MAIN_SEPARATOR;
    let under = |sub: &str| name.starts_with(&format!("{ASK_DIR}{sep}{sub}"));
    if ["skills", "chats", "skripte", "arbeit", "anhaenge", "sicherung"].iter().any(|s| under(s)) {
        return Change::None;
    }
    let order = ORDER.get_or_init(|| Regex::new(r"(?i)^\.ask[\\/]auftraege[\\/][0-9a-f]{6,32}\.json$").expect("gültiges Muster"));
    if order.is_match(name) {
        return Change::Order;
    }
    if under("auftraege") || under("pdf") {
        return Change::None;
    }
    if name.to_lowercase() == format!("{ASK_DIR}{sep}tasks.json") {
        return Change::None; // Datei früherer Versionen
    }
    Change::Files
}

enum Msg {
    Files,
    Order(PathBuf),
    Stop,
}

/// Entprellt die Meldungen (in JavaScript `setTimeout` und `clearTimeout`)
fn debounce_worker(rx: mpsc::Receiver<Msg>, events: Emitter, root: PathBuf) {
    let mut files_at: Option<Instant> = None;
    let mut orders: HashMap<PathBuf, Instant> = HashMap::new();
    loop {
        let next = files_at.into_iter().chain(orders.values().copied()).min();
        let msg = match next {
            Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
            None => rx.recv().map_err(|_| RecvTimeoutError::Disconnected),
        };
        match msg {
            Ok(Msg::Files) => files_at = Some(Instant::now() + FILES_DELAY),
            Ok(Msg::Order(file)) => {
                orders.insert(file, Instant::now() + ORDER_DELAY);
            }
            Ok(Msg::Stop) | Err(RecvTimeoutError::Disconnected) => return,
            Err(RecvTimeoutError::Timeout) => {}
        }
        let now = Instant::now();
        if files_at.is_some_and(|t| t <= now) {
            files_at = None;
            events.emit("files", &files_of(&root));
        }
        let due: Vec<PathBuf> = orders.iter().filter(|(_, t)| **t <= now).map(|(f, _)| f.clone()).collect();
        for file in due {
            orders.remove(&file);
            if file.exists() {
                events.emit("auftrag", &Value::String(file.to_string_lossy().to_string()));
            }
        }
    }
}

fn files_of(root: &Path) -> Value {
    let mut top = Vec::new();
    list_recursive(root, root, 0, &mut top);
    let is_dir = |v: &Value| v["dir"].as_bool().unwrap_or(false);
    let name = |v: &Value| v["name"].as_str().unwrap_or("").to_string();
    top.sort_by(|a, b| is_dir(b).cmp(&is_dir(a)).then_with(|| locale_cmp_de(&name(a), &name(b))));
    // Anzahl Unterlagen im Ordner «Quellen» (E84): die Oberfläche zeigt damit den Schalter «Quellen»
    json!({ "top": top, "recent": recent_files(root), "quellen": crate::sources::count(root) })
}

#[derive(Default)]
struct State {
    root: String,
    watcher: Option<notify::RecommendedWatcher>,
    worker: Option<Sender<Msg>>,
}

/// Hineingezogene Datei (`{ name, type, data }`)
#[derive(Clone, Debug, Default)]
pub struct ImportFile {
    pub name: String,
    /// MIME-Typ (nur für Namen eingefügter Bilder)
    pub kind: String,
    pub data: Vec<u8>,
}

pub struct Workspace {
    pub events: Emitter,
    state: Mutex<State>,
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new()
    }
}

impl Workspace {
    pub fn new() -> Self {
        Workspace { events: Emitter::new(), state: Mutex::new(State::default()) }
    }

    pub fn root(&self) -> String {
        self.state.lock().root.clone()
    }

    /// Ordner setzen, ohne ihn zu öffnen (für Tests und Werkzeuge, die nur die Listen brauchen)
    pub fn set_root(&self, folder: &str) {
        self.state.lock().root = folder.to_string();
    }

    pub fn open(&self, folder: &str) -> Result<()> {
        self.close();
        self.state.lock().root = folder.to_string();
        let root = PathBuf::from(folder);
        std::fs::create_dir_all(root.join(ASK_DIR))?;
        crate::cleanup::hide_folder(&root.join(ASK_DIR));
        let cleanup_root = root.clone();
        // nicht beim Öffnen bremsen; die Sicherung räumt main.js auf
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(2));
            crate::cleanup::cleanup_work_dirs(&cleanup_root);
        });
        let _ = std::fs::remove_dir_all(root.join(ASK_DIR).join("auftraege"));
        self.sync_skills()?;
        let (tx, rx) = mpsc::channel::<Msg>();
        let events = self.events.clone();
        let worker_root = root.clone();
        std::thread::spawn(move || debounce_worker(rx, events, worker_root));
        let watcher = start_watcher(&root, tx.clone());
        {
            let mut st = self.state.lock();
            st.watcher = watcher;
            st.worker = Some(tx);
        }
        self.events.emit("files", &self.files());
        Ok(())
    }

    pub fn close(&self) {
        let mut st = self.state.lock();
        st.watcher = None; // beendet die Beobachtung
        if let Some(tx) = st.worker.take() {
            let _ = tx.send(Msg::Stop);
        }
        st.root.clear();
    }

    /// Kopiert die mitgelieferten Anleitungen nach .ask/skills, wenn sich ihr Inhalt geändert hat.
    pub fn sync_skills(&self) -> Result<()> {
        let source = crate::paths::skills_dir();
        if !source.exists() {
            return Ok(());
        }
        let target = PathBuf::from(self.root()).join(ASK_DIR).join("skills");
        let marker = target.join(".version");
        let version = hash_dir(&source)?;
        if std::fs::read_to_string(&marker).is_ok_and(|v| v == version) {
            return Ok(());
        }
        let _ = std::fs::remove_dir_all(&target);
        copy_dir_all(&source, &target)?;
        std::fs::write(&marker, version)?;
        Ok(())
    }

    pub fn files(&self) -> Value {
        let root = self.root();
        if root.is_empty() {
            return json!({ "top": [], "recent": [] });
        }
        files_of(Path::new(&root))
    }

    /// Inhalt eines Unterordners für den Dateibaum (rel «» = Arbeitsordner), Ordner zuerst
    pub fn list(&self, rel: &str) -> Value {
        let fail = || json!({ "ok": false, "entries": [] });
        let root = self.root();
        if root.is_empty() {
            return fail();
        }
        let dir = if rel.is_empty() { dunce::canonicalize(&root).ok() } else { self.resolve(rel) };
        let Some(dir) = dir else { return fail() };
        let Ok(rd) = std::fs::read_dir(&dir) else { return fail() };
        let all: Vec<_> = rd.filter_map(Result::ok).collect();
        let mut out: Vec<Value> = Vec::new();
        for e in &all {
            let name = e.file_name().to_string_lossy().to_string();
            if out.len() >= MAX_ENTRIES || hidden(&name) {
                continue;
            }
            if let Ok(meta) = std::fs::metadata(e.path()) {
                let path = clean_lexical(&Path::new(rel).join(&name)).to_string_lossy().to_string();
                out.push(entry(&path, &name, meta.is_dir(), meta.len(), mtime_ms(&meta)));
            } // sonst: gerade gelöscht oder gesperrt
        }
        let is_dir = |v: &Value| v["dir"].as_bool().unwrap_or(false);
        out.sort_by(|a, b| is_dir(b).cmp(&is_dir(a)).then_with(|| locale_cmp_de(a["name"].as_str().unwrap_or(""), b["name"].as_str().unwrap_or(""))));
        json!({ "ok": true, "entries": out, "more": all.len() > MAX_ENTRIES })
    }

    /// Dateien im ganzen Projekt nach Namen suchen (für die Suche im Dateibereich), höchstens 200 Treffer
    pub fn find(&self, query: &str) -> Vec<Value> {
        let words: Vec<String> = query.to_lowercase().split_whitespace().map(str::to_string).collect();
        let root = self.root();
        if root.is_empty() || words.is_empty() {
            return Vec::new();
        }
        fn walk(root: &Path, dir: &Path, depth: i32, words: &[String], seen: &mut usize, out: &mut Vec<Value>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for e in rd.filter_map(Result::ok) {
                *seen += 1;
                let name = e.file_name().to_string_lossy().to_string();
                if out.len() >= 200 || *seen > 20000 || hidden(&name) {
                    continue;
                }
                let full = e.path();
                let rel = rel_of(root, &full);
                let lower = rel.to_lowercase();
                if words.iter().all(|w| lower.contains(w.as_str())) {
                    if let Ok(meta) = std::fs::metadata(&full) {
                        out.push(entry(&rel, &name, meta.is_dir(), meta.len(), mtime_ms(&meta)));
                    } // sonst: gerade gelöscht
                }
                if e.file_type().is_ok_and(|t| t.is_dir()) && depth > 0 {
                    walk(root, &full, depth - 1, words, seen, out);
                }
            }
        }
        let mut out = Vec::new();
        let mut seen = 0;
        walk(Path::new(&root), Path::new(&root), 8, &words, &mut seen, &mut out);
        let is_dir = |v: &Value| v["dir"].as_bool().unwrap_or(false);
        let modified = |v: &Value| v["modified"].as_f64().unwrap_or(0.0);
        out.sort_by(|a, b| is_dir(b).cmp(&is_dir(a)).then_with(|| modified(b).total_cmp(&modified(a))));
        out
    }

    /// Hineingezogene Dateien in einen Ordner des Projekts kopieren. Vorhandene Dateien werden nie
    /// überschrieben («name (2).ext»). Liefert die neuen relativen Pfade.
    pub fn import(&self, rel_dir: &str, files: &[ImportFile]) -> Result<Vec<String>> {
        let root = self.root();
        let dir = if !rel_dir.is_empty() {
            self.resolve(rel_dir)
        } else if !root.is_empty() {
            dunce::canonicalize(&root).ok()
        } else {
            None
        };
        let Some(dir) = dir.filter(|d| d.is_dir()) else { bail!("Ordner nicht gefunden.") };
        // resolve() liefert den echten Pfad; der gespeicherte Ordner kann eine Kurzform sein (C:\Users\RUNNER~1\…)
        let real_root = dunce::canonicalize(&root)?;
        let mut done = Vec::new();
        for f in files.iter().take(50) {
            let name = crate::attachments::free_name(&dir, &crate::attachments::clean_name(&f.name, &f.kind));
            if f.data.len() > 200 * 1024 * 1024 {
                bail!("{name} ist grösser als 200 MB.");
            }
            let target = dir.join(&name);
            {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
                file.write_all(&f.data)?;
            }
            done.push(rel_of(&real_root, &target));
        }
        Ok(done)
    }

    /// Absoluter Pfad zu einer Datei im Arbeitsordner, oder `None`, wenn sie ausserhalb liegt.
    pub fn resolve(&self, rel: &str) -> Option<PathBuf> {
        let root = self.root();
        if root.is_empty() {
            return None;
        }
        let full = clean_lexical(&Path::new(&root).join(rel));
        let real = dunce::canonicalize(full).ok()?;
        let real_root = dunce::canonicalize(&root).ok()?.to_string_lossy().to_lowercase();
        let r = real.to_string_lossy().to_lowercase();
        if r == real_root || r.starts_with(&format!("{real_root}{}", std::path::MAIN_SEPARATOR)) { Some(real) } else { None }
    }
}

/// Beobachtet den Ordner (rekursiv) und meldet Änderungen an den Entpreller
fn start_watcher(root: &Path, tx: Sender<Msg>) -> Option<notify::RecommendedWatcher> {
    let roots: Vec<PathBuf> = [Some(root.to_path_buf()), dunce::canonicalize(root).ok()].into_iter().flatten().collect();
    let base = root.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        for p in &event.paths {
            let Some(rel) = roots.iter().find_map(|r| p.strip_prefix(r).ok()) else { continue };
            let name = rel.to_string_lossy().to_string();
            match classify(&name) {
                Change::None => {}
                Change::Order => {
                    let _ = tx.send(Msg::Order(base.join(&name)));
                }
                // Erst melden, wenn eine Weile Ruhe ist
                Change::Files => {
                    let _ = tx.send(Msg::Files);
                }
            }
        }
    })
    .ok()?;
    watcher.watch(root, RecursiveMode::Recursive).ok()?;
    Some(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("bilder")).unwrap();
        std::fs::create_dir(root.join(".ask")).unwrap();
        std::fs::write(root.join("bilder").join("diagramm.png"), "x").unwrap();
        std::fs::write(root.join("ausgaben.csv"), "a;b").unwrap();
        std::fs::write(root.join(".ask").join("geheim.json"), "{}").unwrap();
        let w = Workspace::new();
        w.set_root(&root.to_string_lossy());
        (dir, w)
    }

    fn names(v: &Value) -> Vec<String> {
        v["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap().to_string()).collect()
    }

    fn paths(v: &[Value]) -> Vec<String> {
        v.iter().map(|e| e["path"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn ordnerinhalt_ordner_zuerst_ask_ausgeblendet_unterordner_ueber_den_relativen_pfad() {
        let (_d, w) = project();
        assert_eq!(names(&w.list("")), ["bilder", "ausgaben.csv"]);
        let sub = w.list("bilder");
        let expected = Path::new("bilder").join("diagramm.png").to_string_lossy().to_string();
        assert_eq!(sub["entries"].as_array().unwrap().iter().map(|e| e["path"].as_str().unwrap().to_string()).collect::<Vec<_>>(), [expected]);
        assert_eq!(w.list("..")["ok"], false);
    }

    #[test]
    fn suche_im_ganzen_projekt_nach_teilen_des_namens_ohne_ask() {
        let (_d, w) = project();
        assert_eq!(paths(&w.find("diag")), [Path::new("bilder").join("diagramm.png").to_string_lossy().to_string()]);
        assert!(w.find("geheim").is_empty());
        assert!(w.find("").is_empty());
    }

    #[test]
    fn hineinziehen_kopiert_in_den_gewaehlten_ordner_und_ueberschreibt_nie() {
        let (d, w) = project();
        let file = |name: &str, data: &[u8]| ImportFile { name: name.into(), kind: String::new(), data: data.to_vec() };
        let a = w.import("bilder", &[file("diagramm.png", &[1, 2])]).unwrap();
        assert_eq!(a, [Path::new("bilder").join("diagramm (2).png").to_string_lossy().to_string()]);
        assert_eq!(std::fs::read_to_string(d.path().join("bilder").join("diagramm.png")).unwrap(), "x");
        assert_eq!(w.import("", &[file("..\\..\\boese.bat", &[])]).unwrap(), ["boese.bat"]);
        assert!(w.import("..", &[file("x.txt", &[])]).is_err());
    }

    #[test]
    fn aenderungen_werden_klassifiziert() {
        let sep = std::path::MAIN_SEPARATOR;
        let p = |s: &str| s.replace('/', &sep.to_string());
        assert_eq!(classify("bericht.docx"), Change::Files);
        assert_eq!(classify(""), Change::Files);
        assert_eq!(classify(&p(".ask/chats/x.json")), Change::None);
        assert_eq!(classify(&p(".ask/skills/a/SKILL.md")), Change::None);
        assert_eq!(classify(&p(".ask/pdf/x.pdf")), Change::None);
        assert_eq!(classify(&p(".ask/Tasks.json")), Change::None);
        assert_eq!(classify(&p(".ask/auftraege/abcdef12.json")), Change::Order);
        assert_eq!(classify(&p(".ask/auftraege/zzz.json")), Change::None);
        assert_eq!(classify(&p(".ask/geheim.json")), Change::Files);
    }

    #[test]
    fn sortierung_wie_localecompare_de() {
        let mut v = vec!["b", "B", "a", "Ä", "_x", "1", "ä"];
        v.sort_by(|a, b| locale_cmp_de(a, b));
        assert_eq!(v, ["_x", "1", "a", "ä", "Ä", "b", "B"]);
    }

    fn wait_for(rx: &std::sync::mpsc::Receiver<Value>, secs: u64) -> Option<Value> {
        rx.recv_timeout(Duration::from_secs(secs)).ok()
    }

    #[test]
    fn beobachtet_dateien_und_auftraege_entprellt() {
        let dir = tempfile::tempdir().unwrap();
        let w = Workspace::new();
        let (tx, rx) = std::sync::mpsc::channel::<Value>();
        let (otx, orx) = std::sync::mpsc::channel::<Value>();
        let tx = Mutex::new(tx);
        let otx = Mutex::new(otx);
        w.events.on("files", move |v| {
            let _ = tx.lock().send(v.clone());
        });
        w.events.on("auftrag", move |v| {
            let _ = otx.lock().send(v.clone());
        });
        w.open(&dir.path().to_string_lossy()).unwrap();
        assert!(dir.path().join(".ask").is_dir());
        let first = wait_for(&rx, 5).expect("erste Liste beim Öffnen");
        assert_eq!(first["top"].as_array().unwrap().len(), 0);
        std::thread::sleep(Duration::from_millis(500));
        while rx.try_recv().is_ok() {}
        std::fs::write(dir.path().join("neu.txt"), "x").unwrap();
        let after = wait_for(&rx, 5).expect("Meldung nach Änderung");
        assert_eq!(after["top"][0]["name"], "neu.txt");
        std::fs::create_dir_all(dir.path().join(".ask").join("auftraege")).unwrap();
        // inotify (Linux) hängt die Beobachtung neuer Unterordner erst nach dem Anlegen an
        std::thread::sleep(Duration::from_millis(500));
        let order = dir.path().join(".ask").join("auftraege").join("abcdef12.json");
        std::fs::write(&order, "{}").unwrap();
        let got = wait_for(&orx, 5).expect("Auftrag gemeldet");
        assert_eq!(got.as_str().unwrap(), order.to_string_lossy());
        w.close();
        assert_eq!(w.root(), "");
    }
}
