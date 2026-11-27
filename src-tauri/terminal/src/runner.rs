//! Befehle ausführen: Prozesse starten, Ausgabe puffern, Prozessbaum beenden
//! (Port von open_terminal/utils/runner.py `PipeRunner`, utils/log.py und den Anpassungen in server.py).
//!
//! - Unter Windows läuft jeder Befehl über `cmd.exe /d /s /c` ohne ConPTY, in einem eigenen Job-Objekt
//!   («Kill on Job Close»); Abbrechen beendet den ganzen Prozessbaum. Die Konsole des Befehls steht auf
//!   UTF-8 (`chcp 65001`), Zeilen in der OEM-Codepage werden trotzdem umgewandelt.
//! - Sonst `sh -c` in einer eigenen Prozessgruppe (Entwicklung und Tests).
//! - Die Ausgabe (stdout und stderr getrennt gekennzeichnet) liegt im Speicher; über 50 MB wird die erste Hälfte
//!   verworfen wie bei der Protokolldatei von open-terminal, die Zählung (`offset`) bleibt absolut.
//!
//! Öffentliche Schnittstelle:
//! - `Registry`: `spawn`, `get`, `cleanup_expired`, `kill_all`
//! - `Process`: `id`, `command`, `status`, `exit_code`, `wait_done(zeit)`, `output(offset, tail)`, `kill(force)`

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde_json::{json, Value};
use tokio::process::Command;
use tokio::sync::watch;

use crate::streams::{decode_output, read_lines, Oem, CHUNK};

/// Fertige Prozesse bleiben so lange abrufbar (`_EXPIRY_SECONDS`)
const EXPIRY: Duration = Duration::from_secs(300);
/// Grösse des Protokolls, ab der die erste Hälfte verworfen wird (`MAX_PROCESS_LOG_SIZE`)
const MAX_LOG_BYTES: usize = 50_000_000;

struct Entry {
    kind: &'static str,
    data: String,
}

#[derive(Default)]
struct Log {
    entries: Vec<Entry>,
    /// absolute Nummer des ersten gespeicherten Eintrags
    base: usize,
    bytes: usize,
    rotated: bool,
}

impl Log {
    fn push(&mut self, kind: &'static str, data: String) {
        if self.bytes + data.len() > MAX_LOG_BYTES && !self.entries.is_empty() {
            let drop = self.entries.len().div_ceil(2);
            let removed: usize = self.entries.drain(..drop).map(|e| e.data.len()).sum();
            self.base += drop;
            self.bytes -= removed;
            self.rotated = true;
        }
        self.bytes += data.len();
        self.entries.push(Entry { kind, data });
    }
}

struct State {
    status: String,
    exit_code: Option<i32>,
    finished_at: Option<Instant>,
}

pub struct Process {
    pub id: String,
    pub command: String,
    pub cwd: String,
    state: Mutex<State>,
    log: Mutex<Log>,
    done: watch::Receiver<bool>,
    #[cfg_attr(windows, allow(dead_code))]
    pid: u32,
    #[cfg(windows)]
    job: Mutex<Option<crate::winproc::Job>>,
}

/// Die Ausgabe ab `offset` wie in `log_response`: Einträge, nächster Offset, gekürzt?
pub struct Output {
    pub entries: Vec<Value>,
    pub next_offset: usize,
    pub truncated: bool,
}

impl Process {
    pub fn status(&self) -> String {
        self.state.lock().status.clone()
    }

    pub fn exit_code(&self) -> Option<i32> {
        self.state.lock().exit_code
    }

    pub fn is_running(&self) -> bool {
        self.state.lock().status == "running"
    }

    /// Wartet, bis die Ausgabe zu Ende gelesen und der Prozess beendet ist, höchstens `limit`
    pub async fn wait_done(&self, limit: Duration) {
        let mut rx = self.done.clone();
        let wait = async {
            while !*rx.borrow_and_update() {
                if rx.changed().await.is_err() {
                    break;
                }
            }
        };
        let _ = tokio::time::timeout(limit, wait).await;
    }

    pub fn output(&self, offset: usize, tail: Option<usize>) -> Output {
        let log = self.log.lock();
        let total = log.base + log.entries.len();
        let skip_to = offset.saturating_sub(log.base).min(log.entries.len());
        let selected = &log.entries[skip_to..];
        let skip = tail.map(|t| selected.len().saturating_sub(t)).unwrap_or(0);
        let truncated = (log.rotated && offset < log.base) || skip > 0;
        let entries = selected[skip..].iter().map(|e| json!({ "type": e.kind, "data": e.data })).collect();
        Output { entries, next_offset: total, truncated }
    }

    /// Antwort für `/execute` (mit `cwd`) und `/execute/{id}/status` (ohne)
    pub fn response(&self, with_cwd: bool, offset: usize, tail: Option<usize>) -> Value {
        let out = self.output(offset, tail);
        let (status, exit_code) = {
            let s = self.state.lock();
            (s.status.clone(), s.exit_code)
        };
        let mut m = serde_json::Map::new();
        m.insert("id".into(), json!(self.id));
        m.insert("command".into(), json!(self.command));
        if with_cwd {
            m.insert("cwd".into(), json!(self.cwd));
        }
        m.insert("status".into(), json!(status));
        m.insert("exit_code".into(), json!(exit_code));
        m.insert("log_path".into(), Value::Null);
        m.insert("output".into(), Value::Array(out.entries));
        m.insert("next_offset".into(), json!(out.next_offset));
        m.insert("truncated".into(), json!(out.truncated));
        Value::Object(m)
    }

    /// Beendet den ganzen Prozessbaum. Windows: Job-Objekt; sonst die Prozessgruppe (SIGTERM, ohne `force`
    /// nach 3 s SIGKILL).
    pub fn kill(self: &Arc<Self>, force: bool) {
        #[cfg(windows)]
        {
            let _ = force;
            if let Some(job) = self.job.lock().as_ref() {
                job.terminate(1);
            }
        }
        #[cfg(unix)]
        {
            let pid = self.pid as i32;
            unsafe {
                libc::kill(-pid, if force { libc::SIGKILL } else { libc::SIGTERM });
            }
            if !force {
                let me = self.clone();
                tokio::spawn(async move {
                    me.wait_done(Duration::from_secs(3)).await;
                    if me.is_running() || !*me.done.borrow() {
                        unsafe {
                            libc::kill(-pid, libc::SIGKILL);
                        }
                    }
                });
            }
        }
    }

    fn mark_killed(&self) {
        let mut s = self.state.lock();
        if s.status == "running" {
            s.status = "killed".into();
        }
    }
}

pub struct Registry {
    procs: Mutex<HashMap<String, Arc<Process>>>,
    #[cfg(windows)]
    kept_jobs: Mutex<Vec<crate::winproc::Job>>,
}

impl Registry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { procs: Mutex::new(HashMap::new()), #[cfg(windows)] kept_jobs: Mutex::new(Vec::new()) })
    }

    pub fn get(&self, id: &str) -> Option<Arc<Process>> {
        self.procs.lock().get(id).cloned()
    }

    /// Fertige Prozesse nach fünf Minuten vergessen. Läuft unter Windows noch etwas im Job (ein abgekoppelter
    /// Hintergrundprozess), bleibt der Job offen, damit es nicht mit dem Eintrag verschwindet.
    pub fn cleanup_expired(&self) {
        let expired: Vec<Arc<Process>> = {
            let mut procs = self.procs.lock();
            let ids: Vec<String> = procs.iter().filter(|(_, p)| p.state.lock().finished_at.is_some_and(|t| t.elapsed() > EXPIRY)).map(|(id, _)| id.clone()).collect();
            ids.into_iter().filter_map(|id| procs.remove(&id)).collect()
        };
        #[cfg(windows)]
        {
            for p in expired {
                if let Some(job) = p.job.lock().take() {
                    if job.active_processes() > 0 {
                        self.kept_jobs.lock().push(job);
                    }
                }
            }
            self.kept_jobs.lock().retain(|j| j.active_processes() > 0);
        }
        #[cfg(not(windows))]
        drop(expired);
    }

    /// Alles beenden (Server endet)
    pub fn kill_all(&self) {
        let all: Vec<Arc<Process>> = self.procs.lock().values().cloned().collect();
        for p in all {
            if !*p.done.borrow() {
                p.mark_killed();
                p.kill_now();
            }
        }
        #[cfg(windows)]
        for job in self.kept_jobs.lock().iter() {
            job.terminate(1);
        }
    }

    /// Startet einen Befehl. `env`: zusätzliche Umgebungsvariablen (sonst die des Servers).
    pub async fn spawn(&self, command: &str, cwd: &Path, env: Option<&HashMap<String, String>>) -> std::io::Result<Arc<Process>> {
        let mut cmd = shell_command(command);
        cmd.current_dir(cwd).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        if let Some(extra) = env {
            cmd.envs(extra);
        }
        let mut child = cmd.spawn()?;
        let pid = child.id().unwrap_or(0);
        #[cfg(windows)]
        let job = {
            // Der Prozess läuft angehalten und kommt vor dem ersten Befehl in den Job: nichts entkommt.
            let attach = || -> std::io::Result<crate::winproc::Job> {
                let job = crate::winproc::Job::new()?;
                let handle = child.raw_handle().ok_or_else(|| std::io::Error::other("Prozess-Handle fehlt"))?;
                job.assign(handle as _)?;
                crate::winproc::resume_process(pid)?;
                Ok(job)
            };
            match attach() {
                Ok(job) => job,
                Err(e) => {
                    let _ = child.start_kill();
                    return Err(e);
                }
            }
        };
        let stdin = child.stdin.take();
        let mut stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");
        let (tx, rx) = watch::channel(false);
        let id = new_id();
        let process = Arc::new(Process {
            id: id.clone(),
            command: command.to_string(),
            cwd: cwd.to_string_lossy().to_string(),
            state: Mutex::new(State { status: "running".into(), exit_code: None, finished_at: None }),
            log: Mutex::new(Log::default()),
            done: rx,
            pid,
            #[cfg(windows)]
            job: Mutex::new(Some(job)),
        });
        self.procs.lock().insert(id, process.clone());
        let p = process.clone();
        tokio::spawn(async move {
            let _stdin = stdin; // offen halten, wie open-terminal (Eingaben gibt es nicht)
            let oem = Oem::system();
            let (p1, p2) = (p.clone(), p.clone());
            let out = async move {
                let _ = read_lines(&mut stdout, |line| p1.log.lock().push("stdout", decode_output(&line, &oem)), CHUNK).await;
            };
            let err = async move {
                let _ = read_lines(&mut stderr, |line| p2.log.lock().push("stderr", decode_output(&line, &oem)), CHUNK).await;
            };
            tokio::join!(out, err);
            let status = child.wait().await;
            let code = status.ok().map(exit_code_of);
            {
                let mut s = p.state.lock();
                s.exit_code = code;
                if s.status == "running" {
                    s.status = "done".into();
                }
                s.finished_at = Some(Instant::now());
            }
            let _ = tx.send(true);
        });
        Ok(process)
    }
}

impl Process {
    fn kill_now(self: &Arc<Self>) {
        self.kill(true);
    }

    /// `DELETE /execute/{id}`: als beendet kennzeichnen, Baum beenden, auf das Ende warten
    pub async fn kill_and_wait(self: &Arc<Self>, force: bool) {
        if self.is_running() {
            self.mark_killed();
            self.kill(force);
            self.wait_done(Duration::from_secs(10)).await;
        }
        let mut s = self.state.lock();
        if s.finished_at.is_none() {
            s.finished_at = Some(Instant::now());
        }
    }
}

fn exit_code_of(status: std::process::ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return -sig;
        }
    }
    status.code().unwrap_or(-1)
}

fn new_id() -> String {
    use rand::Rng;
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil(days);
    let suffix: u32 = rand::rng().random_range(0..0x100_0000);
    format!("{y:04}{m:02}{d:02}-{:02}{:02}{:02}-{suffix:06x}", rem / 3600, rem / 60 % 60, rem % 60)
}

fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// Die Shell für einen Befehl: `cmd.exe /d /s /c` (Windows) oder `sh -c`
fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        let comspec = std::env::var("COMSPEC").unwrap_or_else(|_| "cmd.exe".into());
        let mut cmd = Command::new(comspec);
        // /d: keine AutoRun-Befehle; /s: Anführungszeichen um den ganzen Befehl wie bei Python («shell=True»).
        // chcp 65001: Die eigene (versteckte) Konsole des Befehls schreibt UTF-8, auch cmd selbst.
        cmd.raw_arg(format!("/d /s /c \"chcp 65001>nul & {command}\""));
        cmd.creation_flags(crate::winproc::CREATE_SUSPENDED | crate::winproc::CREATE_NO_WINDOW);
        cmd.kill_on_drop(false);
        cmd
    }
    #[cfg(not(windows))]
    {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command).process_group(0).kill_on_drop(false);
        cmd
    }
}
