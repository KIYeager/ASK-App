//! Die ASK-Seite (Webansicht «ask») für askapi: Token aus localStorage lesen, Cookies löschen, laden.
//! In Electron war das der WebContentsView mit der Partition persist:ask.

use crate::{cdp, shell, views};
use ask_core::askapi::AskView;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};

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

    async fn clear_cookies(&self, origin: &str) {
        if let Some(wv) = shell().views.webview(views::ASK) {
            let _ = cdp::call(&wv, "Storage.clearDataForOrigin", json!({ "origin": origin, "storageTypes": "cookies" })).await;
        }
    }
}
