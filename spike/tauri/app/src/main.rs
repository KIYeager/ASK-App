// Tauri-2-Spike für ASK Desktop (docs/TAURI-SPIKE.md). Kein Port der App.
//
// Ein Fenster mit drei Webviews (Feature `unstable`, multiwebview):
//   ui      die unveränderte Oberfläche aus src/renderer/dist, window.ask kommt aus shim/shim.js
//   ask     Anmeldeseite (hier eine Attrappe auf 127.0.0.1), eigener Datenordner
//   browser Browser-Bereich, eigener Datenordner, Navigation und Downloads gesperrt
//
// window.ask -> invoke("ask_invoke", {channel, args}) -> Rust. Rust beantwortet die Kanäle für Lage und
// Sichtbarkeit der Webviews selbst, alles andere geht an den Node-Sidecar (sidecar/sidecar.mjs).
// Ereignisse des Sidecars (z. B. chat:event) gibt Rust mit emit_to("ui", …) an die Oberfläche weiter.
//
// Mit SPIKE_SCENARIO=1 läuft ein Ablauf von selbst durch (Anmeldung, Browser, Chat) und schreibt
// Zeilen «SPIKE {json}» auf stdout. Siehe run-linux.sh.

mod lowjob;
mod remote;

use std::{
  collections::HashMap,
  path::PathBuf,
  sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex,
  },
  time::Duration,
};

use serde_json::{json, Value};
use tauri::{
  webview::{DownloadEvent, NewWindowResponse, PermissionKind, PermissionResponse, WebviewBuilder},
  window::WindowBuilder,
  AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Url, Webview, WebviewUrl,
};
use tokio::{
  io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
  process::{ChildStdin, Command},
  sync::{oneshot, Mutex as AsyncMutex},
};

const SHIM: &str = include_str!("../shim/shim.js");
// Konsole der fremden Seite mitschreiben. Läuft in der Seite selbst und kann von ihr verändert werden;
// WebView2 könnte das über CDP Runtime.consoleAPICalled ausserhalb der Seite (remote.rs, Windows).
const CONSOLE_HOOK: &str = r#"(() => {
  window.__askLogs = [];
  for (const level of ['error', 'warn']) {
    const orig = console[level];
    console[level] = (...a) => { try { window.__askLogs.push(level + ': ' + a.join(' ')); } catch {} return orig.apply(console, a); };
  }
  addEventListener('error', (e) => window.__askLogs.push('error: ' + e.message));
})();"#;

pub fn report(step: &str, data: Value) {
  println!("SPIKE {}", json!({ "step": step, "data": data }));
}

// ---------- Sidecar ----------

struct Sidecar {
  stdin: AsyncMutex<ChildStdin>,
  pending: Mutex<HashMap<u64, oneshot::Sender<Result<Value, String>>>>,
  next: AtomicU64,
}

impl Sidecar {
  async fn call(&self, channel: &str, args: Vec<Value>) -> Result<Value, String> {
    let id = self.next.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = oneshot::channel();
    self.pending.lock().unwrap().insert(id, tx);
    let line = json!({ "id": id, "channel": channel, "args": args }).to_string() + "\n";
    self.stdin.lock().await.write_all(line.as_bytes()).await.map_err(|e| e.to_string())?;
    tokio::time::timeout(Duration::from_secs(30), rx)
      .await
      .map_err(|_| format!("Sidecar antwortet nicht auf {channel}"))?
      .map_err(|_| "Sidecar beendet".to_string())?
  }
}

fn start_sidecar(app: &AppHandle, spike_dir: PathBuf) -> Arc<Sidecar> {
  // Wie heute backend/native-low.js: Kindprozess, Einstellungen über stdin, JSON-Zeilen auf stdout.
  // Für die echte App käme node.exe als Tauri-Sidecar (bundle.externalBin) mit; hier das Node im PATH.
  let rt = tauri::async_runtime::handle();
  let _guard = rt.inner().enter(); // tokio::process braucht die Laufzeit von Tauri
  let mut child = Command::new(std::env::var("SPIKE_NODE").unwrap_or_else(|_| "node".into()))
    .arg("--no-warnings")
    .arg(spike_dir.join("sidecar/sidecar.mjs"))
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::piped())
    .kill_on_drop(true)
    .spawn()
    .expect("Node-Sidecar startet nicht");
  let stdout = child.stdout.take().unwrap();
  let sidecar = Arc::new(Sidecar {
    stdin: AsyncMutex::new(child.stdin.take().unwrap()),
    pending: Mutex::new(HashMap::new()),
    next: AtomicU64::new(1),
  });
  let sc = sidecar.clone();
  let app = app.clone();
  tauri::async_runtime::spawn(async move {
    let _child = child; // lebt so lange wie die Leseschleife
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
      let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
      if let Some(ev) = msg.get("event").and_then(Value::as_str) {
        let payload = msg.get("payload").cloned().unwrap_or(Value::Null);
        if ev == "chat:event" {
          report("chat:event", json!({ "type": payload.get("type") }));
        }
        let _ = app.emit_to("ui", ev, payload);
      } else if let Some(id) = msg.get("id").and_then(Value::as_u64) {
        let res = if msg.get("ok") == Some(&Value::Bool(true)) {
          Ok(msg.get("result").cloned().unwrap_or(Value::Null))
        } else {
          Err(msg.get("error").and_then(Value::as_str).unwrap_or("Fehler").to_string())
        };
        if let Some(tx) = sc.pending.lock().unwrap().remove(&id) {
          let _ = tx.send(res);
        }
      }
    }
  });
  sidecar
}

// ---------- window.ask ----------

fn place(app: &AppHandle, label: &str, rect: &Value) -> Result<(), String> {
  let wv = app.get_webview(label).ok_or("Webview fehlt")?;
  let f = |k: &str| rect.get(k).and_then(Value::as_f64).unwrap_or(0.0);
  #[cfg(target_os = "linux")]
  let _ = wv;
  #[cfg(target_os = "linux")]
  {
    let (label, x, y, w, h) = (label.to_string(), f("x"), f("y"), f("width"), f("height"));
    return app.run_on_main_thread(move || linux::place(&label, x, y, w, h)).map_err(|e| e.to_string());
  }
  // Die Oberfläche meldet CSS-Pixel; Tauri rechnet logische Pixel selbst in physische um.
  #[cfg(not(target_os = "linux"))]
  {
    wv.set_position(LogicalPosition::new(f("x"), f("y"))).map_err(|e| e.to_string())?;
    wv.set_size(LogicalSize::new(f("width").max(1.0), f("height").max(1.0))).map_err(|e| e.to_string())?;
    Ok(())
  }
}

fn visible(app: &AppHandle, label: &str, on: bool) -> Result<(), String> {
  let wv = app.get_webview(label).ok_or("Webview fehlt")?;
  if on { wv.show() } else { wv.hide() }.map_err(|e| e.to_string())
}

#[tauri::command]
async fn ask_invoke(
  app: AppHandle,
  webview: Webview,
  sidecar: tauri::State<'_, Arc<Sidecar>>,
  channel: String,
  args: Vec<Value>,
) -> Result<Value, String> {
  // Nur die eigene Oberfläche darf den Hauptprozess ansprechen (zusätzlich zu capabilities/default.json)
  if webview.label() != "ui" {
    report("ipc-denied", json!({ "from": webview.label(), "channel": channel }));
    return Err("nicht erlaubt".into());
  }
  let arg0 = args.first().cloned().unwrap_or(Value::Null);
  let native = match channel.as_str() {
    "ui:ask-bounds" => Some(place(&app, "ask", &arg0)),
    "ui:browser-bounds" => Some(place(&app, "browser", &arg0)),
    "ui:ask-visible" => Some(visible(&app, "ask", arg0.as_bool().unwrap_or(false))),
    "ui:browser-visible" => Some(visible(&app, "browser", arg0.as_bool().unwrap_or(false))),
    _ => None,
  };
  if let Some(res) = native {
    report("native", json!({ "channel": channel, "args": args }));
    return res.map(|_| Value::Null);
  }
  if channel == "browser:go" {
    let url = Url::parse(arg0.as_str().unwrap_or("about:blank")).map_err(|e| e.to_string())?;
    app.get_webview("browser").ok_or("Browser fehlt")?.navigate(url).map_err(|e| e.to_string())?;
    return Ok(Value::Null);
  }
  let res = sidecar.call(&channel, args).await;
  report("sidecar", json!({ "channel": channel, "ok": res.is_ok() }));
  res
}

// ---------- Erlaubte Navigation im Browser (wie allowedNavigation in src/main/visual.js, stark vereinfacht) ----------

fn allowed_navigation(url: &Url, fixtures: &str) -> bool {
  match url.scheme() {
    "about" | "data" => true,
    // Testregel: die Adresse /gesperrt der Testseiten steht für «lokales Netz / ausserhalb des Ordners»
    "http" | "https" => !(url.as_str().starts_with(fixtures) && url.path() == "/gesperrt"),
    _ => false,
  }
}

fn main() {
  // Ordner spike/tauri (Sidecar, Ergebnisse). Auf einem anderen Rechner mit SPIKE_DIR angeben.
  let spike_dir = std::env::var("SPIKE_DIR").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
  let fixtures = std::env::var("SPIKE_FIXTURES").unwrap_or_else(|_| "http://127.0.0.1:18765".into());

  tauri::Builder::default()
    .invoke_handler(tauri::generate_handler![ask_invoke])
    .setup(move |app| {
      let handle = app.handle().clone();
      let data = app.path().app_local_data_dir()?;
      app.manage(start_sidecar(&handle, spike_dir.clone()));

      let window = WindowBuilder::new(app, "main")
        .title("ASK (Tauri-Spike)")
        .inner_size(1440.0, 920.0)
        .min_inner_size(960.0, 600.0)
        .build()?;
      let size = window.inner_size()?;

      // 1. Oberfläche, füllt das Fenster
      window.add_child(
        WebviewBuilder::new("ui", WebviewUrl::App("index.html".into()))
          .initialization_script(SHIM)
          // Sonst fängt Tauri Dateien aus dem Explorer ab, und die Oberfläche bekommt kein dataTransfer.files
          .disable_drag_drop_handler()
          .on_navigation(|url| url.scheme() == "tauri" || url.host_str() == Some("tauri.localhost"))
          .auto_resize(),
        LogicalPosition::new(0.0, 0.0),
        size,
      )?;

      // 2. ASK-Anmeldeseite, eigener Datenordner (wie die Partition persist:ask)
      let ask_url = Url::parse(&format!("{fixtures}/auth"))?;
      let ask = window.add_child(
        WebviewBuilder::new("ask", WebviewUrl::External(ask_url))
          .data_directory(data.join("ask"))
          .on_permission_request(|wv, kind| {
            // wie setPermissionRequestHandler in window.js: nur für den ASK-Ursprung, nur bekannte Arten
            let origin_ok = wv.url().map(|u| u.host_str() == Some("127.0.0.1")).unwrap_or(false);
            report("permission", json!({ "webview": "ask", "kind": format!("{kind:?}"), "allowed": origin_ok }));
            match (origin_ok, kind) {
              (true, PermissionKind::Notifications | PermissionKind::ClipboardRead) => PermissionResponse::Allow,
              _ => PermissionResponse::Deny,
            }
          }),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(1.0, 1.0),
      )?;
      ask.hide()?;

      // 3. Browser-Bereich, eigener Datenordner (wie persist:ask-browser)
      let fx = fixtures.clone();
      let h2 = handle.clone();
      let browser = window.add_child(
        WebviewBuilder::new("browser", WebviewUrl::External("about:blank".parse()?))
          .data_directory(data.join("browser"))
          .initialization_script(CONSOLE_HOOK)
          .on_navigation(move |url| {
            let ok = allowed_navigation(url, &fx);
            report("navigation", json!({ "url": url.as_str(), "allowed": ok }));
            ok
          })
          .on_download(move |_wv, ev| {
            if let DownloadEvent::Requested { url, .. } = ev {
              report("download", json!({ "url": url.as_str(), "allowed": false }));
              let _ = h2.emit_to("ui", "browser:notice", format!("Download blockiert: {url}"));
            }
            false
          })
          .on_new_window(|url, _features| {
            report("new-window", json!({ "url": url.as_str(), "allowed": false }));
            NewWindowResponse::Deny
          })
          .on_permission_request(|_wv, kind| {
            report("permission", json!({ "webview": "browser", "kind": format!("{kind:?}"), "allowed": false }));
            PermissionResponse::Deny
          }),
        LogicalPosition::new(0.0, 0.0),
        LogicalSize::new(1.0, 1.0),
      )?;
      browser.hide()?;
      #[cfg(target_os = "linux")]
      linux::into_fixed(&window)?;

      if std::env::var("SPIKE_SCENARIO").is_ok() {
        let out = std::env::var("SPIKE_OUT").map(PathBuf::from).unwrap_or_else(|_| spike_dir.join("results"));
        tauri::async_runtime::spawn(remote::scenario(handle, fixtures.clone(), out));
      }
      report("started", json!({ "tauri": tauri::VERSION, "webviews": ["ui", "ask", "browser"] }));
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("Tauri startet nicht");
}

// Linux: Tauri legt Kind-Webviews in eine senkrechte gtk::Box (tauri-runtime-wry 2.12.5, WebviewKind::WindowChild),
// set_position/set_size wirken dort nicht. Für den Spike: alle drei in ein gtk::Fixed umhängen und selbst platzieren.
// Unter Windows und macOS braucht es das nicht (build_as_child mit echten Koordinaten).
#[cfg(target_os = "linux")]
mod linux {
  use gtk::prelude::*;
  use std::{cell::RefCell, collections::HashMap};
  thread_local! {
    static LAYOUT: RefCell<Option<(gtk::Fixed, HashMap<String, gtk::Widget>)>> = const { RefCell::new(None) };
  }
  pub fn into_fixed(window: &tauri::Window) -> tauri::Result<()> {
    let vbox = window.default_vbox()?;
    let fixed = gtk::Fixed::new();
    let mut map = HashMap::new();
    for (kid, label) in vbox.children().into_iter().zip(["ui", "ask", "browser"]) {
      let visible = kid.is_visible();
      vbox.remove(&kid);
      fixed.put(&kid, 0, 0);
      kid.set_visible(visible);
      map.insert(label.to_string(), kid);
    }
    vbox.pack_start(&fixed, true, true, 0);
    fixed.show();
    let ui = map["ui"].clone();
    let size = window.inner_size()?.to_logical::<i32>(window.scale_factor()?);
    ui.set_size_request(size.width, size.height);
    // Oberfläche wächst mit dem Fenster
    fixed.connect_size_allocate(move |_, a| ui.set_size_request(a.width(), a.height()));
    LAYOUT.with(|l| *l.borrow_mut() = Some((fixed, map)));
    Ok(())
  }
  pub fn place(label: &str, x: f64, y: f64, w: f64, h: f64) {
    LAYOUT.with(|l| {
      if let Some((fixed, map)) = l.borrow().as_ref() {
        if let Some(kid) = map.get(label) {
          fixed.move_(kid, x.round() as i32, y.round() as i32);
          kid.set_size_request(w.round().max(1.0) as i32, h.round().max(1.0) as i32);
        }
      }
    });
  }
}

// Für lowjob.rs, damit der Compiler das Modul nicht als unbenutzt meldet.
#[allow(dead_code)]
fn _uses() {
  let _ = lowjob::describe();
}
