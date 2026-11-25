// Tests für die Updates (node --test tests/), mit nachgebautem electron-updater.
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { Updater, notesText, releaseUrl, isBeta } = require('../src/main/updater');
const { isPortable } = require('../src/main/portable');
const { compare, VERSION, nextVersion, fromPrBody } = require('../scripts/release');

const config = { owner: 'KIYeager', repo: 'ASK-Releases' };

function setup({ result = null, error = null, answers = [], busy = false, beta = false, portable = false, packaged = true } = {}) {
  const calls = { dialogs: [], downloads: 0, installs: [], opened: [], beforeInstall: 0, allowPrerelease: [] };
  const fake = {
    allowPrerelease: false,
    async checkForUpdates() {
      calls.allowPrerelease.push(this.allowPrerelease);
      if (error) throw error;
      return result;
    },
    async downloadUpdate() { calls.downloads++; },
    quitAndInstall(...args) { calls.installs.push(args); },
  };
  const state = { busy, beta };
  const u = new Updater({
    app: { isPackaged: packaged, getVersion: () => '0.1.0' },
    config,
    portable,
    beta: () => state.beta,
    busy: () => state.busy,
    ask: async (options) => { calls.dialogs.push(options); return answers.length ? answers.shift() : 1; },
    openExternal: (url) => calls.opened.push(url),
    beforeInstall: async () => { calls.beforeInstall++; },
    updater: fake,
  });
  return { u, calls, state };
}

const available = (version, releaseNotes = '') => ({ isUpdateAvailable: true, updateInfo: { version, releaseNotes } });

test('ohne Paket (npm start) oder ohne Release-Repository: aus', async () => {
  const { u, calls } = setup({ packaged: false });
  assert.strictEqual(u.snapshot().state, 'aus');
  await u.check();
  assert.strictEqual(calls.allowPrerelease.length, 0);
  assert.strictEqual(new Updater({ app: { isPackaged: true }, config: {} }).enabled, false);
});

test('neue Version: fragt, lädt, fragt nach Neustart, installiert mit Fortschrittsfenster', async () => {
  const { u, calls } = setup({ result: available('0.2.0', '<ul><li>Neu: Updates</li></ul>'), answers: [0, 0] });
  await u.check();
  assert.strictEqual(calls.dialogs.length, 2);
  assert.match(calls.dialogs[0].message, /ASK 0\.2\.0 ist da/);
  assert.match(calls.dialogs[0].detail, /• Neu: Updates/);
  assert.strictEqual(calls.downloads, 1);
  assert.strictEqual(calls.beforeInstall, 1);
  assert.deepStrictEqual(calls.installs, [[false, true]]);
});

test('«Später»: nicht von selbst nochmals fragen, aber auf Klick', async () => {
  const { u, calls } = setup({ result: available('0.2.0'), answers: [1] });
  await u.check();
  await u.check();
  assert.strictEqual(calls.dialogs.length, 1);
  assert.strictEqual(u.snapshot().state, 'verfügbar');
  await u.action();
  assert.strictEqual(calls.dialogs.length, 2);
  assert.strictEqual(calls.downloads, 0);
});

test('«Beim Beenden»: kein Neustart, Zustand geladen', async () => {
  const { u, calls } = setup({ result: available('0.2.0'), answers: [0, 1] });
  await u.check();
  assert.strictEqual(calls.installs.length, 0);
  assert.strictEqual(u.snapshot().state, 'geladen');
});

test('während das Modell arbeitet: keine Frage, erst danach', async () => {
  const { u, calls, state } = setup({ result: available('0.2.0'), busy: true, answers: [0, 0] });
  await u.check();
  assert.strictEqual(calls.dialogs.length, 0);
  state.busy = false;
  u.idle();
  await new Promise((r) => setImmediate(r));
  assert.strictEqual(calls.downloads, 1);
  assert.strictEqual(calls.installs.length, 1);
});

test('einzelne EXE: nur Hinweis mit Link, kein Download', async () => {
  const { u, calls } = setup({ result: available('0.2.0'), portable: true, answers: [0] });
  await u.check();
  assert.strictEqual(calls.downloads, 0);
  assert.deepStrictEqual(calls.opened, ['https://github.com/KIYeager/ASK-Releases/releases/tag/v0.2.0']);
});

test('Beta-Schalter geht an electron-updater', async () => {
  const { u, calls, state } = setup({ result: { isUpdateAvailable: false } });
  await u.check();
  state.beta = true;
  await u.check();
  assert.deepStrictEqual(calls.allowPrerelease, [false, true]);
});

test('Fehler: still beim automatischen Prüfen, Meldung beim Klick', async () => {
  const { u, calls } = setup({ error: new Error('net::ERR_INTERNET_DISCONNECTED') });
  await u.check();
  assert.strictEqual(calls.dialogs.length, 0);
  assert.strictEqual(u.snapshot().state, 'fehler');
  await u.check({ manual: true });
  assert.strictEqual(calls.dialogs.length, 1);
  assert.match(calls.dialogs[0].detail, /ERR_INTERNET_DISCONNECTED/);
});

test('kein Update: Meldung nur beim Klick', async () => {
  const { u, calls } = setup({ result: { isUpdateAvailable: false } });
  await u.check();
  assert.strictEqual(calls.dialogs.length, 0);
  await u.check({ manual: true });
  assert.match(calls.dialogs[0].message, /neueste Version/);
});

test('Hilfsfunktionen', () => {
  assert.strictEqual(notesText('<p>Eins &amp; zwei</p><p>Drei</p>'), 'Eins & zwei\nDrei');
  assert.strictEqual(notesText([{ version: '1', note: 'A' }, { version: '2', note: 'B' }]), 'A\nB');
  assert.ok(notesText('x'.repeat(2000)).length < 800);
  assert.strictEqual(releaseUrl(config, '0.3.0-beta.1'), 'https://github.com/KIYeager/ASK-Releases/releases/tag/v0.3.0-beta.1');
  assert.ok(isBeta('0.3.0-beta.1'));
  assert.ok(!isBeta('0.3.0'));
});

test('einzelne EXE erkennen', () => {
  const env = { LOCALAPPDATA: path.join('C:', 'Users', 'lea', 'AppData', 'Local') };
  assert.ok(isPortable(path.join(env.LOCALAPPDATA, 'ASK-Desktop', 'app', '0.1.0-abc', 'ASK.exe'), env));
  assert.ok(!isPortable(path.join(env.LOCALAPPDATA, 'Programs', 'ask-desktop', 'ASK.exe'), env));
  assert.ok(!isPortable('ASK.exe', {}));
});

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
