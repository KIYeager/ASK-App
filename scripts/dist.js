// Baut den Installer ASK-Setup-<Version>.exe mit Tauri (ENTSCHEIDE E60): Oberfläche (Vite), Rust-Hülle,
// NSIS-Installer pro Nutzer, dazu die Signatur für Updates (.sig). Das mitgelieferte Python kommt als
// python-pack aus dem Build-Ordner dazu (ENTSCHEIDE E39); der Pfad steht nicht in tauri.conf.json, weil er je
// Rechner anders ist, sondern in einer Zusatzkonfiguration für diesen Build.
// Signieren: Umgebungsvariable TAURI_SIGNING_PRIVATE_KEY (und ..._PASSWORD), sonst nur ohne Updates (--ohne-signatur).
// Ergebnis in <Build-Ordner>\dist: ASK-Setup-<Version>.exe, ASK-Setup-<Version>.exe.sig, app\ (für die einzelne EXE).
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const { buildDir } = require('./build-dir');
const { version } = require('../package.json');

const repo = path.join(__dirname, '..');
const tauriDir = path.join(repo, 'src-tauri');

// Ressourcen wie in tauri.conf.json, dazu python-pack; für die einzelne EXE wird derselbe Ordner gebraucht
function resources(conf, pythonPack) {
  return { ...conf.bundle.resources, [`${pythonPack.replaceAll('\\', '/')}/`]: 'python-pack/' };
}

function run(cmd, args, env = {}) {
  const res = spawnSync(cmd, args, { cwd: repo, stdio: 'inherit', shell: process.platform === 'win32', env: { ...process.env, ...env } });
  if (res.status !== 0) throw new Error(`${cmd} ${args.join(' ')} endete mit Code ${res.status}`);
}

// Ordner mit ASK.exe und den Ressourcen, so wie der Installer ihn anlegt (für scripts/build-portable.js)
function appFolder(conf, res, out) {
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });
  fs.copyFileSync(path.join(tauriDir, 'target', 'release', 'ASK.exe'), path.join(out, 'ASK.exe'));
  for (const [from, to] of Object.entries(res)) {
    const source = path.resolve(tauriDir, from.replace(/\*\.py$/, ''));
    const target = path.join(out, to);
    if (from.endsWith('*.py')) {
      fs.mkdirSync(target, { recursive: true });
      for (const f of fs.readdirSync(source).filter((n) => n.endsWith('.py'))) fs.copyFileSync(path.join(source, f), path.join(target, f));
    } else {
      fs.cpSync(source, target, { recursive: true });
    }
  }
}

function main() {
  const unsigned = process.argv.includes('--ohne-signatur');
  if (!unsigned && !process.env.TAURI_SIGNING_PRIVATE_KEY) {
    throw new Error('TAURI_SIGNING_PRIVATE_KEY fehlt (README, «Updates signieren»). Ohne Updates bauen: npm run dist -- --ohne-signatur');
  }
  const base = buildDir();
  require('./pack-python').pack(base);
  const conf = JSON.parse(fs.readFileSync(path.join(tauriDir, 'tauri.conf.json'), 'utf8'));
  const res = resources(conf, path.join(base, 'python-pack'));
  const extra = path.join(base, 'tauri.build.json');
  fs.writeFileSync(extra, JSON.stringify({ bundle: { resources: res, createUpdaterArtifacts: !unsigned } }, null, 2));
  run('node', [path.join('scripts', 'nsis-plugins.js')]); // SpiderBanner fürs Fortschrittsfenster (E63)
  run('npx', ['tauri', 'build', '--config', extra, '--bundles', 'nsis']);

  const bundle = path.join(tauriDir, 'target', 'release', 'bundle', 'nsis');
  const setup = fs.readdirSync(bundle).find((f) => f.endsWith('-setup.exe'));
  if (!setup) throw new Error(`Kein Installer in ${bundle}`);
  const dist = path.join(base, 'dist');
  fs.mkdirSync(dist, { recursive: true });
  const target = path.join(dist, `ASK-Setup-${version}.exe`);
  fs.copyFileSync(path.join(bundle, setup), target);
  if (fs.existsSync(path.join(bundle, `${setup}.sig`))) fs.copyFileSync(path.join(bundle, `${setup}.sig`), `${target}.sig`);
  appFolder(conf, res, path.join(dist, 'app'));
  console.log(`Fertig: ${target}`);
}

if (require.main === module) {
  try {
    main();
  } catch (err) {
    console.error(err.message);
    process.exit(1);
  }
}

module.exports = { resources };
