// Lädt das NSIS-Plugin SpiderBanner für das kleine Fortschrittsfenster des Installers (ENTSCHEIDE E59, E63).
// Quelle wie bei electron-builder: nsis-resources-3.4.1 aus electron-builder-binaries, mit derselben Prüfsumme.
// Ziel: src-tauri/windows/plugins/x86-unicode/SpiderBanner.dll (nicht im Repository, siehe .gitignore);
// src-tauri/windows/hooks.nsh bindet es ein, wenn es da ist.
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const URL = 'https://github.com/electron-userland/electron-builder-binaries/releases/download/nsis-resources-3.4.1/nsis-resources-3.4.1.7z';
const SHA256 = '593a9a92ef958321293ac6a2ee61e64bf1bd543142a5bd6b3d310709cc924103';
const ENTRY = 'plugins/x86-unicode/SpiderBanner.dll';
const target = path.join(__dirname, '..', 'src-tauri', 'windows', 'plugins', 'x86-unicode', 'SpiderBanner.dll');

async function ensure(cacheDir) {
  if (fs.existsSync(target)) return target;
  fs.mkdirSync(cacheDir, { recursive: true });
  const archive = path.join(cacheDir, 'nsis-resources-3.4.1.7z');
  if (!fs.existsSync(archive)) {
    const res = await fetch(URL);
    if (!res.ok) throw new Error(`nsis-resources: HTTP ${res.status}`);
    fs.writeFileSync(archive, Buffer.from(await res.arrayBuffer()));
  }
  const sum = crypto.createHash('sha256').update(fs.readFileSync(archive)).digest('hex');
  if (sum !== SHA256) {
    fs.rmSync(archive, { force: true });
    throw new Error(`nsis-resources: Prüfsumme stimmt nicht (${sum})`);
  }
  // tar von Windows (bsdtar) liest 7z
  const out = path.join(cacheDir, 'nsis-resources');
  fs.mkdirSync(out, { recursive: true });
  const r = spawnSync('tar', ['-xf', archive, '-C', out, ENTRY], { stdio: 'inherit' });
  if (r.status !== 0) throw new Error('nsis-resources: Entpacken mit tar fehlgeschlagen');
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.copyFileSync(path.join(out, ENTRY), target);
  return target;
}

module.exports = { ensure };

if (require.main === module) {
  const { buildDir } = require('./build-dir');
  ensure(path.join(buildDir(), 'cache')).then((p) => console.log(p), (e) => { console.error(e.message); process.exit(1); });
}
