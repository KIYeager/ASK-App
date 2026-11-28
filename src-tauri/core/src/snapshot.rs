//! Sicherung des Arbeitsordners vor jedem Auftrag (ENTSCHEIDE E24). Das Modell ändert Dateien direkt;
//! «Rückgängig» setzt den Ordner auf den Stand vor einem Auftrag zurück. Jede Dateifassung liegt nur
//! einmal in <Ablage>/objekte (Name = SHA-1 des Inhalts), ein Stand ist eine Liste
//! Pfad → Prüfsumme in <Ablage>/staende. Unveränderte Dateien (gleiche Grösse und Zeit) werden
//! nicht neu gelesen. Erfasst auch Änderungen durch Skripte, nicht nur durch write_file.
//!
//! Die Ablage liegt in der App ausserhalb des Arbeitsordners (paths::project_data_dir), damit das
//! Modell sie nicht ändern kann: Der Hauptprozess schreibt beim Zurücksetzen mit normalen Rechten.
//! Trotzdem wird jeder Stand beim Lesen geprüft (nur sichere Pfade im Ordner, nur echte Prüfsummen).
//!
//! Öffentliche Schnittstelle:
//! - `Snapshots::new(root, Option<dir>)`, `.read(id)`, `async .take(label)`, `async .diff(id)`,
//!   `async .restore(id)`, `.prune(now_ms)`, `.prune_now()`
//! - `list_files(root, limit) -> Result<BTreeMap<String, FileInfo>>`, `busy(root) -> bool`
//! - `Stand`, `StandFile`, `FileInfo`, `TakeResult`, `Diff`, `RestoreResult`
//! - `STORE`, `MAX_FILE`, `KEEP_DAYS`

use crate::safe::{is_sha1, safe_join, safe_rel};
use crate::util::{js_string, now_ms, num_of, path_key, truthy};
use anyhow::{anyhow, bail, Result};
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, UNIX_EPOCH};

pub const STORE: &str = ".ask/sicherung";
pub const MAX_FILE: u64 = 100 * 1024 * 1024; // grössere Dateien werden nicht gesichert
const MAX_FILES: usize = 5000;
pub const KEEP_DAYS: i64 = 14;
const FRESH_MS: i64 = 60 * 60 * 1000; // so junge Dateifassungen räumt prune nicht weg (laufende Sicherung)

// ~$: Sperrdateien von Office
static SKIP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(\.ask|\.git|node_modules|__pycache__|\.venv)(/|$)|(^|/)~\$").expect("gültiger Ausdruck"));
static ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]+$").expect("gültiger Ausdruck"));

const DAMAGED: &str = "Die Sicherung ist beschädigt.";

fn sha1_hex(data: &[u8]) -> String {
    hex::encode(Sha1::digest(data))
}

fn to_base36(mut n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let mut digits = Vec::new();
    while n > 0 {
        digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[(n % 36) as usize]);
        n /= 36;
    }
    digits.reverse();
    String::from_utf8(digits).unwrap_or_default()
}

fn new_id() -> String {
    let tag: [u8; 3] = rand::random();
    format!("{}-{}", to_base36(now_ms().max(0) as u64), hex::encode(tag))
}

fn mtime_ms(meta: &std::fs::Metadata) -> f64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs() as f64 * 1000.0 + f64::from(d.subsec_nanos()) / 1e6)
}

// Ordner, in denen gerade gesichert wird: prune wartet dann (workspace.rs)
static BUSY_ROOTS: Mutex<Option<HashMap<String, u32>>> = Mutex::new(None);

/// Wird in diesem Ordner gerade gesichert?
pub fn busy(root: &Path) -> bool {
    let key = path_key(&root.to_string_lossy());
    BUSY_ROOTS.lock().as_ref().and_then(|m| m.get(&key)).is_some_and(|n| *n > 0)
}

/// Merkt einen laufenden Auftrag, solange der Wächter lebt
struct BusyGuard(String);

impl BusyGuard {
    fn new(root: &Path) -> Self {
        let key = path_key(&root.to_string_lossy());
        *BUSY_ROOTS.lock().get_or_insert_with(HashMap::new).entry(key.clone()).or_insert(0) += 1;
        BusyGuard(key)
    }
}

impl Drop for BusyGuard {
    fn drop(&mut self) {
        let mut guard = BUSY_ROOTS.lock();
        if let Some(map) = guard.as_mut() {
            if let Some(n) = map.get_mut(&self.0) {
                *n = n.saturating_sub(1);
                if *n == 0 {
                    map.remove(&self.0);
                }
            }
        }
    }
}

/// Grösse und Änderungszeit (Millisekunden) einer Datei im Ordner
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FileInfo {
    pub size: u64,
    pub mtime: f64,
}

/// Eine gesicherte Datei im Stand
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StandFile {
    pub size: u64,
    pub mtime: f64,
    pub hash: String,
}

/// Ein Stand, wie er als JSON in <Ablage>/staende/<id>.json liegt
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stand {
    pub id: String,
    pub time: i64,
    pub label: String,
    pub files: BTreeMap<String, StandFile>,
    pub skipped: Vec<String>,
}

/// Ergebnis von `take`: `{ id, count, skipped }` (skipped: zu gross oder gesperrt)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TakeResult {
    pub id: String,
    pub count: usize,
    pub skipped: Vec<String>,
}

/// Was hat sich seit einem Stand geändert? (Pfade)
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Diff {
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
}

/// Ergebnis von `restore`: `{ changed, added, removed, failed, safety }`
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct RestoreResult {
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub removed: Vec<String>,
    pub failed: Vec<String>,
    pub safety: Option<String>,
}

/// Dateien im Ordner: `pfad/datei` → Grösse und Zeit, mit / als Trenner.
/// limit: Sichern bricht bei zu vielen Dateien ab; Vergleichen und Zurücksetzen nicht (`None`), sonst ginge
/// «Rückgängig» nicht mehr, sobald ein Auftrag viele Dateien erzeugt hat.
pub fn list_files(root: &Path, limit: Option<usize>) -> Result<BTreeMap<String, FileInfo>> {
    let mut out = BTreeMap::new();
    let mut count = 0usize;
    walk(root, "", limit, &mut count, &mut out)?;
    Ok(out)
}

fn walk(dir: &Path, prefix: &str, limit: Option<usize>, count: &mut usize, out: &mut BTreeMap<String, FileInfo>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Ok(()) };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let rel = if prefix.is_empty() { name } else { format!("{prefix}/{name}") };
        if SKIP.is_match(&rel) {
            continue;
        }
        let Ok(file_type) = entry.file_type() else { continue };
        if file_type.is_dir() {
            walk(&entry.path(), &rel, limit, count, out)?;
        } else if file_type.is_file() {
            *count += 1;
            if limit.is_some_and(|l| *count > l) {
                bail!("Mehr als {MAX_FILES} Dateien im Ordner, die App sichert ihn nicht.");
            }
            // gerade gelöscht: überspringen
            if let Ok(meta) = std::fs::metadata(entry.path()) {
                out.insert(rel, FileInfo { size: meta.len(), mtime: mtime_ms(&meta) });
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct Snapshots {
    root: PathBuf,
    dir: PathBuf,
    /// letzter Stand, für Prüfsummen unveränderter Dateien
    last: Arc<Mutex<Option<Stand>>>,
}

impl Snapshots {
    /// dir: Ablage der Sicherung; ohne Angabe .ask/sicherung im Ordner (nur für Tests)
    pub fn new(root: impl Into<PathBuf>, dir: Option<PathBuf>) -> Self {
        let root = root.into();
        let dir = dir.unwrap_or_else(|| root.join(".ask").join("sicherung"));
        Snapshots { root, dir, last: Arc::new(Mutex::new(None)) }
    }

    pub fn object_path(&self, hash: &str) -> Result<PathBuf> {
        if !is_sha1(hash) {
            bail!("Ungültige Prüfsumme in der Sicherung");
        }
        Ok(self.dir.join("objekte").join(&hash[..2]).join(hash))
    }

    pub fn stand_path(&self, id: &str) -> Result<PathBuf> {
        if !ID.is_match(id) {
            bail!("Ungültiger Stand");
        }
        Ok(self.dir.join("staende").join(format!("{id}.json")))
    }

    /// Liest einen Stand und prüft ihn: Ein veränderter Stand darf den Hauptprozess nicht dazu bringen,
    /// ausserhalb des Ordners zu schreiben oder fremde Dateien als «Dateifassung» zu kopieren.
    pub fn read(&self, id: &str) -> Result<Stand> {
        let text = std::fs::read_to_string(self.stand_path(id)?)?;
        let value: Value = serde_json::from_str(&text).map_err(|_| anyhow!(DAMAGED))?;
        checked(&value)
    }

    fn newest(&self) -> Option<Stand> {
        let dir = self.dir.join("staende");
        let mut stands: Vec<(PathBuf, f64)> = Vec::new();
        for entry in std::fs::read_dir(&dir).ok()?.flatten() {
            let meta = std::fs::metadata(entry.path()).ok()?;
            stands.push((entry.path(), mtime_ms(&meta)));
        }
        stands.sort_by(|a, b| b.1.total_cmp(&a.1));
        let (path, _) = stands.into_iter().next()?;
        let value: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        checked(&value).ok()
    }

    /// Prüfsumme einer Datei; known: Eintrag eines früheren Stands, gilt bei gleicher Grösse und Zeit.
    fn hash_of(&self, rel: &str, info: &FileInfo, known: Option<&StandFile>, store: bool) -> Result<String> {
        if let Some(k) = known {
            if k.size == info.size && k.mtime == info.mtime && self.object_path(&k.hash)?.exists() {
                return Ok(k.hash.clone());
            }
        }
        let data = std::fs::read(self.root.join(rel))?;
        let hash = sha1_hex(&data);
        if store {
            let target = self.object_path(&hash)?;
            if !target.exists() {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let tmp = PathBuf::from(format!("{}.{}.tmp", target.display(), std::process::id()));
                std::fs::write(&tmp, &data)?;
                std::fs::rename(&tmp, &target)?;
            }
        }
        Ok(hash)
    }

    /// Stand des Ordners sichern. Liefert `{ id, count, skipped }` (skipped: zu gross oder gesperrt).
    pub async fn take(&self, label: &str) -> Result<TakeResult> {
        let this = self.clone();
        let label = label.to_string();
        tokio::task::spawn_blocking(move || this.take_marked(&label)).await?
    }

    fn take_marked(&self, label: &str) -> Result<TakeResult> {
        let _busy = BusyGuard::new(&self.root);
        self.take_now(label)
    }

    fn take_now(&self, label: &str) -> Result<TakeResult> {
        let current = list_files(&self.root, Some(MAX_FILES))?;
        let before: BTreeMap<String, StandFile> = {
            let mut last = self.last.lock();
            if last.is_none() {
                *last = self.newest(); // nach einem Neustart: Prüfsummen des letzten Stands
            }
            last.as_ref().map(|s| s.files.clone()).unwrap_or_default()
        };
        let mut files = BTreeMap::new();
        let mut skipped = Vec::new();
        for (rel, info) in &current {
            if info.size > MAX_FILE || !safe_rel(rel) {
                skipped.push(rel.clone());
                continue;
            }
            match self.hash_of(rel, info, before.get(rel), true) {
                Ok(hash) => {
                    files.insert(rel.clone(), StandFile { size: info.size, mtime: info.mtime, hash });
                }
                // gesperrt, z. B. von Office exklusiv geöffnet
                Err(_) => skipped.push(rel.clone()),
            }
        }
        let stand = Stand { id: new_id(), time: now_ms(), label: label.to_string(), files, skipped };
        let path = self.stand_path(&stand.id)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string(&stand)?)?;
        let result = TakeResult { id: stand.id.clone(), count: stand.files.len(), skipped: stand.skipped.clone() };
        *self.last.lock() = Some(stand);
        Ok(result)
    }

    /// Was hat sich seit einem Stand geändert? `{ changed, added, removed }` (Pfade)
    pub async fn diff(&self, id: &str) -> Result<Diff> {
        let this = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || this.diff_now(&id)).await?
    }

    fn diff_now(&self, id: &str) -> Result<Diff> {
        let stand = self.read(id)?;
        self.diff_stand(&stand)
    }

    fn diff_stand(&self, stand: &Stand) -> Result<Diff> {
        let current = list_files(&self.root, None)?;
        let mut out = Diff::default();
        for (rel, info) in &current {
            let Some(old) = stand.files.get(rel) else {
                if !stand.skipped.contains(rel) {
                    out.added.push(rel.clone());
                }
                continue;
            };
            if old.size == info.size && old.mtime == info.mtime {
                continue;
            }
            match self.hash_of(rel, info, None, false) {
                Ok(hash) if hash == old.hash => {}
                Ok(_) => out.changed.push(rel.clone()),
                // nicht lesbar: als geändert melden, Wiederherstellen versucht es
                Err(_) => out.changed.push(rel.clone()),
            }
        }
        for rel in stand.files.keys() {
            if !current.contains_key(rel) {
                out.removed.push(rel.clone());
            }
        }
        Ok(out)
    }

    /// Ordner auf einen Stand zurücksetzen. Vorher wird der jetzige Stand gesichert.
    pub async fn restore(&self, id: &str) -> Result<RestoreResult> {
        let this = self.clone();
        let id = id.to_string();
        tokio::task::spawn_blocking(move || this.restore_now(&id)).await?
    }

    fn restore_now(&self, id: &str) -> Result<RestoreResult> {
        let stand = self.read(id)?;
        let d = self.diff_stand(&stand)?;
        // zu viele Dateien: ohne Zwischensicherung zurücksetzen
        let safety = self.take_marked("vor Rückgängig").ok();
        let mut failed = Vec::new();
        for rel in &d.added {
            let removed = (|| -> Result<()> {
                match std::fs::remove_file(safe_join(&self.root, rel)?) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                    _ => {}
                }
                self.remove_empty_parents(rel)
            })();
            if removed.is_err() {
                failed.push(rel.clone());
            }
        }
        for rel in d.changed.iter().chain(d.removed.iter()) {
            let restored = (|| -> Result<()> {
                let target = safe_join(&self.root, rel)?;
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let file = stand.files.get(rel).ok_or_else(|| anyhow!(DAMAGED))?;
                std::fs::copy(self.object_path(&file.hash)?, &target)?;
                let t = UNIX_EPOCH + Duration::try_from_secs_f64((file.mtime / 1000.0).max(0.0)).unwrap_or_default();
                // Inhalt ist zurück; scheitert nur das Datum, gilt die Datei trotzdem als wiederhergestellt
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&target) {
                    let _ = f.set_modified(t);
                }
                Ok(())
            })();
            if restored.is_err() {
                failed.push(rel.clone()); // z. B. in Word geöffnet
            }
        }
        *self.last.lock() = None;
        Ok(RestoreResult { changed: d.changed, added: d.added, removed: d.removed, failed, safety: safety.map(|s| s.id) })
    }

    fn remove_empty_parents(&self, rel: &str) -> Result<()> {
        let mut dir = self.root.join(rel).parent().map(Path::to_path_buf);
        while let Some(d) = dir {
            if d == self.root || !d.starts_with(&self.root) {
                break;
            }
            if std::fs::read_dir(&d)?.next().is_some() {
                return Ok(());
            }
            std::fs::remove_dir(&d)?;
            dir = d.parent().map(Path::to_path_buf);
        }
        Ok(())
    }

    /// Stände älter als KEEP_DAYS löschen, danach Dateifassungen, die kein Stand mehr braucht.
    pub fn prune_now(&self) -> Result<usize> {
        self.prune(now_ms())
    }

    /// Wie `prune_now`, mit einer vorgegebenen Zeit (Millisekunden seit 1970). Liefert die Zahl gelöschter Dateifassungen.
    pub fn prune(&self, now: i64) -> Result<usize> {
        let limit = now - KEEP_DAYS * 24 * 60 * 60 * 1000;
        let stand_dir = self.dir.join("staende");
        let mut used: HashSet<String> = HashSet::new();
        let Ok(names) = std::fs::read_dir(&stand_dir) else { return Ok(0) };
        for entry in names.flatten() {
            let file = entry.path();
            let parsed: Result<()> = (|| {
                let stand: Value = serde_json::from_str(&std::fs::read_to_string(&file)?)?;
                if stand.get("time").and_then(Value::as_f64).is_some_and(|t| t < limit as f64) {
                    std::fs::remove_file(&file)?;
                    return Ok(());
                }
                let files = stand.get("files").and_then(Value::as_object).ok_or_else(|| anyhow!(DAMAGED))?;
                for f in files.values() {
                    let f = f.as_object().ok_or_else(|| anyhow!(DAMAGED))?;
                    if let Some(hash) = f.get("hash").and_then(Value::as_str) {
                        used.insert(hash.to_string());
                    }
                }
                Ok(())
            })();
            if parsed.is_err() {
                let _ = std::fs::remove_file(&file); // unlesbar: weg
            }
        }
        let mut removed = 0;
        let obj_dir = self.dir.join("objekte");
        if obj_dir.exists() {
            for sub in std::fs::read_dir(&obj_dir)?.flatten() {
                let sub_path = sub.path();
                for name in std::fs::read_dir(&sub_path)?.flatten() {
                    if used.contains(name.file_name().to_string_lossy().as_ref()) {
                        continue;
                    }
                    let file = name.path();
                    // Eine gerade laufende Sicherung hat ihre Fassungen schon abgelegt, den Stand aber noch nicht
                    match std::fs::metadata(&file) {
                        Ok(meta) if (now as f64) - mtime_ms(&meta) >= FRESH_MS as f64 => {}
                        _ => continue,
                    }
                    let _ = std::fs::remove_file(&file);
                    removed += 1;
                }
                if std::fs::read_dir(&sub_path)?.next().is_none() {
                    std::fs::remove_dir(&sub_path)?;
                }
            }
        }
        let mut last = self.last.lock();
        if last.as_ref().is_some_and(|s| s.time < limit) {
            *last = None;
        }
        Ok(removed)
    }
}

/// Prüft einen gelesenen Stand und macht daraus einen `Stand`
fn checked(v: &Value) -> Result<Stand> {
    let damaged = || anyhow!(DAMAGED);
    let obj = v.as_object().ok_or_else(damaged)?;
    let files = obj.get("files").and_then(Value::as_object).ok_or_else(damaged)?;
    let mut out = BTreeMap::new();
    for (rel, f) in files {
        if !safe_rel(rel) || SKIP.is_match(rel) || !truthy(f) {
            return Err(damaged());
        }
        let hash = js_string(f.get("hash").unwrap_or(&Value::Null));
        if !is_sha1(&hash) {
            return Err(damaged());
        }
        out.insert(rel.clone(), StandFile { size: num_of(f, "size").max(0.0) as u64, mtime: num_of(f, "mtime"), hash });
    }
    let skipped = match obj.get("skipped") {
        Some(Value::Array(a)) => a.iter().map(js_string).collect(),
        _ => Vec::new(),
    };
    Ok(Stand {
        id: js_string(obj.get("id").unwrap_or(&Value::Null)),
        time: num_of(v, "time") as i64,
        label: js_string(obj.get("label").unwrap_or(&Value::Null)),
        files: out,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::SystemTime;

    struct Folder {
        dir: tempfile::TempDir,
    }

    impl Folder {
        fn new() -> Self {
            Folder { dir: tempfile::tempdir().unwrap() }
        }
        fn root(&self) -> &Path {
            self.dir.path()
        }
        fn write(&self, rel: &str, text: &str) {
            let p = self.root().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        fn read(&self, rel: &str) -> String {
            std::fs::read_to_string(self.root().join(rel)).unwrap()
        }
        fn exists(&self, rel: &str) -> bool {
            self.root().join(rel).exists()
        }
        fn set_mtime(&self, rel: &str, t: SystemTime) {
            std::fs::OpenOptions::new().write(true).open(self.root().join(rel)).unwrap().set_modified(t).unwrap();
        }
    }

    fn objects(root: &Path) -> usize {
        let dir = root.join(".ask/sicherung/objekte");
        if !dir.exists() {
            return 0;
        }
        std::fs::read_dir(dir).unwrap().map(|d| std::fs::read_dir(d.unwrap().path()).unwrap().count()).sum()
    }

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[tokio::test]
    async fn sichern_aenderungen_erkennen_zuruecksetzen() {
        let f = Folder::new();
        f.write("bericht.docx", "Fassung 1");
        f.write("daten/liste.csv", "a;b");
        f.write(".ask/chats/x.json", "{}");
        f.write(".git/HEAD", "ref");
        f.write("~$bericht.docx", "Sperre");
        let s = Snapshots::new(f.root(), None);
        let before = s.take("").await.unwrap();
        assert_eq!(before.count, 2, ".ask, .git und Office-Sperrdateien nicht gesichert");

        // Auftrag: ändern, neu anlegen (auch in neuem Ordner), löschen
        f.write("bericht.docx", "Fassung 2 vom Modell");
        f.write("neu/spiel.html", "<html>");
        std::fs::remove_file(f.root().join("daten/liste.csv")).unwrap();
        let d = s.diff(&before.id).await.unwrap();
        assert_eq!(d, Diff { changed: strings(&["bericht.docx"]), added: strings(&["neu/spiel.html"]), removed: strings(&["daten/liste.csv"]) });

        let r = s.restore(&before.id).await.unwrap();
        assert!(r.failed.is_empty());
        assert_eq!(f.read("bericht.docx"), "Fassung 1");
        assert_eq!(f.read("daten/liste.csv"), "a;b");
        assert!(!f.exists("neu/spiel.html") && !f.exists("neu"), "neue Datei und leerer Ordner weg");
        assert!(f.exists(".ask/chats/x.json") && f.exists(".git/HEAD"));
        assert_eq!(s.diff(&before.id).await.unwrap(), Diff::default());

        // Rückgängig lässt sich rückgängig machen
        s.restore(r.safety.as_deref().unwrap()).await.unwrap();
        assert_eq!(f.read("bericht.docx"), "Fassung 2 vom Modell");
        assert!(f.exists("neu/spiel.html"));
    }

    #[tokio::test]
    async fn jede_fassung_nur_einmal_gespeichert_unveraenderte_dateien_nicht_neu_gelesen() {
        let f = Folder::new();
        f.write("a.txt", "gleich");
        f.write("b.txt", "gleich");
        f.write("c.txt", "anders");
        let s = Snapshots::new(f.root(), None);
        s.take("").await.unwrap();
        assert_eq!(objects(f.root()), 2);
        s.take("").await.unwrap();
        Snapshots::new(f.root(), None).take("").await.unwrap(); // nach Neustart
        assert_eq!(objects(f.root()), 2);
        f.write("c.txt", "neu");
        s.take("").await.unwrap();
        assert_eq!(objects(f.root()), 3);
        // gleiche Grösse, anderer Inhalt und andere Zeit: diff liest die Datei nach
        let id = s.take("").await.unwrap().id;
        let modified = std::fs::metadata(f.root().join("a.txt")).unwrap().modified().unwrap();
        f.write("a.txt", "GLEICH");
        f.set_mtime("a.txt", modified + Duration::from_secs(5));
        assert_eq!(s.diff(&id).await.unwrap().changed, ["a.txt"]);
        // nur berührt (neue Zeit, gleicher Inhalt): keine Änderung
        f.set_mtime("b.txt", modified + Duration::from_secs(9));
        assert_eq!(s.diff(&id).await.unwrap().changed, ["a.txt"]);
    }

    #[tokio::test]
    async fn alte_staende_und_nicht_mehr_gebrauchte_fassungen_werden_geloescht() {
        let f = Folder::new();
        f.write("a.txt", "eins");
        let s = Snapshots::new(f.root(), None);
        let old = s.take("").await.unwrap();
        f.write("a.txt", "zweite Fassung"); // andere Länge: gleich lang und gleiche Zeitmarke gilt als unverändert
        let now = s.take("").await.unwrap();
        assert_eq!(objects(f.root()), 2);
        let file = f.root().join(format!(".ask/sicherung/staende/{}.json", old.id));
        let mut stand: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        stand["time"] = json!(now_ms() - 15 * 24 * 3600 * 1000);
        std::fs::write(&file, stand.to_string()).unwrap();
        assert_eq!(s.prune_now().unwrap(), 0); // frisch abgelegte Fassungen bleiben (laufende Sicherung)
        assert_eq!(s.prune(now_ms() + 2 * 3600 * 1000).unwrap(), 1);
        assert_eq!(objects(f.root()), 1);
        assert!(!file.exists());
        assert_eq!(s.diff(&now.id).await.unwrap(), Diff::default());
    }

    #[tokio::test]
    async fn veraenderter_stand_kein_schreiben_ausserhalb_keine_fremden_dateien() {
        let f = Folder::new();
        let outside = tempfile::tempdir().unwrap();
        f.write("a.txt", "eins");
        let s = Snapshots::new(f.root(), None);
        let stand = s.take("").await.unwrap();
        let file = f.root().join(format!(".ask/sicherung/staende/{}.json", stand.id));
        let original = std::fs::read_to_string(&file).unwrap();
        let parsed: Value = serde_json::from_str(&original).unwrap();
        let hash = parsed["files"].as_object().unwrap().values().next().unwrap()["hash"].as_str().unwrap().to_string();
        let escape = format!("../{}/evil.bat", outside.path().file_name().unwrap().to_string_lossy());
        let cases = [
            (escape.as_str(), hash.as_str()),
            ("..\\evil.bat", hash.as_str()),
            ("C:/evil.bat", hash.as_str()),
            ("/evil.bat", hash.as_str()),
            (".ask/chats/x.json", hash.as_str()),
            ("ok.txt", "../../../../etc/passwd"),
            ("ok.txt", "zz"),
        ];
        for (rel, h) in cases {
            let mut bad = parsed.clone();
            bad["files"][rel] = json!({ "size": 4, "mtime": 1, "hash": h });
            std::fs::write(&file, bad.to_string()).unwrap();
            let e = s.diff(&stand.id).await.unwrap_err();
            assert!(e.to_string().contains("beschädigt"), "{rel}: {e}");
            let e = s.restore(&stand.id).await.unwrap_err();
            assert!(e.to_string().contains("beschädigt"), "{rel}: {e}");
        }
        assert!(!outside.path().join("evil.bat").exists());
        std::fs::write(&file, &original).unwrap();
        f.write("a.txt", "zwei!");
        assert!(s.restore(&stand.id).await.unwrap().failed.is_empty());
        assert_eq!(f.read("a.txt"), "eins");
    }

    #[tokio::test]
    async fn ablage_ausserhalb_des_ordners() {
        let f = Folder::new();
        let store = tempfile::tempdir().unwrap();
        f.write("a.txt", "eins");
        let s = Snapshots::new(f.root(), Some(store.path().to_path_buf()));
        let stand = s.take("").await.unwrap();
        assert!(store.path().join("staende").join(format!("{}.json", stand.id)).exists());
        assert!(!f.exists(".ask/sicherung"));
        f.write("a.txt", "zwei!");
        s.restore(&stand.id).await.unwrap();
        assert_eq!(f.read("a.txt"), "eins");
    }

    #[tokio::test]
    async fn laufende_sicherung_wird_als_beschaeftigt_gemeldet() {
        let f = Folder::new();
        f.write("a.txt", "eins");
        assert!(!busy(f.root()));
        {
            let _guard = BusyGuard::new(f.root());
            assert!(busy(f.root()));
        }
        assert!(!busy(f.root()));
    }

    #[tokio::test]
    async fn zu_viele_dateien_brechen_das_sichern_ab_vergleichen_nicht() {
        let f = Folder::new();
        for i in 0..4 {
            f.write(&format!("d/{i}.txt"), "x");
        }
        assert!(list_files(f.root(), Some(3)).unwrap_err().to_string().contains("Mehr als 5000 Dateien"));
        assert_eq!(list_files(f.root(), None).unwrap().len(), 4);
    }
}
