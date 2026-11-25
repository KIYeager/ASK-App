// Entfernt die Integritätsmarke «Low» wieder von einem Ordner, der aus der Projektliste genommen wird.
// Gegenstück zu ensure_low_label in python/ask_terminal/winproc.py (setzt (OI)(CI)L beim Terminalstart).
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { execFile } = require('node:child_process');

const TIMEOUT_MS = 10 * 60 * 1000;
const w = path.win32;

const norm = (p) => w.resolve(String(p)).replace(/[\\/]+$/, '').toLowerCase();
const isRoot = (p) => w.parse(w.resolve(p)).root.replace(/[\\/]+$/, '').toLowerCase() === norm(p);
// a liegt in b (oder ist b); «C:\a\bc» liegt nicht in «C:\a\b»
const within = (a, b) => a === b || a.startsWith(`${b}\\`);

function defaultRun(file, args, options) {
  return new Promise((resolve, reject) => {
    execFile(file, args, options, (err, stdout, stderr) => (err ? reject(Object.assign(err, { stderr })) : resolve({ stdout, stderr })));
  });
}

// Setzt die Marke eines Ordners samt allem darin auf «Medium» (Standard) zurück. /T ist nötig, weil Dateien,
// die das Low-Terminal angelegt hat, eine eigene Marke tragen und sie nicht vom Ordner erben.
// Gibt { skipped: true, reason } zurück, wenn nichts zu tun ist, sonst { skipped: false }.
async function restoreLabel(folder, { otherFolders = [], run = defaultRun, platform = process.platform, exists = fs.existsSync, home = os.homedir() } = {}) {
  if (platform !== 'win32') return { skipped: true, reason: 'nicht Windows' };
  if (!folder) return { skipped: true, reason: 'kein Ordner' };
  if (!exists(folder)) return { skipped: true, reason: 'Ordner existiert nicht mehr' };
  const f = norm(folder);
  if (isRoot(folder)) return { skipped: true, reason: 'Laufwerkswurzel' };
  if (home && f === norm(home)) return { skipped: true, reason: 'Benutzerordner' };
  for (const other of otherFolders) {
    if (!other) continue;
    const o = norm(other);
    // Mit /T bekämen Dateien eines anderen Projekts eine eigene Medium-Marke.
    if (within(f, o) || within(o, f)) return { skipped: true, reason: `liegt im oder enthält das Projekt ${other}` };
  }
  const icacls = w.join(process.env.SystemRoot || 'C:\\Windows', 'System32', 'icacls.exe');
  await run(icacls, [w.resolve(folder), '/setintegritylevel', '(OI)(CI)M', '/T', '/C', '/Q'], { windowsHide: true, timeout: TIMEOUT_MS, shell: false });
  return { skipped: false };
}

module.exports = { restoreLabel };
