//! GitHub-Projekte holen und hochladen: Repository klonen, Änderungen holen (pull) und als Commit
//! hochladen (push), dazu ein neues Repository anlegen. Ohne git auf dem Laptop (unter Low Integrity
//! laufen die Git-Werkzeuge nicht zuverlässig), direkt über die REST-API von GitHub mit einem
//! persönlichen Token. Das Token kennt nur der Hauptprozess, nie das Modell oder das Terminal.
//!
//! Der letzte abgeglichene Stand (Repository, Branch, Commit und die Prüfsumme jeder Datei) liegt in
//! einer Datei ausserhalb des Arbeitsordners (link_file, in der App unter project_data_dir), damit das
//! Modell ihn nicht ändern kann; er wird beim Lesen trotzdem geprüft. Damit erkennt der Abgleich,
//! was hier, was auf GitHub und was an beiden Orten geändert wurde; Dateien mit Änderungen an
//! beiden Orten werden nie stillschweigend überschrieben, weder hier noch auf GitHub.
//! Was in .gitignore steht, wird nicht hochgeladen (ausser es liegt schon auf GitHub, wie bei git).
//! Branches und Pull Requests ebenfalls über die API: Branch anlegen ist ein neuer Ref, wechseln ein
//! Holen vom anderen Branch (nur ohne Änderungen hier, ENTSCHEIDE E47).
//!
//! Öffentliche Schnittstelle (Namen wie in der JS-Klasse, snake_case; Ergebnisse als JSON mit
//! denselben Feldern wie in JavaScript):
//! - `GitHub::new(store)` mit `with_api`, `with_web`, `with_client`, `with_timeout`, `with_link_file`
//! - `connected`, `connect(token)`, `disconnect`, `whoami`, `request(method, url, body)`
//! - `read_link`, `write_link`, `url`, `remote`, `local_changes`, `conflicts`, `status`
//! - `create`, `clone_repo`, `pull`, `push`, `branches`, `create_branch`, `switch_branch`
//! - `pull_requests`, `create_pull_request`
//! - `key`, `locked`, `exclusive(ordner, f)`: ein Abgleich pro Ordner (schlägt fehl statt zu warten)
//! - freie Funktionen `blob_sha`, `list_local`, `parse_repo`, `valid_link`, `valid_branch`,
//!   `ignore_rule`, `ignored`, Konstante `SKIP`
//! - Fehlertypen `ApiError` (mit `status`), `PushConflict` (mit `conflicts`), `PendingChanges`
//!   (mit `pending`); abrufbar über `err.downcast_ref::<…>()`.

use anyhow::{anyhow, bail, Result};
use parking_lot::Mutex;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use regex::Regex;
use serde_json::{json, Value};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use crate::safe;
use crate::util::{js_string, str_of, take_chars, truthy};

/// GitHub lehnt grössere Dateien ab bzw. warnt
pub const MAX_FILE: u64 = 50 * 1024 * 1024;
pub const MAX_FILES: usize = 3000;
/// Zeitgrenze einer Anfrage an GitHub, sonst hängt der Abgleich ewig
pub const REQUEST_MS: u64 = 30 * 1000;
/// Dateien hoch- und herunterladen darf länger dauern
pub const BLOB_MS: u64 = 120 * 1000;

/// Nicht hochladen: Arbeitsdaten der App (Chats, Skripte, Anleitungen) und typische Ballast-Ordner
pub static SKIP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?:^|/)(\.ask|\.git|node_modules|__pycache__|\.venv)(/|$)").unwrap());

static REPO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_.\-]+/[A-Za-z0-9_.\-]+$").unwrap());
static BRANCH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_./\-]{1,200}$").unwrap());
static LOCK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.lock(/|$)").unwrap());
static REPO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)github\.com[/:]([A-Za-z0-9_.\-]+)/([A-Za-z0-9_.\-]+?)(?:\.git)?(?:[/?#].*)?$").unwrap()
});
static REPO_PLAIN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^([A-Za-z0-9_.\-]+)/([A-Za-z0-9_.\-]+?)(?:\.git)?/?$").unwrap());

const URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-').remove(b'_').remove(b'.').remove(b'!').remove(b'~').remove(b'*').remove(b'\'').remove(b'(').remove(b')');
const URI: &AsciiSet = &URI_COMPONENT
    .remove(b';').remove(b',').remove(b'/').remove(b'?').remove(b':').remove(b'@').remove(b'&').remove(b'=').remove(b'+').remove(b'$').remove(b'#');

/// Wie `encodeURIComponent` in JavaScript
pub fn encode_uri_component(s: &str) -> String {
    utf8_percent_encode(s, URI_COMPONENT).to_string()
}

fn encode_uri(s: &str) -> String {
    utf8_percent_encode(s, URI).to_string()
}

/// Branch in einer Adresse der API: Schrägstriche bleiben (heads/feature/x), der Rest wird kodiert
pub fn ref_path(branch: &str) -> String {
    branch.split('/').map(encode_uri_component).collect::<Vec<_>>().join("/")
}

// ---------- Fehler ----------

/// Antwort von GitHub mit Fehlerstatus (in JS ein Error mit `status`)
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

/// Hochladen abgelehnt, weil Dateien hier und auf GitHub verschieden geändert wurden (JS: `err.conflicts`)
#[derive(Debug, Clone)]
pub struct PushConflict {
    pub conflicts: Vec<String>,
    pub message: String,
}

impl std::fmt::Display for PushConflict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PushConflict {}

/// Wechseln abgelehnt, weil hier noch nicht hochgeladene Änderungen liegen (JS: `err.pending`)
#[derive(Debug, Clone)]
pub struct PendingChanges {
    pub pending: Vec<String>,
    pub message: String,
}

impl std::fmt::Display for PendingChanges {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PendingChanges {}

fn status_of(err: &anyhow::Error) -> Option<u16> {
    err.downcast_ref::<ApiError>().map(|e| e.status)
}

// ---------- Token ----------

/// Das verschlüsselt gespeicherte Token (JS `store: { get(), set(token) }`); die Hülle setzt es um.
pub trait TokenStore: Send + Sync {
    fn get(&self) -> String;
    fn set(&self, token: &str);
}

/// Token nur im Speicher (Tests)
#[derive(Default)]
pub struct MemoryTokenStore(Mutex<String>);

impl MemoryTokenStore {
    pub fn new(token: &str) -> Self {
        Self(Mutex::new(token.to_string()))
    }
}

impl TokenStore for MemoryTokenStore {
    fn get(&self) -> String {
        self.0.lock().clone()
    }
    fn set(&self, token: &str) {
        *self.0.lock() = token.to_string();
    }
}

/// Token über zwei Rückrufe
pub struct FnTokenStore {
    pub get: Box<dyn Fn() -> String + Send + Sync>,
    pub set: Box<dyn Fn(&str) + Send + Sync>,
}

impl TokenStore for FnTokenStore {
    fn get(&self) -> String {
        (self.get)()
    }
    fn set(&self, token: &str) {
        (self.set)(token)
    }
}

// ---------- Prüfsumme und .gitignore ----------

/// Git-Prüfsumme einer Datei, wie `git hash-object`
pub fn blob_sha(data: &[u8]) -> String {
    let mut h = Sha1::new();
    h.update(format!("blob {}\0", data.len()).as_bytes());
    h.update(data);
    hex::encode(h.finalize())
}

#[derive(Clone, Debug)]
enum Tok {
    GSlash,
    GStar,
    Star,
    Any,
    Cls { neg: bool, ranges: Vec<(char, char)> },
    Lit(char),
}

/// Eine Regel aus .gitignore
#[derive(Clone, Debug)]
pub struct IgnoreRule {
    pub base: String,
    pub negate: bool,
    pub dir_only: bool,
    tokens: Vec<Tok>,
}

impl IgnoreRule {
    pub fn matches(&self, text: &str) -> bool {
        glob_match(&self.tokens, text)
    }
}

/// Zeichenklasse aus dem Inneren von [...]: «!» oder «^» vorn kehrt um, a-z sind Bereiche.
/// Ein verdrehter Bereich (z-a) macht die Regel ungültig, wie der Fehler im regulären Ausdruck in JS.
fn parse_class(content: &[char]) -> Option<Tok> {
    let (neg, body) = match content.first() {
        Some('!') | Some('^') => (true, &content[1..]),
        _ => (false, content),
    };
    let mut ranges = Vec::new();
    let mut i = 0;
    while i < body.len() {
        if i + 2 < body.len() && body[i + 1] == '-' {
            if body[i] > body[i + 2] {
                return None;
            }
            ranges.push((body[i], body[i + 2]));
            i += 3;
        } else {
            ranges.push((body[i], body[i]));
            i += 1;
        }
    }
    Some(Tok::Cls { neg, ranges })
}

/// Eine Zeile aus .gitignore als Regel. base: Ordner der .gitignore-Datei relativ zum Arbeitsordner ('' = oben).
pub fn ignore_rule(line: &str, base: &str) -> Option<IgnoreRule> {
    let mut chars: Vec<char> = line.strip_suffix('\r').unwrap_or(line).chars().collect();
    // Nachgestellte Leerzeichen weg, ausser das erste ist mit Rückstrich geschützt
    let mut start = chars.len();
    while start > 0 && chars[start - 1].is_whitespace() {
        start -= 1;
    }
    if start < chars.len() {
        let cut = if start > 0 && chars[start - 1] == '\\' { start + 1 } else { start };
        chars.truncate(cut);
    }
    if chars.is_empty() || chars[0] == '#' {
        return None;
    }
    let negate = chars[0] == '!';
    if negate {
        chars.remove(0);
    }
    let dir_only = chars.last() == Some(&'/');
    if dir_only {
        chars.pop();
    }
    if chars.is_empty() {
        return None;
    }
    let anchored = chars.contains(&'/');
    if chars[0] == '/' {
        chars.remove(0);
    }
    let p = chars;
    // Muster als Liste von Bausteinen, geprüft ohne regulären Ausdruck mit verschachtelten Wiederholungen:
    // Das Modell schreibt .gitignore, und ein Muster wie «*a*a*a*a*b» würde den Hauptprozess sonst festfahren.
    let mut tokens: Vec<Tok> = Vec::new();
    let last_is = |tokens: &Vec<Tok>, want: &str| -> bool {
        matches!((tokens.last(), want), (Some(Tok::GSlash), "gslash") | (Some(Tok::GStar), "gstar") | (Some(Tok::Star), "star"))
    };
    if !anchored {
        tokens.push(Tok::GSlash);
    }
    let mut i = 0;
    while i < p.len() {
        let c = p[i];
        if c == '*' && p.get(i + 1) == Some(&'*') {
            let mut end = i + 2;
            while p.get(end) == Some(&'*') {
                end += 1; // mehrere * hintereinander wie eines
            }
            if p.get(end) == Some(&'/') {
                i = end;
                if !last_is(&tokens, "gslash") {
                    tokens.push(Tok::GSlash); // mehrere **/ hintereinander wie eines
                }
            } else {
                i = end - 1;
                if !last_is(&tokens, "gstar") {
                    tokens.push(Tok::GStar);
                }
            }
        } else if c == '*' {
            while p.get(i + 1) == Some(&'*') {
                i += 1;
            }
            if !last_is(&tokens, "star") {
                tokens.push(Tok::Star);
            }
        } else if c == '?' {
            tokens.push(Tok::Any);
        } else if c == '[' {
            let end = (i + 2..p.len()).find(|&k| p[k] == ']');
            match end {
                None => tokens.push(Tok::Lit('[')),
                Some(end) => {
                    tokens.push(parse_class(&p[i + 1..end])?);
                    i = end;
                }
            }
        } else if c == '\\' && i + 1 < p.len() {
            tokens.push(Tok::Lit(p[i + 1]));
            i += 1;
        } else {
            tokens.push(Tok::Lit(c));
        }
        i += 1;
    }
    Some(IgnoreRule { base: base.to_string(), negate, dir_only, tokens })
}

/// Muster gegen einen Pfad, ein Zustandsdurchlauf (Zeit Zeichen × Bausteine, kein Zurückgehen)
fn glob_match(tokens: &[Tok], text: &str) -> bool {
    let n = tokens.len();
    let mut cur = vec![false; n + 1];
    let mut next = vec![false; n + 1];
    let close = |set: &mut Vec<bool>| {
        for i in 0..n {
            if set[i] && matches!(tokens[i], Tok::Star | Tok::GStar | Tok::GSlash) {
                set[i + 1] = true;
            }
        }
    };
    cur[0] = true;
    close(&mut cur);
    for ch in text.chars() {
        next.iter_mut().for_each(|b| *b = false);
        for i in 0..n {
            if !cur[i] {
                continue;
            }
            match &tokens[i] {
                Tok::Lit(c) => {
                    if ch == *c {
                        next[i + 1] = true;
                    }
                }
                Tok::Any => {
                    if ch != '/' {
                        next[i + 1] = true;
                    }
                }
                Tok::Cls { neg, ranges } => {
                    if ch != '/' && ranges.iter().any(|(a, b)| *a <= ch && ch <= *b) != *neg {
                        next[i + 1] = true;
                    }
                }
                Tok::Star => {
                    if ch != '/' {
                        next[i] = true;
                    }
                }
                Tok::GStar => next[i] = true,
                Tok::GSlash => {
                    next[i] = true;
                    if ch == '/' {
                        next[i + 1] = true;
                    }
                }
            }
        }
        close(&mut next);
        std::mem::swap(&mut cur, &mut next);
    }
    cur[n]
}

fn read_ignore(dir: &Path, base: &str) -> Vec<IgnoreRule> {
    let Ok(bytes) = std::fs::read(dir.join(".gitignore")) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    if text.encode_utf16().count() > 200000 {
        return Vec::new();
    }
    text.split('\n').filter_map(|line| ignore_rule(line, base)).collect()
}

/// Die letzte passende Regel entscheidet, wie bei git
pub fn ignored(rules: &[IgnoreRule], rel: &str, is_dir: bool) -> bool {
    let mut hit = false;
    for r in rules {
        if r.dir_only && !is_dir {
            continue;
        }
        if !r.base.is_empty() && !rel.starts_with(&format!("{}/", r.base)) {
            continue;
        }
        let text = if r.base.is_empty() { rel } else { &rel[r.base.len() + 1..] };
        if r.matches(text) {
            hit = !r.negate;
        }
    }
    hit
}

#[derive(Clone, Debug)]
pub struct LocalFile {
    pub path: String,
    pub full: PathBuf,
}

/// Dateien im Ordner ohne .ask usw. und ohne das, was .gitignore ausschliesst. tracked: Pfade, die
/// schon abgeglichen sind; sie bleiben dabei, auch wenn .gitignore sie inzwischen nennt (wie bei git).
pub fn list_local(root: &Path, tracked: &HashSet<String>) -> Result<Vec<LocalFile>> {
    let mut tracked_dirs = HashSet::new();
    for p in tracked {
        for (i, c) in p.char_indices() {
            if c == '/' {
                tracked_dirs.insert(p[..i].to_string());
            }
        }
    }
    struct Walk<'a> {
        tracked: &'a HashSet<String>,
        tracked_dirs: &'a HashSet<String>,
        out: Vec<LocalFile>,
    }
    fn walk(w: &mut Walk, dir: &Path, base: &str, rules: &mut Vec<IgnoreRule>, parent_ignored: bool) -> Result<()> {
        let mark = rules.len();
        if !parent_ignored {
            rules.extend(read_ignore(dir, base));
        }
        let mut entries: Vec<_> = std::fs::read_dir(dir)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().to_string();
            let full = dir.join(&name);
            let rel = if base.is_empty() { name.clone() } else { format!("{base}/{name}") };
            let ft = entry.file_type()?;
            if SKIP.is_match(&rel) || ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                let ign = parent_ignored || ignored(rules, &rel, true);
                if !ign || w.tracked_dirs.contains(&rel) {
                    walk(w, &full, &rel, rules, ign)?;
                }
            } else if ft.is_file() {
                if (parent_ignored || ignored(rules, &rel, false)) && !w.tracked.contains(&rel) {
                    continue;
                }
                if w.out.len() >= MAX_FILES {
                    bail!("Mehr als {MAX_FILES} Dateien im Ordner. Das ist für den Abgleich zu viel.");
                }
                let size = std::fs::metadata(&full)?.len();
                if size > MAX_FILE {
                    bail!("{rel} ist grösser als 50 MB und kann nicht auf GitHub.");
                }
                w.out.push(LocalFile { path: rel, full });
            }
        }
        rules.truncate(mark);
        Ok(())
    }
    let mut w = Walk { tracked, tracked_dirs: &tracked_dirs, out: Vec::new() };
    walk(&mut w, root, "", &mut Vec::new(), false)?;
    Ok(w.out)
}

// ---------- Repository- und Branch-Angaben ----------

/// «besitzer/name», GitHub-Adresse (auch mit /tree/main oder /blob/… dahinter) oder git@github.com:besitzer/name.git
pub fn parse_repo(text: &str) -> Result<String> {
    let t = text.trim();
    let m = REPO_URL.captures(t).or_else(|| REPO_PLAIN.captures(t));
    match m {
        Some(m) if !(1..=2).any(|i| m[i].chars().all(|c| c == '.')) => Ok(format!("{}/{}", &m[1], &m[2])),
        _ => bail!("Repository als «besitzer/name» oder als GitHub-Adresse angeben."),
    }
}

/// Branch-Namen, die in Adressen der API und in der gespeicherten Verbindung gehen dürfen (Regeln von git, enger)
pub fn valid_branch(name: &str) -> bool {
    BRANCH.is_match(name)
        && !name.contains("..")
        && !name.contains("//")
        && !name.starts_with(['-', '.', '/'])
        && !name.ends_with(['.', '/'])
        && !LOCK.is_match(name)
        && !name.contains("/.")
        && name != "HEAD"
}

/// Der gespeicherte Stand eines Ordners (Datei github.json)
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub repo: String,
    pub branch: String,
    pub commit: Option<String>,
    /// Pfad → Git-Prüfsumme beim letzten Abgleich
    pub files: BTreeMap<String, String>,
}

impl Link {
    pub fn new(repo: &str, branch: &str) -> Self {
        Link { repo: repo.to_string(), branch: branch.to_string(), commit: None, files: BTreeMap::new() }
    }

    pub fn to_json(&self) -> Value {
        let files: serde_json::Map<String, Value> = self.files.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        json!({ "repo": self.repo, "branch": self.branch, "commit": self.commit, "files": files })
    }

    /// Gespeicherte Verbindung prüfen: Repository und Branch gehen in Adressen der GitHub-API, die Pfade in Dateinamen.
    pub fn from_json(v: &Value) -> Option<Link> {
        let repo = v.get("repo")?.as_str()?;
        if !REPO.is_match(repo) || repo.split('/').any(|part| part.chars().all(|c| c == '.')) {
            return None;
        }
        let branch = v.get("branch")?.as_str()?;
        if !valid_branch(branch) {
            return None;
        }
        let commit = match v.get("commit")? {
            Value::Null => None,
            Value::String(s) if safe::is_sha1(s) => Some(s.clone()),
            _ => return None,
        };
        let mut files = BTreeMap::new();
        for (p, sha) in v.get("files")?.as_object()? {
            let sha = sha.as_str()?;
            if !safe::safe_rel(p) || !safe::is_sha1(sha) {
                return None;
            }
            files.insert(p.clone(), sha.to_string());
        }
        Some(Link { repo: repo.to_string(), branch: branch.to_string(), commit, files })
    }
}

/// Gespeicherte Verbindung prüfen (siehe `Link::from_json`)
pub fn valid_link(v: &Value) -> bool {
    Link::from_json(v).is_some()
}

/// Stand auf GitHub
#[derive(Clone, Debug, Default)]
pub struct Remote {
    /// null bei einem leeren Repository
    pub commit: Option<String>,
    pub files: BTreeMap<String, String>,
    /// Art der Datei (ausführbar, Verknüpfung)
    pub modes: BTreeMap<String, String>,
    /// Einträge, die keine Dateien sind (Submodule)
    pub others: Vec<Value>,
}

/// Was hat sich seit dem letzten Abgleich hier geändert?
#[derive(Clone, Debug, Default)]
pub struct LocalChanges {
    pub local: BTreeMap<String, String>,
    pub changed: Vec<String>,
    pub added: Vec<String>,
    pub deleted: Vec<String>,
}

// ---------- GitHub ----------

type LinkFileFn = Arc<dyn Fn(&Path) -> PathBuf + Send + Sync>;

struct BusyGuard<'a> {
    busy: &'a Mutex<HashSet<String>>,
    key: String,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.busy.lock().remove(&self.key);
    }
}

pub struct GitHub {
    api: String,
    web: String,
    store: Arc<dyn TokenStore>,
    client: reqwest::Client,
    timeout_ms: u64,
    link_file: LinkFileFn,
    login: Mutex<String>,
    /// Ordner, in denen der Bereich «GitHub» gerade abgleicht (main.js)
    busy: Mutex<HashSet<String>>,
}

impl GitHub {
    /// store: das verschlüsselt gespeicherte Token. Standard: api.github.com, Verbindung in `<ordner>/.ask/github.json`
    /// (in der App wird `with_link_file` auf `paths::project_data_dir` gesetzt).
    pub fn new(store: Arc<dyn TokenStore>) -> Self {
        GitHub {
            api: "https://api.github.com".into(),
            web: "https://github.com".into(),
            store,
            client: reqwest::Client::new(),
            timeout_ms: REQUEST_MS,
            link_file: Arc::new(|root| root.join(".ask").join("github.json")),
            login: Mutex::new(String::new()),
            busy: Mutex::new(HashSet::new()),
        }
    }

    pub fn with_api(mut self, api: &str) -> Self {
        self.api = api.trim_end_matches('/').to_string();
        self
    }

    pub fn with_web(mut self, web: &str) -> Self {
        self.web = web.trim_end_matches('/').to_string();
        self
    }

    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn with_timeout(mut self, ms: u64) -> Self {
        self.timeout_ms = ms;
        self
    }

    /// Wo die Verbindung eines Ordners gespeichert ist (in der App ausserhalb des Ordners)
    pub fn with_link_file(mut self, f: impl Fn(&Path) -> PathBuf + Send + Sync + 'static) -> Self {
        self.link_file = Arc::new(f);
        self
    }

    pub fn link_file(&self, root: &Path) -> PathBuf {
        (self.link_file)(root)
    }

    /// Ein Abgleich aus dem Bereich «GitHub» zur Zeit pro Ordner; das Modell wartet so lange (tools.js)
    pub fn key(root: &Path) -> String {
        crate::util::path_key(&root.to_string_lossy())
    }

    pub fn locked(&self, root: &Path) -> bool {
        self.busy.lock().contains(&Self::key(root))
    }

    /// Führt `f` aus, solange der Ordner für den Bereich «GitHub» reserviert ist. Ist er es schon, schlägt der
    /// Aufruf fehl (wie in JS; es wird nicht gewartet). Die Reservierung endet auch, wenn `f` abgebrochen wird.
    pub async fn exclusive<T, F, Fut>(&self, root: &Path, f: F) -> Result<T>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T>>,
    {
        let key = Self::key(root);
        if !self.busy.lock().insert(key.clone()) {
            bail!("Ein Abgleich mit GitHub läuft schon.");
        }
        let _guard = BusyGuard { busy: &self.busy, key };
        f().await
    }

    pub fn connected(&self) -> bool {
        !self.store.get().is_empty()
    }

    pub async fn request(&self, method: &str, url: &str, body: Option<Value>) -> Result<Value> {
        self.request_with(method, url, body, Duration::from_millis(self.timeout_ms)).await
    }

    pub async fn request_with(&self, method: &str, url: &str, body: Option<Value>, timeout: Duration) -> Result<Value> {
        let token = self.store.get();
        if token.is_empty() {
            bail!("Nicht mit GitHub verbunden. Die Person kann in der App mit /github ein Token eintragen.");
        }
        let m = reqwest::Method::from_bytes(method.as_bytes())?;
        let mut req = self
            .client
            .request(m, format!("{}{}", self.api, url))
            .timeout(timeout)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .header("User-Agent", "ASK-Desktop");
        if let Some(b) = &body {
            req = req.header("Content-Type", "application/json").body(serde_json::to_vec(b)?);
        }
        let timed_out = |ms: u128| {
            anyhow!(
                "GitHub antwortet nicht (nach {} s abgebrochen). Internetverbindung prüfen und nochmals versuchen.",
                (ms as f64 / 1000.0).round() as i64
            )
        };
        let res = match req.send().await {
            Ok(r) => r,
            Err(e) if e.is_timeout() => return Err(timed_out(timeout.as_millis())),
            Err(e) => return Err(e.into()),
        };
        let status = res.status();
        let text = match res.text().await {
            Ok(t) => t,
            Err(e) if e.is_timeout() => return Err(timed_out(timeout.as_millis())),
            Err(e) => return Err(e.into()),
        };
        let data: Value = if text.is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) };
        if !status.is_success() {
            // Bei 422 steht der Grund oft erst in errors (z. B. «A pull request already exists»)
            let detail = match data.get("errors") {
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|e| e.get("message").filter(|m| truthy(m)).map(js_string))
                    .collect::<Vec<_>>()
                    .join("; "),
                _ => String::new(),
            };
            let first = data.get("message").filter(|m| truthy(m)).map(js_string).unwrap_or_default();
            let mut message = [first, detail].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(": ");
            if message.is_empty() {
                message = take_chars(&text, 200);
            }
            if message.is_empty() {
                message = format!("HTTP {}", status.as_u16());
            }
            if status.as_u16() == 401 {
                bail!("GitHub lehnt das Token ab (abgelaufen oder ungültig). Mit /github neu verbinden.");
            }
            return Err(ApiError { status: status.as_u16(), message: format!("GitHub: {message}") }.into());
        }
        Ok(data)
    }

    /// Token prüfen und speichern; bei Fehler bleibt das alte Token. Ergebnis: Benutzername.
    pub async fn connect(&self, token: &str) -> Result<String> {
        let old = self.store.get();
        self.store.set(token.trim());
        match self.request("GET", "/user", None).await {
            Ok(user) => {
                let login = str_of(&user, "login");
                *self.login.lock() = login.clone();
                Ok(login)
            }
            Err(err) => {
                self.store.set(&old);
                Err(err)
            }
        }
    }

    pub fn disconnect(&self) {
        self.store.set("");
        self.login.lock().clear();
    }

    pub async fn whoami(&self) -> Result<String> {
        if self.login.lock().is_empty() && self.connected() {
            let user = self.request("GET", "/user", None).await?;
            *self.login.lock() = str_of(&user, "login");
        }
        Ok(self.login.lock().clone())
    }

    pub fn read_link(&self, root: &Path) -> Option<Link> {
        let text = std::fs::read(self.link_file(root)).ok()?;
        let v: Value = serde_json::from_slice(&text).ok()?;
        Link::from_json(&v)
    }

    pub fn write_link(&self, root: &Path, link: &Link) -> Result<()> {
        let file = self.link_file(root);
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(file, serde_json::to_string_pretty(&link.to_json())?)?;
        Ok(())
    }

    pub fn url(&self, link: &Link) -> String {
        format!("{}/{}", self.web, link.repo)
    }

    /// Stand auf GitHub: Commit, Dateien (Pfad → Prüfsumme), ihre Art (modes: ausführbar, Verknüpfung) und
    /// Einträge, die keine Dateien sind (others: Submodule). Leeres Repository: commit None.
    pub async fn remote(&self, link: &Link) -> Result<Remote> {
        let head = match self.request("GET", &format!("/repos/{}/git/ref/heads/{}", link.repo, ref_path(&link.branch)), None).await {
            Ok(r) => r["object"]["sha"].as_str().map(str::to_string).ok_or_else(|| anyhow!("GitHub: Antwort ohne Commit"))?,
            Err(err) if matches!(status_of(&err), Some(404 | 409)) => return self.no_branch(link, &err).await,
            Err(err) => return Err(err),
        };
        let tree = self.request("GET", &format!("/repos/{}/git/trees/{}?recursive=1", link.repo, head), None).await?;
        if truthy(&tree["truncated"]) {
            bail!("Das Repository ist für den Abgleich zu gross.");
        }
        let mut out = Remote { commit: Some(head), ..Default::default() };
        for e in tree["tree"].as_array().cloned().unwrap_or_default() {
            let (path, sha) = (str_of(&e, "path"), str_of(&e, "sha"));
            match e["type"].as_str() {
                Some("blob") => {
                    out.modes.insert(path.clone(), str_of(&e, "mode"));
                    out.files.insert(path, sha);
                }
                Some("commit") => {
                    let mode = e.get("mode").filter(|m| truthy(m)).map(js_string).unwrap_or_else(|| "160000".into());
                    out.others.push(json!({ "path": path, "mode": mode, "type": "commit", "sha": sha }));
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Der Branch liess sich nicht lesen. Nur ein Repository ohne jeden Branch ist wirklich leer; sonst wurde der
    /// Branch gelöscht (z. B. nach einem Pull Request), das Repository umbenannt oder das Token verlor den Zugriff.
    /// Ein leerer Stand würde beim Holen alle Dateien hier löschen.
    async fn no_branch(&self, link: &Link, cause: &anyhow::Error) -> Result<Remote> {
        let list = async {
            self.request("GET", &format!("/repos/{}", link.repo), None).await?;
            self.request("GET", &format!("/repos/{}/branches?per_page=1", link.repo), None).await
        }
        .await;
        let list = match list {
            Ok(l) => l,
            Err(err) if status_of(&err) == Some(404) => bail!(
                "Das Repository {} gibt es auf GitHub nicht mehr (umbenannt oder gelöscht) oder das Token hat keinen Zugriff darauf.",
                link.repo
            ),
            Err(err) => return Err(err),
        };
        if list.as_array().is_some_and(|a| a.is_empty()) && matches!(status_of(cause), Some(404 | 409)) {
            return Ok(Remote::default());
        }
        bail!("Den Branch {} gibt es auf GitHub nicht mehr. Wähle im Reiter GitHub einen anderen Branch.", link.branch)
    }

    /// Was hat sich seit dem letzten Abgleich hier geändert?
    pub fn local_changes(&self, root: &Path, link: Option<&Link>) -> Result<LocalChanges> {
        let empty = BTreeMap::new();
        let known = link.map(|l| &l.files).unwrap_or(&empty);
        let tracked: HashSet<String> = known.keys().cloned().collect();
        let mut local = BTreeMap::new();
        for f in list_local(root, &tracked)? {
            local.insert(f.path, blob_sha(&std::fs::read(&f.full)?));
        }
        let changed = local.iter().filter(|(p, sha)| known.get(*p).is_some_and(|k| k != *sha)).map(|(p, _)| p.clone()).collect();
        let added = local.keys().filter(|p| !known.contains_key(*p)).cloned().collect();
        let deleted = known.keys().filter(|p| !local.contains_key(*p) && !SKIP.is_match(p)).cloned().collect();
        Ok(LocalChanges { local, changed, added, deleted })
    }

    /// Dateien, die seit dem letzten Abgleich hier und auf GitHub verschieden geändert wurden.
    /// Hochladen würde die Fassung auf GitHub ersetzen.
    pub fn conflicts(&self, link: Option<&Link>, local: &BTreeMap<String, String>, remote: &Remote) -> Vec<String> {
        let empty = BTreeMap::new();
        let known = link.map(|l| &l.files).unwrap_or(&empty);
        local
            .iter()
            .filter(|(p, sha)| {
                remote.files.get(*p).is_some_and(|r| r != *sha && Some(r) != known.get(*p) && known.get(*p) != Some(*sha))
            })
            .map(|(p, _)| p.clone())
            .collect()
    }

    pub async fn status(&self, root: &Path) -> Result<Value> {
        let Some(link) = self.read_link(root) else {
            let login = if self.connected() { self.whoami().await? } else { String::new() };
            return Ok(json!({ "linked": false, "connected": self.connected(), "login": login }));
        };
        let ch = self.local_changes(root, Some(&link))?;
        let remote = self.remote(&link).await?;
        Ok(json!({
            "linked": true, "connected": true, "login": self.whoami().await?, "repo": link.repo, "branch": link.branch,
            "url": self.url(&link), "changed": ch.changed, "added": ch.added, "deleted": ch.deleted,
            "remoteNewer": remote.commit.is_some() && remote.commit != link.commit,
            "conflicts": self.conflicts(Some(&link), &ch.local, &remote),
        }))
    }

    pub async fn create(&self, root: &Path, name: &str, privat: bool, beschreibung: &str) -> Result<Value> {
        if let Some(l) = self.read_link(root) {
            bail!("Der Ordner ist schon mit {} verbunden.", l.repo);
        }
        let repo = self
            .request("POST", "/user/repos", Some(json!({ "name": name, "private": privat, "description": beschreibung, "auto_init": true })))
            .await?;
        let branch = repo.get("default_branch").filter(|b| truthy(b)).map(js_string).unwrap_or_else(|| "main".into());
        let link = Link::new(&str_of(&repo, "full_name"), &branch);
        // auto_init legt eine README an: zuerst holen, dann den Ordner hochladen
        self.write_link(root, &link)?;
        self.pull(root).await?;
        // Die automatische README weicht einer eigenen: Das Repository ist eben erst angelegt worden
        let mut pushed = self.push(root, "Projekt aus ASK hochgeladen", true).await?;
        let o = pushed.as_object_mut().expect("Objekt");
        o.insert("repo".into(), json!(link.repo));
        o.insert("url".into(), json!(self.url(&link)));
        o.insert("private".into(), repo["private"].clone());
        Ok(pushed)
    }

    /// Repository in den Ordner holen (JS `clone`)
    pub async fn clone_repo(&self, root: &Path, repo_text: &str) -> Result<Value> {
        let repo_name = parse_repo(repo_text)?;
        let existing = self.read_link(root);
        if let Some(e) = &existing {
            if e.repo.to_lowercase() != repo_name.to_lowercase() {
                bail!("Der Ordner ist schon mit {} verbunden. Für ein anderes Repository einen neuen Ordner nehmen.", e.repo);
            }
        }
        let info = self.request("GET", &format!("/repos/{repo_name}"), None).await?;
        let full_name = str_of(&info, "full_name");
        let branch = info.get("default_branch").filter(|b| truthy(b)).map(js_string).unwrap_or_else(|| "main".into());
        self.write_link(root, &existing.unwrap_or_else(|| Link::new(&full_name, &branch)))?;
        let mut r = self.pull(root).await?;
        let o = r.as_object_mut().expect("Objekt");
        o.insert("repo".into(), json!(full_name));
        o.insert("url".into(), json!(self.url(&Link::new(&full_name, &branch))));
        Ok(r)
    }

    /// Änderungen von GitHub holen. Hier geänderte Dateien werden nie überschrieben (Konflikt).
    pub async fn pull(&self, root: &Path) -> Result<Value> {
        let link = self
            .read_link(root)
            .ok_or_else(|| anyhow!("Der Ordner ist mit keinem GitHub-Repository verbunden (github_clone oder github_create)."))?;
        let remote = self.remote(&link).await?;
        let ch = self.local_changes(root, Some(&link))?;
        let known = &link.files;
        let mut files = known.clone();
        let (mut written, mut removed, mut conflicts) = (Vec::new(), Vec::new(), Vec::new());
        for (p, sha) in &remote.files {
            if SKIP.is_match(p) {
                continue;
            }
            // Pfade, die aus dem Ordner hinausführen oder die Windows anders schreibt («.ask./»): nie anlegen
            let Ok(full) = safe::safe_join(root, p) else {
                conflicts.push(p.clone());
                continue;
            };
            let mut mine = ch.local.get(p).cloned();
            // Liegt hier eine Datei, die der Abgleich nicht kennt (von .gitignore ausgeschlossen): nicht überschreiben
            if mine.is_none() && !known.contains_key(p) {
                if let Ok(b) = std::fs::read(&full) {
                    mine = Some(blob_sha(&b));
                }
            }
            if mine.as_ref() == Some(sha) {
                files.insert(p.clone(), sha.clone());
                continue;
            }
            // Hier gelöscht, auf GitHub unverändert: gelöscht lassen (wird beim Hochladen auch dort gelöscht)
            if mine.is_none() && known.get(p) == Some(sha) {
                continue;
            }
            if mine.is_none() || mine.as_ref() == known.get(p) {
                let blob = self
                    .request_with(
                        "GET",
                        &format!("/repos/{}/git/blobs/{}", link.repo, sha),
                        None,
                        Duration::from_millis(self.timeout_ms.max(BLOB_MS)),
                    )
                    .await?;
                let content: String = str_of(&blob, "content").chars().filter(|c| !c.is_whitespace()).collect();
                let bytes = base64_decode(&content)?;
                if let Some(dir) = full.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                std::fs::write(&full, bytes)?;
                files.insert(p.clone(), sha.clone());
                written.push(p.clone());
            } else if known.get(p) != Some(sha) {
                conflicts.push(p.clone()); // hier und auf GitHub verschieden geändert
            }
        }
        // Ohne Commit auf GitHub (leeres Repository) wird hier nie etwas gelöscht
        if remote.commit.is_some() {
            for p in known.keys() {
                if remote.files.contains_key(p) || SKIP.is_match(p) {
                    continue;
                }
                let mine = ch.local.get(p);
                if mine.is_none() || mine == known.get(p) {
                    if mine.is_some() {
                        let _ = std::fs::remove_file(safe::safe_join(root, p)?);
                    }
                    files.remove(p);
                    if mine.is_some() {
                        removed.push(p.clone());
                    }
                } else {
                    conflicts.push(p.clone());
                }
            }
        }
        self.write_link(root, &Link { commit: remote.commit.clone(), files, ..link.clone() })?;
        Ok(json!({ "written": written, "removed": removed, "conflicts": conflicts, "commit": remote.commit }))
    }

    /// Ordner als Commit hochladen. Gibt es auf GitHub Neueres, zuerst pull. Dateien, die hier und auf
    /// GitHub verschieden geändert wurden, ersetzt das Hochladen nur mit overwrite (nach Rückfrage, tools.js).
    pub async fn push(&self, root: &Path, message: &str, overwrite: bool) -> Result<Value> {
        let link = self
            .read_link(root)
            .ok_or_else(|| anyhow!("Der Ordner ist mit keinem GitHub-Repository verbunden (github_create oder github_clone)."))?;
        let remote = self.remote(&link).await?;
        if remote.commit.is_some() && remote.commit != link.commit {
            bail!("Auf GitHub gibt es neuere Änderungen. Zuerst github_pull, dann nochmals hochladen.");
        }
        let ch = self.local_changes(root, Some(&link))?;
        let local = &ch.local;
        if remote.commit.is_none() {
            self.init_empty(&link, root, local).await?;
        }
        let base = if remote.commit.is_some() { remote } else { self.remote(&link).await? };
        let clash = self.conflicts(Some(&link), local, &base);
        if !clash.is_empty() && !overwrite {
            return Err(PushConflict {
                message: format!(
                    "Hier und auf GitHub verschieden geändert: {}. Hochladen würde die Fassung auf GitHub ersetzen. \
                     Die Person entscheiden lassen: Soll GitHub gewinnen, die Datei hier löschen und github_pull; \
                     soll die Fassung hier gewinnen, github_push mit ueberschreiben: true.",
                    clash.iter().take(15).cloned().collect::<Vec<_>>().join(", ")
                ),
                conflicts: clash,
            }
            .into());
        }
        let mode_of = |p: &str| base.modes.get(p).filter(|m| !m.is_empty()).cloned().unwrap_or_else(|| "100644".into());
        let mut tree: Vec<Value> = Vec::new();
        let mut uploaded = 0;
        for (p, sha) in local {
            if base.files.get(p) != Some(sha) {
                let data = std::fs::read(root.join(p.split('/').collect::<PathBuf>()))?;
                self.request_with(
                    "POST",
                    &format!("/repos/{}/git/blobs", link.repo),
                    Some(json!({ "content": base64_encode(&data), "encoding": "base64" })),
                    Duration::from_millis(self.timeout_ms.max(BLOB_MS)),
                )
                .await?;
                uploaded += 1;
            }
            tree.push(json!({ "path": p, "mode": mode_of(p), "type": "blob", "sha": sha })); // ausführbar bleibt ausführbar
        }
        // Auf GitHub liegende Dateien in ausgeschlossenen Ordnern (z. B. .github) und Submodule bleiben erhalten
        for (p, sha) in &base.files {
            if SKIP.is_match(p) {
                tree.push(json!({ "path": p, "mode": mode_of(p), "type": "blob", "sha": sha }));
            }
        }
        tree.extend(base.others.iter().cloned());
        // Gelöscht wird auf GitHub nur, was abgeglichen war und hier fehlt. Was der Abgleich nie geholt hat
        // (Konflikt beim Holen, hier von .gitignore ausgeschlossen), bleibt auf GitHub, wie es ist.
        let known = &link.files;
        let gone = |p: &str| !SKIP.is_match(p) && !local.contains_key(p);
        for (p, sha) in &base.files {
            if gone(p) && !known.contains_key(p) {
                tree.push(json!({ "path": p, "mode": mode_of(p), "type": "blob", "sha": sha }));
            }
        }
        let deleted: Vec<String> = base.files.keys().filter(|p| gone(p) && known.contains_key(*p)).cloned().collect();
        if uploaded == 0 && deleted.is_empty() {
            self.write_link(root, &Link { commit: base.commit.clone(), files: local.clone(), ..link.clone() })?;
            return Ok(json!({ "commit": base.commit, "uploaded": 0, "deleted": [], "nothing": true, "url": self.url(&link) }));
        }
        let new_tree = self.request("POST", &format!("/repos/{}/git/trees", link.repo), Some(json!({ "tree": tree }))).await?;
        let msg = if message.is_empty() { "Änderungen aus ASK" } else { message };
        let commit = self
            .request(
                "POST",
                &format!("/repos/{}/git/commits", link.repo),
                Some(json!({ "message": msg, "tree": new_tree["sha"], "parents": [base.commit] })),
            )
            .await?;
        let sha = str_of(&commit, "sha");
        self.request("PATCH", &format!("/repos/{}/git/refs/heads/{}", link.repo, ref_path(&link.branch)), Some(json!({ "sha": sha }))).await?;
        self.write_link(root, &Link { commit: Some(sha.clone()), files: local.clone(), ..link.clone() })?;
        Ok(json!({ "commit": sha, "uploaded": uploaded, "deleted": deleted, "url": format!("{}/commit/{}", self.url(&link), sha) }))
    }

    // ---------- Branches ----------

    pub fn linked(&self, root: &Path) -> Result<Link> {
        self.read_link(root)
            .ok_or_else(|| anyhow!("Der Ordner ist mit keinem GitHub-Repository verbunden (github_clone oder github_create)."))
    }

    /// Branches des Repositorys (bis 100), dazu der Standard-Branch und der des Ordners
    pub async fn branches(&self, root: &Path) -> Result<Value> {
        let link = self.linked(root)?;
        let (info_url, list_url) = (format!("/repos/{}", link.repo), format!("/repos/{}/branches?per_page=100", link.repo));
        let (info, list) = tokio::try_join!(self.request("GET", &info_url, None), self.request("GET", &list_url, None))?;
        let mut names: Vec<String> =
            list.as_array().map(|a| a.iter().map(|b| str_of(b, "name")).filter(|n| !n.is_empty()).collect()).unwrap_or_default();
        if !names.contains(&link.branch) {
            names.push(link.branch.clone()); // noch nie hochgeladen (leeres Repository)
        }
        let def = info.get("default_branch").filter(|b| truthy(b)).map(js_string).unwrap_or_else(|| "main".into());
        names.sort_by(|a, b| {
            if *a == def {
                std::cmp::Ordering::Less
            } else if *b == def {
                std::cmp::Ordering::Greater
            } else {
                natural_cmp(a, b)
            }
        });
        Ok(json!({ "current": link.branch, "default": def, "branches": names, "url": self.url(&link) }))
    }

    /// Neuer Branch auf dem Stand des letzten Abgleichs, danach ist der Ordner mit ihm verbunden. Änderungen
    /// hier bleiben liegen und gehen beim nächsten Hochladen auf den neuen Branch.
    pub async fn create_branch(&self, root: &Path, name: &str) -> Result<Value> {
        let link = self.linked(root)?;
        let branch = name.trim().to_string();
        if !valid_branch(&branch) {
            bail!("«{branch}» geht nicht als Branch-Name. Erlaubt sind Buchstaben, Ziffern, - _ . und /, z. B. «neue-startseite».");
        }
        if branch == link.branch {
            bail!("Der Ordner ist schon mit dem Branch {branch} verbunden.");
        }
        let Some(commit) = link.commit.clone() else {
            bail!("Auf GitHub gibt es noch keinen Stand, von dem ein Branch abzweigen kann. Zuerst hochladen.");
        };
        if let Err(err) = self
            .request("POST", &format!("/repos/{}/git/refs", link.repo), Some(json!({ "ref": format!("refs/heads/{branch}"), "sha": commit })))
            .await
        {
            if status_of(&err) == Some(422) {
                bail!("Den Branch {branch} gibt es auf GitHub schon. Mit «wechseln» zu ihm wechseln.");
            }
            return Err(err);
        }
        self.write_link(root, &Link { branch: branch.clone(), ..link.clone() })?;
        Ok(json!({ "branch": branch, "from": link.branch, "commit": commit, "url": format!("{}/tree/{}", self.url(&link), ref_path(&branch)) }))
    }

    /// Zu einem anderen Branch wechseln: Die Dateien im Ordner bekommen dessen Stand. Nur ohne Änderungen
    /// hier, sonst gingen sie im Stand des anderen Branches unter (oder würden später dorthin hochgeladen).
    pub async fn switch_branch(&self, root: &Path, name: &str) -> Result<Value> {
        let link = self.linked(root)?;
        let branch = name.trim().to_string();
        if !valid_branch(&branch) {
            bail!("«{branch}» geht nicht als Branch-Name.");
        }
        if branch == link.branch {
            return Ok(json!({ "branch": branch, "written": [], "removed": [], "conflicts": [], "same": true }));
        }
        let ch = self.local_changes(root, Some(&link))?;
        let pending: Vec<String> = ch.changed.iter().chain(&ch.added).chain(&ch.deleted).cloned().collect();
        if !pending.is_empty() {
            let n = pending.len();
            let list = pending.iter().take(8).cloned().collect::<Vec<_>>().join(", ");
            return Err(PendingChanges {
                message: format!(
                    "Nicht gewechselt: Hier gibt es {n} {}, die noch nicht hochgeladen {} ({list}{}). \
                     Beim Wechseln bekäme der Ordner den Stand von {branch}, und die Änderungen würden vermischt oder gingen später auf den falschen Branch. \
                     Zuerst auf {} hochladen oder dafür einen neuen Branch anlegen.",
                    if n == 1 { "Änderung" } else { "Änderungen" },
                    if n == 1 { "ist" } else { "sind" },
                    if n > 8 { " …" } else { "" },
                    link.branch
                ),
                pending,
            }
            .into());
        }
        // Den Branch muss es geben, sonst sähe der Abgleich ein leeres Repository und würde alles löschen
        if let Err(err) = self.request("GET", &format!("/repos/{}/git/ref/heads/{}", link.repo, ref_path(&branch)), None).await {
            if matches!(status_of(&err), Some(404 | 409)) {
                bail!("Den Branch {branch} gibt es auf GitHub nicht.");
            }
            return Err(err);
        }
        self.write_link(root, &Link { branch: branch.clone(), ..link.clone() })?;
        let mut r = self.pull(root).await?;
        // { branch, from, ...r }
        let mut out = serde_json::Map::new();
        out.insert("branch".into(), json!(branch));
        out.insert("from".into(), json!(link.branch));
        out.extend(r.as_object_mut().expect("Objekt").clone());
        Ok(Value::Object(out))
    }

    // ---------- Pull Requests ----------

    pub async fn pull_requests(&self, root: &Path) -> Result<Value> {
        let link = self.linked(root)?;
        let list = self.request("GET", &format!("/repos/{}/pulls?state=open&per_page=30", link.repo), None).await?;
        let out: Vec<Value> = list
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|p| {
                        json!({
                            "number": p["number"], "title": str_of(p, "title"), "url": str_of(p, "html_url"),
                            "head": str_of(&p["head"], "ref"), "base": str_of(&p["base"], "ref"),
                            "user": str_of(&p["user"], "login"), "draft": truthy(&p["draft"]),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(Value::Array(out))
    }

    /// Pull Request vom Branch des Ordners in den Standard-Branch (oder base). Enthält nur, was auf GitHub liegt.
    pub async fn create_pull_request(&self, root: &Path, title: &str, body: &str, base: &str) -> Result<Value> {
        let link = self.linked(root)?;
        let t = title.trim();
        if t.is_empty() {
            bail!("Der Pull Request braucht einen Titel.");
        }
        let target = if !base.trim().is_empty() {
            base.trim().to_string()
        } else {
            let info = self.request("GET", &format!("/repos/{}", link.repo), None).await?;
            info.get("default_branch").filter(|b| truthy(b)).map(js_string).unwrap_or_else(|| "main".into())
        };
        if !valid_branch(&target) {
            bail!("«{target}» geht nicht als Branch-Name.");
        }
        if target == link.branch {
            bail!("Der Ordner ist mit {target} verbunden, dem Ziel des Pull Requests. Zuerst einen eigenen Branch anlegen, dort hochladen und dann den Pull Request erstellen.");
        }
        let pr = self
            .request(
                "POST",
                &format!("/repos/{}/pulls", link.repo),
                Some(json!({ "title": take_chars(t, 250), "body": take_chars(body, 20000), "head": link.branch, "base": target })),
            )
            .await?;
        Ok(json!({ "number": pr["number"], "url": pr["html_url"], "head": link.branch, "base": target }))
    }

    /// In ein leeres Repository lässt sich über die Git-API nicht schreiben: erste Datei über die Contents-API
    async fn init_empty(&self, link: &Link, root: &Path, local: &BTreeMap<String, String>) -> Result<()> {
        let first = local.keys().next();
        let content = match first {
            Some(f) => base64_encode(&std::fs::read(root.join(f.split('/').collect::<PathBuf>()))?),
            None => base64_encode(b"# Projekt\n"),
        };
        self.request_with(
            "PUT",
            &format!("/repos/{}/contents/{}", link.repo, encode_uri(first.map(String::as_str).unwrap_or("README.md"))),
            Some(json!({ "message": "Erste Datei", "content": content, "branch": link.branch })),
            Duration::from_millis(self.timeout_ms.max(BLOB_MS)),
        )
        .await?;
        Ok(())
    }
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn base64_decode(text: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    Ok(base64::engine::general_purpose::STANDARD.decode(text)?)
}

/// Sortierung wie `localeCompare(…, 'de', { numeric: true })`, soweit für Branch-Namen nötig:
/// Zahlen nach Wert, Buchstaben ohne Rücksicht auf Gross- und Kleinschreibung (klein vor gross bei Gleichstand).
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    fn chunks(s: &str) -> Vec<(bool, String)> {
        let mut out: Vec<(bool, String)> = Vec::new();
        for c in s.chars() {
            let d = c.is_ascii_digit();
            match out.last_mut() {
                Some((kind, text)) if *kind == d => text.push(c),
                _ => out.push((d, c.to_string())),
            }
        }
        out
    }
    let (ca, cb) = (chunks(a), chunks(b));
    for (x, y) in ca.iter().zip(&cb) {
        let o = if x.0 && y.0 {
            let (nx, ny) = (x.1.trim_start_matches('0'), y.1.trim_start_matches('0'));
            nx.len().cmp(&ny.len()).then_with(|| nx.cmp(ny))
        } else {
            x.1.to_lowercase().cmp(&y.1.to_lowercase())
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    ca.len().cmp(&cb.len()).then_with(|| b.cmp(a))
}

#[cfg(test)]
pub(crate) mod test_http {
    //! Kleiner HTTP-Server für Tests (statt Netzwerk): eine Anfrage pro Verbindung.
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::task::JoinHandle;

    pub struct Req {
        pub method: String,
        pub path: String,
        pub headers: HashMap<String, String>,
        pub body: Vec<u8>,
    }

    pub enum Reply {
        Json(u16, String),
        /// Verbindung ohne Antwort schliessen
        Drop,
        /// Nie antworten
        Hang,
    }

    pub type Handler = Arc<dyn Fn(Req) -> Reply + Send + Sync>;

    pub async fn serve_at(addr: &str, handler: Handler) -> std::io::Result<JoinHandle<()>> {
        let listener = TcpListener::bind(addr).await?;
        Ok(tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else { break };
                let h = handler.clone();
                tokio::spawn(async move {
                    let _ = conn(stream, h).await;
                });
            }
        }))
    }

    /// Server auf einem freien Port; Ergebnis: Adresse wie `http://127.0.0.1:1234`
    pub async fn serve(handler: Handler) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let task = serve_at(&format!("127.0.0.1:{port}"), handler).await.unwrap();
        (format!("http://127.0.0.1:{port}"), task)
    }

    async fn conn(mut s: TcpStream, h: Handler) -> std::io::Result<()> {
        let mut buf = Vec::new();
        let mut tmp = [0u8; 8192];
        let head_end = loop {
            if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break i;
            }
            let n = s.read(&mut tmp).await?;
            if n == 0 {
                return Ok(());
            }
            buf.extend_from_slice(&tmp[..n]);
        };
        let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
        let mut lines = head.split("\r\n");
        let mut first = lines.next().unwrap_or("").split(' ');
        let method = first.next().unwrap_or("").to_string();
        let path = first.next().unwrap_or("").to_string();
        let headers: HashMap<String, String> = lines
            .filter_map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())))
            .collect();
        let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
        let mut body = buf[head_end + 4..].to_vec();
        while body.len() < len {
            let n = s.read(&mut tmp).await?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&tmp[..n]);
        }
        match h(Req { method, path, headers, body }) {
            Reply::Json(status, text) => {
                let out = format!(
                    "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                s.write_all(out.as_bytes()).await?;
                s.shutdown().await?;
            }
            Reply::Drop => {}
            Reply::Hang => std::future::pending::<()>().await,
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::test_http::*;
    use super::*;
    use std::collections::HashMap;
    use tokio::task::JoinHandle;

    pub(crate) const TOKEN: &str = "gh-test-token";
    const LOGIN: &str = "lea-test";

    fn sha1_hex(data: &[u8]) -> String {
        hex::encode(Sha1::digest(data))
    }

    // Nachbau der GitHub-REST-API, die github.rs benutzt (Repositories, Git-Daten, Contents, Branches,
    // Pull Requests), alles im Speicher (wie tests/e2e/mock-github.js).
    struct Repo {
        name: String,
        private: bool,
        branch: String,
        blobs: HashMap<String, Vec<u8>>,
        trees: HashMap<String, Vec<Value>>,
        commits: HashMap<String, (String, Vec<String>, String)>,
        refs: BTreeMap<String, String>,
        pulls: Vec<Value>,
    }

    #[derive(Default)]
    struct MockState {
        repos: HashMap<String, Repo>,
        counter: u64,
    }

    #[derive(Default)]
    pub(crate) struct Mock {
        state: Mutex<MockState>,
    }

    fn add_blob(repo: &mut Repo, data: &[u8]) -> String {
        let sha = blob_sha(data);
        repo.blobs.insert(sha.clone(), data.to_vec());
        sha
    }

    fn commit_files(repo: &mut Repo, counter: &mut u64, files: &BTreeMap<String, String>, message: &str, parents: &[String]) -> String {
        let entries: Vec<Value> = files.iter().map(|(p, sha)| json!({ "path": p, "mode": "100644", "type": "blob", "sha": sha })).collect();
        let tree = sha1_hex(serde_json::to_string(&entries).unwrap().as_bytes());
        repo.trees.insert(tree.clone(), entries);
        *counter += 1;
        let sha = sha1_hex(format!("{tree}{message}{}{counter}", parents.join(",")).as_bytes());
        repo.commits.insert(sha.clone(), (tree, parents.to_vec(), message.to_string()));
        sha
    }

    fn tree_files(repo: &Repo, head: &str) -> BTreeMap<String, String> {
        repo.trees[&repo.commits[head].0].iter().map(|e| (str_of(e, "path"), str_of(e, "sha"))).collect()
    }

    fn dec(s: &str) -> String {
        percent_encoding::percent_decode_str(s).decode_utf8_lossy().to_string()
    }

    impl Mock {
        fn handle(&self, method: &str, url: &str, auth: &str, body: &Value) -> (u16, Value) {
            if auth != format!("Bearer {TOKEN}") {
                return (401, json!({ "message": "Bad credentials" }));
            }
            let mut guard = self.state.lock();
            let MockState { repos, counter } = &mut *guard;
            let (p, query) = url.split_once('?').unwrap_or((url, ""));
            if method == "GET" && p == "/user" {
                return (200, json!({ "login": LOGIN }));
            }
            if method == "POST" && p == "/user/repos" {
                let name = format!("{LOGIN}/{}", str_of(body, "name"));
                if repos.contains_key(&name) {
                    return (422, json!({ "message": "name already exists on this account" }));
                }
                let mut repo = Repo {
                    name: name.clone(),
                    private: body.get("private") != Some(&json!(false)),
                    branch: "main".into(),
                    blobs: HashMap::new(),
                    trees: HashMap::new(),
                    commits: HashMap::new(),
                    refs: BTreeMap::new(),
                    pulls: Vec::new(),
                };
                if truthy(&body["auto_init"]) {
                    let sha = add_blob(&mut repo, format!("# {}\n", str_of(body, "name")).as_bytes());
                    let files = BTreeMap::from([("README.md".to_string(), sha)]);
                    let c = commit_files(&mut repo, counter, &files, "Initial commit", &[]);
                    repo.refs.insert("main".into(), c);
                }
                let private = repo.private;
                repos.insert(name.clone(), repo);
                return (201, json!({ "full_name": name, "default_branch": "main", "private": private }));
            }
            let re = Regex::new(r"^/repos/([^/]+/[^/]+)(/.*)?$").unwrap();
            if let Some(m) = re.captures(p) {
                let Some(repo) = repos.get_mut(&dec(&m[1])) else { return (404, json!({ "message": "Not Found" })) };
                let rest = m.get(2).map(|r| r.as_str()).unwrap_or("");
                let not_found = (404, json!({ "message": "Not Found" }));
                if method == "GET" && rest.is_empty() {
                    return (200, json!({ "full_name": repo.name, "default_branch": repo.branch, "private": repo.private }));
                }
                if method == "GET" {
                    if let Some(name) = rest.strip_prefix("/git/ref/heads/") {
                        return match repo.refs.get(&dec(name)) {
                            Some(sha) => (200, json!({ "object": { "sha": sha } })),
                            None if !repo.refs.is_empty() => not_found,
                            None => (409, json!({ "message": "Git Repository is empty." })),
                        };
                    }
                    if rest == "/branches" {
                        let list: Vec<Value> = repo.refs.iter().map(|(n, s)| json!({ "name": n, "commit": { "sha": s }, "protected": false })).collect();
                        return (200, Value::Array(list));
                    }
                    if rest == "/pulls" {
                        let state = query.split('&').find_map(|kv| kv.strip_prefix("state=")).unwrap_or("open");
                        let list: Vec<Value> = repo.pulls.iter().filter(|pr| state == "all" || pr["state"] == state).cloned().collect();
                        return (200, Value::Array(list));
                    }
                    if let Some(sha) = rest.strip_prefix("/git/trees/") {
                        let tree_sha = repo.commits.get(sha).map(|c| c.0.clone()).unwrap_or_else(|| sha.to_string());
                        return match repo.trees.get(&tree_sha) {
                            Some(entries) => (200, json!({ "sha": tree_sha, "tree": entries, "truncated": false })),
                            None => not_found,
                        };
                    }
                    if let Some(sha) = rest.strip_prefix("/git/blobs/") {
                        return match repo.blobs.get(sha) {
                            Some(b) => (200, json!({ "content": base64_encode(b), "encoding": "base64" })),
                            None => not_found,
                        };
                    }
                }
                if method == "POST" {
                    if rest == "/git/refs" {
                        let Some(name) = str_of(body, "ref").strip_prefix("refs/heads/").map(str::to_string) else {
                            return (422, json!({ "message": "Reference name is invalid" }));
                        };
                        if repo.refs.contains_key(&name) {
                            return (422, json!({ "message": "Reference already exists" }));
                        }
                        let sha = str_of(body, "sha");
                        if !repo.commits.contains_key(&sha) {
                            return (422, json!({ "message": "Object does not exist" }));
                        }
                        repo.refs.insert(name, sha.clone());
                        return (201, json!({ "ref": body["ref"], "object": { "sha": sha } }));
                    }
                    if rest == "/pulls" {
                        let invalid = |message: String| {
                            (422, json!({ "message": "Validation Failed", "errors": [{ "resource": "PullRequest", "code": "custom", "message": message }] }))
                        };
                        let (head, base) = (str_of(body, "head"), str_of(body, "base"));
                        if !repo.refs.contains_key(&head) || !repo.refs.contains_key(&base) {
                            return invalid(format!("{} invalid", if repo.refs.contains_key(&head) { "base" } else { "head" }));
                        }
                        if repo.refs[&head] == repo.refs[&base] {
                            return invalid(format!("No commits between {base} and {head}"));
                        }
                        if repo.pulls.iter().any(|pr| pr["state"] == "open" && pr["head"]["ref"] == head && pr["base"]["ref"] == base) {
                            return invalid(format!("A pull request already exists for {LOGIN}:{head}."));
                        }
                        let number = repo.pulls.len() + 1;
                        let pr = json!({ "number": number, "state": "open", "title": body["title"], "body": body["body"], "draft": false,
                            "html_url": format!("https://github.com/{}/pull/{number}", repo.name), "head": { "ref": head },
                            "base": { "ref": base }, "user": { "login": LOGIN } });
                        repo.pulls.push(pr.clone());
                        return (201, pr);
                    }
                    if rest == "/git/blobs" {
                        let data = base64_decode(&str_of(body, "content")).unwrap_or_default();
                        return (201, json!({ "sha": add_blob(repo, &data) }));
                    }
                    if rest == "/git/trees" {
                        let entries = body["tree"].as_array().cloned().unwrap_or_default();
                        if let Some(missing) = entries.iter().find(|e| !repo.blobs.contains_key(&str_of(e, "sha"))) {
                            return (422, json!({ "message": format!("tree.sha {} is not a valid blob", str_of(missing, "sha")) }));
                        }
                        let sha = sha1_hex(serde_json::to_string(&entries).unwrap().as_bytes());
                        repo.trees.insert(sha.clone(), entries);
                        return (201, json!({ "sha": sha }));
                    }
                    if rest == "/git/commits" {
                        *counter += 1;
                        let parents: Vec<String> = body["parents"].as_array().map(|a| a.iter().map(js_string).collect()).unwrap_or_default();
                        let (tree, message) = (str_of(body, "tree"), str_of(body, "message"));
                        let sha = sha1_hex(format!("{tree}{message}{}{counter}", parents.join(",")).as_bytes());
                        repo.commits.insert(sha.clone(), (tree, parents, message));
                        return (201, json!({ "sha": sha }));
                    }
                }
                if method == "PATCH" {
                    if let Some(name) = rest.strip_prefix("/git/refs/heads/") {
                        let sha = str_of(body, "sha");
                        if !repo.commits.contains_key(&sha) {
                            return (422, json!({ "message": "Object does not exist" }));
                        }
                        repo.refs.insert(dec(name), sha.clone());
                        return (200, json!({ "object": { "sha": sha } }));
                    }
                }
                if method == "PUT" {
                    if let Some(file) = rest.strip_prefix("/contents/") {
                        let branch = body.get("branch").filter(|b| truthy(b)).map(js_string).unwrap_or_else(|| repo.branch.clone());
                        let head = repo.refs.get(&branch).cloned();
                        let mut files = head.as_deref().map(|h| tree_files(repo, h)).unwrap_or_default();
                        let data = base64_decode(&str_of(body, "content")).unwrap_or_default();
                        files.insert(dec(file), add_blob(repo, &data));
                        let c = commit_files(repo, counter, &files, &str_of(body, "message"), &head.into_iter().collect::<Vec<_>>());
                        repo.refs.insert(branch, c.clone());
                        return (201, json!({ "commit": { "sha": c } }));
                    }
                }
            }
            (404, json!({ "message": "Not Found" }))
        }

        // Aktueller Inhalt einer Datei auf GitHub (Standard-Branch oder branch)
        fn file_on_github(&self, name: &str, file: &str, branch: Option<&str>) -> Option<String> {
            let st = self.state.lock();
            let repo = st.repos.get(name)?;
            let head = repo.refs.get(branch.unwrap_or(&repo.branch))?;
            let sha = tree_files(repo, head).get(file)?.clone();
            Some(String::from_utf8_lossy(&repo.blobs[&sha]).to_string())
        }

        // Eine Änderung direkt auf GitHub (wie aus einer anderen Sitzung)
        fn commit_on_github(&self, name: &str, file: &str, text: &str) {
            let mut guard = self.state.lock();
            let MockState { repos, counter } = &mut *guard;
            let repo = repos.get_mut(name).unwrap();
            let b = repo.branch.clone();
            let head = repo.refs[&b].clone();
            let mut files = tree_files(repo, &head);
            files.insert(file.to_string(), add_blob(repo, text.as_bytes()));
            let c = commit_files(repo, counter, &files, &format!("Änderung an {file}"), &[head]);
            repo.refs.insert(b, c);
        }

        fn create_repo_without_init(&self, name: &str) {
            let auth = format!("Bearer {TOKEN}");
            self.handle("POST", "/user/repos", &auth, &json!({ "name": name, "private": true, "auto_init": false }));
        }
    }

    pub(crate) async fn setup() -> (Arc<Mock>, GitHub, JoinHandle<()>) {
        let mock = Arc::new(Mock::default());
        let m = mock.clone();
        let handler: Handler = Arc::new(move |req: Req| {
            let body: Value = serde_json::from_slice(&req.body).unwrap_or(Value::Null);
            let auth = req.headers.get("authorization").cloned().unwrap_or_default();
            let (status, v) = m.handle(&req.method, &req.path, &auth, &body);
            Reply::Json(status, v.to_string())
        });
        let (base, task) = serve(handler).await;
        let gh = GitHub::new(Arc::new(MemoryTokenStore::default()))
            .with_api(&base)
            .with_web("https://github.example")
            .with_client(reqwest::Client::builder().no_proxy().build().unwrap());
        (mock, gh, task)
    }

    pub(crate) fn folder(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (p, text) in files {
            let full = dir.path().join(p);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, text).unwrap();
        }
        dir
    }

    pub(crate) fn read(dir: &Path, p: &str) -> String {
        std::fs::read_to_string(dir.join(p)).unwrap()
    }

    pub(crate) fn write(dir: &Path, p: &str, text: &str) {
        std::fs::write(dir.join(p), text).unwrap();
    }

    fn err_of<T>(r: Result<T>) -> String {
        match r {
            Ok(_) => panic!("Fehler erwartet"),
            Err(e) => e.to_string(),
        }
    }

    fn strs(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(js_string).collect()
    }

    fn names(files: Vec<LocalFile>) -> Vec<String> {
        let mut n: Vec<String> = files.into_iter().map(|f| f.path).collect();
        n.sort();
        n
    }

    fn list(root: &Path) -> Vec<String> {
        names(list_local(root, &HashSet::new()).unwrap())
    }

    #[test]
    fn pruefsumme_wie_git_und_repository_angaben() {
        assert_eq!(blob_sha(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"); // wie git hash-object
        assert_eq!(parse_repo("https://github.com/lea/projekt.git").unwrap(), "lea/projekt");
        assert_eq!(parse_repo("lea/projekt").unwrap(), "lea/projekt");
        assert!(err_of(parse_repo("projekt")).contains("besitzer/name"));
        assert_eq!(parse_repo("https://github.com/lea/website/tree/main").unwrap(), "lea/website");
        assert_eq!(parse_repo("https://github.com/lea/website/blob/main/README.md").unwrap(), "lea/website");
        assert_eq!(parse_repo("git@github.com:lea/website.git").unwrap(), "lea/website");
        assert!(err_of(parse_repo("../user")).contains("besitzer/name"));
    }

    #[tokio::test]
    async fn verbinden_falsches_token_abgelehnt_richtiges_gespeichert() {
        let (_mock, gh, _s) = setup().await;
        assert!(err_of(gh.connect("falsch").await).contains("lehnt das Token ab"));
        assert!(!gh.connected());
        assert_eq!(gh.connect(TOKEN).await.unwrap(), "lea-test");
        assert!(gh.connected());
    }

    #[tokio::test]
    async fn anlegen_hochladen_in_anderem_ordner_holen_hin_und_her_aendern() {
        let (mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let a = folder(&[("index.html", "<h1>Spiel</h1>"), ("js/spiel.js", "let x = 1;"), (".ask/chats/c.json", "{}"), ("README.md", "# Mein Spiel\n")]);
        let a = a.path();
        let created = gh.create(a, "spiel", true, "").await.unwrap();
        assert_eq!(created["repo"], "lea-test/spiel");
        assert_eq!(created["private"], true);
        assert_eq!(mock.file_on_github("lea-test/spiel", "js/spiel.js", None).as_deref(), Some("let x = 1;"));
        assert_eq!(mock.file_on_github("lea-test/spiel", "README.md", None).as_deref(), Some("# Mein Spiel\n"), "eigene README gewinnt gegen die automatische");
        assert_eq!(mock.file_on_github("lea-test/spiel", ".ask/chats/c.json", None), None, ".ask wird nicht hochgeladen");

        // Andere Sitzung: klonen, ändern, hochladen
        let b = folder(&[]);
        let b = b.path();
        let cloned = gh.clone_repo(b, "https://github.com/lea-test/spiel").await.unwrap();
        assert!(strs(&cloned["written"]).contains(&"index.html".to_string()));
        assert_eq!(read(b, "js/spiel.js"), "let x = 1;");
        write(b, "js/spiel.js", "let x = 2;");
        write(b, "neu.txt", "neu");
        let pushed = gh.push(b, "x auf 2", false).await.unwrap();
        assert_eq!(pushed["uploaded"], 2);

        // Erste Sitzung: Hochladen ohne Holen wird abgelehnt, Holen übernimmt die Änderungen
        write(a, "index.html", "<h1>Spiel 2</h1>");
        assert!(err_of(gh.push(a, "veraltet", false).await).contains("Zuerst github_pull"));
        let pulled = gh.pull(a).await.unwrap();
        assert_eq!(pulled["conflicts"], json!([]));
        assert_eq!(read(a, "js/spiel.js"), "let x = 2;");
        assert_eq!(read(a, "neu.txt"), "neu");
        assert_eq!(read(a, "index.html"), "<h1>Spiel 2</h1>", "eigene Änderung bleibt");
        gh.push(a, "Titel", false).await.unwrap();
        assert_eq!(mock.file_on_github("lea-test/spiel", "index.html", None).as_deref(), Some("<h1>Spiel 2</h1>"));

        // Löschen wird übertragen
        std::fs::remove_file(a.join("neu.txt")).unwrap();
        let del = gh.push(a, "neu.txt weg", false).await.unwrap();
        assert_eq!(del["deleted"], json!(["neu.txt"]));
        gh.pull(b).await.unwrap();
        assert!(!b.join("neu.txt").exists());

        // Konflikt: dieselbe Datei hier und auf GitHub geändert, nichts überschrieben
        mock.commit_on_github("lea-test/spiel", "index.html", "<h1>von GitHub</h1>");
        write(b, "index.html", "<h1>von hier</h1>");
        let conflict = gh.pull(b).await.unwrap();
        assert_eq!(conflict["conflicts"], json!(["index.html"]));
        assert_eq!(read(b, "index.html"), "<h1>von hier</h1>");

        // Ordner a holt die Änderung von GitHub (dort unverändert), danach gibt es nichts hochzuladen
        assert_eq!(gh.pull(a).await.unwrap()["written"], json!(["index.html"]));
        assert_eq!(read(a, "index.html"), "<h1>von GitHub</h1>");
        let again = gh.push(a, "nichts", false).await.unwrap();
        assert!(again["nothing"] == true || again["uploaded"] == 0);
        let status = gh.status(a).await.unwrap();
        assert_eq!(status["repo"], "lea-test/spiel");
    }

    #[tokio::test]
    async fn hochladen_in_ein_leeres_repository() {
        let (mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        mock.create_repo_without_init("leer");
        let a = folder(&[("a.txt", "A"), ("b/c.txt", "C")]);
        gh.clone_repo(a.path(), "lea-test/leer").await.unwrap();
        gh.push(a.path(), "Start", false).await.unwrap();
        assert_eq!(mock.file_on_github("lea-test/leer", "b/c.txt", None).as_deref(), Some("C"));
        assert_eq!(mock.file_on_github("lea-test/leer", "a.txt", None).as_deref(), Some("A"));
    }

    #[tokio::test]
    async fn gitignore_ausgeschlossenes_wird_nicht_hochgeladen_schon_abgeglichenes_bleibt() {
        let (mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let a = folder(&[
            ("app.py", "print(1)"),
            (".env", "GEHEIM=1"),
            (".gitignore", ".env\n*.log\n/build/\n!wichtig.log\ndaten/**/roh\n"),
            ("lauf.log", "x"),
            ("wichtig.log", "y"),
            ("build/app.exe", "bin"),
            ("src/build/ok.txt", "ok"),
            ("daten/2026/roh/a.csv", "1"),
            ("daten/2026/fertig.csv", "2"),
        ]);
        let a = a.path();
        assert_eq!(list(a), [".gitignore", "app.py", "daten/2026/fertig.csv", "src/build/ok.txt", "wichtig.log"]);
        gh.create(a, "geheim", true, "").await.unwrap();
        assert_eq!(mock.file_on_github("lea-test/geheim", ".env", None), None);
        assert_eq!(mock.file_on_github("lea-test/geheim", "build/app.exe", None), None);
        assert_eq!(mock.file_on_github("lea-test/geheim", "app.py", None).as_deref(), Some("print(1)"));
        // Auf GitHub liegt eine Datei, die hier ausgeschlossen und anders ist: Holen überschreibt sie nicht
        mock.commit_on_github("lea-test/geheim", ".env", "GEHEIM=von-github");
        let pulled = gh.pull(a).await.unwrap();
        assert_eq!(pulled["conflicts"], json!([".env"]));
        assert_eq!(read(a, ".env"), "GEHEIM=1");
        // Eine abgeglichene Datei kommt später in .gitignore: Hochladen löscht sie auf GitHub nicht
        write(a, ".gitignore", ".env\n*.log\napp.py\n");
        let pushed = gh.push(a, "gitignore", false).await.unwrap();
        assert_eq!(pushed["deleted"], json!([]));
        assert_eq!(mock.file_on_github("lea-test/geheim", "app.py", None).as_deref(), Some("print(1)"));
    }

    #[tokio::test]
    async fn konflikt_hochladen_ersetzt_die_fassung_auf_github_nur_mit_overwrite() {
        let (mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let a = folder(&[("index.html", "eins")]);
        let a = a.path();
        gh.create(a, "konflikt", true, "").await.unwrap();
        mock.commit_on_github("lea-test/konflikt", "index.html", "von GitHub");
        write(a, "index.html", "von hier");
        assert_eq!(gh.pull(a).await.unwrap()["conflicts"], json!(["index.html"]));
        assert_eq!(gh.status(a).await.unwrap()["conflicts"], json!(["index.html"]));
        let err = gh.push(a, "x", false).await.unwrap_err();
        assert!(err.to_string().contains("verschieden geändert: index.html"));
        assert_eq!(err.downcast_ref::<PushConflict>().unwrap().conflicts, ["index.html"]);
        assert_eq!(mock.file_on_github("lea-test/konflikt", "index.html", None).as_deref(), Some("von GitHub"));
        gh.push(a, "x", true).await.unwrap();
        assert_eq!(mock.file_on_github("lea-test/konflikt", "index.html", None).as_deref(), Some("von hier"));
        assert_eq!(gh.status(a).await.unwrap()["conflicts"], json!([]));
    }

    #[tokio::test]
    async fn gespeicherte_verbindung_wird_geprueft_ablage_ausserhalb_des_ordners() {
        let sha = "a".repeat(40);
        let ok = json!({ "repo": "lea/web", "branch": "main", "commit": null, "files": { "a/b.txt": sha } });
        assert!(valid_link(&ok));
        let with = |k: &str, v: Value| {
            let mut o = ok.clone();
            o[k] = v;
            o
        };
        let bad = [
            with("repo", json!("lea/web/../../user")),
            with("repo", json!("../x")),
            with("repo", json!("lea/web?x=1")),
            with("branch", json!("a/../b")),
            with("commit", json!("xyz")),
            with("files", json!({ "../x": sha })),
            with("files", json!({ "C:/x": sha })),
            with("files", json!({ "a.txt": "kurz" })),
            Value::Null,
        ];
        for b in &bad {
            assert!(!valid_link(b), "{b}");
        }
        let store = tempfile::tempdir().unwrap();
        let a = folder(&[("a.txt", "A")]);
        let a = a.path();
        let (_mock, gh, _s) = setup().await;
        let file = store.path().join("github.json");
        let f2 = file.clone();
        let gh = gh.with_link_file(move |_| f2.clone());
        gh.connect(TOKEN).await.unwrap();
        gh.create(a, "ablage", true, "").await.unwrap();
        assert!(file.exists());
        assert!(!a.join(".ask").join("github.json").exists());
        std::fs::write(&file, json!({ "repo": "lea-test/ablage/../../user", "branch": "main", "commit": null, "files": {} }).to_string()).unwrap();
        assert_eq!(gh.status(a).await.unwrap()["linked"], false);
    }

    #[test]
    fn branch_namen() {
        for ok in ["main", "neue-startseite", "feature/login", "v1.2", "lea_test"] {
            assert!(valid_branch(ok), "{ok}");
        }
        let long = "a".repeat(201);
        for bad in ["", "-x", ".x", "a..b", "a//b", "a/", "a.", "x.lock", "a/.b", "HEAD", "mit leerzeichen", "ä", "../main", long.as_str()] {
            assert!(!valid_branch(bad), "{bad}");
        }
    }

    #[tokio::test]
    async fn branches_auflisten_anlegen_wechseln_nur_ohne_aenderungen_hier() {
        let (mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let a = folder(&[("index.html", "eins"), ("alt.txt", "alt")]);
        let a = a.path();
        gh.create(a, "zweige", true, "").await.unwrap();
        let b = gh.branches(a).await.unwrap();
        assert_eq!(b, json!({ "current": "main", "default": "main", "branches": ["main"], "url": "https://github.example/lea-test/zweige" }));

        // Neuer Branch mit einer Änderung hier: Die Änderung bleibt und geht auf den neuen Branch
        write(a, "index.html", "zwei");
        let created = gh.create_branch(a, "neue-startseite").await.unwrap();
        assert_eq!(created["from"], "main");
        assert_eq!(read(a, "index.html"), "zwei");
        assert!(err_of(gh.create_branch(a, "neue-startseite").await).contains("schon mit dem Branch"));
        assert!(err_of(gh.create_branch(a, "a..b").await).contains("geht nicht als Branch-Name"));
        let b = gh.branches(a).await.unwrap();
        assert_eq!(b["branches"], json!(["main", "neue-startseite"]));
        assert_eq!(b["current"], "neue-startseite");
        assert_eq!(gh.status(a).await.unwrap()["changed"], json!(["index.html"]));

        // Wechseln mit Änderungen hier: abgelehnt, mit Grund
        let err = gh.switch_branch(a, "main").await.unwrap_err();
        assert!(err.to_string().contains("noch nicht hochgeladen") && err.to_string().contains("index.html"));
        assert_eq!(err.downcast_ref::<PendingChanges>().unwrap().pending.len(), 1);
        std::fs::remove_file(a.join("alt.txt")).unwrap();
        write(a, "neu.txt", "neu");
        gh.push(a, "Neue Startseite", false).await.unwrap();
        assert_eq!(mock.file_on_github("lea-test/zweige", "index.html", Some("neue-startseite")).as_deref(), Some("zwei"));
        assert_eq!(mock.file_on_github("lea-test/zweige", "index.html", None).as_deref(), Some("eins"), "main bleibt, wie es war");

        // Zurück auf main: Dateien bekommen dessen Stand, die nur auf dem Branch vorhandenen verschwinden
        let back = gh.switch_branch(a, "main").await.unwrap();
        assert_eq!(back["from"], "neue-startseite");
        assert_eq!(read(a, "index.html"), "eins");
        assert_eq!(read(a, "alt.txt"), "alt");
        assert!(!a.join("neu.txt").exists());
        let s = gh.status(a).await.unwrap();
        assert_eq!(s["branch"], "main");
        assert_eq!(s["changed"], json!([]));
        assert_eq!(s["added"], json!([]));
        assert_eq!(s["deleted"], json!([]));
        assert_eq!(s["remoteNewer"], false);
        assert_eq!(gh.switch_branch(a, "main").await.unwrap()["same"], true);
        assert!(err_of(gh.switch_branch(a, "gibt-es-nicht").await).contains("gibt es auf GitHub nicht"));
        assert_eq!(read(a, "index.html"), "eins", "ein fehlender Branch löscht nichts");

        // Und wieder auf den Branch
        gh.switch_branch(a, "neue-startseite").await.unwrap();
        assert_eq!(read(a, "neu.txt"), "neu");
        assert!(!a.join("alt.txt").exists());
    }

    #[tokio::test]
    async fn pull_requests_erstellen_und_auflisten() {
        let (_mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let a = folder(&[("index.html", "eins")]);
        let a = a.path();
        gh.create(a, "pr", true, "").await.unwrap();
        assert!(err_of(gh.create_pull_request(a, "x", "", "").await).contains("Zuerst einen eigenen Branch"));
        gh.create_branch(a, "titel").await.unwrap();
        assert!(err_of(gh.create_pull_request(a, "Ohne Änderung", "", "").await).contains("No commits between main and titel"));
        write(a, "index.html", "zwei");
        gh.push(a, "Titel geändert", false).await.unwrap();
        assert!(err_of(gh.create_pull_request(a, " ", "", "").await).contains("braucht einen Titel"));
        let pr = gh.create_pull_request(a, "Neuer Titel", "Die Startseite hat einen neuen Titel.", "").await.unwrap();
        assert_eq!(pr, json!({ "number": 1, "url": "https://github.com/lea-test/pr/pull/1", "head": "titel", "base": "main" }));
        assert!(err_of(gh.create_pull_request(a, "Nochmals", "", "").await).contains("already exists"));
        let prs = gh.pull_requests(a).await.unwrap();
        assert_eq!(
            prs,
            json!([{ "number": 1, "title": "Neuer Titel", "url": "https://github.com/lea-test/pr/pull/1", "head": "titel", "base": "main", "user": "lea-test", "draft": false }])
        );
    }

    #[tokio::test]
    async fn branch_auf_github_weg_kein_leerer_stand_lokale_dateien_bleiben() {
        let (_mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let a = folder(&[("index.html", "eins"), ("b/c.txt", "C")]);
        let a = a.path();
        gh.create(a, "weg", true, "").await.unwrap();
        let link = gh.read_link(a).unwrap();
        // Der Branch existiert auf GitHub nicht (mehr), das Repository aber schon
        gh.write_link(a, &Link { branch: "feature-alt".into(), ..link.clone() }).unwrap();
        assert!(err_of(gh.pull(a).await).contains("Den Branch feature-alt gibt es auf GitHub nicht mehr. Wähle im Reiter GitHub einen anderen Branch."));
        assert!(err_of(gh.status(a).await).contains("nicht mehr"));
        assert!(err_of(gh.push(a, "x", false).await).contains("nicht mehr"));
        assert_eq!(read(a, "index.html"), "eins");
        assert_eq!(read(a, "b/c.txt"), "C");
        // Repository umbenannt oder Zugriff verloren
        gh.write_link(a, &Link { repo: "lea-test/umbenannt".into(), ..link }).unwrap();
        assert!(err_of(gh.pull(a).await).contains("Repository lea-test/umbenannt gibt es auf GitHub nicht mehr"));
        assert_eq!(read(a, "index.html"), "eins");
    }

    #[tokio::test]
    async fn ein_leeres_repository_holen_loescht_hier_nichts() {
        let (mock, gh, _s) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        mock.create_repo_without_init("leer2");
        let a = folder(&[("a.txt", "A")]);
        let a = a.path();
        gh.clone_repo(a, "lea-test/leer2").await.unwrap();
        let link = gh.read_link(a).unwrap();
        gh.write_link(a, &Link { files: BTreeMap::from([("a.txt".to_string(), blob_sha(b"A"))]), ..link }).unwrap();
        let r = gh.pull(a).await.unwrap();
        assert_eq!(r["removed"], json!([]));
        assert_eq!(read(a, "a.txt"), "A");
    }

    #[tokio::test]
    async fn anfrage_ohne_antwort_bricht_nach_der_zeitgrenze_ab_und_gibt_den_abgleich_frei() {
        let (base, _task) = serve(Arc::new(|_req: Req| Reply::Hang)).await;
        let gh = GitHub::new(Arc::new(MemoryTokenStore::new(TOKEN)))
            .with_api(&base)
            .with_timeout(30)
            .with_client(reqwest::Client::builder().no_proxy().build().unwrap());
        let a = folder(&[("a.txt", "A")]);
        assert!(err_of(gh.request("GET", "/repos/x/y", None).await).contains("GitHub antwortet nicht"));
        let r = gh.exclusive(a.path(), || gh.request("GET", "/repos/x/y", None)).await;
        assert!(err_of(r).contains("GitHub antwortet nicht"));
        assert!(!gh.locked(a.path()));
    }

    #[tokio::test]
    async fn exclusive_pro_ordner_nur_einer() {
        let gh = GitHub::new(Arc::new(MemoryTokenStore::new(TOKEN)));
        let a = folder(&[]);
        let r = gh
            .exclusive(a.path(), || async {
                assert!(gh.locked(a.path()));
                assert_eq!(err_of(gh.exclusive(a.path(), || async { Ok(()) }).await), "Ein Abgleich mit GitHub läuft schon.");
                Ok(5)
            })
            .await
            .unwrap();
        assert_eq!(r, 5);
        assert!(!gh.locked(a.path()));
    }

    #[test]
    fn ausschlussliste_muster_mit_vielen_sternen_brauchen_keine_rechenzeit_regeln_bleiben_richtig() {
        let evil = [format!("{}*b", "*a".repeat(40)), format!("{}x", "**/".repeat(40)), format!("{}.log", "*".repeat(60)), format!("{}[!x]?", "a*".repeat(30))].join("\n");
        let long = format!("{}.txt", "a".repeat(80));
        let ignore = format!("{evil}\n*.tmp\nbuild/\n/wurzel.txt\ndocs/**/geheim.md\n");
        let a = folder(&[
            (".gitignore", ignore.as_str()),
            (long.as_str(), "1"),
            ("x.tmp", "2"),
            ("build/o.js", "3"),
            ("wurzel.txt", "4"),
            ("sub/wurzel.txt", "5"),
            ("docs/a/b/geheim.md", "6"),
            ("docs/geheim.md", "7"),
            ("docs/ok.md", "8"),
        ]);
        let start = std::time::Instant::now();
        let found = list(a.path());
        assert!(start.elapsed().as_millis() < 1000, "zu langsam: {:?}", start.elapsed());
        let mut want = vec![".gitignore".to_string(), long, "docs/ok.md".to_string(), "sub/wurzel.txt".to_string()];
        want.sort();
        assert_eq!(found, want);
    }

    #[test]
    fn ballast_ordner_werden_in_jeder_tiefe_uebergangen() {
        let a = folder(&[
            ("app.js", "1"),
            ("app/node_modules/x/index.js", "2"),
            ("a/b/.git/config", "3"),
            ("py/.venv/lib.py", "4"),
            ("src/__pycache__/m.pyc", "5"),
            ("src/node_modules_neu/k.js", "6"),
            ("node_modules/y.js", "7"),
        ]);
        assert_eq!(list(a.path()), ["app.js", "src/node_modules_neu/k.js"]);
    }

    #[test]
    fn regeln_klassen_und_ungueltige_bereiche() {
        let rule = ignore_rule("*.[ch]", "").unwrap();
        assert!(rule.matches("a/x.c") && rule.matches("x.h") && !rule.matches("x.o"));
        let rule = ignore_rule("[!a-c]x", "").unwrap();
        assert!(rule.matches("dx") && !rule.matches("bx"));
        assert!(ignore_rule("[z-a]x", "").is_none());
        assert!(ignore_rule("# Kommentar", "").is_none());
        assert!(ignore_rule("   ", "").is_none());
    }
}
