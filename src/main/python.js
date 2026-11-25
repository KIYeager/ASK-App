// Mitgeliefertes Python in der installierten App und der einzelnen EXE (ENTSCHEIDE E39).
// Es liegt als ein Archiv in resources\python-pack (python.zip und python.json mit { id, files, entries }).
// Vor dem ersten Terminal-Start einer neuen Fassung entpackt die App es mit tar.exe von Windows nach
// %LOCALAPPDATA%\ASK-Desktop\python\<id>, zuerst nach <id>.neu und erst nach vollständigem Entpacken
// umbenannt (wie der Starter der einzelnen EXE, E18). Ältere Fassungen räumt sie danach weg.
// Im Entwicklungsmodus (npm start) gibt es kein Archiv; dort gilt Build-Ordner\python direkt.
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');

const DONE = '.fertig';

function readManifest(packDir) {
  try {
    const m = JSON.parse(fs.readFileSync(path.join(packDir, 'python.json'), 'utf8'));
    return m && /^[0-9a-f]{8,64}$/.test(m.id) ? m : null;
  } catch {
    return null;
  }
}

const runtimeRoot = (env = process.env) => (env.LOCALAPPDATA ? path.join(env.LOCALAPPDATA, 'ASK-Desktop', 'python') : '');

class PythonRuntime {
  // packDir: resources\python-pack; root: %LOCALAPPDATA%\ASK-Desktop\python; tarExe und spawnImpl für Tests.
  constructor({ packDir, root, tarExe, spawnImpl = spawn, log = () => {} }) {
    this.packDir = packDir;
    this.root = root;
    this.tarExe = tarExe || path.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'tar.exe');
    this.spawn = spawnImpl;
    this.log = log;
    this.manifest = readManifest(packDir);
    this.pending = null;
  }

  dir() {
    return this.manifest && this.root ? path.join(this.root, this.manifest.id) : '';
  }

  exe() {
    return this.dir() ? path.join(this.dir(), 'python.exe') : '';
  }

  ready() {
    const dir = this.dir();
    return !!dir && fs.existsSync(path.join(dir, DONE)) && fs.existsSync(this.exe());
  }

  // Sorgt dafür, dass Python entpackt ist. onProgress(prozent) während des Entpackens.
  ensure(onProgress = () => {}) {
    if (!this.manifest) return Promise.reject(new Error('Das mitgelieferte Python fehlt in der App (python-pack). Bitte ASK neu installieren.'));
    if (!this.root) return Promise.reject(new Error('LOCALAPPDATA ist nicht gesetzt; Python kann nicht eingerichtet werden.'));
    if (this.ready()) {
      this.cleanup();
      return Promise.resolve(this.dir());
    }
    if (!this.pending) {
      this.pending = this.extract(onProgress).finally(() => { this.pending = null; });
    }
    return this.pending;
  }

  async extract(onProgress) {
    const dir = this.dir();
    const tmp = `${dir}.neu`;
    fs.rmSync(tmp, { recursive: true, force: true });
    fs.mkdirSync(tmp, { recursive: true });
    const total = Math.max(1, Number(this.manifest.entries) || Number(this.manifest.files) || 1);
    this.log(`Entpacke Python ${this.manifest.id} nach ${dir}`);
    onProgress(0);
    await new Promise((resolve, reject) => {
      const child = this.spawn(this.tarExe, ['-x', '-v', '-f', path.join(this.packDir, 'python.zip'), '-C', tmp], { windowsHide: true });
      let count = 0;
      let last = -1;
      let tail = '';
      const onData = (chunk) => {
        const text = String(chunk);
        tail = (tail + text).slice(-400);
        count += (text.match(/\n/g) || []).length;
        const percent = Math.min(99, Math.floor((count / total) * 100));
        if (percent !== last) {
          last = percent;
          onProgress(percent);
        }
      };
      child.stdout?.on('data', onData);
      child.stderr?.on('data', onData);
      child.on('error', (err) => reject(new Error(`Python konnte nicht entpackt werden: ${err.message}`)));
      child.on('exit', (code) => {
        if (code === 0) resolve();
        else reject(new Error(`Python konnte nicht entpackt werden (tar, Code ${code}). ${tail.trim().split('\n').pop() || ''}`.trim()));
      });
    }).catch((err) => {
      fs.rmSync(tmp, { recursive: true, force: true });
      throw err;
    });
    if (!fs.existsSync(path.join(tmp, 'python.exe'))) {
      fs.rmSync(tmp, { recursive: true, force: true });
      throw new Error('Python konnte nicht entpackt werden: python.exe fehlt im Archiv.');
    }
    fs.writeFileSync(path.join(tmp, DONE), `${new Date().toISOString()}\n`);
    fs.rmSync(dir, { recursive: true, force: true }); // halb entpackter Stand ohne .fertig
    fs.renameSync(tmp, dir);
    onProgress(100);
    this.cleanup();
    return dir;
  }

  // Ältere Fassungen und abgebrochene Entpackungen entfernen. Nur mit der Einzelinstanz-Sperre aufrufen.
  cleanup() {
    let entries = [];
    try {
      entries = fs.readdirSync(this.root, { withFileTypes: true });
    } catch {
      return;
    }
    const own = this.manifest.id.toLowerCase();
    for (const entry of entries) {
      if (!entry.isDirectory() || entry.name.toLowerCase() === own) continue;
      fs.promises.rm(path.join(this.root, entry.name), { recursive: true, force: true, maxRetries: 2 }).catch(() => {});
    }
  }
}

module.exports = { PythonRuntime, readManifest, runtimeRoot, DONE };
