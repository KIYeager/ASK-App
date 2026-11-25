// Pfade zu mitgelieferten Teilen, getrennt nach Entwicklung (npm start) und installierter App.
const path = require('node:path');
const crypto = require('node:crypto');
const { app } = require('electron');
const { buildDir } = require('./build-dir');

const repoRoot = path.join(__dirname, '..', '..');

// Python der installierten App: einmal entpackt nach %LOCALAPPDATA%\ASK-Desktop\python\<id> (python.js, ENTSCHEIDE E39)
let runtime = null;
function pythonRuntime() {
  if (!runtime) {
    const { PythonRuntime, runtimeRoot } = require('./python');
    runtime = new PythonRuntime({ packDir: resources('python-pack'), root: runtimeRoot() });
  }
  return runtime;
}

function resources(...parts) {
  return app.isPackaged ? path.join(process.resourcesPath, ...parts) : path.join(repoRoot, 'resources', ...parts);
}

module.exports = {
  pythonExe: () => app.isPackaged
    ? pythonRuntime().exe()
    : path.join(buildDir(), 'python', 'python.exe'),
  pythonRuntime,
  launcherPy: () => app.isPackaged
    ? path.join(process.resourcesPath, 'ask_terminal', 'launcher.py')
    : path.join(repoRoot, 'python', 'ask_terminal', 'launcher.py'),
  skillsDir: () => resources('skills'),
  instructionsDir: () => resources('anweisungen'),
  appConfigFile: () => resources('app-config.json'),
  iconFile: () => resources('icon.ico'),
  rendererDir: () => path.join(repoRoot, 'src', 'renderer'),
  preloadDir: () => path.join(repoRoot, 'src', 'preload'),
  // Daten der App zu einem Arbeitsordner, die das Modell nicht ändern darf (Sicherung für «Rückgängig»,
  // Verbindung mit GitHub). Liegt darum nicht im Arbeitsordner und nicht im Laufzeitordner des
  // Terminals: Beide sind für das Terminal beschreibbar.
  projectDataDir: (folder) => {
    const base = (!app.isPackaged && process.env.ASK_TEST_USER_DATA)
      ? path.join(process.env.ASK_TEST_USER_DATA, 'projekte')
      : path.join(process.env.LOCALAPPDATA || app.getPath('userData'), 'ASK-Desktop', 'projekte');
    const key = crypto.createHash('sha1').update(path.resolve(folder).toLowerCase()).digest('hex').slice(0, 20);
    return path.join(base, key);
  },
  // Laufzeitdaten des Terminals (temporäre Dateien, Logs). Muss lokal sein, nicht im Roaming-Profil.
  runtimeDir: () => (!app.isPackaged && process.env.ASK_RUNTIME_DIR)
    || path.join(process.env.LOCALAPPDATA || app.getPath('userData'), 'ASK-Desktop', 'laufzeit'),
};
