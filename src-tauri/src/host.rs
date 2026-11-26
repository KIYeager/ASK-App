//! Umsetzung von `ask_core::host::Host` mit Tauri: Oberfläche, Dialoge von Windows, Explorer,
//! Zwischenablage, Benachrichtigungen, Fenster.

use crate::views;
use ask_core::host::{Filters, Host};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::atomic::AtomicBool;
use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;

/// Die App wird beendet (RunEvent::ExitRequested nur einmal behandeln)
pub static QUITTING: AtomicBool = AtomicBool::new(false);

pub struct HostImpl {
    app: AppHandle,
}

impl HostImpl {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn window(&self) -> Option<tauri::Window> {
        self.app.get_window(views::MAIN)
    }
}

fn filters_of(dialog: tauri_plugin_dialog::FileDialogBuilder<tauri::Wry>, filters: &Filters) -> tauri_plugin_dialog::FileDialogBuilder<tauri::Wry> {
    let mut d = dialog;
    for (name, exts) in filters {
        let e: Vec<&str> = exts.iter().map(String::as_str).collect();
        d = d.add_filter(name, &e);
    }
    d
}

#[async_trait]
impl Host for HostImpl {
    fn send(&self, channel: &str, payload: Value) {
        crate::shell().send(channel, payload);
    }

    async fn native_box(&self, options: Value) -> Value {
        // Windows-Meldung, solange die eigene Oberfläche nicht bereit ist. Sie kennt nur zwei Knöpfe:
        // der erste ist «Ja», alles andere gilt als Abbrechen.
        let s = |k: &str| options.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let buttons: Vec<String> = options.get("buttons").and_then(Value::as_array)
            .map(|a| a.iter().filter_map(|b| b.as_str().map(str::to_string)).collect()).unwrap_or_default();
        let cancel = options.get("cancelId").and_then(Value::as_i64).unwrap_or(buttons.len().saturating_sub(1) as i64);
        let kind = match s("type").as_str() { "warning" => MessageDialogKind::Warning, "error" => MessageDialogKind::Error, _ => MessageDialogKind::Info };
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut d = self.app.dialog().message(s("detail")).title(s("message")).kind(kind);
        d = match buttons.as_slice() {
            [] | [_] => d.buttons(MessageDialogButtons::OkCustom(buttons.first().cloned().unwrap_or_else(|| "OK".into()))),
            [a, b, ..] => d.buttons(MessageDialogButtons::OkCancelCustom(a.clone(), b.clone())),
        };
        if let Some(w) = self.window() {
            d = d.parent(&w);
        }
        d.show(move |ok| { let _ = tx.send(ok); });
        let ok = rx.await.unwrap_or(false);
        json!({ "response": if ok { 0 } else { cancel }, "checkboxChecked": false })
    }

    async fn open_path(&self, path: &str) -> String {
        match self.app.opener().open_path(path, None::<&str>) {
            Ok(()) => String::new(),
            Err(e) => e.to_string(),
        }
    }

    fn show_item_in_folder(&self, path: &str) {
        let _ = self.app.opener().reveal_item_in_dir(path);
    }

    fn open_external(&self, url: &str) {
        // Nur Webadressen ins System geben: file:, search-ms:, ms-msdt: und andere Protokolle starten Programme
        if url.starts_with("http://") || url.starts_with("https://") {
            let _ = self.app.opener().open_url(url, None::<&str>);
        }
    }

    fn clipboard_write(&self, text: &str) {
        let _ = self.app.clipboard().write_text(text.to_string());
    }

    async fn pick_folder(&self, title: &str, default_path: &str) -> Option<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut d = self.app.dialog().file().set_title(title);
        if !default_path.is_empty() {
            d = d.set_directory(default_path);
        }
        if let Some(w) = self.window() {
            d = d.set_parent(&w);
        }
        d.pick_folder(move |p| { let _ = tx.send(p.and_then(|p| p.into_path().ok())); });
        rx.await.ok().flatten().map(|p| p.to_string_lossy().to_string())
    }

    async fn pick_open_file(&self, title: &str, filters: Filters) -> Option<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut d = filters_of(self.app.dialog().file().set_title(title), &filters);
        if let Some(w) = self.window() {
            d = d.set_parent(&w);
        }
        d.pick_file(move |p| { let _ = tx.send(p.and_then(|p| p.into_path().ok())); });
        rx.await.ok().flatten().map(|p| p.to_string_lossy().to_string())
    }

    async fn pick_save_file(&self, title: &str, default_name: &str, filters: Filters) -> Option<String> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut d = filters_of(self.app.dialog().file().set_title(title).set_file_name(default_name), &filters);
        if let Some(w) = self.window() {
            d = d.set_parent(&w);
        }
        d.save_file(move |p| { let _ = tx.send(p.and_then(|p| p.into_path().ok())); });
        rx.await.ok().flatten().map(|p| p.to_string_lossy().to_string())
    }

    fn notify(&self, title: &str, body: &str, silent: bool, on_click: Box<dyn FnOnce() + Send>) {
        crate::platform::notify(&self.app, title, body, silent, on_click);
    }

    fn window_focused(&self) -> bool {
        self.window().is_some_and(|w| w.is_focused().unwrap_or(false))
    }

    fn flash_frame(&self, on: bool) {
        if let Some(w) = self.window() {
            let _ = w.request_user_attention(if on { Some(tauri::UserAttentionType::Informational) } else { None });
        }
    }

    fn set_progress(&self, value: f64) {
        use tauri::window::{ProgressBarState, ProgressBarStatus};
        let Some(w) = self.window() else { return };
        let state = if value < 0.0 {
            ProgressBarState { status: Some(ProgressBarStatus::None), progress: None }
        } else if value > 1.0 {
            ProgressBarState { status: Some(ProgressBarStatus::Indeterminate), progress: None }
        } else {
            ProgressBarState { status: Some(ProgressBarStatus::Normal), progress: Some((value * 100.0).round() as u64) }
        };
        let _ = w.set_progress_bar(state);
    }

    fn focus_window(&self) {
        if let Some(w) = self.window() {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
    }

    fn set_theme(&self, theme: &str) {
        let t = match theme { "light" => Some(tauri::Theme::Light), "dark" => Some(tauri::Theme::Dark), _ => None };
        self.app.set_theme(t);
    }

    fn app_version(&self) -> String {
        self.app.package_info().version.to_string()
    }

    fn quit(&self) {
        self.app.exit(0);
    }
}
