// Härtungen aus der Fehlersuche: Adressprüfung, Pfade von GitHub, Gerätenamen von Windows
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { checkUrl } = require('../src/main/web');
const { safeJoin } = require('../src/main/safe');

test('Lokale Adressen auch in anderen Schreibweisen gesperrt', () => {
  for (const u of ['http://localhost./', 'http://foo.localhost./', 'http://[::ffff:127.0.0.1]/', 'http://[::ffff:7f00:1]/', 'http://[::]/',
    'http://[0:0:0:0:0:0:0:0]/', 'http://100.64.1.1/', 'http://127.0.0.1./', 'http://[::ffff:c0a8:101]/']) {
    assert.throws(() => checkUrl(u), /gesperrt/, u);
  }
  for (const u of ['https://www.tfbern.ch/', 'https://100.200.1.1/', 'https://[2001:db8::1]/']) assert.ok(checkUrl(u), u);
});

test('Pfade von GitHub bleiben im Ordner (safeJoin)', () => {
  const root = require('node:fs').mkdtempSync(path.join(require('node:os').tmpdir(), 'ask-safe-'));
  assert.strictEqual(safeJoin(root, 'src/app.js'), path.join(root, 'src', 'app.js'));
  for (const p of ['..\\..\\Startup\\e.bat', '../x', 'a/../../x', 'C:/x', 'a\\b', 'name.', 'ordner /x', '', 'a//b']) {
    assert.throws(() => safeJoin(root, p), undefined, p);
  }
});

test('Gerätenamen von Windows bei Anhängen', () => {
  const { cleanName } = require('../src/main/attachments');
  assert.strictEqual(cleanName('nul.txt'), '_nul.txt');
  assert.strictEqual(cleanName('CON'), '_CON');
  assert.strictEqual(cleanName('console.txt'), 'console.txt');
});
