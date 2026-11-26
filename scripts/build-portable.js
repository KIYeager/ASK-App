// Baut die einzelne EXE ASK-<Version>.exe (ENTSCHEIDE E18): scripts/dist.js legt den App-Ordner
// <Build-Ordner>\dist\app an (ASK.exe mit Ressourcen), makensis packt ihn mit dem Starter scripts/starter.nsi
// in eine EXE. makensis ist das NSIS, das Tauri für den Installer geladen hat, sonst eines aus PATH.
// Die einzelne EXE braucht die WebView2-Laufzeit von Windows (auf Windows 10 und 11 vorhanden).
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { buildDir } = require('./build-dir');
const { version } = require('../package.json');

const repo = path.join(__dirname, '..');

// Windows verlangt für die Dateiversion vier Zahlen: 0.2.3 → 0.2.3.0, Beta 0.2.3-beta.1 → 0.2.3.1
function viVersion(v) {
  const m = /^(\d+)\.(\d+)\.(\d+)(?:-[a-z]+\.(\d+))?/i.exec(v);
  if (!m) throw new Error(`Version ${v} hat nicht die Form 1.2.3 oder 1.2.3-beta.4`);
  return `${m[1]}.${m[2]}.${m[3]}.${m[4] || 0}`;
}

function makensis() {
  const tauri = path.join(process.env.LOCALAPPDATA || '', 'tauri', 'NSIS', 'makensis.exe');
  return fs.existsSync(tauri) ? tauri : 'makensis';
}

async function main() {
  const dist = path.join(buildDir(), 'dist');
  const appDir = path.join(dist, 'app');
  if (!fs.existsSync(path.join(appDir, 'ASK.exe'))) throw new Error(`${appDir} fehlt. Zuerst «npm run dist».`);
  const outFile = path.join(dist, `ASK-${version}.exe`);
  // Eigener Ordnername pro Build: Ein neuer Build entpackt sich neu, auch bei gleicher Version.
  const buildId = `${version}-${Date.now().toString(36)}`;
  const defines = {
    BUILD_ID: buildId,
    VERSION: version,
    VI_VERSION: viVersion(version),
    APP_DIR: appDir,
    OUT_FILE: outFile,
    SPLASH: path.join(repo, 'resources', 'splash.bmp'),
    ICON: path.join(repo, 'resources', 'icon.ico'),
  };
  const args = ['-V2', '-INPUTCHARSET', 'UTF8', ...Object.entries(defines).map(([k, v]) => `-D${k}=${v}`),
    path.join(__dirname, 'starter.nsi')];
  console.log(`Packe ${outFile} (Stand ${buildId}), das dauert einige Minuten ...`);
  const started = Date.now();
  await new Promise((resolve, reject) => {
    const p = spawn(makensis(), args, { stdio: 'inherit' });
    p.on('error', reject);
    p.on('exit', (code) => (code === 0 ? resolve() : reject(new Error(`makensis endete mit Code ${code}`))));
  });
  const mb = Math.round(fs.statSync(outFile).size / 1024 / 1024);
  console.log(`Fertig nach ${Math.round((Date.now() - started) / 1000)} s: ${outFile} (${mb} MB)`);
}

if (require.main === module) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}

module.exports = { viVersion };
