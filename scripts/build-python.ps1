# Baut das mitgelieferte Python für ASK Desktop.
# Quelle: offizielles NuGet-Paket «python» der Python Software Foundation (normale Ordnerstruktur,
# verschiebbar, mit pip). Ergebnis: $BuildDir\python; scripts/pack-python.js packt es für die App (ENTSCHEIDE E39).
param(
    [string]$BuildDir = $(if ($env:ASK_BUILD_DIR) { $env:ASK_BUILD_DIR } else { Join-Path $env:LOCALAPPDATA "ASK-Desktop-Build" })
)
$ErrorActionPreference = "Stop"
$version = "3.14.8"
# SHA512 laut NuGet-Katalog (packageHash, Base64)
$expectedHash = "b9oz/C2qiLlE7FKAogz3dyiT38vFX3d9K2IqhooLZmcL3E9REJsgrg3eUXeTHLXRy50jgfHEoX1bx3u91Vjeaw=="
$repo = Split-Path $PSScriptRoot -Parent
$cache = Join-Path $BuildDir "cache"
$target = Join-Path $BuildDir "python"
$nupkg = Join-Path $cache "python.$version.nupkg"

New-Item -ItemType Directory -Force $cache | Out-Null
if (-not (Test-Path $nupkg)) {
    Write-Host "Lade Python $version ..."
    Invoke-WebRequest -UseBasicParsing "https://api.nuget.org/v3-flatcontainer/python/$version/python.$version.nupkg" -OutFile $nupkg
}
$hash = [Convert]::ToBase64String([Security.Cryptography.SHA512]::Create().ComputeHash([IO.File]::ReadAllBytes($nupkg)))
if ($hash -ne $expectedHash) { throw "Prüfsumme von $nupkg stimmt nicht." }

Write-Host "Entpacke nach $target ..."
if (Test-Path $target) { Remove-Item -Recurse -Force $target }
Add-Type -AssemblyName System.IO.Compression.FileSystem
$zip = [IO.Compression.ZipFile]::OpenRead($nupkg)
try {
    foreach ($entry in $zip.Entries) {
        if (-not $entry.FullName.StartsWith("tools/") -or $entry.FullName.EndsWith("/")) { continue }
        $dest = Join-Path $target ($entry.FullName.Substring(6) -replace "/", "\")
        New-Item -ItemType Directory -Force (Split-Path $dest -Parent) | Out-Null
        [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, $dest, $true)
    }
} finally { $zip.Dispose() }

$python = Join-Path $target "python.exe"
$env:PIP_DISABLE_PIP_VERSION_CHECK = "1"
Write-Host "Installiere Pakete ..."
& $python -m pip install --no-compile --no-warn-script-location --only-binary=:all: -r (Join-Path $repo "python\requirements.txt")
if ($LASTEXITCODE -ne 0) { throw "pip install fehlgeschlagen" }

Write-Host "Entferne Testsuiten und Entwicklungsdateien ..."
$site = Join-Path $target "Lib\site-packages"
$remove = @("pandas\tests", "matplotlib\tests", "matplotlib\mpl-data\sample_data") |
    ForEach-Object { Join-Path $site $_ }
$remove += Get-ChildItem (Join-Path $site "numpy") -Directory -Recurse -Filter tests | ForEach-Object FullName
$remove += Join-Path $target "include"
foreach ($p in $remove) { if (Test-Path $p) { Remove-Item -Recurse -Force $p } }
Get-ChildItem $target -Directory -Recurse -Filter __pycache__ | Remove-Item -Recurse -Force

# Vorkompilieren: Mit Low Integrity kann Python keine .pyc-Dateien ins Programmverzeichnis schreiben.
Write-Host "Kompiliere ..."
# unchecked-hash: Python prüft die .pyc nicht gegen Zeitstempel. Nach dem Entpacken aus python.zip (ENTSCHEIDE E39)
# stimmen die Zeitstempel nicht immer auf die Sekunde, und ohne Schreibrecht würde jeder Import neu übersetzen.
& $python -m compileall -q -j 0 --invalidation-mode unchecked-hash (Join-Path $target "Lib") | Out-Null

$files = Get-ChildItem $target -Recurse -File
Write-Host ("Fertig: {0:N0} Dateien, {1:N0} MB in {2}" -f $files.Count, (($files | Measure-Object Length -Sum).Sum / 1MB), $target)
