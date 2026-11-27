// Update-Dateien für eine neue Version (ENTSCHEIDE E37, E60), aufgerufen vom Release-Workflow:
// - latest.json / beta.json für die Tauri-Fassung (tauri-plugin-updater): Version, Text, Signatur, Adresse.
//   Sie liegen im Release «updater» von ASK-Releases und werden bei jeder Version ersetzt. beta.json zeigt
//   immer auf die neueste Version (auch normale), latest.json nur auf normale.
// - latest.yml für die frühere Electron-Fassung (electron-updater): Damit finden installierte Electron-Apps
//   den neuen Installer und wechseln mit einem normalen Update zur Tauri-Fassung. Der Installer entfernt
//   dabei die alte App (src-tauri/windows/hooks.nsh); Einstellungen und Chats bleiben.
// Aufruf: node scripts/manifests.js <Ordner mit ASK-Setup-<Version>.exe und .sig> <Version> <Text-Datei> <Ausgabeordner>
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { updates } = require('../resources/app-config.json');

const isBeta = (v) => v.includes('-');

function tauriManifest({ version, notes, signature, url, date }) {
  return { version, notes, pub_date: date, platforms: { 'windows-x86_64': { signature, url } } };
}

function electronYml({ version, file, sha512, size, date }) {
  return [
    `version: ${version}`,
    'files:',
    `  - url: ${file}`,
    `    sha512: ${sha512}`,
    `    size: ${size}`,
    `path: ${file}`,
    `sha512: ${sha512}`,
    `releaseDate: '${date}'`,
    '',
  ].join('\n');
}

function build({ dir, version, notes, out, date = new Date().toISOString(), owner = updates.owner, repo = updates.repo }) {
  const file = `ASK-Setup-${version}.exe`;
  const setup = path.join(dir, file);
  const data = fs.readFileSync(setup);
  const signature = fs.readFileSync(`${setup}.sig`, 'utf8').trim();
  const url = `https://github.com/${owner}/${repo}/releases/download/v${version}/${file}`;
  fs.mkdirSync(out, { recursive: true });
  const manifest = JSON.stringify(tauriManifest({ version, notes, signature, url, date }), null, 2);
  const written = ['beta.json'];
  fs.writeFileSync(path.join(out, 'beta.json'), manifest);
  if (!isBeta(version)) {
    fs.writeFileSync(path.join(out, 'latest.json'), manifest);
    written.push('latest.json');
  }
  const sha512 = crypto.createHash('sha512').update(data).digest('base64');
  fs.writeFileSync(path.join(out, 'latest.yml'), electronYml({ version, file, sha512, size: data.length, date }));
  written.push('latest.yml');
  return written;
}

if (require.main === module) {
  const [dir, version, notesFile, out] = process.argv.slice(2);
  if (!dir || !version || !out) {
    console.error('Aufruf: node scripts/manifests.js <Ordner> <Version> <Text-Datei> <Ausgabeordner>');
    process.exit(2);
  }
  const notes = notesFile && fs.existsSync(notesFile) ? fs.readFileSync(notesFile, 'utf8').trim() : '';
  console.log(`Geschrieben: ${build({ dir, version, notes, out }).join(', ')}`);
}

module.exports = { build, tauriManifest, electronYml, isBeta };
