// Windows-Installer für ASK: NSIS, Installation pro Nutzer (ohne Admin-Rechte).
// Die einzelne EXE ohne Installation baut scripts/build-portable.js aus win-unpacked (ENTSCHEIDE E18).
// Das mitgelieferte Python baut scripts/build-python.ps1 nach ASK_BUILD_DIR\python.
const path = require('node:path');

const buildDir = require('./src/main/build-dir').buildDir();
let updateUrl = '';
try { updateUrl = require('./resources/app-config.json').updateUrl || ''; } catch { /* ohne Updates */ }

module.exports = {
  appId: 'ch.tfbern.ask',
  productName: 'ASK',
  copyright: 'TF Bern',
  directories: { output: path.join(buildDir, 'dist') },
  // Oberfläche nur gebaut (src/renderer/dist, «npm run build:ui»), ohne Quellen
  files: ['src/main/**/*', 'src/preload/**/*', 'src/vendor/**/*', 'src/renderer/dist/**/*', 'package.json'],
  // Updates (ENTSCHEIDE E34): mit updateUrl in resources/app-config.json entstehen latest.yml und
  // die Angaben für electron-updater. Die Dateien aus dist dann auf diesen Server legen.
  publish: updateUrl ? [{ provider: 'generic', url: updateUrl }] : null,
  extraResources: [
    { from: path.join(buildDir, 'python'), to: 'python' },
    { from: 'python/ask_terminal', to: 'ask_terminal', filter: ['*.py'] },
    { from: 'resources/skills', to: 'skills' },
    { from: 'resources/anweisungen', to: 'anweisungen' },
    { from: 'resources/app-config.json', to: 'app-config.json' },
    { from: 'resources/icon.ico', to: 'icon.ico' },
  ],
  asar: true,
  electronLanguages: ['de', 'en-US'],
  win: {
    target: [{ target: 'nsis', arch: ['x64'] }],
    icon: 'resources/icon.ico',
    executableName: 'ASK',
  },
  nsis: {
    oneClick: true,
    perMachine: false,
    allowElevation: false,
    createDesktopShortcut: true,
    createStartMenuShortcut: true,
    shortcutName: 'ASK',
    uninstallDisplayName: 'ASK Desktop',
    artifactName: 'ASK-Setup-${version}.exe',
    deleteAppDataOnUninstall: false,
  },
};
