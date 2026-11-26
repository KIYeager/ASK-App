//! Prüft, ob ein Ordner als Arbeitsordner taugt.
//! Gesperrt sind OneDrive und SharePoint (Löschungen würden sofort synchronisiert), Laufwerkswurzeln,
//! das Benutzerprofil selbst, AppData, Windows und die Programmordner.
//!
//! Öffentliche Schnittstelle:
//! - `check_folder(input, &FolderContext) -> FolderCheck`, `FolderContext::system()` für den echten Rechner
//! - `read_sync_roots() -> Vec<String>`, `is_inside(child, parent) -> bool`
//! - `reason::*` (die Meldungen), `win_resolve`, `norm` (Pfade nach Art von `path.win32`, auch unter Linux)

use parking_lot::Mutex;
use regex::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

/// Die Meldungen an die Person (wie `REASON` im Original)
pub mod reason {
    pub const MISSING: &str = "Der Ordner existiert nicht.";
    pub const NOT_DIR: &str = "Das ist kein Ordner.";
    pub const NETWORK: &str = "Netzlaufwerke und Netzwerkpfade werden nicht unterstützt. Bitte einen Ordner auf diesem Laptop wählen.";
    pub const ROOT: &str = "Ein ganzes Laufwerk kann nicht gewählt werden. Bitte einen eigenen Projektordner anlegen.";
    pub const ONEDRIVE: &str = "Ordner in OneDrive oder SharePoint sind gesperrt: Änderungen und Löschungen würden sofort in die Cloud synchronisiert. Bitte einen Ordner ausserhalb von OneDrive wählen, zum Beispiel unter C:\\Users\\<Name>\\ASK-Projekte.";
    pub const CONTAINS_SYNC: &str = "Der Ordner enthält einen OneDrive- oder SharePoint-Ordner. Bitte einen Unterordner ohne Cloud-Synchronisation wählen.";
    pub const PROFILE: &str = "Das ganze Benutzerprofil kann nicht gewählt werden. Bitte darin einen eigenen Projektordner anlegen.";
    pub const USERS_ROOT: &str = "Der Ordner mit allen Benutzerprofilen kann nicht gewählt werden.";
    pub const APPDATA: &str = "Ordner in AppData sind gesperrt, dort liegen Einstellungen anderer Programme.";
    pub const SYSTEM: &str = "Systemordner (Windows, Programme, ProgramData) sind gesperrt.";
}

// ---------------------------------------------------------------------------------------------
// Windows-Pfade als Text (`path.win32`): Die Prüfung gilt Windows-Pfaden, auch wenn die Tests unter
// Linux laufen. Darum wird hier nicht mit `std::path` gearbeitet.

/// `path.win32.resolve(p)` für einen einzelnen Pfad: Trenner vereinheitlicht, `.` und `..` aufgelöst,
/// ohne abschliessenden Trenner (ausser bei der Wurzel, z. B. `C:\`). Relative Pfade gelten ab dem
/// aktuellen Ordner.
pub fn win_resolve(p: &str) -> String {
    let s = p.replace('/', "\\");
    let (root, rest): (String, String) = if let Some(unc) = s.strip_prefix("\\\\") {
        let mut parts = unc.splitn(3, '\\');
        let server = parts.next().unwrap_or("");
        let share = parts.next().unwrap_or("");
        let tail = parts.next().unwrap_or("");
        (format!("\\\\{server}\\{share}\\"), tail.to_string())
    } else if s.len() >= 2 && s.as_bytes()[0].is_ascii_alphabetic() && s.as_bytes()[1] == b':' {
        let drive = &s[..2];
        (format!("{drive}\\"), s[2..].trim_start_matches('\\').to_string())
    } else if let Some(tail) = s.strip_prefix('\\') {
        (format!("{}\\", cwd_drive()), tail.to_string())
    } else {
        // relativ: ab dem aktuellen Ordner
        let cwd = std::env::current_dir().map(|d| d.to_string_lossy().replace('/', "\\")).unwrap_or_default();
        let joined = format!("{cwd}\\{s}");
        return win_resolve_absolute(&joined);
    };
    normalize_segments(&root, &rest)
}

fn win_resolve_absolute(s: &str) -> String {
    if s.starts_with('\\') && !s.starts_with("\\\\") {
        // Linux-Ordner (/home/...): ohne Laufwerk, Laufwerk C: annehmen
        return normalize_segments(&format!("{}\\", cwd_drive()), s.trim_start_matches('\\'));
    }
    win_resolve(s)
}

fn cwd_drive() -> String {
    let cwd = std::env::current_dir().map(|d| d.to_string_lossy().to_string()).unwrap_or_default();
    let b = cwd.as_bytes();
    if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        cwd[..2].to_string()
    } else {
        "C:".to_string()
    }
}

fn normalize_segments(root: &str, rest: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in rest.split('\\') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    format!("{root}{}", out.join("\\"))
}

static TRAILING_SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\\/]+$").expect("gültiger Ausdruck"));

/// Schlüssel eines Ordners: aufgelöst, ohne abschliessenden Trenner, klein geschrieben
pub fn norm(p: &str) -> String {
    TRAILING_SEP.replace(&win_resolve(p), "").to_lowercase()
}

/// `child` liegt in `parent` (oder ist `parent`)
pub fn is_inside(child: &str, parent: &str) -> bool {
    let c = norm(child);
    let p = norm(parent);
    c == p || c.starts_with(&format!("{p}\\"))
}

// ---------------------------------------------------------------------------------------------
// Registry (OneDrive und SharePoint)

/// Führt ein Programm ohne Fenster aus und liefert die Ausgabe, wenn es mit Code 0 endet (wie
/// `execFileSync` mit Zeitlimit; bei Fehler oder Zeitüberschreitung `None`).
fn run_hidden(program: &str, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if start.elapsed() > timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(15)),
            Err(_) => break None,
        }
    };
    let bytes = reader.join().unwrap_or_default();
    match status {
        Some(s) if s.success() => Some(String::from_utf8_lossy(&bytes).to_string()),
        _ => None,
    }
}

static REG_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s+\S+\s+REG_(?:EXPAND_)?SZ\s+(.+)$").expect("gültiger Ausdruck"));
static DRIVE_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z]:\\").expect("gültiger Ausdruck"));
static TENANT_VALUE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s+([a-z]:\\.+?)\s+REG_\w+").expect("gültiger Ausdruck"));
static TENANTS_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\\Tenants\\").expect("gültiger Ausdruck"));

fn lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l))
}

/// Synchronisierte OneDrive- und SharePoint-Ordner aus der Registry (auch Bibliotheken,
/// deren Pfad nicht «OneDrive» enthält). Unter Linux gibt es keine: leere Liste.
pub fn read_sync_roots() -> Vec<String> {
    let mut roots: Vec<String> = Vec::new();
    let queries: [[&str; 5]; 2] = [
        ["query", "HKCU\\Software\\SyncEngines\\Providers\\OneDrive", "/s", "/v", "MountPoint"],
        ["query", "HKCU\\Software\\Microsoft\\OneDrive\\Accounts", "/s", "/v", "UserFolder"],
    ];
    for args in queries {
        // Schlüssel fehlt: kein OneDrive eingerichtet.
        if let Some(out) = run_hidden("reg.exe", &args, Duration::from_secs(5)) {
            for line in lines(&out) {
                if let Some(m) = REG_VALUE.captures(line) {
                    let value = m[1].trim();
                    if DRIVE_PATH.is_match(value) {
                        roots.push(value.to_string());
                    }
                }
            }
        }
    }
    if let Some(out) = run_hidden("reg.exe", &["query", "HKCU\\Software\\Microsoft\\OneDrive\\Accounts", "/s"], Duration::from_secs(5)) {
        // Unter ...\Tenants\<Name> sind die Werte selbst Ordnerpfade synchronisierter Bibliotheken.
        let mut in_tenants = false;
        for line in lines(&out) {
            if line.starts_with("HKEY_") {
                in_tenants = TENANTS_KEY.is_match(line);
            }
            if in_tenants {
                if let Some(m) = TENANT_VALUE.captures(line) {
                    roots.push(m[1].trim().to_string());
                }
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    roots.retain(|r| seen.insert(r.clone()));
    roots
}

// Die Abfrage der Registry startet dreimal reg.exe und hält die Oberfläche kurz an: eine Minute merken
static SYNC_CACHE: Mutex<(i64, Vec<String>)> = Mutex::new((0, Vec::new()));

fn cached_sync_roots() -> Vec<String> {
    let mut cache = SYNC_CACHE.lock();
    if crate::util::now_ms() - cache.0 > 60000 {
        *cache = (crate::util::now_ms(), read_sync_roots());
    }
    cache.1.clone()
}

// ---------------------------------------------------------------------------------------------

type RealpathFn = Box<dyn Fn(&str) -> std::io::Result<String> + Send + Sync>;
type IsDirFn = Box<dyn Fn(&str) -> std::io::Result<bool> + Send + Sync>;

/// Umgebung der Prüfung (Umgebungsvariablen, Registry, Dateisystem); Tests ersetzen sie.
pub struct FolderContext {
    pub env: HashMap<String, String>,
    pub sync_roots: Vec<String>,
    pub realpath: RealpathFn,
    /// Ist der Pfad ein Ordner? Fehler, wenn er nicht gelesen werden kann.
    pub is_dir: IsDirFn,
}

impl FolderContext {
    /// Der echte Rechner
    pub fn system() -> Self {
        FolderContext {
            env: std::env::vars().collect(),
            sync_roots: cached_sync_roots(),
            realpath: Box::new(|p| dunce::canonicalize(p).map(|r| r.to_string_lossy().to_string())),
            is_dir: Box::new(|p| std::fs::metadata(p).map(|m| m.is_dir())),
        }
    }

    /// Umgebungsvariable; Windows unterscheidet Gross- und Kleinschreibung nicht
    fn var(&self, key: &str) -> Option<&str> {
        if let Some(v) = self.env.get(key) {
            return Some(v.as_str()).filter(|s| !s.is_empty());
        }
        self.env.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str()).filter(|s| !s.is_empty())
    }
}

/// Ergebnis von `check_folder`: `ok` mit dem aufgelösten Pfad, sonst `reason`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderCheck {
    pub ok: bool,
    pub reason: Option<String>,
    pub path: Option<String>,
}

impl FolderCheck {
    fn no(reason: &str) -> Self {
        FolderCheck { ok: false, reason: Some(reason.to_string()), path: None }
    }

    /// Wie das Objekt `{ ok, reason }` bzw. `{ ok, path }` des Originals
    pub fn to_json(&self) -> serde_json::Value {
        if self.ok {
            serde_json::json!({ "ok": true, "path": self.path })
        } else {
            serde_json::json!({ "ok": false, "reason": self.reason })
        }
    }
}

static DRIVE_ROOT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[a-z]:\\?$").expect("gültiger Ausdruck"));
static ONEDRIVE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)onedrive").expect("gültiger Ausdruck"));

pub fn check_folder(input: &str, ctx: &FolderContext) -> FolderCheck {
    if input.is_empty() {
        return FolderCheck::no(reason::MISSING);
    }
    if input.starts_with("\\\\") {
        return FolderCheck::no(reason::NETWORK);
    }
    match (ctx.is_dir)(input) {
        Ok(true) => {}
        Ok(false) => return FolderCheck::no(reason::NOT_DIR),
        Err(_) => return FolderCheck::no(reason::MISSING),
    }
    let Ok(real) = (ctx.realpath)(input) else { return FolderCheck::no(reason::MISSING) };
    if real.starts_with("\\\\") {
        return FolderCheck::no(reason::NETWORK);
    }
    if DRIVE_ROOT.is_match(&real) {
        return FolderCheck::no(reason::ROOT);
    }

    let profile = ctx.var("USERPROFILE");
    if let Some(profile) = profile {
        if norm(&real) == norm(profile) {
            return FolderCheck::no(reason::PROFILE);
        }
        if is_inside(profile, &real) {
            return FolderCheck::no(reason::USERS_ROOT);
        }
    }

    let mut sync_roots: Vec<String> = ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"]
        .iter()
        .filter_map(|k| ctx.var(k).map(str::to_string))
        .collect();
    sync_roots.extend(ctx.sync_roots.iter().cloned());
    if sync_roots.iter().any(|r| is_inside(&real, r)) {
        return FolderCheck::no(reason::ONEDRIVE);
    }
    if real.split('\\').any(|part| ONEDRIVE_NAME.is_match(part)) {
        return FolderCheck::no(reason::ONEDRIVE);
    }
    if sync_roots.iter().any(|r| is_inside(r, &real)) {
        return FolderCheck::no(reason::CONTAINS_SYNC);
    }

    if let Some(profile) = profile {
        if is_inside(&real, &format!("{}\\AppData", profile.trim_end_matches(['\\', '/']))) {
            return FolderCheck::no(reason::APPDATA);
        }
    }
    for key in ["APPDATA", "LOCALAPPDATA"] {
        if let Some(dir) = ctx.var(key) {
            if is_inside(&real, dir) {
                return FolderCheck::no(reason::APPDATA);
            }
        }
    }

    for key in ["SystemRoot", "windir", "ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData"] {
        if let Some(dir) = ctx.var(key) {
            if is_inside(&real, dir) || is_inside(dir, &real) {
                return FolderCheck::no(reason::SYSTEM);
            }
        }
    }
    FolderCheck { ok: true, reason: None, path: Some(real) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> HashMap<String, String> {
        [
            ("USERPROFILE", "C:\\Users\\lea"),
            ("APPDATA", "C:\\Users\\lea\\AppData\\Roaming"),
            ("LOCALAPPDATA", "C:\\Users\\lea\\AppData\\Local"),
            ("OneDrive", "C:\\Users\\lea\\OneDrive - ICT TFBERN"),
            ("OneDriveCommercial", "C:\\Users\\lea\\OneDrive - ICT TFBERN"),
            ("SystemRoot", "C:\\WINDOWS"),
            ("ProgramFiles", "C:\\Program Files"),
            ("ProgramFiles(x86)", "C:\\Program Files (x86)"),
            ("ProgramData", "C:\\ProgramData"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn ctx() -> FolderContext {
        FolderContext {
            env: env(),
            sync_roots: vec!["C:\\Users\\lea\\ICT TFBERN\\Klasse 1A - Dokumente".to_string()],
            realpath: Box::new(|p| Ok(p.to_string())),
            is_dir: Box::new(|_| Ok(true)),
        }
    }

    fn reason_of(p: &str) -> Option<String> {
        check_folder(p, &ctx()).reason
    }

    #[test]
    fn normaler_projektordner_ist_erlaubt() {
        let r = check_folder("C:\\Users\\lea\\ASK-Projekte\\Bericht", &ctx());
        assert_eq!(r, FolderCheck { ok: true, reason: None, path: Some("C:\\Users\\lea\\ASK-Projekte\\Bericht".to_string()) });
        assert!(check_folder("D:\\Schule\\Projekt", &ctx()).ok);
    }

    #[test]
    fn onedrive_ueber_umgebungsvariable_namen_und_registry() {
        let want = Some(reason::ONEDRIVE.to_string());
        assert_eq!(reason_of("C:\\Users\\lea\\OneDrive - ICT TFBERN\\Dokumente"), want);
        assert_eq!(reason_of("c:\\users\\lea\\onedrive - ict tfbern"), want);
        assert_eq!(reason_of("D:\\Backup\\OneDrive-alt\\x"), want);
        assert_eq!(reason_of("C:\\Users\\lea\\ICT TFBERN\\Klasse 1A - Dokumente\\Abgaben"), want);
    }

    #[test]
    fn junction_in_onedrive_wird_aufgeloest() {
        let mut c = ctx();
        c.realpath = Box::new(|p| {
            Ok(if p == "C:\\Users\\lea\\Projekt" { "C:\\Users\\lea\\OneDrive - ICT TFBERN\\Projekt".to_string() } else { p.to_string() })
        });
        assert_eq!(check_folder("C:\\Users\\lea\\Projekt", &c).reason.as_deref(), Some(reason::ONEDRIVE));
    }

    #[test]
    fn ordner_der_onedrive_enthaelt() {
        assert_eq!(reason_of("C:\\Users\\lea\\ICT TFBERN").as_deref(), Some(reason::CONTAINS_SYNC));
    }

    #[test]
    fn laufwerkswurzeln_profil_und_benutzerordner() {
        assert_eq!(reason_of("C:\\").as_deref(), Some(reason::ROOT));
        assert_eq!(reason_of("D:").as_deref(), Some(reason::ROOT));
        assert_eq!(reason_of("C:\\Users\\lea").as_deref(), Some(reason::PROFILE));
        assert_eq!(reason_of("C:\\Users\\lea\\").as_deref(), Some(reason::PROFILE));
        assert_eq!(reason_of("C:\\Users").as_deref(), Some(reason::USERS_ROOT));
    }

    #[test]
    fn appdata_windows_und_programme() {
        assert_eq!(reason_of("C:\\Users\\lea\\AppData\\Local\\Temp\\x").as_deref(), Some(reason::APPDATA));
        assert_eq!(reason_of("C:\\Windows\\System32").as_deref(), Some(reason::SYSTEM));
        assert_eq!(reason_of("C:\\Program Files\\ASK").as_deref(), Some(reason::SYSTEM));
        assert_eq!(reason_of("C:\\Program Files (x86)").as_deref(), Some(reason::SYSTEM));
        assert_eq!(reason_of("C:\\ProgramData\\x").as_deref(), Some(reason::SYSTEM));
    }

    #[test]
    fn netzwerkpfade_und_fehlende_ordner() {
        assert_eq!(reason_of("\\\\server\\freigabe\\x").as_deref(), Some(reason::NETWORK));
        let mut missing = ctx();
        missing.is_dir = Box::new(|_| Err(std::io::Error::new(std::io::ErrorKind::NotFound, "ENOENT")));
        assert_eq!(check_folder("C:\\gibtsnicht", &missing).reason.as_deref(), Some(reason::MISSING));
        let mut file = ctx();
        file.is_dir = Box::new(|_| Ok(false));
        assert_eq!(check_folder("C:\\Users\\lea\\a.txt", &file).reason.as_deref(), Some(reason::NOT_DIR));
    }

    #[test]
    fn windows_pfade_als_text() {
        assert_eq!(win_resolve("C:/Arbeit/A/../B/"), "C:\\Arbeit\\B");
        assert_eq!(win_resolve("C:\\"), "C:\\");
        assert_eq!(norm("C:\\"), "c:");
        assert_eq!(norm("c:\\USERS\\Anna\\"), "c:\\users\\anna");
        assert!(is_inside("C:\\a\\b\\c", "c:\\A\\B"));
        assert!(!is_inside("C:\\a\\bc", "C:\\a\\b"));
    }
}
