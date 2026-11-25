// Windows-Installer für ASK: NSIS, Installation pro Nutzer (ohne Admin-Rechte).
// Die einzelne EXE ohne Installation baut scripts/build-portable.js aus win-unpacked (ENTSCHEIDE E18).
// Das mitgelieferte Python baut scripts/build-python.ps1 nach ASK_BUILD_DIR\python, scripts/pack-python.js
// packt es nach ASK_BUILD_DIR\python-pack (ein Archiv statt 12'000 Dateien, ENTSCHEIDE E39).
const path = require('node:path');

const buildDir = require('./src/main/build-dir').buildDir();
// Release-Repository für Updates (ENTSCHEIDE E33), dieselbe Angabe liest die App zur Laufzeit.
const { updates } = require('./resources/app-config.json');

module.exports = {
  appId: 'ch.tfbern.ask',
  productName: 'ASK',
  copyright: 'TF Bern',
  directories: { output: path.join(buildDir, 'dist') },
  files: ['src/**/*', 'package.json'],
  extraResources: [
    { from: path.join(buildDir, 'python-pack'), to: 'python-pack' },
    { from: 'python/ask_terminal', to: 'ask_terminal', filter: ['*.py'] },
    { from: 'resources/skills', to: 'skills' },
    { from: 'resources/anweisungen', to: 'anweisungen' },
    { from: 'resources/app-config.json', to: 'app-config.json' },
    { from: 'resources/icon.ico', to: 'icon.ico' },
  ],
  asar: true,
  // Schreibt app-update.yml in die App und latest.yml neben den Installer. Hochgeladen wird nicht
  // von electron-builder, sondern im Workflow .github/workflows/release.yml.
  publish: [{ provider: 'github', owner: updates.owner, repo: updates.repo }],
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
    // Beim Deinstallieren (nicht bei Updates) das entpackte Python in %LOCALAPPDATA% entfernen
    include: 'scripts/installer.nsh',
  },
};
