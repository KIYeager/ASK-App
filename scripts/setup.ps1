# Richtet die Entwicklungsumgebung ein: npm-Pakete und mitgeliefertes Python im Build-Ordner
# ausserhalb von OneDrive, im Projekt nur eine Junction auf node_modules.
# Aufruf im Projektordner:  powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
#
# Wichtig: «npm install» nicht direkt im Projektordner ausführen. npm ersetzt die Junction sonst
# durch einen echten Ordner, und OneDrive synchronisiert mehrere hundert MB.
param(
    [string]$BuildDir = $(if ($env:ASK_BUILD_DIR) { $env:ASK_BUILD_DIR } else { Join-Path $env:LOCALAPPDATA "ASK-Desktop-Build" }),
    [switch]$OhnePython
)
$ErrorActionPreference = "Stop"
$repo = Split-Path $PSScriptRoot -Parent
$deps = Join-Path $BuildDir "deps"
New-Item -ItemType Directory -Force $deps | Out-Null
Write-Host "Build-Ordner: $BuildDir"

# npm-Pakete im Build-Ordner installieren (genau nach package-lock.json).
Copy-Item (Join-Path $repo "package.json"), (Join-Path $repo "package-lock.json") $deps -Force
Push-Location $deps
try {
    npm ci --no-audit --no-fund
    if ($LASTEXITCODE -ne 0) { throw "npm ci fehlgeschlagen" }
    # Neuere npm-Versionen führen Installationsskripte nur nach Freigabe aus; Electron lädt hier seine Laufzeit.
    node (Join-Path $deps "node_modules\electron\install.js")
    if ($LASTEXITCODE -ne 0) { throw "Electron konnte nicht geladen werden" }
} finally { Pop-Location }

# Junction im Projekt auf die installierten Pakete.
$link = Join-Path $repo "node_modules"
$target = Join-Path $deps "node_modules"
$item = Get-Item $link -Force -ErrorAction SilentlyContinue
if ($item) {
    if (-not ($item.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
        throw "$link ist ein normaler Ordner (z. B. nach «npm install» im Projekt). Bitte löschen und das Skript erneut ausführen."
    }
    cmd /c rmdir "$link" | Out-Null
}
cmd /c mklink /J "$link" "$target" | Out-Null
Write-Host "node_modules -> $target"

if (-not $OhnePython) { & (Join-Path $PSScriptRoot "build-python.ps1") -BuildDir $BuildDir }

# Build-Ordner merken, damit npm start und npm run dist ihn ohne Umgebungsvariable finden.
[IO.File]::WriteAllText((Join-Path $repo ".ask-build-dir"), $BuildDir)

Write-Host ""
Write-Host "Fertig. Starten: npm start    Tests: npm test    Installer: npm run dist"
