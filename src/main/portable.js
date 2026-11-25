// Einzelne EXE ohne Installation (ENTSCHEIDE E18). Der Starter (scripts/starter.nsi) entpackt die
// App beim ersten Start nach %LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build> und ruft sie danach
// direkt von dort auf. Nach einem Update bleiben ältere Stände liegen (je rund 650 MB), dazu
// abgebrochene Entpackungen (*.neu). Diese räumt die App beim Start weg.
const fs = require('node:fs');
const path = require('node:path');

// Ein jüngerer .neu-Ordner kann einem Starter gehören, der gerade entpackt.
const MIN_AGE_MS = 60 * 60 * 1000;

function appCacheRoot(env = process.env) {
  return env.LOCALAPPDATA ? path.join(env.LOCALAPPDATA, 'ASK-Desktop', 'app') : '';
}

// Ältere Stände und abgebrochene Entpackungen in root, ohne den eigenen Stand (ownName).
function staleAppDirs(root, ownName, now = Date.now()) {
  let entries;
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    return [];
  }
  const result = [];
  for (const entry of entries) {
    if (!entry.isDirectory() || entry.name.toLowerCase() === ownName.toLowerCase()) continue;
    const dir = path.join(root, entry.name);
    try {
      if (/\.neu$/i.test(entry.name) && now - fs.statSync(dir).birthtimeMs < MIN_AGE_MS) continue;
      result.push(dir);
    } catch {
      // verschwunden oder nicht lesbar
    }
  }
  return result;
}

// Läuft die App aus der einzelnen EXE (entpackt unter appCacheRoot) statt installiert?
function isPortable(execPath = process.execPath, env = process.env) {
  const root = appCacheRoot(env);
  return !!root && path.dirname(path.dirname(execPath)).toLowerCase() === root.toLowerCase();
}

// Nur mit der Einzelinstanz-Sperre aufrufen: Dann läuft kein anderer Stand mehr.
function cleanupOldVersions() {
  if (!isPortable()) return;
  const root = appCacheRoot();
  const ownDir = path.dirname(process.execPath);
  // Das fs von Electron behandelt app.asar als Ordner und kann sie nicht löschen.
  const rawFs = require('original-fs');
  for (const dir of staleAppDirs(root, path.basename(ownDir))) {
    rawFs.promises.rm(dir, { recursive: true, force: true, maxRetries: 2 }).catch(() => {});
  }
}

module.exports = { appCacheRoot, isPortable, staleAppDirs, cleanupOldVersions, MIN_AGE_MS };
