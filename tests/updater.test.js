// Updates (src/main/updater.js)
const test = require('node:test');
const assert = require('node:assert');
const { Updater, compareVersions, versionFromYaml } = require('../src/main/updater');

test('Versionen vergleichen', () => {
  assert.ok(compareVersions('0.2.0', '0.1.9') > 0);
  assert.ok(compareVersions('1.10.0', '1.9.0') > 0);
  assert.strictEqual(compareVersions('v1.0.0', '1.0'), 0);
  assert.ok(compareVersions('1.0.0-beta.2', '1.0.0') < 0);
  assert.ok(compareVersions('1.0.0-beta.10', '1.0.0-beta.2') > 0);
});

test('Version aus latest.yml', () => {
  assert.strictEqual(versionFromYaml("version: 0.3.1\nfiles:\n  - url: ASK-Setup-0.3.1.exe\n"), '0.3.1');
  assert.strictEqual(versionFromYaml("version: '1.0.0'"), '1.0.0');
  assert.strictEqual(versionFromYaml('nichts'), '');
});

test('Ohne Adresse oder im Entwicklungsmodus aus', () => {
  assert.strictEqual(new Updater({ version: '0.1.0', url: '', packaged: true }).status.state, 'aus');
  assert.strictEqual(new Updater({ version: '0.1.0', url: 'https://x.ch/ask', packaged: false }).status.state, 'aus');
  assert.strictEqual(new Updater({ version: '0.1.0', url: 'http://x.ch/ask', packaged: true }).status.state, 'aus');
});

test('Einzelne EXE: Hinweis mit Link auf die neue EXE', async () => {
  const fetchImpl = async (url) => ({ ok: true, text: async () => (url.includes('latest.yml') ? 'version: 0.2.0\n' : '') });
  const u = new Updater({ version: '0.1.0', url: 'https://updates.example.ch/ask/', packaged: true, portable: true, fetchImpl });
  const s = await u.check();
  assert.strictEqual(s.state, 'verfuegbar');
  assert.strictEqual(s.available, '0.2.0');
  assert.strictEqual(s.download, 'https://updates.example.ch/ask/ASK-0.2.0.exe');
  const same = new Updater({ version: '0.2.0', url: 'https://u.ch', packaged: true, portable: true, fetchImpl });
  assert.strictEqual((await same.check()).state, 'aktuell');
  const down = new Updater({ version: '0.1.0', url: 'https://u.ch', packaged: true, portable: true, fetchImpl: async () => ({ ok: false, status: 404 }) });
  assert.strictEqual((await down.check()).state, 'fehler');
});

test('Installierte App: Ereignisse von electron-updater werden zum Status', async () => {
  const { EventEmitter } = require('node:events');
  const au = new EventEmitter();
  au.setFeedURL = (o) => { au.feed = o; };
  au.checkForUpdates = async () => { au.emit('update-available', { version: '0.5.0' }); au.emit('download-progress', { percent: 42.4 }); };
  au.quitAndInstall = () => { au.installed = true; };
  const u = new Updater({ version: '0.1.0', url: 'https://u.ch/ask', packaged: true, loadAutoUpdater: () => au });
  u.setupAutoUpdater();
  assert.deepStrictEqual(au.feed, { provider: 'generic', url: 'https://u.ch/ask' });
  await u.check();
  assert.strictEqual(u.status.state, 'laedt');
  assert.strictEqual(u.status.progress, 42);
  assert.strictEqual(u.install(), false);
  au.emit('update-downloaded', { version: '0.5.0' });
  assert.strictEqual(u.status.state, 'bereit');
  assert.strictEqual(u.install(), true);
  await new Promise((r) => setImmediate(r));
  assert.ok(au.installed);
});
