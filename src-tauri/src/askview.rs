//! Die ASK-Seite (Webansicht «ask») für askapi: Token aus localStorage lesen, Cookies löschen, laden.
//! In Electron war das der WebContentsView mit der Partition persist:ask.

use crate::{cdp, shell, views};
use ask_core::askapi::AskView;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::Manager;

/// Lädt die ASK-Seite gerade? (Tauri meldet Beginn und Ende über on_page_load)
pub static LOADING: AtomicBool = AtomicBool::new(false);

pub struct AskViewImpl;

#[async_trait]
impl AskView for AskViewImpl {
    async fn eval(&self, js: &str) -> anyhow::Result<Value> {
        let wv = shell().views.webview(views::ASK).ok_or_else(|| anyhow::anyhow!("ASK-Seite nicht vorhanden"))?;
        cdp::eval(&wv, js).await
    }

    fn url(&self) -> String {
        shell().views.webview(views::ASK).and_then(|w| w.url().ok()).map(|u| u.to_string()).unwrap_or_default()
    }

    fn is_loading(&self) -> bool {
        LOADING.load(Ordering::SeqCst)
    }

    fn is_destroyed(&self) -> bool {
        shell().views.webview(views::ASK).is_none()
    }

    fn load_url(&self, url: &str) {
        if let (Some(wv), Ok(u)) = (shell().views.webview(views::ASK), url::Url::parse(url)) {
            *shell().views.ask_origin.lock() = u.origin().ascii_serialization();
            LOADING.store(true, Ordering::SeqCst);
            let _ = wv.navigate(u);
        }
    }

    async fn clear_site_data(&self, origin: &str) {
        if let Some(wv) = shell().views.webview(views::ASK) {
            let _ = cdp::call(&wv, "Storage.clearDataForOrigin", json!({ "origin": origin, "storageTypes": "cookies,local_storage" })).await;
        }
    }
}

/// Fenster «Bei ASK anmelden» (ENTSCHEIDE E57): dieselbe Sitzung wie die ASK-Seite (Datenordner «ask»), für den Fall,
/// dass die Seite im Hauptfenster nicht erscheint. askapi liest dort das Token (`watch_login`).
pub const LOGIN: &str = "ask-login";

pub struct LoginView;

#[async_trait]
impl AskView for LoginView {
    async fn eval(&self, js: &str) -> anyhow::Result<Value> {
        let wv = shell().app.get_webview(LOGIN).ok_or_else(|| anyhow::anyhow!("Anmeldefenster geschlossen"))?;
        cdp::eval(&wv, js).await
    }

    fn url(&self) -> String {
        shell().app.get_webview(LOGIN).and_then(|w| w.url().ok()).map(|u| u.to_string()).unwrap_or_default()
    }

    fn is_loading(&self) -> bool {
        false
    }

    fn is_destroyed(&self) -> bool {
        shell().app.get_webview(LOGIN).is_none()
    }

    fn load_url(&self, url: &str) {
        if let (Some(wv), Ok(u)) = (shell().app.get_webview(LOGIN), url::Url::parse(url)) {
            let _ = wv.navigate(u);
        }
    }

    async fn clear_site_data(&self, _origin: &str) {}
}

/// Anmeldefenster öffnen (oder nach vorne holen) und das Token darin beobachten
pub fn open_login(ask_url: &str) -> bool {
    use tauri::webview::{NewWindowResponse, WebviewWindowBuilder};
    let s = shell();
    if let Some(w) = s.app.get_webview_window(LOGIN) {
        let _ = w.set_focus();
        return true;
    }
    let Ok(target) = url::Url::parse(&format!("{}/auth", ask_url.trim_end_matches('/'))) else { return false };
    let app = s.app.clone();
    let b = WebviewWindowBuilder::new(&s.app, LOGIN, tauri::WebviewUrl::External(target))
        .title("Bei ASK anmelden")
        .inner_size(560.0, 780.0)
        .min_inner_size(420.0, 500.0)
        .data_directory(views::data_dir("ask"))
        .on_new_window(move |url, _f| {
            // Popups wie im Hauptfenster in den normalen Browser
            if matches!(url.scheme(), "http" | "https") {
                use tauri_plugin_opener::OpenerExt;
                let _ = app.opener().open_url(url.as_str(), None::<&str>);
            }
            NewWindowResponse::Deny
        });
    if b.build().is_err() {
        return false;
    }
    if let Some(core) = s.core_opt() {
        core.ask.watch_login(std::sync::Arc::new(LoginView));
    }
    true
}

/// Im Anmeldefenster angemeldet: Fenster zu, ASK-Seite im Hauptfenster neu laden (hat jetzt das Token)
pub fn login_done() {
    let s = shell();
    if let Some(w) = s.app.get_webview_window(LOGIN) {
        let _ = w.close();
    }
    let url = ask_core::settings::get("askUrl").as_str().unwrap_or("").to_string();
    if !url.is_empty() {
        AskViewImpl.load_url(&url);
    }
}
