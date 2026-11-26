//! Webansichten für visual.rs (`Page`, `PageProvider`): der Browser-Bereich der App, unsichtbare Fenster für
//! Tests ohne sichtbaren Browser und das Fenster mit pdf.js zum Zeichnen von PDF-Seiten. Gesteuert über das
//! DevTools-Protokoll von WebView2 (cdp.rs). In Electron waren das webContents, ein BrowserWindow mit
//! `offscreen: true` und das unsichtbare Fenster mit render/pdf.html.
//!
//! «Unsichtbar» heisst hier: ein normales Fenster weit ausserhalb des Bildschirms, ohne Eintrag in der Taskleiste.
//! Ein verstecktes Fenster zeichnet WebView2 nicht, dann gäbe es keine Bildschirmfotos.

use crate::{cdp, shell, views};
use anyhow::{anyhow, Result};
use ask_core::visual::{KeyEvent, KeyKind, LoadError, MouseKind, Page, PageEvent, PageListener, PageProvider};
use async_trait::async_trait;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
use tauri::{Manager, WebviewUrl, Webview, Wry};

/// Ladezähler und letzter Ladefehler je Webansicht (Bezeichnung)
type ByLabel<T> = LazyLock<Mutex<HashMap<String, T>>>;
static LOADS: ByLabel<(u64, Option<LoadError>)> = LazyLock::new(Default::default);
static LOADED: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);
/// Abonnenten der Seitenereignisse je Webansicht
static LISTENERS: ByLabel<Vec<(u64, PageListener)>> = LazyLock::new(Default::default);
static SEQ: AtomicU64 = AtomicU64::new(1);
/// Neuestes Bild des Screencasts je unsichtbarem Fenster: (JPEG, neu seit dem letzten take_paint)
static PAINTS: ByLabel<(Vec<u8>, bool)> = LazyLock::new(Default::default);
/// Name der DevTools-Bindung, über die Seiten Eingaben der Person melden (zufällig, damit Seiten sie nicht kennen)
static INPUT_BINDING: LazyLock<String> = LazyLock::new(|| format!("__askEingabe{:016x}", rand_u64()));

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish()
}

/// Was `watch` zusätzlich zu Laden, Konsole und Absturz beobachtet
#[derive(Clone, Copy, Default)]
pub struct Watch {
    /// Bilder laufend mitschneiden (Page.startScreencast), für die Live-Bilder des unsichtbaren Fensters
    pub screencast: bool,
    /// Klicks und Tasten der Person melden (PageEvent::Input), für den Browser-Bereich
    pub input: bool,
}

/// Skript in jeder Seite: echte Klicks und Tasten (auch die über das DevTools-Protokoll, wie in Electron
/// before-input-event; `browse` unterscheidet sie am Zeitpunkt) an die Bindung melden
fn input_script(binding: &str) -> String {
    format!(r#"(() => {{
  const send = globalThis['{binding}'];
  if (typeof send !== 'function' || globalThis.__askEingabeAktiv) return;
  globalThis.__askEingabeAktiv = true;
  addEventListener('mousedown', (e) => {{ if (e.isTrusted) send('mouseDown'); }}, true);
  addEventListener('keydown', (e) => {{ if (e.isTrusted) send(e.key && e.key.length === 1 ? 'keyDown' : 'rawKeyDown'); }}, true);
}})();"#)
}

/// Nur der Ordner, den visual.rs freigegeben hat, ist für Navigationen der unsichtbaren Fenster erlaubt
static FILE_ROOTS: ByLabel<String> = LazyLock::new(Default::default);

/// Navigation fertig (Fehler: None = geladen)
pub fn page_loaded(label: &str, error: Option<LoadError>) {
    {
        let mut l = LOADS.lock();
        let e = l.entry(label.to_string()).or_insert((0, None));
        e.0 += 1;
        e.1 = error;
    }
    LOADED.notify_waiters();
}

fn load_gen(label: &str) -> u64 {
    LOADS.lock().get(label).map(|e| e.0).unwrap_or(0)
}

/// Ereignis an die Abonnenten einer Webansicht
pub fn page_event(label: &str, event: PageEvent) {
    let list: Vec<PageListener> = LISTENERS.lock().get(label).map(|l| l.iter().map(|(_, f)| f.clone()).collect()).unwrap_or_default();
    for f in list {
        f(&event);
    }
}

/// Darf diese Webansicht dorthin navigieren?
pub fn navigation_allowed(label: &str, url: &str) -> bool {
    let root = if label == views::BROWSER {
        shell().views.browser_file_root.lock().clone()
    } else {
        FILE_ROOTS.lock().get(label).cloned().unwrap_or_default()
    };
    ask_core::visual::allowed_navigation(url, &root)
}

/// Konsole, Absturz und Ladeende einer Webansicht melden (Windows: WebView2-Ereignisse); dazu nach `opts`
/// Screencast und Eingaben der Person
pub fn watch(wv: &Webview<Wry>, opts: Watch) {
    #[cfg(windows)]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        use webview2_com::{take_pwstr, DevToolsProtocolEventReceivedEventHandler, NavigationCompletedEventHandler, ProcessFailedEventHandler};
        let label = wv.label().to_string();
        let _ = wv.with_webview(move |pw| unsafe {
            let Ok(core) = pw.controller().CoreWebView2() else { return };
            let mut token = 0i64;
            let l1 = label.clone();
            let _ = core.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(move |_s, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut ok = windows::core::BOOL::default();
                    args.IsSuccess(&mut ok)?;
                    let error = if ok.as_bool() {
                        None
                    } else {
                        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                        args.WebErrorStatus(&mut status)?;
                        Some(if status == COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                            LoadError::Aborted
                        } else {
                            LoadError::Failed(crate::platform::web_error_text(status.0))
                        })
                    };
                    page_loaded(&l1, error);
                    Ok(())
                })),
                &mut token,
            );
            let l2 = label.clone();
            let _ = core.add_ProcessFailed(
                &ProcessFailedEventHandler::create(Box::new(move |_s, _a| {
                    page_event(&l2, PageEvent::Gone);
                    Ok(())
                })),
                &mut token,
            );
            // Ereignisse des DevTools-Protokolls: Name → Verarbeitung der Parameter
            let on = |name: &str, f: Box<dyn Fn(Value)>| {
                if let Ok(receiver) = core.GetDevToolsProtocolEventReceiver(&windows::core::HSTRING::from(name)) {
                    let mut t = 0i64;
                    let _ = receiver.add_DevToolsProtocolEventReceived(
                        &DevToolsProtocolEventReceivedEventHandler::create(Box::new(move |_s, args| {
                            let Some(args) = args else { return Ok(()) };
                            let mut json = windows::core::PWSTR::null();
                            args.ParameterObjectAsJson(&mut json)?;
                            if let Ok(v) = serde_json::from_str::<Value>(&take_pwstr(json)) {
                                f(v);
                            }
                            Ok(())
                        })),
                        &mut t,
                    );
                }
            };
            let l3 = label.clone();
            on("Runtime.consoleAPICalled", Box::new(move |v| page_event(&l3, console_event(&v))));
            if opts.input {
                let l4 = label.clone();
                on("Runtime.bindingCalled", Box::new(move |v| {
                    if v.get("name").and_then(Value::as_str) == Some(INPUT_BINDING.as_str()) {
                        let kind = v.get("payload").and_then(Value::as_str).unwrap_or("");
                        if matches!(kind, "mouseDown" | "keyDown" | "rawKeyDown") {
                            page_event(&l4, PageEvent::Input { kind: kind.to_string() });
                        }
                    }
                }));
            }
            if opts.screencast {
                let l5 = label.clone();
                on("Page.screencastFrame", Box::new(move |v| {
                    use base64::Engine;
                    if let Some(data) = v.get("data").and_then(Value::as_str) {
                        if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                            PAINTS.lock().insert(l5.clone(), (bytes, true));
                        }
                    }
                    // Jedes Bild bestätigen, sonst schickt WebView2 keine weiteren
                    let session = v.get("sessionId").cloned().unwrap_or(Value::Null);
                    let l = l5.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(wv) = shell().app.get_webview(&l) {
                            let _ = cdp::call(&wv, "Page.screencastFrameAck", json!({ "sessionId": session })).await;
                        }
                    });
                }));
            }
        });
        let wv = wv.clone();
        tauri::async_runtime::spawn(async move {
            let _ = cdp::call(&wv, "Runtime.enable", json!({})).await;
            if opts.input {
                let binding = INPUT_BINDING.as_str();
                let script = input_script(binding);
                let _ = cdp::call(&wv, "Runtime.addBinding", json!({ "name": binding })).await;
                let _ = cdp::call(&wv, "Page.addScriptToEvaluateOnNewDocument", json!({ "source": script })).await;
                let _ = cdp::eval(&wv, &script).await; // die schon geladene Seite
            }
            if opts.screencast {
                // Live-Bilder wie in Electron: höchstens 960 Pixel breit, JPEG mit Qualität 80
                let _ = cdp::call(&wv, "Page.startScreencast", json!({ "format": "jpeg", "quality": 80, "maxWidth": 960, "maxHeight": 600, "everyNthFrame": 1 })).await;
            }
        });
    }
    #[cfg(not(windows))]
    let _ = wv;
}

#[cfg_attr(not(windows), allow(dead_code))]
fn console_event(v: &Value) -> PageEvent {
    let level = match v.get("type").and_then(Value::as_str).unwrap_or("log") {
        "error" | "assert" => "error",
        "warning" => "warning",
        "debug" => "verbose",
        _ => "info",
    };
    let message = v.get("args").and_then(Value::as_array).map(|a| {
        a.iter().map(|x| x.get("value").map(ask_core::util::js_string).or_else(|| x.get("description").and_then(Value::as_str).map(str::to_string)).unwrap_or_default())
            .collect::<Vec<_>>().join(" ")
    }).unwrap_or_default();
    let frame = v.pointer("/stackTrace/callFrames/0");
    PageEvent::Console {
        level: level.into(),
        message,
        line: frame.and_then(|f| f.get("lineNumber")).and_then(Value::as_i64).map(|n| n + 1).unwrap_or(0),
        source: frame.and_then(|f| f.get("url")).and_then(Value::as_str).unwrap_or("").into(),
    }
}

pub struct WebPage {
    label: String,
    /// Browser-Bereich der App (nie schliessen)
    pane: bool,
}

impl WebPage {
    fn webview(&self) -> Result<Webview<Wry>> {
        shell().app.get_webview(&self.label).ok_or_else(|| anyhow!("Die Seite ist geschlossen."))
    }
}

#[async_trait]
impl Page for WebPage {
    async fn load_url(&self, url: &str) -> Result<(), LoadError> {
        let wv = self.webview().map_err(|e| LoadError::Failed(e.to_string()))?;
        let target = url::Url::parse(url).map_err(|e| LoadError::Failed(e.to_string()))?;
        let before = load_gen(&self.label);
        wv.navigate(target).map_err(|e| LoadError::Failed(e.to_string()))?;
        loop {
            let notified = LOADED.notified();
            if let Some((gen, err)) = LOADS.lock().get(&self.label).cloned() {
                if gen > before {
                    return match err {
                        None => Ok(()),
                        Some(e) => Err(e),
                    };
                }
            }
            // Ohne Meldung (Linux) reicht der Zustand der Seite
            if !cfg!(windows) {
                tokio::time::sleep(Duration::from_millis(200)).await;
                if wv.url().map(|u| u.as_str() == url).unwrap_or(false) {
                    return Ok(());
                }
                continue;
            }
            notified.await;
        }
    }

    fn url(&self) -> String {
        self.webview().ok().and_then(|w| w.url().ok()).map(|u| u.to_string()).unwrap_or_default()
    }

    fn is_closed(&self) -> bool {
        self.webview().is_err()
    }

    fn visible(&self) -> bool {
        self.pane
    }

    async fn eval(&self, js: &str) -> Result<Value> {
        cdp::eval(&self.webview()?, js).await
    }

    async fn screenshot_png(&self) -> Result<Vec<u8>> {
        cdp::screenshot(&self.webview()?).await
    }

    fn take_paint(&self) -> Option<Vec<u8>> {
        let mut paints = PAINTS.lock();
        let entry = paints.get_mut(&self.label)?;
        if !entry.1 {
            return None;
        }
        entry.1 = false;
        Some(entry.0.clone())
    }

    fn last_paint(&self) -> Option<Vec<u8>> {
        PAINTS.lock().get(&self.label).map(|e| e.0.clone())
    }

    async fn mouse(&self, kind: MouseKind, x: f64, y: f64, clicks: u32) -> Result<()> {
        let k = match kind { MouseKind::Move => "mouseMoved", MouseKind::Down => "mousePressed", MouseKind::Up => "mouseReleased" };
        let button = if kind == MouseKind::Move { "none" } else { "left" };
        cdp::mouse(&self.webview()?, k, x, y, button, clicks, (0.0, 0.0)).await
    }

    async fn key(&self, e: KeyEvent) -> Result<()> {
        let kind = match e.kind {
            KeyKind::Down if !e.text.is_empty() => "keyDown",
            KeyKind::Down => "rawKeyDown",
            KeyKind::Char => "char",
            KeyKind::Up => "keyUp",
        };
        cdp::key(&self.webview()?, kind, &e.key, &e.code, e.key_code, &e.text, e.modifiers).await
    }

    async fn insert_text(&self, text: &str) -> Result<()> {
        cdp::insert_text(&self.webview()?, text).await
    }

    async fn set_viewport(&self, width: u32, height: u32) -> Result<()> {
        cdp::set_viewport(&self.webview()?, width, height).await
    }

    fn set_file_root(&self, root: &str) {
        if self.pane {
            *shell().views.browser_file_root.lock() = root.to_string();
        } else {
            FILE_ROOTS.lock().insert(self.label.clone(), root.to_string());
        }
    }

    fn focus(&self) {
        if let Ok(wv) = self.webview() {
            let _ = wv.set_focus();
        }
    }

    fn give_back(&self) {
        if let Some(ui) = shell().views.webview(views::UI) {
            let _ = ui.set_focus();
        }
    }

    fn subscribe(&self, listener: PageListener) -> u64 {
        let id = SEQ.fetch_add(1, Ordering::SeqCst);
        LISTENERS.lock().entry(self.label.clone()).or_default().push((id, listener));
        id
    }

    fn unsubscribe(&self, id: u64) {
        if let Some(list) = LISTENERS.lock().get_mut(&self.label) {
            list.retain(|(i, _)| *i != id);
        }
    }

    fn close(&self) {
        if self.pane {
            return;
        }
        LISTENERS.lock().remove(&self.label);
        FILE_ROOTS.lock().remove(&self.label);
        LOADS.lock().remove(&self.label);
        PAINTS.lock().remove(&self.label);
        if let Some(w) = shell().app.get_webview_window(&self.label) {
            let _ = w.destroy();
        }
    }
}

/// Fenster ausserhalb des Bildschirms, ohne Taskleiste, ohne Rechte (keine Berechtigung «ui»)
fn hidden_window(label: &str, url: WebviewUrl, incognito: bool, opts: Watch) -> Result<Webview<Wry>> {
    let nav_label = label.to_string();
    let mut b = WebviewWindowBuilder::new(&shell().app, label, url)
        .title("ASK")
        .inner_size(1280.0, 800.0)
        .position(-32000.0, -32000.0)
        .skip_taskbar(true)
        .decorations(false)
        .focused(false)
        .visible(true)
        .on_navigation(move |u| navigation_allowed(&nav_label, u.as_str()))
        .on_new_window(|_u, _f| NewWindowResponse::Deny)
        .on_download(|_w, _e| false);
    if incognito {
        b = b.incognito(true);
    }
    #[cfg(not(windows))]
    {
        let l = label.to_string();
        b = b.on_page_load(move |_w, p| {
            if p.event() == tauri::webview::PageLoadEvent::Finished {
                page_loaded(&l, None);
            }
        });
    }
    let w = b.build()?;
    let wv = shell().app.get_webview(label).ok_or_else(|| anyhow!("Fenster nicht angelegt"))?;
    let _ = w.set_ignore_cursor_events(true);
    crate::platform::setup_browser_view(&wv);
    watch(&wv, opts);
    Ok(wv)
}

pub struct Pages;

#[async_trait]
impl PageProvider for Pages {
    async fn pane(&self) -> Option<Arc<dyn Page>> {
        let views = &shell().views;
        views.ensure_browser()?;
        // Die Oberfläche öffnet den Bereich und meldet Grösse und Sichtbarkeit; höchstens 2 s warten
        for _ in 0..40 {
            if views.browser_ready() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        if !views.browser_ready() {
            return None;
        }
        shell().send("browser:show", Value::Null);
        Some(Arc::new(WebPage { label: views::BROWSER.into(), pane: true }))
    }

    async fn offscreen(&self) -> Result<Arc<dyn Page>> {
        let label = format!("unsichtbar-{}", SEQ.fetch_add(1, Ordering::SeqCst));
        hidden_window(&label, WebviewUrl::External("about:blank".parse()?), true, Watch { screencast: true, input: false })?;
        Ok(Arc::new(WebPage { label, pane: false }))
    }

    async fn render_window(&self, url: &str) -> Result<Arc<dyn Page>> {
        let label = format!("zeichnen-{}", SEQ.fetch_add(1, Ordering::SeqCst));
        let target: url::Url = url.parse()?;
        // Die Seite darf nur ihre eigenen Dateien laden (pdf.html und pdf.js)
        FILE_ROOTS.lock().insert(label.clone(), ask_core::paths::resources(&[]).to_string_lossy().to_string());
        let before = load_gen(&label);
        hidden_window(&label, WebviewUrl::External(target), true, Watch::default())?;
        let page = WebPage { label: label.clone(), pane: false };
        // auf das erste Laden warten
        for _ in 0..150 {
            if load_gen(&label) > before {
                break;
            }
            if !cfg!(windows) && page.eval("document.readyState").await.ok().and_then(|v| v.as_str().map(|s| s == "complete")).unwrap_or(false) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Ok(Arc::new(page))
    }
}
