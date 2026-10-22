// Umwandlung von Word-, Excel- und PowerPoint-Dateien in PDF mit dem installierten Office.
//
// Das Modell läuft mit Low Integrity und kann Office nicht selbst steuern. Es legt deshalb einen
// Auftrag in .ask/auftraege/ ab; die App (normale Stufe) prüft die Datei und wandelt sie um.
// Abgelehnt werden Dateien mit Makros, ActiveX, OLE-Objekten, externen Verweisen oder
// verknüpften Feldern. Office öffnet die Datei schreibgeschützt mit deaktivierten Makros.
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');

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

  $ext = [IO.Path]::GetExtension($Quelle).ToLower()
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

// Wandelt eine Datei des Arbeitsordners um. Ziel: .ask/pdf/<name>.pdf. Aufträge laufen nacheinander.
function convertToPdf(root, source) {
  const ext = path.extname(source).toLowerCase();
  if (!EXTENSIONS.has(ext)) return Promise.resolve({ fehler: 'Nur .docx, .xlsx und .pptx werden umgewandelt.' });
  const target = path.join(root, '.ask', 'pdf', `${path.basename(source, ext)}${ext === '.docx' ? '' : ext.replace('.', '-')}.pdf`);
  const job = queue.then(() => runConversion(source, target)).then((res) => (res.ok ? { ok: true, target } : res));
  queue = job.catch(() => {});
  return job;
}

module.exports = { convertToPdf };
