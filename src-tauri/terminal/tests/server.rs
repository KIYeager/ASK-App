//! Integrationstest für den Terminal-Server (Port von tests/test_terminal.py, Klasse TerminalTest).
//!
//! Der Server läuft im selben Prozess (`ask_terminal::serve`) auf einem freien Port und wird über HTTP
//! angesprochen, genau wie die App ihn anspricht. Die Befehle laufen unter Linux mit `sh`, unter Windows mit
//! `cmd.exe`. Was Low Integrity, den Starter und die echte `ASK.exe` braucht, steht in `src-tauri/tests/`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ask_terminal::{serve, Config, ServerHandle};
use serde_json::{json, Value};

const ORIGIN: &str = "https://ask.example.ch";
const KEY: &str = "geheimer-schluessel-fuer-tests-0123456789";

struct Srv {
    _base: tempfile::TempDir,
    handle: ServerHandle,
    workdir: PathBuf,
    outside: PathBuf,
    client: reqwest::Client,
}

impl Srv {
    async fn start() -> Srv {
        Self::start_with(60.0).await
    }

    async fn start_with(execute_timeout: f64) -> Srv {
        let base = tempfile::tempdir().unwrap();
        // Umlaut im Pfad, wie die App ihn schickt
        let workdir = base.path().join("Arbeitsordner Prüfung");
        let outside = base.path().join("ausserhalb");
        std::fs::create_dir_all(&workdir).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("geheim.txt"), "nicht lesen").unwrap();
        std::fs::write(workdir.join("Übung.txt"), "Grüezi").unwrap();
        let config = Config {
            key: KEY.into(),
            port: 0,
            workdir: workdir.to_string_lossy().to_string(),
            origin: ORIGIN.into(),
            execute_timeout,
            low_integrity: true,
            read_only: Vec::new(),
        };
        let handle = serve(config).await.unwrap();
        let client = reqwest::Client::builder().no_proxy().timeout(Duration::from_secs(90)).build().unwrap();
        Srv { _base: base, handle, workdir, outside, client }
    }

    fn url(&self, path: &str) -> String {
        format!("http://127.0.0.1:{}{path}", self.handle.addr().port())
    }

    async fn call(&self, method: &str, path: &str, body: Option<Value>, key: Option<&str>) -> (u16, Value) {
        let mut req = self.client.request(method.parse().unwrap(), self.url(path)).header("Authorization", format!("Bearer {}", key.unwrap_or(KEY)));
        if let Some(b) = body {
            req = req.header("Content-Type", "application/json").body(b.to_string());
        }
        let res = req.send().await.unwrap();
        let status = res.status().as_u16();
        let text = res.text().await.unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    async fn get(&self, path: &str) -> (u16, Value) {
        self.call("GET", path, None, None).await
    }

    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        self.call("POST", path, Some(body), None).await
    }

    async fn run(&self, command: &str) -> Value {
        let (status, body) = self.post("/execute", json!({ "command": command })).await;
        assert_eq!(status, 200, "{body}");
        body
    }
}

fn output(body: &Value) -> String {
    body["output"].as_array().unwrap().iter().map(|e| e["data"].as_str().unwrap()).collect()
}

fn url_encode(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

#[tokio::test]
async fn health_ohne_key_und_info_mit_key() {
    let srv = Srv::start().await;
    let res = srv.client.get(srv.url("/health")).send().await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.json::<Value>().await.unwrap(), json!({ "status": "ok" }));
    let (status, info) = srv.get("/ask/info").await;
    assert_eq!(status, 200);
    // Ausserhalb von Windows meldet der Server «Low» nur, weil die Einstellungen es sagen; unter Windows kommt es
    // vom Token (dort prüft tests/terminal_e2e.rs, dass es wirklich stimmt).
    assert_eq!(info["low_integrity"], cfg!(not(windows)));
    let real = std::fs::canonicalize(&srv.workdir).unwrap();
    assert_eq!(info["workdir"].as_str().unwrap().to_lowercase().replace("\\\\?\\", ""), real.to_string_lossy().to_lowercase().replace("\\\\?\\", ""));
    assert!(info["pid"].as_u64().unwrap() > 0);
}

#[tokio::test]
async fn key_ist_pflicht() {
    let srv = Srv::start().await;
    let (status, body) = srv.call("GET", "/files/list", None, Some("falsch")).await;
    assert_eq!((status, body["detail"].as_str().unwrap()), (401, "Invalid API key"));
    let res = srv.client.get(srv.url("/files/list")).send().await.unwrap();
    assert_eq!(res.status(), 401, "ohne Kopfzeile");
    let res = srv.client.get(srv.url("/files/list")).header("Authorization", format!("Basic {KEY}")).send().await.unwrap();
    assert_eq!(res.status(), 401, "falsches Schema");
    // Anmeldung vor der Prüfung der Eingaben
    let (status, _) = srv.call("GET", "/files/read", None, Some("falsch")).await;
    assert_eq!(status, 401);
    let (status, body) = srv.get("/files/read").await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(srv.get("/gibt-es-nicht").await.0, 404);
}

#[tokio::test]
async fn sperre_blockiert_ausserhalb() {
    let srv = Srv::start().await;
    let outside = srv.outside.join("geheim.txt");
    let windows_ini = if cfg!(windows) { "C:\\Windows\\win.ini" } else { "/etc/passwd" };
    let relative = format!("..{}ausserhalb{}geheim.txt", std::path::MAIN_SEPARATOR, std::path::MAIN_SEPARATOR);
    for path in [outside.to_string_lossy().to_string(), relative.clone(), windows_ini.to_string()] {
        let (status, body) = srv.get(&format!("/files/read?path={}", url_encode(&path))).await;
        assert_eq!(status, 403, "{path}: {body}");
        assert!(body["detail"].as_str().unwrap().starts_with("Zugriff verweigert:"));
    }
    let (status, body) = srv.post("/files/write", json!({ "path": relative.replace("geheim", "neu"), "content": "x" })).await;
    assert!([400, 403].contains(&status), "{body}");
    assert!(!srv.outside.join("neu.txt").exists());
    let (status, body) = srv.post("/execute?wait=5", json!({ "command": "echo x", "cwd": srv.outside.to_string_lossy() })).await;
    assert_eq!(status, 403, "{body}");
    for path in ["/files/list?directory=..", "/files/glob?pattern=*&path=..", "/files/grep?query=nicht&path=.."] {
        assert_eq!(srv.get(path).await.0, 403, "{path}");
    }
    let (status, _) = srv.post("/files/replace", json!({ "path": outside.to_string_lossy(), "replacements": [{ "target": "nicht", "replacement": "x" }] })).await;
    assert_eq!(status, 403);
    assert_eq!(std::fs::read_to_string(&outside).unwrap(), "nicht lesen");
}

#[cfg(unix)]
#[tokio::test]
async fn sperre_loest_symlinks_auf() {
    let srv = Srv::start().await;
    std::os::unix::fs::symlink(&srv.outside, srv.workdir.join("weg")).unwrap();
    assert_eq!(srv.get("/files/read?path=weg/geheim.txt").await.0, 403);
    assert_eq!(srv.post("/files/write", json!({ "path": "weg/neu.txt", "content": "x" })).await.0, 403);
    // beim Durchsuchen werden Verknüpfungen nach aussen nicht betreten
    let (_, grep) = srv.get("/files/grep?query=nicht&regex=false").await;
    assert!(grep["matches"].as_array().unwrap().is_empty(), "{grep}");
    let (_, list) = srv.get("/files/list").await;
    assert!(list["entries"].as_array().unwrap().iter().all(|e| e["name"] != "weg"));
}

#[tokio::test]
async fn schreiben_innerhalb() {
    let srv = Srv::start().await;
    let (status, body) = srv.post("/files/write", json!({ "path": "ausgabe/test.txt", "content": "Grüsse" })).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["size"], "Grüsse".len());
    assert_eq!(std::fs::read_to_string(srv.workdir.join("ausgabe").join("test.txt")).unwrap(), "Grüsse");
    let (status, body) = srv.get("/files/read?path=ausgabe/test.txt").await;
    assert_eq!((status, body["content"].as_str().unwrap(), body["total_lines"].as_u64().unwrap()), (200, "Grüsse", 1));
}

#[tokio::test]
async fn lesen_zeilenbereich_ersetzen_und_suchen() {
    let srv = Srv::start().await;
    srv.post("/files/write", json!({ "path": "a.txt", "content": "eins\nzwei\ndrei\nzwei\n" })).await;
    let (_, body) = srv.get("/files/read?path=a.txt&start_line=2&end_line=3").await;
    assert_eq!(body["content"], "zwei\ndrei\n");
    assert_eq!(body["total_lines"], 4);
    // Parameter mit dem Wert «null» gelten als nicht gesetzt (die App schickt sie so)
    let (status, body) = srv.get("/files/read?path=a.txt&start_line=null&end_line=null").await;
    assert_eq!((status, body["total_lines"].as_u64().unwrap()), (200, 4));
    let (status, body) = srv.post("/files/replace", json!({ "path": "a.txt", "replacements": [{ "target": "zwei", "replacement": "2" }] })).await;
    assert_eq!(status, 400, "{body}");
    assert!(body["detail"].as_str().unwrap().contains("Found 2 occurrences"));
    let (status, _) = srv.post("/files/replace", json!({ "path": "a.txt", "replacements": [{ "target": "zwei", "replacement": "2", "allow_multiple": true }] })).await;
    assert_eq!(status, 200);
    let (_, body) = srv.get("/files/read?path=a.txt").await;
    assert_eq!(body["content"], "eins\n2\ndrei\n2\n");
    assert_eq!(srv.get("/files/read?path=fehlt.txt").await.0, 404);
    let (_, list) = srv.get("/files/list?directory=.").await;
    let names: Vec<&str> = list["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"a.txt") && names.contains(&"Übung.txt"), "{names:?}");
    let (_, glob) = srv.get("/files/glob?pattern=*.txt&path=.&max_results=200").await;
    assert_eq!(glob["matches"].as_array().unwrap().len(), 2, "{glob}");
    let (_, grep) = srv.get("/files/grep?query=DREI&path=.&regex=false&case_insensitive=true").await;
    assert_eq!(grep["matches"][0]["line"], 3);
    assert_eq!(grep["matches"][0]["content"], "drei");
}

#[tokio::test]
async fn quellen_suchen_lesen_und_sperre() {
    let srv = Srv::start().await;
    std::fs::create_dir_all(srv.workdir.join("Quellen")).unwrap();
    std::fs::write(srv.workdir.join("Quellen").join("Skript Zellbiologie.md"), "# Zellen\nZellen haben einen Kern.\n# Mitose\nBei der Mitose teilt sich die Zelle.").unwrap();
    let (status, list) = srv.get("/quellen/list").await;
    assert_eq!(status, 200, "{list}");
    assert_eq!(list["dateien"][0]["datei"], "Quellen/Skript Zellbiologie.md");
    assert_eq!(list["dateien"][0]["teile"], 2);
    let (status, found) = srv.get(&format!("/quellen/search?query={}&limit=3", url_encode("Wie läuft die Mitose ab?"))).await;
    assert_eq!(status, 200, "{found}");
    assert_eq!(found["results"][0]["ort"], "Abschnitt «Mitose»");
    let (status, part) = srv.get(&format!("/quellen/read?path={}&ort=Mitose", url_encode("Skript Zellbiologie.md"))).await;
    assert_eq!(status, 200, "{part}");
    assert!(part["text"].as_str().unwrap().contains("teilt sich"));
    assert_eq!(srv.get("/quellen/search").await.0, 422, "query fehlt");
    assert_eq!(srv.get(&format!("/quellen/read?path={}", url_encode("../ausserhalb/geheim.txt"))).await.0, 403);
    assert_eq!(srv.get(&format!("/quellen/read?path={}", url_encode("Übung.txt"))).await.0, 404, "nur Dateien in Quellen");
    assert!(srv.workdir.join(".ask").join("quellen").join("index.json").is_file());
}

#[tokio::test]
async fn word_mit_inhaltssteuerelementen_und_kopfzeile() {
    // Port von test_read_docx_with_content_controls
    use std::io::Write;
    let srv = Srv::start().await;
    let ns = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
    let document = format!(
        r#"<w:document {ns}><w:body><w:sdt><w:sdtContent><w:p><w:r><w:t>Deckblatt Titel</w:t></w:r></w:p>
        <w:tbl><w:tblGrid><w:gridCol/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Name: Kayu</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:sdtContent></w:sdt>
        <w:p><w:r><w:t>Haupttext</w:t></w:r></w:p><w:sectPr><w:headerReference w:type="default" r:id="rId1"/></w:sectPr></w:body></w:document>"#
    );
    let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x" Target="header1.xml"/></Relationships>"#;
    let header = format!(r#"<w:hdr {ns}><w:p><w:r><w:t>Kopf mit Logo</w:t></w:r></w:p></w:hdr>"#);
    let file = std::fs::File::create(srv.workdir.join("vorlage.docx")).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, content) in [("word/document.xml", document.as_str()), ("word/_rels/document.xml.rels", rels), ("word/header1.xml", header.as_str())] {
        zip.start_file(name, options).unwrap();
        zip.write_all(content.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let (status, body) = srv.get("/files/read?path=vorlage.docx").await;
    assert_eq!(status, 200, "{body}");
    for text in ["Deckblatt Titel", "Name: Kayu", "Haupttext", "Kopf mit Logo"] {
        assert!(body["content"].as_str().unwrap().contains(text), "{text}");
    }
}

#[tokio::test]
async fn execute_utf8_und_arbeitsordner() {
    let srv = Srv::start().await;
    let command = if cfg!(windows) { "echo äöü ß & cd & dir /b" } else { "echo 'äöü ß'; pwd; ls" };
    let body = srv.run(command).await;
    assert_eq!(body["status"], "done", "{body}");
    assert_eq!(body["exit_code"], 0);
    let text = output(&body);
    assert!(text.contains("äöü ß"), "{text}");
    assert!(text.contains("Übung.txt"), "{text}");
    assert!(text.contains("Arbeitsordner Prüfung"), "{text}");
    assert!(!text.contains("\x1b["));
    assert_eq!(body["cwd"].as_str().unwrap().to_lowercase(), std::fs::canonicalize(&srv.workdir).unwrap().to_string_lossy().to_lowercase().trim_start_matches("\\\\?\\"));
}

#[tokio::test]
async fn execute_cwd_im_unterordner_und_umgebung() {
    let srv = Srv::start().await;
    srv.post("/files/write", json!({ "path": "unter/x.txt", "content": "x" })).await;
    let command = if cfg!(windows) { "dir /b & echo %MEINE_VAR%" } else { "ls; echo $MEINE_VAR" };
    let (status, body) = srv.post("/execute", json!({ "command": command, "cwd": "unter", "env": { "MEINE_VAR": "wert" } })).await;
    assert_eq!(status, 200, "{body}");
    let text = output(&body);
    assert!(text.contains("x.txt") && text.contains("wert"), "{text}");
}

#[tokio::test]
async fn exitcode_stdout_und_stderr_getrennt_gekennzeichnet() {
    let srv = Srv::start().await;
    let command = if cfg!(windows) { "echo aus & echo fehler 1>&2 & exit 3" } else { "echo aus; echo fehler >&2; exit 3" };
    let body = srv.run(command).await;
    assert_eq!(body["exit_code"], 3, "{body}");
    assert_eq!(body["status"], "done");
    let entries = body["output"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["type"] == "stdout" && e["data"].as_str().unwrap().contains("aus")));
    assert!(entries.iter().any(|e| e["type"] == "stderr" && e["data"].as_str().unwrap().contains("fehler")));
}

#[tokio::test]
async fn offsets_wait_und_status() {
    let srv = Srv::start().await;
    let command = if cfg!(windows) { "echo a & echo b & ping -n 3 127.0.0.1 >nul & echo c" } else { "echo a; echo b; sleep 1.5; echo c" };
    let started = Instant::now();
    let (status, body) = srv.post("/execute?wait=0", json!({ "command": command })).await;
    assert_eq!(status, 200);
    assert_eq!(body["status"], "running", "{body}");
    assert!(started.elapsed() < Duration::from_millis(1200));
    let id = body["id"].as_str().unwrap().to_string();
    // ohne Warten: nur was schon da ist
    tokio::time::sleep(Duration::from_millis(400)).await;
    let (_, first) = srv.get(&format!("/execute/{id}/status?wait=0&offset=0")).await;
    assert!(output(&first).contains('a'), "{first}");
    let next = first["next_offset"].as_u64().unwrap();
    assert!(next >= 2);
    // mit Warten: bis das Ende, nur neue Ausgabe ab dem Offset
    let (status, last) = srv.get(&format!("/execute/{id}/status?wait=30&offset={next}")).await;
    assert_eq!(status, 200);
    assert_eq!(last["status"], "done", "{last}");
    assert_eq!(last["exit_code"], 0);
    assert_eq!(output(&last).trim(), "c");
    assert_eq!(last["truncated"], false);
    // tail: nur die letzten Einträge
    let (_, tail) = srv.get(&format!("/execute/{id}/status?offset=0&tail=1")).await;
    assert_eq!(tail["output"].as_array().unwrap().len(), 1);
    assert_eq!(tail["truncated"], true);
    assert_eq!(srv.get("/execute/gibtsnicht/status").await.0, 404);
    assert_eq!(srv.call("DELETE", "/execute/gibtsnicht", None, None).await.0, 404);
}

#[tokio::test]
async fn zeitlimit_aus_den_einstellungen() {
    let srv = Srv::start_with(0.4).await;
    let command = if cfg!(windows) { "ping -n 4 127.0.0.1 >nul" } else { "sleep 2" };
    let started = Instant::now();
    let body = srv.run(command).await;
    assert_eq!(body["status"], "running", "ohne wait gilt execute_timeout: {body}");
    assert!(started.elapsed() < Duration::from_millis(1800));
    let id = body["id"].as_str().unwrap();
    srv.call("DELETE", &format!("/execute/{id}?force=true"), None, None).await;
}

#[tokio::test]
async fn abbrechen_beendet_den_ganzen_prozessbaum() {
    // Port von test_kill_tree: Der Enkelprozess schreibt seine PID und schläft; nach DELETE muss er weg sein.
    let srv = Srv::start().await;
    let command = if cfg!(windows) {
        // cmd startet ping als Kind; die PID holt ein zweiter Befehl über die Prozessliste
        "ping -n 120 -l 1 127.0.0.1 >nul".to_string()
    } else {
        "sh -c 'echo $$ > pid.txt; exec sleep 120'".to_string()
    };
    let (status, body) = srv.post("/execute?wait=0", json!({ "command": command })).await;
    assert_eq!(status, 200, "{body}");
    let id = body["id"].as_str().unwrap().to_string();
    #[cfg(unix)]
    {
        let pid_file = srv.workdir.join("pid.txt");
        let mut pid = 0i32;
        for _ in 0..50 {
            if let Ok(t) = std::fs::read_to_string(&pid_file) {
                if let Ok(p) = t.trim().parse() {
                    pid = p;
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(pid > 0, "Enkelprozess hat keine PID geschrieben");
        let (status, body) = srv.call("DELETE", &format!("/execute/{id}?force=true"), None, None).await;
        assert_eq!((status, body["status"].as_str().unwrap()), (200, "killed"));
        tokio::time::sleep(Duration::from_millis(500)).await;
        let alive = std::path::Path::new(&format!("/proc/{pid}")).exists() && !std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap_or_default().contains(") Z");
        assert!(!alive, "Prozess {pid} läuft noch");
    }
    #[cfg(windows)]
    {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let (status, _) = srv.call("DELETE", &format!("/execute/{id}?force=true"), None, None).await;
        assert_eq!(status, 200);
        tokio::time::sleep(Duration::from_millis(1500)).await;
        // die Prüfung als Skriptdatei, damit cmd keine Anführungszeichen und «|» durcheinanderbringt
        srv.post("/files/write", json!({ "path": "pruefen.ps1", "content": "@(Get-CimInstance Win32_Process -Filter \"Name='PING.EXE'\" | Where-Object { $_.CommandLine -like '*-n 120 -l 1*' }).Count" })).await;
        let check = "powershell -NoProfile -ExecutionPolicy Bypass -File pruefen.ps1";
        let body = srv.run(check).await;
        assert_eq!(output(&body).trim(), "0", "ping läuft noch: {body}");
    }
    let (_, status) = srv.get(&format!("/execute/{id}/status")).await;
    assert_eq!(status["status"], "killed");
}

#[cfg(unix)]
#[tokio::test]
async fn sehr_lange_zeile_geht_nicht_verloren() {
    let srv = Srv::start().await;
    let body = srv.run("head -c 300000 /dev/zero | tr '\\0' x; echo; echo ende").await;
    let text = output(&body);
    assert_eq!(text.matches('x').count(), 300_000);
    assert!(text.trim_end().ends_with("ende"));
    assert_eq!(body["exit_code"], 0);
}

#[tokio::test]
async fn cors_nur_fuer_den_ask_origin() {
    let srv = Srv::start().await;
    let preflight = |origin: &'static str| {
        srv.client
            .request(reqwest::Method::OPTIONS, srv.url("/files/list"))
            .header("Origin", origin)
            .header("Access-Control-Request-Method", "GET")
            .header("Access-Control-Request-Headers", "authorization")
            .header("Access-Control-Request-Private-Network", "true")
            .send()
    };
    let res = preflight(ORIGIN).await.unwrap();
    assert_eq!(res.status(), 200);
    assert_eq!(res.headers()["access-control-allow-origin"], ORIGIN);
    assert_eq!(res.headers()["access-control-allow-private-network"], "true");
    let res = preflight("https://fremd.example").await.unwrap();
    assert_eq!(res.status(), 400);
    assert!(res.headers().get("access-control-allow-origin").is_none());
    // normale Antworten tragen den Origin nur für den erlaubten
    let res = srv.client.get(srv.url("/health")).header("Origin", ORIGIN).send().await.unwrap();
    assert_eq!(res.headers()["access-control-allow-origin"], ORIGIN);
    let res = srv.client.get(srv.url("/health")).header("Origin", "https://fremd.example").send().await.unwrap();
    assert!(res.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn fremder_host_wird_abgewiesen() {
    // Schutz vor DNS-Rebinding
    let srv = Srv::start().await;
    let res = srv.client.get(srv.url("/health")).header("Host", "boese.example").send().await.unwrap();
    assert_eq!(res.status(), 400);
}

#[tokio::test]
async fn nur_auf_127_0_0_1() {
    let srv = Srv::start().await;
    assert!(srv.handle.addr().ip().is_loopback());
    assert_eq!(srv.handle.addr().ip().to_string(), "127.0.0.1");
}
