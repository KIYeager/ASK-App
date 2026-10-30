const test = require('node:test');
const assert = require('node:assert');
const os = require('node:os');
const { parsePages, keyCode, Visual, allowedNavigation } = require('../src/main/visual');
const { apiMessages, loopReason } = require('../src/main/agent');
const fs = require('node:fs');
const path = require('node:path');
const { runTool, toolSpecs } = require('../src/main/tools');

test('Seitenangaben und Tasten', () => {
  assert.deepStrictEqual(parsePages(''), [1, 2, 3]);
  assert.deepStrictEqual(parsePages('2'), [2]);
  assert.deepStrictEqual(parsePages('2-4'), [2, 3, 4]);
  assert.deepStrictEqual(parsePages('5, 1; 5'), [5, 1]);
  assert.deepStrictEqual(parsePages('1-50'), [1, 2, 3, 4, 5, 6], 'höchstens 6');
  assert.deepStrictEqual(parsePages('x'), [1, 2, 3]);
  assert.strictEqual(keyCode('ArrowUp'), 'Up');
  assert.strictEqual(keyCode('Leertaste'), 'Space');
  assert.strictEqual(keyCode('a'), 'a');
});

test('Das Modell erfährt, dass die App einen sichtbaren Browser hat', () => {
  const spec = toolSpecs().find((s) => s.function.name === 'browser').function.description;
  assert.match(spec, /sichtbar im Browser-Bereich/);
  assert.ok(!/unsichtbar/.test(spec), spec);
  const prompt = fs.readFileSync(path.join(__dirname, '..', 'resources', 'anweisungen', 'systemprompt.md'), 'utf8');
  assert.match(prompt, /Die App hat einen eingebauten Browser/);
});

test('Browser: Ziele und Navigation', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-browser-'));
  try {
    fs.writeFileSync(path.join(root, 'spiel.html'), '<p>x</p>');
    fs.writeFileSync(path.join(root, 'daten.txt'), 'x');
    const v = new Visual({});
    assert.strictEqual(v.target(root, ''), null, 'ohne ziel: auf der offenen Seite weiter');
    assert.strictEqual(v.target(root, 'https://gloomwit.vercel.app').url, 'https://gloomwit.vercel.app/');
    assert.strictEqual(v.target(root, 'gloomwit.vercel.app').url, 'https://gloomwit.vercel.app/');
    const file = v.target(root, 'spiel.html');
    assert.match(file.url, /^file:.*spiel\.html$/);
    assert.ok(allowedNavigation(`${file.fileRoot}andere.html`, file.fileRoot), 'andere Datei im Arbeitsordner');
    assert.ok(!allowedNavigation('file:///C:/Windows/win.ini', file.fileRoot));
    assert.ok(allowedNavigation('https://example.ch/x', ''));
    assert.ok(!allowedNavigation('file:///C:/x.html', ''));
    assert.ok(!allowedNavigation('javascript:alert(1)', file.fileRoot));
    assert.throws(() => v.target(root, 'daten.txt'), /nur \.html/);
    assert.throws(() => v.target(root, '../aussen.html'), /nicht gefunden|Arbeitsordner/);
    assert.throws(() => v.target(root, 'file:///C:/Windows/win.ini'), /nur http/);
    assert.throws(() => v.target(root, 'javascript:alert(1)'), /nur http/);
    for (const local of ['http://127.0.0.1:8000/', 'http://localhost/', 'http://192.168.1.1/']) assert.throws(() => v.target(root, local), /lokalen Netz/, local);
    assert.strictEqual(new Visual({ allowLocal: true }).target(root, 'http://127.0.0.1:8000/').url, 'http://127.0.0.1:8000/');
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('Ansichten: Bilder als eigene Nachricht, nur die letzten zwei, Schleifenschutz zählt sie nicht als Eingabe', () => {
  const view = (n) => ({ role: 'user', auto: true, note: 'view', content: `[Ansicht] Bilder zu view_file(${n}.docx)`,
    attachments: [{ path: `.ask/arbeit/c/ansicht/${n}.png`, name: `${n}.png`, image: true }] });
  const call = (id) => ({ role: 'assistant', content: '', tool_calls: [{ id, type: 'function', function: { name: 'view_file', arguments: '{"path":"a.docx"}' } }] });
  const result = (id) => ({ role: 'tool', tool_call_id: id, content: 'Seiten 1 von 1' });
  const messages = [{ role: 'user', content: 'mach den Bericht', attachments: [{ path: '.ask/anhaenge/c/foto.png', name: 'foto.png', image: true }] },
    call('1'), result('1'), view('a'), call('2'), result('2'), view('b'), call('3'), result('3'), view('c')];
  const out = apiMessages('sys', messages, false, (a) => `data:image/png;base64,${a.name}`);
  const views = out.filter((m) => m.role === 'user').slice(1);
  assert.strictEqual(views[0].content, '[Ansicht] Bilder zu view_file(a.docx): nicht mehr mitgeschickt.');
  assert.strictEqual(views[1].content[1].image_url.url, 'data:image/png;base64,b.png');
  assert.strictEqual(views[2].content[0].text, '[Ansicht] Bilder zu view_file(c.docx)');
  assert.ok(Array.isArray(out[1].content), 'Anhang der Person bleibt mit Bild');
  // drei gleiche Aufrufe seit der Eingabe der Person, obwohl Ansichten dazwischen liegen
  assert.match(loopReason(messages), /3-mal/);
});

test('Hochladen: Häkchen merkt sich das Repository, danach ohne Rückfrage', async () => {
  const trusted = new Set();
  const asked = [];
  const pushed = [];
  const github = {
    connected: () => true,
    status: async () => ({ linked: true, repo: 'lea/web', changed: ['index.html'], added: [], deleted: [] }),
    push: async (_folder, message) => { pushed.push(message); return { uploaded: 1, deleted: [], url: 'https://github.com/lea/web' }; },
  };
  const ctx = (checked) => ({ github, folder: 'C:\\x', githubTrust: { has: (r) => trusted.has(r), add: (r) => trusted.add(r) },
    confirm: async (message, _detail, options) => { asked.push({ message, checkbox: options?.checkbox }); return { ok: true, checked }; } });
  const first = await runTool(null, 'github_push', { nachricht: 'eins' }, ctx(false));
  assert.ok(first.ok);
  assert.strictEqual(asked.length, 1);
  assert.strictEqual(asked[0].checkbox, 'Für dieses Repository nicht mehr fragen');
  await runTool(null, 'github_push', { nachricht: 'zwei' }, ctx(true));
  assert.deepStrictEqual([...trusted], ['lea/web']);
  await runTool(null, 'github_push', { nachricht: 'drei' }, ctx(false));
  assert.strictEqual(asked.length, 2, 'dritter Push ohne Rückfrage');
  assert.deepStrictEqual(pushed, ['eins', 'zwei', 'drei']);
  // Abgelehnt: nichts hochgeladen
  trusted.clear();
  const no = await runTool(null, 'github_push', { nachricht: 'vier' }, { ...ctx(false), confirm: async () => ({ ok: false, checked: true }) });
  assert.ok(!no.ok && pushed.length === 3 && trusted.size === 0);
  // ohne Bildfähigkeit kein view_file
  const v = await runTool(null, 'view_file', { path: 'a.png' }, { visual: {}, folder: 'C:\\x', canSee: false });
  assert.ok(!v.ok && /keine Bilder/.test(v.output));
});
