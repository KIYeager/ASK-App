// Arbeitsordner: Grundstruktur anlegen, Skills kopieren, Änderungen beobachten, Dateien auflisten.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { EventEmitter } = require('node:events');
const paths = require('./paths');

const ASK_DIR = '.ask';
const OUTPUT_DIR = 'ausgabe';
const MAX_ENTRIES = 300;

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

// Akzeptiert auch abweichende Schreibweisen, die ein Modell gern benutzt.
function normalizeTasks(raw) {
  if (!raw || typeof raw !== 'object') return null;
  const list = Array.isArray(raw) ? raw : raw.aufgaben || raw.tasks || raw.todos || raw.items || [];
  const tasks = list.map((t) => {
    if (typeof t === 'string') return { titel: t, status: 'offen' };
    const titel = t.titel || t.title || t.text || t.name || t.beschreibung || t.description || '';
    let status = String(t.status || t.state || '').toLowerCase();
    if (t.done === true || t.erledigt === true || t.completed === true) status = 'erledigt';
    if (['done', 'completed', 'fertig', 'ok', 'erledigt'].includes(status)) status = 'erledigt';
    else if (['in_progress', 'in-progress', 'in arbeit', 'in_arbeit', 'läuft', 'laeuft', 'running', 'aktiv']
      .includes(status)) status = 'in_arbeit';
    else status = 'offen';
    return { titel: String(titel), status };
  }).filter((t) => t.titel);
  return { auftrag: Array.isArray(raw) ? '' : String(raw.auftrag || raw.goal || raw.titel || raw.title || ''), aufgaben: tasks };
}

class Workspace extends EventEmitter {
  constructor() {
    super();
    this.root = '';
    this.watcher = null;
    this.timer = null;
    this.pending = new Set();
  }

  open(folder) {
    this.close();
    this.root = folder;
    fs.mkdirSync(path.join(folder, ASK_DIR), { recursive: true });
    fs.mkdirSync(path.join(folder, OUTPUT_DIR), { recursive: true });
    fs.rmSync(path.join(folder, ASK_DIR, 'auftraege'), { recursive: true, force: true });
    this.syncSkills();
    try {
      this.watcher = fs.watch(folder, { recursive: true }, (event, name) => this.onChange(name || ''));
      this.watcher.on('error', () => {});
    } catch {
      this.watcher = null;
    }
    this.emit('files', this.files());
    this.emit('tasks', this.tasks());
  }

  close() {
    if (this.watcher) this.watcher.close();
    this.watcher = null;
    clearTimeout(this.timer);
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
    if (name.startsWith(path.join(ASK_DIR, 'skills'))) return;
    if (/^\.ask[\\/]auftraege[\\/][0-9a-f]{6,32}\.json$/i.test(name)) {
      const file = path.join(this.root, name);
      if (fs.existsSync(file)) this.emit('auftrag', file);
      return;
    }
    if (name.startsWith(path.join(ASK_DIR, 'auftraege')) || name.startsWith(path.join(ASK_DIR, 'pdf'))) return;
    this.pending.add(name.toLowerCase() === path.join(ASK_DIR, 'tasks.json').toLowerCase() ? 'tasks' : 'files');
    clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      const kinds = this.pending;
      this.pending = new Set();
      if (!this.root) return;
      if (kinds.has('tasks')) this.emit('tasks', this.tasks());
      if (kinds.has('files')) this.emit('files', this.files());
    }, 250);
  }

  files() {
    if (!this.root) return { top: [], output: [] };
    const top = [];
    listRecursive(this.root, this.root, 0, top);
    const output = [];
    listRecursive(this.root, path.join(this.root, OUTPUT_DIR), 3, output);
    output.sort((a, b) => b.modified - a.modified);
    const sortTop = (a, b) => (b.dir - a.dir) || a.name.localeCompare(b.name, 'de');
    return { top: top.sort(sortTop), output: output.filter((e) => !e.dir) };
  }

  tasks() {
    if (!this.root) return null;
    try {
      const text = fs.readFileSync(path.join(this.root, ASK_DIR, 'tasks.json'), 'utf8');
      return normalizeTasks(JSON.parse(text.replace(/^﻿/, '')));
    } catch (err) {
      return err.code === 'ENOENT' ? null : { fehler: 'tasks.json ist gerade nicht lesbar', aufgaben: [] };
    }
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

module.exports = { Workspace, normalizeTasks, ASK_DIR, OUTPUT_DIR };
