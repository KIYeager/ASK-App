//! Werkzeuge für das Modell (ENTSCHEIDE E19). Ausgeführt werden sie über das lokale Terminal
//! (open-terminal mit Low Integrity und Ordnersperre) mit dem echten Key, den nur der Hauptprozess
//! kennt. Jedes Werkzeug liefert den Text für das Modell und eine kurze Anzeige für die Oberfläche.
//!
//! Portiert aus src/main/tools.js. Öffentliche Schnittstelle:
//! - `tools() -> &[Tool]` (JS `TOOLS`: Name, Titel, Beschreibung, Parameter), `tool_specs(command_hint)`
//!   (OpenAI-Format), `describe(name, args) -> Described`, `parse_args(raw)`, `clean_args(raw)`, `clip(text, max)`
//! - `run_tool(terminal, name, raw_args, &ToolCtx) -> ToolResult`: Fehler gehen als Text an das Modell zurück
//! - `request(terminal, method, path, body, timeout_ms) -> Response`: HTTP an das lokale Terminal
//!   (`TerminalEndpoint` ist ein `Terminal` oder, für Tests, ein eigener Ersatz), `servers_request_fn(terminal)`
//!   macht daraus die `RequestFn` für `servers::Servers`
//! - `blocked_command(command)`: Programme mit Fenster werden abgelehnt
//! - `ToolCtx` (JS: das `ctx`-Objekt von `runTool`), `ToolResult`, `GithubTrust`, `VisualTools` und `VisualOpts`
//!
//! `ctx.visual` (src/main/visual.js, in Rust `crate::visual`) ist hier das Trait [`VisualTools`] mit genau den
//! beiden Methoden, die tools.js aufruft: `view` (Werkzeug view_file) und `browse` (Werkzeug browser). Beide
//! liefern `{ ok, output, summary, display?, images? }` als JSON (`ToolResult::from_value` liest es).
//!
//! Unterschiede zu JS:
//! * `terminal` ist `Option<&dyn TerminalEndpoint>` (Tests übergeben in JS `null`); fehlt es, meldet jedes
//!   Werkzeug, das das Terminal braucht, «Das Terminal läuft nicht …».
//! * `request` setzt eine Zeitgrenze für die ganze Anfrage (in Node nur für Stille auf der Leitung).
//! * Rückfragen ohne `ctx.confirm` gelten als abgelehnt (in JS wäre das ein TypeError).

use crate::askapi::{Signal, Source};
use crate::bild::{Images, FORMATS as FLUX_FORMATS, QUALITIES as FLUX_QUALITIES};
use crate::github::{encode_uri_component, GitHub, PendingChanges};
use crate::history;
use crate::host::{ConfirmFn, ConfirmOptions, Confirmed};
use crate::memorytools::{self, KnowledgeCtx, MemoryToolCtx};
use crate::servers::{RequestFn, Servers, TerminalResponse};
use crate::terminal::Terminal;
use crate::util::{js_string, num_of, str_of, take_chars, truthy};
use crate::web::{PageOptions, Web, PAGE_CHARS};
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

macro_rules! re {
    ($p:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($p).expect("regex"))
    }};
}

/// Zeichen pro Ergebnis an das Modell
pub const MAX_OUTPUT: usize = 12000;
const POLL_SECONDS: u64 = 2;
/// Standard-Zeitgrenze einer Anfrage an das Terminal
pub const REQUEST_TIMEOUT_MS: u64 = 330_000;

// ---------- Terminal ----------

/// Was `request` vom Terminal wissen muss (JS: `terminal.state`, `terminal.port`, `terminal.key`)
pub trait TerminalEndpoint: Send + Sync {
    fn state(&self) -> String;
    fn port(&self) -> u16;
    fn key(&self) -> String;
}

impl TerminalEndpoint for Terminal {
    fn state(&self) -> String {
        Terminal::state(self)
    }
    fn port(&self) -> u16 {
        Terminal::port(self)
    }
    fn key(&self) -> String {
        Terminal::key(self)
    }
}

/// Feste Angaben eines Terminals (Zustand, Port, Key), z. B. für Hintergrundaufgaben
#[derive(Clone, Debug)]
pub struct EndpointSnapshot {
    pub state: String,
    pub port: u16,
    pub key: String,
}

impl EndpointSnapshot {
    pub fn of(t: &dyn TerminalEndpoint) -> Self {
        EndpointSnapshot { state: t.state(), port: t.port(), key: t.key() }
    }
}

impl TerminalEndpoint for EndpointSnapshot {
    fn state(&self) -> String {
        self.state.clone()
    }
    fn port(&self) -> u16 {
        self.port
    }
    fn key(&self) -> String {
        self.key.clone()
    }
}

/// Antwort des Terminals: Status, JSON (falls lesbar), Text und Inhaltstyp
#[derive(Clone, Debug, Default)]
pub struct Response {
    pub status: u16,
    pub json: Option<Value>,
    pub text: String,
    pub kind: String,
}

fn local_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    // Das Terminal läuft auf diesem Rechner: nie über einen Proxy
    CLIENT.get_or_init(|| reqwest::Client::builder().no_proxy().build().expect("HTTP-Client"))
}

/// Anfrage an das lokale Terminal mit dem Key im Kopf
pub async fn request(terminal: &dyn TerminalEndpoint, method: &str, path: &str, body: Option<&Value>, timeout_ms: u64) -> Result<Response> {
    if terminal.state() != "läuft" || terminal.port() == 0 {
        bail!("Das Terminal läuft nicht. Bitte in der Statusleiste neu starten.");
    }
    let method = reqwest::Method::from_bytes(method.as_bytes())?;
    let mut req = local_client()
        .request(method, format!("http://127.0.0.1:{}{path}", terminal.port()))
        .timeout(Duration::from_millis(timeout_ms))
        .header("Authorization", format!("Bearer {}", terminal.key()));
    if let Some(b) = body {
        req = req.header("Content-Type", "application/json").body(b.to_string());
    }
    let res = req.send().await.map_err(|e| if e.is_timeout() { anyhow!("Zeitüberschreitung beim Terminal") } else { anyhow!("{e}") })?;
    let status = res.status().as_u16();
    let kind = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let bytes = res.bytes().await.map_err(|e| if e.is_timeout() { anyhow!("Zeitüberschreitung beim Terminal") } else { anyhow!("{e}") })?;
    let text = String::from_utf8_lossy(&bytes).to_string();
    let json = serde_json::from_str::<Value>(&text).ok(); // kein JSON: leer
    Ok(Response { status, json, text, kind })
}

/// `request` als `RequestFn` für `servers::Servers` (die Terminal-API des Projekts)
pub fn servers_request_fn(terminal: Arc<dyn TerminalEndpoint>) -> RequestFn {
    Arc::new(move |method, path, body| {
        let terminal = terminal.clone();
        Box::pin(async move {
            let r = request(&*terminal, &method, &path, body.as_ref(), REQUEST_TIMEOUT_MS).await?;
            Ok(TerminalResponse { status: r.status, json: r.json, text: r.text })
        })
    })
}

fn fail(res: &Response) -> anyhow::Error {
    let detail = res.json.as_ref().and_then(|j| j.get("detail")).filter(|d| truthy(d));
    let text = match detail {
        Some(Value::String(s)) => s.clone(),
        Some(d) => d.to_string(),
        None => take_chars(&res.text, 500),
    };
    anyhow!(if text.is_empty() { format!("HTTP {}", res.status) } else { text })
}

/// So viel Ausgabe eines Befehls behält die App höchstens, während sie auf ihn wartet (Bytes). Das Modell bekommt
/// davon `MAX_OUTPUT` Zeichen, die Oberfläche 6000; ohne Grenze füllte ein Skript, das endlos schreibt, den Speicher.
const LIVE_OUTPUT_MAX: usize = 300_000;

/// Gesammelte Ausgabe auf `LIVE_OUTPUT_MAX` halten: Anfang und Ende bleiben, die Mitte fällt weg.
fn trim_live(output: &mut String) {
    if output.len() <= LIVE_OUTPUT_MAX {
        return;
    }
    let mut head = LIVE_OUTPUT_MAX / 5;
    while !output.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = output.len() - LIVE_OUTPUT_MAX * 3 / 5;
    while !output.is_char_boundary(tail) {
        tail += 1;
    }
    *output = format!("{}\n… (Ausgabe gekürzt) …\n{}", &output[..head], &output[tail..]);
}

/// Kürzt in der Mitte: ein Drittel Anfang, der Rest Ende
pub fn clip(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        return text.to_string();
    }
    let head = (max as f64 * 0.3).floor() as usize;
    format!(
        "{}\n… ({} Zeichen ausgelassen) …\n{}",
        chars[..head].iter().collect::<String>(),
        chars.len() - max,
        chars[chars.len() - (max - head)..].iter().collect::<String>()
    )
}

/// Abfrage-Teil einer Adresse; Werte ohne Inhalt (null, leer) fallen weg
fn q(params: &[(&str, Value)]) -> String {
    params
        .iter()
        .filter(|(_, v)| !v.is_null() && !matches!(v, Value::String(s) if s.is_empty()))
        .map(|(k, v)| format!("{}={}", encode_uri_component(k), encode_uri_component(&js_string(v))))
        .collect::<Vec<_>>()
        .join("&")
}

fn arg(a: &Value, key: &str) -> Value {
    a.get(key).cloned().unwrap_or(Value::Null)
}

/// `${a.key}` in einer Vorlage: fehlend ergibt «undefined», null «null»
fn tpl(a: &Value, key: &str) -> String {
    match a.get(key) {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(v) => js_string(v),
    }
}

/// `a.key || ''`
fn or_empty(a: &Value, key: &str) -> String {
    match a.get(key) {
        Some(v) if truthy(v) => js_string(v),
        _ => String::new(),
    }
}

fn lines(text: &str) -> usize {
    text.replace("\r\n", "\n").split('\n').count()
}

fn zeilen(n: usize) -> String {
    format!("{n} {}", if n == 1 { "Zeile" } else { "Zeilen" })
}

fn output_text(json: &Value) -> String {
    json.get("output")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|o| js_string(o.get("data").unwrap_or(&Value::Null))).collect::<String>())
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

fn output_len(json: &Value) -> usize {
    json.get("output").and_then(Value::as_array).map(Vec::len).unwrap_or(0)
}

// ---------- Programme mit Fenster sperren ----------

/// Programme mit Fenster sperren Dateien und warten auf Eingaben, die niemand macht (Word hielt so
/// Dokumente fest). Erkannt werden sie als eigener Befehl, auch nach &, | oder «start».
const WINDOWED: [&str; 12] = ["start", "winword", "excel", "powerpnt", "outlook", "msaccess", "notepad", "mspaint", "explorer", "msedge", "chrome", "firefox"];

/// Befehlsteile, getrennt durch &, |, Klammern oder Zeilenumbruch; in Anführungszeichen trennt nichts.
fn command_parts(command: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut part = String::new();
    let mut quoted = false;
    for c in command.chars() {
        if c == '"' {
            quoted = !quoted;
        }
        if !quoted && matches!(c, '&' | '|' | '(' | ')' | '\r' | '\n') {
            parts.push(std::mem::take(&mut part));
        } else {
            part.push(c);
        }
    }
    parts.push(part);
    parts
}

/// Programm eines Befehlsteils (ohne Pfad und .exe); hinter «call», «@» und «cmd /c» zählt, was dort gestartet wird.
fn program_of(part: &str, depth: usize) -> String {
    let s = part.trim();
    let s = re!(r"^@\s*").replace(s, "");
    let s = re!(r"(?i)^call\s+").replace(&s, "").to_string();
    if depth < 3 {
        if let Some(shell) = re!(r#"(?i)^(?:"[^"]*\\)?cmd(?:\.exe)?"?\s+(?:/\S+\s+)*?/[ck]\s*(.*)$"#).captures(&s) {
            let inner = shell[1].trim();
            let inner = re!(r#"^"(.*)"$"#).replace(inner, "$1").to_string();
            return command_parts(&inner)
                .iter()
                .map(|p| program_of(p, depth + 1))
                .find(|p| WINDOWED.contains(&p.as_str()))
                .unwrap_or_default();
        }
    }
    let word = if let Some(rest) = s.strip_prefix('"') { rest.split('"').next().unwrap_or("") } else { s.split(char::is_whitespace).next().unwrap_or("") };
    let name = word.rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
    re!(r"\.(exe|com)$").replace(&name, "").to_string()
}

/// Meldung für das Modell, wenn der Befehl ein Programm mit Fenster startet; sonst leer
pub fn blocked_command(command: &str) -> String {
    if !command_parts(command).iter().any(|p| WINDOWED.contains(&program_of(p, 0).as_str())) {
        return String::new();
    }
    concat!(
        "Abgelehnt: Dieser Befehl würde ein Programm mit Fenster starten (z. B. Word, Excel, Explorer oder «start»). ",
        "Das sperrt Dateien und wartet auf Eingaben. Für eine PDF-Prüfung python .ask/skills/pruefen.py <datei> --pdf verwenden; ",
        "sonst der Person sagen, dass sie die Datei selbst öffnen soll."
    )
    .to_string()
}

// ---------- Kontext und Ergebnis ----------

/// Fortschritt eines Werkzeugs (JS `onProgress(text)`)
pub use crate::bild::ProgressFn;
/// Live-Daten für die Oberfläche (JS `onLive(data)`)
pub type LiveFn = Arc<dyn Fn(Value) + Send + Sync>;

/// Repositories, in die ohne Rückfrage hochgeladen wird (Häkchen im Dialog, zurücksetzen im GitHub-Dialog).
/// Gross- und Kleinschreibung zählen nicht. Mit `with_store` gespeichert (main.js: Einstellung `githubTrusted`).
type SaveTrust = Arc<dyn Fn(&[String]) + Send + Sync>;

#[derive(Default)]
pub struct GithubTrust {
    list: Mutex<Vec<String>>,
    save: Option<SaveTrust>,
}

impl GithubTrust {
    pub fn new() -> Self {
        Self::default()
    }
    /// Mit gespeicherter Liste; `save` bekommt nach jeder Änderung die ganze Liste
    pub fn with_store(initial: Vec<String>, save: impl Fn(&[String]) + Send + Sync + 'static) -> Self {
        Self { list: Mutex::new(initial), save: Some(Arc::new(save)) }
    }
    pub fn list(&self) -> Vec<String> {
        self.list.lock().clone()
    }
    pub fn has(&self, repo: &str) -> bool {
        self.list.lock().iter().any(|r| r.to_lowercase() == repo.to_lowercase())
    }
    pub fn add(&self, repo: &str) {
        if self.has(repo) {
            return;
        }
        let list = { let mut l = self.list.lock(); l.push(repo.to_string()); l.clone() };
        if let Some(save) = &self.save {
            save(&list);
        }
    }
    pub fn remove(&self, repo: &str) {
        let list = { let mut l = self.list.lock(); l.retain(|r| r.to_lowercase() != repo.to_lowercase()); l.clone() };
        if let Some(save) = &self.save {
            save(&list);
        }
    }
}

/// Optionen für `VisualTools` (JS: das Objekt hinter dem Pfad bzw. Ziel)
#[derive(Clone, Default)]
pub struct VisualOpts {
    /// `a.seiten` des Modells, unverändert (nur `view`)
    pub seiten: Value,
    /// `a.aktionen` des Modells, unverändert (nur `browse`)
    pub aktionen: Value,
    /// `a.warten` des Modells, unverändert (nur `browse`)
    pub warten: Value,
    pub chat: String,
    pub on_live: Option<LiveFn>,
    pub signal: Option<Signal>,
    /// Ports lokaler Server, die der Browser öffnen darf (nur `browse`, ENTSCHEIDE E48)
    pub local_ports: HashSet<u16>,
}

/// Was tools.js auf `ctx.visual` aufruft (src/main/visual.js, ENTSCHEIDE E48)
#[async_trait]
pub trait VisualTools: Send + Sync {
    /// `visual.view(folder, path, { seiten, chat, onLive, signal })`: Werkzeug `view_file`. Liefert
    /// `{ ok, output, summary, display?, images? }`.
    async fn view(&self, folder: &str, path: &str, opts: VisualOpts) -> Result<Value>;
    /// `visual.browse(folder, ziel, { aktionen, warten, chat, onLive, signal, localPorts })`: Werkzeug `browser`.
    /// `ziel` ist leer, wenn das Modell keines angibt (auf der offenen Seite weiterarbeiten).
    async fn browse(&self, folder: &str, ziel: &str, opts: VisualOpts) -> Result<Value>;
    /// Werkzeug `mod_testen` (ENTSCHEIDE E75): Mod in einer Testansicht der Oberfläche laden, Bildschirmfoto und Bericht
    async fn mod_test(&self, _folder: &str, _test: crate::visual::ModTest) -> Result<Value> {
        bail!("Mods testen geht nur in der App.")
    }
}

/// Das `ctx`-Objekt von `runTool`
#[derive(Clone, Default)]
pub struct ToolCtx {
    pub signal: Option<Signal>,
    pub on_progress: Option<ProgressFn>,
    pub on_live: Option<LiveFn>,
    pub github: Option<Arc<GitHub>>,
    pub github_trust: Option<Arc<GithubTrust>>,
    pub visual: Option<Arc<dyn VisualTools>>,
    pub servers: Option<Arc<Servers>>,
    pub web: Option<Arc<Web>>,
    /// Bilder mit Flux; nur gesetzt, wenn ASK dem Konto das Werkzeug anbietet
    pub images: Option<Arc<Images>>,
    /// Frühere Chats freigegeben (Schalter «Chats»)
    pub chat_memory: bool,
    /// Das Modell kann Bilder ansehen
    pub can_see: bool,
    pub chat: String,
    pub chat_title: String,
    pub knowledge: Option<KnowledgeCtx>,
    /// Arbeitsordner; leer: keiner
    pub folder: String,
    pub confirm: Option<ConfirmFn>,
    /// Der Bereich «GitHub» selbst ruft auf: darf auch während eines Abgleichs
    pub panel: bool,
}

/// Ergebnis eines Werkzeugs
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ToolResult {
    pub ok: bool,
    pub output: String,
    pub summary: String,
    pub display: Option<String>,
    /// Aufgabenliste (update_tasks)
    pub tasks: Option<Vec<Value>>,
    pub sources: Vec<Source>,
    /// Bilder des Ergebnisses (Pfade im Arbeitsordner)
    pub images: Vec<String>,
    /// false: die Bilder gehen nicht automatisch an das Modell
    pub view: Option<bool>,
    pub plan: Option<Value>,
    /// Datei, die das Werkzeug neu angelegt hat (zählt für die Rückfrage wie von write_file geschrieben)
    pub made: Option<String>,
}

impl ToolResult {
    pub fn new(ok: bool, output: impl Into<String>, summary: impl Into<String>) -> Self {
        ToolResult { ok, output: output.into(), summary: summary.into(), ..Default::default() }
    }

    fn with_display(mut self, display: impl Into<String>) -> Self {
        self.display = Some(display.into());
        self
    }

    /// Ergebnis aus JSON `{ ok, output, summary, display?, images?, sources?, tasks?, view?, plan? }`;
    /// fehlt `ok`, gilt es als gelungen (wie `result.ok !== false` in agent.js)
    pub fn from_value(v: &Value) -> Self {
        ToolResult {
            ok: v.get("ok") != Some(&json!(false)),
            output: str_of(v, "output"),
            summary: str_of(v, "summary"),
            display: v.get("display").filter(|d| !d.is_null()).map(js_string),
            tasks: v.get("tasks").and_then(Value::as_array).cloned(),
            sources: v
                .get("sources")
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|s| Source { url: str_of(s, "url"), title: str_of(s, "title") }).collect())
                .unwrap_or_default(),
            images: v.get("images").and_then(Value::as_array).map(|a| a.iter().map(js_string).collect()).unwrap_or_default(),
            view: v.get("view").and_then(Value::as_bool),
            plan: v.get("plan").filter(|p| !p.is_null()).cloned(),
            made: None,
        }
    }

    /// Als JSON mit denselben Feldern wie in JavaScript
    pub fn to_value(&self) -> Value {
        let mut m = Map::new();
        m.insert("ok".into(), json!(self.ok));
        m.insert("output".into(), json!(self.output));
        m.insert("summary".into(), json!(self.summary));
        if let Some(d) = &self.display {
            m.insert("display".into(), json!(d));
        }
        if let Some(t) = &self.tasks {
            m.insert("tasks".into(), json!(t));
        }
        if !self.sources.is_empty() {
            m.insert("sources".into(), json!(self.sources));
        }
        if !self.images.is_empty() {
            m.insert("images".into(), json!(self.images));
        }
        if let Some(v) = self.view {
            m.insert("view".into(), json!(v));
        }
        if let Some(p) = &self.plan {
            m.insert("plan".into(), p.clone());
        }
        Value::Object(m)
    }
}

impl From<memorytools::ToolOutput> for ToolResult {
    fn from(o: memorytools::ToolOutput) -> Self {
        ToolResult { ok: o.ok, output: o.output, summary: o.summary, display: o.display, ..Default::default() }
    }
}

// ---------- Befehle ausführen ----------

/// Führt einen Befehl aus und liefert die Ausgabe laufend (`on_progress`). Stoppt über `signal`.
/// Braucht der Befehl Python (python, py, pip, ein .py-Skript)?
pub fn uses_python(command: &str) -> bool {
    command.split(|c: char| c.is_whitespace() || matches!(c, '&' | '|' | '(' | ')' | ';' | '"'))
        .map(|w| w.to_lowercase())
        .any(|w| {
            let w = w.rsplit(['\\', '/']).next().unwrap_or("").trim_end_matches(".exe").to_string();
            matches!(w.as_str(), "python" | "python3" | "py" | "pip" | "pip3") || w.ends_with(".py")
        })
}

/// Python entpackt die installierte App beim ersten Start im Hintergrund (ENTSCHEIDE E39, E61). Ein Befehl mit
/// Python wartet darauf und zeigt den Fortschritt; alles andere läuft sofort.
async fn wait_for_python(command: &str, ctx: &ToolCtx) -> Result<()> {
    if !crate::paths::is_packaged() || !uses_python(command) {
        return Ok(());
    }
    let runtime = crate::python::shared();
    if runtime.ready() {
        return Ok(());
    }
    let progress = ctx.on_progress.clone();
    runtime.ensure(move |p| {
        if let Some(f) = &progress {
            f(&format!("Python wird eingerichtet (einmalig) … {p} %"));
        }
    }).await?;
    Ok(())
}

async fn run_command(terminal: &dyn TerminalEndpoint, args: &Value, ctx: &ToolCtx) -> Result<ToolResult> {
    let command = str_of(args, "command").trim().to_string();
    if command.is_empty() {
        bail!("command fehlt");
    }
    let blocked = blocked_command(&command);
    if !blocked.is_empty() {
        return Ok(ToolResult::new(false, blocked, "abgelehnt"));
    }
    wait_for_python(&command, ctx).await?;
    let wait = num_of(args, "wait");
    let limit = (if wait == 0.0 { 120.0 } else { wait }).clamp(5.0, 600.0);
    let cancelled = || ctx.signal.as_ref().is_some_and(Signal::is_cancelled);
    if cancelled() {
        bail!("abgebrochen");
    }
    let started = request(terminal, "POST", &format!("/execute?{}", q(&[("wait", json!(POLL_SECONDS))])), Some(&json!({ "command": command })), REQUEST_TIMEOUT_MS).await?;
    let Some(mut data) = started.json.clone().filter(|_| started.status == 200) else { return Err(fail(&started)) };
    let id = data.get("id").cloned().unwrap_or(Value::Null);
    let id_text = js_string(&id);
    let mut output = output_text(&data);
    trim_live(&mut output);
    let mut offset = data.get("next_offset").and_then(Value::as_i64).unwrap_or(output_len(&data) as i64);
    let end = Instant::now() + Duration::from_secs_f64(limit);
    let snapshot = EndpointSnapshot::of(terminal);
    let kill_path = format!("/execute/{}", encode_uri_component(&id_text));
    let is_running = |d: &Value| d.get("status") == Some(&json!("running"));
    // Stopp während des Starts: Der Befehl läuft schon, das Ereignis kommt aber nicht mehr
    if cancelled() {
        if is_running(&data) {
            let _ = request(&snapshot, "DELETE", &kill_path, None, REQUEST_TIMEOUT_MS).await;
        }
        bail!("abgebrochen");
    }
    // Stopp während des Wartens: der Befehl wird beendet (läuft weiter, auch wenn diese Funktion schon zurück ist)
    let killer = ctx.signal.clone().map(|sig| {
        let (snap, path) = (snapshot.clone(), kill_path.clone());
        tokio::spawn(async move {
            sig.cancelled().await;
            let _ = request(&snap, "DELETE", &path, None, REQUEST_TIMEOUT_MS).await;
        })
    });
    let looped: Result<()> = async {
        while is_running(&data) && Instant::now() < end {
            if cancelled() {
                bail!("abgebrochen");
            }
            if let Some(f) = &ctx.on_progress {
                f(&output);
            }
            let res = request(
                terminal,
                "GET",
                &format!("/execute/{}/status?{}", encode_uri_component(&id_text), q(&[("wait", json!(POLL_SECONDS)), ("offset", json!(offset))])),
                None,
                REQUEST_TIMEOUT_MS,
            )
            .await?;
            let Some(json) = res.json.clone().filter(|_| res.status == 200) else { return Err(fail(&res)) };
            data = json;
            output.push_str(&output_text(&data));
            trim_live(&mut output);
            offset = data.get("next_offset").and_then(Value::as_i64).unwrap_or(offset + output_len(&data) as i64);
        }
        Ok(())
    }
    .await;
    if let Some(k) = killer {
        if !cancelled() {
            k.abort();
        }
    }
    looped?;
    let running = is_running(&data);
    let exit_code = data.get("exit_code").cloned();
    let exit_text = match &exit_code {
        None => "undefined".to_string(),
        Some(Value::Null) => "null".to_string(),
        Some(v) => js_string(v),
    };
    let head = if running { format!("Läuft nach {limit} s noch (id {}). Mit command_status nachfragen oder warten.", tpl(&json!({ "id": id }), "id")) } else { format!("Exitcode {exit_text}") };
    let exit_ok = exit_code.as_ref().and_then(Value::as_f64) == Some(0.0);
    let shown = clip(&output, MAX_OUTPUT);
    Ok(ToolResult {
        ok: running || exit_ok,
        output: format!("{head}\n{}", if shown.is_empty() { "(keine Ausgabe)" } else { &shown }),
        display: Some(if output.trim_end().is_empty() { "(keine Ausgabe)".to_string() } else { output.trim_end().to_string() }),
        summary: if running { "läuft noch".to_string() } else if exit_ok { String::new() } else { format!("Exitcode {exit_text}") },
        ..Default::default()
    })
}

async fn command_status(terminal: &dyn TerminalEndpoint, args: &Value) -> Result<ToolResult> {
    let res = request(terminal, "GET", &format!("/execute/{}/status?{}", encode_uri_component(&js_string(&arg(args, "id"))), q(&[("wait", json!(30))])), None, REQUEST_TIMEOUT_MS).await?;
    let Some(json) = res.json.clone().filter(|_| res.status == 200) else { return Err(fail(&res)) };
    let mut out = output_text(&json);
    trim_live(&mut out);
    let head = if json.get("status") == Some(&json!("running")) {
        "Läuft noch.".to_string()
    } else {
        format!("Exitcode {}", match json.get("exit_code") {
            None => "undefined".to_string(),
            Some(Value::Null) => "null".to_string(),
            Some(v) => js_string(v),
        })
    };
    let shown = if out.trim_end().is_empty() { head.clone() } else { out.trim_end().to_string() };
    Ok(ToolResult { ok: true, output: format!("{head}\n{}", clip(&out, MAX_OUTPUT)), display: Some(shown), summary: head, ..Default::default() })
}

async fn read_file(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let res = request(
        terminal,
        "GET",
        &format!("/files/read?{}", q(&[("path", arg(a, "path")), ("start_line", arg(a, "start_line")), ("end_line", arg(a, "end_line"))])),
        None,
        REQUEST_TIMEOUT_MS,
    )
    .await?;
    if res.status != 200 {
        return Err(fail(&res));
    }
    let Some(content) = res.json.as_ref().and_then(|j| j.get("content")).and_then(Value::as_str) else {
        return Ok(ToolResult::new(true, format!("Binärdatei ({}), kann nicht als Text gelesen werden.", res.kind), "Binärdatei"));
    };
    let total = match res.json.as_ref().and_then(|j| j.get("total_lines")) {
        Some(Value::Number(n)) if n.as_f64() == Some(1.0) => "1 Zeile".to_string(),
        Some(v) => format!("{} Zeilen", js_string(v)),
        None => "undefined Zeilen".to_string(),
    };
    Ok(ToolResult::new(true, format!("{}\n\n({total} insgesamt)", clip(content, 30000)), zeilen(lines(content))))
}

async fn write_file(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let content = str_of(a, "content");
    let mut body = Map::new();
    if let Some(p) = a.get("path") {
        body.insert("path".into(), p.clone());
    }
    body.insert("content".into(), json!(content));
    let res = request(terminal, "POST", "/files/write", Some(&Value::Object(body)), REQUEST_TIMEOUT_MS).await?;
    if res.status != 200 {
        return Err(fail(&res));
    }
    let n = lines(&content);
    Ok(ToolResult::new(true, format!("Geschrieben: {} ({})", tpl(a, "path"), zeilen(n)), zeilen(n)).with_display(content))
}

async fn edit_file(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let mut body = Map::new();
    if let Some(p) = a.get("path") {
        body.insert("path".into(), p.clone());
    }
    body.insert(
        "replacements".into(),
        json!([{ "target": str_of(a, "old_text"), "replacement": str_of(a, "new_text"), "allow_multiple": a.get("replace_all").is_some_and(truthy) }]),
    );
    let res = request(terminal, "POST", "/files/replace", Some(&Value::Object(body)), REQUEST_TIMEOUT_MS).await?;
    if res.status != 200 {
        return Err(fail(&res));
    }
    let old = tpl(a, "old_text");
    let new = tpl(a, "new_text");
    let display = old.split('\n').map(|l| format!("- {l}")).chain(new.split('\n').map(|l| format!("+ {l}"))).collect::<Vec<_>>().join("\n");
    Ok(ToolResult::new(true, format!("Geändert: {}", tpl(a, "path")), format!("{} neu", zeilen(lines(&str_of(a, "new_text"))))).with_display(display))
}

async fn list_files(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let dir = Some(or_empty(a, "path")).filter(|p| !p.is_empty()).unwrap_or_else(|| ".".into());
    let res = request(terminal, "GET", &format!("/files/list?{}", q(&[("directory", json!(dir))])), None, REQUEST_TIMEOUT_MS).await?;
    let Some(json) = res.json.clone().filter(|_| res.status == 200) else { return Err(fail(&res)) };
    let entries: Vec<String> = json
        .get("entries")
        .filter(|e| truthy(e))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .map(|e| {
                    let dir = e.get("type") == Some(&json!("directory")) || truthy(e.get("is_dir").unwrap_or(&Value::Null)) || truthy(e.get("isDirectory").unwrap_or(&Value::Null));
                    let name = tpl(e, "name");
                    if dir {
                        format!("{name}/")
                    } else {
                        match e.get("size") {
                            Some(s) => format!("{name} ({} B)", js_string(s)),
                            None => name,
                        }
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let n = entries.len();
    Ok(ToolResult::new(true, if entries.is_empty() { "(leer)".to_string() } else { entries.join("\n") }, format!("{n} Einträge")))
}

/// Erster Wert einer Liste von Feldern, der in JavaScript wahr ist (`a.x || a.y || …`)
fn first_truthy(v: &Value, keys: &[&str]) -> Vec<Value> {
    keys.iter().filter_map(|k| v.get(*k)).find(|x| truthy(x)).and_then(Value::as_array).cloned().unwrap_or_default()
}

async fn find_files(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let dir = Some(or_empty(a, "path")).filter(|p| !p.is_empty()).unwrap_or_else(|| ".".into());
    let res = request(terminal, "GET", &format!("/files/glob?{}", q(&[("pattern", arg(a, "pattern")), ("path", json!(dir)), ("max_results", json!(200))])), None, REQUEST_TIMEOUT_MS).await?;
    let Some(json) = res.json.clone().filter(|_| res.status == 200) else { return Err(fail(&res)) };
    let list = first_truthy(&json, &["matches", "results", "entries"]);
    let names: Vec<String> = list
        .iter()
        .map(|m| match m {
            Value::String(s) => s.clone(),
            other => {
                let p = other.get("path").filter(|x| truthy(x)).or_else(|| other.get("name"));
                p.map(js_string).unwrap_or_default()
            }
        })
        .collect();
    Ok(ToolResult::new(true, if names.is_empty() { "Keine Treffer.".to_string() } else { names.join("\n") }, format!("{} Treffer", names.len())))
}

async fn search_text(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let dir = Some(or_empty(a, "path")).filter(|p| !p.is_empty()).unwrap_or_else(|| ".".into());
    let res = request(
        terminal,
        "GET",
        &format!("/files/grep?{}", q(&[("query", arg(a, "text")), ("path", json!(dir)), ("regex", json!(false)), ("case_insensitive", json!(true))])),
        None,
        REQUEST_TIMEOUT_MS,
    )
    .await?;
    let Some(json) = res.json.clone().filter(|_| res.status == 200) else { return Err(fail(&res)) };
    let list = first_truthy(&json, &["matches", "results"]);
    let found: Vec<String> = list
        .iter()
        .map(|m| match m {
            Value::String(s) => s.clone(),
            other => {
                let file = other.get("file").filter(|x| truthy(x)).or_else(|| other.get("path")).map(js_string).unwrap_or_else(|| "undefined".into());
                let line = [other.get("line"), other.get("line_number")].into_iter().flatten().find(|x| !x.is_null()).map(js_string).unwrap_or_else(|| "undefined".into());
                let text = [other.get("content"), other.get("text")].into_iter().flatten().find(|x| !x.is_null()).map(js_string).unwrap_or_default();
                format!("{file}:{line}: {}", text.trim())
            }
        })
        .collect();
    let clipped = clip(&found.join("\n"), MAX_OUTPUT);
    Ok(ToolResult::new(true, if clipped.is_empty() { "Keine Treffer.".to_string() } else { clipped }, format!("{} Treffer", found.len())))
}

// ---------- Quellen (ENTSCHEIDE E82) ----------

/// Eine Unterlage mit Ort und Verweis für die Antwort
fn source_head(r: &Value) -> String {
    let file = str_of(r, "datei");
    let ort = str_of(r, "ort");
    let name = file.rsplit('/').next().unwrap_or(&file).to_string();
    let label = if ort.is_empty() { name } else { format!("{name}, {ort}") };
    format!("{file} · {ort} · Verweis: [{label}]({})", crate::sources::link(&file, r.get("seite").and_then(Value::as_u64)))
}

fn source_notes(json: &Value) -> String {
    let notes: Vec<String> = json["hinweise"].as_array().into_iter().flatten().map(|h| format!("- {}: {}", str_of(h, "datei"), str_of(h, "hinweis"))).collect();
    if notes.is_empty() { String::new() } else { format!("\n\nHinweise zu Unterlagen:\n{}", notes.join("\n")) }
}

async fn quellen_suchen(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let res = request(terminal, "GET", &format!("/quellen/search?{}", q(&[("query", arg(a, "suche")), ("limit", arg(a, "anzahl"))])), None, REQUEST_TIMEOUT_MS).await?;
    let Some(json) = res.json.clone().filter(|_| res.status == 200) else { return Err(fail(&res)) };
    let results = json["results"].as_array().cloned().unwrap_or_default();
    let files = json["dateien"].as_u64().unwrap_or(0);
    if results.is_empty() {
        let text = if files == 0 {
            format!("Im Ordner {} gibt es keine durchsuchbaren Unterlagen.", crate::sources::FOLDER)
        } else {
            format!("Keine Treffer in {files} Unterlagen. Versuche andere Wörter (Fachbegriff, Einzahl, Synonym) oder lies eine Unterlage mit quelle_lesen.")
        };
        return Ok(ToolResult::new(true, format!("{text}{}", source_notes(&json)), "keine Treffer"));
    }
    let list: Vec<String> = results.iter().enumerate().map(|(i, r)| format!("[{}] {}\n{}", i + 1, source_head(r), str_of(r, "text"))).collect();
    let n = results.len();
    Ok(ToolResult::new(true, clip(&format!("{}{}", list.join("\n\n"), source_notes(&json)), MAX_OUTPUT), if n == 1 { "1 Treffer".to_string() } else { format!("{n} Treffer") }))
}

async fn quelle_lesen(terminal: &dyn TerminalEndpoint, a: &Value) -> Result<ToolResult> {
    let ort = a.get("ort").filter(|v| truthy(v)).cloned().unwrap_or(Value::Null);
    let res = request(terminal, "GET", &format!("/quellen/read?{}", q(&[("path", arg(a, "datei")), ("seite", arg(a, "seite")), ("ort", ort)])), None, REQUEST_TIMEOUT_MS).await?;
    let Some(json) = res.json.clone().filter(|_| res.status == 200) else { return Err(fail(&res)) };
    let teile: Vec<String> = json["teile"].as_array().into_iter().flatten().map(js_string).collect();
    let more = if teile.len() > 1 { format!("\n\nTeile dieser Unterlage: {}", teile.iter().take(60).cloned().collect::<Vec<_>>().join(", ")) } else { String::new() };
    let hint = json.get("hinweis").filter(|h| truthy(h)).map(|h| format!("\nHinweis: {}", js_string(h))).unwrap_or_default();
    if str_of(&json, "ort").is_empty() {
        return Ok(ToolResult::new(true, format!("{}: kein lesbarer Text.{hint}{more}", str_of(&json, "datei")), "ohne Text"));
    }
    let text = str_of(&json, "text");
    let cut = if json["gekuerzt"].as_bool() == Some(true) { "\n(gekürzt)" } else { "" };
    Ok(ToolResult::new(true, format!("{}\n\n{text}{cut}{hint}{more}", source_head(&json)), str_of(&json, "ort")))
}

fn update_tasks(a: &Value) -> ToolResult {
    // Die Liste gehört zum Chat und steht dort im Ergebnis dieses Aufrufs (früher: eine Datei für den ganzen Ordner)
    let status = |s: Option<&Value>| match s.and_then(Value::as_str) {
        Some(x @ ("offen" | "in_arbeit" | "erledigt")) => x.to_string(),
        _ => "offen".to_string(),
    };
    let tasks: Vec<Value> = a
        .get("aufgaben")
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter(|t| truthy(t) && !or_empty(t, "titel").trim().is_empty())
                .take(100)
                .map(|t| json!({ "titel": take_chars(js_string(&t["titel"]).trim(), 300), "status": status(t.get("status")) }))
                .collect()
        })
        .unwrap_or_default();
    let done = tasks.iter().filter(|t| t["status"] == "erledigt").count();
    let summary = format!("{done} von {} erledigt", tasks.len());
    ToolResult { ok: true, output: "Aufgabenliste gespeichert.".into(), summary, tasks: Some(tasks), ..Default::default() }
}

// ---------- GitHub ----------

/// GitHub: Projekt holen und hochladen (src/main/github.js). Das Token kennt nur der Hauptprozess;
/// Anlegen und Hochladen bestätigt die Person in einem Dialog (ctx.confirm).
const NOT_CONNECTED: &str = "Nicht mit GitHub verbunden. Sag der Person: in der App /github eingeben und ein Token eintragen.";

enum Gate {
    Go(Arc<GitHub>, String),
    Stop(Box<ToolResult>),
}

fn github_gate(ctx: &ToolCtx) -> Result<Gate> {
    let gh = match (&ctx.github, ctx.folder.is_empty()) {
        (Some(gh), false) => gh.clone(),
        _ => bail!("GitHub ist hier nicht verfügbar."),
    };
    if !gh.connected() {
        return Ok(Gate::Stop(Box::new(ToolResult::new(false, NOT_CONNECTED, "nicht verbunden"))));
    }
    // Die Person gleicht gerade selbst im Bereich «GitHub» ab (main.js): nicht gleichzeitig
    if !ctx.panel && gh.locked(Path::new(&ctx.folder)) {
        return Ok(Gate::Stop(Box::new(ToolResult::new(false, "Die Person gleicht gerade selbst mit GitHub ab. Kurz warten und nochmals versuchen.", "Abgleich läuft"))));
    }
    Ok(Gate::Go(gh, ctx.folder.clone()))
}

fn strs(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().map(js_string).collect()).unwrap_or_default()
}

fn list_of(items: &[String], max: usize) -> String {
    if items.is_empty() {
        return "–".to_string();
    }
    let more = if items.len() > max { format!(" und {} weitere", items.len() - max) } else { String::new() };
    format!("{}{more}", items.iter().take(max).cloned().collect::<Vec<_>>().join(", "))
}

fn list15(items: &[String]) -> String {
    list_of(items, 15)
}

async fn ask_user(ctx: &ToolCtx, message: String, detail: String, options: ConfirmOptions) -> Confirmed {
    match &ctx.confirm {
        Some(c) => c(message, detail, options).await,
        None => Confirmed::default(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

async fn github_status(ctx: &ToolCtx) -> Result<ToolResult> {
    let g = github_gate(ctx)?;
    let Gate::Go(gh, folder) = g else { return Ok(gate_stop(g)) };
    let s = gh.status(Path::new(&folder)).await?;
    if !truthy(&s["linked"]) {
        return Ok(ToolResult::new(
            true,
            format!("Mit GitHub verbunden als {}. Der Ordner ist mit keinem Repository verbunden (github_clone oder github_create).", tpl(&s, "login")),
            "kein Repository",
        ));
    }
    let (changed, added, deleted) = (strs(&s["changed"]), strs(&s["added"]), strs(&s["deleted"]));
    let newer = truthy(&s["remoteNewer"]);
    let output = [
        format!("Repository: {} ({}), Branch {}", tpl(&s, "repo"), tpl(&s, "url"), tpl(&s, "branch")),
        format!("Geändert: {}", list15(&changed)),
        format!("Neu: {}", list15(&added)),
        format!("Gelöscht: {}", list15(&deleted)),
        if newer { "Auf GitHub gibt es neuere Änderungen: zuerst github_pull.".to_string() } else { "GitHub ist auf dem Stand des letzten Abgleichs.".to_string() },
    ]
    .join("\n");
    let summary = format!("{} · {} Änderungen hier{}", tpl(&s, "repo"), changed.len() + added.len() + deleted.len(), if newer { " · auf GitHub Neueres" } else { "" });
    Ok(ToolResult::new(true, output, summary))
}

fn gate_stop(g: Gate) -> ToolResult {
    match g {
        Gate::Stop(r) => *r,
        Gate::Go(..) => ToolResult::default(),
    }
}

async fn github_clone(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let g = github_gate(ctx)?;
    let Gate::Go(gh, folder) = g else { return Ok(gate_stop(g)) };
    let r = gh.clone_repo(Path::new(&folder), &js_string(&arg(a, "repo"))).await?;
    let (written, conflicts) = (strs(&r["written"]), strs(&r["conflicts"]));
    let output = format!(
        "Geholt von {}: {} Dateien.{}",
        tpl(&r, "repo"),
        written.len(),
        if conflicts.is_empty() { String::new() } else { format!(" Nicht überschrieben, weil hier schon anders vorhanden: {}.", list15(&conflicts)) }
    );
    let summary = format!("{} Dateien von {}{}", written.len(), tpl(&r, "repo"), if conflicts.is_empty() { String::new() } else { format!(" · {} Konflikte", conflicts.len()) });
    Ok(ToolResult::new(true, output, summary))
}

async fn github_pull(ctx: &ToolCtx) -> Result<ToolResult> {
    let g = github_gate(ctx)?;
    let Gate::Go(gh, folder) = g else { return Ok(gate_stop(g)) };
    let r = gh.pull(Path::new(&folder)).await?;
    let (written, removed, conflicts) = (strs(&r["written"]), strs(&r["removed"]), strs(&r["conflicts"]));
    let output = [
        format!("Geholt: {}", list15(&written)),
        format!("Gelöscht (auf GitHub entfernt): {}", list15(&removed)),
        if conflicts.is_empty() {
            "Keine Konflikte.".to_string()
        } else {
            format!("Konflikte (hier und auf GitHub geändert, nichts überschrieben): {}. Die Person entscheiden lassen, welche Fassung gilt.", list15(&conflicts))
        },
    ]
    .join("\n");
    let summary = format!(
        "{} geholt{}{}",
        written.len(),
        if removed.is_empty() { String::new() } else { format!(", {} gelöscht", removed.len()) },
        if conflicts.is_empty() { String::new() } else { format!(" · {} Konflikte", conflicts.len()) }
    );
    Ok(ToolResult::new(true, output, summary))
}

async fn github_push(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let g = github_gate(ctx)?;
    let Gate::Go(gh, folder) = g else { return Ok(gate_stop(g)) };
    let root = Path::new(&folder);
    let s = gh.status(root).await?;
    if !truthy(&s["linked"]) {
        return Ok(ToolResult::new(false, "Der Ordner ist mit keinem Repository verbunden (github_create oder github_clone).", "kein Repository"));
    }
    let changes: Vec<String> = ["changed", "added", "deleted"].iter().flat_map(|k| strs(&s[*k])).collect();
    if changes.is_empty() {
        return Ok(ToolResult::new(true, "Nichts hochzuladen: keine Änderungen seit dem letzten Abgleich.", "nichts zu tun"));
    }
    let clash = strs(&s["conflicts"]);
    let repo = js_string(&s["repo"]);
    let message = js_string(&arg(a, "nachricht"));
    let overwrite = a.get("ueberschreiben") == Some(&json!(true)) && !clash.is_empty();
    if !clash.is_empty() && !overwrite {
        return Ok(ToolResult {
            ok: false,
            summary: plural(clash.len(), "Konflikt", "Konflikte"),
            output: format!(
                "Nicht hochgeladen. Hier und auf GitHub verschieden geändert: {}. Hochladen würde die Fassung auf GitHub ersetzen. Frag die Person, welche Fassung gelten soll. GitHub gewinnt: die Datei hier löschen und github_pull. Hier gewinnt: github_push mit ueberschreiben: true.",
                list15(&clash)
            ),
            ..Default::default()
        });
    }
    // Überschreiben auf GitHub bestätigt die Person immer, auch wenn sie sonst nicht mehr gefragt werden will
    if overwrite {
        let ok = ask_user(
            ctx,
            format!("{} auf GitHub mit der Fassung von hier ersetzen?", plural(clash.len(), "Datei", "Dateien")),
            format!(
                "Repository: {repo}\nNachricht: {message}\n\nAuf GitHub inzwischen geändert:\n{}\n\nDie Änderungen auf GitHub an diesen Dateien gehen verloren (im Verlauf von GitHub bleiben sie).",
                list_of(&clash, 20)
            ),
            ConfirmOptions::default(),
        )
        .await
        .ok;
        if !ok {
            return Ok(ToolResult::new(false, "Die Person hat das Überschreiben abgelehnt.", "abgelehnt"));
        }
    } else if !ctx.github_trust.as_ref().is_some_and(|t| t.has(&repo)) {
        let answer = ask_user(
            ctx,
            format!("{} Änderungen auf GitHub hochladen?", changes.len()),
            format!("Repository: {repo}\nNachricht: {message}\n\n{}", list_of(&changes, 20)),
            ConfirmOptions { checkbox: Some("Für dieses Repository nicht mehr fragen".into()), danger: false },
        )
        .await;
        if !answer.ok {
            return Ok(ToolResult::new(false, "Die Person hat das Hochladen abgelehnt.", "abgelehnt"));
        }
        if answer.checked {
            if let Some(t) = &ctx.github_trust {
                t.add(&repo);
            }
        }
    }
    let r = gh.push(root, &message, overwrite).await?;
    let deleted = strs(&r["deleted"]);
    Ok(ToolResult::new(
        true,
        format!("Hochgeladen: {} Dateien, {} gelöscht. {}", tpl(&r, "uploaded"), deleted.len(), tpl(&r, "url")),
        format!("{} hochgeladen{} · {repo}", tpl(&r, "uploaded"), if deleted.is_empty() { String::new() } else { format!(", {} gelöscht", deleted.len()) }),
    ))
}

async fn github_create(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let g = github_gate(ctx)?;
    let Gate::Go(gh, folder) = g else { return Ok(gate_stop(g)) };
    let privat = a.get("privat") != Some(&json!(false));
    let account = gh.whoami().await?;
    let ok = ask_user(
        ctx,
        format!(
            "Neues {} GitHub-Repository «{}» anlegen und den Arbeitsordner hochladen?",
            if privat { "privates" } else { "ÖFFENTLICHES" },
            tpl(a, "name")
        ),
        format!("Konto: {account}\nOrdner: {folder}\nHochgeladen wird alles ausser .ask (Chats, Skripte)."),
        ConfirmOptions::default(),
    )
    .await
    .ok;
    if !ok {
        return Ok(ToolResult::new(false, "Die Person hat das Anlegen abgelehnt.", "abgelehnt"));
    }
    let r = gh.create(Path::new(&folder), tpl(a, "name").trim(), privat, &or_empty(a, "beschreibung")).await?;
    Ok(ToolResult::new(
        true,
        format!(
            "Angelegt: {} ({}), {} Dateien hochgeladen. {}",
            tpl(&r, "repo"),
            if truthy(&r["private"]) { "privat" } else { "öffentlich" },
            tpl(&r, "uploaded"),
            tpl(&r, "url")
        ),
        format!("{} · {} Dateien", tpl(&r, "repo"), tpl(&r, "uploaded")),
    ))
}

async fn github_branch(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let g = github_gate(ctx)?;
    let Gate::Go(gh, folder) = g else { return Ok(gate_stop(g)) };
    let root = Path::new(&folder);
    let aktion = Some(or_empty(a, "aktion")).filter(|x| !x.is_empty()).unwrap_or_else(|| "liste".into());
    if aktion == "liste" {
        let b = gh.branches(root).await?;
        let (default, current) = (js_string(&b["default"]), js_string(&b["current"]));
        let names: Vec<String> = strs(&b["branches"])
            .iter()
            .map(|n| format!("{n}{}{}", if *n == default { " (Standard)" } else { "" }, if *n == current { " (verbunden)" } else { "" }))
            .collect();
        return Ok(ToolResult::new(true, format!("Branches: {}", names.join(", ")), format!("{} Branches · {current}", names.len())));
    }
    let name = or_empty(a, "name");
    if name.trim().is_empty() {
        return Ok(ToolResult::new(false, "Für wechseln und neu braucht es den Namen des Branches (name).", "Name fehlt"));
    }
    if aktion == "neu" {
        let r = gh.create_branch(root, &name).await?;
        let branch = js_string(&r["branch"]);
        return Ok(ToolResult::new(
            true,
            format!(
                "Branch {branch} angelegt (von {}), der Ordner ist jetzt mit ihm verbunden. Änderungen hier gehen beim nächsten github_push auf {branch}. {}",
                tpl(&r, "from"),
                tpl(&r, "url")
            ),
            format!("{branch} angelegt"),
        ));
    }
    if aktion == "wechseln" {
        return match gh.switch_branch(root, &name).await {
            Ok(r) => {
                let branch = js_string(&r["branch"]);
                if truthy(&r["same"]) {
                    return Ok(ToolResult::new(true, format!("Der Ordner ist schon mit {branch} verbunden."), branch));
                }
                let (written, removed, conflicts) = (strs(&r["written"]), strs(&r["removed"]), strs(&r["conflicts"]));
                let output = [
                    format!("Gewechselt von {} zu {branch}.", tpl(&r, "from")),
                    format!("Geholt: {}", list15(&written)),
                    format!("Gelöscht (gibt es auf {branch} nicht): {}", list15(&removed)),
                    if conflicts.is_empty() { String::new() } else { format!("Nicht überschrieben (hier anders vorhanden): {}.", list15(&conflicts)) },
                ]
                .into_iter()
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join("\n");
                Ok(ToolResult::new(true, output, format!("{branch} · {} geholt", written.len())))
            }
            Err(err) => {
                if err.downcast_ref::<PendingChanges>().is_some() {
                    return Ok(ToolResult::new(
                        false,
                        format!("{err} Frag die Person, ob du zuerst hochladen (github_push) oder einen neuen Branch anlegen sollst."),
                        "Änderungen noch nicht hochgeladen",
                    ));
                }
                Err(err)
            }
        };
    }
    Ok(ToolResult::new(false, format!("Unbekannte Aktion «{aktion}». Möglich: liste, wechseln, neu."), "unbekannte Aktion"))
}

async fn github_pr(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let g = github_gate(ctx)?;
    let Gate::Go(gh, folder) = g else { return Ok(gate_stop(g)) };
    let root = Path::new(&folder);
    let aktion = or_empty(a, "aktion");
    if aktion.is_empty() || aktion == "liste" {
        let prs = gh.pull_requests(root).await?;
        let list = prs.as_array().cloned().unwrap_or_default();
        if list.is_empty() {
            return Ok(ToolResult::new(true, "Keine offenen Pull Requests.", "keine offenen"));
        }
        let output = list
            .iter()
            .map(|p| format!("#{} {} ({} → {}, {}) {}", tpl(p, "number"), tpl(p, "title"), tpl(p, "head"), tpl(p, "base"), tpl(p, "user"), tpl(p, "url")))
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(ToolResult::new(true, output, format!("{} offen", list.len())));
    }
    if aktion != "neu" {
        return Ok(ToolResult::new(false, format!("Unbekannte Aktion «{aktion}». Möglich: liste, neu."), "unbekannte Aktion"));
    }
    let titel = or_empty(a, "titel");
    if titel.trim().is_empty() {
        return Ok(ToolResult::new(false, "Für einen neuen Pull Request braucht es einen Titel (titel).", "Titel fehlt"));
    }
    let s = gh.status(root).await?;
    if !truthy(&s["linked"]) {
        return Ok(ToolResult::new(false, "Der Ordner ist mit keinem Repository verbunden (github_create oder github_clone).", "kein Repository"));
    }
    let pending: Vec<String> = ["changed", "added", "deleted"].iter().flat_map(|k| strs(&s[*k])).collect();
    if !pending.is_empty() {
        return Ok(ToolResult::new(
            false,
            format!(
                "Hier gibt es {} Änderungen, die noch nicht hochgeladen sind ({}). Ein Pull Request enthält nur, was auf GitHub liegt: zuerst github_push.",
                pending.len(),
                list_of(&pending, 8)
            ),
            "zuerst hochladen",
        ));
    }
    let ziel = {
        let z = or_empty(a, "ziel").trim().to_string();
        if z.is_empty() { js_string(&gh.branches(root).await?["default"]) } else { z }
    };
    let branch = js_string(&s["branch"]);
    if ziel == branch {
        return Ok(ToolResult::new(
            false,
            format!("Der Ordner ist mit {ziel} verbunden, dem Ziel des Pull Requests. Zuerst mit github_branch (aktion neu) einen eigenen Branch anlegen, dort hochladen, dann den Pull Request erstellen."),
            "eigener Branch nötig",
        ));
    }
    let beschreibung = or_empty(a, "beschreibung");
    let detail = format!("Von {branch} nach {ziel}\nTitel: {titel}{}", if beschreibung.is_empty() { String::new() } else { format!("\n\n{}", take_chars(&beschreibung, 1500)) });
    if !ask_user(ctx, format!("Pull Request in {} erstellen?", js_string(&s["repo"])), detail, ConfirmOptions::default()).await.ok {
        return Ok(ToolResult::new(false, "Die Person hat den Pull Request abgelehnt.", "abgelehnt"));
    }
    let r = gh.create_pull_request(root, &titel, &beschreibung, &ziel).await?;
    Ok(ToolResult::new(
        true,
        format!("Pull Request #{} erstellt ({} → {}): {}", tpl(&r, "number"), tpl(&r, "head"), tpl(&r, "base"), tpl(&r, "url")),
        format!("#{} · {} → {}", tpl(&r, "number"), tpl(&r, "head"), tpl(&r, "base")),
    ))
}

// ---------- Internet ----------

fn web_of(ctx: &ToolCtx) -> Result<Arc<Web>> {
    ctx.web.clone().ok_or_else(|| anyhow!("Das Internet ist hier nicht verfügbar."))
}

async fn web_search(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let web = web_of(ctx)?;
    let r = web.search(&str_of(a, "anfrage"), ctx.signal.as_ref()).await?;
    if r.results.is_empty() {
        let note = r.note.as_deref().filter(|n| !n.is_empty()).map(|n| format!(" ({n})")).unwrap_or_default();
        return Ok(ToolResult::new(true, format!("Keine Treffer für «{}».{note}", tpl(a, "anfrage")), "keine Treffer"));
    }
    let output = r
        .results
        .iter()
        .enumerate()
        .map(|(i, x)| format!("{}. {}\n   {}{}", i + 1, x.title, x.url, if x.snippet.is_empty() { String::new() } else { format!("\n   {}", x.snippet) }))
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ToolResult {
        ok: true,
        output: format!("Treffer ({}):\n{output}", r.engine),
        summary: format!("{} Treffer{}", r.results.len(), if r.engine == "ASK" { String::new() } else { format!(" · {}", r.engine) }),
        sources: r.results.iter().map(|x| Source { url: x.url.clone(), title: x.title.clone() }).collect(),
        ..Default::default()
    })
}

async fn web_fetch(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let web = web_of(ctx)?;
    let p = web
        .page(
            &str_of(a, "url"),
            PageOptions {
                signal: ctx.signal.clone(),
                ab: num_of(a, "ab") as i64,
                root: Some(ctx.folder.clone()).filter(|f| !f.is_empty()).map(PathBuf::from),
                chat: Some(ctx.chat.clone()).filter(|c| !c.is_empty()),
            },
        )
        .await?;
    let sources = vec![Source { url: p.url.clone(), title: p.title.clone() }];
    if let Some(pdf) = &p.pdf {
        return Ok(ToolResult { ok: true, output: format!("PDF gespeichert unter {pdf}. Mit read_file lesen."), summary: "PDF gespeichert".into(), sources, ..Default::default() });
    }
    let more = if p.to < p.total { format!("\n\n(Zeichen {}–{} von {}; weiter mit ab: {})", p.from, p.to, p.total, p.to) } else { String::new() };
    let text = if p.text.is_empty() { "(kein lesbarer Text, die Seite braucht wohl JavaScript; dann browser)" } else { p.text.as_str() };
    let long = p.text.chars().count() > 3000;
    Ok(ToolResult {
        ok: true,
        output: format!("{}\n{}\n\n{text}{more}", p.title, p.url),
        display: Some(format!("{}\n{}\n\n{}{}", p.title, p.url, take_chars(&p.text, 3000), if long { " …" } else { "" })),
        summary: format!("{} · {}k Zeichen", take_chars(&p.title, 60), (p.total as f64 / 100.0).round() / 10.0),
        sources,
        ..Default::default()
    })
}

// ---------- Frühere Chats ----------

fn history_gate(ctx: &ToolCtx) -> Option<ToolResult> {
    if !ctx.chat_memory || ctx.folder.is_empty() {
        return Some(ToolResult::new(false, "Frühere Chats sind nicht freigegeben (Schalter «Chats» oben ist aus).", "nicht freigegeben"));
    }
    None
}

fn chat_search(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    if let Some(stop) = history_gate(ctx) {
        return Ok(stop);
    }
    let hits = history::search(Path::new(&ctx.folder), &str_of(a, "anfrage"), &ctx.chat)?;
    if hits.is_empty() {
        return Ok(ToolResult::new(true, format!("Kein früherer Chat enthält «{}».", tpl(a, "anfrage")), "keine Treffer"));
    }
    let output = hits
        .iter()
        .map(|h| format!("«{}» (id {}, {})\n{}", h.title, h.id, history::date(h.updated), h.snippets.iter().map(|s| format!("   {s}")).collect::<Vec<_>>().join("\n")))
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(ToolResult::new(true, format!("{output}\n\nGanzen Chat mit chat_read lesen."), plural(hits.len(), "Chat", "Chats")))
}

fn chat_read(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    if let Some(stop) = history_gate(ctx) {
        return Ok(stop);
    }
    let r = history::read(Path::new(&ctx.folder), &str_of(a, "id"), num_of(a, "ab") as i64)?;
    let more = if r.to < r.total { format!("\n\n(Zeichen {}–{} von {}; weiter mit ab: {})", r.from, r.to, r.total, r.to) } else { String::new() };
    Ok(ToolResult::new(
        true,
        format!("Chat «{}» vom {}\n\n{}{more}", r.title, history::date(r.updated), r.text),
        format!("«{}»", take_chars(&r.title, 50)),
    ))
}

// ---------- Ansehen ----------

const NO_VISION: &str = "Das Modell in ASK kann keine Bilder ansehen. Prüfe stattdessen mit pruefen.py.";
const SHOT_LATER: &str = "Das Bildschirmfoto folgt in der nächsten Nachricht.";

fn visual_of(ctx: &ToolCtx) -> Result<Arc<dyn VisualTools>> {
    match (&ctx.visual, ctx.folder.is_empty()) {
        (Some(v), false) => Ok(v.clone()),
        _ => bail!("Ansehen ist hier nicht verfügbar."),
    }
}

async fn view_file(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let v = visual_of(ctx)?;
    if !ctx.can_see {
        return Ok(ToolResult::new(false, NO_VISION, "Bilder nicht möglich"));
    }
    let opts = VisualOpts { seiten: arg(a, "seiten"), chat: ctx.chat.clone(), on_live: ctx.on_live.clone(), signal: ctx.signal.clone(), ..Default::default() };
    Ok(ToolResult::from_value(&v.view(&ctx.folder, &str_of(a, "path"), opts).await?))
}

async fn browser(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let v = visual_of(ctx)?;
    // Lokale Adressen nur für Server, die die App in diesem Projekt gestartet hat (ENTSCHEIDE E48)
    let opts = VisualOpts {
        aktionen: arg(a, "aktionen"),
        warten: arg(a, "warten"),
        chat: ctx.chat.clone(),
        on_live: ctx.on_live.clone(),
        signal: ctx.signal.clone(),
        local_ports: ctx.servers.as_ref().map(|s| s.allowed_ports()).unwrap_or_default(),
        ..Default::default()
    };
    let mut r = ToolResult::from_value(&v.browse(&ctx.folder, &or_empty(a, "ziel"), opts).await?);
    if !ctx.can_see {
        r.output = r.output.replace(SHOT_LATER, "Das Bildschirmfoto bekommst du nicht: das Modell in ASK kann keine Bilder ansehen.");
    }
    Ok(r)
}

/// Mod ausprobieren (ENTSCHEIDE E75): in einer unsichtbaren Testansicht von ASK mit Beispieldaten, auch wenn er aus
/// ist. Der Code läuft dort ohne Zugriff auf den Hauptprozess, ohne Internet und mit eigenem, leerem Speicher.
async fn mod_testen(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let v = visual_of(ctx)?;
    let Some(mods) = ctx.knowledge.as_ref().and_then(|k| k.mods.clone()) else { bail!("Mods sind hier nicht verfügbar.") };
    let id = crate::mods::slug(&str_of(a, "id"));
    let Some(info) = mods.info(&id) else {
        let ids: Vec<String> = mods.list().into_iter().map(|m| m.manifest.id).collect();
        return Ok(ToolResult::new(false, format!("Keinen Mod «{id}» gefunden. Vorhanden: {}", if ids.is_empty() { "keine".into() } else { ids.join(", ") }), "nicht gefunden"));
    };
    if !info.fehler.is_empty() {
        return Ok(ToolResult::new(false, format!("mod.json ist fehlerhaft: {}", info.fehler), "fehlerhaft"));
    }
    let test = crate::visual::ModTest {
        id: info.manifest.id.clone(),
        hash: info.hash.clone(),
        style: info.manifest.style.clone(),
        script: info.manifest.script.clone(),
        befehle: serde_json::to_value(&info.manifest.befehle).unwrap_or(Value::Null),
        dark: str_of(a, "farbschema") != "hell",
        aktionen: a.get("aktionen").and_then(Value::as_array).cloned().unwrap_or_default(),
        chat: ctx.chat.clone(),
    };
    let mut r = ToolResult::from_value(&v.mod_test(&ctx.folder, test).await?);
    if !ctx.can_see {
        r.output = r.output.replace("Das Bildschirmfoto folgt in der nächsten Nachricht.", "Das Bildschirmfoto bekommst du nicht: das Modell in ASK kann keine Bilder ansehen. Verlass dich auf den Bericht.");
    }
    Ok(r)
}

// ---------- Entwicklungsserver ----------

/// Entwicklungsserver (src/main/servers.js, ENTSCHEIDE E48): laufen im Terminal weiter, ohne dass der Auftrag
/// wartet. Die Person sieht sie im Browser-Bereich und kann sie dort öffnen und stoppen.
fn servers_of(ctx: &ToolCtx) -> Result<Arc<Servers>> {
    ctx.servers.clone().ok_or_else(|| anyhow!("Server sind hier nicht verfügbar."))
}

fn server_state(state: &str) -> &str {
    match state {
        "fehler" => "Fehler",
        other => other,
    }
}

fn server_line(s: &crate::servers::ServerSnapshot) -> String {
    let exit = match s.exit_code {
        Some(code) if s.state != "läuft" => format!(", Exitcode {code}"),
        _ => String::new(),
    };
    format!(
        "{} «{}»: {}{exit}{} ({})",
        s.id,
        s.name,
        server_state(&s.state),
        if s.url.is_empty() { ", keine Adresse erkannt".to_string() } else { format!(", {}", s.url) },
        s.command
    )
}

async fn server_starten(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    wait_for_python(&str_of(a, "befehl"), ctx).await?;
    let servers = servers_of(ctx)?;
    let command = {
        let b = or_empty(a, "befehl");
        if b.is_empty() { or_empty(a, "command") } else { b }.trim().to_string()
    };
    if command.is_empty() {
        bail!("befehl fehlt");
    }
    let blocked = blocked_command(&command);
    if !blocked.is_empty() {
        return Ok(ToolResult::new(false, blocked, "abgelehnt"));
    }
    let started = servers.start(&command, &or_empty(a, "name"), "modell").await?;
    let s = if started.existing { started.server.clone() } else { servers.wait_ready(&started.server.id, 8000).await.unwrap_or_else(|| started.server.clone()) };
    let out = servers.output(&s.id, 30).join("\n").trim().to_string();
    let head = if started.existing { format!("Läuft schon: {}", server_line(&s)) } else { server_line(&s) };
    let next = if s.state == "fehler" || s.state == "gestoppt" {
        "Der Server ist beendet. Ausgabe prüfen, Fehler beheben und neu starten.".to_string()
    } else if !s.url.is_empty() {
        format!("Mit browser (ziel: {}) öffnen.{}", s.url, if s.ready { "" } else { " Er antwortet noch nicht; kurz warten." })
    } else {
        "Noch keine Adresse erkannt; später mit server_liste nachsehen.".to_string()
    };
    let shown = clip(&out, 4000);
    Ok(ToolResult {
        ok: s.state != "fehler",
        output: format!("{head}\n{next}\n\nAusgabe:\n{}", if shown.is_empty() { "(noch keine)" } else { &shown }),
        display: Some(if out.is_empty() { head } else { out }),
        summary: format!("{}{}", server_state(&s.state), if s.url.is_empty() { String::new() } else { format!(" · {}", s.url) }),
        ..Default::default()
    })
}

async fn server_stoppen(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let servers = servers_of(ctx)?;
    let id = js_string(&arg(a, "id"));
    let Some(s) = servers.get(&id) else {
        return Ok(ToolResult::new(false, format!("Kein Server mit id {}. Mit server_liste nachsehen.", tpl(a, "id")), "nicht gefunden"));
    };
    servers.stop(&s.id).await?;
    Ok(ToolResult::new(true, format!("Gestoppt: {} ({})", s.name, s.id), format!("{} gestoppt", s.name)))
}

fn server_liste(ctx: &ToolCtx) -> Result<ToolResult> {
    let servers = servers_of(ctx)?;
    let list = servers.list();
    if list.is_empty() {
        return Ok(ToolResult::new(true, "Keine Server gestartet.", "keine"));
    }
    let output = list
        .iter()
        .map(|s| {
            let tail = servers.output(&s.id, 8).join("\n   ");
            format!("{}{}", server_line(s), if tail.is_empty() { String::new() } else { format!("\n   {tail}") })
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    let live = list.iter().filter(|s| s.state == "läuft" || s.state == "startet").count();
    Ok(ToolResult::new(true, clip(&output, MAX_OUTPUT), format!("{live} {}", if live == 1 { "läuft" } else { "laufen" })))
}

// ---------- Bilder ----------

async fn bild_erstellen(ctx: &ToolCtx, a: &Value) -> Result<ToolResult> {
    let images = match (&ctx.images, ctx.folder.is_empty()) {
        (Some(i), false) => i.clone(),
        _ => bail!("Bilder erstellen ist hier nicht verfügbar."),
    };
    let r = images
        .create(
            Path::new(&ctx.folder),
            a,
            crate::bild::CreateOptions { chat: ctx.chat.clone(), signal: ctx.signal.clone(), on_progress: ctx.on_progress.clone() },
        )
        .await?;
    let one = r.files.len() == 1;
    Ok(ToolResult {
        ok: true,
        view: Some(false), // die Person sieht die Bilder in der App; das Modell nur auf Wunsch mit view_file
        summary: format!("{} · {} s", if one { r.files[0].clone() } else { format!("{} Bilder", r.files.len()) }, r.seconds),
        output: format!(
            "{}: {}{}\nDie Person sieht {} in der App. Ansehen kannst du {} mit view_file; zum Ändern bild_erstellen mit vorlage.",
            if one { "Bild gespeichert" } else { "Bilder gespeichert" },
            r.files.join(", "),
            if r.text.is_empty() { String::new() } else { format!("\nFlux/ASK: {}", r.text) },
            if one { "das Bild" } else { "die Bilder" },
            if one { "es" } else { "sie" }
        ),
        images: r.files,
        ..Default::default()
    })
}

// ---------- Tabelle der Werkzeuge ----------

/// Beschreibung eines Werkzeugs (JS: ein Eintrag von `TOOLS` ohne `run`)
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub label: String,
    pub description: String,
    pub parameters: Value,
}

fn tool(name: &str, label: &str, description: &str, parameters: Value) -> Tool {
    Tool { name: name.into(), label: label.into(), description: description.into(), parameters }
}

/// Alle Werkzeuge in der Reihenfolge, in der sie dem Modell angeboten werden (JS `TOOLS`)
pub fn tools() -> &'static [Tool] {
    static ALL: OnceLock<Vec<Tool>> = OnceLock::new();
    ALL.get_or_init(build_tools)
}

fn build_tools() -> Vec<Tool> {
    let mut all = vec![
        tool(
            "run_command",
            "Befehl",
            "Führt einen Befehl in cmd.exe im Arbeitsordner aus und liefert Ausgabe und Exitcode. ",
            json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Befehl für cmd.exe, z. B. python auswertung.py" },
                    "wait": { "type": "number", "description": "Höchstens so viele Sekunden warten (Standard 120)." },
                },
                "required": ["command"],
            }),
        ),
        tool(
            "command_status",
            "Befehl",
            "Fragt nach Ausgabe und Exitcode eines Befehls, der beim Aufruf noch lief.",
            json!({ "type": "object", "properties": { "id": { "type": "string", "description": "id aus run_command" } }, "required": ["id"] }),
        ),
        tool(
            "read_file",
            "Lesen",
            "Liest eine Datei im Arbeitsordner. Word, PDF, Excel und PowerPoint werden als Text geliefert. Bei langen Dateien start_line und end_line angeben.",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Pfad relativ zum Arbeitsordner" },
                    "start_line": { "type": "integer", "description": "Erste Zeile (ab 1)" },
                    "end_line": { "type": "integer", "description": "Letzte Zeile" },
                },
                "required": ["path"],
            }),
        ),
        tool(
            "write_file",
            "Schreiben",
            "Schreibt eine Textdatei im Arbeitsordner (legt Ordner an, überschreibt vorhandene Dateien).",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Pfad relativ zum Arbeitsordner" },
                    "content": { "type": "string", "description": "Vollständiger Inhalt" },
                },
                "required": ["path", "content"],
            }),
        ),
        tool(
            "edit_file",
            "Bearbeiten",
            "Ersetzt in einer Datei einen Textabschnitt durch einen neuen. old_text muss genau einmal vorkommen (ausser replace_all ist true).",
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "old_text": { "type": "string", "description": "Bisheriger Text, genau wie in der Datei" },
                    "new_text": { "type": "string", "description": "Neuer Text" },
                    "replace_all": { "type": "boolean", "description": "Alle Vorkommen ersetzen" },
                },
                "required": ["path", "old_text", "new_text"],
            }),
        ),
        tool(
            "list_files",
            "Ordner",
            "Listet den Inhalt eines Ordners im Arbeitsordner auf.",
            json!({ "type": "object", "properties": { "path": { "type": "string", "description": "Ordner, Standard ." } } }),
        ),
        tool(
            "find_files",
            "Suchen",
            "Sucht Dateien nach Namen mit einem Muster wie *.docx oder **/*.py.",
            json!({
                "type": "object",
                "properties": { "pattern": { "type": "string" }, "path": { "type": "string", "description": "Ordner, Standard ." } },
                "required": ["pattern"],
            }),
        ),
        tool(
            "quellen_suchen",
            "Quellen",
            "Durchsucht die Unterlagen im Ordner Quellen (PDF, Word, PowerPoint, Excel, Text) und liefert die besten Stellen mit Datei, Seite oder Abschnitt, Ausschnitt und dem Verweis für die Antwort.",
            json!({
                "type": "object",
                "properties": {
                    "suche": { "type": "string", "description": "Suchwörter, am besten Fachbegriffe, z. B. Photosynthese Chlorophyll" },
                    "anzahl": { "type": "integer", "description": "Höchstens so viele Treffer (Standard 8, höchstens 20)" },
                },
                "required": ["suche"],
            }),
        ),
        tool(
            "quelle_lesen",
            "Quelle",
            "Liest einen ganzen Teil einer Unterlage im Ordner Quellen: eine Seite (PDF), eine Folie (PowerPoint) oder einen Abschnitt (Word, Text). Ohne seite und ort den ersten Teil; das Ergebnis nennt alle Teile.",
            json!({
                "type": "object",
                "properties": {
                    "datei": { "type": "string", "description": "Pfad der Unterlage, z. B. Quellen/Skript.pdf" },
                    "seite": { "type": "integer", "description": "Seite (PDF) oder Folie (PowerPoint)" },
                    "ort": { "type": "string", "description": "Abschnitt oder Blatt, z. B. Zellen" },
                },
                "required": ["datei"],
            }),
        ),
        tool(
            "search_text",
            "Suchen",
            "Sucht einen Text in den Dateien des Arbeitsordners und liefert Datei, Zeile und Fundstelle.",
            json!({
                "type": "object",
                "properties": { "text": { "type": "string" }, "path": { "type": "string", "description": "Ordner oder Datei, Standard ." } },
                "required": ["text"],
            }),
        ),
        tool(
            "update_tasks",
            "Aufgaben",
            "Legt die Aufgabenliste dieses Chats an oder aktualisiert sie. Die Person sieht sie über dem Eingabefeld. Bei Aufträgen mit mehr als zwei Schritten zuerst anlegen und nach jedem Schritt aktualisieren.",
            json!({
                "type": "object",
                "properties": {
                    "auftrag": { "type": "string", "description": "Kurzbeschreibung des Auftrags" },
                    "aufgaben": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": { "titel": { "type": "string" }, "status": { "type": "string", "enum": ["offen", "in_arbeit", "erledigt"] } },
                            "required": ["titel", "status"],
                        },
                    },
                },
                "required": ["aufgaben"],
            }),
        ),
        tool(
            "github_status",
            "GitHub",
            "Zeigt, ob der Arbeitsordner mit einem GitHub-Repository verbunden ist, was hier geändert ist und ob es auf GitHub Neueres gibt.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "github_clone",
            "GitHub",
            "Holt ein GitHub-Repository in den Arbeitsordner und verbindet den Ordner damit. Vorhandene Dateien mit gleichem Namen werden nicht überschrieben.",
            json!({ "type": "object", "properties": { "repo": { "type": "string", "description": "besitzer/name oder GitHub-Adresse" } }, "required": ["repo"] }),
        ),
        tool(
            "github_pull",
            "GitHub",
            "Holt Änderungen vom verbundenen GitHub-Repository. Hier geänderte Dateien werden nie überschrieben, sondern als Konflikt gemeldet.",
            json!({ "type": "object", "properties": {} }),
        ),
        tool(
            "github_push",
            "GitHub",
            concat!(
                "Lädt die Änderungen im Arbeitsordner als Commit auf das verbundene GitHub-Repository hoch (ohne .ask und ohne das, was .gitignore ausschliesst). ",
                "Die Person bestätigt vorher, ausser sie hat das für dieses Repository abgeschaltet."
            ),
            json!({
                "type": "object",
                "properties": {
                    "nachricht": { "type": "string", "description": "Kurze Commit-Nachricht, was sich geändert hat" },
                    "ueberschreiben": { "type": "boolean", "description": "Nur nach einem gemeldeten Konflikt und wenn die Person es will: Die Fassung hier ersetzt die auf GitHub." },
                },
                "required": ["nachricht"],
            }),
        ),
        tool(
            "github_create",
            "GitHub",
            "Legt ein neues GitHub-Repository an (standardmässig privat), verbindet den Arbeitsordner damit und lädt ihn hoch. Die Person bestätigt vorher.",
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Name des Repositorys, z. B. frogger-spiel" },
                    "privat": { "type": "boolean", "description": "Privat (Standard true). Öffentlich nur, wenn die Person es ausdrücklich will." },
                    "beschreibung": { "type": "string" },
                },
                "required": ["name"],
            }),
        ),
        tool(
            "github_branch",
            "GitHub",
            concat!(
                "Branches des verbundenen Repositorys: «liste» zeigt sie, «neu» legt einen Branch auf dem Stand des letzten Abgleichs an und verbindet den Ordner damit ",
                "(Änderungen hier bleiben und gehen beim nächsten github_push auf den neuen Branch), «wechseln» verbindet den Ordner mit einem anderen Branch und holt dessen Stand. ",
                "Wechseln geht nur ohne Änderungen, die noch nicht hochgeladen sind."
            ),
            json!({
                "type": "object",
                "properties": {
                    "aktion": { "type": "string", "enum": ["liste", "wechseln", "neu"] },
                    "name": { "type": "string", "description": "Name des Branches (bei wechseln und neu), z. B. neue-startseite" },
                },
                "required": ["aktion"],
            }),
        ),
        tool(
            "github_pr",
            "GitHub",
            concat!(
                "Pull Requests des verbundenen Repositorys: «liste» zeigt die offenen mit Adresse, «neu» erstellt einen vom Branch des Ordners in den Standard-Branch ",
                "(oder ziel). Er enthält nur, was schon hochgeladen ist; vorher github_push. Die Person bestätigt vorher."
            ),
            json!({
                "type": "object",
                "properties": {
                    "aktion": { "type": "string", "enum": ["liste", "neu"] },
                    "titel": { "type": "string", "description": "Titel des Pull Requests (bei neu)" },
                    "beschreibung": { "type": "string", "description": "Was sich ändert und warum (bei neu)" },
                    "ziel": { "type": "string", "description": "Ziel-Branch, sonst der Standard-Branch" },
                },
                "required": ["aktion"],
            }),
        ),
        tool(
            "web_search",
            "Websuche",
            concat!(
                "Sucht im Internet und liefert bis zu 8 Treffer mit Titel, Adresse und Ausschnitt. Für aktuelle Informationen, Dokumentation und Fakten, ",
                "die du nicht sicher weisst. Danach die passenden Seiten mit web_fetch lesen. Nicht curl verwenden."
            ),
            json!({ "type": "object", "properties": { "anfrage": { "type": "string", "description": "Suchbegriffe, z. B. «Proxmox 9 Neuerungen»" } }, "required": ["anfrage"] }),
        ),
        tool(
            "web_fetch",
            "Webseite",
            &format!(
                "Holt eine Webseite und liefert ihren lesbaren Text (ohne HTML), bis {PAGE_CHARS} Zeichen pro Aufruf; mit «ab» weiterlesen. PDF-Dateien werden gespeichert und mit read_file gelesen."
            ),
            json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Vollständige Adresse mit https://" },
                    "ab": { "type": "integer", "description": "Ab diesem Zeichen weiterlesen (aus dem letzten Ergebnis)" },
                },
                "required": ["url"],
            }),
        ),
        tool(
            "chat_search",
            "Chats durchsuchen",
            "Durchsucht die früheren Chats in diesem Arbeitsordner (Eingaben, Antworten, Werkzeugaufrufe) und liefert passende Chats mit id und Fundstellen.",
            json!({ "type": "object", "properties": { "anfrage": { "type": "string", "description": "Suchwörter, alle müssen vorkommen" } }, "required": ["anfrage"] }),
        ),
        tool(
            "chat_read",
            "Chat lesen",
            "Liest einen früheren Chat aus diesem Arbeitsordner als Text (Eingaben, Antworten, Werkzeugaufrufe als Kurzzeile), bis 15000 Zeichen pro Aufruf; mit «ab» weiterlesen.",
            json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "id aus der Liste oder aus chat_search" }, "ab": { "type": "integer", "description": "Ab diesem Zeichen weiterlesen" } },
                "required": ["id"],
            }),
        ),
        tool(
            "view_file",
            "Ansehen",
            concat!(
                "Zeigt dir eine Datei so, wie sie aussieht, als Bild: Bilder, PDF, Word, Excel und PowerPoint (Seiten bzw. Folien; Standard die ersten 3, höchstens 6 pro Aufruf). ",
                "Nach dem Erstellen oder einer grösseren Änderung einmal ansehen, Fehler beheben, höchstens einmal erneut ansehen."
            ),
            json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Pfad relativ zum Arbeitsordner" },
                    "seiten": { "type": "string", "description": "Seiten oder Folien, z. B. «1-3» oder «2, 5»" },
                },
                "required": ["path"],
            }),
        ),
        tool(
            "browser",
            "Browser",
            concat!(
                "Der eingebaute Browser der App: Öffnet eine Webseite (ziel: https://…) oder eine HTML-Datei im Arbeitsordner sichtbar im Browser-Bereich rechts in der App. ",
                "Die Person sieht alles live und kann selbst klicken, tippen und sich anmelden. Ohne ziel arbeitest du auf der offenen Seite weiter, ohne sie neu zu laden. ",
                "Führt Klicks, Tasten, Eingaben und Scrollen aus und liefert Adresse, Titel, Fehler der Konsole, den sichtbaren Text, die Bedienelemente und ein Bildschirmfoto. ",
                "Damit bedienst du Webseiten, prüfst Spiele und zeigst der Person Seiten. Frag vor allem, was etwas abschickt, kauft, postet oder löscht. ",
                "Adressen auf diesem Rechner (http://localhost:…) öffnet er nur für Server, die du mit server_starten gestartet hast."
            ),
            json!({
                "type": "object",
                "properties": {
                    "ziel": { "type": "string", "description": "Adresse wie https://example.ch oder HTML-Datei relativ zum Arbeitsordner. Weglassen: auf der offenen Seite weiterarbeiten." },
                    "warten": { "type": "number", "description": "Millisekunden nach dem Laden warten (Standard 1000)" },
                    "aktionen": {
                        "type": "array",
                        "description": "Nacheinander ausführen, z. B. [{\"klick\": \"#start\"}, {\"taste\": \"ArrowRight\", \"dauer\": 500}, {\"warten\": 1000}]",
                        "items": {
                            "type": "object",
                            "properties": {
                                "klick": { "type": "string", "description": "CSS-Selektor des Elements, das angeklickt wird" },
                                "klick_text": { "type": "string", "description": "Sichtbarer Text eines Links oder Knopfs, der angeklickt wird" },
                                "x": { "type": "number", "description": "Klick auf Pixel-Position (mit y), z. B. in ein Canvas" },
                                "y": { "type": "number" },
                                "taste": { "type": "string", "description": "Taste: ArrowUp, ArrowDown, ArrowLeft, ArrowRight, Space, Enter, Escape oder ein Zeichen" },
                                "dauer": { "type": "number", "description": "So lange die Taste halten (ms)" },
                                "text": { "type": "string", "description": "Text tippen, ins Feld aus «in» oder ins fokussierte Feld. Nie Passwörter: Anmelden macht die Person selbst." },
                                "in": { "type": "string", "description": "CSS-Selektor des Eingabefelds für text" },
                                "scroll": { "type": "number", "description": "So viele Pixel nach unten scrollen (negativ: nach oben)" },
                                "warten": { "type": "number", "description": "So lange warten (ms, höchstens 5000)" },
                            },
                        },
                    },
                },
            }),
        ),
        tool(
            "server_starten",
            "Server",
            concat!(
                "Startet einen Entwicklungsserver oder ein anderes Programm, das weiterlaufen soll (z. B. npm run dev, python -m http.server 8000, node server.js), ",
                "im Arbeitsordner im Hintergrund. Wartet einige Sekunden und liefert id, Adresse (falls in der Ausgabe erkannt) und die ersten Zeilen der Ausgabe. ",
                "Der Server läuft weiter, auch nach dem Auftrag; die Person sieht ihn im Browser-Bereich. Die Adresse öffnest du mit browser."
            ),
            json!({
                "type": "object",
                "properties": {
                    "befehl": { "type": "string", "description": "Befehl für cmd.exe, z. B. npm run dev" },
                    "name": { "type": "string", "description": "Kurzer Name für die Anzeige, z. B. «Webseite»" },
                },
                "required": ["befehl"],
            }),
        ),
        tool(
            "server_stoppen",
            "Server",
            "Stoppt einen Server, den server_starten oder die Person gestartet hat.",
            json!({ "type": "object", "properties": { "id": { "type": "string", "description": "id aus server_starten oder server_liste" } }, "required": ["id"] }),
        ),
        tool(
            "server_liste",
            "Server",
            "Listet die Server dieses Projekts mit Zustand, Adresse und den letzten Zeilen der Ausgabe.",
            json!({ "type": "object", "properties": {} }),
        ),
        // Bilder mit Flux (src/main/bild.js): ASK führt Flux auf dem Server aus, die App speichert die Bilder
        // im Arbeitsordner. Nur angeboten, wenn ASK dem Konto das Werkzeug anbietet (agent.js).
        tool(
            "bild_erstellen",
            "Bild erstellen",
            concat!(
                "Erstellt oder bearbeitet Bilder mit Flux, dem Bildmodell von ASK, und speichert sie im Arbeitsordner (Standard: bilder/). ",
                "Für jedes gewünschte Bild, Foto, Zeichnung, Illustration oder Hintergrundbild. Zum Ändern eines Bildes (angehängt oder früher erstellt) ",
                "den Pfad als vorlage angeben und in beschreibung nur die Änderung; das Ergebnis ersetzt die Vorlage, ein Anhang bleibt und das Ergebnis kommt nach bilder/. ",
                "Diagramme und Grafiken aus Daten weiterhin mit Python. ",
                "Qualität, Stil und Anzahl nur angeben, wenn die Person sie nennt, das Format ebenso, ausser das Bild kommt in ein Dokument: ",
                "dann das Format passend zur Stelle wählen, z. B. breitbild für eine ganze Folie, hochformat für eine ganze A4-Seite. Dauert meist 10 bis 60 Sekunden."
            ),
            json!({
                "type": "object",
                "properties": {
                    "beschreibung": { "type": "string", "description": "Neues Bild: was es zeigen soll, in den Worten und der Sprache der Person, mit allen Details. Bearbeiten: nur die Änderung, z. B. «mach den Himmel stürmisch»." },
                    "vorlage": { "type": "string", "description": "Nur zum Bearbeiten: Pfad des Bildes im Arbeitsordner, z. B. ein Anhang oder ein früher erstelltes Bild" },
                    "datei": { "type": "string", "description": "Optional: speichern unter diesem Pfad, z. B. «plakat/titelbild.png». Sonst neue Bilder unter bilder/ mit Namen aus der Beschreibung." },
                    "format": { "type": "string", "enum": FLUX_FORMATS, "description": "quadrat (1:1), hochformat (2:3), querformat (3:2), breitbild (16:9), handy (9:16), panorama (21:9). Beim Bearbeiten leer: Form bleibt." },
                    "qualitaet": { "type": "string", "enum": FLUX_QUALITIES, "description": "schnell (Entwurf), hoch (Standard), maximal (beste Details, langsamer)" },
                    "stil": { "type": "string", "description": "Optional: foto, comic, aquarell, oel, pixel, bausteine, skizze, render. Leer, wenn die Beschreibung den Stil schon nennt." },
                    "anzahl": { "type": "integer", "description": "Verschiedene Bilder, 1 bis 4 (Standard 1)" },
                    "referenzen": { "type": "string", "description": concat!(
                        "Nur neue Bilder: echte, benannte Dinge, die das Bildmodell vielleicht nicht genau kennt (Bauwerke, Berge, Städte, bestimmte Fahrzeug- oder Tierarten). ",
                        "Höchstens zwei, getrennt mit «;», je «Titel des deutschen Wikipedia-Artikels = kurze englische Beschreibung», z. B. «Zytglogge = the Zytglogge clock tower in Bern». Nie für Personen oder Alltägliches."
                    ) },
                    "tf_branding": { "type": "boolean", "description": "true, wenn Logo oder Auftritt der TF Bern aufs Bild soll (Schule, «unser Logo», Merchandise, Plakate). Das echte Logo wird dann eingesetzt." },
                    "verwandeln": { "type": "boolean", "description": "true nur, wenn ein benanntes Ding zu etwas anderem wird (z. B. «Zytglogge aus Schokolade»)" },
                },
                "required": ["beschreibung"],
            }),
        ),
    ];
    // ASK Docs (docs.rs, ENTSCHEIDE E81)
    all.extend(crate::docs::doc_tools());
    // Gedächtnis und eigene Skills (memorytools)
    all.extend(memorytools::knowledge_tools().into_iter().map(|k| Tool {
        name: k.name.into(),
        label: k.label.into(),
        description: k.description.into(),
        parameters: k.parameters,
    }));
    all
}

fn find_tool(name: &str) -> Option<&'static Tool> {
    tools().iter().find(|t| t.name == name)
}

/// Werkzeugbeschreibungen im OpenAI-Format; die Befehlsbeschreibung kommt aus resources/anweisungen.
pub fn tool_specs(command_hint: &str) -> Vec<Value> {
    tools()
        .iter()
        .map(|t| {
            let description = if t.name == "run_command" { format!("{}{command_hint}", t.description) } else { t.description.clone() };
            json!({ "type": "function", "function": { "name": t.name, "description": description, "parameters": t.parameters } })
        })
        .collect()
}

/// Argumente eines Aufrufs: ein Objekt bleibt, Text wird als JSON gelesen (leer ergibt `{}`)
pub fn parse_args(raw: &Value) -> Result<Value> {
    if raw.is_object() || raw.is_array() {
        return Ok(raw.clone());
    }
    let text = if truthy(raw) { js_string(raw) } else { String::new() };
    let text = text.trim();
    if text.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(text).map_err(|_| anyhow!("Die Argumente sind kein gültiges JSON: {}", take_chars(text, 200)))
}

/// Titel und Kurztext eines Aufrufs für die Oberfläche
#[derive(Clone, Debug, PartialEq)]
pub struct Described {
    pub label: String,
    pub detail: String,
}

fn describe_detail(name: &str, a: &Value) -> String {
    let path_start = || format!("{}{}", tpl(a, "path"), if a.get("start_line").is_some_and(truthy) { format!(" (ab Zeile {})", js_string(&a["start_line"])) } else { String::new() });
    match name {
        "run_command" => or_empty(a, "command"),
        "command_status" => format!("Status {}", tpl(a, "id")),
        "read_file" => path_start(),
        "write_file" | "edit_file" => or_empty(a, "path"),
        "list_files" => Some(or_empty(a, "path")).filter(|p| !p.is_empty()).unwrap_or_else(|| ".".into()),
        "find_files" => or_empty(a, "pattern"),
        "search_text" => format!("«{}»{}", tpl(a, "text"), if a.get("path").is_some_and(truthy) { format!(" in {}", js_string(&a["path"])) } else { String::new() }),
        "quellen_suchen" => format!("«{}»", tpl(a, "suche")),
        "quelle_lesen" => format!("{}{}", tpl(a, "datei"), match (a.get("seite").filter(|v| truthy(v)), a.get("ort").filter(|v| truthy(v))) {
            (Some(n), _) => format!(", S. {}", js_string(n)),
            (None, Some(o)) => format!(", {}", js_string(o)),
            _ => String::new(),
        }),
        "update_tasks" => or_empty(a, "auftrag"),
        "github_status" => "Status".into(),
        "github_clone" => format!("holen {}", tpl(a, "repo")),
        "github_pull" => "holen".into(),
        "github_push" => format!("hochladen «{}»", or_empty(a, "nachricht")),
        "github_create" => format!("neu {}{}", tpl(a, "name"), if a.get("privat") == Some(&json!(false)) { " (öffentlich)" } else { " (privat)" }),
        "github_branch" => match a.get("aktion").and_then(Value::as_str) {
            Some("wechseln") => format!("Branch wechseln zu {}", or_empty(a, "name")),
            Some("neu") => format!("neuer Branch {}", or_empty(a, "name")),
            _ => "Branches".into(),
        },
        "github_pr" => {
            if a.get("aktion") == Some(&json!("neu")) {
                format!("Pull Request «{}»", or_empty(a, "titel"))
            } else {
                "Pull Requests".into()
            }
        }
        "web_search" | "chat_search" => or_empty(a, "anfrage"),
        "web_fetch" => or_empty(a, "url"),
        "chat_read" => or_empty(a, "id"),
        "view_file" => format!("{}{}", tpl(a, "path"), if a.get("seiten").is_some_and(truthy) { format!(", Seiten {}", js_string(&a["seiten"])) } else { String::new() }),
        "browser" => {
            let ziel = Some(or_empty(a, "ziel")).filter(|z| !z.is_empty()).unwrap_or_else(|| "offene Seite".into());
            let n = a.get("aktionen").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            format!("{ziel}{}", if n > 0 { format!(", {n} Aktionen") } else { String::new() })
        }
        "server_starten" => or_empty(a, "befehl"),
        "server_stoppen" => format!("stoppen {}", tpl(a, "id")),
        "server_liste" => "Liste".into(),
        "bild_erstellen" => format!("{}{}", if a.get("vorlage").is_some_and(truthy) { format!("{}: ", js_string(&a["vorlage"])) } else { String::new() }, or_empty(a, "beschreibung")),
        other if crate::docs::is_doc_tool(other) => crate::docs::describe(other, a),
        other => memorytools::describe(other, a),
    }
}

/// Titel und Kurztext eines Werkzeugaufrufs (Kurztext auf eine Zeile und 200 Zeichen gekürzt)
pub fn describe(name: &str, args: &Value) -> Described {
    if name == "goal_done" {
        return Described { label: "Ziel erreicht".into(), detail: String::new() }; // Werkzeug der Arbeitsschleife (agent.js)
    }
    if name == "denkaufwand" {
        let level = args.get("stufe").map(js_string).unwrap_or_default();
        let reason = args.get("grund").map(js_string).unwrap_or_default();
        let detail = if reason.is_empty() { level } else { format!("{level}: {reason}") };
        return Described { label: "Denkaufwand".into(), detail: take_chars(&detail, 200) }; // Werkzeug der Arbeitsschleife (ENTSCHEIDE E62)
    }
    let tool = find_tool(name);
    let detail = if tool.is_some() { describe_detail(name, args) } else { String::new() };
    Described {
        label: tool.map(|t| t.label.clone()).unwrap_or_else(|| name.to_string()),
        detail: take_chars(&re!(r"\s+").replace_all(&detail, " "), 200),
    }
}

/// Argumente eines Werkzeugaufrufs: «null», eine Zahl oder eine Liste werden zu `{}`, damit kein Schritt daran
/// scheitert. Unlesbares JSON bleibt, wie es ist: `run_tool` meldet dem Modell den Fehler.
pub fn clean_args(raw: &Value) -> String {
    let text = match raw {
        Value::String(s) => s.clone(),
        Value::Object(_) | Value::Array(_) => raw.to_string(),
        _ => String::new(),
    };
    if text.trim().is_empty() {
        return "{}".to_string();
    }
    match serde_json::from_str::<Value>(&text) {
        Ok(v) if v.is_object() => text,
        Ok(_) => "{}".to_string(),
        Err(_) => text,
    }
}

async fn run(terminal: Option<&dyn TerminalEndpoint>, name: &str, args: &Value, ctx: &ToolCtx) -> Result<ToolResult> {
    let term = || terminal.ok_or_else(|| anyhow!("Das Terminal läuft nicht. Bitte in der Statusleiste neu starten."));
    match name {
        "run_command" => run_command(term()?, args, ctx).await,
        "command_status" => command_status(term()?, args).await,
        "read_file" => read_file(term()?, args).await,
        "write_file" => write_file(term()?, args).await,
        "edit_file" => edit_file(term()?, args).await,
        "list_files" => list_files(term()?, args).await,
        "find_files" => find_files(term()?, args).await,
        "search_text" => search_text(term()?, args).await,
        "quellen_suchen" => quellen_suchen(term()?, args).await,
        "quelle_lesen" => quelle_lesen(term()?, args).await,
        "update_tasks" => Ok(update_tasks(args)),
        "github_status" => github_status(ctx).await,
        "github_clone" => github_clone(ctx, args).await,
        "github_pull" => github_pull(ctx).await,
        "github_push" => github_push(ctx, args).await,
        "github_create" => github_create(ctx, args).await,
        "github_branch" => github_branch(ctx, args).await,
        "github_pr" => github_pr(ctx, args).await,
        "web_search" => web_search(ctx, args).await,
        "web_fetch" => web_fetch(ctx, args).await,
        "chat_search" => chat_search(ctx, args),
        "chat_read" => chat_read(ctx, args),
        "view_file" => view_file(ctx, args).await,
        "browser" => browser(ctx, args).await,
        "server_starten" => server_starten(ctx, args).await,
        "server_stoppen" => server_stoppen(ctx, args).await,
        "server_liste" => server_liste(ctx),
        "bild_erstellen" => bild_erstellen(ctx, args).await,
        "mod_testen" => mod_testen(ctx, args).await,
        other if crate::docs::is_doc_tool(other) => crate::docs::run(other, args, ctx).await.unwrap_or_else(|| bail!("Unbekanntes Werkzeug: {other}")),
        other => {
            let mctx = MemoryToolCtx {
                folder: ctx.folder.clone(),
                chat: ctx.chat.clone(),
                chat_title: ctx.chat_title.clone(),
                knowledge: ctx.knowledge.clone(),
                confirm: ctx.confirm.clone(),
            };
            match memorytools::run(other, args, &mctx).await {
                Some(r) => r.map(ToolResult::from),
                None => bail!("Unbekanntes Werkzeug: {other}"),
            }
        }
    }
}

/// Führt einen Werkzeugaufruf aus. Fehler gehen als Text an das Modell zurück.
pub async fn run_tool(terminal: Option<&dyn TerminalEndpoint>, name: &str, raw_args: &Value, ctx: &ToolCtx) -> ToolResult {
    if find_tool(name).is_none() {
        return ToolResult::new(
            false,
            format!("Unbekanntes Werkzeug: {name}. Verfügbar: {}", tools().iter().map(|t| t.name.as_str()).collect::<Vec<_>>().join(", ")),
            "unbekannt",
        );
    }
    let result = match parse_args(raw_args) {
        Ok(args) => run(terminal, name, &args, ctx).await,
        Err(e) => Err(e),
    };
    match result {
        Ok(r) => r,
        Err(err) => {
            if ctx.signal.as_ref().is_some_and(Signal::is_cancelled) {
                return ToolResult::new(false, "Von der Person abgebrochen.", "abgebrochen");
            }
            let message = err.to_string();
            ToolResult::new(false, format!("Fehler: {message}"), take_chars(&message, 200))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::askapi::test_http::{serve, Resp};
    use crate::servers::ServersOptions;
    use std::collections::HashMap;

    #[test]
    fn gesammelte_ausgabe_bleibt_begrenzt_anfang_und_ende_bleiben() {
        let mut out = String::from("ANFANG\n");
        for i in 0..40 {
            out.push_str(&"ä".repeat(10_000)); // Mehrbyte-Zeichen: Schnitt nie mitten in einem Zeichen
            out.push_str(&format!("\nZeile {i}\n"));
            trim_live(&mut out);
            assert!(out.len() <= LIVE_OUTPUT_MAX + 100, "nie viel grösser als die Grenze: {}", out.len());
        }
        assert!(out.starts_with("ANFANG\n"));
        assert!(out.ends_with("Zeile 39\n"));
        assert!(out.contains("(Ausgabe gekürzt)"));
        let mut short = String::from("kurz");
        trim_live(&mut short);
        assert_eq!(short, "kurz");
    }

    // ---- tests/tools.test.js ----

    #[test]
    fn programme_mit_fenster_werden_abgelehnt() {
        for command in [
            "start ausgabe\\bericht.docx",
            "winword ausgabe\\bericht.docx",
            "\"C:\\Program Files\\Microsoft Office\\root\\Office16\\WINWORD.EXE\" /q bericht.docx",
            "dir & start .",
            "excel.exe a.xlsx",
            "python a.py && POWERPNT vortrag.pptx",
            "explorer ausgabe",
            "cmd /c start notepad",
            "cmd.exe /d /c \"start bericht.docx\"",
            "call start bericht.docx",
            "@start bericht.docx",
            "if exist a.docx (start a.docx)",
        ] {
            assert!(!blocked_command(command).is_empty(), "{command}");
        }
    }

    #[test]
    fn normale_befehle_laufen() {
        for command in [
            "python .ask/skripte/abc/excel.py",
            "python .ask/skills/pruefen.py ausgabe/Auswertung.xlsx --pdf",
            "echo start",
            "tasklist | findstr WINWORD",
            "python restart.py",
            "dir C:\\Windows\\explorer.exe",
            "type ausgabe\\notepad.txt",
            "python -c \"print(start)\"",
            "git commit -m \"fix (start) of loop\"",
            "python -c \"x = 1 | 2; print(excel)\"",
            "cmd /c dir",
        ] {
            assert_eq!(blocked_command(command), "", "{command}");
        }
    }

    // ---- tests/askapi.test.js «Werkzeuge: Argumente, Kürzen, Beschreibungen» ----

    #[test]
    fn werkzeuge_argumente_kuerzen_beschreibungen() {
        assert_eq!(parse_args(&json!("{\"a\":1}")).unwrap(), json!({ "a": 1 }));
        assert_eq!(parse_args(&json!("")).unwrap(), json!({}));
        let err = parse_args(&json!("{kaputt")).unwrap_err().to_string();
        assert!(err.contains("kein gültiges JSON"), "{err}");
        let c = clip(&format!("{}{}", "a".repeat(100), "b".repeat(100)), 50);
        assert!(c.starts_with(&"a".repeat(15)) && c.ends_with(&"b".repeat(35)) && c.contains("150 Zeichen ausgelassen"), "{c}");
        let specs = tool_specs(" Hinweis");
        let run = specs.iter().find(|s| s["function"]["name"] == "run_command").unwrap();
        assert!(run["function"]["description"].as_str().unwrap().ends_with(" Hinweis"));
        assert!(specs.iter().all(|s| s["type"] == "function" && s["function"]["parameters"]["type"] == "object"));
    }

    #[test]
    fn argumente_bereinigen_und_beschreiben() {
        assert_eq!(clean_args(&json!("null")), "{}");
        assert_eq!(clean_args(&json!("[1]")), "{}");
        assert_eq!(clean_args(&json!("  ")), "{}");
        assert_eq!(clean_args(&Value::Null), "{}");
        assert_eq!(clean_args(&json!("{kaputt")), "{kaputt");
        assert_eq!(clean_args(&json!({ "a": 1 })), "{\"a\":1}");
        assert_eq!(describe("read_file", &json!({ "path": "a.txt", "start_line": 5 })), Described { label: "Lesen".into(), detail: "a.txt (ab Zeile 5)".into() });
        assert_eq!(describe("goal_done", &json!({})).label, "Ziel erreicht");
        assert_eq!(describe("gibt_es_nicht", &json!({})), Described { label: "gibt_es_nicht".into(), detail: String::new() });
        assert_eq!(describe("run_command", &json!({ "command": "dir\n  /b" })).detail, "dir /b");
        assert_eq!(describe("merken", &json!({ "text": "Etwas" })).detail, "Etwas");
        assert_eq!(describe("github_branch", &json!({ "aktion": "wechseln", "name": "x" })).detail, "Branch wechseln zu x");
    }

    // ---- Terminal-Ersatz über HTTP ----

    struct Fake(u16);
    impl TerminalEndpoint for Fake {
        fn state(&self) -> String {
            "läuft".into()
        }
        fn port(&self) -> u16 {
            self.0
        }
        fn key(&self) -> String {
            "geheim".into()
        }
    }

    #[tokio::test]
    async fn terminal_gestoppt_befehle_und_dateien_ueber_http() {
        // Gestoppt: freundliche Meldung statt Verbindungsfehler
        let stopped = EndpointSnapshot { state: "gestoppt".into(), port: 0, key: String::new() };
        let r = run_tool(Some(&stopped), "list_files", &json!({}), &ToolCtx::default()).await;
        assert!(!r.ok && r.output.contains("Das Terminal läuft nicht"), "{}", r.output);
        let r = run_tool(None, "read_file", &json!({ "path": "a" }), &ToolCtx::default()).await;
        assert!(!r.ok && r.output.starts_with("Fehler: Das Terminal läuft nicht"), "{}", r.output);

        let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let p2 = polls.clone();
        let server = serve(move |req| {
            assert_eq!(req.headers.get("authorization").map(String::as_str), Some("Bearer geheim"));
            let path = req.path.as_str();
            if req.method == "POST" && path.starts_with("/execute?") {
                return Resp::json(200, &json!({ "id": "p1", "status": "running", "exit_code": null, "output": [{ "type": "stdout", "data": "a\r\n" }], "next_offset": 1 }));
            }
            if path.starts_with("/execute/p1/status?") {
                assert!(path.contains("offset=1"), "{path}");
                p2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                return Resp::json(200, &json!({ "id": "p1", "status": "exited", "exit_code": 0, "output": [{ "type": "stdout", "data": "b" }], "next_offset": 2 }));
            }
            if path.starts_with("/files/read?path=a.txt&start_line=2") {
                return Resp::json(200, &json!({ "content": "zwei\ndrei", "total_lines": 3 }));
            }
            if path.starts_with("/files/read?") {
                return Resp::json(404, &json!({ "detail": "Datei nicht gefunden" }));
            }
            if path.starts_with("/files/list?directory=.") {
                return Resp::json(200, &json!({ "entries": [{ "name": "a.txt", "type": "file", "size": 5 }, { "name": "ordner", "type": "directory" }] }));
            }
            Resp::json(500, &json!({}))
        })
        .await;
        let port: u16 = server.url.rsplit(':').next().unwrap().parse().unwrap();
        let t = Fake(port);
        let progress: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let pr = progress.clone();
        let ctx = ToolCtx { on_progress: Some(Arc::new(move |t| pr.lock().push(t.to_string()))), ..Default::default() };
        let r = run_tool(Some(&t), "run_command", &json!({ "command": "echo a" }), &ctx).await;
        assert!(r.ok, "{}", r.output);
        assert_eq!(r.output, "Exitcode 0\na\nb");
        assert_eq!(r.summary, "");
        assert_eq!(progress.lock().as_slice(), ["a\n"]);
        let r = run_tool(Some(&t), "run_command", &json!({ "command": "start x.docx" }), &ctx).await;
        assert_eq!((r.ok, r.summary.as_str()), (false, "abgelehnt"));
        let r = run_tool(Some(&t), "read_file", &json!({ "path": "a.txt", "start_line": 2 }), &ctx).await;
        assert_eq!(r.output, "zwei\ndrei\n\n(3 Zeilen insgesamt)");
        assert_eq!(r.summary, "2 Zeilen");
        let r = run_tool(Some(&t), "read_file", &json!({ "path": "fehlt" }), &ctx).await;
        assert_eq!((r.ok, r.output.as_str()), (false, "Fehler: Datei nicht gefunden"));
        let r = run_tool(Some(&t), "list_files", &json!({}), &ctx).await;
        assert_eq!(r.output, "a.txt (5 B)\nordner/");
        assert_eq!(r.summary, "2 Einträge");
        // Abbruch durch die Person: gemeldet statt als Fehler
        let signal = Signal::new();
        signal.cancel();
        let ctx = ToolCtx { signal: Some(signal), ..Default::default() };
        let r = run_tool(Some(&t), "run_command", &json!({ "command": "echo a" }), &ctx).await;
        assert_eq!((r.ok, r.summary.as_str()), (false, "abgebrochen"));
        let r = run_tool(Some(&t), "gibt_es_nicht", &json!({}), &ToolCtx::default()).await;
        assert!(r.output.starts_with("Unbekanntes Werkzeug: gibt_es_nicht. Verfügbar: run_command, command_status"), "{}", r.output);
        let r = run_tool(Some(&t), "list_files", &json!("{kaputt"), &ToolCtx::default()).await;
        assert!(r.output.starts_with("Fehler: Die Argumente sind kein gültiges JSON: {kaputt"), "{}", r.output);
    }

    #[tokio::test]
    async fn aufgabenliste_wird_bereinigt() {
        let r = run_tool(
            None,
            "update_tasks",
            &json!({ "auftrag": "x", "aufgaben": [{ "titel": " A ", "status": "erledigt" }, { "titel": "", "status": "offen" }, { "titel": "B", "status": "komisch" }] }),
            &ToolCtx::default(),
        )
        .await;
        assert!(r.ok);
        assert_eq!(r.tasks.unwrap(), vec![json!({ "titel": "A", "status": "erledigt" }), json!({ "titel": "B", "status": "offen" })]);
        assert_eq!(r.summary, "1 von 2 erledigt");
    }

    // ---- tests/github.test.js «Werkzeuge github_branch und github_pr» ----

    #[tokio::test]
    async fn werkzeuge_github_branch_und_github_pr() {
        use crate::github::tests::{folder, read, setup, write, TOKEN};
        let (_mock, gh, _task) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let gh = Arc::new(gh);
        let dir = folder(&[("index.html", "eins")]);
        let a = dir.path();
        let asked: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let (asked2, answer2) = (asked.clone(), answer.clone());
        let confirm: ConfirmFn = Arc::new(move |m, d, _o| {
            asked2.lock().push((m, d));
            let ok = answer2.load(std::sync::atomic::Ordering::SeqCst);
            Box::pin(async move { Confirmed { ok, checked: false } })
        });
        let ctx = ToolCtx { github: Some(gh.clone()), folder: a.to_string_lossy().to_string(), confirm: Some(confirm), ..Default::default() };
        let run = |name: &'static str, args: Value, ctx: ToolCtx| async move { run_tool(None, name, &args, &ctx).await };
        gh.create(a, "werkzeug", true, "").await.unwrap();
        let mut r = run("github_branch", json!({ "aktion": "liste" }), ctx.clone()).await;
        assert!(r.ok && r.output.contains("main (Standard) (verbunden)"), "{}", r.output);
        r = run("github_branch", json!({ "aktion": "neu" }), ctx.clone()).await;
        assert!(!r.ok && r.output.contains("Namen des Branches"));
        r = run("github_branch", json!({ "aktion": "neu", "name": "aenderung" }), ctx.clone()).await;
        assert!(r.ok && r.output.contains("aenderung angelegt"), "{}", r.output);

        // PR mit Änderungen, die noch nicht hochgeladen sind: zuerst hochladen
        write(a, "index.html", "zwei");
        r = run("github_pr", json!({ "aktion": "neu", "titel": "Änderung" }), ctx.clone()).await;
        assert!(!r.ok && r.output.contains("zuerst github_push"), "{}", r.output);
        // Wechseln mit Änderungen: abgelehnt, das Modell soll fragen
        r = run("github_branch", json!({ "aktion": "wechseln", "name": "main" }), ctx.clone()).await;
        assert!(!r.ok && r.output.contains("noch nicht hochgeladen") && r.output.contains("Frag die Person"), "{}", r.output);
        gh.push(a, "zwei", false).await.unwrap();

        // Abgelehnt: kein PR
        answer.store(false, std::sync::atomic::Ordering::SeqCst);
        r = run("github_pr", json!({ "aktion": "neu", "titel": "Änderung", "beschreibung": "Text geändert" }), ctx.clone()).await;
        assert!(!r.ok && r.output.contains("abgelehnt"));
        assert_eq!(asked.lock().len(), 1);
        assert!(asked.lock()[0].0.contains("Pull Request in lea-test/werkzeug erstellen"), "{:?}", asked.lock());
        assert!(asked.lock()[0].1.contains("Von aenderung nach main"));
        assert_eq!(gh.pull_requests(a).await.unwrap(), json!([]));
        answer.store(true, std::sync::atomic::Ordering::SeqCst);
        r = run("github_pr", json!({ "aktion": "neu", "titel": "Änderung", "beschreibung": "Text geändert" }), ctx.clone()).await;
        assert!(r.ok && r.output.contains("#1") && r.output.contains("pull/1"), "{}", r.output);
        r = run("github_pr", json!({ "aktion": "liste" }), ctx.clone()).await;
        assert!(r.ok && r.output.contains("#1 Änderung (aenderung → main"), "{}", r.output);

        // Zurück auf main, dort kein PR auf sich selbst
        r = run("github_branch", json!({ "aktion": "wechseln", "name": "main" }), ctx.clone()).await;
        assert!(r.ok && r.output.contains("Gewechselt von aenderung zu main"), "{}", r.output);
        assert_eq!(read(a, "index.html"), "eins");
        r = run("github_pr", json!({ "aktion": "neu", "titel": "x" }), ctx.clone()).await;
        assert!(!r.ok && r.output.contains("eigenen Branch"), "{}", r.output);

        // Während die Person im Bereich «GitHub» abgleicht, wartet das Modell
        let ctx2 = ctx.clone();
        gh.exclusive(a, || async move {
            let busy = run_tool(None, "github_pull", &json!({}), &ctx2).await;
            assert!(!busy.ok && busy.output.contains("gleicht gerade selbst"), "{}", busy.output);
            let panel = run_tool(None, "github_status", &json!({}), &ToolCtx { panel: true, ..ctx2.clone() }).await;
            assert!(panel.ok, "der Bereich selbst darf");
            Ok(())
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn github_push_fragt_einmal_und_merkt_sich_die_antwort() {
        use crate::github::tests::{folder, setup, write, TOKEN};
        let (_mock, gh, _task) = setup().await;
        gh.connect(TOKEN).await.unwrap();
        let gh = Arc::new(gh);
        let dir = folder(&[("a.txt", "eins")]);
        let a = dir.path();
        gh.create(a, "vertrauen", true, "").await.unwrap();
        let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (n, reply) = (asked.clone(), Arc::new(Mutex::new(Confirmed { ok: true, checked: true })));
        let r2 = reply.clone();
        let confirm: ConfirmFn = Arc::new(move |_m, _d, o| {
            n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            assert_eq!(o.checkbox.as_deref(), Some("Für dieses Repository nicht mehr fragen"));
            let c = *r2.lock();
            Box::pin(async move { c })
        });
        let trust = Arc::new(GithubTrust::new());
        let ctx = ToolCtx { github: Some(gh.clone()), github_trust: Some(trust.clone()), folder: a.to_string_lossy().to_string(), confirm: Some(confirm), ..Default::default() };
        write(a, "a.txt", "zwei");
        let r = run_tool(None, "github_push", &json!({ "nachricht": "zwei" }), &ctx).await;
        assert!(r.ok && r.output.starts_with("Hochgeladen: 1 Dateien, 0 gelöscht."), "{}", r.output);
        assert!(trust.has("lea-test/vertrauen"));
        write(a, "a.txt", "drei");
        let r = run_tool(None, "github_push", &json!({ "nachricht": "drei" }), &ctx).await;
        assert!(r.ok, "{}", r.output);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1, "beim zweiten Mal nicht mehr gefragt");
        // Ohne Verbindung und ohne Ordner
        let r = run_tool(None, "github_status", &json!({}), &ToolCtx::default()).await;
        assert_eq!(r.output, "Fehler: GitHub ist hier nicht verfügbar.");
    }

    // ---- tests/servers.test.js «Werkzeuge server_starten, server_liste, server_stoppen» ----

    /// Ausgabe, Zustand und Exitcode eines Prozesses
    type FakeProc = (Vec<Value>, String, Value);

    #[derive(Default)]
    struct FakeTerminal {
        procs: Mutex<HashMap<String, FakeProc>>,
        next: Mutex<u32>,
    }

    impl FakeTerminal {
        fn handle(&self, method: &str, path: &str) -> TerminalResponse {
            let ok = |j: Value| TerminalResponse { status: 200, json: Some(j), text: String::new() };
            if method == "POST" && path.starts_with("/execute") {
                let id = {
                    let mut n = self.next.lock();
                    *n += 1;
                    format!("p{n}")
                };
                // Vite meldet seine Adresse gleich nach dem Start
                let out = vec![json!({ "type": "stdout", "data": "  ➜  Local:   http://localhost:5173/\n" })];
                self.procs.lock().insert(id.clone(), (out, "running".into(), Value::Null));
                return ok(json!({ "id": id, "status": "running", "exit_code": null, "output": [], "next_offset": 0 }));
            }
            let m = re!(r"^/execute/([^/?]+)(/status\?wait=0&offset=([0-9]+))?$").captures(path);
            let mut procs = self.procs.lock();
            let Some((m, p)) = m.and_then(|m| procs.get_mut(&m[1]).map(|p| (m, p))) else {
                return TerminalResponse { status: 404, json: Some(json!({ "detail": "Process not found" })), text: String::new() };
            };
            if method == "DELETE" {
                p.1 = "killed".into();
                p.2 = json!(-1);
                return ok(json!({ "status": "killed" }));
            }
            let offset: usize = m.get(3).and_then(|o| o.as_str().parse().ok()).unwrap_or(0);
            ok(json!({ "id": &m[1], "status": p.1, "exit_code": p.2, "output": p.0[offset.min(p.0.len())..], "next_offset": p.0.len() }))
        }
    }

    #[tokio::test]
    async fn werkzeuge_server_starten_server_liste_server_stoppen() {
        let names: Vec<String> = tool_specs("").iter().map(|s| s["function"]["name"].as_str().unwrap().to_string()).collect();
        for n in ["server_starten", "server_stoppen", "server_liste"] {
            assert!(names.iter().any(|x| x == n), "{n}");
        }
        let t = Arc::new(FakeTerminal::default());
        let t2 = t.clone();
        let request: RequestFn = Arc::new(move |m, p, _b| {
            let r = t2.handle(&m, &p);
            Box::pin(async move { Ok(r) })
        });
        let mut o = ServersOptions::new(request);
        o.probe = Arc::new(|_| Box::pin(async { true }));
        o.poll_ms = 10;
        let servers = Arc::new(Servers::new(o));
        let ctx = ToolCtx { servers: Some(servers.clone()), ..Default::default() };
        let r = run_tool(None, "server_starten", &json!({ "befehl": "npm run dev", "name": "Webseite" }), &ctx).await;
        assert!(r.ok, "{}", r.output);
        assert!(r.output.contains("p1 «Webseite»: läuft, http://localhost:5173/"), "{}", r.output);
        assert!(r.output.contains("browser (ziel: http://localhost:5173/)"), "{}", r.output);
        let list = run_tool(None, "server_liste", &json!({}), &ctx).await;
        assert!(Regex::new(r"Local:\s+http://localhost:5173").unwrap().is_match(&list.output), "{}", list.output);
        assert_eq!(list.summary, "1 läuft");
        let blocked = run_tool(None, "server_starten", &json!({ "befehl": "start index.html" }), &ctx).await;
        assert_eq!(blocked.summary, "abgelehnt");
        let stopped = run_tool(None, "server_stoppen", &json!({ "id": "p1" }), &ctx).await;
        assert!(stopped.ok);
        assert_eq!(servers.get("p1").unwrap().state, "gestoppt");
        let none = run_tool(None, "server_starten", &json!({ "befehl": "npm run dev" }), &ToolCtx::default()).await;
        assert!(none.output.contains("nicht verfügbar"), "{}", none.output);
    }

    // ---- tests/history.test.js «Suchen und lesen; nur mit Schalter» (die Werkzeuge) ----

    #[tokio::test]
    async fn chat_search_und_chat_read_nur_mit_schalter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ask/chats")).unwrap();
        let chat = |id: &str, title: &str, text: &str| {
            json!({ "id": id, "title": title, "updated": 1_700_000_000_000_i64, "messages": [
                { "role": "user", "content": format!("Wie sichere ich {text}?") },
                { "role": "assistant", "content": "Backups laufen täglich um zwei Uhr." },
            ] })
        };
        std::fs::write(dir.path().join(".ask/chats/alt.json"), chat("alt", "Bericht Proxmox", "Proxmox Backups").to_string()).unwrap();
        std::fs::write(dir.path().join(".ask/chats/jetzt.json"), chat("jetzt", "Laufender Chat", "Proxmox Backups").to_string()).unwrap();
        let folder = dir.path().to_string_lossy().to_string();
        let off = run_tool(None, "chat_search", &json!({ "anfrage": "proxmox" }), &ToolCtx { folder: folder.clone(), chat_memory: false, ..Default::default() }).await;
        assert!(!off.ok && off.output.contains("nicht freigegeben"), "{}", off.output);
        let on = run_tool(None, "chat_search", &json!({ "anfrage": "proxmox" }), &ToolCtx { folder: folder.clone(), chat_memory: true, chat: "jetzt".into(), ..Default::default() }).await;
        assert!(on.ok && on.output.contains("«Bericht Proxmox» (id alt"), "{}", on.output);
        assert!(!on.output.contains("Laufender Chat"), "laufender Chat nicht");
        let read = run_tool(None, "chat_read", &json!({ "id": "alt" }), &ToolCtx { folder, chat_memory: true, ..Default::default() }).await;
        assert!(read.ok && read.output.starts_with("Chat «Bericht Proxmox» vom"), "{}", read.output);
    }

    // ---- tests/memory.test.js und tests/skills.test.js über run_tool ----

    #[tokio::test]
    async fn werkzeuge_merken_vergessen_skills_ueber_run_tool() {
        use crate::memory::Memory;
        use crate::skills::Skills;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let proj = std::env::temp_dir().join("ASK-Projekte").join("Budget").to_string_lossy().to_string();
        let memory = Arc::new(Memory::new(dir.path().join("gedaechtnis.json")));
        let on = Arc::new(AtomicBool::new(true));
        let changed = Arc::new(AtomicUsize::new(0));
        let (on2, ch2) = (on.clone(), changed.clone());
        let skills = Arc::new(Skills::new(dir.path()));
        let asked: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(AtomicBool::new(false));
        let (asked2, answer2) = (asked.clone(), answer.clone());
        let confirm: ConfirmFn = Arc::new(move |m, d, _o| {
            asked2.lock().push((m, d));
            let ok = answer2.load(Ordering::SeqCst);
            Box::pin(async move { Confirmed { ok, checked: false } })
        });
        let ctx = ToolCtx {
            folder: proj.clone(),
            chat: "c1".into(),
            chat_title: "Test".into(),
            knowledge: Some(KnowledgeCtx {
                memory: Some(memory.clone()),
                skills: Some(skills.clone()),
                mods: None,
                catalog: None,
                catalog_sources: None,
                mods_changed: None,
                memory_on: Some(Arc::new(move || on2.load(Ordering::SeqCst))),
                changed: Some(Arc::new(move || {
                    ch2.fetch_add(1, Ordering::SeqCst);
                })),
            }),
            confirm: Some(confirm),
            ..Default::default()
        };
        let run = |name: &'static str, args: Value| {
            let ctx = ctx.clone();
            async move { run_tool(None, name, &Value::String(args.to_string()), &ctx).await }
        };
        let mut r = run("merken", json!({ "text": "Mag Tabellen statt Fliesstext.", "bereich": "konto", "tags": ["stil"] })).await;
        assert!(r.ok, "{}", r.output);
        assert!(Regex::new(r"Gemerkt \(id m[0-9a-f]+, alle Projekte\)").unwrap().is_match(&r.output), "{}", r.output);
        run("merken", json!({ "text": "Excel-Dateien liegen in daten/.", "bereich": "projekt" })).await;
        assert_eq!(memory.for_project(&proj).len(), 1);
        r = run("merken", json!({ "text": "Mein Passwort ist geheim123", "bereich": "konto" })).await;
        assert!(!r.ok);
        r = run("vergessen", json!({ "text": "tabellen statt" })).await;
        assert!(r.ok, "{}", r.output);
        r = run("vergessen", json!({ "id": "m00000000" })).await;
        assert!(!r.ok);
        assert_eq!(changed.load(Ordering::SeqCst), 3);
        on.store(false, Ordering::SeqCst);
        r = run("merken", json!({ "text": "Noch etwas Neues.", "bereich": "konto" })).await;
        assert!(!r.ok && r.output.contains("ausgeschaltet"), "{}", r.output);
        assert!(tool_specs("").iter().any(|s| s["function"]["name"] == "merken"));

        // Skills (skills.test.js)
        r = run("skill_lesen", json!({ "name": "bericht" })).await;
        assert!(!r.ok && r.output.contains("noch keine"), "{}", r.output);
        let args = json!({ "name": "Mein Bericht", "beschreibung": "Bericht nach Vorlage", "anleitung": "1. Lesen\n2. Schreiben" });
        r = run("skill_erstellen", args.clone()).await;
        assert!(!r.ok, "abgelehnt");
        assert!(asked.lock()[0].0.contains("Skill «mein-bericht» speichern?"), "{:?}", asked.lock());
        assert!(skills.list().is_empty());
        answer.store(true, Ordering::SeqCst);
        r = run("skill_erstellen", args.clone()).await;
        assert!(r.ok && r.output.contains("/mein-bericht"), "{}", r.output);
        run("skill_erstellen", args).await;
        assert!(asked.lock()[2].0.contains("ersetzen?"), "{:?}", asked.lock());
        r = run("skill_lesen", json!({ "name": "Mein Bericht" })).await;
        assert!(r.ok && r.output.contains("Bericht nach Vorlage\n\n1. Lesen\n2. Schreiben"), "{}", r.output);
        r = run("skill_erstellen", json!({ "name": "github", "beschreibung": "x", "anleitung": "y" })).await;
        assert!(!r.ok && r.output.contains("Befehl der App"), "{}", r.output);
        let long = format!("Anfang\n{}\nENDE", "x".repeat(2000));
        r = run("skill_erstellen", json!({ "name": "lang", "beschreibung": "x", "anleitung": long })).await;
        assert!(r.ok, "{}", r.output);
        assert!(asked.lock().last().unwrap().1.contains("ENDE"), "ganze Anleitung in der Rückfrage");
        let count = asked.lock().len();
        r = run("skill_erstellen", json!({ "name": "zulang", "beschreibung": "x", "anleitung": "y".repeat(3001) })).await;
        assert!(!r.ok);
        assert_eq!(asked.lock().len(), count, "gar nicht erst gefragt");
    }

    // ---- ctx.visual (Ersatz für visual.rs) ----

    struct FakeVisual {
        seen: Mutex<Vec<(String, String, VisualOpts)>>,
    }

    #[async_trait]
    impl VisualTools for FakeVisual {
        async fn view(&self, folder: &str, path: &str, opts: VisualOpts) -> Result<Value> {
            self.seen.lock().push((folder.into(), path.into(), opts));
            Ok(json!({ "ok": true, "output": "Bild 2 × 2 Pixel.", "summary": "2 × 2 Pixel", "images": ["a.png"] }))
        }
        async fn browse(&self, folder: &str, ziel: &str, opts: VisualOpts) -> Result<Value> {
            self.seen.lock().push((folder.into(), ziel.into(), opts));
            Ok(json!({ "ok": true, "output": format!("Seite geladen. {SHOT_LATER}"), "summary": "ok", "images": ["s.png"] }))
        }
    }

    #[tokio::test]
    async fn mod_testen_braucht_einen_vorhandenen_mod() {
        let t = tempfile::tempdir().unwrap();
        let mods = Arc::new(crate::mods::Mods::new(t.path().join("mods")));
        let v = Arc::new(FakeVisual { seen: Mutex::new(Vec::new()) });
        let ctx = ToolCtx { visual: Some(v), folder: "C:\\x".into(), chat: "c1".into(),
            knowledge: Some(KnowledgeCtx { mods: Some(mods.clone()), ..Default::default() }), ..Default::default() };
        let r = run_tool(None, "mod_testen", &json!({ "id": "neon" }), &ctx).await;
        assert!(!r.ok && r.output.contains("Keinen Mod «neon»"), "{}", r.output);
        mods.save(&[("mod.json".into(), br#"{"id":"neon","name":"Neon"}"#.to_vec())], false).unwrap();
        let r = run_tool(None, "mod_testen", &json!({ "id": "neon" }), &ctx).await;
        assert!(!r.ok && r.output.contains("nur in der App"), "Attrappe ohne Testansicht: {}", r.output);
    }

    #[tokio::test]
    async fn ansehen_und_browser_ueber_das_trait() {
        let v = Arc::new(FakeVisual { seen: Mutex::new(Vec::new()) });
        let mut ctx = ToolCtx { visual: Some(v.clone()), folder: "C:\\x".into(), can_see: false, chat: "c1".into(), ..Default::default() };
        let r = run_tool(None, "view_file", &json!({ "path": "a.png" }), &ctx).await;
        assert!(!r.ok && r.summary == "Bilder nicht möglich");
        assert!(v.seen.lock().is_empty());
        let r = run_tool(None, "browser", &json!({ "ziel": "http://localhost:5173/" }), &ctx).await;
        assert!(r.output.contains("Das Bildschirmfoto bekommst du nicht"), "{}", r.output);
        assert_eq!(r.images, ["s.png"]);
        ctx.can_see = true;
        let r = run_tool(None, "view_file", &json!({ "path": "a.png", "seiten": "1-2" }), &ctx).await;
        assert!(r.ok && r.images == ["a.png"], "{}", r.output);
        assert_eq!(v.seen.lock().last().unwrap().2.seiten, json!("1-2"));
        let r = run_tool(None, "view_file", &json!({ "path": "a.png" }), &ToolCtx::default()).await;
        assert_eq!(r.output, "Fehler: Ansehen ist hier nicht verfügbar.");
    }

    #[test]
    fn python_commands_are_recognised() {
        for c in ["python auswertung.py", "py -3 a.py", "pip install x", "C:\\Pfad\\python.exe -m http.server", ".ask\\skripte\\x.py", "cd a && python b.py"] {
            assert!(uses_python(c), "{c}");
        }
        for c in ["dir", "node server.js", "npm run dev", "type pythonnotizen.txt"] {
            assert!(!uses_python(c), "{c}");
        }
    }
}