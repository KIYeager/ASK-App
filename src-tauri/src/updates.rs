//! Updates mit tauri-plugin-updater (ENTSCHEIDE E60): signierte Pakete aus dem öffentlichen Repository
//! KIYeager/ASK-Releases. Der Release-Workflow legt dort im Release «updater» zwei Dateien ab:
//! latest.json (nur normale Versionen) und beta.json (die neueste, auch Vorabversionen). Den Ablauf
//! (Fragen, Countdown, Aufschieben) steuert ask_core::updater.

use anyhow::{anyhow, Result};
use ask_core::updater::{UpdateBackend, UpdateInfo};
use async_trait::async_trait;
use parking_lot::Mutex;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

pub fn manifest_url(owner: &str, repo: &str, beta: bool) -> String {
    let file = if beta { "beta.json" } else { "latest.json" };
    format!("https://github.com/{owner}/{repo}/releases/download/updater/{file}")
}

pub struct TauriUpdates {
    app: AppHandle,
    owner: String,
    repo: String,
    found: Mutex<Option<Update>>,
    bytes: Mutex<Option<Vec<u8>>>,
}

impl TauriUpdates {
    pub fn new(app: AppHandle, owner: &str, repo: &str) -> Self {
        Self { app, owner: owner.into(), repo: repo.into(), found: Mutex::new(None), bytes: Mutex::new(None) }
    }
}

#[async_trait]
impl UpdateBackend for TauriUpdates {
    async fn check(&self, beta: bool) -> Result<Option<UpdateInfo>> {
        let url = manifest_url(&self.owner, &self.repo, beta).parse()?;
        let updater = self.app.updater_builder().endpoints(vec![url])?.build()?;
        let update = updater.check().await.map_err(|e| anyhow!("{e}"))?;
        let info = update.as_ref().map(|u| UpdateInfo { version: u.version.clone(), notes: u.body.clone().unwrap_or_default() });
        *self.found.lock() = update;
        Ok(info)
    }

    async fn download(&self, progress: Box<dyn Fn(u32) + Send + Sync>) -> Result<()> {
        let update = self.found.lock().clone().ok_or_else(|| anyhow!("Kein Update gefunden."))?;
        let mut done = 0usize;
        let bytes = update
            .download(
                |chunk, total| {
                    done += chunk;
                    if let Some(t) = total.filter(|t| *t > 0) {
                        progress(((done as f64 / t as f64) * 100.0).round() as u32);
                    }
                },
                || {},
            )
            .await
            .map_err(|e| anyhow!("{e}"))?;
        *self.bytes.lock() = Some(bytes);
        Ok(())
    }

    async fn install(&self) -> Result<()> {
        let update = self.found.lock().clone().ok_or_else(|| anyhow!("Kein Update gefunden."))?;
        let bytes = self.bytes.lock().take().ok_or_else(|| anyhow!("Das Update ist nicht heruntergeladen."))?;
        // Unter Windows startet das den Installer (sichtbar, «passive») und beendet ASK; er startet ASK danach neu.
        update.install(bytes).map_err(|e| anyhow!("{e}"))?;
        self.app.restart();
    }
}
