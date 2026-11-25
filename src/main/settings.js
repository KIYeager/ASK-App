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
  betaUpdates: false,
};

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

function addRecent(folder) {
  const s = load();
  const key = folder.toLowerCase();
  s.recent = [folder, ...s.recent.filter((f) => f.toLowerCase() !== key)].slice(0, MAX_RECENT);
  s.folder = folder;
  save();
}

function removeRecent(folder) {
  const s = load();
  s.recent = s.recent.filter((f) => f.toLowerCase() !== folder.toLowerCase());
  if (s.folder.toLowerCase() === folder.toLowerCase()) s.folder = '';
  save();
}

module.exports = { load, get, set, addRecent, removeRecent, appConfig };
