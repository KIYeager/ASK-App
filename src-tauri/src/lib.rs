//! ASK Desktop: Tauri-Hülle (Fenster, Webansichten, Dialoge, Benachrichtigungen, Updates).
//! Die Logik liegt in ask-core (src-tauri/core); die Oberfläche in src/renderer.

mod askview;
mod cdp;
mod host;
mod ipc;
mod platform;
mod protocol;
mod updates;
mod views;

use ask_core::app::App;
use serde_json::{json, Value};
use std::sync::{Arc, OnceLock};
use tauri::{AppHandle, Emitter, Manager, RunEvent, Webview, Wry};

/// Alles, was die Hülle zur Laufzeit braucht. Es gibt genau eine.
pub struct Shell {
    pub app: AppHandle,
    pub views: views::Views,
    core: OnceLock<Arc<App>>,
}

static SHELL: OnceLock<Shell> = OnceLock::new();

/// Die Hülle (nach dem Start immer vorhanden)
pub fn shell() -> &'static Shell {
    SHELL.get().expect("Hülle gestartet")
}

impl Shell {
    pub fn core(&self) -> &Arc<App> {
        self.core.get().expect("App gestartet")
    }

    pub fn core_opt(&self) -> Option<&Arc<App>> {
        self.core.get()
    }

    /// Nachricht an die eigene Oberfläche
    pub fn send(&self, channel: &str, payload: Value) {
        let _ = self.app.emit_to(tauri::EventTarget::webview(views::UI), channel, payload);
    }

    /// ASK-Seite geladen oder Laden begonnen (askapi liest danach das Token)
    pub fn ask_page_load(&self, _wv: &Webview<Wry>, finished: bool, url: &str) {
        if let Some(core) = self.core.get() {
            core.ask_page_load(finished, url);
        }
    }

    /// Adresse und Titel des Browsers an die Oberfläche (did-navigate, page-title-updated usw.)
    pub fn browser_state(&self, wv: &Webview<Wry>) {
        let url = wv.url().map(|u| u.to_string()).unwrap_or_default();
        let wv = wv.clone();
        tauri::async_runtime::spawn(async move {
            let info = cdp::eval(&wv, "({ title: document.title, loading: document.readyState !== 'complete' })")
                .await
                .unwrap_or(Value::Null);
            let (back, forward) = platform::can_go(&wv).await;
            shell().send("browser:state", json!({
                "url": url,
                "title": info.get("title").cloned().unwrap_or(json!("")),
                "back": back,
                "forward": forward,
                "loading": info.get("loading").cloned().unwrap_or(json!(false)),
            }));
        });
    }
}

pub fn run() {
    platform::set_app_user_model_id();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // Zweiter Start: das vorhandene Fenster nach vorne holen
            if let Some(w) = app.get_window(views::MAIN) {
                let _ = w.unminimize();
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_asynchronous_uri_scheme_protocol("askfile", protocol::handle)
        .invoke_handler(tauri::generate_handler![ipc::ipc])
        .setup(|app| {
            setup(app.handle().clone())?;
            Ok(())
        });
    let app = builder.build(tauri::generate_context!()).expect("ASK startet nicht");
    app.run(|handle, event| match event {
        RunEvent::ExitRequested { api, code, .. } => {
            // Beim Beenden laufende Aufträge stoppen und speichern lassen (höchstens 5 s), Terminals beenden
            if code.is_none() && !host::QUITTING.swap(true, std::sync::atomic::Ordering::SeqCst) {
                api.prevent_exit();
                let handle = handle.clone();
                tauri::async_runtime::spawn(async move {
                    if let Some(s) = SHELL.get() {
                        if let Some(core) = s.core.get() {
                            core.shutdown().await;
                        }
                    }
                    handle.exit(0);
                });
            }
        }
        RunEvent::WindowEvent { label, event: tauri::WindowEvent::Focused(true), .. } if label == views::MAIN => {
            if let Some(w) = handle.get_window(views::MAIN) {
                let _ = w.request_user_attention(None);
            }
        }
        _ => {}
    });
}

fn setup(app: AppHandle) -> anyhow::Result<()> {
    let packaged = !cfg!(debug_assertions);
    let resources_dir = if packaged {
        app.path().resource_dir()?
    } else {
        ask_core::paths::repo_root().join("resources")
    };
    let mut cfg = ask_core::paths::config();
    cfg.packaged = packaged;
    cfg.resources_dir = resources_dir;
    cfg.exe = std::env::current_exe().unwrap_or_default();
    ask_core::paths::init(cfg);

    let new_shell = Shell { app: app.clone(), views: views::Views::new(app.clone()), core: OnceLock::new() };
    if SHELL.set(new_shell).is_err() {
        anyhow::bail!("Hülle doppelt gestartet");
    }
    let host = Arc::new(host::HostImpl::new(app.clone()));
    let core = App::new(host);
    let _ = shell().core.set(core.clone());
    let dark = matches!(ask_core::settings::get("theme").as_str(), Some("dunkel"));
    shell().views.create(dark)?;
    tauri::async_runtime::spawn(async move { core.start().await });
    Ok(())
}
