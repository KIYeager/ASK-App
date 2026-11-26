//! Ende-zu-Ende-Test mit der gebauten App: `Terminal` und `NativeLowBackend` starten `ask --terminal` als eigenen
//! Prozess (unter Windows mit Low Integrity, Ordnerkennzeichnung und Job-Objekt) und sprechen mit ihm über HTTP.
//! Port der Windows-Fälle aus tests/test_terminal.py (Low Integrity, Umlaut im Ordner, OEM-Ausgabe, Schreiben
//! ausserhalb, Beenden des Prozessbaums) und von `CleanupTest` (Server endet, wenn stdin endet).
//!
//! Läuft auf jedem System; die Fälle, die Windows brauchen, tragen `#[cfg(windows)]`.

use std::io::{BufRead, Write};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ask_core::terminal::native_low::NativeLowBackend;
use ask_core::terminal::{StartOptions, Terminal};
use ask_core::tools::{run_tool, ToolCtx};
use serde_json::{json, Value};

const APP: &str = env!("CARGO_BIN_EXE_ask");

fn runtime() -> std::path::PathBuf {
    static DIR: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let dir = DIR.get_or_init(|| {
        let d = tempfile::tempdir().unwrap();
        std::env::set_var("ASK_RUNTIME_DIR", d.path());
        d
    });
    dir.path().to_path_buf()
}

async fn start_terminal(work: &std::path::Path) -> Terminal {
    runtime();
    let backend = NativeLowBackend::new().with_executable(APP);
    let t = Terminal::new(Arc::new(backend));
    t.start(StartOptions { folder: work.to_string_lossy().to_string(), ..Default::default() }).await;
    assert_eq!(t.state(), "läuft", "{} / {:?}", t.error(), t.logs());
    t
}

async fn run(t: &Terminal, command: &str) -> ask_core::tools::ToolResult {
    run_tool(Some(t), "run_command", &json!({ "command": command }), &ToolCtx::default()).await
}

#[tokio::test]
async fn app_als_terminal_server_low_integrity_und_befehle() {
    let base = tempfile::tempdir().unwrap();
    // Umlaut im Pfad, wie in test_terminal.py
    let work = base.path().join("Arbeitsordner Prüfung");
    let outside = base.path().join("ausserhalb");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(work.join("Übung.txt"), "Grüezi").unwrap();
    let t = start_terminal(&work).await;

    // Unter Windows prüft die App selbst, dass der Server wirklich Low ist; sonst wäre der Zustand «fehler».
    let info = t.info().unwrap();
    assert_eq!(info["low_integrity"], true, "{info}");
    #[cfg(windows)]
    assert_eq!(info["integrity_sid"], "S-1-16-4096");

    let r = run(&t, if cfg!(windows) { "echo äöü ß & cd & dir /b" } else { "echo 'äöü ß'; pwd; ls" }).await;
    assert!(r.ok, "{r:?}");
    for wanted in ["äöü ß", "Übung.txt", "Arbeitsordner Prüfung"] {
        assert!(r.output.contains(wanted), "{wanted} fehlt: {}", r.output);
    }

    // Lesen und Schreiben über die Werkzeuge, mit Sperre
    let ctx = ToolCtx::default();
    let r = run_tool(Some(&t), "write_file", &json!({ "path": "ausgabe/test.txt", "content": "Grüsse" }), &ctx).await;
    assert!(r.ok, "{r:?}");
    assert_eq!(std::fs::read_to_string(work.join("ausgabe").join("test.txt")).unwrap(), "Grüsse");
    let r = run_tool(Some(&t), "read_file", &json!({ "path": outside.to_string_lossy() }), &ctx).await;
    assert!(!r.ok && r.output.contains("ausserhalb des Arbeitsordners"), "{r:?}");

    #[cfg(windows)]
    {
        // Kinder des Servers laufen mit Low (Port von test_low_integrity_and_info und test_shell_cannot_write_outside)
        let r = run(&t, "whoami /groups").await;
        assert!(r.output.contains("S-1-16-4096"), "Low Mandatory Level fehlt: {}", r.output);
        let target = outside.join("shell.txt");
        run(&t, &format!("echo x > \"{}\"", target.display())).await;
        assert!(!target.exists(), "Befehl konnte ausserhalb schreiben");
        let home_file = std::path::PathBuf::from(std::env::var("USERPROFILE").unwrap()).join(format!("ask-test-{}.txt", std::process::id()));
        run(&t, &format!("echo x > \"{}\"", home_file.display())).await;
        assert!(!home_file.exists(), "Befehl konnte im Benutzerordner schreiben");
        // innerhalb darf er schreiben
        run(&t, "echo drin > drin.txt").await;
        assert!(work.join("drin.txt").exists());
        // ping schreibt in der OEM-Codepage (deutsches Windows: «Ping wird ausgeführt für …»)
        let r = run(&t, "ping -n 1 127.0.0.1").await;
        assert!(!r.output.contains('\u{fffd}') && r.output.contains("127.0.0.1"), "{}", r.output);
    }
    t.stop().await;
    assert_eq!(t.state(), "gestoppt");
}

#[cfg(windows)]
#[tokio::test]
async fn job_beendet_befehle_beim_stoppen() {
    let base = tempfile::tempdir().unwrap();
    let work = base.path().join("ordner");
    std::fs::create_dir_all(&work).unwrap();
    let t = start_terminal(&work).await;
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{}/execute?wait=0", t.port());
    let started: Value = client.post(url).bearer_auth(t.key()).json(&json!({ "command": "ping -n 300 -l 2 127.0.0.1 >nul" })).send().await.unwrap().json().await.unwrap();
    assert_eq!(started["status"], "running");
    let listing = || {
        let out = Command::new("tasklist").args(["/fi", "imagename eq PING.EXE", "/fo", "csv"]).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    };
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(listing().contains("PING.EXE"), "ping läuft nicht");
    t.stop().await; // «stop» und Job: Server und alle Kinder enden
    for _ in 0..40 {
        if !listing().contains("PING.EXE") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("ping läuft nach dem Stoppen noch");
}

#[test]
fn server_endet_wenn_stdin_endet() {
    // Port von CleanupTest: Endet die App (stdin geht zu), endet der Server.
    let base = tempfile::tempdir().unwrap();
    let work = base.path().join("ordner");
    std::fs::create_dir_all(&work).unwrap();
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let mut child = Command::new(APP).arg("--terminal").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let cfg = json!({ "key": "k".repeat(40), "port": port, "workdir": work.to_string_lossy(), "origin": "https://ask.example.ch", "low_integrity": false });
    writeln!(stdin, "{cfg}").unwrap();
    stdin.flush().unwrap();
    let mut out = std::io::BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    out.read_line(&mut line).unwrap();
    let event: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(event["event"], "server", "{line}");
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_ok());
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success(), "{status:?}");
            break;
        }
        assert!(Instant::now() < deadline, "Server endet nicht, obwohl stdin zu ist");
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(std::net::TcpStream::connect(("127.0.0.1", port)).is_err());
}

#[test]
fn server_ohne_einstellungen_endet_mit_fehler() {
    let mut child = Command::new(APP).arg("--terminal").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let line = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(line.contains("\"event\":\"error\""), "{line}");
}
