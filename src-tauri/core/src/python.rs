//! Mitgeliefertes Python in der installierten App und der einzelnen EXE (ENTSCHEIDE E39).
//! Es liegt als ein Archiv in resources\python-pack (python.zip und python.json mit { id, files, entries }).
//! Vor dem ersten Terminal-Start einer neuen Fassung entpackt die App es mit tar.exe von Windows nach
//! %LOCALAPPDATA%\ASK-Desktop\python\<id>, zuerst nach <id>.neu und erst nach vollständigem Entpacken
//! umbenannt (wie der Starter der einzelnen EXE, E18). Ältere Fassungen räumt sie danach weg.
//! Im Entwicklungsmodus (npm start) gibt es kein Archiv; dort gilt Build-Ordner\python direkt.
//!
//! Öffentliche Schnittstelle:
//! - `shared() -> &'static PythonRuntime` (aus `paths::resources(["python-pack"])` und `runtime_root`)
//! - `PythonRuntime::new(PythonOptions)`, `.dir()`, `.exe()`, `.ready()`,
//!   `async .ensure(on_progress: impl Fn(u32)) -> Result<PathBuf>`, `async .ensure_quiet()`, `.cleanup()`
//! - `read_manifest(pack_dir) -> Option<Manifest>`, `runtime_root(local_app_data) -> PathBuf`, `DONE`
//! - `TarSpawner` (ersetzbar für Tests), `SystemTar`

use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, OnceLock};
use tokio::io::AsyncReadExt;

pub const DONE: &str = ".fertig";

/// Inhalt von python.json
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    pub id: String,
    pub files: f64,
    pub entries: f64,
}

static MANIFEST_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9a-f]{8,64}$").expect("gültiger Ausdruck"));

pub fn read_manifest(pack_dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(pack_dir.join("python.json")).ok()?;
    let m: Value = serde_json::from_str(&text).ok()?;
    let id = m.get("id")?.as_str()?;
    if !MANIFEST_ID.is_match(id) {
        return None;
    }
    Some(Manifest { id: id.to_string(), files: crate::util::num_of(&m, "files"), entries: crate::util::num_of(&m, "entries") })
}

/// %LOCALAPPDATA%\ASK-Desktop\python; leer, wenn LOCALAPPDATA fehlt
pub fn runtime_root(local_app_data: Option<&str>) -> PathBuf {
    match local_app_data {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir).join("ASK-Desktop").join("python"),
        _ => PathBuf::new(),
    }
}

/// Startet tar.exe (in Tests ein Ersatz). `on_data` bekommt jedes Stück Ausgabe (stdout und stderr).
/// `Err(Text)`: das Programm liess sich nicht starten; `Ok(code)`: es ist mit diesem Code beendet.
#[async_trait]
pub trait TarSpawner: Send + Sync {
    async fn run(&self, exe: &Path, args: &[String], on_data: &mut (dyn for<'s> FnMut(&'s str) + Send)) -> std::result::Result<Option<i32>, String>;
}

/// Liest einen Strom und schickt die Stücke als Text weiter
fn pump(mut stream: impl tokio::io::AsyncRead + Unpin + Send + 'static, tx: tokio::sync::mpsc::UnboundedSender<String>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        while let Ok(n) = stream.read(&mut buf).await {
            if n == 0 || tx.send(String::from_utf8_lossy(&buf[..n]).into_owned()).is_err() {
                break;
            }
        }
    })
}

/// Das echte tar.exe
pub struct SystemTar;

#[async_trait]
impl TarSpawner for SystemTar {
    async fn run(&self, exe: &Path, args: &[String], on_data: &mut (dyn for<'s> FnMut(&'s str) + Send)) -> std::result::Result<Option<i32>, String> {
        let mut cmd = tokio::process::Command::new(exe);
        cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        // stdout und stderr zusammen: je ein Leser, die Stücke laufen über einen Kanal
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            readers.push(pump(out, tx.clone()));
        }
        if let Some(err) = child.stderr.take() {
            readers.push(pump(err, tx.clone()));
        }
        drop(tx);
        while let Some(chunk) = rx.recv().await {
            on_data(&chunk);
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        Ok(status.code())
    }
}

/// Angaben für `PythonRuntime::new`
pub struct PythonOptions {
    /// resources\python-pack
    pub pack_dir: PathBuf,
    /// %LOCALAPPDATA%\ASK-Desktop\python (leer: nicht eingerichtet)
    pub root: PathBuf,
    /// tar.exe; ohne Angabe das von Windows (System32)
    pub tar_exe: Option<PathBuf>,
    pub spawner: Arc<dyn TarSpawner>,
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

impl PythonOptions {
    pub fn new(pack_dir: PathBuf, root: PathBuf) -> Self {
        PythonOptions { pack_dir, root, tar_exe: None, spawner: Arc::new(SystemTar), log: Arc::new(|_| {}) }
    }
}

pub struct PythonRuntime {
    pack_dir: PathBuf,
    root: PathBuf,
    tar_exe: PathBuf,
    spawner: Arc<dyn TarSpawner>,
    log: Arc<dyn Fn(&str) + Send + Sync>,
    manifest: Option<Manifest>,
    /// Zwei Starts gleichzeitig entpacken nur einmal: Der zweite wartet und findet Python fertig vor.
    pending: tokio::sync::Mutex<()>,
}

async fn remove_dir(dir: PathBuf) {
    let _ = tokio::task::spawn_blocking(move || std::fs::remove_dir_all(dir)).await;
}

impl PythonRuntime {
    pub fn new(options: PythonOptions) -> Self {
        let tar_exe = options.tar_exe.unwrap_or_else(|| {
            let system_root = std::env::var("SystemRoot").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "C:\\Windows".to_string());
            PathBuf::from(system_root).join("System32").join("tar.exe")
        });
        PythonRuntime {
            manifest: read_manifest(&options.pack_dir),
            pack_dir: options.pack_dir,
            root: options.root,
            tar_exe,
            spawner: options.spawner,
            log: options.log,
            pending: tokio::sync::Mutex::new(()),
        }
    }

    /// Ordner dieser Fassung; leer ohne Archiv oder ohne Laufzeitordner
    pub fn dir(&self) -> PathBuf {
        match &self.manifest {
            Some(m) if !self.root.as_os_str().is_empty() => self.root.join(&m.id),
            _ => PathBuf::new(),
        }
    }

    /// python.exe; leer, wenn es kein Archiv gibt
    pub fn exe(&self) -> PathBuf {
        let dir = self.dir();
        if dir.as_os_str().is_empty() { PathBuf::new() } else { dir.join("python.exe") }
    }

    pub fn ready(&self) -> bool {
        let dir = self.dir();
        !dir.as_os_str().is_empty() && dir.join(DONE).exists() && self.exe().exists()
    }

    /// Sorgt dafür, dass Python entpackt ist. `on_progress(prozent)` während des Entpackens.
    pub async fn ensure(&self, on_progress: impl Fn(u32) + Send + Sync) -> Result<PathBuf> {
        if self.manifest.is_none() {
            bail!("Das mitgelieferte Python fehlt in der App (python-pack). Bitte ASK neu installieren.");
        }
        if self.root.as_os_str().is_empty() {
            bail!("LOCALAPPDATA ist nicht gesetzt; Python kann nicht eingerichtet werden.");
        }
        if self.ready() {
            self.cleanup();
            return Ok(self.dir());
        }
        let _guard = self.pending.lock().await;
        if self.ready() {
            return Ok(self.dir()); // ein anderer Aufruf war schneller
        }
        self.extract(&on_progress).await
    }

    /// Wie `ensure`, ohne Fortschritt
    pub async fn ensure_quiet(&self) -> Result<PathBuf> {
        self.ensure(|_| {}).await
    }

    async fn extract(&self, on_progress: &(impl Fn(u32) + Send + Sync)) -> Result<PathBuf> {
        let manifest = self.manifest.as_ref().ok_or_else(|| anyhow!("Python-Archiv fehlt"))?;
        let dir = self.dir();
        let mut tmp_name = dir.as_os_str().to_os_string();
        tmp_name.push(".neu");
        let tmp = PathBuf::from(tmp_name);
        remove_dir(tmp.clone()).await;
        std::fs::create_dir_all(&tmp)?;
        let total = {
            let n = if manifest.entries != 0.0 { manifest.entries } else { manifest.files };
            if n.is_nan() || n < 1.0 { 1.0 } else { n }
        };
        (self.log)(&format!("Entpacke Python {} nach {}", manifest.id, dir.display()));
        on_progress(0);
        let args: Vec<String> = vec![
            "-x".into(),
            "-v".into(),
            "-f".into(),
            self.pack_dir.join("python.zip").to_string_lossy().to_string(),
            "-C".into(),
            tmp.to_string_lossy().to_string(),
        ];
        let mut count = 0usize;
        let mut last: i64 = -1;
        let mut tail = String::new();
        let result = {
            let mut on_data = |text: &str| {
                tail.push_str(text);
                let chars: Vec<char> = tail.chars().collect();
                if chars.len() > 400 {
                    tail = chars[chars.len() - 400..].iter().collect();
                }
                count += text.matches('\n').count();
                let percent = (((count as f64 / total) * 100.0).floor() as i64).min(99);
                if percent != last {
                    last = percent;
                    on_progress(percent as u32);
                }
            };
            self.spawner.run(&self.tar_exe, &args, &mut on_data).await
        };
        let failure: Option<String> = match result {
            Err(msg) => Some(format!("Python konnte nicht entpackt werden: {msg}")),
            Ok(Some(0)) => None,
            Ok(code) => {
                let code = code.map_or_else(|| "null".to_string(), |c| c.to_string());
                let last_line = tail.trim().split('\n').next_back().unwrap_or("").to_string();
                Some(format!("Python konnte nicht entpackt werden (tar, Code {code}). {last_line}").trim().to_string())
            }
        };
        if let Some(message) = failure {
            remove_dir(tmp).await;
            bail!("{message}");
        }
        if !tmp.join("python.exe").exists() {
            remove_dir(tmp).await;
            bail!("Python konnte nicht entpackt werden: python.exe fehlt im Archiv.");
        }
        std::fs::write(tmp.join(DONE), format!("{}\n", chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ")))?;
        remove_dir(dir.clone()).await; // halb entpackter Stand ohne .fertig
        std::fs::rename(&tmp, &dir)?;
        on_progress(100);
        self.cleanup();
        Ok(dir)
    }

    /// Ältere Fassungen und abgebrochene Entpackungen entfernen. Nur mit der Einzelinstanz-Sperre aufrufen.
    pub fn cleanup(&self) {
        let Some(manifest) = &self.manifest else { return };
        let Ok(entries) = std::fs::read_dir(&self.root) else { return };
        let own = manifest.id.to_lowercase();
        for entry in entries.flatten() {
            let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
            if !is_dir || entry.file_name().to_string_lossy().to_lowercase() == own {
                continue;
            }
            let path = entry.path();
            std::thread::spawn(move || {
                let _ = std::fs::remove_dir_all(path);
            });
        }
    }
}

/// Das Python dieser App (für `paths::python_exe` und das Terminal)
pub fn shared() -> &'static PythonRuntime {
    static R: OnceLock<PythonRuntime> = OnceLock::new();
    R.get_or_init(|| {
        let local = std::env::var("LOCALAPPDATA").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| crate::paths::local_app_data().to_string_lossy().to_string());
        PythonRuntime::new(PythonOptions::new(crate::paths::resources(&["python-pack"]), runtime_root(Some(&local))))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parking_lot::Mutex;

    /// Nachgebautes tar.exe: legt die Dateien im Zielordner an und meldet jede Zeile wie bsdtar -v
    struct FakeTar {
        mode: &'static str,
        calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    }

    #[async_trait]
    impl TarSpawner for FakeTar {
        async fn run(&self, exe: &Path, args: &[String], on_data: &mut (dyn for<'s> FnMut(&'s str) + Send)) -> std::result::Result<Option<i32>, String> {
            self.calls.lock().push((exe.to_path_buf(), args.to_vec()));
            let dest = PathBuf::from(&args[args.iter().position(|a| a == "-C").unwrap() + 1]);
            tokio::task::yield_now().await;
            if self.mode == "fail" {
                on_data("tar.exe: Error opening archive: Disk full\n");
                return Ok(Some(1));
            }
            std::fs::create_dir_all(dest.join("Lib")).unwrap();
            if self.mode != "no-exe" {
                std::fs::write(dest.join("python.exe"), "exe").unwrap();
            }
            std::fs::write(dest.join("Lib").join("os.py"), "").unwrap();
            on_data("x ./\nx ./Lib/\n");
            on_data("x ./Lib/os.py\nx ./python.exe\n");
            Ok(Some(0))
        }
    }

    struct Setup {
        _base: tempfile::TempDir,
        root: PathBuf,
        rt: PythonRuntime,
        tar: Arc<FakeTar>,
    }

    fn setup(mode: &'static str) -> Setup {
        let base = tempfile::tempdir().unwrap();
        let pack_dir = base.path().join("python-pack");
        let root = base.path().join("python");
        std::fs::create_dir_all(&pack_dir).unwrap();
        std::fs::write(pack_dir.join("python.json"), r#"{"id":"abcdef0123456789","files":3,"entries":4}"#).unwrap();
        std::fs::write(pack_dir.join("python.zip"), "").unwrap();
        let tar = Arc::new(FakeTar { mode, calls: Mutex::new(Vec::new()) });
        let mut options = PythonOptions::new(pack_dir, root.clone());
        options.tar_exe = Some(PathBuf::from("tar.exe"));
        options.spawner = tar.clone();
        Setup { _base: base, root, rt: PythonRuntime::new(options), tar }
    }

    #[test]
    fn ordner_unter_localappdata_ask_desktop_python() {
        let base = "C:\\Users\\lea\\AppData\\Local";
        assert_eq!(runtime_root(Some(base)), PathBuf::from(base).join("ASK-Desktop").join("python"));
        assert_eq!(runtime_root(None), PathBuf::new());
    }

    #[tokio::test]
    async fn entpackt_beim_ersten_mal_ueber_neu_mit_fortschritt() {
        let s = setup("ok");
        let progress = Arc::new(Mutex::new(Vec::new()));
        let p = progress.clone();
        let dir = s.rt.ensure(move |x| p.lock().push(x)).await.unwrap();
        assert_eq!(dir, s.root.join("abcdef0123456789"));
        assert!(dir.join("python.exe").exists());
        assert!(dir.join(DONE).exists());
        assert!(!PathBuf::from(format!("{}.neu", dir.display())).exists());
        assert_eq!(s.rt.exe(), dir.join("python.exe"));
        let calls = s.tar.calls.lock();
        assert_eq!(calls[0].1[..3], ["-x", "-v", "-f"]);
        assert!(calls[0].1.contains(&format!("{}.neu", dir.display())));
        let progress = progress.lock();
        assert_eq!(progress[0], 0);
        assert_eq!(*progress.last().unwrap(), 100);
        assert!(progress.contains(&50)); // 2 von 4 Einträgen
    }

    #[tokio::test]
    async fn schon_entpackt_kein_tar_aeltere_fassungen_werden_entfernt() {
        let s = setup("ok");
        s.rt.ensure_quiet().await.unwrap();
        std::fs::create_dir_all(s.root.join("1111111111111111").join("Lib")).unwrap();
        std::fs::create_dir_all(s.root.join("abcdef0123456789.neu")).unwrap();
        s.rt.ensure_quiet().await.unwrap();
        assert_eq!(s.tar.calls.lock().len(), 1);
        for _ in 0..100 {
            if std::fs::read_dir(&s.root).unwrap().count() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        let names: Vec<String> = std::fs::read_dir(&s.root).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert_eq!(names, ["abcdef0123456789"]);
    }

    #[tokio::test]
    async fn zwei_starts_gleichzeitig_entpacken_nur_einmal() {
        let s = setup("ok");
        let (a, b) = tokio::join!(s.rt.ensure_quiet(), s.rt.ensure_quiet());
        assert_eq!(a.unwrap(), b.unwrap());
        assert_eq!(s.tar.calls.lock().len(), 1);
    }

    #[tokio::test]
    async fn halb_entpackter_ordner_ohne_fertig_wird_ersetzt() {
        let s = setup("ok");
        let dir = s.root.join("abcdef0123456789");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("python.exe"), "alt").unwrap();
        assert!(!s.rt.ready());
        s.rt.ensure_quiet().await.unwrap();
        assert_eq!(s.tar.calls.lock().len(), 1);
        assert_eq!(std::fs::read_to_string(dir.join("python.exe")).unwrap(), "exe");
    }

    #[tokio::test]
    async fn fehler_tar_scheitert_oder_python_exe_fehlt_nichts_bleibt_liegen() {
        for mode in ["fail", "no-exe"] {
            let s = setup(mode);
            let err = s.rt.ensure_quiet().await.unwrap_err();
            assert!(err.to_string().contains("Python konnte nicht entpackt werden"), "{mode}: {err}");
            let left = if s.root.exists() { std::fs::read_dir(&s.root).unwrap().count() } else { 0 };
            assert_eq!(left, 0, "{mode}");
            assert!(!s.rt.ready());
        }
    }

    #[tokio::test]
    async fn fehlertext_von_tar_enthaelt_code_und_letzte_zeile() {
        let s = setup("fail");
        let err = s.rt.ensure_quiet().await.unwrap_err();
        assert_eq!(err.to_string(), "Python konnte nicht entpackt werden (tar, Code 1). tar.exe: Error opening archive: Disk full");
    }

    #[tokio::test]
    async fn ohne_archiv_in_der_app_verstaendlicher_fehler() {
        let base = tempfile::tempdir().unwrap();
        let rt = PythonRuntime::new(PythonOptions::new(base.path().join("fehlt"), base.path().join("python")));
        assert_eq!(rt.exe(), PathBuf::new());
        let err = rt.ensure_quiet().await.unwrap_err();
        assert!(err.to_string().contains("neu installieren"));
    }
}
