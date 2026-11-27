//! Terminal-Server von ASK Desktop: Ersatz für das Python-Paket `python/ask_terminal` (open-terminal 0.14.2
//! mit den ASK-Anpassungen), ohne Python.
//!
//! Die App startet sich selbst mit `--terminal` (Low Integrity, im Job-Objekt) und schickt die Einstellungen als eine
//! JSON-Zeile über stdin, damit der Key weder in der Kommandozeile noch in einer Datei steht. Der Server hört nur
//! auf 127.0.0.1, meldet sich mit JSON-Zeilen auf stdout (`{"event": "server", …}`) und endet, wenn stdin endet oder
//! «stop» kommt.
//!
//! Öffentliche Schnittstelle:
//! - `run_from_stdin() -> i32`: blockierend, baut eine eigene tokio-Laufzeit; Rückgabe ist der Exitcode
//! - `serve(config) -> ServerHandle` (async): derselbe Server im eigenen Prozess, z. B. für Tests
//! - `Config`: Einstellungen (`key`, `port`, `workdir`, `origin`, `execute_timeout`, `low_integrity`)
//! - Module: `guard` (Ordnersperre), `files` (Datei-API), `documents` (Text aus Dokumenten), `sources` (Quellen,
//!   ENTSCHEIDE E82), `runner` (Befehle),
//!   `streams` (Ausgabe lesen und umwandeln), `winproc` (nur Windows: Low Integrity, Job-Objekte, Label)

pub mod documents;
pub mod files;
pub mod guard;
pub mod http;
pub mod runner;
pub mod sources;
pub mod streams;
#[cfg(windows)]
pub mod winproc;

use serde::Deserialize;
use std::io::BufRead;

pub use http::{serve, ServerHandle};

/// Die Einstellungen, die der Starter als JSON-Zeile schickt. Unbekannte Felder werden überlesen.
#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub key: String,
    pub port: u16,
    pub workdir: String,
    /// Der einzige Origin, der von der Webansicht aus zugreifen darf (CORS)
    #[serde(default)]
    pub origin: String,
    /// Wartezeit in Sekunden, wenn eine Anfrage keine angibt (0: nicht warten)
    #[serde(default = "default_execute_timeout")]
    pub execute_timeout: f64,
    /// Nur ausserhalb von Windows: ob der Server «Low» meldet (Entwicklung und Tests). Unter Windows zählt das Token.
    #[serde(default = "default_true")]
    pub low_integrity: bool,
    /// Ordner nur zum Lesen (Vorlagen der Schule, ENTSCHEIDE E66)
    #[serde(default)]
    pub read_only: Vec<String>,
}

fn default_execute_timeout() -> f64 {
    60.0
}

fn default_true() -> bool {
    true
}

/// Meldung als JSON-Zeile auf stdout
pub fn emit(event: &str, fields: serde_json::Value) {
    use std::io::Write;
    let mut m = serde_json::Map::new();
    m.insert("event".into(), event.into());
    if let serde_json::Value::Object(o) = fields {
        m.extend(o);
    }
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{}", serde_json::Value::Object(m));
    let _ = out.flush();
}

/// Einstieg für `ASK.exe --terminal`: Einstellungen von stdin lesen, Server starten, bei Ende von stdin
/// oder «stop» alles beenden. Gibt den Exitcode zurück (0 normal, 2 ohne Einstellungen, 3 Ordner fehlt,
/// 6 Port nicht bindbar).
pub fn run_from_stdin() -> i32 {
    let stdin = std::io::stdin();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
        emit("error", serde_json::json!({ "code": "config", "message": "Keine Einstellungen auf stdin erhalten" }));
        return 2;
    }
    let config: Config = match serde_json::from_str(line.trim()) {
        Ok(c) => c,
        Err(e) => {
            emit("error", serde_json::json!({ "code": "config", "message": format!("Einstellungen nicht lesbar: {e}") }));
            return 2;
        }
    };
    if !std::path::Path::new(&config.workdir).is_dir() {
        emit("error", serde_json::json!({ "code": "workdir", "message": format!("Ordner nicht gefunden: {}", config.workdir) }));
        return 3;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            emit("error", serde_json::json!({ "code": "runtime", "message": e.to_string() }));
            return 6;
        }
    };
    let code = runtime.block_on(async {
        let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
        // stdin endet, wenn die App beendet wird oder abstürzt; «stop» beendet auf Wunsch
        std::thread::spawn(move || {
            for command in std::io::stdin().lock().lines() {
                match command {
                    Ok(c) if c.trim() == "stop" => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            let _ = stop_tx.send(());
        });
        let handle = match serve(config.clone()).await {
            Ok(h) => h,
            Err(e) => {
                emit("error", serde_json::json!({ "code": "bind", "message": format!("Port {} nicht verfügbar: {e}", config.port) }));
                return 6;
            }
        };
        let (integrity, _) = http::integrity_of(&config);
        emit("server", serde_json::json!({ "pid": std::process::id(), "integrity": integrity }));
        let _ = stop_rx.await;
        handle.shutdown();
        0
    });
    runtime.shutdown_timeout(std::time::Duration::from_secs(2));
    code
}
