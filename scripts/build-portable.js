// Baut die einzelne EXE ASK-<Version>.exe (ENTSCHEIDE E18): electron-builder erzeugt den
// App-Ordner win-unpacked, makensis packt ihn mit dem Starter scripts/starter.nsi in eine EXE.
// makensis läuft hier selbst, weil electron-builder es nach festen 4 Minuten abbricht.
const fs = require('node:fs');
const path = require('node:path');
const { spawn } = require('node:child_process');
const { build, Platform, Arch } = require('electron-builder');
const { getMakeNsisPath } = require('app-builder-lib/out/toolsets/windows');
const config = require('../electron-builder.config');
const { version } = require('../package.json');

const repo = path.join(__dirname, '..');

async function main() {
  await build({ targets: Platform.WINDOWS.createTarget('dir', Arch.x64), config, publish: 'never' });
  const appDir = path.join(config.directories.output, 'win-unpacked');
  const outFile = path.join(config.directories.output, `ASK-${version}.exe`);
  // Eigener Ordnername pro Build: Ein neuer Build entpackt sich neu, auch bei gleicher Version.
  const buildId = `${version}-${Date.now().toString(36)}`;
  const defines = {
    BUILD_ID: buildId,
    VERSION: version,
    APP_DIR: appDir,
    OUT_FILE: outFile,
    SPLASH: path.join(repo, 'resources', 'splash.bmp'),
    ICON: path.join(repo, 'resources', 'icon.ico'),
  };
  const makensis = await getMakeNsisPath();
  const args = ['-V2', '-INPUTCHARSET', 'UTF8', ...Object.entries(defines).map(([k, v]) => `-D${k}=${v}`),
    path.join(__dirname, 'starter.nsi')];
  console.log(`Packe ${outFile} (Stand ${buildId}), das dauert einige Minuten ...`);
  const started = Date.now();
  await new Promise((resolve, reject) => {
    const p = spawn(makensis.path, args, { env: { ...process.env, ...makensis.env }, stdio: 'inherit' });
    p.on('error', reject);
    p.on('exit', (code) => (code === 0 ? resolve() : reject(new Error(`makensis endete mit Code ${code}`))));
  });
  const mb = Math.round(fs.statSync(outFile).size / 1024 / 1024);
  console.log(`Fertig nach ${Math.round((Date.now() - started) / 1000)} s: ${outFile} (${mb} MB)`);
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
