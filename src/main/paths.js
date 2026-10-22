// Pfade zu mitgelieferten Teilen, getrennt nach Entwicklung (npm start) und installierter App.
const path = require('node:path');
const os = require('node:os');
const { app } = require('electron');

const repoRoot = path.join(__dirname, '..', '..');

function buildDir() {
  return process.env.ASK_BUILD_DIR || path.join(process.env.LOCALAPPDATA || os.homedir(), 'ASK-Desktop-Build');
}

function resources(...parts) {
  return app.isPackaged ? path.join(process.resourcesPath, ...parts) : path.join(repoRoot, 'resources', ...parts);
}

module.exports = {
  pythonExe: () => app.isPackaged
    ? path.join(process.resourcesPath, 'python', 'python.exe')
    : path.join(buildDir(), 'python', 'python.exe'),
  launcherPy: () => app.isPackaged
    ? path.join(process.resourcesPath, 'ask_terminal', 'launcher.py')
    : path.join(repoRoot, 'python', 'ask_terminal', 'launcher.py'),
  skillsDir: () => resources('skills'),
  instructionsDir: () => resources('anweisungen'),
  appConfigFile: () => resources('app-config.json'),
  iconFile: () => resources('icon.ico'),
  rendererDir: () => path.join(repoRoot, 'src', 'renderer'),
  preloadDir: () => path.join(repoRoot, 'src', 'preload'),
  // Laufzeitdaten des Terminals (temporäre Dateien, Logs). Muss lokal sein, nicht im Roaming-Profil.
  runtimeDir: () => process.env.ASK_RUNTIME_DIR
    || path.join(process.env.LOCALAPPDATA || app.getPath('userData'), 'ASK-Desktop', 'laufzeit'),
};
