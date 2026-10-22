# Handtest Phase 0: open-terminal lokal starten und in ASK von Hand verbinden.
# Aufruf:  powershell -ExecutionPolicy Bypass -File scripts\phase0-test.ps1 -AskUrl https://ask.example.ch
param(
    [string]$AskUrl = "*",
    [int]$Port = 8765
)
$ErrorActionPreference = "Stop"

# Kurzer Pfad, weil pip sonst an der 260-Zeichen-Grenze von Windows scheitert.
$base = Join-Path $env:USERPROFILE "ASK-Test"
$venv = Join-Path $base "venv"
$ordner = Join-Path $base "ordner"
New-Item -ItemType Directory -Force $ordner | Out-Null
if (-not (Test-Path (Join-Path $ordner "notiz.txt"))) {
    Set-Content -Encoding utf8 (Join-Path $ordner "notiz.txt") "Das ist eine Testdatei für ASK."
}

if (-not (Test-Path (Join-Path $venv "Scripts\python.exe"))) {
    Write-Host "Richte Python-Umgebung ein (einmalig, etwa 1 Minute) ..."
    python -m venv $venv
    & (Join-Path $venv "Scripts\python.exe") -m pip install --quiet --disable-pip-version-check open-terminal==0.14.2
}

$origin = $AskUrl
if ($AskUrl -ne "*") { $origin = ([Uri]$AskUrl).GetLeftPart([UriPartial]::Authority) }
& (Join-Path $venv "Scripts\python.exe") (Join-Path $PSScriptRoot "phase0_start.py") $ordner $Port $origin
