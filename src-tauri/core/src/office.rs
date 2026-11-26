//! Umwandlung von Word-, Excel- und PowerPoint-Dateien in PDF mit dem installierten Office.
//!
//! Das Modell läuft mit Low Integrity und kann Office nicht selbst steuern. Es legt deshalb einen
//! Auftrag in .ask/auftraege/ ab; die App (normale Stufe) prüft die Datei und wandelt sie um.
//! Abgelehnt werden Dateien mit Makros, ActiveX, OLE-Objekten, externen Verweisen oder
//! verknüpften Feldern: zuerst hier in der App (ooxml.rs, liest das XML wirklich), danach nochmals
//! grob im PowerShell-Skript. Office öffnet die Datei schreibgeschützt mit deaktivierten Makros.
//! Office bekommt immer eine Kopie im Temp-Ordner: Hängt oder stürzt es ab, bleibt die Datei im
//! Arbeitsordner nicht gesperrt. Aus der Kopie wird ein Verweis auf eine externe Word-Vorlage
//! entfernt (sonst sucht Word sie im Netz, hängt oder schickt Anmeldedaten an fremde Rechner).
//!
//! PowerShell und die Registry gibt es nur unter Windows (`#[cfg(windows)]`); auf anderen Systemen
//! meldet `office_installed` false und die Umwandlung «Microsoft Office ist nicht installiert oder
//! nicht nutzbar.».
//!
//! Öffentliche Schnittstelle:
//! - `async convert_to_pdf(root, source, fresh) -> Conversion`
//! - `async office_installed(ext) -> bool`
//! - `pdf_target(root, source) -> PathBuf`

use crate::ooxml::check_ooxml;
use regex::Regex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[cfg(windows)]
const TIMEOUT_MS: u64 = 120000;
const EXTENSIONS: [&str; 3] = [".docx", ".xlsx", ".pptx"];

#[allow(dead_code)] // nur unter Windows gestartet
const SCRIPT: &str = r####"
$ErrorActionPreference = 'Stop'
$Quelle = $env:ASK_QUELLE
$Ziel = $env:ASK_ZIEL
function Melde($o) { [Console]::Out.WriteLine(($o | ConvertTo-Json -Compress)); [Console]::Out.Flush() }
function Lies($eintrag) { $r = New-Object IO.StreamReader($eintrag.Open()); try { $r.ReadToEnd() } finally { $r.Close() } }
try {
  Add-Type -AssemblyName System.IO.Compression.FileSystem
  $ext = [IO.Path]::GetExtension($Quelle).ToLower()
  if ($ext -eq '.docx') {
    # Verweis auf eine externe Vorlage (attachedTemplate) aus der Kopie entfernen
    $zip = [IO.Compression.ZipFile]::Open($Quelle, 'Update')
    try {
      foreach ($name in @('word/_rels/settings.xml.rels', 'word/settings.xml')) {
        $e = $zip.GetEntry($name)
        if (-not $e) { continue }
        $text = Lies $e
        $neu = $text -replace '<Relationship [^>]*attachedTemplate[^>]*/>', '' -replace '<w:attachedTemplate [^>]*/>', ''
        if ($neu -ne $text) {
          $e.Delete()
          $w = New-Object IO.StreamWriter(($zip.CreateEntry($name)).Open(), (New-Object Text.UTF8Encoding $false))
          try { $w.Write($neu) } finally { $w.Close() }
        }
      }
    } finally { $zip.Dispose() }
  }
  $zip = [IO.Compression.ZipFile]::OpenRead($Quelle)
  try {
    foreach ($e in $zip.Entries) {
      if ($e.FullName -match 'vbaProject|activeX|oleObject|embeddings/.*\.(bin|exe|dll|js|vbs|ps1|bat|cmd)$') { throw "Die Datei enthält $($e.FullName)." }
      if ($e.FullName -match '\.rels$') {
        foreach ($m in [regex]::Matches((Lies $e), '<Relationship [^>]*>')) {
          if ($m.Value -match 'TargetMode="External"' -and $m.Value -notmatch '/hyperlink"') { throw 'Die Datei verweist auf externe Inhalte.' }
        }
      }
      if ($e.FullName -match '^(word/(document|header\d*|footer\d*)|xl/externalLinks/.*)\.xml$') {
        $xml = Lies $e
        $feld = '(DDEAUTO|DDE|INCLUDETEXT|INCLUDEPICTURE|LINK)\b'
        if ($e.FullName -like 'xl/externalLinks/*' -or $xml -cmatch ('<w:instrText[^>]*>\s*' + $feld) -or $xml -cmatch ('w:instr="\s*' + $feld)) { throw 'Die Datei enthält verknüpfte Felder.' }
      }
    }
  } finally { $zip.Dispose() }

  $name = @{ '.docx' = 'WINWORD'; '.xlsx' = 'EXCEL'; '.pptx' = 'POWERPNT' }[$ext]
  $progId = @{ '.docx' = 'Word.Application'; '.xlsx' = 'Excel.Application'; '.pptx' = 'PowerPoint.Application' }[$ext]
  $vorher = @(Get-Process $name -ErrorAction SilentlyContinue | ForEach-Object Id)
  try { $app = New-Object -ComObject $progId } catch { throw 'Microsoft Office ist nicht installiert oder nicht nutzbar.' }
  $neu = @(Get-Process $name -ErrorAction SilentlyContinue | Where-Object { $vorher -notcontains $_.Id } | ForEach-Object Id)
  Melde @{ pids = $neu }
  $app.AutomationSecurity = 3
  # Office hängt Automatisierung teils an eine laufende Instanz an. Beendet wird sie nur, wenn sie
  # unsichtbar ist und nichts mehr offen hat; ein Office-Fenster der Nutzerin bleibt unangetastet.
  if ($ext -eq '.docx') {
    $sichtbar = $app.Visible
    $app.DisplayAlerts = 0
    $doc = $app.Documents.Open($Quelle, $false, $true, $false)
    # Inhaltsverzeichnisse füllt Word sonst erst beim Öffnen durch die Person
    try { foreach ($toc in @($doc.TablesOfContents)) { $toc.Update() } } catch { }
    $doc.ExportAsFixedFormat($Ziel, 17)
    $doc.Close(0)
    if (-not $sichtbar -and $app.Documents.Count -eq 0) { $app.Quit() }
  } elseif ($ext -eq '.xlsx') {
    $sichtbar = $app.Visible
    $app.DisplayAlerts = $false
    $wb = $app.Workbooks.Open($Quelle, 0, $true)
    $wb.ExportAsFixedFormat(0, $Ziel)
    $wb.Close($false)
    if (-not $sichtbar -and $app.Workbooks.Count -eq 0) { $app.Quit() }
  } else {
    $vorhanden = $app.Presentations.Count
    $pres = $app.Presentations.Open($Quelle, -1, 0, 0)
    $pres.SaveAs($Ziel, 32)
    $pres.Close()
    if ($neu.Count -gt 0 -and $vorhanden -eq 0 -and $app.Presentations.Count -eq 0) { $app.Quit() }
  }
  [void][Runtime.InteropServices.Marshal]::ReleaseComObject($app)
  if (-not (Test-Path $Ziel)) { throw 'Office hat kein PDF erzeugt.' }
  Melde @{ ok = $true }
} catch {
  Melde @{ fehler = $_.Exception.Message }
}
"####;

/// Das Skript als Base64 von UTF-16LE (`-EncodedCommand`)
#[allow(dead_code)] // nur unter Windows gestartet
fn encoded_script() -> String {
    use base64::Engine;
    let bytes: Vec<u8> = SCRIPT.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Ergebnis einer Umwandlung: `ok` mit `target` (und `cached`, wenn das PDF schon aktuell war),
/// sonst `fehler` mit dem Text für die Person.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Conversion {
    pub ok: bool,
    pub target: Option<PathBuf>,
    pub cached: bool,
    pub fehler: Option<String>,
}

impl Conversion {
    fn error(message: impl Into<String>) -> Self {
        Conversion { fehler: Some(message.into()), ..Default::default() }
    }

    /// Wie das Objekt `{ ok, target, cached }` bzw. `{ fehler }` des Originals
    pub fn to_json(&self) -> Value {
        if self.ok {
            let target = self.target.as_ref().map(|t| t.to_string_lossy().to_string());
            if self.cached { json!({ "ok": true, "target": target, "cached": true }) } else { json!({ "ok": true, "target": target }) }
        } else {
            json!({ "fehler": self.fehler })
        }
    }
}

/// Endung wie `path.extname`: ab dem letzten Punkt, aber nicht bei Namen wie «.gitignore»
fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_string(),
        _ => String::new(),
    }
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Ziel der Umwandlung: .ask/pdf/<name>.pdf
pub fn pdf_target(root: &Path, source: &Path) -> PathBuf {
    let name = file_name(source);
    let ext = ext_of(&name).to_lowercase();
    // wie path.basename(source, ext): schneidet nur ab, wenn die Endung genau so geschrieben ist
    let base = name.strip_suffix(ext.as_str()).filter(|_| !ext.is_empty()).unwrap_or(&name);
    let suffix = if ext == ".docx" { String::new() } else { ext.replacen('.', "-", 1) };
    root.join(".ask").join("pdf").join(format!("{base}{suffix}.pdf"))
}

#[cfg(windows)]
async fn run_conversion(source: &Path, target: &Path) -> Conversion {
    use tokio::io::{AsyncBufReadExt, BufReader};

    if let Some(dir) = target.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::remove_file(target);
    let system_root = std::env::var("SystemRoot").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "C:\\Windows".to_string());
    let ps = Path::new(&system_root).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe");
    let mut cmd = tokio::process::Command::new(ps);
    cmd.args(["-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded_script()])
        .env("ASK_QUELLE", source)
        .env("ASK_ZIEL", target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => return Conversion::error(err.to_string()),
    };
    let stdout = child.stdout.take();
    let mut pids: Vec<u64> = Vec::new();
    let mut result: Option<Conversion> = None;
    let work = async {
        if let Some(stdout) = stdout {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                // andere Ausgabe ignorieren
                let Ok(msg) = serde_json::from_str::<Value>(line.trim()) else { continue };
                if let Some(p) = msg.get("pids") {
                    let list: Vec<&Value> = match p {
                        Value::Array(a) => a.iter().collect(),
                        other => vec![other],
                    };
                    pids = list.into_iter().filter(|v| crate::util::truthy(v)).filter_map(Value::as_u64).collect();
                }
                if crate::util::truthy(msg.get("ok").unwrap_or(&Value::Null)) {
                    result = Some(Conversion { ok: true, ..Default::default() });
                } else if crate::util::truthy(msg.get("fehler").unwrap_or(&Value::Null)) {
                    result = Some(Conversion::error(crate::util::str_of(&msg, "fehler")));
                }
            }
        }
        let _ = child.wait().await;
    };
    if tokio::time::timeout(std::time::Duration::from_millis(TIMEOUT_MS), work).await.is_err() {
        let _ = child.start_kill();
        // Nur die Office-Prozesse beenden, die für diese Umwandlung gestartet wurden.
        for pid in &pids {
            let mut kill = std::process::Command::new("taskkill");
            kill.args(["/F", "/PID", &pid.to_string()]);
            {
                use std::os::windows::process::CommandExt;
                kill.creation_flags(0x0800_0000);
            }
            let _ = kill.output();
        }
        return Conversion::error("Office hat nicht innerhalb von 2 Minuten geantwortet.");
    }
    result.unwrap_or_else(|| Conversion::error("PowerShell konnte Office nicht starten (eventuell durch Richtlinien gesperrt)."))
}

#[cfg(not(windows))]
async fn run_conversion(_source: &Path, _target: &Path) -> Conversion {
    Conversion::error("Microsoft Office ist nicht installiert oder nicht nutzbar.")
}

// Office verliert manchmal die Verbindung (RPC_E_DISCONNECTED); dann einmal mit neuer Instanz versuchen.
static DISCONNECTED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)RPC_E_|RPC-Server|getrennt|disconnected|0x80010108|0x800706BA|0x800706BE").expect("gültiger Ausdruck"));

/// Neuer Ordner im Temp-Ordner (wie `mkdtemp`)
fn make_temp_dir() -> std::io::Result<PathBuf> {
    let tag: [u8; 3] = rand::random();
    let tag = hex::encode(tag);
    for _ in 0..20 {
        let suffix: String = (0..6).map(|_| char::from(b"abcdefghijklmnopqrstuvwxyz0123456789"[rand::random::<u8>() as usize % 36])).collect();
        let dir = std::env::temp_dir().join(format!("ask-pdf-{tag}-{suffix}"));
        match std::fs::create_dir(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::other("kein freier Name im Temp-Ordner"))
}

/// Umwandlung einer Kopie im Temp-Ordner; das Original im Arbeitsordner öffnet Office nie.
async fn convert_copy(source: &Path, target: &Path) -> Conversion {
    let dir = match make_temp_dir() {
        Ok(d) => d,
        Err(err) => return Conversion::error(format!("Datei konnte nicht kopiert werden: {err}")),
    };
    let result = convert_in(&dir, source, target).await;
    for attempt in 0..4 {
        if std::fs::remove_dir_all(&dir).is_ok() || !dir.exists() {
            break;
        }
        if attempt < 3 {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        // Office hält die Kopie noch: Windows räumt den Temp-Ordner auf
    }
    result
}

async fn convert_in(dir: &Path, source: &Path, target: &Path) -> Conversion {
    let copy_error = |err: std::io::Error| Conversion::error(format!("Datei konnte nicht kopiert werden: {err}"));
    let copy = dir.join(file_name(source));
    // Geprüft wird genau der Inhalt, den Office danach öffnet (nicht die Datei im Ordner, die sich ändern kann)
    let data = match std::fs::read(source) {
        Ok(d) => d,
        Err(err) => return copy_error(err),
    };
    let rejected = check_ooxml(&data);
    if !rejected.is_empty() {
        return Conversion::error(rejected);
    }
    if let Err(err) = std::fs::write(&copy, &data) {
        return copy_error(err);
    }
    let mut res = run_conversion(&copy, target).await;
    if res.fehler.as_deref().is_some_and(|f| DISCONNECTED.is_match(f)) {
        if let Err(err) = std::fs::write(&copy, &data) {
            return copy_error(err);
        }
        res = run_conversion(&copy, target).await;
    }
    res
}

// Aufträge laufen nacheinander (die Warteschlange der tokio-Sperre ist der Reihe nach)
static QUEUE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn mtime_ms(p: &Path) -> Option<f64> {
    let d = std::fs::metadata(p).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(d.as_secs() as f64 * 1000.0 + f64::from(d.subsec_nanos()) / 1e6)
}

/// Wandelt eine Datei des Arbeitsordners um. Aufträge laufen nacheinander. fresh: ein PDF, das jünger ist als
/// die Datei, gilt als aktuell (Vorschau beim Anklicken, ohne Office jedes Mal neu zu starten).
pub async fn convert_to_pdf(root: &Path, source: &Path, fresh: bool) -> Conversion {
    let ext = ext_of(&file_name(source)).to_lowercase();
    if !EXTENSIONS.contains(&ext.as_str()) {
        return Conversion::error("Nur .docx, .xlsx und .pptx werden umgewandelt.");
    }
    let target = pdf_target(root, source);
    if fresh {
        // noch kein PDF: umwandeln
        if let (Some(t), Some(s)) = (mtime_ms(&target), mtime_ms(source)) {
            if t >= s {
                return Conversion { ok: true, target: Some(target), cached: true, fehler: None };
            }
        }
    }
    let _turn = QUEUE.lock().await;
    let res = convert_copy(source, &target).await;
    if res.ok { Conversion { ok: true, target: Some(target), cached: false, fehler: None } } else { res }
}

fn program_of(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        ".docx" => Some("Word.Application"),
        ".xlsx" => Some("Excel.Application"),
        ".pptx" => Some("PowerPoint.Application"),
        _ => None,
    }
}

#[cfg(windows)]
async fn reg_query(key: &str) -> bool {
    let system_root = std::env::var("SystemRoot").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "C:\\Windows".to_string());
    let reg = Path::new(&system_root).join("System32").join("reg.exe");
    let mut cmd = tokio::process::Command::new(reg);
    cmd.args(["query", key])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    match tokio::time::timeout(std::time::Duration::from_secs(10), cmd.status()).await {
        Ok(Ok(status)) => status.success(),
        _ => false,
    }
}

/// Ist Word, Excel bzw. PowerPoint installiert? Einmal pro Programm in der Registrierung nachsehen.
#[cfg(windows)]
pub async fn office_installed(ext: &str) -> bool {
    static INSTALLED: tokio::sync::Mutex<Option<std::collections::HashMap<&'static str, bool>>> = tokio::sync::Mutex::const_new(None);
    let Some(prog) = program_of(ext) else { return false };
    let mut cache = INSTALLED.lock().await;
    let map = cache.get_or_insert_with(Default::default);
    if let Some(known) = map.get(prog) {
        return *known;
    }
    let found = reg_query(&format!("HKCR\\{prog}\\CLSID")).await;
    map.insert(prog, found);
    found
}

/// Ist Word, Excel bzw. PowerPoint installiert? Ausserhalb von Windows nie.
#[cfg(not(windows))]
pub async fn office_installed(ext: &str) -> bool {
    let _ = program_of(ext);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_ziel_im_ordner_ask_pdf() {
        let root = Path::new("arbeit");
        let t = |s: &str| pdf_target(root, Path::new(s)).strip_prefix("arbeit").unwrap().to_string_lossy().replace('\\', "/");
        assert_eq!(t("a/Bericht.docx"), ".ask/pdf/Bericht.pdf");
        assert_eq!(t("a/Liste.xlsx"), ".ask/pdf/Liste-xlsx.pdf");
        // wie path.basename(source, ext): eine grosse Endung bleibt im Namen
        assert_eq!(t("Folien.PPTX"), ".ask/pdf/Folien.PPTX-pptx.pdf");
        assert_eq!(t("A.DOCX"), ".ask/pdf/A.DOCX.pdf");
    }

    #[tokio::test]
    async fn andere_endungen_werden_nicht_umgewandelt() {
        let r = convert_to_pdf(Path::new("x"), Path::new("x/a.txt"), false).await;
        assert!(!r.ok);
        assert_eq!(r.fehler.as_deref(), Some("Nur .docx, .xlsx und .pptx werden umgewandelt."));
    }

    #[tokio::test]
    async fn aktuelles_pdf_gilt_mit_fresh_als_umgewandelt() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("a.docx");
        std::fs::write(&source, "x").unwrap();
        let target = pdf_target(dir.path(), &source);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "%PDF").unwrap();
        let r = convert_to_pdf(dir.path(), &source, true).await;
        assert!(r.ok && r.cached);
        assert_eq!(r.target, Some(target));
    }

    #[tokio::test]
    async fn gefaehrliche_dateien_werden_vor_office_abgelehnt() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("kaputt.docx");
        std::fs::write(&source, "kein zip").unwrap();
        let r = convert_to_pdf(dir.path(), &source, false).await;
        assert!(!r.ok);
        assert_eq!(r.fehler.as_deref(), Some("Die Datei ist kein gültiges Office-Dokument."));
    }

    #[cfg(not(windows))]
    #[tokio::test]
    async fn ohne_windows_ist_office_nicht_installiert() {
        assert!(!office_installed(".docx").await);
        assert!(!office_installed(".txt").await);
    }

    #[test]
    fn skript_ist_als_utf16_base64_kodiert() {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded_script()).unwrap();
        let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        assert_eq!(String::from_utf16(&units).unwrap(), SCRIPT);
    }
}
