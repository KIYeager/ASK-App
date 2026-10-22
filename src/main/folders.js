// Prüft, ob ein Ordner als Arbeitsordner taugt.
// Gesperrt sind OneDrive und SharePoint (Löschungen würden sofort synchronisiert), Laufwerkswurzeln,
// das Benutzerprofil selbst, AppData, Windows und die Programmordner.
const fs = require('node:fs');
const path = require('node:path').win32;
const { execFileSync } = require('node:child_process');

function norm(p) {
  return path.resolve(p).replace(/[\\/]+$/, '').toLowerCase();
}

function isInside(child, parent) {
  const c = norm(child);
  const p = norm(parent);
  return c === p || c.startsWith(p + '\\');
}

// Synchronisierte OneDrive- und SharePoint-Ordner aus der Registry (auch Bibliotheken,
// deren Pfad nicht «OneDrive» enthält).
function readSyncRoots() {
  const roots = [];
  const queries = [
    ['HKCU\\Software\\SyncEngines\\Providers\\OneDrive', '/s', '/v', 'MountPoint'],
    ['HKCU\\Software\\Microsoft\\OneDrive\\Accounts', '/s', '/v', 'UserFolder'],
  ];
  for (const args of queries) {
    try {
      const out = execFileSync('reg.exe', ['query', ...args], { encoding: 'utf8', windowsHide: true, timeout: 5000 });
      for (const line of out.split(/\r?\n/)) {
        const m = line.match(/^\s+\S+\s+REG_(?:EXPAND_)?SZ\s+(.+)$/);
        if (m && /^[a-z]:\\/i.test(m[1].trim())) roots.push(m[1].trim());
      }
    } catch {
      // Schlüssel fehlt: kein OneDrive eingerichtet.
    }
  }
  try {
    const out = execFileSync('reg.exe', ['query', 'HKCU\\Software\\Microsoft\\OneDrive\\Accounts', '/s'],
      { encoding: 'utf8', windowsHide: true, timeout: 5000 });
    // Unter ...\Tenants\<Name> sind die Werte selbst Ordnerpfade synchronisierter Bibliotheken.
    let inTenants = false;
    for (const line of out.split(/\r?\n/)) {
      if (/^HKEY_/.test(line)) inTenants = /\\Tenants\\/i.test(line);
      const m = inTenants && line.match(/^\s+([a-z]:\\.+?)\s+REG_\w+/i);
      if (m) roots.push(m[1].trim());
    }
  } catch {
    // ignorieren
  }
  return [...new Set(roots)];
}

function defaultContext() {
  const env = process.env;
  return {
    env,
    syncRoots: readSyncRoots(),
    realpath: (p) => fs.realpathSync.native(p),
    stat: (p) => fs.statSync(p),
  };
}

const REASON = {
  missing: 'Der Ordner existiert nicht.',
  notDir: 'Das ist kein Ordner.',
  network: 'Netzlaufwerke und Netzwerkpfade werden nicht unterstützt. Bitte einen Ordner auf diesem Laptop wählen.',
  root: 'Ein ganzes Laufwerk kann nicht gewählt werden. Bitte einen eigenen Projektordner anlegen.',
  onedrive: 'Ordner in OneDrive oder SharePoint sind gesperrt: Änderungen und Löschungen würden sofort in die Cloud synchronisiert. Bitte einen Ordner ausserhalb von OneDrive wählen, zum Beispiel unter C:\\Users\\<Name>\\ASK-Projekte.',
  containsSync: 'Der Ordner enthält einen OneDrive- oder SharePoint-Ordner. Bitte einen Unterordner ohne Cloud-Synchronisation wählen.',
  profile: 'Das ganze Benutzerprofil kann nicht gewählt werden. Bitte darin einen eigenen Projektordner anlegen.',
  usersRoot: 'Der Ordner mit allen Benutzerprofilen kann nicht gewählt werden.',
  appdata: 'Ordner in AppData sind gesperrt, dort liegen Einstellungen anderer Programme.',
  system: 'Systemordner (Windows, Programme, ProgramData) sind gesperrt.',
};

function checkFolder(input, ctx = defaultContext()) {
  const env = ctx.env;
  if (!input) return { ok: false, reason: REASON.missing };
  if (/^\\\\/.test(input)) return { ok: false, reason: REASON.network };

  let real;
  try {
    if (!ctx.stat(input).isDirectory()) return { ok: false, reason: REASON.notDir };
    real = ctx.realpath(input);
  } catch {
    return { ok: false, reason: REASON.missing };
  }
  if (/^\\\\/.test(real)) return { ok: false, reason: REASON.network };
  if (/^[a-z]:\\?$/i.test(real)) return { ok: false, reason: REASON.root };

  const profile = env.USERPROFILE;
  if (profile) {
    if (norm(real) === norm(profile)) return { ok: false, reason: REASON.profile };
    if (isInside(profile, real)) return { ok: false, reason: REASON.usersRoot };
  }

  const oneDriveEnv = ['OneDrive', 'OneDriveCommercial', 'OneDriveConsumer'].map((k) => env[k]).filter(Boolean);
  const syncRoots = [...oneDriveEnv, ...(ctx.syncRoots || [])];
  if (syncRoots.some((r) => isInside(real, r))) return { ok: false, reason: REASON.onedrive };
  if (real.split('\\').some((part) => /onedrive/i.test(part))) return { ok: false, reason: REASON.onedrive };
  if (syncRoots.some((r) => isInside(r, real))) return { ok: false, reason: REASON.containsSync };

  if (profile && isInside(real, path.join(profile, 'AppData'))) return { ok: false, reason: REASON.appdata };
  for (const key of ['APPDATA', 'LOCALAPPDATA']) {
    if (env[key] && isInside(real, env[key])) return { ok: false, reason: REASON.appdata };
  }

  const systemDirs = ['SystemRoot', 'windir', 'ProgramFiles', 'ProgramFiles(x86)', 'ProgramW6432', 'ProgramData']
    .map((k) => env[k]).filter(Boolean);
  for (const dir of systemDirs) {
    if (isInside(real, dir) || isInside(dir, real)) return { ok: false, reason: REASON.system };
  }
  return { ok: true, path: real };
}

module.exports = { checkFolder, readSyncRoots, isInside, REASON };
