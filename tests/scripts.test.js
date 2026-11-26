// Tests für die Build- und Release-Skripte (node --test tests/). Die Tests der App selbst sind nach Rust
// portiert (src-tauri/core, cargo test).
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { compare, VERSION, nextVersion, fromPrBody } = require('../scripts/release');
const { pythonId, countEntries } = require('../scripts/pack-python');

test('Versionen für npm run release', () => {
  assert.ok(VERSION.test('0.2.0') && VERSION.test('0.3.0-beta.1'));
  assert.ok(!VERSION.test('v0.2.0') && !VERSION.test('0.2'));
  assert.strictEqual(compare('0.2.0', '0.1.9'), 1);
  assert.strictEqual(compare('0.3.0-beta.1', '0.3.0'), -1);
  assert.strictEqual(compare('0.3.0-beta.2', '0.3.0-beta.1'), 1);
  assert.strictEqual(compare('0.3.0-beta.1', '0.2.5'), 1);
  assert.strictEqual(compare('0.1.0', '0.1.0'), 0);
});

test('Version beim Merge: automatisch hochzählen', () => {
  const v = (...a) => nextVersion(...a).version;
  assert.strictEqual(v('0.2.0', 'beta'), '0.2.1-beta.1');
  assert.strictEqual(v('0.2.0', 'release'), '0.2.1');
  assert.strictEqual(v('0.2.1-beta.1', 'beta'), '0.2.1-beta.2');
  assert.strictEqual(v('0.2.1-beta.2', 'release'), '0.2.1');
});

test('Version beim Merge: eigene Nummer aus dem PR', () => {
  const v = (...a) => nextVersion(...a).version;
  assert.strictEqual(v('0.2.0', 'release', '0.3.0'), '0.3.0');
  assert.strictEqual(v('0.2.0', 'beta', '0.3.0'), '0.3.0-beta.1');
  assert.strictEqual(v('0.3.0-beta.1', 'beta', '0.3.0'), '0.3.0-beta.2');
  assert.strictEqual(v('0.2.0', 'beta', '0.3.0-beta.4'), '0.3.0-beta.4');
  assert.match(nextVersion('0.2.0', 'release', '0.2.0').error, /nicht grösser/);
  assert.match(nextVersion('0.2.0', 'release', '0.3.0-beta.1').error, /Beta/);
  assert.match(nextVersion('0.2.0', 'release', 'v1').error, /Form/);
  assert.match(nextVersion('0.2.0', 'kein-release').error, /Release-Art/);
});

test('Angaben aus dem Text des PRs', () => {
  assert.deepStrictEqual(fromPrBody('Text\n\n**Version:** `0.3.0`\nRelease-Text: Neue Seitenleiste\n'), { wanted: '0.3.0', text: 'Neue Seitenleiste' });
  assert.deepStrictEqual(fromPrBody('- version: 1.0.0'), { wanted: '1.0.0', text: '' });
  assert.deepStrictEqual(fromPrBody(''), { wanted: '', text: '' });
  // Die Vorlage selbst erwähnt «Version: 0.3.0» nur in einem Kommentar mitten im Satz
  const template = require('node:fs').readFileSync(path.join(__dirname, '..', '.github', 'pull_request_template.md'), 'utf8');
  assert.deepStrictEqual(fromPrBody(template), { wanted: '', text: '' });
});

test('Dateiversion der einzelnen EXE: immer vier Zahlen, auch bei Betas', () => {
  const { viVersion } = require('../scripts/build-portable');
  assert.strictEqual(viVersion('0.2.3'), '0.2.3.0');
  assert.strictEqual(viVersion('0.2.3-beta.1'), '0.2.3.1');
  assert.strictEqual(viVersion('1.10.0-beta.12'), '1.10.0.12');
  assert.throws(() => viVersion('v1'), /Form/);
});

test('Fassung hängt nur von Build-Skript und requirements.txt ab', () => {
  const files = { 'scripts/build-python.ps1': 'a', 'python/requirements.txt': 'numpy==1' };
  const id = pythonId((f) => files[f]);
  assert.match(id, /^[0-9a-f]{16}$/);
  assert.strictEqual(pythonId((f) => files[f]), id);
  assert.notStrictEqual(pythonId((f) => ({ ...files, 'python/requirements.txt': 'numpy==2' })[f]), id);
  assert.match(pythonId(), /^[0-9a-f]{16}$/); // echte Dateien im Repository
});

test('Dateien und Ordner zählen', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-count-'));
  fs.mkdirSync(path.join(dir, 'a', 'b'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'x'), '');
  fs.writeFileSync(path.join(dir, 'a', 'b', 'y'), '');
  assert.deepStrictEqual(countEntries(dir), { files: 2, dirs: 2 });
});
