// Tests für src/main/lowlabel.js: Marke «Low» entfernen (der Prozessaufruf ist ersetzt).
const test = require('node:test');
const assert = require('node:assert');
const { restoreLabel } = require('../src/main/lowlabel');

function setup(opts = {}) {
  const calls = [];
  const run = async (file, args, options) => { calls.push({ file, args, options }); };
  const base = { run, platform: 'win32', exists: () => true, home: 'C:\\Users\\anna', otherFolders: [], ...opts };
  return { calls, call: (folder) => restoreLabel(folder, base) };
}

test('ruft icacls mit /T ohne Shell auf', async () => {
  const { calls, call } = setup();
  const r = await call('C:\\Arbeit\\Projekt');
  assert.strictEqual(r.skipped, false);
  assert.strictEqual(calls.length, 1);
  assert.match(calls[0].file, /System32\\icacls\.exe$/i);
  assert.deepStrictEqual(calls[0].args, ['C:\\Arbeit\\Projekt', '/setintegritylevel', '(OI)(CI)M', '/T', '/C', '/Q']);
  assert.strictEqual(calls[0].options.windowsHide, true);
  assert.strictEqual(calls[0].options.shell, false);
  assert.ok(calls[0].options.timeout >= 60000);
});

test('nicht unter Windows: nichts tun', async () => {
  const { calls, call } = setup({ platform: 'linux' });
  const r = await call('C:\\Arbeit\\Projekt');
  assert.strictEqual(r.skipped, true);
  assert.strictEqual(calls.length, 0);
});

test('Ordner existiert nicht mehr: nichts tun', async () => {
  const { calls, call } = setup({ exists: () => false });
  assert.strictEqual((await call('C:\\Arbeit\\Weg')).skipped, true);
  assert.strictEqual(calls.length, 0);
});

test('Laufwerkswurzel und Benutzerordner werden nie zurückgesetzt', async () => {
  const { calls, call } = setup();
  assert.strictEqual((await call('C:\\')).skipped, true);
  assert.strictEqual((await call('c:\\users\\ANNA\\')).skipped, true);
  assert.strictEqual(calls.length, 0);
});

test('gleicher, enthaltener und enthaltender Ordner eines anderen Projekts: überspringen', async () => {
  const other = ['C:\\Arbeit\\A\\B'];
  for (const f of ['c:\\arbeit\\a\\b', 'C:\\Arbeit\\A\\B\\sub', 'C:/Arbeit/A', 'C:\\Arbeit\\A\\B\\']) {
    const { calls, call } = setup({ otherFolders: other });
    const r = await call(f);
    assert.strictEqual(r.skipped, true, f);
    assert.strictEqual(calls.length, 0, f);
  }
});

test('Ordner mit gleichem Namensanfang zählt nicht als darin', async () => {
  const { calls, call } = setup({ otherFolders: ['C:\\a\\b'] });
  assert.strictEqual((await call('C:\\a\\bc')).skipped, false);
  assert.strictEqual((await call('C:\\a\\b2\\x')).skipped, false);
  assert.strictEqual(calls.length, 2);
});

test('Fehler des Prozesses gehen an die aufrufende Stelle', async () => {
  const { call } = setup({ run: async () => { throw new Error('icacls kaputt'); } });
  await assert.rejects(call('C:\\Arbeit\\Projekt'), /icacls kaputt/);
});
