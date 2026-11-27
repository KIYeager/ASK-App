//! Schnittstelle zur Tauri-Hülle: alles, was ein Fenster, Windows-Dialoge oder das System braucht.
//! Die Hülle (src-tauri/src) setzt `Host` um; Tests nehmen `NullHost` oder eine eigene Attrappe.
//! Webansichten (ASK-Anmeldeseite, Browser der App, unsichtbares Fenster zum Zeichnen) haben eigene
//! Schnittstellen in askapi.rs (`AskView`) und visual.rs (`Page`, `PageProvider`).

use async_trait::async_trait;
use futures::future::BoxFuture;
use serde_json::{json, Value};
use std::sync::Arc;

/// Nachricht an die Oberfläche (in Electron `webContents.send(channel, payload)`)
pub type SendFn = Arc<dyn Fn(&str, Value) + Send + Sync>;

/// Optionen einer Rückfrage (main.js `confirm`)
#[derive(Clone, Debug, Default)]
pub struct ConfirmOptions {
    /// Häkchen im Dialog («nicht mehr fragen»)
    pub checkbox: Option<String>,
    /// «Ja» rot (Löschen, nicht rückgängig)
    pub danger: bool,
}

/// Antwort einer Rückfrage. Ohne Häkchen ist `checked` immer false.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Confirmed {
    pub ok: bool,
    pub checked: bool,
}

/// Rückfrage vor Aktionen nach aussen: `confirm(message, detail, options)`
pub type ConfirmFn = Arc<dyn Fn(String, String, ConfirmOptions) -> BoxFuture<'static, Confirmed> + Send + Sync>;

/// Rückfrage, die immer gleich antwortet (Tests, automatische Tests mit ASK_TEST_AUTOCONFIRM)
pub fn fixed_confirm(answer: Confirmed) -> ConfirmFn {
    Arc::new(move |_m, _d, _o| Box::pin(async move { answer }))
}

/// Dateifilter für Dialoge: (Name, Endungen ohne Punkt)
pub type Filters = Vec<(String, Vec<String>)>;

#[async_trait]
pub trait Host: Send + Sync + 'static {
    /// Nachricht an die Oberfläche
    fn send(&self, channel: &str, payload: Value);
    /// Meldung von Windows (nur solange die eigene Oberfläche nicht geladen ist). Optionen wie
    /// `dialog.showMessageBox`; Ergebnis `{ response, checkboxChecked }`.
    async fn native_box(&self, options: Value) -> Value;
    /// Datei oder Ordner mit dem zugehörigen Programm öffnen; Ergebnis: Fehlertext oder leer
    async fn open_path(&self, path: &str) -> String;
    fn show_item_in_folder(&self, path: &str);
    fn open_external(&self, url: &str);
    fn clipboard_write(&self, text: &str);
    async fn pick_folder(&self, title: &str, default_path: &str) -> Option<String>;
    async fn pick_open_file(&self, title: &str, filters: Filters) -> Option<String>;
    /// Mehrere Dateien wählen (Quellen hinzufügen, ENTSCHEIDE E82); leer: abgebrochen
    async fn pick_open_files(&self, title: &str, filters: Filters) -> Vec<String>;
    async fn pick_save_file(&self, title: &str, default_name: &str, filters: Filters) -> Option<String>;
    /// Windows-Benachrichtigung; on_click läuft beim Klick darauf
    fn notify(&self, title: &str, body: &str, silent: bool, on_click: Box<dyn FnOnce() + Send>);
    fn window_focused(&self) -> bool;
    fn flash_frame(&self, on: bool);
    /// Fortschritt in der Taskleiste: 0..1, 2 = unbestimmt, negativ = aus
    fn set_progress(&self, value: f64);
    /// Fenster nach vorne holen (wiederherstellen, zeigen, fokussieren)
    fn focus_window(&self);
    /// Farbschema: system | light | dark
    fn set_theme(&self, theme: &str);
    fn app_version(&self) -> String;
    fn quit(&self);
}

/// Host ohne Fenster (Tests): schickt nichts, antwortet mit Abbrechen.
#[derive(Default)]
pub struct NullHost {
    pub sent: parking_lot::Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl Host for NullHost {
    fn send(&self, channel: &str, payload: Value) {
        self.sent.lock().push((channel.to_string(), payload));
    }
    async fn native_box(&self, options: Value) -> Value {
        json!({ "response": options.get("cancelId").cloned().unwrap_or(json!(-1)), "checkboxChecked": false })
    }
    async fn open_path(&self, _path: &str) -> String { String::new() }
    fn show_item_in_folder(&self, _path: &str) {}
    fn open_external(&self, _url: &str) {}
    fn clipboard_write(&self, _text: &str) {}
    async fn pick_folder(&self, _t: &str, _d: &str) -> Option<String> { None }
    async fn pick_open_file(&self, _t: &str, _f: Filters) -> Option<String> { None }
    async fn pick_open_files(&self, _t: &str, _f: Filters) -> Vec<String> { Vec::new() }
    async fn pick_save_file(&self, _t: &str, _n: &str, _f: Filters) -> Option<String> { None }
    fn notify(&self, _t: &str, _b: &str, _s: bool, _c: Box<dyn FnOnce() + Send>) {}
    fn window_focused(&self) -> bool { true }
    fn flash_frame(&self, _on: bool) {}
    fn set_progress(&self, _v: f64) {}
    fn focus_window(&self) {}
    fn set_theme(&self, _t: &str) {}
    fn app_version(&self) -> String { env!("CARGO_PKG_VERSION").to_string() }
    fn quit(&self) {}
}
