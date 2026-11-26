// Tests für die Update-Dateien (scripts/manifests.js)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { build } = require('../scripts/manifests');

function setup(version) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-manifest-'));
  const file = path.join(dir, `ASK-Setup-${version}.exe`);
  fs.writeFileSync(file, 'MZ installer');
  fs.writeFileSync(`${file}.sig`, 'c2lnbmF0dXI=\n');
  return { dir, file, out: path.join(dir, 'out') };
}

test('normale Version: latest.json, beta.json und latest.yml für die Electron-Fassung', () => {
  const { dir, file, out } = setup('0.4.0');
  const written = build({ dir, version: '0.4.0', notes: 'Neu: Tauri', out, date: '2026-11-26T10:00:00.000Z', owner: 'o', repo: 'r' });
  assert.deepStrictEqual(written.sort(), ['beta.json', 'latest.json', 'latest.yml']);
  const m = JSON.parse(fs.readFileSync(path.join(out, 'latest.json'), 'utf8'));
  assert.strictEqual(m.version, '0.4.0');
  assert.strictEqual(m.notes, 'Neu: Tauri');
  assert.strictEqual(m.platforms['windows-x86_64'].signature, 'c2lnbmF0dXI=');
  assert.strictEqual(m.platforms['windows-x86_64'].url, 'https://github.com/o/r/releases/download/v0.4.0/ASK-Setup-0.4.0.exe');
  const yml = fs.readFileSync(path.join(out, 'latest.yml'), 'utf8');
  const sha = crypto.createHash('sha512').update(fs.readFileSync(file)).digest('base64');
  assert.match(yml, /^version: 0\.4\.0$/m);
  assert.match(yml, /^path: ASK-Setup-0\.4\.0\.exe$/m);
  assert.ok(yml.includes(`sha512: ${sha}`));
  assert.match(yml, /size: 12/);
});

test('Beta: nur beta.json (und latest.yml), latest.json bleibt', () => {
  const { dir, out } = setup('0.4.0-beta.1');
  const written = build({ dir, version: '0.4.0-beta.1', notes: '', out });
  assert.deepStrictEqual(written.sort(), ['beta.json', 'latest.yml']);
  assert.ok(!fs.existsSync(path.join(out, 'latest.json')));
});
