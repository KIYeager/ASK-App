//! Steuerung der App (früher src/main/main.js): Projekte und ihre Laufzeiten (Terminal, Ordner,
//! Agenten), Zustand für die Oberfläche, alle Aufrufe der Oberfläche (`handle`). Platzhalter, bis
//! die anderen Module portiert sind.

use crate::host::Host;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

pub struct App {
    pub host: Arc<dyn Host>,
}

impl App {
    pub fn new(host: Arc<dyn Host>) -> Arc<Self> {
        Arc::new(Self { host })
    }

    pub async fn start(self: &Arc<Self>) {}

    pub async fn handle(self: &Arc<Self>, channel: &str, _args: Vec<Value>) -> Result<Value, String> {
        Err(format!("Unbekannter Aufruf: {channel}"))
    }

    pub fn resolve_file(&self, _rel: &str) -> Option<PathBuf> {
        None
    }

    pub fn ui_reloading(&self) {}

    /// Die Hülle meldet: ASK-Seite beginnt zu laden (finished=false) oder ist geladen
    pub fn ask_page_load(&self, _finished: bool, _url: &str) {}

    /// Die Hülle meldet: ASK-Seite nicht geladen (ENTSCHEIDE E53)
    pub fn ask_load_failed(&self, _code: i32, _description: &str, _url: &str) {}

    pub async fn shutdown(&self) {}
}
