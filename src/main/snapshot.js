// Sicherung des Arbeitsordners vor jedem Auftrag (ENTSCHEIDE E24). Das Modell ändert Dateien direkt;
// «Rückgängig» setzt den Ordner auf den Stand vor einem Auftrag zurück. Jede Dateifassung liegt nur
// einmal in .ask/sicherung/objekte (Name = SHA-1 des Inhalts), ein Stand ist eine Liste
// Pfad → Prüfsumme in .ask/sicherung/staende. Unveränderte Dateien (gleiche Grösse und Zeit) werden
// nicht neu gelesen. Erfasst auch Änderungen durch Skripte, nicht nur durch write_file.
const fs = require('node:fs');
const fsp = require('node:fs/promises');
const path = require('node:path');
const crypto = require('node:crypto');

const STORE = path.join('.ask', 'sicherung');
const SKIP = /^(\.ask|\.git|node_modules|__pycache__|\.venv)(\/|$)|(^|\/)~\$/i; // ~$: Sperrdateien von Office
const MAX_FILE = 100 * 1024 * 1024; // grössere Dateien werden nicht gesichert
const MAX_FILES = 5000;
const KEEP_DAYS = 14;

const sha1 = (data) => crypto.createHash('sha1').update(data).digest('hex');
const newId = () => `${Date.now().toString(36)}-${crypto.randomBytes(3).toString('hex')}`;

// Dateien im Ordner: { 'pfad/datei': { size, mtime } } mit / als Trenner
// Ordner, in denen gerade gesichert wird: prune wartet dann (workspace.js)
const busyRoots = new Map();
const busy = (root) => (busyRoots.get(path.resolve(root).toLowerCase()) || 0) > 0;
function mark(root, delta) {
  const key = path.resolve(root).toLowerCase();
  const n = (busyRoots.get(key) || 0) + delta;
  if (n > 0) busyRoots.set(key, n); else busyRoots.delete(key);
}

// limit: Sichern bricht bei zu vielen Dateien ab; Vergleichen und Zurücksetzen nicht, sonst ginge
// «Rückgängig» nicht mehr, sobald ein Auftrag viele Dateien erzeugt hat.
async function listFiles(root, limit = MAX_FILES) {
  const out = {};
  let count = 0;
  const walk = async (dir) => {
    let entries;
    try {
      entries = await fsp.readdir(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      const rel = path.relative(root, full).split(path.sep).join('/');
      if (SKIP.test(rel)) continue;
      if (entry.isDirectory()) await walk(full);
      else if (entry.isFile()) {
        if (++count > limit) throw new Error(`Mehr als ${MAX_FILES} Dateien im Ordner, die App sichert ihn nicht.`);
        try {
          const st = await fsp.stat(full);
          out[rel] = { size: st.size, mtime: st.mtimeMs };
        } catch {
          // gerade gelöscht
        }
      }
    }
  };
  await walk(root);
  return out;
}

class Snapshots {
  constructor(root) {
    this.root = root;
    this.dir = path.join(root, STORE);
    this.last = null; // letzter Stand, für Prüfsummen unveränderter Dateien
  }

  objectPath(hash) {
    return path.join(this.dir, 'objekte', hash.slice(0, 2), hash);
  }

  standPath(id) {
    if (!/^[\w-]+$/.test(String(id))) throw new Error('Ungültiger Stand');
    return path.join(this.dir, 'staende', `${id}.json`);
  }

  read(id) {
    return JSON.parse(fs.readFileSync(this.standPath(id), 'utf8'));
  }

  newest() {
    try {
      const dir = path.join(this.dir, 'staende');
      const latest = fs.readdirSync(dir).map((n) => ({ n, t: fs.statSync(path.join(dir, n)).mtimeMs })).sort((a, b) => b.t - a.t)[0];
      return latest ? JSON.parse(fs.readFileSync(path.join(dir, latest.n), 'utf8')) : null;
    } catch {
      return null;
    }
  }

  // Prüfsumme einer Datei; known: Eintrag eines früheren Stands, gilt bei gleicher Grösse und Zeit.
  async hashOf(rel, info, known, store) {
    if (known && known.size === info.size && known.mtime === info.mtime && fs.existsSync(this.objectPath(known.hash))) return known.hash;
    const data = await fsp.readFile(path.join(this.root, rel));
    const hash = sha1(data);
    if (store) {
      const target = this.objectPath(hash);
      if (!fs.existsSync(target)) {
        await fsp.mkdir(path.dirname(target), { recursive: true });
        const tmp = `${target}.${process.pid}.tmp`;
        await fsp.writeFile(tmp, data);
        await fsp.rename(tmp, target);
      }
    }
    return hash;
  }

  // Stand des Ordners sichern. Liefert { id, count, skipped } (skipped: zu gross oder gesperrt).
  async take(label = '') {
    mark(this.root, 1);
    try {
      return await this.takeNow(label);
    } finally {
      mark(this.root, -1);
    }
  }

  async takeNow(label) {
    const current = await listFiles(this.root);
    if (!this.last) this.last = this.newest(); // nach einem Neustart: Prüfsummen des letzten Stands
    const before = this.last?.files || {};
    const files = {};
    const skipped = [];
    for (const [rel, info] of Object.entries(current)) {
      if (info.size > MAX_FILE) { skipped.push(rel); continue; }
      try {
        files[rel] = { ...info, hash: await this.hashOf(rel, info, before[rel], true) };
      } catch {
        skipped.push(rel); // gesperrt, z. B. von Office exklusiv geöffnet
      }
    }
    const stand = { id: newId(), time: Date.now(), label, files, skipped };
    await fsp.mkdir(path.dirname(this.standPath(stand.id)), { recursive: true });
    await fsp.writeFile(this.standPath(stand.id), JSON.stringify(stand));
    this.last = stand;
    return { id: stand.id, count: Object.keys(files).length, skipped };
  }

  // Was hat sich seit einem Stand geändert? { changed, added, removed } (Pfade)
  async diff(id) {
    const stand = this.read(id);
    const current = await listFiles(this.root, Infinity);
    const out = { changed: [], added: [], removed: [] };
    for (const [rel, info] of Object.entries(current)) {
      const old = stand.files[rel];
      if (!old) {
        if (!stand.skipped.includes(rel)) out.added.push(rel);
        continue;
      }
      if (old.size === info.size && old.mtime === info.mtime) continue;
      try {
        if (await this.hashOf(rel, info, null, false) !== old.hash) out.changed.push(rel);
      } catch {
        out.changed.push(rel); // nicht lesbar: als geändert melden, Wiederherstellen versucht es
      }
    }
    for (const rel of Object.keys(stand.files)) if (!current[rel]) out.removed.push(rel);
    return out;
  }

  // Ordner auf einen Stand zurücksetzen. Vorher wird der jetzige Stand gesichert.
  async restore(id) {
    const stand = this.read(id);
    const d = await this.diff(id);
    let safety = null;
    try {
      safety = await this.take('vor Rückgängig');
    } catch {
      // zu viele Dateien: ohne Zwischensicherung zurücksetzen
    }
    const failed = [];
    for (const rel of d.added) {
      try {
        await fsp.rm(path.join(this.root, rel), { force: true });
        await this.removeEmptyParents(rel);
      } catch {
        failed.push(rel);
      }
    }
    for (const rel of [...d.changed, ...d.removed]) {
      try {
        const target = path.join(this.root, rel);
        await fsp.mkdir(path.dirname(target), { recursive: true });
        await fsp.copyFile(this.objectPath(stand.files[rel].hash), target);
        const t = new Date(stand.files[rel].mtime);
        await fsp.utimes(target, t, t);
      } catch {
        failed.push(rel); // z. B. in Word geöffnet
      }
    }
    this.last = null;
    return { ...d, failed, safety: safety?.id || null };
  }

  async removeEmptyParents(rel) {
    let dir = path.dirname(path.join(this.root, rel));
    while (dir.length > this.root.length && path.relative(this.root, dir) && !path.relative(this.root, dir).startsWith('..')) {
      if ((await fsp.readdir(dir)).length) return;
      await fsp.rmdir(dir);
      dir = path.dirname(dir);
    }
  }

  // Stände älter als KEEP_DAYS löschen, danach Dateifassungen, die kein Stand mehr braucht.
  prune(now = Date.now()) {
    const limit = now - KEEP_DAYS * 24 * 60 * 60 * 1000;
    const standDir = path.join(this.dir, 'staende');
    const used = new Set();
    let names = [];
    try { names = fs.readdirSync(standDir); } catch { return 0; }
    for (const name of names) {
      const file = path.join(standDir, name);
      try {
        const stand = JSON.parse(fs.readFileSync(file, 'utf8'));
        if (stand.time < limit) fs.rmSync(file, { force: true });
        else for (const f of Object.values(stand.files)) used.add(f.hash);
      } catch {
        fs.rmSync(file, { force: true }); // unlesbar: weg
      }
    }
    let removed = 0;
    const objDir = path.join(this.dir, 'objekte');
    for (const sub of fs.existsSync(objDir) ? fs.readdirSync(objDir) : []) {
      for (const name of fs.readdirSync(path.join(objDir, sub))) {
        if (!used.has(name)) {
          fs.rmSync(path.join(objDir, sub, name), { force: true });
          removed++;
        }
      }
      if (!fs.readdirSync(path.join(objDir, sub)).length) fs.rmdirSync(path.join(objDir, sub));
    }
    if (this.last && this.last.time < limit) this.last = null;
    return removed;
  }
}

module.exports = { Snapshots, listFiles, busy, STORE, MAX_FILE, KEEP_DAYS };
