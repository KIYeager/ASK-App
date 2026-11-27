//! Hauptfenster mit drei Webansichten (Tauri «multiwebview», Feature `unstable`), wie in Electron mit
//! `WebContentsView` (ENTSCHEIDE E9, E42, E60):
//! - «ui»: die eigene Oberfläche (React), füllt das Fenster
//! - «ask»: die ASK-Seite, nur für die Anmeldung sichtbar (eigener Datenordner wie die Partition persist:ask)
//! - «browser»: Browser der App (E27, E29), eigener Datenordner wie persist:ask-browser, erst bei Bedarf
//!
//! Die Oberfläche meldet Lage und Sichtbarkeit der Bereiche in CSS-Pixeln; hier werden sie mit dem
//! Zoom der Oberfläche umgerechnet.

use parking_lot::Mutex;
use serde_json::Value;
use std::path::PathBuf;
use tauri::webview::{NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, Rect, WebviewUrl, Webview, Window, WindowBuilder, Wry};

pub const MAIN: &str = "main";
pub const UI: &str = "ui";
pub const ASK: &str = "ask";
pub const BROWSER: &str = "browser";

/// Zustand des Fensters für die Knöpfe in der Kopfzeile
pub fn window_state(v: &Views) -> Value {
    let max = v.window().and_then(|w| w.is_maximized().ok()).unwrap_or(false);
    serde_json::json!({ "maximized": max })
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Bounds {
    pub fn from_json(v: &Value) -> Option<Self> {
        let n = |k: &str| v.get(k).and_then(Value::as_f64);
        Some(Self { x: n("x")?, y: n("y")?, width: n("width")?, height: n("height")? })
    }
    fn usable(&self) -> bool {
        self.width > 0.0 && self.height > 0.0
    }
}

#[derive(Default)]
struct Layout {
    zoom: f64,
    ask_bounds: Option<Bounds>,
    ask_visible: bool,
    browser_bounds: Option<Bounds>,
    browser_visible: bool,
}

/// Zustand der Ansichten im Hauptfenster
pub struct Views {
    pub app: AppHandle,
    layout: Mutex<Layout>,
    /// Ordner, dessen Dateien der Browser der App öffnen darf (Arbeitsordner, visual.rs)
    pub browser_file_root: Mutex<String>,
    /// Ursprung der ASK-Seite (Berechtigungen nur für diese)
    pub ask_origin: Mutex<String>,
}

/// Datenordner einer Webansicht (Anmeldungen bleiben gespeichert), lokal statt im Roaming-Profil
pub fn data_dir(name: &str) -> PathBuf {
    ask_core::paths::local_app_data().join("ASK-Desktop").join("webview").join(name)
}

const SHORTCUTS: &str = r#"
// Tastenkürzel wie im unsichtbaren Menü der Electron-Fassung
window.addEventListener('keydown', (e) => {
  if (!e.ctrlKey || e.altKey) return;
  let key = '';
  if (e.key === '+' || e.key === '=' ) key = 'zoom-in';
  else if (e.key === '-') key = 'zoom-out';
  else if (e.key === '0') key = 'zoom-reset';
  else if (e.key.toLowerCase() === 'r' && !e.shiftKey) key = 'reload-ask';
  else if (e.shiftKey && e.key.toLowerCase() === 'i') key = 'devtools-ask';
  else if (e.shiftKey && e.key.toLowerCase() === 'j') key = 'devtools-ui';
  else if (e.shiftKey && e.key.toLowerCase() === 'm') key = 'mods-off';
  if (!key) return;
  e.preventDefault();
  window.__TAURI_INTERNALS__.invoke('ipc', { channel: 'ui:shortcut', args: [key] }).catch(() => {});
}, true);
"#;

impl Views {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            layout: Mutex::new(Layout { zoom: 1.0, ..Default::default() }),
            browser_file_root: Mutex::new(String::new()),
            ask_origin: Mutex::new(String::new()),
        }
    }

    pub fn window(&self) -> Option<Window<Wry>> {
        self.app.get_window(MAIN)
    }

    pub fn webview(&self, label: &str) -> Option<Webview<Wry>> {
        self.app.get_webview(label)
    }

    /// Hauptfenster mit der Oberfläche und der (versteckten) ASK-Seite anlegen
    pub fn create(&self, dark: bool) -> tauri::Result<()> {
        let bg = if dark { tauri::window::Color(0x16, 0x19, 0x1b, 0xff) } else { tauri::window::Color(0xff, 0xff, 0xff, 0xff) };
        let window = WindowBuilder::new(&self.app, MAIN)
            .title("ASK")
            .inner_size(1440.0, 920.0)
            .min_inner_size(960.0, 600.0)
            .background_color(bg)
            .visible(false)
            .build()?;
        let size = window.inner_size()?.to_logical::<f64>(window.scale_factor()?);
        let ui_url = match std::env::var("ASK_DEV_UI") {
            Ok(u) if cfg!(debug_assertions) && is_local_dev_url(&u) => WebviewUrl::External(u.parse().expect("URL")),
            _ => WebviewUrl::App("index.html".into()),
        };
        let main = window.clone();
        let ui = WebviewBuilder::new(UI, ui_url)
            .initialization_script(SHORTCUTS)
            .on_navigation(|url| {
                // Die Oberfläche navigiert nie weg (wie will-navigate → preventDefault)
                matches!(url.scheme(), "tauri" | "http" | "https") && is_own_ui(url)
            })
            .on_new_window(|_url, _features| NewWindowResponse::Deny)
            .on_page_load(move |_wv, payload| {
                if payload.event() == PageLoadEvent::Finished {
                    let _ = main.show();
                } else if let Some(core) = crate::shell().core_opt() {
                    // Oberfläche lädt neu: offene Dialoge gelten als abgebrochen
                    core.ui_reloading();
                }
            })
            .auto_resize();
        window.add_child(ui, LogicalPosition::new(0.0, 0.0), size)?;
        self.create_ask(&window)?;
        // Ohne Rahmen von Windows (E73): Titel und Knöpfe zeichnet die Oberfläche. Erst jetzt abschalten, damit
        // Tauri die Ränder zum Ziehen der Grösse über den Webansichten anlegt (nur beim Umschalten für das Fenster).
        let _ = window.set_decorations(false);
        let _ = window.set_shadow(true);
        Ok(())
    }

    fn create_ask(&self, window: &Window<Wry>) -> tauri::Result<()> {
        let app = self.app.clone();
        let ask = WebviewBuilder::new(ASK, WebviewUrl::External("about:blank".parse().expect("URL")))
            .data_directory(data_dir("ask"))
            .on_new_window(move |url, _features| {
                // Links aus ASK in neuen Fenstern: im Standardbrowser
                if matches!(url.scheme(), "http" | "https") {
                    use tauri_plugin_opener::OpenerExt;
                    let _ = app.opener().open_url(url.as_str(), None::<&str>);
                }
                NewWindowResponse::Deny
            })
            .on_page_load(|wv, payload| {
                crate::shell().ask_page_load(&wv, payload.event() == PageLoadEvent::Finished, payload.url().as_str());
            });
        let wv = window.add_child(ask, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0))?;
        let _ = wv.hide();
        crate::platform::setup_ask_view(&wv);
        Ok(())
    }

    /// Browser der App anlegen, falls es ihn noch nicht gibt
    pub fn ensure_browser(&self) -> Option<Webview<Wry>> {
        if let Some(wv) = self.webview(BROWSER) {
            return Some(wv);
        }
        let window = self.window()?;
        let builder = WebviewBuilder::new(BROWSER, WebviewUrl::External("about:blank".parse().expect("URL")))
            .data_directory(data_dir("browser"))
            .on_navigation(|url| crate::pages::navigation_allowed(BROWSER, url.as_str()))
            .on_new_window(|url, features| {
                if !matches!(url.scheme(), "http" | "https") {
                    return NewWindowResponse::Deny;
                }
                // Echte Popups (Anmeldung über Microsoft usw.) als eigenes Fenster, neue Tabs in derselben Ansicht
                if features.size().is_some() {
                    return NewWindowResponse::Allow;
                }
                if let Some(wv) = crate::shell().views.webview(BROWSER) {
                    let _ = wv.navigate(url);
                }
                NewWindowResponse::Deny
            })
            .on_download(|_wv, event| {
                if let tauri::webview::DownloadEvent::Requested { url, .. } = event {
                    let name = url.path_segments().and_then(|mut s| s.next_back().map(str::to_string)).unwrap_or_default();
                    crate::shell().send("browser:notice", Value::String(format!("Download blockiert: {name}")));
                }
                false
            })
            .on_page_load(|wv, payload| {
                if !cfg!(windows) && payload.event() == PageLoadEvent::Finished {
                    crate::pages::page_loaded(BROWSER, None);
                }
                crate::shell().browser_state(&wv)
            });
        let wv = window.add_child(builder, LogicalPosition::new(0.0, 0.0), LogicalSize::new(10.0, 10.0)).ok()?;
        let _ = wv.set_background_color(Some(tauri::window::Color(0, 0, 0, 255)));
        let _ = wv.hide();
        crate::platform::setup_browser_view(&wv);
        crate::pages::watch(&wv, crate::pages::Watch { screencast: false, input: true });
        self.apply_browser_layout();
        Some(wv)
    }

    pub fn zoom(&self) -> f64 {
        self.layout.lock().zoom
    }

    /// Zoom der Oberfläche (Ctrl +/-/0); die Bereiche liegen in Pixeln darüber und werden neu gesetzt
    pub fn set_zoom(&self, delta: f64) {
        let z = {
            let mut l = self.layout.lock();
            // Electron: Zoomstufe ±0.5 entspricht Faktor 1.2^0.5
            l.zoom = if delta == 0.0 { 1.0 } else { (l.zoom * 1.2f64.powf(delta)).clamp(0.5, 3.0) };
            l.zoom
        };
        if let Some(ui) = self.webview(UI) {
            let _ = ui.set_zoom(z);
        }
        self.apply_ask_layout();
        self.apply_browser_layout();
    }

    pub fn set_ask_bounds(&self, b: Option<Bounds>) {
        self.layout.lock().ask_bounds = b;
        self.apply_ask_layout();
    }

    pub fn set_ask_visible(&self, v: bool) {
        self.layout.lock().ask_visible = v;
        self.apply_ask_layout();
    }

    pub fn set_browser_bounds(&self, b: Option<Bounds>) {
        self.layout.lock().browser_bounds = b;
        self.apply_browser_layout();
    }

    pub fn set_browser_visible(&self, v: bool) {
        self.layout.lock().browser_visible = v;
        self.apply_browser_layout();
    }

    fn place(&self, wv: &Webview<Wry>, bounds: Option<Bounds>, visible: bool) {
        let z = self.zoom();
        match bounds {
            Some(b) if visible && b.usable() => {
                let _ = wv.set_bounds(Rect {
                    position: LogicalPosition::new(b.x * z, b.y * z).into(),
                    size: LogicalSize::new(b.width * z, b.height * z).into(),
                });
                let _ = wv.show();
            }
            _ => {
                let _ = wv.hide();
            }
        }
    }

    pub fn apply_ask_layout(&self) {
        let (b, v) = { let l = self.layout.lock(); (l.ask_bounds, l.ask_visible) };
        if let Some(wv) = self.webview(ASK) {
            self.place(&wv, b, v);
        }
    }

    pub fn apply_browser_layout(&self) {
        let (b, v) = { let l = self.layout.lock(); (l.browser_bounds, l.browser_visible) };
        if let Some(wv) = self.webview(BROWSER) {
            self.place(&wv, b, v);
        }
    }

    /// Ist der Browser-Bereich sichtbar und gross genug? (sonst testet visual.rs unsichtbar)
    pub fn browser_ready(&self) -> bool {
        let (b, v) = { let l = self.layout.lock(); (l.browser_bounds, l.browser_visible) };
        let win_ok = self.window().is_some_and(|w| w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(true));
        self.webview(BROWSER).is_some() && v && b.is_some_and(|b| b.width >= 200.0 && b.height >= 150.0) && win_ok
    }
}

fn is_local_dev_url(u: &str) -> bool {
    url::Url::parse(u).is_ok_and(|p| p.scheme() == "http" && matches!(p.host_str(), Some("localhost" | "127.0.0.1")))
}

/// Adresse der eigenen Oberfläche (gebaut: tauri://localhost bzw. http://tauri.localhost, Entwicklung: Vite)
pub fn is_own_ui_url(url: &url::Url) -> bool {
    matches!(url.scheme(), "tauri" | "http" | "https") && is_own_ui(url)
}

fn is_own_ui(url: &url::Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "tauri.localhost" | "127.0.0.1"))
}
