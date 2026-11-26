//! Entwicklungsserver eines Projekts (ENTSCHEIDE E48): npm run dev, python -m http.server, node server.js usw.
//! Sie laufen wie jeder Befehl im geschützten Terminal (Low Integrity, Job-Objekt), aber ohne dass jemand auf
//! ihr Ende wartet. Die App merkt sich id, Befehl, Startzeit und die letzten Zeilen der Ausgabe, erkennt darin
//! die Adresse (http://localhost:5173 usw.) und stoppt sie über das Terminal. Eine Instanz pro Projekt-Terminal.
//!
//! Öffentliche Schnittstelle:
//! - `Servers::new(ServersOptions::new(request))`; `request(method, path, body)` ist die Terminal-API des
//!   Projekts (tools.js `request`) und bleibt hereingereicht. Weitere Optionen: `probe`, `blocked_ports`,
//!   `available`, `now`, `poll_ms`.
//! - `list`, `get(id)`, `output(id, n)`, `running`, `allowed_ports`
//! - `start(command, name, by)`, `poll`, `wait_ready(id, ms)`, `stop(id)`, `remove(id)`, `stop_all`, `terminal_gone`
//! - `on("changed", f)`: `f` bekommt die Liste der Server als JSON-Array (JS: `servers.on('changed', …)`)
//! - freie Funktionen `detect_address`, `strip_ansi`, `probe_port`

use anyhow::{anyhow, bail, Result};
use futures::future::{BoxFuture, FutureExt, Shared};
use parking_lot::Mutex;
use regex::Regex;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tokio::task::JoinHandle;

use crate::github::encode_uri_component;
use crate::util::{js_string, now_ms, take_chars, Emitter};

pub const MAX_LINES: usize = 200;
const LINE_CHARS: usize = 2000;
/// laufende Server pro Projekt
pub const MAX_SERVERS: usize = 5;
const POLL_MS: u64 = 1000;
/// läuft er so lange ohne erkannte Adresse, gilt er trotzdem als gestartet
const READY_WITHOUT_URL_MS: i64 = 15000;
/// so viele Abfragen hintereinander ohne Antwort, dann gilt der Server als ausgefallen
const MAX_MISSES: u32 = 5;

fn active(state: &str) -> bool {
    state == "startet" || state == "läuft"
}

static ANSI: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]").unwrap());

/// Farben und Steuerzeichen (Vite, Next.js färben ihre Ausgabe)
pub fn strip_ansi(text: &str) -> String {
    ANSI.replace_all(text, "").into_owned()
}

fn valid_port(n: u64) -> bool {
    n > 0 && n < 65536
}

static URLS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)(?-u:\b)https?://(\[[0-9a-f:]+\]|[a-z0-9.-]+):([0-9]{1,5})(/[^\s'"<>)\]]*)?"#).unwrap()
});
static SERVING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)Serving HTTP on (\S+) port ([0-9]{1,5})").unwrap());
static LISTENING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?-u:\b)(?:listening|läuft|running|started|lauscht)(?-u:\b)[^\n]{0,30}?(?-u:\b)port(?-u:\b)\s*:?\s*([0-9]{2,5})(?-u:\b)").unwrap()
});
static LISTENING_ON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?-u:\b)listening (?:on|at)\s+:?([0-9]{2,5})(?-u:\b)").unwrap());

fn is_local_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "0.0.0.0" | "::1" | "::" | "[::1]" | "[::]")
        || (host.starts_with("127.") && host.split('.').count() == 4 && host.split('.').all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
}

/// Adresse eines lokalen Servers aus einer Zeile der Ausgabe, z. B.
///   «➜  Local:   http://localhost:5173/» (Vite), «- Local: http://localhost:3000» (Next.js),
///   «Serving HTTP on 0.0.0.0 port 8000 (http://0.0.0.0:8000/)» (Python), «Running on http://127.0.0.1:5000» (Flask),
///   «Server listening on port 3000» (Express und eigene Server). Adressen im Netz (Network: http://192.168…) zählen nicht.
/// Liefert (url, port) mit localhost bzw. 127.0.0.1, oder None.
pub fn detect_address(line: &str) -> Option<(String, u16)> {
    let text = strip_ansi(line);
    for m in URLS.captures_iter(&text) {
        let host = m[1].to_lowercase();
        let Ok(port) = m[2].parse::<u64>() else { continue };
        if !is_local_host(&host) || !valid_port(port) {
            continue;
        }
        let scheme = if m[0].to_lowercase().starts_with("https") { "https" } else { "http" };
        // 0.0.0.0 und :: heissen «alle Adressen»; geöffnet wird der Server auf diesem Rechner
        let name = if host.starts_with("127.") { "127.0.0.1" } else { "localhost" };
        let rest = m.get(3).map(|r| r.as_str()).unwrap_or("/").trim_end_matches(['.', ',', ';', ':']);
        let rest = if rest.is_empty() { "/" } else { rest };
        return Some((format!("{scheme}://{name}:{port}{rest}"), port as u16));
    }
    if let Some(s) = SERVING.captures(&text) {
        if let Ok(p) = s[2].parse::<u64>() {
            if valid_port(p) {
                return Some((format!("http://localhost:{p}/"), p as u16));
            }
        }
    }
    let listening = LISTENING.captures(&text).or_else(|| LISTENING_ON.captures(&text));
    if let Some(l) = listening {
        if let Ok(p) = l[1].parse::<u64>() {
            if valid_port(p) {
                return Some((format!("http://localhost:{p}/"), p as u16));
            }
        }
    }
    None
}

/// Antwortet etwas auf dem Port? (Der Server ist dann wirklich bereit, nicht nur angekündigt.)
pub async fn probe_port(port: u16, timeout_ms: u64) -> bool {
    let t = Duration::from_millis(timeout_ms);
    if matches!(tokio::time::timeout(t, tokio::net::TcpStream::connect(("127.0.0.1", port))).await, Ok(Ok(_))) {
        return true;
    }
    // Server, die nur auf ::1 lauschen (localhost unter Windows)
    matches!(tokio::time::timeout(t, tokio::net::TcpStream::connect(("::1", port))).await, Ok(Ok(_)))
}

fn output_text(json: &Value) -> String {
    json.get("output")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|o| o.get("data").map(js_string).unwrap_or_default()).collect::<String>())
        .unwrap_or_default()
}

fn short_name(command: &str) -> String {
    let c = command.split_whitespace().collect::<Vec<_>>().join(" ");
    if c.chars().count() > 40 {
        format!("{}…", take_chars(&c, 39))
    } else {
        c
    }
}

/// Antwort der Terminal-API (JS `{ status, json, text }`)
#[derive(Clone, Debug, Default)]
pub struct TerminalResponse {
    pub status: u16,
    pub json: Option<Value>,
    pub text: String,
}

/// `request(method, path, body)`: Terminal-API des Projekts (tools.js `request`)
pub type RequestFn = Arc<dyn Fn(String, String, Option<Value>) -> BoxFuture<'static, Result<TerminalResponse>> + Send + Sync>;
pub type ProbeFn = Arc<dyn Fn(u16) -> BoxFuture<'static, bool> + Send + Sync>;

pub struct ServersOptions {
    pub request: RequestFn,
    /// Antwortet etwas auf dem Port? (Standard: `probe_port`)
    pub probe: ProbeFn,
    /// Ports, die nie freigegeben werden (die Terminals der App)
    pub blocked_ports: Arc<dyn Fn() -> Vec<u16> + Send + Sync>,
    /// false, solange das Terminal nicht antwortet (dann wird nicht abgefragt und nichts gezählt)
    pub available: Arc<dyn Fn() -> bool + Send + Sync>,
    pub now: Arc<dyn Fn() -> i64 + Send + Sync>,
    pub poll_ms: u64,
}

impl ServersOptions {
    pub fn new(request: RequestFn) -> Self {
        ServersOptions {
            request,
            probe: Arc::new(|port| Box::pin(probe_port(port, 800))),
            blocked_ports: Arc::new(Vec::new),
            available: Arc::new(|| true),
            now: Arc::new(now_ms),
            poll_ms: POLL_MS,
        }
    }
}

/// Ein Eintrag der Liste, wie die Oberfläche ihn sieht
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerSnapshot {
    pub id: String,
    pub name: String,
    pub command: String,
    pub started: i64,
    pub state: String,
    pub exit_code: Option<i64>,
    pub url: String,
    pub port: u16,
    pub ready: bool,
    pub error: String,
    pub by: String,
    pub last: String,
}

/// Ergebnis von `start`: der Server, und `existing`, wenn derselbe Befehl schon lief
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StartedServer {
    #[serde(flatten)]
    pub server: ServerSnapshot,
    #[serde(skip_serializing_if = "is_false")]
    pub existing: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

struct Server {
    id: String,
    name: String,
    command: String,
    started: i64,
    state: String,
    exit_code: Option<i64>,
    url: String,
    port: u16,
    ready: bool,
    error: String,
    by: String,
    lines: Vec<String>,
    partial: String,
    offset: i64,
    misses: u32,
    stopping: bool,
}

impl Server {
    fn snapshot(&self) -> ServerSnapshot {
        ServerSnapshot {
            id: self.id.clone(),
            name: self.name.clone(),
            command: self.command.clone(),
            started: self.started,
            state: self.state.clone(),
            exit_code: self.exit_code,
            url: self.url.clone(),
            port: self.port,
            ready: self.ready,
            error: self.error.clone(),
            by: self.by.clone(),
            last: self.lines.last().cloned().unwrap_or_default(),
        }
    }

    fn add_output(&mut self, text: &str) {
        let joined = format!("{}{}", self.partial, text.replace("\r\n", "\n"));
        let mut parts: Vec<&str> = joined.split('\n').collect();
        self.partial = parts.pop().unwrap_or("").to_string();
        // Fortschrittsanzeigen überschreiben ihre Zeile mit \r: nur der letzte Stand zählt
        let clean = |line: &str| take_chars(&strip_ansi(line.split('\r').rfind(|p| !p.is_empty()).unwrap_or("")), LINE_CHARS);
        for raw in parts {
            let line = clean(raw);
            if self.port == 0 {
                if let Some((url, port)) = detect_address(&line) {
                    self.url = url;
                    self.port = port;
                }
            }
            self.lines.push(line);
        }
        if self.lines.len() > MAX_LINES {
            let cut = self.lines.len() - MAX_LINES;
            self.lines.drain(..cut);
        }
        if self.partial.chars().count() > LINE_CHARS {
            let line = clean(&self.partial);
            self.lines.push(line);
            self.partial.clear();
        }
        if self.port == 0 && !self.partial.is_empty() {
            if let Some((url, port)) = detect_address(&self.partial) {
                // Zeile ohne Zeilenumbruch, z. B. eine Eingabeaufforderung
                self.url = url;
                self.port = port;
            }
        }
    }
}

type SharedStart = Shared<BoxFuture<'static, std::result::Result<ServerSnapshot, String>>>;

struct Inner {
    request: RequestFn,
    probe: ProbeFn,
    blocked_ports: Arc<dyn Fn() -> Vec<u16> + Send + Sync>,
    available: Arc<dyn Fn() -> bool + Send + Sync>,
    now: Arc<dyn Fn() -> i64 + Send + Sync>,
    poll_ms: u64,
    /// id → Eintrag, in der Reihenfolge des Starts
    servers: Mutex<Vec<Arc<Mutex<Server>>>>,
    timer: Mutex<Option<JoinHandle<()>>>,
    polling: AtomicBool,
    /// Befehl → die noch laufenden Starts (reserviert den Platz vor dem ersten await)
    starting: Mutex<HashMap<String, SharedStart>>,
    events: Emitter,
}

/// Eine Instanz pro Projekt-Terminal; Klonen teilt den Zustand.
#[derive(Clone)]
pub struct Servers {
    inner: Arc<Inner>,
}

struct PollingGuard<'a>(&'a AtomicBool);

impl Drop for PollingGuard<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Servers {
    pub fn new(o: ServersOptions) -> Self {
        Servers {
            inner: Arc::new(Inner {
                request: o.request,
                probe: o.probe,
                blocked_ports: o.blocked_ports,
                available: o.available,
                now: o.now,
                poll_ms: o.poll_ms,
                servers: Mutex::new(Vec::new()),
                timer: Mutex::new(None),
                polling: AtomicBool::new(false),
                starting: Mutex::new(HashMap::new()),
                events: Emitter::new(),
            }),
        }
    }

    /// Zuhörer für Ereignisse; bisher nur «changed» mit der Liste der Server
    pub fn on(&self, event: &str, f: impl Fn(&Value) + Send + Sync + 'static) {
        self.inner.events.on(event, f);
    }

    fn all(&self) -> Vec<Arc<Mutex<Server>>> {
        self.inner.servers.lock().clone()
    }

    fn find(&self, id: &str) -> Option<Arc<Mutex<Server>>> {
        self.inner.servers.lock().iter().find(|s| s.lock().id == id).cloned()
    }

    pub fn list(&self) -> Vec<ServerSnapshot> {
        self.all().iter().map(|s| s.lock().snapshot()).collect()
    }

    pub fn get(&self, id: &str) -> Option<ServerSnapshot> {
        self.find(id).map(|s| s.lock().snapshot())
    }

    pub fn output(&self, id: &str, n: usize) -> Vec<String> {
        let Some(s) = self.find(id) else { return Vec::new() };
        let s = s.lock();
        let mut all: Vec<String> = s.lines.clone();
        if !s.partial.is_empty() {
            all.push(s.partial.clone());
        }
        let from = all.len().saturating_sub(n);
        all.split_off(from)
    }

    fn running_entries(&self) -> Vec<Arc<Mutex<Server>>> {
        self.all().into_iter().filter(|s| active(&s.lock().state)).collect()
    }

    pub fn running(&self) -> Vec<ServerSnapshot> {
        self.running_entries().iter().map(|s| s.lock().snapshot()).collect()
    }

    /// Ports, die der Browser für das Modell öffnen darf: nur von Servern, die die App gestartet hat und die noch laufen
    pub fn allowed_ports(&self) -> std::collections::HashSet<u16> {
        let blocked: std::collections::HashSet<u16> = (self.inner.blocked_ports)().into_iter().collect();
        self.running().into_iter().map(|s| s.port).filter(|p| *p != 0 && !blocked.contains(p)).collect()
    }

    fn changed(&self) {
        let list = serde_json::to_value(self.list()).unwrap_or(Value::Null);
        // Ein Fehler in einem Zuhörer darf den Abgleich nicht anhalten
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.inner.events.emit("changed", &list)));
    }

    /// Startet einen Befehl, ohne auf sein Ende zu warten. by: «person» oder «modell»
    pub async fn start(&self, command: &str, name: &str, by: &str) -> Result<StartedServer> {
        let cmd = command.trim().to_string();
        if cmd.is_empty() {
            bail!("Kein Befehl angegeben.");
        }
        let same = self.running().into_iter().find(|s| s.command == cmd);
        if let Some(same) = same {
            return Ok(StartedServer { server: same, existing: true });
        }
        // Zweiter Start desselben Befehls, während der erste noch läuft: auf dessen Ergebnis warten statt nochmals starten
        let pending = self.inner.starting.lock().get(&cmd).cloned();
        if let Some(pending) = pending {
            let server = pending.await.map_err(|e| anyhow!(e))?;
            return Ok(StartedServer { server, existing: true });
        }
        let job: SharedStart = {
            let mut starting = self.inner.starting.lock();
            if self.running().len() + starting.len() >= MAX_SERVERS {
                bail!("Es laufen schon {MAX_SERVERS} Server. Zuerst einen stoppen.");
            }
            let me = self.clone();
            let (c, n, b) = (cmd.clone(), name.to_string(), by.to_string());
            let job: SharedStart = async move { me.launch(c, n, b).await.map_err(|e| e.to_string()) }.boxed().shared();
            starting.insert(cmd.clone(), job.clone());
            job
        };
        let result = job.await;
        self.inner.starting.lock().remove(&cmd);
        Ok(StartedServer { server: result.map_err(|e| anyhow!(e))?, existing: false })
    }

    async fn launch(&self, cmd: String, name: String, by: String) -> Result<ServerSnapshot> {
        // wait=0: sofort zurück (ohne wait wartet open-terminal bis zu 60 s auf das Ende)
        let res = (self.inner.request)("POST".into(), "/execute?wait=0".into(), Some(json!({ "command": cmd }))).await?;
        let id = res.json.as_ref().and_then(|j| j.get("id")).filter(|i| crate::util::truthy(i)).map(js_string);
        let (Some(id), 200) = (id, res.status) else {
            let detail = res.json.as_ref().and_then(|j| j.get("detail"));
            return match detail {
                Some(Value::String(d)) => Err(anyhow!(d.clone())),
                _ => Err(anyhow!("Der Server liess sich nicht starten (HTTP {}).", res.status)),
            };
        };
        let name = take_chars(name.trim(), 60);
        let s = Arc::new(Mutex::new(Server {
            id,
            name: if name.is_empty() { short_name(&cmd) } else { name },
            command: cmd,
            started: (self.inner.now)(),
            state: "startet".into(),
            exit_code: None,
            url: String::new(),
            port: 0,
            ready: false,
            error: String::new(),
            by,
            lines: Vec::new(),
            partial: String::new(),
            offset: 0,
            misses: 0,
            stopping: false,
        }));
        self.inner.servers.lock().push(s.clone());
        self.apply(&s, res.json.as_ref().unwrap_or(&Value::Null)).await;
        self.changed();
        self.schedule();
        let snap = s.lock().snapshot();
        Ok(snap)
    }

    /// Neue Ausgabe und Status aus einer Antwort des Terminals übernehmen
    async fn apply(&self, s: &Arc<Mutex<Server>>, json: &Value) {
        let port_to_probe = {
            let mut s = s.lock();
            let text = output_text(json);
            if !text.is_empty() {
                s.add_output(&text);
            }
            s.offset = match json.get("next_offset").and_then(Value::as_i64) {
                Some(o) => o,
                None => s.offset + json.get("output").and_then(Value::as_array).map(|a| a.len() as i64).unwrap_or(0),
            };
            let status = json.get("status").filter(|v| crate::util::truthy(v)).map(js_string);
            if let Some(status) = status.filter(|st| st != "running") {
                if active(&s.state) {
                    s.exit_code = json.get("exit_code").and_then(Value::as_i64);
                    s.state = if s.stopping || status == "killed" || s.exit_code == Some(0) { "gestoppt" } else { "fehler" }.into();
                    if s.state == "fehler" {
                        s.error = format!("Beendet mit Exitcode {}", s.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "null".into()));
                    }
                    s.ready = false;
                    return;
                }
            }
            if s.state != "startet" {
                return;
            }
            if s.port != 0 && !s.ready { Some(s.port) } else { None }
        };
        if let Some(port) = port_to_probe {
            let ok = (self.inner.probe)(port).await;
            let mut s = s.lock();
            // Während der Probe kann stop() den Server beendet haben: den Zustand nicht überschreiben
            if s.state != "startet" || s.stopping {
                return;
            }
            if ok {
                s.ready = true;
            }
        }
        let mut s = s.lock();
        if s.ready || (s.port == 0 && (self.inner.now)() - s.started >= READY_WITHOUT_URL_MS) {
            s.state = "läuft".into();
        }
    }

    fn schedule(&self) {
        let mut timer = self.inner.timer.lock();
        if timer.is_some() || self.running_entries().is_empty() {
            return;
        }
        let me = self.clone();
        *timer = Some(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(me.inner.poll_ms)).await;
            *me.inner.timer.lock() = None;
            me.poll().await;
            me.schedule();
        }));
    }

    pub async fn poll(&self) {
        if self.inner.polling.load(Ordering::SeqCst) || !(self.inner.available)() {
            return;
        }
        self.inner.polling.store(true, Ordering::SeqCst);
        let _guard = PollingGuard(&self.inner.polling);
        for s in self.running_entries() {
            let before = s.lock().snapshot();
            if let Err(_miss) = self.poll_one(&s).await {
                let mut s = s.lock();
                s.misses += 1;
                if s.misses >= MAX_MISSES && active(&s.state) {
                    s.state = "fehler".into();
                    s.error = "Das Terminal antwortet nicht.".into();
                }
            }
            if s.lock().snapshot() != before {
                self.changed();
            }
        }
    }

    async fn poll_one(&self, s: &Arc<Mutex<Server>>) -> Result<()> {
        let (id, offset) = {
            let s = s.lock();
            (s.id.clone(), s.offset)
        };
        let path = format!("/execute/{}/status?wait=0&offset={}", encode_uri_component(&id), offset);
        let res = (self.inner.request)("GET".into(), path, None).await?;
        if res.status == 404 {
            let mut s = s.lock();
            let stopping = s.stopping;
            s.state = if stopping { "gestoppt" } else { "fehler" }.into();
            if !stopping {
                s.error = "Das Terminal kennt den Server nicht mehr.".into();
            }
        } else if let (200, Some(json)) = (res.status, res.json.as_ref()) {
            s.lock().misses = 0;
            self.apply(s, json).await;
        } else {
            bail!("HTTP {}", res.status);
        }
        Ok(())
    }

    /// Wartet, bis der Server eine Adresse hat und antwortet, beendet ist oder ms vorbei sind
    pub async fn wait_ready(&self, id: &str, ms: i64) -> Option<ServerSnapshot> {
        let end = (self.inner.now)() + ms;
        loop {
            let s = self.find(id)?;
            let snap = s.lock().snapshot();
            if !active(&snap.state) || snap.ready || (self.inner.now)() >= end {
                return Some(snap);
            }
            tokio::time::sleep(Duration::from_millis(250.min(self.inner.poll_ms))).await;
            if self.inner.timer.lock().is_none() && !self.inner.polling.load(Ordering::SeqCst) {
                self.poll().await;
            }
        }
    }

    pub async fn stop(&self, id: &str) -> Result<bool> {
        let Some(s) = self.find(id) else { return Ok(false) };
        let (sid, is_active) = {
            let mut g = s.lock();
            let a = active(&g.state);
            if a {
                g.stopping = true;
            }
            (g.id.clone(), a)
        };
        if is_active {
            let path = format!("/execute/{}", encode_uri_component(&sid));
            let res = (self.inner.request)("DELETE".into(), path, None).await.and_then(|r| {
                if r.status != 200 && r.status != 404 {
                    Err(anyhow!("HTTP {}", r.status))
                } else {
                    Ok(r)
                }
            });
            if let Err(err) = res {
                s.lock().stopping = false;
                bail!("Der Server liess sich nicht stoppen: {err}");
            }
            let mut g = s.lock();
            g.state = "gestoppt".into();
            g.ready = false;
        }
        self.changed();
        Ok(true)
    }

    /// Beendete Server aus der Liste nehmen
    pub fn remove(&self, id: &str) -> bool {
        let removed = {
            let mut all = self.inner.servers.lock();
            match all.iter().position(|s| s.lock().id == id) {
                Some(i) if !active(&all[i].lock().state) => {
                    all.remove(i);
                    true
                }
                _ => false,
            }
        };
        if removed {
            self.changed();
        }
        removed
    }

    fn clear_timer(&self) {
        if let Some(t) = self.inner.timer.lock().take() {
            t.abort();
        }
    }

    /// Alle stoppen (Projekt wird geschlossen, App wird beendet). Fehler zählen nicht: Das Terminal beendet beim
    /// Stoppen ohnehin alle Prozesse seines Job-Objekts.
    pub async fn stop_all(&self) {
        let ids: Vec<String> = self.running().into_iter().map(|s| s.id).collect();
        futures::future::join_all(ids.iter().map(|id| self.stop(id))).await;
        self.clear_timer();
    }

    /// Das Terminal wurde beendet oder neu gestartet: Seine Prozesse sind weg
    pub fn terminal_gone(&self) {
        let mut any = false;
        for s in self.running_entries() {
            let mut s = s.lock();
            s.state = "gestoppt".into();
            s.ready = false;
            s.error = "Das Terminal wurde beendet.".into();
            any = true;
        }
        self.clear_timer();
        if any {
            self.changed();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Ersatz für open-terminal: Prozesse mit Ausgabe (Liste von Einträgen), Status und Exitcode
    struct Proc {
        output: Vec<Value>,
        status: String,
        exit_code: Value,
    }

    #[derive(Default)]
    struct FakeTerminal {
        procs: Mutex<HashMap<String, Proc>>,
        calls: Mutex<Vec<String>>,
        next: Mutex<u32>,
    }

    impl FakeTerminal {
        fn add(&self, id: &str, data: &str) {
            self.procs.lock().get_mut(id).unwrap().output.push(json!({ "type": "stdout", "data": data }));
        }

        fn set(&self, id: &str, status: &str, exit_code: i64) {
            let mut p = self.procs.lock();
            let p = p.get_mut(id).unwrap();
            p.status = status.into();
            p.exit_code = json!(exit_code);
        }

        fn handle(&self, method: &str, path: &str, body: Option<Value>) -> TerminalResponse {
            let ok = |j: Value| TerminalResponse { status: 200, json: Some(j), text: String::new() };
            self.calls.lock().push(format!("{method} {path}"));
            if method == "POST" && path.starts_with("/execute") {
                let id = {
                    let mut n = self.next.lock();
                    *n += 1;
                    format!("p{n}")
                };
                let _ = body;
                self.procs.lock().insert(id.clone(), Proc { output: vec![], status: "running".into(), exit_code: Value::Null });
                return ok(json!({ "id": id, "status": "running", "exit_code": null, "output": [], "next_offset": 0 }));
            }
            let re = Regex::new(r"^/execute/([^/?]+)(/status\?wait=0&offset=([0-9]+))?$").unwrap();
            let m = re.captures(path);
            let mut procs = self.procs.lock();
            let Some((m, p)) = m.and_then(|m| procs.get_mut(&m[1]).map(|p| (m, p))) else {
                return TerminalResponse { status: 404, json: Some(json!({ "detail": "Process not found" })), text: String::new() };
            };
            if method == "DELETE" {
                p.status = "killed".into();
                p.exit_code = json!(-1);
                return ok(json!({ "status": "killed" }));
            }
            let offset: usize = m.get(3).and_then(|o| o.as_str().parse().ok()).unwrap_or(0);
            ok(json!({ "id": &m[1], "status": p.status, "exit_code": p.exit_code,
                "output": p.output[offset.min(p.output.len())..], "next_offset": p.output.len() }))
        }

        fn request_fn(self: &Arc<Self>) -> RequestFn {
            let me = self.clone();
            Arc::new(move |m, p, b| {
                let r = me.handle(&m, &p, b);
                Box::pin(async move { Ok(r) })
            })
        }
    }

    fn make(t: &Arc<FakeTerminal>, tweak: impl FnOnce(&mut ServersOptions)) -> Servers {
        let mut o = ServersOptions::new(t.request_fn());
        o.probe = Arc::new(|_| Box::pin(async { true }));
        o.poll_ms = 10;
        tweak(&mut o);
        Servers::new(o)
    }

    fn fake() -> Arc<FakeTerminal> {
        Arc::new(FakeTerminal::default())
    }

    #[test]
    fn adresse_aus_der_ausgabe_ueblicher_entwicklungsserver() {
        let cases = [
            ("  \x1b[32m➜\x1b[39m  \x1b[1mLocal\x1b[22m:   \x1b[36mhttp://localhost:\x1b[1m5173\x1b[22m/\x1b[39m", "http://localhost:5173/", 5173),
            ("   - Local:        http://localhost:3000", "http://localhost:3000/", 3000),
            ("Serving HTTP on 0.0.0.0 port 8000 (http://0.0.0.0:8000/) ...", "http://localhost:8000/", 8000),
            ("Serving HTTP on :: port 8080 (http://[::]:8080/) ...", "http://localhost:8080/", 8080),
            (" * Running on http://127.0.0.1:5000", "http://127.0.0.1:5000/", 5000),
            ("Server läuft auf http://127.0.0.1:8000/index.html.", "http://127.0.0.1:8000/index.html", 8000),
            ("Server listening on port 4000", "http://localhost:4000/", 4000),
            ("Listening on :3001", "http://localhost:3001/", 3001),
            ("App running at https://localhost:8443/app", "https://localhost:8443/app", 8443),
        ];
        for (line, url, port) in cases {
            assert_eq!(detect_address(line), Some((url.to_string(), port)), "{line}");
        }
        for line in ["  ➜  Network: http://192.168.1.20:5173/", "Siehe https://vitejs.dev/config", "VITE v6.3.1  ready in 412 ms", "Port 99999 is in use", "http://example.com:8080/"] {
            assert_eq!(detect_address(line), None, "{line}");
        }
        assert_eq!(strip_ansi("\x1b[1mfett\x1b[22m"), "fett");
    }

    #[tokio::test]
    async fn start_adresse_erkannt_zustand_startet_laeuft_ausgabe_gekuerzt_stopp_ueber_delete() {
        let t = fake();
        let s = make(&t, |_| {});
        let changes = Arc::new(Mutex::new(0));
        let c = changes.clone();
        s.on("changed", move |_| *c.lock() += 1);
        let started = s.start("npm run dev", "Webseite", "modell").await.unwrap().server;
        assert_eq!(started.state, "startet");
        assert_eq!(started.name, "Webseite");
        assert!(t.calls.lock()[0].starts_with("POST /execute?wait=0"), "ohne Warten starten");
        t.add(&started.id, "\n  VITE v6.3.1  ready in 412 ms\n\n");
        t.add(&started.id, "  ➜  Local:   http://localhost:5173/\n  ➜  Network: use --host to expose\n");
        let ready = s.wait_ready(&started.id, 2000).await.unwrap();
        assert_eq!(ready.state, "läuft");
        assert_eq!(ready.url, "http://localhost:5173/");
        assert_eq!(ready.port, 5173);
        assert!(ready.ready);
        assert_eq!(s.allowed_ports().into_iter().collect::<Vec<_>>(), [5173]);
        assert!(*changes.lock() >= 2);

        for i in 0..250 {
            t.add(&started.id, &format!("Zeile {i}\n"));
        }
        s.poll().await;
        let out = s.output(&started.id, MAX_LINES);
        assert_eq!(out.len(), MAX_LINES);
        assert_eq!(out.last().unwrap(), "Zeile 249");

        // Derselbe Befehl läuft schon: kein zweiter Start
        let again = s.start("npm run dev", "", "person").await.unwrap();
        assert_eq!(again.server.id, started.id);
        assert!(again.existing);

        assert!(s.stop(&started.id).await.unwrap());
        assert!(t.calls.lock().contains(&format!("DELETE /execute/{}", started.id)));
        assert_eq!(s.get(&started.id).unwrap().state, "gestoppt");
        assert!(s.allowed_ports().is_empty(), "gestoppte Server geben keinen Port frei");
        s.stop_all().await;
    }

    #[tokio::test]
    async fn server_beendet_sich_selbst_exitcode_fehler_bei_code_ungleich_0() {
        let t = fake();
        let s = make(&t, |_| {});
        let a = s.start("node server.js", "", "person").await.unwrap().server;
        t.add(&a.id, "Error: Cannot find module 'express'\n");
        t.set(&a.id, "done", 1);
        s.poll().await;
        let snap = s.get(&a.id).unwrap();
        assert_eq!(snap.state, "fehler");
        assert_eq!(snap.exit_code, Some(1));
        assert!(snap.last.contains("Cannot find module"));
        let b = s.start("python -m http.server", "", "person").await.unwrap().server;
        t.set(&b.id, "done", 0);
        s.poll().await;
        assert_eq!(s.get(&b.id).unwrap().state, "gestoppt");
        assert!(s.remove(&a.id));
        assert_eq!(s.list().len(), 1);
        s.stop_all().await;
    }

    #[tokio::test]
    async fn port_erst_freigegeben_wenn_der_server_antwortet_ports_der_terminals_bleiben_gesperrt() {
        let t = fake();
        let open = Arc::new(AtomicBool::new(false));
        let o = open.clone();
        let s = make(&t, |opt| {
            opt.probe = Arc::new(move |_| {
                let o = o.clone();
                Box::pin(async move { o.load(Ordering::SeqCst) })
            });
            opt.blocked_ports = Arc::new(|| vec![24567]);
        });
        let a = s.start("python -m http.server 8000", "", "person").await.unwrap().server;
        t.add(&a.id, "Serving HTTP on 0.0.0.0 port 8000 (http://0.0.0.0:8000/) ...\n");
        s.poll().await;
        assert_eq!(s.get(&a.id).unwrap().state, "startet");
        assert_eq!(s.get(&a.id).unwrap().url, "http://localhost:8000/");
        open.store(true, Ordering::SeqCst);
        s.poll().await;
        assert_eq!(s.get(&a.id).unwrap().state, "läuft");
        let b = s.start("python fake.py", "", "person").await.unwrap().server;
        t.add(&b.id, "Server listening on port 24567\n");
        s.poll().await;
        let mut ports: Vec<u16> = s.allowed_ports().into_iter().collect();
        ports.sort();
        assert_eq!(ports, [8000]);
        s.stop_all().await;
    }

    #[tokio::test]
    async fn ohne_erkennbare_adresse_gilt_ein_laufender_server_nach_einer_weile_als_gestartet() {
        let t = fake();
        let clock = Arc::new(Mutex::new(1000i64));
        let c = clock.clone();
        let s = make(&t, |o| o.now = Arc::new(move || *c.lock()));
        let a = s.start("node bot.js", "", "person").await.unwrap().server;
        s.poll().await;
        assert_eq!(s.get(&a.id).unwrap().state, "startet");
        *clock.lock() += 20000;
        s.poll().await;
        assert_eq!(s.get(&a.id).unwrap().state, "läuft");
        assert_eq!(s.get(&a.id).unwrap().url, "");
        assert!(s.allowed_ports().is_empty());
        s.stop_all().await;
    }

    #[tokio::test]
    async fn terminal_beendet_oder_kennt_den_prozess_nicht_mehr() {
        let t = fake();
        let s = make(&t, |_| {});
        let a = s.start("npm start", "", "person").await.unwrap().server;
        t.procs.lock().remove(&a.id);
        s.poll().await;
        assert_eq!(s.get(&a.id).unwrap().state, "fehler");
        let b = s.start("npm run dev", "", "person").await.unwrap().server;
        s.terminal_gone();
        assert_eq!(s.get(&b.id).unwrap().state, "gestoppt");
        assert_eq!(s.running().len(), 0);
    }

    #[tokio::test]
    async fn ausgabe_mit_r_fortschritt_und_ueber_stuecke_verteilten_zeilen() {
        let t = fake();
        let s = make(&t, |_| {});
        let a = s.start("npm run dev", "", "person").await.unwrap().server;
        t.add(&a.id, "lädt 10%\rlädt 50%\rlädt 100%\r\nServer läuft auf http://loc");
        s.poll().await;
        t.add(&a.id, "alhost:3000\n");
        s.poll().await;
        assert_eq!(s.output(&a.id, MAX_LINES), ["lädt 100%", "Server läuft auf http://localhost:3000"]);
        assert_eq!(s.get(&a.id).unwrap().port, 3000);
        s.stop_all().await;
    }

    #[tokio::test]
    async fn hoechstens_fuenf_laufende_server_leerer_befehl_abgelehnt() {
        let t = fake();
        let s = make(&t, |_| {});
        for i in 0..5 {
            s.start(&format!("node s{i}.js"), "", "person").await.unwrap();
        }
        assert!(s.start("node s5.js", "", "person").await.unwrap_err().to_string().contains("schon 5 Server"));
        assert!(s.start("  ", "", "person").await.unwrap_err().to_string().contains("Kein Befehl"));
        s.stop_all().await;
        assert_eq!(s.running().len(), 0);
    }

    #[tokio::test]
    async fn probe_auf_den_port_offen_und_geschlossen() {
        let srv = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = srv.local_addr().unwrap().port();
        assert!(probe_port(port, 800).await);
        drop(srv);
        assert!(!probe_port(port, 800).await);
    }

    #[tokio::test]
    async fn stop_waehrend_der_probe_apply_ueberschreibt_gestoppt_nicht_mit_laeuft() {
        let t = fake();
        let gate = Arc::new(tokio::sync::Notify::new());
        let g = gate.clone();
        let s = make(&t, |o| {
            o.probe = Arc::new(move |_| {
                let g = g.clone();
                Box::pin(async move {
                    g.notified().await;
                    true
                })
            })
        });
        let started = s.start("npm run dev", "", "person").await.unwrap().server;
        t.add(&started.id, "  ➜  Local:   http://localhost:5173/\n");
        let s2 = s.clone();
        let polling = tokio::spawn(async move { s2.poll().await }); // bleibt in der Probe hängen
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(s.stop(&started.id).await.unwrap());
        assert_eq!(s.get(&started.id).unwrap().state, "gestoppt");
        gate.notify_waiters();
        gate.notify_one();
        polling.await.unwrap();
        assert_eq!(s.get(&started.id).unwrap().state, "gestoppt");
        assert!(!s.get(&started.id).unwrap().ready);
        assert!(s.allowed_ports().is_empty());
        s.stop_all().await;
    }

    #[tokio::test]
    async fn zwei_gleichzeitige_starts_desselben_befehls_starten_nur_einen_prozess() {
        let t = fake();
        let s = make(&t, |_| {});
        let (a, b) = tokio::join!(s.start("node a.js", "", "person"), s.start("node a.js", "", "person"));
        let (a, b) = (a.unwrap(), b.unwrap());
        assert_eq!(a.server.id, b.server.id);
        assert!(a.existing || b.existing);
        assert_eq!(t.calls.lock().iter().filter(|c| c.starts_with("POST /execute")).count(), 1);
        assert_eq!(s.running().len(), 1);
        // Gleichzeitige Starts zählen auch gegen die Höchstzahl
        let cmds: Vec<String> = (0..7).map(|i| format!("node s{i}.js")).collect();
        let many = futures::future::join_all(cmds.iter().map(|c| s.start(c, "", "person"))).await;
        assert_eq!(many.iter().filter(|r| r.is_err()).count(), 3);
        assert_eq!(s.running().len(), 5);
        s.stop_all().await;
    }

    #[tokio::test]
    async fn ein_fehler_in_einem_zuhoerer_haelt_den_abgleich_nicht_an() {
        let t = fake();
        let s = make(&t, |_| {});
        let started = s.start("node a.js", "", "person").await.unwrap().server;
        s.on("changed", |_| panic!("Zuhörer kaputt"));
        t.add(&started.id, "Zeile\n");
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(!s.inner.polling.load(Ordering::SeqCst));
        assert_eq!(s.output(&started.id, MAX_LINES), ["Zeile"]);
        s.stop_all().await;
    }
}
