// Arbeitsordner: Grundstruktur anlegen, Skills kopieren, Änderungen beobachten, Dateien auflisten.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { EventEmitter } = require('node:events');
const paths = require('./paths');
const { cleanupWorkDirs, hideFolder } = require('./cleanup');

const ASK_DIR = '.ask';
const MAX_ENTRIES = 300;
const RECENT = 12; // Seitenleiste «Zuletzt geändert»
const HIDDEN = /^(\.ask|\.git|node_modules|__pycache__|\.venv)$|^~\$/i; // nicht in der Seitenleiste

function listRecursive(root, dir, depth, out) {
  if (out.length >= MAX_ENTRIES || depth < 0) return;
  let entries;
  try {
    entries = fs.readdirSync(dir, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    if (out.length >= MAX_ENTRIES) return;
    const full = path.join(dir, entry.name);
    const rel = path.relative(root, full);
    if (rel === ASK_DIR || rel.startsWith(ASK_DIR + path.sep)) continue;
    let stat;
    try {
      stat = fs.statSync(full);
    } catch {
      continue;
    }
    out.push({ path: rel, name: entry.name, dir: entry.isDirectory(), size: stat.size, modified: stat.mtimeMs,
      depth: rel.split(path.sep).length - 1 });
    if (entry.isDirectory()) listRecursive(root, full, depth - 1, out);
  }
}

// Zuletzt geänderte Dateien im ganzen Ordner (ohne .ask, .git, node_modules usw.), neueste zuerst.
function recentFiles(root) {
  const files = [];
  let seen = 0;
  const walk = (dir, depth) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      if (++seen > 5000 || HIDDEN.test(entry.name)) continue;
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        if (depth > 0) walk(full, depth - 1);
      } else {
        try {
          const stat = fs.statSync(full);
          files.push({ path: path.relative(root, full), name: entry.name, dir: false, size: stat.size, modified: stat.mtimeMs });
        } catch {
          // gerade gelöscht
        }
      }
    }
  };
  walk(root, 6);
  return files.sort((a, b) => b.modified - a.modified).slice(0, RECENT);
}

function hashDir(dir) {
  const hash = crypto.createHash('sha1');
  const walk = (d) => {
    for (const entry of fs.readdirSync(d, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const full = path.join(d, entry.name);
      if (entry.isDirectory()) walk(full);
      else hash.update(path.relative(dir, full)).update(fs.readFileSync(full));
    }
  };
  walk(dir);
  return hash.digest('hex');
}

class Workspace extends EventEmitter {
  constructor() {
    super();
    this.root = '';
    this.watcher = null;
    this.timer = null;
  }

  open(folder) {
    this.close();
    this.root = folder;
    fs.mkdirSync(path.join(folder, ASK_DIR), { recursive: true });
    hideFolder(path.join(folder, ASK_DIR));
    setTimeout(() => cleanupWorkDirs(folder), 2000); // nicht beim Öffnen bremsen; die Sicherung räumt main.js auf
    fs.rmSync(path.join(folder, ASK_DIR, 'auftraege'), { recursive: true, force: true });
    this.syncSkills();
    try {
      this.watcher = fs.watch(folder, { recursive: true }, (event, name) => this.onChange(name || ''));
      this.watcher.on('error', () => {});
    } catch {
      this.watcher = null;
    }
    this.emit('files', this.files());
  }

  close() {
    if (this.watcher) this.watcher.close();
    this.watcher = null;
    clearTimeout(this.timer);
    clearTimeout(this.cleanupTimer);
    for (const t of this.orderTimers?.values() || []) clearTimeout(t);
    this.orderTimers = new Map();
    this.root = '';
  }

  // Kopiert die mitgelieferten Anleitungen nach .ask/skills, wenn sich ihr Inhalt geändert hat.
  syncSkills() {
    const source = paths.skillsDir();
    if (!fs.existsSync(source)) return;
    const target = path.join(this.root, ASK_DIR, 'skills');
    const marker = path.join(target, '.version');
    const version = hashDir(source);
    try {
      if (fs.readFileSync(marker, 'utf8') === version) return;
    } catch {
      // noch nicht kopiert
    }
    fs.rmSync(target, { recursive: true, force: true });
    fs.cpSync(source, target, { recursive: true });
    fs.writeFileSync(marker, version);
  }

  onChange(name) {
    if (['skills', 'chats', 'skripte', 'arbeit', 'anhaenge', 'sicherung'].some((sub) => name.startsWith(path.join(ASK_DIR, sub)))) return;
    if (/^\.ask[\\/]auftraege[\\/][0-9a-f]{6,32}\.json$/i.test(name)) {
      // Erst melden, wenn die Datei eine Weile unverändert ist: sonst wäre sie vielleicht halb geschrieben
      const file = path.join(this.root, name);
      this.orderTimers = this.orderTimers || new Map();
      clearTimeout(this.orderTimers.get(file));
      this.orderTimers.set(file, setTimeout(() => {
        this.orderTimers.delete(file);
        if (fs.existsSync(file)) this.emit('auftrag', file);
      }, 300));
      return;
    }
    if (name.startsWith(path.join(ASK_DIR, 'auftraege')) || name.startsWith(path.join(ASK_DIR, 'pdf'))) return;
    if (name.toLowerCase() === path.join(ASK_DIR, 'tasks.json').toLowerCase()) return; // Datei früherer Versionen
    clearTimeout(this.timer);
    this.timer = setTimeout(() => { if (this.root) this.emit('files', this.files()); }, 250);
  }

  files() {
    if (!this.root) return { top: [], recent: [] };
    const top = [];
    listRecursive(this.root, this.root, 0, top);
    const sortTop = (a, b) => (b.dir - a.dir) || a.name.localeCompare(b.name, 'de');
    return { top: top.sort(sortTop), recent: recentFiles(this.root) };
  }

  // Inhalt eines Unterordners für den Dateibaum (rel '' = Arbeitsordner), Ordner zuerst
  list(rel = '') {
    if (!this.root) return { ok: false, entries: [] };
    const dir = rel ? this.resolve(rel) : fs.realpathSync.native(this.root);
    if (!dir) return { ok: false, entries: [] };
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return { ok: false, entries: [] };
    }
    const out = [];
    for (const entry of entries) {
      if (out.length >= MAX_ENTRIES || HIDDEN.test(entry.name)) continue;
      const full = path.join(dir, entry.name);
      try {
        const stat = fs.statSync(full);
        out.push({ path: path.join(rel, entry.name), name: entry.name, dir: stat.isDirectory(), size: stat.size, modified: stat.mtimeMs });
      } catch {
        // gerade gelöscht oder gesperrt
      }
    }
    out.sort((a, b) => (b.dir - a.dir) || a.name.localeCompare(b.name, 'de'));
    return { ok: true, entries: out, more: entries.length > MAX_ENTRIES };
  }

  // Absoluter Pfad zu einer Datei im Arbeitsordner, oder null, wenn sie ausserhalb liegt.
  resolve(rel) {
    if (!this.root) return null;
    const full = path.resolve(this.root, rel);
    let real;
    try {
      real = fs.realpathSync.native(full);
    } catch {
      return null;
    }
    const root = fs.realpathSync.native(this.root).toLowerCase();
    const r = real.toLowerCase();
    return r === root || r.startsWith(root + path.sep) ? real : null;
  }
}

module.exports = { Workspace, recentFiles, ASK_DIR };
