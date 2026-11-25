// Tests für den GitHub-Abgleich ohne git (src/main/github.js) gegen den Nachbau der GitHub-API.
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { GitHub, blobSha, parseRepo, validLink, validBranch, listLocal } = require('../src/main/github');
const { runTool } = require('../src/main/tools');
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
  assert.strictEqual(parseRepo('https://github.com/lea/website/tree/main'), 'lea/website');
  assert.strictEqual(parseRepo('https://github.com/lea/website/blob/main/README.md'), 'lea/website');
  assert.strictEqual(parseRepo('git@github.com:lea/website.git'), 'lea/website');
  assert.throws(() => parseRepo('../user'), /besitzer\/name/);
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

test('.gitignore: Ausgeschlossenes wird nicht hochgeladen, schon Abgeglichenes bleibt', async () => {
  const { mock, gh } = setup();
  await gh.connect(TOKEN);
  const a = folder({ 'app.py': 'print(1)', '.env': 'GEHEIM=1', '.gitignore': '.env\n*.log\n/build/\n!wichtig.log\ndaten/**/roh\n',
    'lauf.log': 'x', 'wichtig.log': 'y', 'build/app.exe': 'bin', 'src/build/ok.txt': 'ok', 'daten/2026/roh/a.csv': '1', 'daten/2026/fertig.csv': '2' });
  try {
    assert.deepStrictEqual(listLocal(a).map((f) => f.path).sort(), ['.gitignore', 'app.py', 'daten/2026/fertig.csv', 'src/build/ok.txt', 'wichtig.log']);
    await gh.create(a, 'geheim');
    assert.strictEqual(mock.fileOnGitHub('lea-test/geheim', '.env'), null);
    assert.strictEqual(mock.fileOnGitHub('lea-test/geheim', 'build/app.exe'), null);
    assert.strictEqual(mock.fileOnGitHub('lea-test/geheim', 'app.py'), 'print(1)');
    // Auf GitHub liegt eine Datei, die hier ausgeschlossen und anders ist: Holen überschreibt sie nicht
    mock.commitOnGitHub('lea-test/geheim', '.env', 'GEHEIM=von-github');
    const pulled = await gh.pull(a);
    assert.deepStrictEqual(pulled.conflicts, ['.env']);
    assert.strictEqual(read(a, '.env'), 'GEHEIM=1');
    // Eine abgeglichene Datei kommt später in .gitignore: Hochladen löscht sie auf GitHub nicht
    fs.writeFileSync(path.join(a, '.gitignore'), '.env\n*.log\napp.py\n');
    const pushed = await gh.push(a, 'gitignore');
    assert.deepStrictEqual(pushed.deleted, []);
    assert.strictEqual(mock.fileOnGitHub('lea-test/geheim', 'app.py'), 'print(1)');
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Konflikt: Hochladen ersetzt die Fassung auf GitHub nur mit overwrite', async () => {
  const { mock, gh } = setup();
  await gh.connect(TOKEN);
  const a = folder({ 'index.html': 'eins' });
  try {
    await gh.create(a, 'konflikt');
    mock.commitOnGitHub('lea-test/konflikt', 'index.html', 'von GitHub');
    fs.writeFileSync(path.join(a, 'index.html'), 'von hier');
    assert.deepStrictEqual((await gh.pull(a)).conflicts, ['index.html']);
    assert.deepStrictEqual((await gh.status(a)).conflicts, ['index.html']);
    await assert.rejects(gh.push(a, 'x'), /verschieden geändert: index\.html/);
    assert.strictEqual(mock.fileOnGitHub('lea-test/konflikt', 'index.html'), 'von GitHub');
    await gh.push(a, 'x', { overwrite: true });
    assert.strictEqual(mock.fileOnGitHub('lea-test/konflikt', 'index.html'), 'von hier');
    assert.deepStrictEqual((await gh.status(a)).conflicts, []);
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Gespeicherte Verbindung wird geprüft, Ablage ausserhalb des Ordners', async () => {
  const sha = 'a'.repeat(40);
  const ok = { repo: 'lea/web', branch: 'main', commit: null, files: { 'a/b.txt': sha } };
  assert.strictEqual(validLink(ok), true);
  for (const bad of [{ ...ok, repo: 'lea/web/../../user' }, { ...ok, repo: '../x' }, { ...ok, repo: 'lea/web?x=1' }, { ...ok, branch: 'a/../b' },
    { ...ok, commit: 'xyz' }, { ...ok, files: { '../x': sha } }, { ...ok, files: { 'C:/x': sha } }, { ...ok, files: { 'a.txt': 'kurz' } }, null]) {
    assert.strictEqual(validLink(bad), false, JSON.stringify(bad));
  }
  const store = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-github-ablage-'));
  const a = folder({ 'a.txt': 'A' });
  try {
    const { gh } = setup();
    gh.linkFile = () => path.join(store, 'github.json');
    await gh.connect(TOKEN);
    await gh.create(a, 'ablage');
    assert.ok(fs.existsSync(path.join(store, 'github.json')));
    assert.ok(!fs.existsSync(path.join(a, '.ask', 'github.json')));
    fs.writeFileSync(path.join(store, 'github.json'), JSON.stringify({ repo: 'lea-test/ablage/../../user', branch: 'main', commit: null, files: {} }));
    assert.strictEqual((await gh.status(a)).linked, false);
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
    fs.rmSync(store, { recursive: true, force: true });
  }
});

test('Branch-Namen', () => {
  for (const ok of ['main', 'neue-startseite', 'feature/login', 'v1.2', 'lea_test']) assert.strictEqual(validBranch(ok), true, ok);
  for (const bad of ['', '-x', '.x', 'a..b', 'a//b', 'a/', 'a.', 'x.lock', 'a/.b', 'HEAD', 'mit leerzeichen', 'ä', '../main', 'a'.repeat(201), null]) {
    assert.strictEqual(validBranch(bad), false, String(bad));
  }
});

test('Branches: auflisten, anlegen, wechseln (nur ohne Änderungen hier)', async () => {
  const { mock, gh } = setup();
  await gh.connect(TOKEN);
  const a = folder({ 'index.html': 'eins', 'alt.txt': 'alt' });
  try {
    await gh.create(a, 'zweige');
    let b = await gh.branches(a);
    assert.deepStrictEqual(b, { current: 'main', default: 'main', branches: ['main'], url: 'https://github.example/lea-test/zweige' });

    // Neuer Branch mit einer Änderung hier: Die Änderung bleibt und geht auf den neuen Branch
    fs.writeFileSync(path.join(a, 'index.html'), 'zwei');
    const created = await gh.createBranch(a, 'neue-startseite');
    assert.strictEqual(created.from, 'main');
    assert.strictEqual(read(a, 'index.html'), 'zwei');
    await assert.rejects(gh.createBranch(a, 'neue-startseite'), /schon mit dem Branch/);
    await assert.rejects(gh.createBranch(a, 'a..b'), /geht nicht als Branch-Name/);
    b = await gh.branches(a);
    assert.deepStrictEqual(b.branches, ['main', 'neue-startseite']);
    assert.strictEqual(b.current, 'neue-startseite');
    assert.deepStrictEqual((await gh.status(a)).changed, ['index.html']);

    // Wechseln mit Änderungen hier: abgelehnt, mit Grund
    await assert.rejects(gh.switchBranch(a, 'main'), (err) => /noch nicht hochgeladen/.test(err.message) && /index\.html/.test(err.message) && err.pending.length === 1);
    fs.rmSync(path.join(a, 'alt.txt'));
    fs.writeFileSync(path.join(a, 'neu.txt'), 'neu');
    await gh.push(a, 'Neue Startseite');
    assert.strictEqual(mock.fileOnGitHub('lea-test/zweige', 'index.html', 'neue-startseite'), 'zwei');
    assert.strictEqual(mock.fileOnGitHub('lea-test/zweige', 'index.html'), 'eins', 'main bleibt, wie es war');

    // Zurück auf main: Dateien bekommen dessen Stand, die nur auf dem Branch vorhandenen verschwinden
    const back = await gh.switchBranch(a, 'main');
    assert.strictEqual(back.from, 'neue-startseite');
    assert.strictEqual(read(a, 'index.html'), 'eins');
    assert.strictEqual(read(a, 'alt.txt'), 'alt');
    assert.ok(!fs.existsSync(path.join(a, 'neu.txt')));
    const s = await gh.status(a);
    assert.strictEqual(s.branch, 'main');
    assert.deepStrictEqual([...s.changed, ...s.added, ...s.deleted], []);
    assert.strictEqual(s.remoteNewer, false);
    assert.strictEqual((await gh.switchBranch(a, 'main')).same, true);
    await assert.rejects(gh.switchBranch(a, 'gibt-es-nicht'), /gibt es auf GitHub nicht/);
    assert.strictEqual(read(a, 'index.html'), 'eins', 'ein fehlender Branch löscht nichts');

    // Und wieder auf den Branch
    await gh.switchBranch(a, 'neue-startseite');
    assert.strictEqual(read(a, 'neu.txt'), 'neu');
    assert.ok(!fs.existsSync(path.join(a, 'alt.txt')));
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Pull Requests: erstellen und auflisten', async () => {
  const { gh } = setup();
  await gh.connect(TOKEN);
  const a = folder({ 'index.html': 'eins' });
  try {
    await gh.create(a, 'pr');
    await assert.rejects(gh.createPullRequest(a, { title: 'x' }), /Zuerst einen eigenen Branch/);
    await gh.createBranch(a, 'titel');
    await assert.rejects(gh.createPullRequest(a, { title: 'Ohne Änderung' }), /No commits between main and titel/);
    fs.writeFileSync(path.join(a, 'index.html'), 'zwei');
    await gh.push(a, 'Titel geändert');
    await assert.rejects(gh.createPullRequest(a, { title: ' ' }), /braucht einen Titel/);
    const pr = await gh.createPullRequest(a, { title: 'Neuer Titel', body: 'Die Startseite hat einen neuen Titel.' });
    assert.deepStrictEqual(pr, { number: 1, url: 'https://github.com/lea-test/pr/pull/1', head: 'titel', base: 'main' });
    await assert.rejects(gh.createPullRequest(a, { title: 'Nochmals' }), /already exists/);
    const prs = await gh.pullRequests(a);
    assert.deepStrictEqual(prs, [{ number: 1, title: 'Neuer Titel', url: 'https://github.com/lea-test/pr/pull/1', head: 'titel', base: 'main', user: 'lea-test', draft: false }]);
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Werkzeuge github_branch und github_pr', async () => {
  const { gh } = setup();
  await gh.connect(TOKEN);
  const a = folder({ 'index.html': 'eins' });
  const asked = [];
  let answer = true;
  const ctx = { github: gh, folder: a, confirm: async (message, detail) => { asked.push({ message, detail }); return answer; } };
  try {
    await gh.create(a, 'werkzeug');
    let r = await runTool(null, 'github_branch', { aktion: 'liste' }, ctx);
    assert.ok(r.ok && /main \(Standard\) \(verbunden\)/.test(r.output), r.output);
    r = await runTool(null, 'github_branch', { aktion: 'neu' }, ctx);
    assert.ok(!r.ok && /Namen des Branches/.test(r.output));
    r = await runTool(null, 'github_branch', { aktion: 'neu', name: 'aenderung' }, ctx);
    assert.ok(r.ok && /aenderung angelegt/.test(r.output), r.output);

    // PR mit Änderungen, die noch nicht hochgeladen sind: zuerst hochladen
    fs.writeFileSync(path.join(a, 'index.html'), 'zwei');
    r = await runTool(null, 'github_pr', { aktion: 'neu', titel: 'Änderung' }, ctx);
    assert.ok(!r.ok && /zuerst github_push/.test(r.output), r.output);
    // Wechseln mit Änderungen: abgelehnt, das Modell soll fragen
    r = await runTool(null, 'github_branch', { aktion: 'wechseln', name: 'main' }, ctx);
    assert.ok(!r.ok && /noch nicht hochgeladen/.test(r.output) && /Frag die Person/.test(r.output), r.output);
    await gh.push(a, 'zwei');

    // Abgelehnt: kein PR
    answer = false;
    r = await runTool(null, 'github_pr', { aktion: 'neu', titel: 'Änderung', beschreibung: 'Text geändert' }, ctx);
    assert.ok(!r.ok && /abgelehnt/.test(r.output));
    assert.strictEqual(asked.length, 1);
    assert.match(asked[0].message, /Pull Request in lea-test\/werkzeug erstellen/);
    assert.match(asked[0].detail, /Von aenderung nach main/);
    assert.deepStrictEqual(await gh.pullRequests(a), []);
    answer = true;
    r = await runTool(null, 'github_pr', { aktion: 'neu', titel: 'Änderung', beschreibung: 'Text geändert' }, ctx);
    assert.ok(r.ok && /#1/.test(r.output) && /pull\/1/.test(r.output), r.output);
    r = await runTool(null, 'github_pr', { aktion: 'liste' }, ctx);
    assert.ok(r.ok && /#1 Änderung \(aenderung → main/.test(r.output), r.output);

    // Zurück auf main, dort kein PR auf sich selbst
    r = await runTool(null, 'github_branch', { aktion: 'wechseln', name: 'main' }, ctx);
    assert.ok(r.ok && /Gewechselt von aenderung zu main/.test(r.output), r.output);
    assert.strictEqual(read(a, 'index.html'), 'eins');
    r = await runTool(null, 'github_pr', { aktion: 'neu', titel: 'x' }, ctx);
    assert.ok(!r.ok && /eigenen Branch/.test(r.output));

    // Während die Person im Bereich «GitHub» abgleicht, wartet das Modell
    await gh.exclusive(a, async () => {
      const busy = await runTool(null, 'github_pull', {}, ctx);
      assert.ok(!busy.ok && /gleicht gerade selbst/.test(busy.output));
      assert.ok((await runTool(null, 'github_status', {}, { ...ctx, panel: true })).ok, 'der Bereich selbst darf');
    });
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

// Wie setup(), aber mit eigenem fetch (z. B. das hängt) und kurzer Zeitgrenze
function setupWith(fetchWrap, extra = {}) {
  const mock = createMockGitHub();
  const fetchImpl = async (url, options) => {
    const r = mock.handle(options.method, url.replace('https://api.example', ''),
      { authorization: options.headers.Authorization }, options.body ? JSON.parse(options.body) : null);
    return { ok: r.status < 300, status: r.status, text: async () => JSON.stringify(r.body) };
  };
  let token = TOKEN;
  const gh = new GitHub({ api: 'https://api.example', web: 'https://github.example', store: { get: () => token, set: (t) => { token = t; } },
    fetchImpl: fetchWrap ? (url, options) => fetchWrap(fetchImpl, url, options) : fetchImpl, ...extra });
  return { mock, gh };
}

test('Branch auf GitHub weg: kein leerer Stand, lokale Dateien bleiben', async () => {
  const { gh } = setup();
  await gh.connect(TOKEN);
  const a = folder({ 'index.html': 'eins', 'b/c.txt': 'C' });
  try {
    await gh.create(a, 'weg');
    const link = gh.readLink(a);
    // Der Branch existiert auf GitHub nicht (mehr), das Repository aber schon
    gh.writeLink(a, { ...link, branch: 'feature-alt' });
    await assert.rejects(gh.pull(a), /Den Branch feature-alt gibt es auf GitHub nicht mehr\. Wähle im Reiter GitHub einen anderen Branch\./);
    await assert.rejects(gh.status(a), /nicht mehr/);
    await assert.rejects(gh.push(a, 'x'), /nicht mehr/);
    assert.strictEqual(read(a, 'index.html'), 'eins');
    assert.strictEqual(read(a, 'b/c.txt'), 'C');
    // Repository umbenannt oder Zugriff verloren
    gh.writeLink(a, { ...link, repo: 'lea-test/umbenannt' });
    await assert.rejects(gh.pull(a), /Repository lea-test\/umbenannt gibt es auf GitHub nicht mehr/);
    assert.strictEqual(read(a, 'index.html'), 'eins');
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Ein leeres Repository holen löscht hier nichts', async () => {
  const { mock, gh } = setup();
  await gh.connect(TOKEN);
  mock.handle('POST', '/user/repos', { authorization: `Bearer ${TOKEN}` }, { name: 'leer2', private: true, auto_init: false });
  const a = folder({ 'a.txt': 'A' });
  try {
    await gh.clone(a, 'lea-test/leer2');
    const link = gh.readLink(a);
    gh.writeLink(a, { ...link, files: { 'a.txt': blobSha(Buffer.from('A')) } });
    const r = await gh.pull(a);
    assert.deepStrictEqual(r.removed, []);
    assert.strictEqual(read(a, 'a.txt'), 'A');
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Anfrage ohne Antwort bricht nach der Zeitgrenze ab und gibt den Abgleich frei', async () => {
  const hang = (real, url, options) => new Promise((_, reject) => options.signal.addEventListener('abort', () => reject(options.signal.reason)));
  const { gh } = setupWith(hang, { timeoutMs: 30 });
  const a = folder({ 'a.txt': 'A' });
  try {
    await assert.rejects(gh.request('GET', '/repos/x/y'), /GitHub antwortet nicht/);
    await assert.rejects(gh.exclusive(a, () => gh.request('GET', '/repos/x/y')), /GitHub antwortet nicht/);
    assert.strictEqual(gh.locked(a), false);
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Ausschlussliste: Muster mit vielen * brauchen keine Rechenzeit, Regeln bleiben richtig', () => {
  const evil = [`${'*a'.repeat(40)}*b`, `${'**/'.repeat(40)}x`, `${'*'.repeat(60)}.log`, `${'a*'.repeat(30)}[!x]?`].join('\n');
  const a = folder({ ['.git' + 'ignore']: `${evil}\n*.tmp\nbuild/\n/wurzel.txt\ndocs/**/geheim.md\n`,
    [`${'a'.repeat(80)}.txt`]: '1', 'x.tmp': '2', 'build/o.js': '3', 'wurzel.txt': '4', 'sub/wurzel.txt': '5',
    'docs/a/b/geheim.md': '6', 'docs/geheim.md': '7', 'docs/ok.md': '8' });
  try {
    const start = process.hrtime.bigint();
    const names = listLocal(a).map((f) => f.path).sort();
    const ms = Number(process.hrtime.bigint() - start) / 1e6;
    assert.ok(ms < 1000, `zu langsam: ${ms} ms`);
    assert.deepStrictEqual(names, ['.git' + 'ignore', `${'a'.repeat(80)}.txt`, 'docs/ok.md', 'sub/wurzel.txt'].sort());
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});

test('Ballast-Ordner werden in jeder Tiefe übergangen', () => {
  const a = folder({ 'app.js': '1', 'app/node_modules/x/index.js': '2', 'a/b/.git/config': '3', 'py/.venv/lib.py': '4',
    'src/__pycache__/m.pyc': '5', 'src/node_modules_neu/k.js': '6', 'node_modules/y.js': '7' });
  try {
    assert.deepStrictEqual(listLocal(a).map((f) => f.path).sort(), ['app.js', 'src/node_modules_neu/k.js']);
  } finally {
    fs.rmSync(a, { recursive: true, force: true });
  }
});
