// Hält den versteckten Ordner .ask klein: Hilfsskripte, Zwischendateien und Anhänge
// (.ask/skripte/<Chat>, .ask/arbeit/<Chat>, .ask/anhaenge/<Chat>) werden nach einiger Zeit gelöscht.
// Chats, Anleitungen und Aufgabenliste bleiben. Die Sicherung räumt snapshot.js selbst auf.
const fs = require('node:fs');
const path = require('node:path');
const { execFile } = require('node:child_process');

const KEEP_DAYS = 14;
const WORK_DIRS = ['skripte', 'arbeit', 'anhaenge'];

// Löscht Dateien, die seit KEEP_DAYS Tagen unverändert sind, und danach leere Unterordner.
// Liefert die Anzahl gelöschter Dateien.
function cleanupWorkDirs(root, now = Date.now()) {
  const limit = now - KEEP_DAYS * 24 * 60 * 60 * 1000;
  let removed = 0;
  const walk = (dir, top) => {
    let entries;
    try {
      entries = fs.readdirSync(dir, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      const full = path.join(dir, entry.name);
      try {
        if (entry.isDirectory()) walk(full, false);
        else if (fs.statSync(full).mtimeMs < limit) {
          fs.rmSync(full, { force: true });
          removed++;
        }
      } catch {
        // gesperrt oder verschwunden: beim nächsten Mal
      }
    }
    if (!top) {
      try {
        if (fs.readdirSync(dir).length === 0) fs.rmdirSync(dir);
      } catch {
        // nicht leer oder gesperrt
      }
    }
  };
  for (const sub of WORK_DIRS) walk(path.join(root, '.ask', sub), true);
  return removed;
}

// Markiert .ask als versteckt (wie Git den Ordner .git), damit Lernende ihn im Explorer nicht sehen.
function hideFolder(dir) {
  if (process.platform !== 'win32') return;
  execFile('attrib', ['+h', dir], { windowsHide: true }, () => {});
}

module.exports = { cleanupWorkDirs, hideFolder, KEEP_DAYS, WORK_DIRS };
