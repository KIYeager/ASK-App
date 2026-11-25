// Umwandlung von Word-, Excel- und PowerPoint-Dateien in PDF mit dem installierten Office.
//
// Das Modell läuft mit Low Integrity und kann Office nicht selbst steuern. Es legt deshalb einen
// Auftrag in .ask/auftraege/ ab; die App (normale Stufe) prüft die Datei und wandelt sie um.
// Abgelehnt werden Dateien mit Makros, ActiveX, OLE-Objekten, externen Verweisen oder
// verknüpften Feldern: zuerst hier in der App (ooxml.js, liest das XML wirklich), danach nochmals
// grob im PowerShell-Skript. Office öffnet die Datei schreibgeschützt mit deaktivierten Makros.
// Office bekommt immer eine Kopie im Temp-Ordner: Hängt oder stürzt es ab, bleibt die Datei im
// Arbeitsordner nicht gesperrt. Aus der Kopie wird ein Verweis auf eine externe Word-Vorlage
// entfernt (sonst sucht Word sie im Netz, hängt oder schickt Anmeldedaten an fremde Rechner).
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn } = require('node:child_process');
const { checkOoxml } = require('./ooxml');

const TIMEOUT_MS = 120000;
const EXTENSIONS = new Set(['.docx', '.xlsx', '.pptx']);

const SCRIPT = String.raw`
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
`;

const encoded = Buffer.from(SCRIPT, 'utf16le').toString('base64');

let queue = Promise.resolve();

function runConversion(source, target) {
  return new Promise((resolve) => {
    fs.mkdirSync(path.dirname(target), { recursive: true });
    fs.rmSync(target, { force: true });
    const ps = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe');
    const child = spawn(ps, ['-NoProfile', '-NonInteractive', '-EncodedCommand', encoded], {
      windowsHide: true, env: { ...process.env, ASK_QUELLE: source, ASK_ZIEL: target },
    });
    let pids = [];
    let result = null;
    let buffer = '';
    child.stdout.on('data', (chunk) => {
      buffer += chunk.toString('utf8');
      let i;
      while ((i = buffer.indexOf('\n')) >= 0) {
        const line = buffer.slice(0, i).trim();
        buffer = buffer.slice(i + 1);
        try {
          const msg = JSON.parse(line);
          if (msg.pids) pids = [].concat(msg.pids).filter(Boolean);
          if (msg.ok || msg.fehler) result = msg;
        } catch { /* andere Ausgabe ignorieren */ }
      }
    });
    const timer = setTimeout(() => {
      child.kill();
      // Nur die Office-Prozesse beenden, die für diese Umwandlung gestartet wurden.
      for (const pid of pids) { try { process.kill(pid); } catch { /* bereits beendet */ } }
      result = { fehler: 'Office hat nicht innerhalb von 2 Minuten geantwortet.' };
    }, TIMEOUT_MS);
    child.on('exit', () => {
      clearTimeout(timer);
      resolve(result || { fehler: 'PowerShell konnte Office nicht starten (eventuell durch Richtlinien gesperrt).' });
    });
    child.on('error', (err) => {
      clearTimeout(timer);
      resolve({ fehler: err.message });
    });
  });
}

// Office verliert manchmal die Verbindung (RPC_E_DISCONNECTED); dann einmal mit neuer Instanz versuchen.
const DISCONNECTED = /RPC_E_|RPC-Server|getrennt|disconnected|0x80010108|0x800706BA|0x800706BE/i;

// Umwandlung einer Kopie im Temp-Ordner; das Original im Arbeitsordner öffnet Office nie.
async function convertCopy(source, target) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `ask-pdf-${crypto.randomBytes(3).toString('hex')}-`));
  try {
    const copy = path.join(dir, path.basename(source));
    // Geprüft wird genau der Inhalt, den Office danach öffnet (nicht die Datei im Ordner, die sich ändern kann)
    const data = fs.readFileSync(source);
    const rejected = checkOoxml(data);
    if (rejected) return { fehler: rejected };
    fs.writeFileSync(copy, data);
    let res = await runConversion(copy, target);
    if (res.fehler && DISCONNECTED.test(res.fehler)) {
      fs.writeFileSync(copy, data);
      res = await runConversion(copy, target);
    }
    return res;
  } catch (err) {
    return { fehler: `Datei konnte nicht kopiert werden: ${err.message}` };
  } finally {
    try { fs.rmSync(dir, { recursive: true, force: true, maxRetries: 3 }); } catch { /* Office hält die Kopie noch: Windows räumt den Temp-Ordner auf */ }
  }
}

// Ziel der Umwandlung: .ask/pdf/<name>.pdf
function pdfTarget(root, source) {
  const ext = path.extname(source).toLowerCase();
  return path.join(root, '.ask', 'pdf', `${path.basename(source, ext)}${ext === '.docx' ? '' : ext.replace('.', '-')}.pdf`);
}

// Wandelt eine Datei des Arbeitsordners um. Aufträge laufen nacheinander. fresh: ein PDF, das jünger ist als
// die Datei, gilt als aktuell (Vorschau beim Anklicken, ohne Office jedes Mal neu zu starten).
function convertToPdf(root, source, { fresh = false } = {}) {
  const ext = path.extname(source).toLowerCase();
  if (!EXTENSIONS.has(ext)) return Promise.resolve({ fehler: 'Nur .docx, .xlsx und .pptx werden umgewandelt.' });
  const target = pdfTarget(root, source);
  if (fresh) {
    try {
      if (fs.statSync(target).mtimeMs >= fs.statSync(source).mtimeMs) return Promise.resolve({ ok: true, target, cached: true });
    } catch { /* noch kein PDF */ }
  }
  const job = queue.then(() => convertCopy(source, target)).then((res) => (res.ok ? { ok: true, target } : res));
  queue = job.catch(() => {});
  return job;
}

// Ist Word, Excel bzw. PowerPoint installiert? Einmal pro Programm in der Registrierung nachsehen.
const PROGRAMS = { '.docx': 'Word.Application', '.xlsx': 'Excel.Application', '.pptx': 'PowerPoint.Application' };
const installed = new Map();
function officeInstalled(ext, { platform = process.platform, run = regQuery } = {}) {
  const prog = PROGRAMS[String(ext).toLowerCase()];
  if (!prog || platform !== 'win32') return Promise.resolve(false);
  if (!installed.has(prog)) installed.set(prog, run(`HKCR\\${prog}\\CLSID`).catch(() => false));
  return installed.get(prog);
}

function regQuery(key) {
  return new Promise((resolve) => {
    const reg = path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'reg.exe');
    const child = spawn(reg, ['query', key], { windowsHide: true, stdio: 'ignore' });
    const timer = setTimeout(() => { child.kill(); resolve(false); }, 10000);
    child.on('error', () => { clearTimeout(timer); resolve(false); });
    child.on('exit', (code) => { clearTimeout(timer); resolve(code === 0); });
  });
}

module.exports = { convertToPdf, officeInstalled, pdfTarget };
