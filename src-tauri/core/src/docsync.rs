//! Geteilte Docs (ENTSCHEIDE E82): Mehrere Personen schreiben am selben Doc, jede auf ihrem Laptop, ohne eigenen
//! Server. Ausgetauscht wird über einen Ordner, den alle haben: OneDrive oder Teams (synchronisiert) oder ein Laufwerk
//! der Schule. Dort liegt pro Doc ein Ordner `<name>.askdoc` mit `doc.json`, `teilnehmer/<kennung>.jsonl` und `bilder/`.
//!
//! Damit OneDrive nie Konfliktkopien macht, schreibt jede Person nur in ihre eigene Datei, und zwar nur am Ende
//! (Protokoll der Änderungen). Jede App liest die Protokolle aller und setzt daraus dasselbe Doc zusammen, egal in
//! welcher Reihenfolge die Dateien ankommen: Das Doc ist eine Folge von Absätzen (Blöcke wie in Markdown), jeder mit
//! fester Kennung. Ein neuer Absatz steht «nach» einem anderen (Reihenfolge wie RGA: unter demselben Vorgänger kommt der
//! neuere zuerst), Text ändern gilt pro Absatz mit Lamport-Zeit (die spätere Fassung gilt), Löschen ist endgültig.
//! Ändern zwei Personen gleichzeitig denselben Absatz, gilt die spätere Fassung; verschiedene Absätze gehen nie verloren.
//!
//! Im Projekt bleibt das Doc eine normale Datei `docs/….md`: Editor, Werkzeuge von ASK und Export arbeiten wie immer.
//! Alle 1,5 Sekunden vergleicht die App die Datei mit dem letzten Stand (eigene Änderungen werden Einträge im eigenen
//! Protokoll) und liest Neues aus den anderen Protokollen (dann schreibt sie die Datei neu). Der Editor speichert mit
//! der Prüfsumme des Stands, den er bearbeitet hat; seine Änderung wird gegen genau diesen Stand verglichen, darum gehen
//! Änderungen anderer, die inzwischen kamen, nicht verloren.
//!
//! Sicherheit: Der geteilte Ordner darf nicht in einem Projekt liegen (das Terminal mit niedriger Integritätsstufe könnte
//! sonst Einträge im Namen anderer schreiben). Was aus dem Ordner kommt, ist nur Text und Bilder (geprüfte Namen und
//! Endungen, Grösse begrenzt); die Kommentare bleiben bei jeder Person (docs.rs).
//!
//! Öffentliche Schnittstelle:
//! - `Op`, `Crdt` (`apply`, `visible`, `render`), `split_blocks`, `diff_ops`
//! - `Shared` (`create`, `open`, `edit`, `tick`, `info`, `recent_by_others`)
//! - Verwaltung: `start(notify)`, `share`, `join`, `unshare`, `find`, `save`, `info`, `prompt_note`, `check_place`

use crate::safe::{safe_join, safe_rel};
use crate::settings;
use crate::util::{now_ms, same_path, str_of, take_chars};
use anyhow::{bail, Result};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

/// Endung des geteilten Ordners eines Docs
pub const EXT: &str = "askdoc";
const LOGS: &str = "teilnehmer";
const IMAGES: &str = "bilder";
/// Abstand der Abgleiche
pub const TICK_MS: u64 = 1500;
const MAX_BLOCKS: usize = 5000;
const MAX_LOG: u64 = 50_000_000;
const MAX_IMAGE: u64 = 20_000_000;
const KEEP_VERSIONS: usize = 40;
/// Einstellung mit den geteilten Docs dieses Laptops
const SETTING: &str = "geteilteDocs";

/// Kennung eines Absatzes oder einer Fassung: (Lamport-Zeit, Kennung der Person)
pub type Id = (u64, String);

/// Ein Eintrag im Protokoll einer Person (eine Zeile JSON)
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    /// Wer schreibt (Name zum Anzeigen)
    Hallo { wer: String, name: String, zeit: i64 },
    /// Neuer Absatz nach `nach` (None: am Anfang)
    Neu { id: Id, nach: Option<Id>, text: String, zeit: i64 },
    /// Neuer Text eines Absatzes; `ts` ist die Zeit dieser Fassung
    Text { id: Id, ts: Id, text: String, zeit: i64 },
    /// Absatz gelöscht
    Weg { id: Id, zeit: i64 },
}

// ---------- Zusammenführen (CRDT) ----------

/// Stand eines geteilten Docs aus den Protokollen aller
#[derive(Default, Debug)]
pub struct Crdt {
    place: HashMap<Id, Option<Id>>,
    /// Nachfolger je Vorgänger, neueste zuerst
    children: HashMap<Option<Id>, Vec<Id>>,
    text: HashMap<Id, (Id, String)>,
    deleted: HashSet<Id>,
    /// Letzte Änderung je Absatz: (Person, Zeit)
    author: HashMap<Id, (String, i64)>,
    pub clock: u64,
    /// Namen der Personen
    pub names: HashMap<String, String>,
    /// Letzte Änderung je Person
    pub seen: HashMap<String, i64>,
}

impl Crdt {
    fn see(&mut self, who: &str, zeit: i64) {
        let e = self.seen.entry(who.to_string()).or_insert(0);
        *e = (*e).max(zeit);
    }

    /// Eintrag anwenden; true, wenn sich der sichtbare Text ändern kann. Mehrfach anwenden schadet nicht.
    pub fn apply(&mut self, op: &Op) -> bool {
        match op {
            Op::Hallo { wer, name, zeit } => {
                self.names.insert(wer.clone(), take_chars(name.trim(), 60));
                self.see(wer, *zeit);
                false
            }
            Op::Neu { id, nach, text, zeit } => {
                self.clock = self.clock.max(id.0);
                if self.place.contains_key(id) || self.place.len() >= MAX_BLOCKS {
                    return false;
                }
                self.place.insert(id.clone(), nach.clone());
                let sib = self.children.entry(nach.clone()).or_default();
                let pos = sib.iter().position(|x| x < id).unwrap_or(sib.len());
                sib.insert(pos, id.clone());
                if !self.text.get(id).is_some_and(|(ts, _)| ts > id) {
                    self.text.insert(id.clone(), (id.clone(), text.clone()));
                    self.author.insert(id.clone(), (id.1.clone(), *zeit));
                }
                self.see(&id.1, *zeit);
                true
            }
            Op::Text { id, ts, text, zeit } => {
                self.clock = self.clock.max(ts.0);
                self.see(&ts.1, *zeit);
                if self.text.get(id).is_some_and(|(t, _)| t >= ts) {
                    return false;
                }
                self.text.insert(id.clone(), (ts.clone(), text.clone()));
                self.author.insert(id.clone(), (ts.1.clone(), *zeit));
                self.place.contains_key(id)
            }
            Op::Weg { id, .. } => self.deleted.insert(id.clone()) && self.place.contains_key(id),
        }
    }

    /// Sichtbare Absätze in der Reihenfolge des Docs
    pub fn visible(&self) -> Vec<(Id, String)> {
        let mut out = Vec::new();
        let mut stack: Vec<Id> = self.children.get(&None).map(|c| c.iter().rev().cloned().collect()).unwrap_or_default();
        while let Some(id) = stack.pop() {
            if !self.deleted.contains(&id) {
                out.push((id.clone(), self.text.get(&id).map(|t| t.1.clone()).unwrap_or_default()));
            }
            if let Some(c) = self.children.get(&Some(id)) {
                stack.extend(c.iter().rev().cloned());
            }
        }
        out
    }

    /// Das Doc als Markdown
    pub fn render(&self) -> String {
        join_blocks(&self.visible().into_iter().map(|b| b.1).collect::<Vec<_>>())
    }

    /// Wer einen Absatz zuletzt geändert hat
    pub fn author_of(&self, id: &Id) -> Option<&(String, i64)> {
        self.author.get(id)
    }
}

/// Markdown in Absätze: an Leerzeilen, aber nie in einem Codeblock
pub fn split_blocks(md: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut fence: Option<&str> = None;
    for line in md.lines() {
        let t = line.trim_start();
        if let Some(f) = fence {
            cur.push(line);
            if t.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if t.starts_with("```") || t.starts_with("~~~") {
            fence = Some(&t[..3]);
            cur.push(line);
            continue;
        }
        if line.trim().is_empty() {
            if !cur.is_empty() {
                out.push(cur.join("\n").trim_end().to_string());
                cur.clear();
            }
            continue;
        }
        cur.push(line);
    }
    if !cur.is_empty() {
        out.push(cur.join("\n").trim_end().to_string());
    }
    out
}

fn join_blocks(blocks: &[String]) -> String {
    if blocks.is_empty() { String::new() } else { format!("{}\n", blocks.join("\n\n")) }
}

/// Einträge, die aus dem Stand `base` (Absätze mit Kennung) den neuen Text machen. Gleiche Absätze bleiben, an
/// derselben Stelle geänderte werden «Text», übrige werden gelöscht oder neu eingefügt.
pub fn diff_ops(base: &[(Id, String)], new: &[String], me: &str, clock: &mut u64, zeit: i64) -> Vec<Op> {
    let mut ops = Vec::new();
    let (n, m) = (base.len(), new.len());
    // Gleicher Anfang und gleiches Ende zuerst (schnell und meist schon fast alles)
    let mut pre = 0;
    while pre < n && pre < m && base[pre].1 == new[pre] {
        pre += 1;
    }
    let mut suf = 0;
    while suf < n - pre && suf < m - pre && base[n - 1 - suf].1 == new[m - 1 - suf] {
        suf += 1;
    }
    let (bo, bn) = (&base[pre..n - suf], &new[pre..m - suf]);
    // Dazwischen: längste gemeinsame Folge, bei sehr vielen Absätzen alles als eine Lücke
    let mut pairs: Vec<(Option<usize>, Option<usize>)> = Vec::new(); // (alt, neu) in Reihenfolge
    if bo.len() * bn.len() <= 4_000_000 {
        let mut dp = vec![vec![0u32; bn.len() + 1]; bo.len() + 1];
        for i in (0..bo.len()).rev() {
            for j in (0..bn.len()).rev() {
                dp[i][j] = if bo[i].1 == bn[j] { dp[i + 1][j + 1] + 1 } else { dp[i + 1][j].max(dp[i][j + 1]) };
            }
        }
        let (mut i, mut j) = (0, 0);
        while i < bo.len() || j < bn.len() {
            if i < bo.len() && j < bn.len() && bo[i].1 == bn[j] {
                pairs.push((Some(i), Some(j)));
                i += 1;
                j += 1;
            } else if j < bn.len() && (i == bo.len() || dp[i][j + 1] >= dp[i + 1][j]) {
                pairs.push((None, Some(j)));
                j += 1;
            } else {
                pairs.push((Some(i), None));
                i += 1;
            }
        }
    } else {
        pairs.extend((0..bo.len()).map(|i| (Some(i), None)));
        pairs.extend((0..bn.len()).map(|j| (None, Some(j))));
    }
    let mut prev: Option<Id> = if pre > 0 { Some(base[pre - 1].0.clone()) } else { None };
    let mut k = 0;
    while k < pairs.len() {
        if let (Some(i), Some(_)) = pairs[k] {
            prev = Some(bo[i].0.clone());
            k += 1;
            continue;
        }
        // Lücke bis zum nächsten gleichen Absatz: gelöschte und neue Absätze paarweise als Änderung
        let mut gone = Vec::new();
        let mut added = Vec::new();
        while k < pairs.len() && !matches!(pairs[k], (Some(_), Some(_))) {
            match pairs[k] {
                (Some(i), None) => gone.push(i),
                (None, Some(j)) => added.push(j),
                _ => {}
            }
            k += 1;
        }
        let pair = gone.len().min(added.len());
        for t in 0..pair {
            *clock += 1;
            let id = bo[gone[t]].0.clone();
            ops.push(Op::Text { id: id.clone(), ts: (*clock, me.to_string()), text: bn[added[t]].clone(), zeit });
            prev = Some(id);
        }
        for &i in &gone[pair..] {
            ops.push(Op::Weg { id: bo[i].0.clone(), zeit });
        }
        for &j in &added[pair..] {
            *clock += 1;
            let id: Id = (*clock, me.to_string());
            ops.push(Op::Neu { id: id.clone(), nach: prev.clone(), text: bn[j].clone(), zeit });
            prev = Some(id);
        }
    }
    ops
}

// ---------- Ablage: Ordner oder OneDrive ----------

/// Wo die Protokolle liegen: ein Ordner auf dem Laptop (`FolderStore`, E82) oder ein Ordner in OneDrive, geöffnet über
/// einen Link (`msgraph::GraphStore`, E83). Jede Person schreibt nur ihre eigene Datei und nur ans Ende.
pub trait Store: Send {
    /// Kurz für die Anzeige: Ordner oder «OneDrive (Link)»
    fn describe(&self) -> String;
    /// Link zum Weitergeben (nur OneDrive)
    fn link(&self) -> Option<String> {
        None
    }
    /// Gibt es dort ein geteiltes Doc?
    fn check(&mut self) -> Result<()>;
    /// Protokolle: (Kennung der Person, Grösse in Bytes)
    fn logs(&mut self) -> Result<Vec<(String, u64)>>;
    /// Protokoll einer Person ab Byte `from`
    fn read_log(&mut self, who: &str, from: u64) -> Result<Vec<u8>>;
    /// Ans eigene Protokoll anhängen
    fn append(&mut self, text: &str) -> Result<()>;
    /// Gesammeltes hochladen (OneDrive); im Ordner ist schon alles geschrieben
    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
    fn images(&mut self) -> Result<Vec<String>>;
    fn get_image(&mut self, name: &str) -> Result<Vec<u8>>;
    fn put_image(&mut self, name: &str, bytes: Vec<u8>) -> Result<()>;
}

/// Ablage in einem Ordner, den alle haben (OneDrive synchronisiert, Teams, Laufwerk der Schule)
pub struct FolderStore {
    pub dir: PathBuf,
    me: String,
}

impl FolderStore {
    pub fn new(dir: &Path, me: &str) -> FolderStore {
        FolderStore { dir: dir.into(), me: me.into() }
    }

    /// Ordner für ein neues geteiltes Doc anlegen
    pub fn create(dir: &Path, me: &str, title: &str) -> Result<FolderStore> {
        std::fs::create_dir_all(dir.join(LOGS))?;
        std::fs::create_dir_all(dir.join(IMAGES))?;
        std::fs::write(dir.join("doc.json"), serde_json::to_string_pretty(&json!({ "format": 1, "titel": title, "erstellt": now_ms(), "app": "ASK" }))?)?;
        Ok(FolderStore::new(dir, me))
    }
}

impl Store for FolderStore {
    fn describe(&self) -> String {
        self.dir.to_string_lossy().to_string()
    }
    fn check(&mut self) -> Result<()> {
        if !self.dir.join("doc.json").is_file() || !self.dir.join(LOGS).is_dir() {
            bail!("In diesem Ordner liegt kein geteiltes Doc (doc.json und teilnehmer/ fehlen).");
        }
        Ok(())
    }
    fn logs(&mut self) -> Result<Vec<(String, u64)>> {
        let Ok(entries) = std::fs::read_dir(self.dir.join(LOGS)) else { bail!("Der geteilte Ordner ist nicht erreichbar: {}", self.dir.display()) };
        Ok(entries.flatten().filter_map(|e| {
            let file = e.file_name().to_string_lossy().to_string();
            let who = file.strip_suffix(".jsonl")?.to_string();
            Some((who, e.metadata().ok()?.len()))
        }).collect())
    }
    fn read_log(&mut self, who: &str, from: u64) -> Result<Vec<u8>> {
        let mut f = std::fs::File::open(self.dir.join(LOGS).join(format!("{who}.jsonl")))?;
        f.seek(SeekFrom::Start(from))?;
        let mut buf = Vec::new();
        f.take(MAX_LOG).read_to_end(&mut buf)?;
        Ok(buf)
    }
    fn append(&mut self, text: &str) -> Result<()> {
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join(LOGS).join(format!("{}.jsonl", self.me)))?;
        f.write_all(text.as_bytes())?; // ein Schreiben pro Änderung: nie eine halbe Zeile von uns
        Ok(())
    }
    fn images(&mut self) -> Result<Vec<String>> {
        Ok(std::fs::read_dir(self.dir.join(IMAGES)).map(|r| r.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect()).unwrap_or_default())
    }
    fn get_image(&mut self, name: &str) -> Result<Vec<u8>> {
        Ok(std::fs::read(self.dir.join(IMAGES).join(name))?)
    }
    fn put_image(&mut self, name: &str, bytes: Vec<u8>) -> Result<()> {
        std::fs::create_dir_all(self.dir.join(IMAGES))?;
        Ok(std::fs::write(self.dir.join(IMAGES).join(name), bytes)?)
    }
}

// ---------- Ein geteiltes Doc ----------

fn hash(s: &str) -> String {
    crate::docs::hash(s)
}

fn valid_writer(w: &str) -> bool {
    (4..=32).contains(&w.len()) && w.chars().all(|c| c.is_ascii_alphanumeric())
}

fn image_name_ok(name: &str) -> bool {
    let lower = name.to_lowercase();
    name.len() <= 120 && safe_rel(name) && !name.contains('/')
        && [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().any(|e| lower.ends_with(e))
}

/// Ein geteiltes Doc dieses Laptops
pub struct Shared {
    /// Projekt (Ordner) und Pfad des Docs darin
    pub project: String,
    pub rel: String,
    /// Wo die Protokolle liegen
    pub store: Box<dyn Store>,
    /// Eigene Kennung und Name
    pub me: String,
    pub name: String,
    crdt: Crdt,
    offsets: HashMap<String, u64>,
    /// Prüfsumme des Stands, der zuletzt in der Datei im Projekt stand
    written: String,
    /// Letzte Stände (Prüfsumme → Absätze), damit eine Änderung gegen den Stand verglichen wird, auf dem sie beruht
    versions: VecDeque<(String, Vec<(Id, String)>)>,
    pub error: String,
}

impl Shared {
    fn new(project: &str, rel: &str, store: Box<dyn Store>, me: &str, name: &str) -> Shared {
        Shared { project: project.into(), rel: rel.into(), store, me: me.into(), name: name.into(), crdt: Crdt::default(),
            offsets: HashMap::new(), written: String::new(), versions: VecDeque::new(), error: String::new() }
    }

    /// Neu teilen: den jetzigen Inhalt als erste Einträge in eine neue, leere Ablage schreiben
    pub fn create(project: &str, rel: &str, store: Box<dyn Store>, me: &str, name: &str, content: &str) -> Result<Shared> {
        let mut s = Shared::new(project, rel, store, me, name);
        let mut ops = vec![Op::Hallo { wer: me.into(), name: name.into(), zeit: now_ms() }];
        let mut clock = 0;
        ops.extend(diff_ops(&[], &split_blocks(content), me, &mut clock, now_ms()));
        s.append(&ops)?;
        for op in &ops {
            s.crdt.apply(op);
        }
        s.store.flush()?;
        s.written = hash(content);
        s.remember();
        Ok(s)
    }

    /// Vorhandenes geteiltes Doc öffnen und alle Protokolle lesen
    pub fn open(project: &str, rel: &str, mut store: Box<dyn Store>, me: &str, name: &str) -> Result<Shared> {
        store.check()?;
        let mut s = Shared::new(project, rel, store, me, name);
        s.read_logs()?;
        if s.crdt.names.get(me).map(String::as_str) != Some(name) {
            let hallo = Op::Hallo { wer: me.into(), name: name.into(), zeit: now_ms() };
            s.append(std::slice::from_ref(&hallo))?;
            s.crdt.apply(&hallo);
            s.store.flush()?;
        }
        Ok(s)
    }

    /// Nur für die Anzeige, wenn sich die Ablage gerade nicht öffnen lässt
    fn broken(project: &str, rel: &str, store: Box<dyn Store>, me: &str, name: &str, error: String) -> Shared {
        let mut s = Shared::new(project, rel, store, me, name);
        s.error = error;
        s
    }

    fn append(&mut self, ops: &[Op]) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let mut text = String::new();
        for op in ops {
            text.push_str(&serde_json::to_string(op)?);
            text.push('\n');
        }
        self.store.append(&text)
    }

    /// Neue Zeilen aus allen Protokollen; true, wenn sich etwas geändert hat
    fn read_logs(&mut self) -> Result<bool> {
        let mut changed = false;
        for (who, len) in self.store.logs()? {
            if !valid_writer(&who) {
                continue;
            }
            let mut from = self.offsets.get(&who).copied().unwrap_or(0);
            if len < from {
                from = 0; // Datei ersetzt (z. B. von OneDrive): alles nochmals, Einträge doppelt anwenden schadet nicht
            }
            if len == from || len > MAX_LOG {
                continue;
            }
            let buf = self.store.read_log(&who, from)?;
            // Nur ganze Zeilen; der Rest kommt beim nächsten Mal
            let Some(end) = buf.iter().rposition(|b| *b == b'\n') else { continue };
            for line in String::from_utf8_lossy(&buf[..=end]).lines() {
                let Ok(op) = serde_json::from_str::<Op>(line) else { continue };
                // Einträge gehören der Person, deren Datei es ist
                let own = match &op {
                    Op::Hallo { wer, .. } => wer == &who,
                    Op::Neu { id, .. } => id.1 == who,
                    Op::Text { ts, .. } => ts.1 == who,
                    Op::Weg { .. } => true,
                };
                if own && self.crdt.apply(&op) {
                    changed = true;
                }
            }
            self.offsets.insert(who, from + end as u64 + 1);
        }
        Ok(changed)
    }

    fn remember(&mut self) {
        let v = self.crdt.visible();
        let h = hash(&join_blocks(&v.iter().map(|b| b.1.clone()).collect::<Vec<_>>()));
        self.versions.retain(|x| x.0 != h);
        self.versions.push_back((h, v));
        while self.versions.len() > KEEP_VERSIONS {
            self.versions.pop_front();
        }
    }

    fn local_path(&self) -> Result<PathBuf> {
        safe_join(Path::new(&self.project), &self.rel)
    }

    /// Eigene Änderung: `content` beruht auf dem Stand mit der Prüfsumme `base` (unbekannt: der jetzige Stand)
    pub fn edit(&mut self, content: &str, base: Option<&str>) -> Result<bool> {
        let base_blocks = base.and_then(|b| self.versions.iter().rev().find(|v| v.0 == b)).map(|v| v.1.clone()).unwrap_or_else(|| self.crdt.visible());
        let mut clock = self.crdt.clock;
        let ops = diff_ops(&base_blocks, &split_blocks(content), &self.me, &mut clock, now_ms());
        if ops.is_empty() {
            return Ok(false);
        }
        self.append(&ops)?;
        for op in &ops {
            self.crdt.apply(op);
        }
        Ok(true)
    }

    fn read_local(&self) -> Option<String> {
        let bytes = std::fs::read(self.local_path().ok()?).ok()?;
        Some(String::from_utf8_lossy(&bytes).replace("\r\n", "\n"))
    }

    /// Zusammengeführten Stand in die Datei im Projekt schreiben, aber nur, wenn sie seit `seen` niemand geändert hat
    fn write_local(&mut self, seen: Option<&str>) -> Result<bool> {
        let render = self.crdt.render();
        let now = self.read_local();
        if now.as_deref() == Some(render.as_str()) {
            self.written = hash(&render);
            self.remember();
            return Ok(false);
        }
        if let Some(seen) = seen {
            if now.as_deref().map(hash).as_deref() != Some(seen) {
                return Ok(false); // inzwischen gespeichert: beim nächsten Abgleich als eigene Änderung
            }
        }
        let path = self.local_path()?;
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        let tmp = path.with_extension(format!("md.{}.tmp", hex::encode(rand::random::<[u8; 3]>())));
        std::fs::write(&tmp, &render)?;
        std::fs::rename(&tmp, &path)?;
        self.written = hash(&render);
        self.remember();
        Ok(true)
    }

    /// Ein Abgleich: eigene Änderungen aus der Datei, Neues der anderen, Datei neu schreiben. true: Datei geändert.
    pub fn tick(&mut self) -> Result<bool> {
        let local = self.read_local();
        let seen = local.as_deref().map(hash);
        if let (Some(c), Some(h)) = (&local, &seen) {
            if *h != self.written {
                let base = self.written.clone();
                self.edit(c, Some(&base))?;
                self.written = h.clone();
                self.remember();
            }
        }
        self.store.flush()?;
        self.read_logs()?;
        let wrote = self.write_local(seen.as_deref())?;
        self.sync_images();
        self.error.clear();
        Ok(wrote)
    }

    /// Bilder, auf die das Doc mit bilder/… zeigt, in beide Richtungen kopieren (nie überschreiben, nie löschen)
    fn sync_images(&mut self) {
        let Ok(local_doc) = self.local_path() else { return };
        let Some(local_dir) = local_doc.parent().map(|d| d.join(IMAGES)) else { return };
        let text = self.crdt.render();
        let re = regex::Regex::new(r"!\[[^\]]*\]\(bilder/([^)\s/\\]+)\)").expect("regex");
        let wanted: Vec<String> = re.captures_iter(&text).map(|c| c[1].to_string()).filter(|n| image_name_ok(n)).collect();
        if wanted.is_empty() {
            return;
        }
        let Ok(there) = self.store.images() else { return };
        let doc_dir = self.rel.rsplit_once('/').map(|x| x.0.to_string()).unwrap_or_else(|| "docs".into());
        for name in wanted {
            let local = local_dir.join(&name);
            if local.is_file() {
                if !there.contains(&name) && std::fs::metadata(&local).map(|m| m.len() <= MAX_IMAGE).unwrap_or(false) {
                    if let Ok(bytes) = std::fs::read(&local) {
                        let _ = self.store.put_image(&name, bytes);
                    }
                }
            } else if there.contains(&name) && safe_join(Path::new(&self.project), &format!("{doc_dir}/{IMAGES}/{name}")).is_ok() {
                // Im Projekt nur über einen sicheren Pfad schreiben (keine Verknüpfung hinaus)
                if let Ok(bytes) = self.store.get_image(&name) {
                    if bytes.len() as u64 <= MAX_IMAGE {
                        let _ = std::fs::create_dir_all(&local_dir);
                        let _ = std::fs::write(&local, bytes);
                    }
                }
            }
        }
    }

    /// Wer mitschreibt: [{ name, ich, zuletzt }]
    pub fn people(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self.crdt.names.iter().map(|(w, n)| json!({ "name": n, "ich": w == &self.me, "zuletzt": self.crdt.seen.get(w).copied().unwrap_or(0) })).collect();
        out.sort_by(|a, b| b["zuletzt"].as_i64().cmp(&a["zuletzt"].as_i64()));
        out
    }

    /// Letzte Änderungen der anderen: [{ name, zeit, text }]
    pub fn recent_by_others(&self, n: usize) -> Vec<Value> {
        let mut out: Vec<(i64, String, String)> = self.crdt.visible().iter()
            .filter_map(|(id, text)| self.crdt.author_of(id).filter(|a| a.0 != self.me).map(|a| (a.1, self.crdt.names.get(&a.0).cloned().unwrap_or_else(|| "Jemand".into()), plain(text))))
            .collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.0));
        out.into_iter().take(n).map(|(z, name, text)| json!({ "name": name, "zeit": z, "text": text })).collect()
    }

    pub fn info(&self) -> Value {
        json!({ "geteilt": true, "ordner": self.store.describe(), "link": self.store.link(), "personen": self.people(), "anderen": self.recent_by_others(5), "fehler": self.error })
    }
}

/// Kurzer, lesbarer Auszug eines Absatzes (ohne Zeichen von Markdown)
fn plain(text: &str) -> String {
    let t: String = text.chars().map(|c| if matches!(c, '|' | '#' | '*' | '_' | '`' | '>' | '~') { ' ' } else { c }).collect();
    let t = t.split_whitespace().filter(|w| !w.chars().all(|c| c == '-' || c == ':')).collect::<Vec<_>>().join(" ");
    take_chars(&t, 80)
}

// ---------- Verwaltung ----------

type Notify = Arc<dyn Fn(&str, &str) + Send + Sync>;

#[derive(Default)]
struct Manager {
    docs: Mutex<Vec<Arc<Mutex<Shared>>>>,
    notify: Mutex<Option<Notify>>,
    started: std::sync::atomic::AtomicBool,
}

fn manager() -> &'static Manager {
    static M: OnceLock<Manager> = OnceLock::new();
    M.get_or_init(Manager::default)
}

fn links() -> Vec<Value> {
    settings::get(SETTING).as_array().cloned().unwrap_or_default()
}

fn save_links(list: Vec<Value>) {
    settings::set(json!({ SETTING: list }));
}

fn same_doc(a_project: &str, a_rel: &str, b_project: &str, b_rel: &str) -> bool {
    same_path(a_project, b_project) && a_rel.eq_ignore_ascii_case(b_rel)
}

/// Das geteilte Doc zu Projekt und Pfad, falls es eines ist
pub fn find(project: &str, rel: &str) -> Option<Arc<Mutex<Shared>>> {
    manager().docs.lock().iter().find(|d| { let d = d.lock(); same_doc(&d.project, &d.rel, project, rel) }).cloned()
}

/// Darf dieser Ordner der geteilte Ordner sein? Nicht in einem Projekt, nicht bei den Chats ohne Projekt, nicht in den
/// Daten der App (dort schreibt das Terminal bzw. nur die App).
pub fn check_place(dir: &str) -> Result<()> {
    if dir.trim().is_empty() || !Path::new(dir).is_dir() {
        bail!("Den Ordner gibt es nicht.");
    }
    let mut forbidden: Vec<String> = settings::get("projects").as_array().map(|a| a.iter().map(|p| str_of(p, "folder")).filter(|f| !f.is_empty()).collect()).unwrap_or_default();
    forbidden.push(crate::paths::free_dir().to_string_lossy().to_string());
    forbidden.push(crate::paths::user_data().to_string_lossy().to_string());
    for f in forbidden {
        if crate::folders::is_inside(dir, &f) || crate::folders::is_inside(&f, dir) {
            bail!("Der geteilte Ordner darf nicht in einem Projekt liegen und kein Projekt enthalten. Nimm einen Ordner in OneDrive, Teams oder auf einem Laufwerk der Schule.");
        }
    }
    Ok(())
}

/// Geteiltes Doc merken: `place` ist { ordner } oder { art: "link", link, laufwerk, kennung }
fn add(shared: Shared, place: Value) {
    let mut link = json!({ "projekt": shared.project, "pfad": shared.rel, "ich": shared.me, "name": shared.name });
    if let (Some(l), Some(p)) = (link.as_object_mut(), place.as_object()) {
        l.extend(p.iter().map(|(k, v)| (k.clone(), v.clone())));
    }
    let mut list = links();
    list.retain(|l| !same_doc(&str_of(l, "projekt"), &str_of(l, "pfad"), &shared.project, &shared.rel));
    list.push(link);
    save_links(list);
    let mut docs = manager().docs.lock();
    docs.retain(|d| { let d = d.lock(); !same_doc(&d.project, &d.rel, &shared.project, &shared.rel) });
    docs.push(Arc::new(Mutex::new(shared)));
}

fn new_writer() -> String {
    hex::encode(rand::random::<[u8; 5]>())
}

/// Doc über einen Ordner teilen: in `parent` einen Ordner `<name>.askdoc` anlegen (E82)
pub fn share(project: &str, rel: &str, parent: &str, name: &str) -> Result<Value> {
    if find(project, rel).is_some() {
        bail!("Dieses Doc ist schon geteilt.");
    }
    check_place(parent)?;
    let doc = crate::docs::read(project, rel)?;
    let title = str_of(&doc, "title");
    let base = crate::docs::slug(&title);
    let dir = (1..100).map(|n| Path::new(parent).join(if n == 1 { format!("{base}.{EXT}") } else { format!("{base}-{n}.{EXT}") })).find(|d| !d.exists())
        .ok_or_else(|| anyhow::anyhow!("Kein freier Name im Ordner."))?;
    let me = new_writer();
    let store = FolderStore::create(&dir, &me, &title)?;
    let shared = Shared::create(project, &str_of(&doc, "path"), Box::new(store), &me, name, &str_of(&doc, "content"))?;
    let info = shared.info();
    add(shared, json!({ "ordner": dir.to_string_lossy() }));
    Ok(info)
}

/// Doc per Link teilen (E83): Ordner in OneDrive anlegen, Link zum Bearbeiten für die Schule. Blockiert.
pub fn share_link(project: &str, rel: &str, name: &str, rt: tokio::runtime::Handle, token: crate::msgraph::TokenFn) -> Result<Value> {
    if find(project, rel).is_some() {
        bail!("Dieses Doc ist schon geteilt.");
    }
    let doc = crate::docs::read(project, rel)?;
    let title = str_of(&doc, "title");
    let me = new_writer();
    let store = crate::msgraph::create_shared(rt, token, &crate::docs::slug(&title), &title, &me)?;
    let place = json!({ "art": "link", "link": store.link, "laufwerk": store.drive, "kennung": store.folder });
    let shared = Shared::create(project, &str_of(&doc, "path"), Box::new(store), &me, name, &str_of(&doc, "content"))?;
    let info = shared.info();
    add(shared, place);
    Ok(info)
}

/// Freier Pfad unter docs/ für ein geöffnetes geteiltes Doc
fn free_rel(project: &str, title: &str) -> Result<String> {
    let base = crate::docs::slug(title);
    (1..1000).map(|n| if n == 1 { format!("docs/{base}.md") } else { format!("docs/{base}-{n}.md") })
        .find(|r| safe_join(Path::new(project), r).map(|p| !p.exists()).unwrap_or(false))
        .ok_or_else(|| anyhow::anyhow!("Kein freier Name unter docs/."))
}

/// Geteiltes Doc aus einem Ordner öffnen (`….askdoc` oder ein Ordner, der genau eines enthält); legt es im Projekt an
pub fn join(project: &str, picked: &str, name: &str) -> Result<String> {
    let mut dir = PathBuf::from(picked);
    if !dir.join("doc.json").is_file() {
        let found: Vec<PathBuf> = std::fs::read_dir(&dir).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.join("doc.json").is_file() && p.extension().is_some_and(|e| e == EXT)).collect()).unwrap_or_default();
        match found.len() {
            1 => dir = found[0].clone(),
            0 => bail!("In diesem Ordner liegt kein geteiltes Doc. Wähle den Ordner, der auf «.{EXT}» endet."),
            _ => bail!("In diesem Ordner liegen mehrere geteilte Docs. Wähle den richtigen Ordner «….{EXT}»."),
        }
    }
    check_place(&dir.to_string_lossy())?;
    if let Some(l) = links().iter().find(|l| same_path(&str_of(l, "ordner"), &dir.to_string_lossy()) && same_path(&str_of(l, "projekt"), project)) {
        return Ok(str_of(l, "pfad")); // schon da
    }
    let meta: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("doc.json"))?).unwrap_or(Value::Null);
    let title = { let t = str_of(&meta, "titel"); if t.trim().is_empty() { "Geteiltes Doc".to_string() } else { take_chars(&t, 120) } };
    let rel = free_rel(project, &title)?;
    let me = new_writer();
    let mut shared = Shared::open(project, &rel, Box::new(FolderStore::new(&dir, &me)), &me, name)?;
    shared.write_local(None)?;
    add(shared, json!({ "ordner": dir.to_string_lossy() }));
    Ok(rel)
}

/// Geteiltes Doc über einen Link öffnen (aus Teams, OneDrive oder SharePoint); legt es im Projekt an. Blockiert.
pub fn join_link(project: &str, link: &str, name: &str, rt: tokio::runtime::Handle, token: crate::msgraph::TokenFn) -> Result<String> {
    if let Some(l) = links().iter().find(|l| str_of(l, "link") == link.trim() && same_path(&str_of(l, "projekt"), project)) {
        return Ok(str_of(l, "pfad")); // schon da
    }
    let me = new_writer();
    let store = crate::msgraph::open_link(rt, token, link, &me)?;
    let title = { let t = store.title()?; if t.trim().is_empty() { "Geteiltes Doc".to_string() } else { take_chars(&t, 120) } };
    let rel = free_rel(project, &title)?;
    let place = json!({ "art": "link", "link": store.link, "laufwerk": store.drive, "kennung": store.folder });
    let mut shared = Shared::open(project, &rel, Box::new(store), &me, name)?;
    shared.write_local(None)?;
    add(shared, place);
    Ok(rel)
}

/// Nicht mehr teilen: Das Doc bleibt im Projekt, der geteilte Ordner bleibt für die anderen
pub fn unshare(project: &str, rel: &str) {
    let mut list = links();
    list.retain(|l| !same_doc(&str_of(l, "projekt"), &str_of(l, "pfad"), project, rel));
    save_links(list);
    manager().docs.lock().retain(|d| { let d = d.lock(); !same_doc(&d.project, &d.rel, project, rel) });
}

/// Speichern aus dem Editor oder von ASK: `None`, wenn das Doc nicht geteilt ist. Sonst wird die Änderung gegen den
/// Stand `base` verglichen, mit dem Neuen der anderen zusammengeführt und geschrieben. Die Protokolle der anderen liest
/// hier nur die Ablage im Ordner; OneDrive liest der nächste Abgleich (keine Netzanfrage, während die Oberfläche wartet).
pub fn save(project: &str, rel: &str, content: &str, base: Option<&str>) -> Option<Result<Value>> {
    let shared = find(project, rel)?;
    let mut s = shared.lock();
    let r = (|| {
        if s.store.link().is_none() {
            s.read_logs()?;
        }
        s.edit(content, base)?;
        let render = s.crdt.render();
        s.write_local(None)?;
        let path = s.local_path()?;
        let modified = std::fs::metadata(&path).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
        let merged = render != join_blocks(&split_blocks(content));
        Ok(json!({ "ok": true, "path": s.rel, "hash": hash(&render), "modified": modified, "title": crate::docs::title_of(&render, &s.rel), "content": render, "merged": merged }))
    })();
    Some(r)
}

/// Angaben zum Teilen für die Oberfläche; `{ geteilt: false }`, wenn das Doc nicht geteilt ist
pub fn info(project: &str, rel: &str) -> Value {
    find(project, rel).map(|s| s.lock().info()).unwrap_or_else(|| json!({ "geteilt": false }))
}

/// Kurz für das Modell: mit wem geteilt, was andere zuletzt geändert haben
pub fn prompt_note(project: &str, rel: &str) -> String {
    let Some(s) = find(project, rel) else { return String::new() };
    let s = s.lock();
    let others: Vec<String> = s.people().iter().filter(|p| p["ich"] != true).map(|p| str_of(p, "name")).collect();
    let via = if s.store.link().is_some() { "per Link über OneDrive".to_string() } else { format!("über den Ordner {}", s.store.describe()) };
    let mut out = format!("Geteiltes Doc (live, {via}), schreiben mit: {}.", if others.is_empty() { "noch niemand".into() } else { others.join(", ") });
    let recent = s.recent_by_others(5);
    if !recent.is_empty() {
        let now = now_ms();
        let lines: Vec<String> = recent.iter().map(|r| format!("{} vor {} Min.: «{}»", str_of(r, "name"), ((now - r["zeit"].as_i64().unwrap_or(now)) / 60_000).max(0), str_of(r, "text"))).collect();
        out.push_str(&format!(" Letzte Änderungen der anderen: {}.", lines.join("; ")));
    }
    out
}

/// Gemerkte geteilte Docs öffnen (blockiert: OneDrive braucht das Netz). Eine Ablage, die sich gerade nicht öffnen
/// lässt, bleibt mit Fehler in der Liste und wird beim nächsten Start wieder versucht.
fn load_all(rt: tokio::runtime::Handle) {
    for l in links() {
        let (project, rel, me) = (str_of(&l, "projekt"), str_of(&l, "pfad"), str_of(&l, "ich"));
        let name = display_name(Some(str_of(&l, "name")));
        if !valid_writer(&me) || !safe_rel(&rel) || find(&project, &rel).is_some() {
            continue;
        }
        let store: Box<dyn Store> = if str_of(&l, "art") == "link" {
            Box::new(crate::msgraph::GraphStore::new(rt.clone(), crate::msgraph::app_token(rt.clone()), &str_of(&l, "laufwerk"), &str_of(&l, "kennung"), &str_of(&l, "link"), &me))
        } else {
            Box::new(FolderStore::new(Path::new(&str_of(&l, "ordner")), &me))
        };
        // Beim Start ist der letzte Stand unbekannt: die Datei im Projekt gilt als Änderung gegen den Stand der Ablage
        let shared = match Shared::open(&project, &rel, store, &me, &name) {
            Ok(s) => s,
            Err(e) => {
                let store: Box<dyn Store> = Box::new(FolderStore::new(Path::new(&str_of(&l, "ordner")), &me));
                let mut s = Shared::broken(&project, &rel, store, &me, &name, e.to_string());
                if str_of(&l, "art") == "link" {
                    s.store = Box::new(crate::msgraph::GraphStore::new(rt.clone(), crate::msgraph::app_token(rt.clone()), &str_of(&l, "laufwerk"), &str_of(&l, "kennung"), &str_of(&l, "link"), &me));
                }
                s
            }
        };
        manager().docs.lock().push(Arc::new(Mutex::new(shared)));
    }
}

/// Geteilte Docs laden und regelmässig abgleichen. `notify(projekt, pfad)` meldet geänderte Docs. Nur einmal.
pub fn start(notify: impl Fn(&str, &str) + Send + Sync + 'static) {
    let m = manager();
    *m.notify.lock() = Some(Arc::new(notify));
    if m.started.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let Ok(rt) = tokio::runtime::Handle::try_current() else { return };
    rt.spawn(async move {
        let h = tokio::runtime::Handle::current();
        let _ = tokio::task::spawn_blocking(move || load_all(h)).await;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(TICK_MS)).await;
            let docs: Vec<Arc<Mutex<Shared>>> = manager().docs.lock().clone();
            for d in docs {
                let d2 = d.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let mut s = d2.lock();
                    let r = s.tick();
                    if let Err(e) = &r {
                        s.error = e.to_string();
                    }
                    (s.project.clone(), s.rel.clone(), r.unwrap_or(false))
                }).await;
                if let Ok((project, rel, true)) = result {
                    if let Some(n) = manager().notify.lock().clone() {
                        n(&project, &rel);
                    }
                }
            }
        }
    });
}

/// Name zum Anzeigen: aus ASK (falls angemeldet), sonst der Benutzername von Windows
pub fn display_name(ask_name: Option<String>) -> String {
    ask_name.filter(|n| !n.trim().is_empty()).map(|n| take_chars(n.trim(), 60))
        .or_else(|| std::env::var("USERNAME").ok()).or_else(|| std::env::var("USER").ok()).unwrap_or_else(|| "Jemand".into())
}

/// Ist dieses Doc geteilt? (für Liste und Systemprompt)
pub fn is_shared(project: &str, rel: &str) -> bool {
    links().iter().any(|l| same_doc(&str_of(l, "projekt"), &str_of(l, "pfad"), project, rel))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn absaetze_und_codeblock() {
        assert_eq!(split_blocks("# T\n\nA\nB\n\n\n```\nx\n\ny\n```\n\n- 1\n- 2\n"), s(&["# T", "A\nB", "```\nx\n\ny\n```", "- 1\n- 2"]));
        assert_eq!(join_blocks(&s(&["a", "b"])), "a\n\nb\n");
    }

    #[test]
    fn gleichzeitig_an_verschiedenen_stellen_gleiches_ergebnis() {
        // Lea legt an, Tim und Lea ändern gleichzeitig verschiedene Absätze und fügen ein
        let mut clock = 0;
        let start = diff_ops(&[], &s(&["# Titel", "Eins", "Zwei", "Drei"]), "lea", &mut clock, 1);
        let mut lea = Crdt::default();
        let mut tim = Crdt::default();
        for op in &start { lea.apply(op); tim.apply(op); }
        let mut cl = lea.clock;
        let a = diff_ops(&lea.visible(), &s(&["# Titel", "Eins (Lea)", "Zwei", "Neu von Lea", "Drei"]), "lea", &mut cl, 2);
        let mut ct = tim.clock;
        let b = diff_ops(&tim.visible(), &s(&["# Titel", "Eins", "Zwei", "Drei (Tim)", "Schluss von Tim"]), "tim", &mut ct, 2);
        for op in &a { lea.apply(op); }
        for op in &b { tim.apply(op); }
        // Austausch in verschiedener Reihenfolge, auch doppelt
        for op in &b { lea.apply(op); }
        for op in a.iter().chain(a.iter()) { tim.apply(op); }
        assert_eq!(lea.render(), tim.render());
        assert_eq!(lea.render(), "# Titel\n\nEins (Lea)\n\nZwei\n\nNeu von Lea\n\nDrei (Tim)\n\nSchluss von Tim\n");
    }

    #[test]
    fn gleicher_absatz_spaetere_fassung_gilt_und_loeschen() {
        let mut clock = 0;
        let start = diff_ops(&[], &s(&["A", "B"]), "lea", &mut clock, 1);
        let (mut x, mut y) = (Crdt::default(), Crdt::default());
        for op in &start { x.apply(op); y.apply(op); }
        let mut c1 = x.clock;
        let a = diff_ops(&x.visible(), &s(&["A von Lea", "B"]), "lea", &mut c1, 2);
        let mut c2 = y.clock;
        let b = diff_ops(&y.visible(), &s(&["A von Tim"]), "tim", &mut c2, 2); // ändert A, löscht B
        for op in a.iter().chain(b.iter()) { x.apply(op); }
        for op in b.iter().chain(a.iter()) { y.apply(op); }
        assert_eq!(x.render(), y.render());
        assert_eq!(x.render(), "A von Tim\n", "gleiche Lamport-Zeit: die grössere Kennung gilt, überall gleich");
        // Ein Absatz, der nach einem noch unbekannten kommt, erscheint, sobald der Vorgänger da ist
        let mut z = Crdt::default();
        let later = Op::Neu { id: (50, "tim".into()), nach: Some((49, "lea".into())), text: "danach".into(), zeit: 3 };
        z.apply(&later);
        assert_eq!(z.render(), "");
        z.apply(&Op::Neu { id: (49, "lea".into()), nach: None, text: "vorher".into(), zeit: 3 });
        assert_eq!(z.render(), "vorher\n\ndanach\n");
    }

    #[test]
    fn zwei_laptops_ueber_einen_ordner() {
        let tmp = tempfile::tempdir().unwrap();
        let (p1, p2, shared) = (tmp.path().join("lea"), tmp.path().join("tim"), tmp.path().join("onedrive"));
        for d in [&p1, &p2, &shared] { std::fs::create_dir_all(d).unwrap(); }
        let (f1, f2) = (p1.to_string_lossy().to_string(), p2.to_string_lossy().to_string());
        let rel = crate::docs::create(&f1, "Gruppenarbeit", "## Einleitung\n\n…\n\n## Schluss\n\n…").unwrap();
        let content = str_of(&crate::docs::read(&f1, &rel).unwrap(), "content");
        let dir = shared.join("gruppenarbeit.askdoc");
        let mut lea = Shared::create(&f1, &rel, Box::new(FolderStore::create(&dir, "aaaa1111", "Gruppenarbeit").unwrap()), "aaaa1111", "Lea", &content).unwrap();
        let mut tim = Shared::open(&f2, "docs/gruppenarbeit.md", Box::new(FolderStore::new(&dir, "bbbb2222")), "bbbb2222", "Tim").unwrap();
        assert!(tim.write_local(None).unwrap());
        assert_eq!(str_of(&crate::docs::read(&f2, "docs/gruppenarbeit.md").unwrap(), "content"), content);
        // Tim schreibt in der Datei (wie der Editor oder ASK), Lea gleichzeitig mit dem Editor auf einem älteren Stand
        let tim_text = content.replace("## Einleitung\n\n…", "## Einleitung\n\nText von Tim.");
        std::fs::write(p2.join("docs/gruppenarbeit.md"), &tim_text).unwrap();
        tim.tick().unwrap();
        let base = hash(&content);
        let lea_text = content.replace("## Schluss\n\n…", "## Schluss\n\nText von Lea.");
        // Lea speichert über den Editor (mit dem Stand, auf dem ihre Änderung beruht)
        lea.read_logs().unwrap();
        lea.edit(&lea_text, Some(&base)).unwrap();
        lea.write_local(None).unwrap();
        tim.tick().unwrap();
        lea.tick().unwrap();
        let a = str_of(&crate::docs::read(&f1, &rel).unwrap(), "content");
        let b = str_of(&crate::docs::read(&f2, "docs/gruppenarbeit.md").unwrap(), "content");
        assert_eq!(a, b);
        assert!(a.contains("Text von Tim.") && a.contains("Text von Lea."), "{a}");
        assert_eq!(lea.people().len(), 2);
        assert_eq!(str_of(&lea.recent_by_others(5)[0], "name"), "Tim");
        // Zwei Dateien im Ordner, jede nur von einer Person geschrieben
        let logs: Vec<String> = std::fs::read_dir(dir.join("teilnehmer")).unwrap().flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
        assert_eq!(logs.len(), 2);
    }

    #[test]
    fn halbe_zeilen_und_fremde_eintraege() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("p");
        std::fs::create_dir_all(&p).unwrap();
        let f = p.to_string_lossy().to_string();
        let rel = crate::docs::create(&f, "X", "A").unwrap();
        let dir = tmp.path().join("x.askdoc");
        let mut s = Shared::create(&f, &rel, Box::new(FolderStore::create(&dir, "aaaa1111", "X").unwrap()), "aaaa1111", "Lea", &str_of(&crate::docs::read(&f, &rel).unwrap(), "content")).unwrap();
        // Ein Eintrag, der vorgibt, von Lea zu sein, in Tims Datei: zählt nicht. Eine halbe Zeile wartet.
        let fake = serde_json::to_string(&Op::Neu { id: (99, "aaaa1111".into()), nach: None, text: "gefälscht".into(), zeit: 1 }).unwrap();
        let ok = serde_json::to_string(&Op::Neu { id: (98, "cccc3333".into()), nach: None, text: "von Tim".into(), zeit: 1 }).unwrap();
        std::fs::write(dir.join("teilnehmer/cccc3333.jsonl"), format!("{fake}\n{ok}\n{{\"op\":\"neu\",\"id\"")).unwrap();
        std::fs::write(dir.join("teilnehmer/..jsonl"), format!("{ok}\n")).unwrap();
        s.tick().unwrap();
        let text = str_of(&crate::docs::read(&f, &rel).unwrap(), "content");
        assert!(text.contains("von Tim") && !text.contains("gefälscht"), "{text}");
    }

    /// Nachbau von Microsoft Graph: Dateien im Speicher, Ordnerkennung = Name des Ordners `….askdoc`
    fn graph_mock(files: &parking_lot::Mutex<HashMap<String, Vec<u8>>>, req: &crate::askapi::test_http::Req) -> crate::askapi::test_http::Resp {
        use crate::askapi::test_http::Resp;
        let raw = req.path.split('?').next().unwrap_or("");
        let path = percent_encoding::percent_decode_str(raw).decode_utf8_lossy().to_string();
        let mut f = files.lock();
        if req.headers.get("authorization").or(req.headers.get("Authorization")).map(String::as_str) != Some("Bearer t") {
            return Resp::json(401, &json!({}));
        }
        if let Some(rest) = path.strip_prefix("/me/drive/root:/ASK Docs/") {
            if let Some(name) = rest.strip_suffix("/doc.json:/content") {
                f.insert(format!("{name}/doc.json"), req.body.clone().into_bytes());
                return Resp::json(201, &json!({ "id": "x", "parentReference": { "driveId": "d1", "id": name } }));
            }
            let exists = f.keys().any(|k| k.starts_with(&format!("{rest}/")));
            return if exists { Resp::json(200, &json!({ "id": rest })) } else { Resp::json(404, &json!({})) };
        }
        if let Some(rest) = path.strip_prefix("/drives/d1/items/") {
            if let Some(folder) = rest.strip_suffix("/createLink") {
                return Resp::json(200, &json!({ "link": { "webUrl": format!("https://tfbern.sharepoint.com/:f:/g/{folder}") } }));
            }
            let (folder, rel) = rest.split_once(":/").unwrap_or((rest, ""));
            if let Some(dir) = rel.strip_suffix(":/children") {
                let prefix = format!("{folder}/{dir}/");
                let list: Vec<Value> = f.iter().filter(|(k, _)| k.starts_with(&prefix)).map(|(k, v)| json!({ "name": &k[prefix.len()..], "size": v.len() })).collect();
                return if list.is_empty() { Resp::json(404, &json!({})) } else { Resp::json(200, &json!({ "value": list })) };
            }
            if let Some(file) = rel.strip_suffix(":/content") {
                let key = format!("{folder}/{file}");
                if req.method == "PUT" {
                    f.insert(key, req.body.clone().into_bytes());
                    return Resp::json(201, &json!({}));
                }
                let Some(data) = f.get(&key) else { return Resp::json(404, &json!({})) };
                let range = req.headers.get("range").or(req.headers.get("Range")).and_then(|r| r.strip_prefix("bytes=")).and_then(|r| r.trim_end_matches('-').parse::<usize>().ok());
                return match range {
                    Some(from) if from >= data.len() => Resp::text(416, "text/plain", ""),
                    Some(from) => Resp::text(206, "application/octet-stream", data[from..].to_vec()),
                    None => Resp::text(200, "application/octet-stream", data.clone()),
                };
            }
        }
        if let Some(id) = path.strip_prefix("/shares/").and_then(|p| p.strip_suffix("/driveItem")) {
            let link = String::from_utf8(base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, id.trim_start_matches("u!")).unwrap_or_default()).unwrap_or_default();
            let folder = link.rsplit('/').next().unwrap_or("").to_string();
            if !f.keys().any(|k| k.starts_with(&format!("{folder}/"))) {
                return Resp::json(404, &json!({}));
            }
            return Resp::json(200, &json!({ "id": folder, "name": folder, "folder": {}, "parentReference": { "driveId": "d1" } }));
        }
        Resp::json(404, &json!({ "error": path }))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn zwei_laptops_per_link_ueber_onedrive() {
        let files = Arc::new(parking_lot::Mutex::new(HashMap::<String, Vec<u8>>::new()));
        let f = files.clone();
        let server = crate::askapi::test_http::serve(move |req| graph_mock(&f, req)).await;
        std::env::set_var("ASK_TEST_GRAPH", &server.url);
        let tmp = tempfile::tempdir().unwrap();
        let (p1, p2) = (tmp.path().join("lea"), tmp.path().join("tim"));
        for d in [&p1, &p2] { std::fs::create_dir_all(d).unwrap(); }
        let (f1, f2) = (p1.to_string_lossy().to_string(), p2.to_string_lossy().to_string());
        let rt = tokio::runtime::Handle::current();
        let out = tokio::task::spawn_blocking(move || {
            let token: crate::msgraph::TokenFn = Arc::new(|| Ok("t".to_string()));
            let rel = crate::docs::create(&f1, "Gruppenarbeit", "## Einleitung\n\n…\n\n## Schluss\n\n…").unwrap();
            let content = str_of(&crate::docs::read(&f1, &rel).unwrap(), "content");
            // Lea teilt: Ordner in OneDrive, Link für die Schule
            let store = crate::msgraph::create_shared(rt.clone(), token.clone(), "gruppenarbeit", "Gruppenarbeit", "aaaa1111").unwrap();
            let link = store.link.clone();
            assert!(link.starts_with("https://tfbern.sharepoint.com/"), "{link}");
            assert!(crate::msgraph::create_shared(rt.clone(), token.clone(), "gruppenarbeit", "Gruppenarbeit", "aaaa1111").unwrap().folder.ends_with("-2.askdoc"), "nie über ein vorhandenes");
            let mut lea = Shared::create(&f1, &rel, Box::new(store), "aaaa1111", "Lea", &content).unwrap();
            // Tim öffnet den Link aus Teams
            let store2 = crate::msgraph::open_link(rt.clone(), token.clone(), &link, "bbbb2222").unwrap();
            assert_eq!(store2.title().unwrap(), "Gruppenarbeit");
            let mut tim = Shared::open(&f2, "docs/gruppenarbeit.md", Box::new(store2), "bbbb2222", "Tim").unwrap();
            tim.write_local(None).unwrap();
            assert_eq!(str_of(&crate::docs::read(&f2, "docs/gruppenarbeit.md").unwrap(), "content"), content);
            // Beide schreiben gleichzeitig an verschiedenen Stellen
            std::fs::write(p2.join("docs/gruppenarbeit.md"), content.replace("## Einleitung\n\n…", "## Einleitung\n\nText von Tim.")).unwrap();
            std::fs::write(p1.join("docs/gruppenarbeit.md"), content.replace("## Schluss\n\n…", "## Schluss\n\nText von Lea.")).unwrap();
            for _ in 0..2 { tim.tick().unwrap(); lea.tick().unwrap(); }
            let a = str_of(&crate::docs::read(&f1, &rel).unwrap(), "content");
            let b = str_of(&crate::docs::read(&f2, "docs/gruppenarbeit.md").unwrap(), "content");
            assert_eq!(a, b);
            assert!(a.contains("Text von Tim.") && a.contains("Text von Lea."), "{a}");
            assert_eq!(lea.people().len(), 2);
            assert_eq!(lea.info()["link"], json!(link));
            // Ein falscher Link
            assert!(crate::msgraph::open_link(rt.clone(), token.clone(), "https://tfbern.sharepoint.com/:f:/g/gibt-es-nicht", "cccc3333").is_err());
            assert!(crate::msgraph::open_link(rt, token, "kein link", "cccc3333").is_err());
            files.lock().keys().cloned().collect::<Vec<_>>()
        }).await.unwrap();
        assert!(out.iter().any(|k| k == "gruppenarbeit.askdoc/teilnehmer/aaaa1111.jsonl"));
        assert!(out.iter().any(|k| k == "gruppenarbeit.askdoc/teilnehmer/bbbb2222.jsonl"));
    }
}
