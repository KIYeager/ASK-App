// Tests für den GitHub-Abgleich ohne git (src/main/github.js) gegen den Nachbau der GitHub-API.
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { GitHub, blobSha, parseRepo } = require('../src/main/github');
const { createMockGitHub, TOKEN } = require('./e2e/mock-github');

// fetch-Ersatz, der direkt den Nachbau aufruft
function setup() {
  const mock = createMockGitHub();
  const fetchImpl = async (url, options) => {
    const r = mock.handle(options.method, url.replace('https://api.example', ''),
      { authorization: options.headers.Authorization }, options.body ? JSON.parse(options.body) : null);
    return { ok: r.status < 300, status: r.status, text: async () => JSON.stringify(r.body) };
  };
  let token = '';
  const gh = new GitHub({ api: 'https://api.example', web: 'https://github.example', store: { get: () => token, set: (t) => { token = t; } }, fetchImpl });
  return { mock, gh };
}

const folder = (files = {}) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-github-'));
  for (const [p, text] of Object.entries(files)) {
    fs.mkdirSync(path.dirname(path.join(dir, p)), { recursive: true });
    fs.writeFileSync(path.join(dir, p), text);
  }
  return dir;
};
const read = (dir, p) => fs.readFileSync(path.join(dir, p), 'utf8');

test('Prüfsumme wie git, Repository-Angaben', () => {
  assert.strictEqual(blobSha(Buffer.from('')), 'e69de29bb2d1d6434b8b29ae775ad8c2e48c5391'); // wie git hash-object
  assert.strictEqual(parseRepo('https://github.com/lea/projekt.git'), 'lea/projekt');
  assert.strictEqual(parseRepo('lea/projekt'), 'lea/projekt');
  assert.throws(() => parseRepo('projekt'), /besitzer\/name/);
});

test('Verbinden: falsches Token abgelehnt, richtiges gespeichert', async () => {
  const { gh } = setup();
  await assert.rejects(gh.connect('falsch'), /lehnt das Token ab/);
  assert.strictEqual(gh.connected(), false);
  assert.strictEqual(await gh.connect(TOKEN), 'lea-test');
  assert.strictEqual(gh.connected(), true);
});

test('Anlegen, hochladen, in anderem Ordner holen, hin und her ändern', async () => {
  const { mock, gh } = setup();
  await gh.connect(TOKEN);
  const a = folder({ 'index.html': '<h1>Spiel</h1>', 'js/spiel.js': 'let x = 1;', '.ask/chats/c.json': '{}', 'README.md': '# Mein Spiel\n' });
  try {
    const created = await gh.create(a, 'spiel');
    assert.strictEqual(created.repo, 'lea-test/spiel');
    assert.strictEqual(created.private, true);
    assert.strictEqual(mock.fileOnGitHub('lea-test/spiel', 'js/spiel.js'), 'let x = 1;');
    assert.strictEqual(mock.fileOnGitHub('lea-test/spiel', 'README.md'), '# Mein Spiel\n', 'eigene README gewinnt gegen die automatische');
    assert.strictEqual(mock.fileOnGitHub('lea-test/spiel', '.ask/chats/c.json'), null, '.ask wird nicht hochgeladen');

    // Andere Sitzung: klonen, ändern, hochladen
    const b = folder();
    try {
      const cloned = await gh.clone(b, 'https://github.com/lea-test/spiel');
      assert.ok(cloned.written.includes('index.html'));
      assert.strictEqual(read(b, 'js/spiel.js'), 'let x = 1;');
      fs.writeFileSync(path.join(b, 'js/spiel.js'), 'let x = 2;');
      fs.writeFileSync(path.join(b, 'neu.txt'), 'neu');
      const pushed = await gh.push(b, 'x auf 2');
      assert.strictEqual(pushed.uploaded, 2);

      // Erste Sitzung: Hochladen ohne Holen wird abgelehnt, Holen übernimmt die Änderungen
      fs.writeFileSync(path.join(a, 'index.html'), '<h1>Spiel 2</h1>');
      await assert.rejects(gh.push(a, 'veraltet'), /Zuerst github_pull/);
      const pulled = await gh.pull(a);
      assert.deepStrictEqual(pulled.conflicts, []);
      assert.strictEqual(read(a, 'js/spiel.js'), 'let x = 2;');
      assert.strictEqual(read(a, 'neu.txt'), 'neu');
      assert.strictEqual(read(a, 'index.html'), '<h1>Spiel 2</h1>', 'eigene Änderung bleibt');
      await gh.push(a, 'Titel');
      assert.strictEqual(mock.fileOnGitHub('lea-test/spiel', 'index.html'), '<h1>Spiel 2</h1>');

      // Löschen wird übertragen
      fs.rmSync(path.join(a, 'neu.txt'));
      const del = await gh.push(a, 'neu.txt weg');
      assert.deepStrictEqual(del.deleted, ['neu.txt']);
      await gh.pull(b);
      assert.ok(!fs.existsSync(path.join(b, 'neu.txt')));

      // Konflikt: dieselbe Datei hier und auf GitHub geändert, nichts überschrieben
      mock.commitOnGitHub('lea-test/spiel', 'index.html', '<h1>von GitHub</h1>');
      fs.writeFileSync(path.join(b, 'index.html'), '<h1>von hier</h1>');
      const conflict = await gh.pull(b);
      assert.deepStrictEqual(conflict.conflicts, ['index.html']);
      assert.strictEqual(read(b, 'index.html'), '<h1>von hier</h1>');

      // Ordner a holt die Änderung von GitHub (dort unverändert), danach gibt es nichts hochzuladen
      assert.deepStrictEqual((await gh.pull(a)).written, ['index.html']);
      assert.strictEqual(read(a, 'index.html'), '<h1>von GitHub</h1>');
      const again = await gh.push(a, 'nichts');
      assert.ok(again.nothing === true || again.uploaded === 0);
      const status = await gh.status(a);
      assert.strictEqual(status.repo, 'lea-test/spiel');
    } finally {
      fs.rmSync(b, { recursive: true, force: true });
    }
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Hochladen in ein leeres Repository', async () => {
  const { mock, gh } = setup();
  await gh.connect(TOKEN);
  mock.handle('POST', '/user/repos', { authorization: `Bearer ${TOKEN}` }, { name: 'leer', private: true, auto_init: false });
  const a = folder({ 'a.txt': 'A', 'b/c.txt': 'C' });
  try {
    await gh.clone(a, 'lea-test/leer');
    await gh.push(a, 'Start');
    assert.strictEqual(mock.fileOnGitHub('lea-test/leer', 'b/c.txt'), 'C');
    assert.strictEqual(mock.fileOnGitHub('lea-test/leer', 'a.txt'), 'A');
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});
