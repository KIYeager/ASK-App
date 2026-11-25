// Einstellungen der App als JSON im Benutzerordner von Electron (%APPDATA%\ASK).
const fs = require('node:fs');
const path = require('node:path');
const { app } = require('electron');
const paths = require('./paths');

const MAX_RECENT = 8;

const defaults = {
  askUrl: '',
  folder: '',
  recent: [],
  port: 0,
  warningAccepted: false,
  sidebarOpen: true,
  chatbarOpen: true,
  model: '',
  webSearch: false,
  thinking: 'standard',
  approvals: 'heikel', // Rückfrage vor heiklen Schritten: aus | heikel | alle (approvals.js)
  theme: 'system', // system | hell | dunkel
  betaUpdates: false,
};

// Farben der Projekte in der Seitenleiste (TF Blau zuerst, danach Sekundärfarben des CD)
const PROJECT_COLORS = ['#0082B4', '#F28855', '#4EB68E', '#EE716D', '#60C5E2', '#A2CE8D', '#A5ACAE'];


let data = null;

function file() {
  return path.join(app.getPath('userData'), 'einstellungen.json');
}

function appConfig() {
  try {
    return JSON.parse(fs.readFileSync(paths.appConfigFile(), 'utf8'));
  } catch {
    return {};
  }
}

function load() {
  if (data) return data;
  let stored = {};
  try {
    stored = JSON.parse(fs.readFileSync(file(), 'utf8'));
  } catch {
    stored = {};
  }
  data = { ...defaults, ...stored };
  if (!data.askUrl && appConfig().askUrl) data.askUrl = appConfig().askUrl;
  if (!Array.isArray(data.recent)) data.recent = [];
  // Projekte (ein Projekt = ein Arbeitsordner). Ältere Einstellungen kennen nur «recent».
  if (!Array.isArray(data.projects)) {
    data.projects = data.recent.map((folder, i) => ({ folder, name: path.win32.basename(folder) || folder, color: PROJECT_COLORS[i % PROJECT_COLORS.length] }));
  }
  return data;
}

function save() {
  fs.mkdirSync(path.dirname(file()), { recursive: true });
  const text = JSON.stringify(data, null, 2);
  const tmp = file() + '.tmp';
  try {
    fs.writeFileSync(tmp, text);
    fs.renameSync(tmp, file());
  } catch {
    // Umbenennen kann scheitern (Virenscanner, umgeleitete Ordner): dann direkt schreiben.
    fs.rmSync(tmp, { force: true });
    fs.writeFileSync(file(), text);
  }
}

function get(key) {
  return load()[key];
}

function set(values) {
  Object.assign(load(), values);
  save();
}

const same = (a, b) => String(a || '').toLowerCase() === String(b || '').toLowerCase();

function findProject(folder) {
  return load().projects.find((p) => same(p.folder, folder)) || null;
}

// Ordner als zuletzt verwendet und als aktuellen Ordner merken; ohne Projekt wird eines angelegt.
function addRecent(folder, name = '') {
  const s = load();
  s.recent = [folder, ...s.recent.filter((f) => !same(f, folder))].slice(0, MAX_RECENT);
  s.folder = folder;
  const project = findProject(folder);
  if (!project) {
    const used = new Set(s.projects.map((p) => p.color));
    const color = PROJECT_COLORS.find((c) => !used.has(c)) || PROJECT_COLORS[s.projects.length % PROJECT_COLORS.length];
    s.projects = [{ folder, name: String(name || path.win32.basename(folder) || folder).slice(0, 80), color }, ...s.projects];
  } else if (name) {
    project.name = String(name).slice(0, 80);
  }
  save();
}

function removeRecent(folder) {
  const s = load();
  s.recent = s.recent.filter((f) => !same(f, folder));
  s.projects = s.projects.filter((p) => !same(p.folder, folder));
  if (same(s.folder, folder)) s.folder = '';
  save();
}

// Name oder Farbe eines Projekts ändern
function updateProject(folder, patch = {}) {
  const project = findProject(folder);
  if (!project) return false;
  if (typeof patch.name === 'string' && patch.name.trim()) project.name = patch.name.trim().slice(0, 80);
  if (typeof patch.color === 'string' && /^#[0-9a-f]{6}$/i.test(patch.color)) project.color = patch.color;
  save();
  return true;
}

module.exports = { load, get, set, addRecent, removeRecent, updateProject, findProject, appConfig, PROJECT_COLORS };
