// Windows-Installer für ASK: NSIS, Installation pro Nutzer (ohne Admin-Rechte).
// Das mitgelieferte Python baut scripts/build-python.ps1 nach ASK_BUILD_DIR\python.
const path = require('node:path');
const os = require('node:os');

const buildDir = process.env.ASK_BUILD_DIR || path.join(process.env.LOCALAPPDATA || os.homedir(), 'ASK-Desktop-Build');

module.exports = {
  appId: 'ch.tfbern.ask',
  productName: 'ASK',
  copyright: 'TF Bern',
  directories: { output: path.join(buildDir, 'dist') },
  files: ['src/**/*', 'package.json'],
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
