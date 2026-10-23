// Build-Ordner für Entwicklung und Installer-Build (nicht für die installierte App).
// Reihenfolge: Umgebungsvariable ASK_BUILD_DIR, dann .ask-build-dir im Projekt (schreibt
// scripts/setup.ps1), sonst %LOCALAPPDATA%\ASK-Desktop-Build.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const repoRoot = path.join(__dirname, '..', '..');

function buildDir() {
  if (process.env.ASK_BUILD_DIR) return process.env.ASK_BUILD_DIR;
  try {
    const remembered = fs.readFileSync(path.join(repoRoot, '.ask-build-dir'), 'utf8').trim();
    if (remembered) return remembered;
  } catch {
    // noch kein setup.ps1 gelaufen
  }
  return path.join(process.env.LOCALAPPDATA || os.homedir(), 'ASK-Desktop-Build');
}

module.exports = { buildDir };
