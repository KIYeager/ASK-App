// Packt das mitgelieferte Python (Build-Ordner\python, von build-python.ps1) in ein einziges Archiv
// Build-Ordner\python-pack\python.zip mit python.json (ENTSCHEIDE E39). Tauri legt den Ordner
// als resources\python-pack in die App; die App entpackt ihn beim ersten Start einer neuen Fassung
// (src-tauri/core/src/python.rs). Statt rund 12'000 Dateien schreibt ein Update so nur noch das Archiv neu.
//
// Die Fassung (id) hängt nur von build-python.ps1 und python/requirements.txt ab: Gleiches Python ergibt
// dieselbe id, und installierte Apps entpacken nach einem Update nichts neu.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

const repo = path.join(__dirname, '..');

function pythonId(read = (f) => fs.readFileSync(path.join(repo, f))) {
  const hash = crypto.createHash('sha256');
  for (const file of ['scripts/build-python.ps1', 'python/requirements.txt']) {
    hash.update(file).update('\0').update(read(file)).update('\0');
  }
  return hash.digest('hex').slice(0, 16);
}

// Dateien und Ordner zählen (für die Fortschrittsanzeige beim Entpacken)
function countEntries(dir) {
  let files = 0;
  let dirs = 0;
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true })) {
      if (entry.isDirectory()) {
        dirs++;
        walk(path.join(d, entry.name));
      } else {
        files++;
      }
    }
  };
  walk(dir);
  return { files, dirs };
}

function pack(buildDir = require('./build-dir').buildDir()) {
  const source = path.join(buildDir, 'python');
  const out = path.join(buildDir, 'python-pack');
  const zip = path.join(out, 'python.zip');
  const manifestFile = path.join(out, 'python.json');
  if (!fs.existsSync(path.join(source, 'python.exe'))) {
    throw new Error(`Python fehlt in ${source}. Zuerst scripts\\build-python.ps1 ausführen.`);
  }
  const id = pythonId();
  try {
    const old = JSON.parse(fs.readFileSync(manifestFile, 'utf8'));
    if (old.id === id && fs.statSync(zip).mtimeMs > fs.statSync(path.join(source, 'python.exe')).mtimeMs) {
      console.log(`Python-Archiv ist aktuell (${id}).`);
      return old;
    }
  } catch {
    // noch kein Archiv
  }
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });
  const { files, dirs } = countEntries(source);
  console.log(`Packe ${files} Dateien aus ${source} nach ${zip} …`);
  // tar.exe von Windows (bsdtar) schreibt mit -a und der Endung .zip ein ZIP-Archiv.
  const tar = process.platform === 'win32' ? path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe') : 'bsdtar';
  execFileSync(tar, ['-a', '-c', '-f', zip, '-C', source, '.'], { stdio: 'inherit' });
  const manifest = { id, files, entries: files + dirs, bytes: fs.statSync(zip).size };
  fs.writeFileSync(manifestFile, `${JSON.stringify(manifest, null, 2)}\n`);
  console.log(`Fertig: ${Math.round(manifest.bytes / 1024 / 1024)} MB, Fassung ${id}.`);
  return manifest;
}

if (require.main === module) {
  try {
    pack();
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
}

module.exports = { pack, pythonId, countEntries };
