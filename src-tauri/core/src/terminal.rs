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
//! - `native_low::NativeLowBackend`: startet die App mit `--terminal` (Terminal-Server in Rust, Crate `ask-terminal`)
//!   mit Low Integrity im Job-Objekt, Key über stdin; `with_executable` tauscht das Programm (Tests), `build_env`

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

/// Backend «nativ mit Low Integrity»: Die App startet sich selbst mit `--terminal` (Terminal-Server in Rust,
/// Crate `ask-terminal`). Der Starter, der früher in Python lief (launcher.py), steckt hier: Arbeits- und
/// Laufzeitordner mit Low kennzeichnen, den Server angehalten starten, sein Token auf Low absenken, ihn in ein
/// Job-Objekt legen und fortsetzen. Der Key geht über stdin, nie über Kommandozeile oder Datei. Schliesst die App
/// stdin (oder stürzt ab, dann schliesst Windows das Job-Handle), endet der Server mit allen Kindprozessen.
pub mod native_low {
    use super::*;
    use std::path::{Path, PathBuf};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::process::{ChildStdin, Command};
    use tokio::sync::{watch, Notify};

    /// Umgebungsvariablen, die nicht an das Modell weitergegeben werden.
    const DROP_ENV: [&str; 8] = ["ELECTRON_RUN_AS_NODE", "ELECTRON_NO_ATTACH_CONSOLE", "NODE_OPTIONS", "PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP", "VIRTUAL_ENV", "OPEN_TERMINAL_API_KEY"];

    #[cfg(windows)]
    type JobHandle = Arc<ask_terminal::winproc::Job>;
    /// Ausserhalb von Windows gibt es kein Job-Objekt
    #[cfg(not(windows))]
    #[derive(Clone)]
    struct JobHandle;

    struct Running {
        id: u64,
        stdin: Option<ChildStdin>,
        kill: Arc<Notify>,
        exited: watch::Receiver<bool>,
        #[cfg_attr(not(windows), allow(dead_code))]
        job: JobHandle,
    }

    pub struct NativeLowBackend {
        cur: Arc<Mutex<Option<Running>>>,
        next_id: AtomicU64,
        executable: Option<PathBuf>,
        low_integrity: bool,
        execute_timeout: f64,
    }

    impl Default for NativeLowBackend {
        fn default() -> Self {
            NativeLowBackend { cur: Arc::new(Mutex::new(None)), next_id: AtomicU64::new(0), executable: None, low_integrity: true, execute_timeout: 60.0 }
        }
    }

    impl NativeLowBackend {
        pub fn new() -> Self {
            Self::default()
        }

        /// Statt der laufenden App ein anderes Programm als Server starten (Tests: gebaute App oder Attrappe)
        pub fn with_executable(mut self, path: impl Into<PathBuf>) -> Self {
            self.executable = Some(path.into());
            self
        }

        /// false: ohne Kennzeichnung und ohne Absenken des Tokens (nur Tests)
        pub fn with_low_integrity(mut self, low: bool) -> Self {
            self.low_integrity = low;
            self
        }

        /// Nur Tests: stdin des Servers schliessen, wie bei einem Absturz der App
        #[cfg(test)]
        pub fn close_stdin_for_test(&self) {
            if let Some(r) = self.cur.lock().as_mut() {
                r.stdin.take();
            }
        }

        /// Wartezeit für Befehle ohne eigene Angabe (Sekunden)
        pub fn with_execute_timeout(mut self, seconds: f64) -> Self {
            self.execute_timeout = seconds;
            self
        }
    }

    /// Umgebung für den Server und seine Befehle (wie `build_env` im Starter): ohne die Variablen aus `DROP_ENV`,
    /// mit dem mitgelieferten Python im PATH und allen Schreibzielen im Laufzeitordner.
    pub fn build_env(runtime: &Path, python_dir: &Path, base: impl IntoIterator<Item = (String, String)>) -> Vec<(String, String)> {
        // Windows kennt Variablen ohne Gross-/Kleinschreibung (Python macht die Namen gross); sonst unverändert
        let norm = |k: &str| if cfg!(windows) { k.to_uppercase() } else { k.to_string() };
        let mut env: std::collections::BTreeMap<String, String> = base
            .into_iter()
            .filter(|(k, _)| !DROP_ENV.contains(&k.to_uppercase().as_str()) && !k.to_uppercase().starts_with("ELECTRON_"))
            .map(|(k, v)| (norm(&k), v))
            .collect();
        let user_base = runtime.join("python-benutzer");
        let mut path: Vec<PathBuf> = vec![python_dir.to_path_buf(), python_dir.join("Scripts"), user_base.join("Python314").join("Scripts")];
        path.extend(std::env::split_paths(&env.get("PATH").cloned().unwrap_or_default()));
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "nutzer".into());
        let s = |p: PathBuf| p.to_string_lossy().to_string();
        let set: Vec<(&str, String)> = vec![
            ("PATH", std::env::join_paths(path).map(|p| p.to_string_lossy().to_string()).unwrap_or_default()),
            ("TEMP", s(runtime.join("tmp"))),
            ("TMP", s(runtime.join("tmp"))),
            ("SHELL", "cmd.exe".into()),
            ("USER", user),
            ("PYTHONUTF8", "1".into()),
            ("PYTHONIOENCODING", "utf-8".into()),
            ("PYTHONUNBUFFERED", "1".into()),
            ("PYTHONDONTWRITEBYTECODE", "1".into()),
            ("PYTHONUSERBASE", s(user_base)),
            // Kein PIP_USER: In einer .venv scheitert `pip install` damit («Can not perform a '--user' install»).
            // Ist site-packages nicht beschreibbar (Low Integrity), weicht pip selbst auf eine Benutzerinstallation
            // aus; sie landet dank PYTHONUSERBASE in runtime/python-benutzer.
            ("PIP_DISABLE_PIP_VERSION_CHECK", "1".into()),
            ("PIP_NO_WARN_SCRIPT_LOCATION", "1".into()),
            ("MPLCONFIGDIR", s(runtime.join("matplotlib"))),
            ("MPLBACKEND", "Agg".into()),
        ];
        for (k, v) in set {
            env.insert(k.to_string(), v);
        }
        env.into_iter().collect()
    }

    fn event(ev: &EventFn, name: &str, fields: Value) {
        let mut m = serde_json::Map::new();
        m.insert("event".into(), json!(name));
        if let Value::Object(o) = fields {
            m.extend(o);
        }
        ev(Value::Object(m));
    }

    /// Wie der Starter bei einem Fehler: Meldung, dann Ende mit Exitcode
    fn fail(ev: &EventFn, code: i32, fields: Value) {
        event(ev, "error", fields);
        event(ev, "exit", json!({ "code": code }));
        event(ev, "process-exit", json!({ "code": code }));
    }

    #[async_trait]
    impl Backend for NativeLowBackend {
        fn running(&self) -> bool {
            self.cur.lock().is_some()
        }

        async fn start(&self, o: BackendStart) -> Result<()> {
            let ev = o.on_event.clone();
            event(&ev, "launcher", json!({ "pid": std::process::id() }));
            let workdir = ask_terminal::guard::realpath(Path::new(&o.workdir));
            if !workdir.is_dir() {
                fail(&ev, 3, json!({ "code": "workdir", "message": format!("Ordner nicht gefunden: {}", workdir.display()) }));
                return Ok(());
            }
            let runtime = crate::paths::runtime_dir();
            for sub in ["tmp", "matplotlib", "python-benutzer"] {
                if let Err(err) = std::fs::create_dir_all(runtime.join(sub)) {
                    fail(&ev, 5, json!({ "code": "spawn", "message": format!("Laufzeitordner nicht erstellbar: {err}") }));
                    return Ok(());
                }
            }
            let low = self.low_integrity && cfg!(windows);
            #[cfg(windows)]
            if low {
                for folder in [workdir.clone(), runtime.clone()] {
                    let f = folder.clone();
                    let (ok, message) = tokio::task::spawn_blocking(move || ask_terminal::winproc::ensure_low_label(&f)).await.unwrap_or((false, "Abbruch".into()));
                    if !ok {
                        fail(&ev, 4, json!({ "code": "label", "path": folder.to_string_lossy(), "message": message }));
                        return Ok(());
                    }
                }
                event(&ev, "label", json!({ "ok": true }));
            }

            let exe = match &self.executable {
                Some(p) => p.clone(),
                None => match std::env::current_exe() {
                    Ok(p) => p,
                    Err(err) => {
                        fail(&ev, 5, json!({ "code": "spawn", "message": err.to_string() }));
                        return Ok(());
                    }
                },
            };
            let python_dir = crate::paths::python_exe().parent().map(Path::to_path_buf).unwrap_or_default();
            let env = build_env(&runtime, &python_dir, std::env::vars());
            let mut cmd = Command::new(&exe);
            cmd.arg("--terminal")
                .env_clear()
                .envs(env)
                .current_dir(&workdir)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            #[cfg(windows)]
            cmd.creation_flags(ask_terminal::winproc::CREATE_SUSPENDED | ask_terminal::winproc::CREATE_NO_WINDOW);
            let mut child = match cmd.spawn() {
                Ok(c) => c,
                Err(err) => {
                    fail(&ev, 5, json!({ "code": "spawn", "message": err.to_string() }));
                    return Ok(());
                }
            };
            let pid = child.id().unwrap_or(0);
            // Angehalten: erst Token und Job, dann läuft der Server los (er hatte nie höhere Rechte im Einsatz)
            #[cfg(windows)]
            let job: JobHandle = {
                let attach = || -> std::io::Result<JobHandle> {
                    let job = ask_terminal::winproc::Job::new()?;
                    let handle = child.raw_handle().ok_or_else(|| std::io::Error::other("Prozess-Handle fehlt"))?;
                    if low {
                        ask_terminal::winproc::lower_token_to_low(handle as _)?;
                    }
                    job.assign(handle as _)?;
                    ask_terminal::winproc::resume_process(pid)?;
                    Ok(Arc::new(job))
                };
                match attach() {
                    Ok(j) => j,
                    Err(err) => {
                        let _ = child.start_kill();
                        fail(&ev, 5, json!({ "code": "spawn", "message": err.to_string() }));
                        return Ok(());
                    }
                }
            };
            #[cfg(not(windows))]
            let job = JobHandle;

            let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
            let mut stdin = child.stdin.take().expect("stdin");
            let stdout = child.stdout.take().expect("stdout");
            let stderr = child.stderr.take().expect("stderr");
            let cfg = json!({
                "key": o.key,
                "port": o.port,
                "workdir": workdir.to_string_lossy(),
                "origin": o.origin.clone().unwrap_or_default(),
                "execute_timeout": self.execute_timeout,
                "low_integrity": low || (!cfg!(windows) && self.low_integrity),
            });
            // stdin bleibt offen: Endet die App, endet stdin, und der Server beendet sich
            let _ = stdin.write_all(format!("{cfg}\n").as_bytes()).await;
            let _ = stdin.flush().await;
            event(&ev, "started", json!({ "pid": pid, "low_integrity": low }));

            let e1 = ev.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    let text = line.trim_end().to_string();
                    if text.starts_with('{') {
                        if let Ok(Value::Object(m)) = serde_json::from_str::<Value>(&text) {
                            if m.contains_key("event") {
                                e1(Value::Object(m));
                                continue;
                            }
                        }
                    }
                    event(&e1, "log", json!({ "line": text }));
                }
            });
            let e2 = ev.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    event(&e2, "log", json!({ "line": line.trim_end() }));
                }
            });

            let kill = Arc::new(Notify::new());
            let (exited_tx, exited_rx) = watch::channel(false);
            *self.cur.lock() = Some(Running { id, stdin: Some(stdin), kill: kill.clone(), exited: exited_rx, job: job.clone() });
            let cur = self.cur.clone();
            tokio::spawn(async move {
                let status = tokio::select! {
                    s = child.wait() => s,
                    _ = kill.notified() => {
                        let _ = child.kill().await;
                        child.wait().await
                    }
                };
                // Reste im Job beenden (wie `job.terminate()` im Starter)
                #[cfg(windows)]
                job.terminate(1);
                #[cfg(not(windows))]
                let _ = job;
                {
                    let mut c = cur.lock();
                    if c.as_ref().is_some_and(|r| r.id == id) {
                        *c = None;
                    }
                }
                let _ = exited_tx.send(true);
                let code = status.ok().and_then(|s| s.code());
                event(&ev, "exit", json!({ "code": code }));
                event(&ev, "process-exit", json!({ "code": code }));
            });
            Ok(())
        }

        /// Bittet den Server zu enden («stop», dann stdin schliessen); nach 5 s wird er hart beendet, und der Job
        /// räumt alle Kindprozesse ab.
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
                #[cfg(windows)]
                r.job.terminate(1);
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

    /// Ein Laufzeitordner für alle Tests (die Umgebungsvariable gilt für den ganzen Prozess)
    fn shared_runtime() -> std::path::PathBuf {
        static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
        let dir = DIR.get_or_init(|| {
            let d = tempfile::tempdir().unwrap();
            std::env::set_var("ASK_RUNTIME_DIR", d.path());
            d
        });
        dir.path().to_path_buf()
    }

    // ---------- Starter (ersetzt tests/test_terminal.py::CleanupTest und launcher.py) ----------

    #[test]
    fn umgebung_wie_build_env_im_starter() {
        use native_low::build_env;
        let runtime = std::path::Path::new("/laufzeit");
        let python = std::path::Path::new("/py");
        let base = [
            ("PATH", "/usr/bin"),
            ("ELECTRON_RUN_AS_NODE", "1"),
            ("electron_irgendwas", "1"),
            ("NODE_OPTIONS", "--x"),
            ("PYTHONPATH", "/x"),
            ("PYTHONHOME", "/x"),
            ("PYTHONSTARTUP", "/x"),
            ("VIRTUAL_ENV", "/x"),
            ("OPEN_TERMINAL_API_KEY", "geheim"),
            ("MEINE", "bleibt"),
        ];
        let env: std::collections::HashMap<String, String> = build_env(runtime, python, base.iter().map(|(k, v)| (k.to_string(), v.to_string()))).into_iter().collect();
        for dropped in ["ELECTRON_RUN_AS_NODE", "electron_irgendwas", "NODE_OPTIONS", "PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP", "VIRTUAL_ENV", "OPEN_TERMINAL_API_KEY"] {
            assert!(!env.contains_key(dropped), "{dropped} muss fehlen");
        }
        assert!(!env.keys().any(|k| k.starts_with("OPEN_TERMINAL_")), "keine OPEN_TERMINAL_*-Variablen mehr");
        assert_eq!(env["MEINE"], "bleibt");
        let sep = if cfg!(windows) { ';' } else { ':' };
        let path: Vec<&str> = env["PATH"].split(sep).collect();
        assert_eq!(path[0], "/py", "Python zuerst");
        assert!(path[1].ends_with("Scripts"));
        assert!(path[2].ends_with("Scripts") && path[2].contains("python-benutzer"));
        assert_eq!(*path.last().unwrap(), "/usr/bin");
        for (k, v) in [("PYTHONUTF8", "1"), ("PYTHONIOENCODING", "utf-8"), ("PYTHONUNBUFFERED", "1"), ("PYTHONDONTWRITEBYTECODE", "1"), ("MPLBACKEND", "Agg"), ("SHELL", "cmd.exe"), ("PIP_DISABLE_PIP_VERSION_CHECK", "1"), ("PIP_NO_WARN_SCRIPT_LOCATION", "1")] {
            assert_eq!(env[k], v, "{k}");
        }
        assert!(!env.contains_key("PIP_USER"), "kein PIP_USER");
        assert!(env["TEMP"].contains("tmp") && env["TMP"] == env["TEMP"]);
        assert!(env["PYTHONUSERBASE"].contains("python-benutzer"));
        assert!(env["MPLCONFIGDIR"].contains("matplotlib"));
    }

    /// Eine Attrappe statt der App: liest die Einstellungen, meldet sich wie der Server und endet bei «stop» oder
    /// wenn stdin endet. So lässt sich der Starter (Einstellungen, Ereignisse, Stoppen) ohne gebaute App prüfen.
    #[cfg(unix)]
    fn fake_server(dir: &std::path::Path) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let script = "#!/bin/sh\nread cfg\necho \"{\\\"event\\\":\\\"server\\\",\\\"pid\\\":$$}\"\necho \"cfg:$cfg\"\necho \"fehler\" >&2\n\
            echo \"umgebung:$PYTHONUTF8:${ELECTRON_X:-weg}:$MEINE_VAR\"\necho \"cwd:$(pwd)\"\nwhile read cmd; do [ \"$cmd\" = stop ] && exit 7; done\nexit 8\n";
        let path = dir.join("attrappe.sh");
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[cfg(unix)]
    async fn collect_events(backend: &native_low::NativeLowBackend, workdir: &str) -> Arc<Mutex<Vec<Value>>> {
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let e = events.clone();
        backend
            .start(BackendStart {
                workdir: workdir.into(),
                port: 4711,
                key: "schluessel".into(),
                origin: Some("https://ask.example.ch".into()),
                system_prompt: None,
                execute_description: None,
                on_event: Arc::new(move |v| e.lock().push(v)),
            })
            .await
            .unwrap();
        events
    }

    #[cfg(unix)]
    async fn wait_for(events: &Arc<Mutex<Vec<Value>>>, name: &str) -> Value {
        for _ in 0..100 {
            if let Some(v) = events.lock().iter().find(|v| v["event"] == name) {
                return v.clone();
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("Ereignis {name} kam nicht: {:?}", events.lock());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn starter_schickt_einstellungen_ueber_stdin_leitet_ereignisse_weiter_und_stoppt() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = shared_runtime();
        let work = dir.path().join("Arbeitsordner Prüfung");
        std::fs::create_dir_all(&work).unwrap();
        let backend = native_low::NativeLowBackend::new().with_executable(fake_server(dir.path())).with_execute_timeout(12.0);
        let events = collect_events(&backend, &work.to_string_lossy()).await;
        let server = wait_for(&events, "server").await;
        assert!(server["pid"].as_u64().unwrap() > 0);
        assert!(backend.running());
        let cfg = loop {
            if let Some(line) = events.lock().iter().find_map(|v| v["line"].as_str().and_then(|l| l.strip_prefix("cfg:")).map(str::to_string)) {
                break serde_json::from_str::<Value>(&line).unwrap();
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        };
        assert_eq!(cfg["key"], "schluessel");
        assert_eq!(cfg["port"], 4711);
        assert_eq!(cfg["origin"], "https://ask.example.ch");
        assert_eq!(cfg["execute_timeout"], 12.0);
        assert!(cfg["workdir"].as_str().unwrap().ends_with("Arbeitsordner Prüfung"), "Umlaut bleibt erhalten: {cfg}");
        for sub in ["tmp", "matplotlib", "python-benutzer"] {
            assert!(runtime.join(sub).is_dir(), "{sub}");
        }
        let lines: Vec<String> = events.lock().iter().filter(|v| v["event"] == "log").map(|v| v["line"].as_str().unwrap().to_string()).collect();
        assert!(lines.contains(&"fehler".to_string()), "stderr kommt als log: {lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("umgebung:1:weg:")), "{lines:?}");
        assert!(lines.iter().any(|l| l.starts_with("cwd:") && l.ends_with("Arbeitsordner Prüfung")), "{lines:?}");
        let names: Vec<String> = events.lock().iter().map(|v| v["event"].as_str().unwrap().to_string()).collect();
        assert_eq!(&names[..2], ["launcher", "started"]);
        // «stop» beendet den Server; Exitcode 7 beweist, dass er das Wort bekam (bei stdin-Ende wäre es 8)
        backend.stop().await;
        let exit = wait_for(&events, "exit").await;
        assert_eq!(exit["code"], 7);
        assert!(wait_for(&events, "process-exit").await["code"] == 7);
        assert!(!backend.running());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn starter_server_endet_wenn_stdin_endet() {
        let dir = tempfile::tempdir().unwrap();
        let _ = shared_runtime();
        let backend = native_low::NativeLowBackend::new().with_executable(fake_server(dir.path()));
        let events = collect_events(&backend, &dir.path().to_string_lossy()).await;
        wait_for(&events, "server").await;
        // wie ein Absturz der App: stdin geht zu, ohne «stop»
        backend.close_stdin_for_test();
        assert_eq!(wait_for(&events, "exit").await["code"], 8);
    }

    #[tokio::test]
    async fn starter_meldet_fehler_wie_der_python_starter() {
        let dir = tempfile::tempdir().unwrap();
        let _ = shared_runtime();
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let start = |workdir: String, exe: Option<std::path::PathBuf>| {
            let e = events.clone();
            let mut backend = native_low::NativeLowBackend::new();
            if let Some(exe) = exe {
                backend = backend.with_executable(exe);
            }
            async move {
                backend
                    .start(BackendStart {
                        workdir,
                        port: 1,
                        key: "k".into(),
                        origin: None,
                        system_prompt: None,
                        execute_description: None,
                        on_event: Arc::new(move |v| e.lock().push(v)),
                    })
                    .await
                    .unwrap();
                backend
            }
        };
        let backend = start(dir.path().join("gibt-es-nicht").to_string_lossy().to_string(), None).await;
        assert!(!backend.running());
        {
            let ev = events.lock();
            assert_eq!(ev[0]["event"], "launcher");
            assert_eq!((ev[1]["event"].as_str(), ev[1]["code"].as_str()), (Some("error"), Some("workdir")));
            assert!(ev[1]["message"].as_str().unwrap().starts_with("Ordner nicht gefunden:"));
            assert_eq!(ev[2]["event"], "exit");
            assert_eq!(ev[2]["code"], 3);
        }
        events.lock().clear();
        let backend = start(dir.path().to_string_lossy().to_string(), Some(dir.path().join("fehlt.exe"))).await;
        assert!(!backend.running());
        let ev = events.lock();
        assert!(ev.iter().any(|v| v["event"] == "error" && v["code"] == "spawn"), "{ev:?}");
        assert!(ev.iter().any(|v| v["event"] == "exit" && v["code"] == 5));
    }

    // ---------- Terminal und Rust-Server über HTTP, Ende zu Ende ----------

    /// Backend, das den Rust-Server im selben Prozess startet (statt als eigene App): So laufen `Terminal`, die
    /// Werkzeuge und der echte Server auf jedem System über HTTP zusammen. Mit der gebauten App (eigener Prozess,
    /// Low Integrity, Job-Objekt) läuft derselbe Ablauf in `src-tauri/tests/terminal_e2e.rs`.
    struct InProcessBackend {
        server: Mutex<Option<ask_terminal::ServerHandle>>,
    }

    #[async_trait]
    impl Backend for InProcessBackend {
        fn running(&self) -> bool {
            self.server.lock().is_some()
        }

        async fn start(&self, o: BackendStart) -> Result<()> {
            let config = ask_terminal::Config {
                key: o.key,
                port: o.port,
                workdir: o.workdir,
                origin: o.origin.unwrap_or_default(),
                execute_timeout: 60.0,
                low_integrity: true,
            };
            *self.server.lock() = Some(ask_terminal::serve(config).await?);
            Ok(())
        }

        async fn stop(&self) {
            if let Some(h) = self.server.lock().take() {
                h.shutdown();
            }
        }
    }

    #[tokio::test]
    async fn terminal_und_rust_server_ueber_http() {
        use crate::tools::{run_tool, ToolCtx};
        let dir = tempfile::tempdir().unwrap();
        let work = dir.path().join("Arbeitsordner Prüfung");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("Übung.txt"), "Grüezi").unwrap();
        let t = Terminal::new(Arc::new(InProcessBackend { server: Mutex::new(None) }));
        t.start(opts(&work.to_string_lossy())).await;
        assert_eq!(t.state(), "läuft", "{}", t.error());
        assert_eq!(t.snapshot()["lowIntegrity"], true);
        assert_eq!(t.info().unwrap()["workdir"].as_str().unwrap().to_lowercase().replace("\\\\?\\", ""), std::fs::canonicalize(&work).unwrap().to_string_lossy().to_lowercase().replace("\\\\?\\", ""));
        t.check_health(t.generation()).await;
        assert_eq!(t.state(), "läuft");

        let ctx = ToolCtx::default();
        let run = |name: &'static str, args: Value| {
            let t = t.clone();
            let ctx = ctx.clone();
            async move { run_tool(Some(&t), name, &args, &ctx).await }
        };
        let r = run("run_command", json!({ "command": "echo hallo welt" })).await;
        assert!(r.ok, "{r:?}");
        assert!(r.output.starts_with("Exitcode 0") && r.output.contains("hallo welt"), "{}", r.output);
        let r = run("run_command", json!({ "command": "exit 3" })).await;
        assert!(!r.ok && r.output.starts_with("Exitcode 3"), "{r:?}");
        let r = run("write_file", json!({ "path": "ausgabe/test.txt", "content": "eins\nzwei\n" })).await;
        assert!(r.ok, "{r:?}");
        let r = run("read_file", json!({ "path": "ausgabe/test.txt", "start_line": 2 })).await;
        assert!(r.ok && r.output.contains("zwei") && r.output.contains("2 Zeilen insgesamt"), "{r:?}");
        let r = run("edit_file", json!({ "path": "ausgabe/test.txt", "old_text": "zwei", "new_text": "2" })).await;
        assert!(r.ok, "{r:?}");
        let r = run("list_files", json!({ "path": "." })).await;
        assert!(r.ok && r.output.contains("ausgabe/") && r.output.contains("Übung.txt"), "{r:?}");
        let r = run("find_files", json!({ "pattern": "*.txt" })).await;
        assert!(r.ok && r.output.contains("test.txt"), "{r:?}");
        let r = run("search_text", json!({ "text": "GRÜEZI" })).await;
        assert!(r.ok && r.output.contains("Übung.txt"), "{r:?}");
        // die Sperre gilt auch für die Werkzeuge
        let r = run("read_file", json!({ "path": dir.path().to_string_lossy() })).await;
        assert!(!r.ok && r.output.contains("ausserhalb des Arbeitsordners"), "{r:?}");
        // ein lange laufender Befehl wird über DELETE beendet (wie beim Stoppen durch die Person)
        let (port, key) = (t.port(), t.key());
        let client = reqwest::Client::new();
        let sleeper = if cfg!(windows) { "ping -n 60 127.0.0.1 >nul" } else { "sleep 60" };
        let started: Value = client
            .post(format!("http://127.0.0.1:{port}/execute?wait=0"))
            .bearer_auth(&key)
            .json(&json!({ "command": sleeper }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(started["status"], "running");
        let killed = client.delete(format!("http://127.0.0.1:{port}/execute/{}", started["id"].as_str().unwrap())).bearer_auth(&key).send().await.unwrap();
        assert_eq!(killed.status(), 200);
        t.stop().await;
        assert_eq!(t.state(), "gestoppt");
        let mut gone = false;
        for _ in 0..40 {
            if get_json(port, "/health", None, 500).await.is_err() {
                gone = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(gone, "Server ist weg");
    }
}
