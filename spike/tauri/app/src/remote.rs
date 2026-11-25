// Fernsteuerung des Browser-Bereichs (Frage 3) und der automatische Ablauf des Spikes.
//
// Was Tauri 2.12 selbst kann (alle Plattformen): eval_with_callback (JS mit Ergebnis), navigate, on_navigation,
// on_download, on_new_window, on_permission_request, data_directory, clear_all_browsing_data, cookies.
// Was fehlt: Bildschirmfoto, echte (isTrusted) Maus- und Tastatureingaben, Konsole ausserhalb der Seite.
// Dafür geht es über with_webview an die Plattform:
//   Linux   webkit2gtk::WebView::snapshot (getestet). Echte Eingaben: keine API (nur über GDK-Ereignisse, nicht gebaut).
//   Windows ICoreWebView2::CallDevToolsProtocolMethod: Page.captureScreenshot, Input.dispatchMouseEvent,
//           Input.insertText, Input.dispatchKeyEvent, Runtime.consoleAPICalled (nur kompiliert, nicht ausgeführt).

use std::{path::PathBuf, time::Duration};

use serde_json::{json, Value};
use tauri::{AppHandle, Manager, Url, Webview};
use tokio::sync::oneshot;

use crate::report;

/// JavaScript in einer Webview ausführen und das Ergebnis (JSON) abwarten.
pub async fn js(wv: &Webview, code: &str) -> Value {
  let (tx, rx) = oneshot::channel::<String>();
  let tx = std::sync::Mutex::new(Some(tx));
  if wv
    .eval_with_callback(code, move |res| {
      if let Some(tx) = tx.lock().unwrap().take() {
        let _ = tx.send(res);
      }
    })
    .is_err()
  {
    return Value::Null;
  }
  match tokio::time::timeout(Duration::from_secs(10), rx).await {
    Ok(Ok(s)) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
    _ => json!("zeitüberschreitung"),
  }
}

// ---------- Bildschirmfoto einer einzelnen Webview ----------

#[cfg(target_os = "linux")]
pub async fn snapshot(wv: &Webview, path: PathBuf) -> Result<(i32, i32), String> {
  use webkit2gtk::{SnapshotOptions, SnapshotRegion, WebViewExt};
  let (tx, rx) = oneshot::channel();
  wv.with_webview(move |pw| {
    pw.inner().snapshot(SnapshotRegion::Visible, SnapshotOptions::NONE, None::<&gio::Cancellable>, move |res| {
      let out = res.map_err(|e| e.to_string()).and_then(|surface| {
        let img = cairo::ImageSurface::try_from(surface).map_err(|_| "keine ImageSurface".to_string())?;
        let mut f = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        img.write_to_png(&mut f).map_err(|e| e.to_string())?;
        Ok((img.width(), img.height()))
      });
      let _ = tx.send(out);
    });
  })
  .map_err(|e| e.to_string())?;
  rx.await.map_err(|_| "kein Ergebnis".to_string())?
}

#[cfg(windows)]
pub async fn snapshot(wv: &Webview, path: PathBuf) -> Result<(i32, i32), String> {
  use base64::Engine;
  let res = win::cdp(wv, "Page.captureScreenshot", json!({ "format": "png" })).await?;
  let data = res.get("data").and_then(Value::as_str).ok_or("keine Daten")?;
  let png = base64::engine::general_purpose::STANDARD.decode(data).map_err(|e| e.to_string())?;
  std::fs::write(&path, &png).map_err(|e| e.to_string())?;
  Ok((0, 0))
}

/// Echter Klick (isTrusted = true), wie wc.sendInputEvent in visual.js.
#[cfg(windows)]
pub async fn click_trusted(wv: &Webview, x: f64, y: f64) -> Result<(), String> {
  for kind in ["mouseMoved", "mousePressed", "mouseReleased"] {
    win::cdp(wv, "Input.dispatchMouseEvent", json!({ "type": kind, "x": x, "y": y, "button": "left", "clickCount": 1 })).await?;
  }
  Ok(())
}
#[cfg(not(windows))]
pub async fn click_trusted(_wv: &Webview, _x: f64, _y: f64) -> Result<(), String> {
  Err("WebKitGTK hat keine API für echte Eingaben; nur über GDK-Ereignisse (nicht gebaut)".into())
}

/// Text einfügen wie wc.insertText.
#[cfg(windows)]
pub async fn insert_text_trusted(wv: &Webview, text: &str) -> Result<(), String> {
  win::cdp(wv, "Input.insertText", json!({ "text": text })).await.map(|_| ())
}
#[cfg(not(windows))]
pub async fn insert_text_trusted(_wv: &Webview, _text: &str) -> Result<(), String> {
  Err("auf Linux nicht verfügbar".into())
}

#[cfg(windows)]
pub mod win {
  //! WebView2 über COM (webview2-com 0.39, dieselbe Version wie in Tauri 2.12). Nur kompiliert.
  use serde_json::Value;
  use tauri::Webview;
  use tokio::sync::oneshot;
  use webview2_com::{
    CallDevToolsProtocolMethodCompletedHandler, DevToolsProtocolEventReceivedEventHandler,
    Microsoft::Web::WebView2::Win32::ICoreWebView2,
  };
  use windows::core::HSTRING;

  fn core(pw: &tauri::webview::PlatformWebview) -> windows::core::Result<ICoreWebView2> {
    unsafe { pw.controller().CoreWebView2() }
  }

  /// Chrome DevTools Protocol über ICoreWebView2::CallDevToolsProtocolMethod.
  pub async fn cdp(wv: &Webview, method: &'static str, params: Value) -> Result<Value, String> {
    let (tx, rx) = oneshot::channel::<Result<String, String>>();
    wv.with_webview(move |pw| {
      let run = || -> windows::core::Result<()> {
        let core = core(&pw)?;
        let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |hr, json| {
          let _ = tx.send(hr.map(|_| json).map_err(|e| e.to_string()));
          Ok(())
        }));
        unsafe { core.CallDevToolsProtocolMethod(&HSTRING::from(method), &HSTRING::from(params.to_string()), &handler) }
      };
      if let Err(e) = run() {
        eprintln!("CDP {method}: {e}");
      }
    })
    .map_err(|e| e.to_string())?;
    let s = rx.await.map_err(|_| "kein Ergebnis".to_string())??;
    serde_json::from_str(&s).map_err(|e| e.to_string())
  }

  /// Konsole der Seite ausserhalb der Seite mitlesen (ersetzt wc.on('console-message')).
  pub fn watch_console(wv: &Webview, on_message: impl Fn(String) + Send + 'static) {
    let _ = wv.with_webview(move |pw| {
      let run = || -> windows::core::Result<()> {
        let core = core(&pw)?;
        let receiver = unsafe { core.GetDevToolsProtocolEventReceiver(&HSTRING::from("Runtime.consoleAPICalled"))? };
        let handler = DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_sender, args| {
          if let Some(args) = args {
            let mut json = windows::core::PWSTR::null();
            unsafe { args.ParameterObjectAsJson(&mut json)? };
            on_message(unsafe { json.to_string().unwrap_or_default() });
          }
          Ok(())
        }));
        let mut token = 0i64;
        unsafe { receiver.add_DevToolsProtocolEventReceived(&handler, &mut token)? };
        let enable = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(|_, _| Ok(())));
        unsafe { core.CallDevToolsProtocolMethod(&HSTRING::from("Runtime.enable"), &HSTRING::from("{}"), &enable) }
      };
      if let Err(e) = run() {
        eprintln!("Konsole: {e}");
      }
    });
  }
}

// ---------- Ablauf ----------

async fn wait(ms: u64) {
  tokio::time::sleep(Duration::from_millis(ms)).await;
}

fn screen(out: &PathBuf, name: &str) {
  // Ganzes Fenster mit allen Webviews (X11 unter Xvfb; unter Windows nicht vorhanden)
  if cfg!(windows) {
    return;
  }
  let path = out.join(name);
  let ok = std::process::Command::new("import").args(["-window", "root"]).arg(&path).status().map(|s| s.success()).unwrap_or(false);
  report("screenshot", json!({ "file": name, "ok": ok }));
}

pub async fn scenario(app: AppHandle, fixtures: String, out: PathBuf) {
  let _ = std::fs::create_dir_all(&out);
  let ui = || app.get_webview("ui").unwrap();
  let ask = || app.get_webview("ask").unwrap();
  let browser = || app.get_webview("browser").unwrap();
  let sidecar = app.state::<std::sync::Arc<crate::Sidecar>>().inner().clone();

  // 1. Oberfläche läuft unverändert mit dem Shim
  let mut ready = Value::Null;
  for _ in 0..60 {
    ready = js(&ui(), "({ root: document.getElementById('root')?.children.length || 0, ask: typeof window.ask, methods: window.ask ? Object.keys(window.ask).length : 0, text: (document.body.innerText || '').slice(0, 160) })").await;
    if ready.get("root").and_then(Value::as_u64).unwrap_or(0) > 0 && ready.get("text").and_then(Value::as_str).map(|t| t.len() > 20).unwrap_or(false) {
      break;
    }
    wait(250).await;
  }
  report("ui-rendered", ready);
  // Binärdaten über die Brücke (wie Hineinziehen in den Dateibereich, RightPanel.jsx): Uint8Array -> echter workspace.import
  let _ = js(&ui(), "window.__imp = 'läuft'; window.ask.importFiles('', [{ name: 'spike-anhang.bin', type: '', data: new Uint8Array(300000).map((_, i) => i % 251) }]).then((r) => { window.__imp = r; }, (e) => { window.__imp = 'Fehler: ' + e; }); 1").await;
  wait(800).await;
  let imp = js(&ui(), "window.__imp").await;
  let on_disk = std::env::var("SPIKE_FOLDER").ok().and_then(|f| std::fs::read(PathBuf::from(f).join("spike-anhang.bin")).ok());
  let intact = on_disk.as_ref().map(|d| d.len() == 300000 && d.iter().enumerate().all(|(i, b)| *b as usize == i % 251));
  report("binary-import", json!({ "result": imp, "bytes_on_disk": on_disk.map(|d| d.len()), "intact": intact }));
  wait(800).await;
  screen(&out, "1-oberflaeche.png");

  // 2. Anmeldung: Zustand «nicht angemeldet» -> Oberfläche meldet #ask-host -> Webview "ask" erscheint darüber
  let _ = sidecar.call("settings:set", vec![json!({ "ask": { "state": "nicht-angemeldet" } })]).await;
  wait(1000).await;
  let ask_rect = js(&ask(), "({ w: innerWidth, h: innerHeight, title: document.title })").await;
  report("ask-webview", json!({ "inside": ask_rect, "bounds": ask().bounds().map(|b| format!("{b:?}")).unwrap_or_default() }));
  screen(&out, "2-anmeldung.png");
  // Token lesen wie askapi.js (executeJavaScript('localStorage.token'))
  let mut token = String::new();
  for _ in 0..40 {
    token = js(&ask(), "localStorage.token || ''").await.as_str().unwrap_or("").to_string();
    if !token.is_empty() {
      break;
    }
    wait(250).await;
  }
  report("ask-token", json!({ "read": !token.is_empty(), "length": token.len() }));
  let _ = sidecar.call("settings:set", vec![json!({ "ask": { "state": "angemeldet", "user": "Spike" } })]).await;
  wait(800).await;

  // 3. Browser-Bereich: Reiter öffnen -> Oberfläche meldet #browser-host -> Webview "browser" erscheint dort
  let tab = js(&ui(), "(() => { const b = [...document.querySelectorAll('[role=tab]')].find((x) => x.textContent.includes('Browser')); if (b) b.click(); return !!b; })()").await;
  report("browser-tab", json!({ "clicked": tab }));
  wait(600).await;
  let _ = browser().navigate(Url::parse(&format!("{fixtures}/seite")).unwrap());
  wait(1500).await;
  let page = js(&browser(), "({ title: document.title, url: location.href, w: innerWidth, h: innerHeight })").await;
  report("browser-page", json!({ "page": page, "bounds": browser().bounds().map(|b| format!("{b:?}")).unwrap_or_default() }));

  // 4. Fernsteuerung: zuerst echte Eingaben (Windows: CDP), dann über JavaScript (überall, aber isTrusted=false)
  let pos = js(&browser(), "(() => { const r = document.getElementById('knopf').getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()").await;
  let (x, y) = (pos["x"].as_f64().unwrap_or(0.0), pos["y"].as_f64().unwrap_or(0.0));
  let trusted = click_trusted(&browser(), x, y).await;
  wait(300).await;
  report("click-trusted", json!({ "call": format!("{trusted:?}"), "page": js(&browser(), "document.getElementById('klick').textContent").await }));
  let _ = js(&browser(), "document.getElementById('feld').focus()").await;
  let typed = insert_text_trusted(&browser(), "Max").await;
  wait(300).await;
  report("type-trusted", json!({ "call": format!("{typed:?}"), "page": js(&browser(), "document.getElementById('eingabe').textContent").await }));
  let click = js(&browser(), "(() => { document.getElementById('knopf').click(); return document.getElementById('klick').textContent; })()").await;
  report("click-js", json!({ "result": click }));
  let typed = js(&browser(), "(() => { const f = document.getElementById('feld'); f.focus(); f.value = 'Lea'; f.dispatchEvent(new InputEvent('input', { bubbles: true })); return document.getElementById('eingabe').textContent; })()").await;
  report("type-js", json!({ "result": typed }));
  let logs = js(&browser(), "window.__askLogs || []").await;
  report("console", json!({ "logs": logs }));
  #[cfg(windows)]
  win::watch_console(&browser(), |json| report("console-cdp", json!({ "event": json.chars().take(200).collect::<String>() })));
  let shot = snapshot(&browser(), out.join("4-browser-snapshot.png")).await;
  report("snapshot", json!({ "result": format!("{shot:?}") }));

  // 5. Sperren: Navigation, Download, Popup
  let _ = js(&browser(), "document.getElementById('weg').click()").await;
  wait(1200).await;
  let after = js(&browser(), "location.pathname").await;
  report("navigation-after-blocked-link", json!({ "path": after }));
  let _ = js(&browser(), "document.getElementById('dl').click()").await;
  wait(1200).await;
  let _ = js(&browser(), "document.getElementById('popup').click()").await;
  wait(800).await;

  // 6. Darf die fremde Seite die App ansprechen? (darf sie nicht)
  let _ = js(&browser(), "window.__ipc = 'läuft'; (async () => { try { if (!window.__TAURI_INTERNALS__) { window.__ipc = 'kein __TAURI_INTERNALS__'; return; } const r = await window.__TAURI_INTERNALS__.invoke('ask_invoke', { channel: 'state:get', args: [] }); window.__ipc = 'ERREICHT: ' + JSON.stringify(r).slice(0, 60); } catch (e) { window.__ipc = 'abgelehnt: ' + e; } })(); 1").await;
  wait(1000).await;
  report("ipc-from-browser", json!({ "result": js(&browser(), "window.__ipc").await }));
  let _ = js(&ask(), "window.__ipc = 'läuft'; (async () => { try { const r = await window.__TAURI_INTERNALS__.invoke('ask_invoke', { channel: 'state:get', args: [] }); window.__ipc = 'ERREICHT'; } catch (e) { window.__ipc = 'abgelehnt: ' + e; } })(); 1").await;
  wait(800).await;
  report("ipc-from-ask", json!({ "result": js(&ask(), "window.__ipc").await }));
  screen(&out, "3-browser-bereich.png");

  // 7. Chat über die echte Eingabe: Text ins Feld, Enter -> chat:send -> Sidecar -> chat:event -> Oberfläche
  let sent = js(&ui(), "(() => { const ta = document.querySelector('textarea'); if (!ta) return 'kein Eingabefeld'; ta.focus(); const set = Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set; set.call(ta, 'Räum den Ordner auf'); ta.dispatchEvent(new Event('input', { bubbles: true })); ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', bubbles: true, cancelable: true })); return 'gesendet'; })()").await;
  report("chat-input", json!({ "result": sent }));
  let mut seen = Value::Null;
  for _ in 0..40 {
    wait(250).await;
    seen = js(&ui(), "document.body.innerText.includes('Welche Dateien darf ich löschen?')").await;
    if seen == Value::Bool(true) {
      break;
    }
  }
  report("chat-question-visible", json!({ "visible": seen }));
  screen(&out, "5-chat-frage.png");
  let answered = js(&ui(), "(() => { const b = [...document.querySelectorAll('button')].find((x) => x.textContent.trim().startsWith('Nichts löschen')); if (b) b.click(); return !!b; })()").await;
  report("chat-answer", json!({ "clicked": answered }));
  wait(800).await;
  let approval = js(&ui(), "(() => { const b = [...document.querySelectorAll('button')].find((x) => /^(Ablehnen|Nein)/.test(x.textContent.trim())); if (b) { b.click(); return b.textContent.trim(); } return [...document.querySelectorAll('button')].map((x) => x.textContent.trim()).filter(Boolean).slice(-12); })()").await;
  report("chat-approval", json!({ "result": approval }));
  let mut done = Value::Null;
  for _ in 0..40 {
    wait(250).await;
    done = js(&ui(), "document.body.innerText.includes('Erledigt.')").await;
    if done == Value::Bool(true) {
      break;
    }
  }
  report("chat-done-visible", json!({ "visible": done }));
  wait(500).await;
  screen(&out, "6-chat-fertig.png");
  report("end", json!({}));
  app.exit(0);
}
