# Spike unter Windows (WebView2) durchlaufen lassen. Im Container nicht getestet (Linux).
# Voraussetzungen: Rust (rustup, MSVC), WebView2 Runtime (in Windows 10/11 vorhanden), Node 22, im Repo «npm ci».
# Aufruf aus dem Repo:  powershell -ExecutionPolicy Bypass -File spike\tauri\run-windows.ps1
# Ergebnis: spike\tauri\results\windows\run.log und 4-browser-snapshot.png (Bildschirmfoto über CDP).
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$repo = Resolve-Path (Join-Path $here '..\..')
$out = Join-Path $here 'results\windows'
New-Item -ItemType Directory -Force $out | Out-Null
$work = Join-Path $env:TEMP "ask-tauri-spike-$PID"
New-Item -ItemType Directory -Force (Join-Path $work 'folder\daten') | Out-Null
Set-Content (Join-Path $work 'folder\ausgaben.csv') 'Datum;Kategorie;Betrag'

Push-Location $repo; npx vite build | Out-Null; Pop-Location
node (Join-Path $here 'tools\gen-shim.mjs')
Push-Location (Join-Path $here 'app'); cargo build; Pop-Location
$fx = Start-Process node -ArgumentList (Join-Path $here 'fixtures\server.mjs') -PassThru -WindowStyle Hidden
try {
  Start-Sleep -Milliseconds 500
  $env:SPIKE_SCENARIO = '1'; $env:SPIKE_OUT = $out; $env:SPIKE_DIR = $here
  $env:SPIKE_FOLDER = Join-Path $work 'folder'; $env:SPIKE_DATA = Join-Path $work 'data'
  & (Join-Path $here 'app\target\debug\ask-tauri-spike.exe') | Tee-Object (Join-Path $out 'run.log')
} finally {
  Stop-Process $fx -ErrorAction SilentlyContinue
  Remove-Item -Recurse -Force $work -ErrorAction SilentlyContinue
}
# Zu prüfen in run.log: click-trusted / type-trusted «Ok(())» (CDP), console-cdp, snapshot, navigation, download,
# new-window, ipc-from-browser «abgelehnt», chat-done-visible true. Danach Klicks per CDP: «isTrusted=true»
# in der Seite, siehe 4-browser-snapshot.png.
