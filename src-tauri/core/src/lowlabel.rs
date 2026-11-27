//! Entfernt die Integritätsmarke «Low» wieder von einem Ordner, der aus der Projektliste genommen wird.
//! Gegenstück zu ensure_low_label in ask_terminal::winproc::ensure_low_label (setzt (OI)(CI)L beim Terminalstart).
//!
//! Öffentliche Schnittstelle:
//! - `restore_label(folder, &RestoreOptions) -> Result<RestoreResult>`
//! - `RestoreOptions::default()` (echter Rechner), `RunFn`, `RunOptions`

use crate::folders::{norm, win_resolve};
use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use std::sync::Arc;
use std::time::Duration;

pub const TIMEOUT_MS: u64 = 10 * 60 * 1000;

/// Optionen des Prozessaufrufs (wie bei `execFile`)
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOptions {
    pub windows_hide: bool,
    pub timeout_ms: u64,
    pub shell: bool,
}

/// Führt ein Programm aus: (Datei, Argumente, Optionen). Fehler, wenn es scheitert.
pub type RunFn = Arc<dyn Fn(String, Vec<String>, RunOptions) -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Standard: das Programm wirklich starten, mit Zeitlimit
pub fn default_run() -> RunFn {
    Arc::new(|file, args, options| {
        Box::pin(async move {
            let mut cmd = tokio::process::Command::new(&file);
            cmd.args(&args).stdin(std::process::Stdio::null()).kill_on_drop(true);
            #[cfg(windows)]
            if options.windows_hide {
                cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
            }
            let out = tokio::time::timeout(Duration::from_millis(options.timeout_ms), cmd.output())
                .await
                .map_err(|_| anyhow!("{file} hat nicht innerhalb der Zeit geantwortet."))??;
            if out.status.success() {
                Ok(())
            } else {
                let stderr = String::from_utf8_lossy(&out.stderr);
                let stdout = String::from_utf8_lossy(&out.stdout);
                let detail = if stderr.trim().is_empty() { stdout } else { stderr };
                Err(anyhow!("Command failed: {file} ({}) {}", out.status, detail.trim()))
            }
        })
    })
}

pub struct RestoreOptions {
    /// Ordner der übrigen Projekte
    pub other_folders: Vec<String>,
    pub run: RunFn,
    /// Läuft die App unter Windows? Sonst gibt es nichts zu tun.
    pub windows: bool,
    pub exists: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    /// Benutzerordner; leer: nicht prüfen
    pub home: String,
}

impl Default for RestoreOptions {
    fn default() -> Self {
        RestoreOptions {
            other_folders: Vec::new(),
            run: default_run(),
            windows: cfg!(windows),
            exists: Arc::new(|p| std::path::Path::new(p).exists()),
            home: crate::paths::home().to_string_lossy().to_string(),
        }
    }
}

/// `skipped: true` mit Grund, wenn nichts zu tun war
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreResult {
    pub skipped: bool,
    pub reason: Option<String>,
}

impl RestoreResult {
    fn skip(reason: impl Into<String>) -> Self {
        RestoreResult { skipped: true, reason: Some(reason.into()) }
    }
}

/// Wurzel eines aufgelösten Windows-Pfads (`C:\` oder `\\server\freigabe\`)
fn root_of(resolved: &str) -> String {
    if let Some(unc) = resolved.strip_prefix("\\\\") {
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        format!("\\\\{server}\\{share}\\")
    } else if resolved.len() >= 3 {
        resolved[..3].to_string()
    } else {
        resolved.to_string()
    }
}

fn is_root(p: &str) -> bool {
    root_of(&win_resolve(p)).trim_end_matches(['\\', '/']).to_lowercase() == norm(p)
}

/// a liegt in b (oder ist b); «C:\a\bc» liegt nicht in «C:\a\b»
fn within(a: &str, b: &str) -> bool {
    a == b || a.starts_with(&format!("{b}\\"))
}

/// Setzt die Marke eines Ordners samt allem darin auf «Medium» (Standard) zurück. /T ist nötig, weil Dateien,
/// die das Low-Terminal angelegt hat, eine eigene Marke tragen und sie nicht vom Ordner erben.
/// Gibt `skipped: true` mit Grund zurück, wenn nichts zu tun ist, sonst `skipped: false`.
pub async fn restore_label(folder: &str, options: &RestoreOptions) -> Result<RestoreResult> {
    if !options.windows {
        return Ok(RestoreResult::skip("nicht Windows"));
    }
    if folder.is_empty() {
        return Ok(RestoreResult::skip("kein Ordner"));
    }
    if !(options.exists)(folder) {
        return Ok(RestoreResult::skip("Ordner existiert nicht mehr"));
    }
    let f = norm(folder);
    if is_root(folder) {
        return Ok(RestoreResult::skip("Laufwerkswurzel"));
    }
    if !options.home.is_empty() && f == norm(&options.home) {
        return Ok(RestoreResult::skip("Benutzerordner"));
    }
    for other in &options.other_folders {
        if other.is_empty() {
            continue;
        }
        let o = norm(other);
        // Mit /T bekämen Dateien eines anderen Projekts eine eigene Medium-Marke.
        if within(&f, &o) || within(&o, &f) {
            return Ok(RestoreResult::skip(format!("liegt im oder enthält das Projekt {other}")));
        }
    }
    let system_root = std::env::var("SystemRoot").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "C:\\Windows".to_string());
    let icacls = format!("{}\\System32\\icacls.exe", system_root.trim_end_matches(['\\', '/']));
    let args = vec![win_resolve(folder), "/setintegritylevel".into(), "(OI)(CI)M".into(), "/T".into(), "/C".into(), "/Q".into()];
    (options.run)(icacls, args, RunOptions { windows_hide: true, timeout_ms: TIMEOUT_MS, shell: false }).await?;
    Ok(RestoreResult { skipped: false, reason: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    type Calls = Arc<Mutex<Vec<(String, Vec<String>, RunOptions)>>>;

    fn setup(windows: bool, exists: bool, other: &[&str]) -> (Calls, RestoreOptions) {
        let calls: Calls = Arc::new(Mutex::new(Vec::new()));
        let c = calls.clone();
        let run: RunFn = Arc::new(move |file, args, options| {
            c.lock().push((file, args, options));
            Box::pin(async { Ok(()) })
        });
        let opts = RestoreOptions {
            other_folders: other.iter().map(|s| s.to_string()).collect(),
            run,
            windows,
            exists: Arc::new(move |_| exists),
            home: "C:\\Users\\anna".to_string(),
        };
        (calls, opts)
    }

    #[tokio::test]
    async fn ruft_icacls_mit_t_ohne_shell_auf() {
        let (calls, opts) = setup(true, true, &[]);
        let r = restore_label("C:\\Arbeit\\Projekt", &opts).await.unwrap();
        assert!(!r.skipped);
        let calls = calls.lock();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].0.to_lowercase().ends_with("system32\\icacls.exe"));
        assert_eq!(calls[0].1, ["C:\\Arbeit\\Projekt", "/setintegritylevel", "(OI)(CI)M", "/T", "/C", "/Q"]);
        assert!(calls[0].2.windows_hide);
        assert!(!calls[0].2.shell);
        assert!(calls[0].2.timeout_ms >= 60000);
    }

    #[tokio::test]
    async fn nicht_unter_windows_nichts_tun() {
        let (calls, opts) = setup(false, true, &[]);
        assert!(restore_label("C:\\Arbeit\\Projekt", &opts).await.unwrap().skipped);
        assert_eq!(calls.lock().len(), 0);
    }

    #[tokio::test]
    async fn ordner_existiert_nicht_mehr_nichts_tun() {
        let (calls, opts) = setup(true, false, &[]);
        assert!(restore_label("C:\\Arbeit\\Weg", &opts).await.unwrap().skipped);
        assert_eq!(calls.lock().len(), 0);
    }

    #[tokio::test]
    async fn laufwerkswurzel_und_benutzerordner_werden_nie_zurueckgesetzt() {
        let (calls, opts) = setup(true, true, &[]);
        assert!(restore_label("C:\\", &opts).await.unwrap().skipped);
        assert!(restore_label("c:\\users\\ANNA\\", &opts).await.unwrap().skipped);
        assert_eq!(calls.lock().len(), 0);
    }

    #[tokio::test]
    async fn gleicher_enthaltener_und_enthaltender_ordner_eines_anderen_projekts_ueberspringen() {
        for f in ["c:\\arbeit\\a\\b", "C:\\Arbeit\\A\\B\\sub", "C:/Arbeit/A", "C:\\Arbeit\\A\\B\\"] {
            let (calls, opts) = setup(true, true, &["C:\\Arbeit\\A\\B"]);
            let r = restore_label(f, &opts).await.unwrap();
            assert!(r.skipped, "{f}");
            assert_eq!(calls.lock().len(), 0, "{f}");
        }
    }

    #[tokio::test]
    async fn ordner_mit_gleichem_namensanfang_zaehlt_nicht_als_darin() {
        let (calls, opts) = setup(true, true, &["C:\\a\\b"]);
        assert!(!restore_label("C:\\a\\bc", &opts).await.unwrap().skipped);
        assert!(!restore_label("C:\\a\\b2\\x", &opts).await.unwrap().skipped);
        assert_eq!(calls.lock().len(), 2);
    }

    #[tokio::test]
    async fn fehler_des_prozesses_gehen_an_die_aufrufende_stelle() {
        let (_calls, mut opts) = setup(true, true, &[]);
        opts.run = Arc::new(|_, _, _| Box::pin(async { Err(anyhow!("icacls kaputt")) }));
        let err = restore_label("C:\\Arbeit\\Projekt", &opts).await.unwrap_err();
        assert!(err.to_string().contains("icacls kaputt"));
    }
}
