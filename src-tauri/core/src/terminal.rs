//! Verwaltung des lokalen Terminals: Port, Key, Start, Health-Check, Neustart.
//! Das eigentliche Starten übernimmt ein Backend (Trait `Backend`, Standard `native_low::NativeLowBackend`
//! aus backend/native-low.js), damit sich die Isolationsvariante später austauschen lässt.
//!
//! Öffentliche Schnittstelle:
//! - `Terminal::new(backend)`, `Terminal::native()` (mit Low Integrity)
//! - `start(options) -> Future`, `stop() -> Future`: Der Aufruf wirkt sofort (wie ein JS-Promise, auch ohne
//!   `.await`); mehrere Aufrufe laufen nacheinander, nur der letzte Start läuft wirklich.
//! - `state`, `error`, `detail`, `port`, `key`, `folder`, `info`, `logs`, `generation`, `url`, `snapshot`
//! - `set_prepare(f)`: Hook vor jedem Start (Python entpacken, ENTSCHEIDE E39), mit Fortschrittsmeldung
//! - `check_health(generation)` (Probe alle 10 s, öffentlich für Tests)
//! - Ereignisse über `on(name, f)`: «state» (Schnappschuss als JSON), «port» (Zahl)
//! - freie Funktionen `is_port_free`, `choose_port`, `get_json`
//! - `native_low::NativeLowBackend`: startet `python launcher.py` und gibt den Key über stdin

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::util::{now_ms, Emitter};

const PORT_MIN: u16 = 20000;
const PORT_MAX: u16 = 39999;
/// so viele Proben hintereinander ohne Antwort, bevor das Terminal als ausgefallen gilt
const HEALTH_MISSES: u32 = 3;
const MAX_LOGS: usize = 300;

pub async fn is_port_free(port: u16) -> bool {
    tokio::net::TcpListener::bind(("127.0.0.1", port)).await.is_ok()
}

pub async fn choose_port(preferred: Option<u16>) -> Result<u16> {
    if let Some(p) = preferred.filter(|p| *p != 0) {
        if is_port_free(p).await {
            return Ok(p);
        }
    }
    for _ in 0..50 {
        let port = {
            use rand::Rng;
            rand::rng().random_range(PORT_MIN..PORT_MAX)
        };
        if is_port_free(port).await {
            return Ok(port);
        }
    }
    bail!("Kein freier Port gefunden")
}

fn local_client() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    // Das Terminal läuft auf diesem Rechner: nie über einen Proxy
    CLIENT.get_or_init(|| reqwest::Client::builder().no_proxy().build().expect("HTTP-Client"))
}

/// GET auf das Terminal, Antwort als JSON; alles ausser HTTP 200 ist ein Fehler
pub async fn get_json(port: u16, path: &str, key: Option<&str>, timeout_ms: u64) -> Result<Value> {
    let mut req = local_client().get(format!("http://127.0.0.1:{port}{path}")).timeout(Duration::from_millis(timeout_ms));
    if let Some(k) = key.filter(|k| !k.is_empty()) {
        req = req.header("Authorization", format!("Bearer {k}"));
    }
    let res = req.send().await.map_err(|e| if e.is_timeout() { anyhow!("Zeitüberschreitung") } else { e.into() })?;
    if res.status().as_u16() != 200 {
        bail!("HTTP {}", res.status().as_u16());
    }
    Ok(res.json().await?)
}

// ---------- Backend ----------

/// Ereignisse des Backends als JSON: `{event:'log', line}`, `{event:'error', code, message}`,
/// `{event:'exit'|'process-exit', code}` und alles, was der Starter sonst auf stdout schreibt.
pub type EventFn = Arc<dyn Fn(Value) + Send + Sync>;

/// Was ein Backend zum Starten braucht
#[derive(Clone)]
pub struct BackendStart {
    pub workdir: String,
    pub port: u16,
    pub key: String,
    pub origin: Option<String>,
    pub system_prompt: Option<String>,
    pub execute_description: Option<String>,
    pub on_event: EventFn,
}

/// Ein anderes Backend (Sandbox, WSL, AppContainer) muss nur `start`, `stop` und `running`
/// mit derselben Bedeutung anbieten.
#[async_trait]
pub trait Backend: Send + Sync + 'static {
    async fn start(&self, options: BackendStart) -> Result<()>;
    async fn stop(&self);
    fn running(&self) -> bool;
}

/// Backend «nativ mit Low Integrity»: startet python/ask_terminal/launcher.py mit dem mitgelieferten
/// Python. Der Starter kennzeichnet den Ordner, senkt den Server auf Low und hält ihn in einem
/// Job-Objekt. Der Key geht über stdin, nie über Kommandozeile oder Datei.
pub mod native_low {
    use super::*;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::{ChildStdin, Command};
    use tokio::sync::{watch, Notify};

    struct Running {
        id: u64,
        stdin: Option<ChildStdin>,
        kill: Arc<Notify>,
        exited: watch::Receiver<bool>,
    }

    #[derive(Default)]
    pub struct NativeLowBackend {
        cur: Arc<Mutex<Option<Running>>>,
        next_id: AtomicU64,
    }

    impl NativeLowBackend {
        pub fn new() -> Self {
            Self::default()
        }
    }

    #[async_trait]
    impl Backend for NativeLowBackend {
        fn running(&self) -> bool {
            self.cur.lock().is_some()
        }

        async fn start(&self, o: BackendStart) -> Result<()> {
            let python = crate::paths::python_exe();
            if !python.exists() {
                bail!("Mitgeliefertes Python fehlt: {}", python.display());
            }
            // -X utf8: Die Einstellungen gehen als UTF-8 über stdin. Ohne den Schalter liest Python sie auf Windows in
            // der ANSI-Codepage, und ein Ordner oder Benutzername mit Umlaut («Prüfung») wird nicht gefunden.
            let mut cmd = Command::new(&python);
            cmd.args(["-E", "-s", "-B", "-X", "utf8"])
                .arg(crate::paths::launcher_py())
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            #[cfg(windows)]
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW (JS: windowsHide)
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(err) => {
                    (o.on_event)(json!({ "event": "error", "code": "spawn", "message": err.to_string() }));
                    return Err(err.into());
                }
            };
            let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
            let mut stdin = child.stdin.take().expect("stdin");
            let stdout = child.stdout.take().expect("stdout");
            let stderr = child.stderr.take().expect("stderr");
            let ev = o.on_event.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    match serde_json::from_str::<Value>(&line) {
                        Ok(v) => ev(v),
                        Err(_) => ev(json!({ "event": "log", "line": line })),
                    }
                }
            });
            let ev = o.on_event.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    ev(json!({ "event": "log", "line": line }));
                }
            });
            let cfg = {
                let mut m = serde_json::Map::new();
                m.insert("key".into(), json!(o.key));
                m.insert("port".into(), json!(o.port));
                m.insert("workdir".into(), json!(o.workdir));
                if let Some(v) = &o.origin {
                    m.insert("origin".into(), json!(v));
                }
                m.insert("runtime".into(), json!(crate::paths::runtime_dir().to_string_lossy()));
                if let Some(v) = &o.system_prompt {
                    m.insert("system_prompt".into(), json!(v));
                }
                if let Some(v) = &o.execute_description {
                    m.insert("execute_description".into(), json!(v));
                }
                m.insert("low_integrity".into(), json!(true));
                Value::Object(m)
            };
            let _ = stdin.write_all(format!("{cfg}\n").as_bytes()).await;
            let _ = stdin.flush().await;
            let kill = Arc::new(Notify::new());
            let (exited_tx, exited_rx) = watch::channel(false);
            *self.cur.lock() = Some(Running { id, stdin: Some(stdin), kill: kill.clone(), exited: exited_rx });
            let cur = self.cur.clone();
            let ev = o.on_event.clone();
            tokio::spawn(async move {
                let status = tokio::select! {
                    s = child.wait() => s,
                    _ = kill.notified() => {
                        let _ = child.kill().await;
                        child.wait().await
                    }
                };
                {
                    let mut c = cur.lock();
                    if c.as_ref().is_some_and(|r| r.id == id) {
                        *c = None;
                    }
                }
                let _ = exited_tx.send(true);
                ev(json!({ "event": "process-exit", "code": status.ok().and_then(|s| s.code()) }));
            });
            Ok(())
        }

        /// Bittet den Starter, den Job zu beenden; nach 5 s wird er hart beendet (der Job räumt trotzdem auf).
        async fn stop(&self) {
            let Some(mut r) = self.cur.lock().take() else { return };
            if let Some(mut stdin) = r.stdin.take() {
                if stdin.write_all(b"stop\n").await.is_err() {
                    r.kill.notify_one();
                }
                let _ = stdin.shutdown().await;
                drop(stdin);
            }
            let mut exited = r.exited.clone();
            let wait = async {
                while !*exited.borrow_and_update() {
                    if exited.changed().await.is_err() {
                        break;
                    }
                }
            };
            if tokio::time::timeout(Duration::from_secs(5), wait).await.is_err() {
                r.kill.notify_one(); // bereits beendet ist kein Fehler
            }
        }
    }
}

// ---------- Terminal ----------

/// Zwischenstand beim Starten melden, z. B. «Python wird eingerichtet … 40 %»
pub type ReportFn = Arc<dyn Fn(String) + Send + Sync>;
/// Vor jedem Start, z. B. Python entpacken (ENTSCHEIDE E39)
pub type PrepareFn = Arc<dyn Fn(ReportFn) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Optionen von `start` (JS `{ folder, preferredPort, origin, systemPrompt, executeDescription }`)
#[derive(Clone, Debug, Default)]
pub struct StartOptions {
    pub folder: String,
    pub preferred_port: Option<u16>,
    pub origin: Option<String>,
    pub system_prompt: Option<String>,
    pub execute_description: Option<String>,
}

#[derive(Default)]
struct State {
    /// gestoppt | startet | läuft | fehler
    state: String,
    error: String,
    /// Zwischenstand beim Starten
    detail: String,
    port: u16,
    key: String,
    folder: String,
    info: Option<Value>,
    logs: VecDeque<String>,
    misses: u32,
    unhealthy: bool,
}

struct Inner {
    backend: Arc<dyn Backend>,
    st: Mutex<State>,
    generation: AtomicU64,
    prepare: Mutex<Option<PrepareFn>>,
    health_timer: Mutex<Option<JoinHandle<()>>>,
    /// Starten und Stoppen laufen streng nacheinander: Jeder Auftrag wartet auf das Ende des vorigen.
    queue: Mutex<Option<oneshot::Receiver<()>>>,
    events: Emitter,
}

/// Klonen teilt den Zustand.
#[derive(Clone)]
pub struct Terminal {
    inner: Arc<Inner>,
}

impl Terminal {
    /// backend: nur für Tests etwas anderes; sonst das Terminal mit Low Integrity (`native()`)
    pub fn new(backend: Arc<dyn Backend>) -> Self {
        Terminal {
            inner: Arc::new(Inner {
                backend,
                st: Mutex::new(State { state: "gestoppt".into(), ..Default::default() }),
                generation: AtomicU64::new(0),
                prepare: Mutex::new(None),
                health_timer: Mutex::new(None),
                queue: Mutex::new(None),
                events: Emitter::new(),
            }),
        }
    }

    pub fn native() -> Self {
        Self::new(Arc::new(native_low::NativeLowBackend::new()))
    }

    /// Ereignisse: «state» (Schnappschuss) und «port» (Zahl)
    /// Dasselbe Terminal (geteilter Zustand)?
    pub fn ptr_eq(&self, other: &Terminal) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    pub fn on(&self, event: &str, f: impl Fn(&Value) + Send + Sync + 'static) {
        self.inner.events.on(event, f);
    }

    pub fn set_prepare(&self, f: Option<PrepareFn>) {
        *self.inner.prepare.lock() = f;
    }

    pub fn state(&self) -> String {
        self.inner.st.lock().state.clone()
    }

    pub fn error(&self) -> String {
        self.inner.st.lock().error.clone()
    }

    pub fn detail(&self) -> String {
        self.inner.st.lock().detail.clone()
    }

    pub fn port(&self) -> u16 {
        self.inner.st.lock().port
    }

    pub fn key(&self) -> String {
        self.inner.st.lock().key.clone()
    }

    pub fn folder(&self) -> String {
        self.inner.st.lock().folder.clone()
    }

    pub fn info(&self) -> Option<Value> {
        self.inner.st.lock().info.clone()
    }

    pub fn logs(&self) -> Vec<String> {
        self.inner.st.lock().logs.iter().cloned().collect()
    }

    pub fn generation(&self) -> u64 {
        self.inner.generation.load(Ordering::SeqCst)
    }

    pub fn url(&self) -> String {
        let port = self.port();
        if port != 0 { format!("http://127.0.0.1:{port}") } else { String::new() }
    }

    fn snapshot_of(&self, st: &State) -> Value {
        let url = if st.port != 0 { format!("http://127.0.0.1:{}", st.port) } else { String::new() };
        json!({ "state": st.state, "error": st.error, "detail": st.detail, "port": st.port, "url": url, "folder": st.folder,
            "lowIntegrity": st.info.as_ref().and_then(|i| i.get("low_integrity")).cloned().unwrap_or(Value::Null) })
    }

    pub fn snapshot(&self) -> Value {
        self.snapshot_of(&self.inner.st.lock())
    }

    fn emit_state(&self) {
        self.inner.events.emit("state", &self.snapshot());
    }

    fn set_state(&self, state: &str, error: &str) {
        {
            let mut st = self.inner.st.lock();
            st.state = state.to_string();
            st.error = error.to_string();
            if state != "startet" {
                st.detail.clear();
            }
        }
        self.emit_state();
    }

    pub fn log(&self, line: &str) {
        let mut st = self.inner.st.lock();
        st.logs.push_back(line.to_string());
        if st.logs.len() > MAX_LOGS {
            st.logs.pop_front();
        }
    }

    fn clear_health_timer(&self) {
        if let Some(t) = self.inner.health_timer.lock().take() {
            t.abort();
        }
    }

    /// Reiht einen Auftrag hinter den vorigen ein (sofort, nicht erst beim ersten Abfragen) und führt ihn
    /// in einer eigenen Aufgabe aus, wie ein JS-Promise.
    fn enqueue<F, Fut>(&self, job: F) -> impl Future<Output = ()> + Send + 'static
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let (done_tx, done_rx) = oneshot::channel::<()>();
        let prev = self.inner.queue.lock().replace(done_rx);
        let handle = tokio::spawn(async move {
            if let Some(prev) = prev {
                let _ = prev.await; // endet auch, wenn der Sender verworfen wird
            }
            job().await;
            drop(done_tx);
        });
        async move {
            let _ = handle.await;
        }
    }

    /// Mehrere Aufrufe kurz hintereinander (Doppelklick auf «Neu starten», zwei Ordnerwechsel) laufen
    /// nacheinander, und nur der letzte startet wirklich: Sonst bleibt ein Terminal übrig, von dem die App
    /// nichts mehr weiss, oder das Terminal läuft in einem anderen Ordner, als die App anzeigt.
    pub fn start(&self, options: StartOptions) -> impl Future<Output = ()> + Send + 'static {
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.clear_health_timer();
        let me = self.clone();
        self.enqueue(move || async move { me.start_now(options, generation).await })
    }

    fn current(&self, generation: u64) -> bool {
        generation == self.inner.generation.load(Ordering::SeqCst)
    }

    async fn start_now(&self, options: StartOptions, generation: u64) {
        if !self.current(generation) {
            return;
        }
        self.inner.backend.stop().await;
        if !self.current(generation) {
            return;
        }
        {
            let mut st = self.inner.st.lock();
            st.folder = options.folder.clone();
            st.info = None;
            st.misses = 0;
            st.unhealthy = false;
        }
        self.set_state("startet", "");
        match self.run_start(&options, generation).await {
            Ok(()) => {}
            Err(err) => {
                if !self.current(generation) {
                    return;
                }
                self.log(&format!("Start fehlgeschlagen: {err}"));
                self.inner.backend.stop().await;
                if !self.current(generation) {
                    return;
                }
                let current = self.error();
                self.set_state("fehler", &if current.is_empty() { err.to_string() } else { current });
            }
        }
    }

    async fn run_start(&self, options: &StartOptions, generation: u64) -> Result<()> {
        let prepare = self.inner.prepare.lock().clone();
        if let Some(prepare) = prepare {
            let me = self.clone();
            let report: ReportFn = Arc::new(move |detail: String| {
                if !me.current(generation) {
                    return;
                }
                me.inner.st.lock().detail = detail;
                me.emit_state();
            });
            prepare(report).await?;
            if !self.current(generation) {
                return Ok(());
            }
            self.inner.st.lock().detail.clear();
        }
        let port = choose_port(options.preferred_port).await?;
        let key = {
            use base64::Engine;
            use rand::RngCore;
            let mut bytes = [0u8; 32];
            rand::rng().fill_bytes(&mut bytes);
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        };
        {
            let mut st = self.inner.st.lock();
            st.port = port;
            st.key = key.clone();
        }
        self.inner.events.emit("port", &json!(port));
        let me = self.clone();
        self.inner
            .backend
            .start(BackendStart {
                workdir: options.folder.clone(),
                port,
                key,
                origin: options.origin.clone(),
                system_prompt: options.system_prompt.clone(),
                execute_description: options.execute_description.clone(),
                on_event: Arc::new(move |ev| me.on_backend_event(&ev, generation)),
            })
            .await?;
        self.wait_healthy(generation).await
    }

    fn on_backend_event(&self, ev: &Value, generation: u64) {
        if !self.current(generation) {
            return;
        }
        let kind = ev.get("event").and_then(Value::as_str).unwrap_or("");
        if kind == "log" {
            self.log(&crate::util::str_of(ev, "line"));
        } else {
            self.log(&ev.to_string());
        }
        if kind == "error" {
            let msg = crate::util::str_of(ev, "message");
            self.inner.st.lock().error = if ev.get("code").and_then(Value::as_str) == Some("label") {
                format!("Der Ordner konnte nicht geschützt werden ({msg}). Bitte einen Ordner auf der internen Festplatte unter dem eigenen Benutzerprofil wählen.")
            } else {
                msg
            };
        }
        if kind == "exit" || kind == "process-exit" {
            self.clear_health_timer();
            let state = {
                let mut st = self.inner.st.lock();
                st.unhealthy = false;
                st.state.clone()
            };
            if state == "läuft" || state == "startet" {
                let e = self.error();
                self.set_state("fehler", &if e.is_empty() { "Das Terminal wurde unerwartet beendet.".to_string() } else { e });
            }
        }
    }

    async fn wait_healthy(&self, generation: u64) -> Result<()> {
        let deadline = now_ms() + 60000;
        while now_ms() < deadline {
            if !self.current(generation) {
                return Ok(());
            }
            if self.state() == "fehler" {
                bail!("{}", self.error());
            }
            if !self.inner.backend.running() {
                let e = self.error();
                bail!("{}", if e.is_empty() { "Das Terminal hat sich beim Start beendet.".to_string() } else { e });
            }
            let (port, key) = {
                let st = self.inner.st.lock();
                (st.port, st.key.clone())
            };
            let info = match get_json(port, "/ask/info", Some(&key), 1500).await {
                Ok(i) => i,
                Err(_) => {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    continue;
                }
            };
            if !self.current(generation) {
                return Ok(());
            }
            let low = info.get("low_integrity") == Some(&json!(true));
            self.inner.st.lock().info = Some(info);
            // Der Starter bricht ab, wenn er das Terminal nicht schützen kann; die App prüft es trotzdem selbst.
            if !low {
                bail!("Das Terminal läuft nicht mit niedriger Integritätsstufe und wurde darum nicht freigegeben.");
            }
            self.set_state("läuft", "");
            let me = self.clone();
            *self.inner.health_timer.lock() = Some(tokio::spawn(async move {
                let mut tick = tokio::time::interval(Duration::from_secs(10));
                tick.tick().await; // setInterval wartet zuerst das Intervall ab
                loop {
                    tick.tick().await;
                    me.check_health(generation).await;
                }
            }));
            return Ok(());
        }
        bail!("Das Terminal antwortet nicht.")
    }

    /// Eine einzelne verpasste Probe (Laptop war kurz im Ruhezustand, ein Befehl lastet den Rechner aus) ist kein
    /// Ausfall. Antwortet das Terminal später wieder, gilt es wieder als laufend, ohne dass die Person neu starten muss.
    pub async fn check_health(&self, generation: u64) {
        {
            let st = self.inner.st.lock();
            if !self.current(generation) || (st.state != "läuft" && !st.unhealthy) {
                return;
            }
        }
        let port = self.port();
        match get_json(port, "/health", None, 3000).await {
            Ok(_) => {
                if !self.current(generation) {
                    return;
                }
                let recovered = {
                    let mut st = self.inner.st.lock();
                    st.misses = 0;
                    st.unhealthy && self.inner.backend.running()
                };
                if recovered {
                    self.inner.st.lock().unhealthy = false;
                    self.set_state("läuft", "");
                }
            }
            Err(_) => {
                if !self.current(generation) || self.state() != "läuft" {
                    return;
                }
                let failed = {
                    let mut st = self.inner.st.lock();
                    st.misses += 1;
                    if st.misses >= HEALTH_MISSES {
                        st.unhealthy = true;
                        true
                    } else {
                        false
                    }
                };
                if failed {
                    self.set_state("fehler", "Das Terminal antwortet nicht mehr.");
                }
            }
        }
    }

    pub fn stop(&self) -> impl Future<Output = ()> + Send + 'static {
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.clear_health_timer();
        self.inner.st.lock().unhealthy = false;
        let me = self.clone();
        self.enqueue(move || async move {
            me.inner.backend.stop().await;
            if me.current(generation) && me.state() != "gestoppt" {
                me.set_state("gestoppt", "");
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::github::test_http::{serve_at, Handler, Reply, Req};
    use std::sync::atomic::AtomicBool;

    // Ersatz für native_low: ein kleiner HTTP-Server, Start und Stopp dauern einen Moment
    struct FakeBackend {
        low_integrity: bool,
        server: Mutex<Option<JoinHandle<()>>>,
        starts: Mutex<Vec<String>>,
        alive: Mutex<(i32, i32)>, // (jetzt, höchstens)
        healthy: Arc<AtomicBool>,
    }

    impl FakeBackend {
        fn new(low_integrity: bool) -> Arc<Self> {
            Arc::new(FakeBackend {
                low_integrity,
                server: Mutex::new(None),
                starts: Mutex::new(Vec::new()),
                alive: Mutex::new((0, 0)),
                healthy: Arc::new(AtomicBool::new(true)),
            })
        }
    }

    #[async_trait]
    impl Backend for FakeBackend {
        fn running(&self) -> bool {
            self.server.lock().is_some()
        }

        async fn start(&self, o: BackendStart) -> Result<()> {
            tokio::time::sleep(Duration::from_millis(30)).await;
            let (low, healthy, workdir) = (self.low_integrity, self.healthy.clone(), o.workdir.clone());
            let handler: Handler = Arc::new(move |req: Req| {
                if !healthy.load(Ordering::SeqCst) && req.path == "/health" {
                    return Reply::Drop;
                }
                Reply::Json(200, json!({ "low_integrity": low, "workdir": workdir }).to_string())
            });
            let task = serve_at(&format!("127.0.0.1:{}", o.port), handler).await?;
            *self.server.lock() = Some(task);
            self.starts.lock().push(o.workdir);
            let mut a = self.alive.lock();
            a.0 += 1;
            a.1 = a.1.max(a.0);
            Ok(())
        }

        async fn stop(&self) {
            let Some(task) = self.server.lock().take() else { return };
            tokio::time::sleep(Duration::from_millis(30)).await;
            task.abort();
            let _ = task.await;
            self.alive.lock().0 -= 1;
        }
    }

    fn opts(folder: &str) -> StartOptions {
        StartOptions { folder: folder.into(), ..Default::default() }
    }

    #[tokio::test]
    async fn mehrere_starts_kurz_hintereinander_nur_der_letzte_laeuft_kein_terminal_bleibt_uebrig() {
        let backend = FakeBackend::new(true);
        let t = Terminal::new(backend.clone());
        tokio::join!(t.start(opts("A")), t.start(opts("B")), t.start(opts("C")));
        assert_eq!(t.state(), "läuft");
        assert_eq!(t.folder(), "C");
        assert_eq!(*backend.starts.lock(), ["C"]);
        t.start(opts("D")).await;
        drop(t.start(opts("E"))); // läuft trotzdem an und wird gleich wieder gestoppt
        t.stop().await;
        assert_eq!(t.state(), "gestoppt");
        assert_eq!(backend.alive.lock().0, 0);
        assert_eq!(backend.alive.lock().1, 1);
        t.stop().await;
    }

    #[tokio::test]
    async fn terminal_ohne_niedrige_integritaetsstufe_wird_nicht_freigegeben() {
        let t = Terminal::new(FakeBackend::new(false));
        t.start(opts("A")).await;
        assert_eq!(t.state(), "fehler");
        assert!(t.error().contains("niedriger Integritätsstufe"));
        t.stop().await;
    }

    #[tokio::test]
    async fn ueberwachung_eine_verpasste_probe_ist_kein_ausfall_nach_drei_gilt_es_als_ausgefallen_und_erholt_sich() {
        let backend = FakeBackend::new(true);
        let t = Terminal::new(backend.clone());
        t.start(opts("A")).await;
        let generation = t.generation();
        backend.healthy.store(false, Ordering::SeqCst);
        t.check_health(generation).await;
        t.check_health(generation).await;
        assert_eq!(t.state(), "läuft");
        t.check_health(generation).await;
        assert_eq!(t.state(), "fehler");
        backend.healthy.store(true, Ordering::SeqCst);
        t.check_health(generation).await;
        assert_eq!(t.state(), "läuft");
        t.stop().await;
    }

    #[tokio::test]
    async fn prepare_meldet_zwischenstand_und_ereignisse_state_und_port() {
        let backend = FakeBackend::new(true);
        let t = Terminal::new(backend);
        let details = Arc::new(Mutex::new(Vec::<String>::new()));
        let d = details.clone();
        t.on("state", move |v| d.lock().push(v["detail"].as_str().unwrap_or("").to_string()));
        let ports = Arc::new(Mutex::new(Vec::<u64>::new()));
        let p = ports.clone();
        t.on("port", move |v| p.lock().push(v.as_u64().unwrap()));
        t.set_prepare(Some(Arc::new(|report: ReportFn| {
            Box::pin(async move {
                report("Python wird eingerichtet … 40 %".into());
                Ok(())
            })
        })));
        t.start(opts("A")).await;
        assert_eq!(t.state(), "läuft");
        assert!(details.lock().iter().any(|d| d.contains("40 %")));
        assert_eq!(ports.lock().len(), 1);
        assert_eq!(t.detail(), "");
        assert_eq!(ports.lock()[0], t.port() as u64);
        assert_eq!(t.url(), format!("http://127.0.0.1:{}", t.port()));
        assert_eq!(t.snapshot()["lowIntegrity"], true);
        t.stop().await;
    }

    #[tokio::test]
    async fn fehler_in_prepare_gibt_fehler_mit_text() {
        let t = Terminal::new(FakeBackend::new(true));
        t.set_prepare(Some(Arc::new(|_r: ReportFn| Box::pin(async { Err(anyhow!("Python konnte nicht entpackt werden")) }))));
        t.start(opts("A")).await;
        assert_eq!(t.state(), "fehler");
        assert_eq!(t.error(), "Python konnte nicht entpackt werden");
        assert!(t.logs().iter().any(|l| l.starts_with("Start fehlgeschlagen:")));
    }

    #[tokio::test]
    async fn port_wird_gewaehlt_und_belegter_wunschport_uebergangen() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let busy = l.local_addr().unwrap().port();
        assert!(!is_port_free(busy).await);
        let p = choose_port(Some(busy)).await.unwrap();
        assert_ne!(p, busy);
        assert!((PORT_MIN..PORT_MAX).contains(&p));
    }
}
