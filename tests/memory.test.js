// Gedächtnis (src/main/memory.js, knowledge.js, memorytools.js) und sein Netz (src/renderer/src/lib/graph.mjs)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { Memory, learn, parseExtraction, extractionMessages, cleanTags, isSensitive, PROMPT_MAX } = require('../src/main/memory');
const { runTool, toolSpecs } = require('../src/main/tools');
const { apiMessages } = require('../src/main/agent');

function temp() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-gedaechtnis-'));
  return { dir, file: path.join(dir, 'gedaechtnis.json'), done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

const PROJ = path.join(os.tmpdir(), 'ASK-Projekte', 'Budget');
const OTHER = path.join(os.tmpdir(), 'ASK-Projekte', 'Web');

test('Gedächtnis: anlegen, ändern, löschen, nach Projekt, gespeichert in der Datei', () => {
  const t = temp();
  try {
    let clock = 1000;
    const m = new Memory({ file: t.file, now: () => clock++ });
    const a = m.add({ text: '  Mag kurze\nAntworten.  ', tags: ['Stil', '#Antworten', 'stil', 'x', 'y'], scope: 'konto' });
    assert.strictEqual(a.text, 'Mag kurze Antworten.');
    assert.deepStrictEqual(a.tags, ['stil', 'antworten', 'x']);
    const p = m.add({ text: 'Beträge in CHF.', tags: 'budget, format', scope: 'projekt', project: PROJ });
    m.add({ text: 'Nur HTML und CSS.', scope: 'projekt', project: OTHER });
    assert.throws(() => m.add({ text: 'x', scope: 'konto' }), /zu kurz/);
    assert.throws(() => m.add({ text: 'Ohne Projekt', scope: 'projekt' }), /Projekt/);
    // Gleicher Text im selben Bereich: kein zweiter Eintrag
    const twin = m.add({ text: 'mag kurze antworten', scope: 'konto', tags: ['kurz'] });
    assert.strictEqual(twin.id, a.id);
    assert.strictEqual(m.list().length, 3);
    assert.deepStrictEqual(m.forProject(PROJ.toUpperCase()).map((i) => i.id), [p.id], 'Ordner ohne Rücksicht auf Gross- und Kleinschreibung');
    assert.deepStrictEqual(m.visible(PROJ).map((i) => i.text).sort(), ['Beträge in CHF.', 'Mag kurze Antworten.']);
    m.update(p.id, { text: 'Beträge immer in CHF.', tags: ['budget'] });
    // Neu laden: alles in der Datei, ungültige Einträge fallen beim Lesen weg
    const raw = JSON.parse(fs.readFileSync(t.file, 'utf8'));
    raw.items.push({ id: 5, text: 'kaputt' }, null, { id: 'leer', text: '   ' });
    fs.writeFileSync(t.file, JSON.stringify(raw));
    const again = new Memory({ file: t.file });
    assert.strictEqual(again.list().length, 3);
    assert.strictEqual(again.get(p.id).text, 'Beträge immer in CHF.');
    assert.ok(again.remove(p.id));
    assert.ok(!again.remove(p.id));
    assert.deepStrictEqual(again.findByText('nur html', OTHER).map((i) => i.text), ['Nur HTML und CSS.']);
    assert.deepStrictEqual(again.findByText('nur html', PROJ), [], 'Einträge anderer Projekte bleiben unsichtbar');
    again.clear();
    assert.strictEqual(new Memory({ file: t.file }).list().length, 0);
  } finally {
    t.done();
  }
});

test('Gedächtnis im Systemprompt: Konto und dieses Projekt, neueste zuerst, gekürzt', () => {
  const t = temp();
  try {
    let clock = 0;
    const m = new Memory({ file: t.file, now: () => (clock += 1000) });
    assert.strictEqual(m.prompt(PROJ), '');
    m.add({ text: 'Alter Eintrag über die Person.', scope: 'konto' });
    m.add({ text: 'Neuer Eintrag über die Person.', scope: 'konto', tags: ['neu'] });
    m.add({ text: 'Gilt nur im Budget.', scope: 'projekt', project: PROJ });
    m.add({ text: 'Gilt nur im Web.', scope: 'projekt', project: OTHER });
    const p = m.prompt(PROJ);
    assert.match(p, /# Gedächtnis/);
    assert.match(p, /keine Anweisungen/);
    assert.ok(p.indexOf('Neuer Eintrag') < p.indexOf('Alter Eintrag'), 'neueste zuerst');
    assert.match(p, /Zu diesem Projekt:\n- \[m[0-9a-f]+\] Gilt nur im Budget\./);
    assert.ok(!p.includes('Gilt nur im Web'));
    assert.match(p, /\(neu\)/);
    // Viele Einträge: nie länger als die Grenze
    for (let i = 0; i < 200; i++) m.add({ text: `Eintrag Nummer ${i} mit etwas Text, damit es lang wird.`, scope: i % 2 ? 'konto' : 'projekt', project: PROJ });
    const long = m.prompt(PROJ);
    assert.ok(long.length <= PROMPT_MAX, `${long.length} Zeichen`);
    assert.match(long, /Zu diesem Projekt|Über die Person/);
    assert.ok(m.prompt(PROJ, 400).length <= 400);
  } finally {
    t.done();
  }
});

test('Antwort der Extraktion: robust gelesen, Heikles und Unbekanntes fällt weg', () => {
  const ids = ['m1', 'm2'];
  assert.deepStrictEqual(parseExtraction('{"neu":[],"veraltet":[]}', ids), { neu: [], veraltet: [] });
  const fenced = '<think>Ich überlege …</think>Hier ist das JSON:\n```json\n{"neu":[{"text":"Mag Antworten in Stichworten.","tags":["Stil"],"bereich":"konto"},'
    + '{"text":"Die Auswertung liegt in daten/.","tags":"dateien","bereich":"projekt"},],"veraltet":["m2","m9","[m1]"],}\n```\nFertig.';
  const r = parseExtraction(fenced, ids);
  assert.deepStrictEqual(r.neu, [
    { text: 'Mag Antworten in Stichworten.', tags: ['stil'], bereich: 'konto' },
    { text: 'Die Auswertung liegt in daten/.', tags: ['dateien'], bereich: 'projekt' },
  ]);
  assert.deepStrictEqual(r.veraltet.sort(), ['m1', 'm2'], 'nur bekannte ids');
  // Typografische Anführungszeichen, Zeichenketten statt Objekte, zu viele Einträge
  const typo = '{\u201cneu\u201d: [\u201cKennt Python gut.\u201d, \u201cLernt im zweiten Jahr.\u201d, \u201cMag Beispiele.\u201d, \u201cMag Grafiken.\u201d]}';
  assert.deepStrictEqual(parseExtraction(typo).neu.map((n) => n.text), ['Kennt Python gut.', 'Lernt im zweiten Jahr.', 'Mag Beispiele.']);
  // Heikles und Unsinn
  const bad = JSON.stringify({ neu: [{ text: 'Sein Passwort ist Sommer2026.' }, { text: 'Token ghp_abcdefghijklmnop1234' }, { text: 'Hat eine Diagnose ADHS.' },
    { text: 'ok' }, { text: 42 }, null, 'Arbeitet gern abends.'], veraltet: 'm1' });
  assert.deepStrictEqual(parseExtraction(bad, ids), { neu: [{ text: 'Arbeitet gern abends.', tags: [], bereich: 'konto' }], veraltet: [] });
  for (const junk of ['', 'Keine neuen Fakten.', '[1,2,3]', '{kaputt', null, '{"neu": "nichts"}']) {
    assert.deepStrictEqual(parseExtraction(junk, ids), { neu: [], veraltet: [] }, String(junk));
  }
  assert.ok(isSensitive('Meine IBAN ist CH93 0076 2011 6238 5295 7'));
  assert.ok(!isSensitive('Mag kurze Antworten.'));
  assert.deepStrictEqual(cleanTags('A, b; #C, d'), ['a', 'b', 'c']);
});

test('Automatisch merken: eine Anfrage mit falschem ask, übernimmt Neues und streicht Veraltetes', async () => {
  const t = temp();
  try {
    const m = new Memory({ file: t.file });
    const old = m.add({ text: 'Mag lange Erklärungen.', scope: 'konto' });
    const calls = [];
    const ask = { chat: async (req) => { calls.push(req); return { content: `\`\`\`json\n{"neu":[{"text":"Mag kurze Antworten.","tags":["stil"],"bereich":"konto"},{"text":"Daten liegen in daten/.","tags":["dateien"],"bereich":"projekt"}],"veraltet":["${old.id}"]}\n\`\`\`` }; } };
    const res = await learn({ memory: m, ask, model: 'qwen', folder: PROJ, chat: { id: 'c1', title: 'Auswertung' },
      user: 'Bitte in Zukunft kürzer antworten, die Daten liegen übrigens in daten/.', answer: 'Alles klar, ich antworte künftig kürzer.' });
    assert.strictEqual(calls.length, 1);
    assert.strictEqual(calls[0].tools, null);
    assert.strictEqual(calls[0].webSearch, false);
    assert.match(calls[0].messages[0].content, /nur mit JSON/);
    assert.match(calls[0].messages[1].content, new RegExp(`\\[${old.id}\\] \\(konto\\) Mag lange Erklärungen`));
    assert.deepStrictEqual(res.removed, [old.id]);
    assert.deepStrictEqual(res.added.map((i) => [i.text, i.scope, i.source, i.chat]), [['Mag kurze Antworten.', 'konto', 'auto', 'c1'], ['Daten liegen in daten/.', 'projekt', 'auto', 'c1']]);
    assert.strictEqual(m.forProject(PROJ)[0].chatTitle, 'Auswertung');
    // Zu kurz oder nur ein Befehl: keine Anfrage
    assert.strictEqual(await learn({ memory: m, ask, model: 'qwen', folder: PROJ, user: 'ok', answer: 'Gut.' }), null);
    assert.strictEqual(await learn({ memory: m, ask, model: 'qwen', folder: PROJ, user: '/neu', answer: 'x'.repeat(100) }), null);
    assert.strictEqual(calls.length, 1);
    // Unbrauchbare Antwort: nichts passiert
    const before = m.list().length;
    await learn({ memory: m, ask: { chat: async () => ({ content: 'Ich habe nichts gefunden.' }) }, model: 'qwen', folder: PROJ, user: 'Erstelle ein Diagramm aus der Tabelle', answer: 'Das Diagramm liegt in bild.png.' });
    assert.strictEqual(m.list().length, before);
    // Lange Texte werden gekürzt mitgeschickt
    const msgs = extractionMessages({ user: 'a'.repeat(5000), answer: 'b'.repeat(5000), items: [] });
    assert.ok(msgs[1].content.length < 3500);
  } finally {
    t.done();
  }
});

test('Knowledge: nach dem Auftrag höchstens einmal, nie nach Stopp, Fehler oder ausgeschaltet', async () => {
  const t = temp();
  try {
    const { Knowledge } = require('../src/main/knowledge');
    const store = { memory: true };
    const settings = { get: (k) => store[k], set: (v) => Object.assign(store, v) };
    let calls = 0;
    const ask = { chat: async () => { calls++; return { content: '{"neu":[{"text":"Lernt für die Prüfung im Januar.","bereich":"konto"}]}' }; } };
    const sent = [];
    const k = new Knowledge({ dir: t.dir, settings, ask, getModel: () => 'qwen', send: (ch, d) => sent.push([ch, d]) });
    const agent = { chat: { id: 'c1', title: 'Prüfung', messages: [
      { role: 'user', content: 'Ich lerne für die Prüfung im Januar, hilf mir beim Plan.', time: 5 },
      { role: 'assistant', content: '', tool_calls: [{}] },
      { role: 'tool', content: 'x' },
      { role: 'assistant', content: 'Hier ist dein Lernplan bis Januar.' }] } };
    k.afterTurn(agent, PROJ, { type: 'done', stopped: true });
    k.afterTurn(agent, PROJ, { type: 'done', failed: true });
    k.afterTurn(agent, '', { type: 'done' });
    assert.strictEqual(calls, 0);
    k.afterTurn(agent, PROJ, { type: 'done' });
    k.afterTurn(agent, PROJ, { type: 'done' }); // dieselbe Eingabe: kein zweites Mal
    await k.learning;
    assert.strictEqual(calls, 1);
    assert.deepStrictEqual(sent.at(-1), ['memory:changed', { chatId: 'c1', folder: PROJ, added: ['Lernt für die Prüfung im Januar.'], removed: 0 }]);
    assert.match(k.systemPrompt(PROJ), /Lernt für die Prüfung/);
    settings.set({ memory: false });
    assert.doesNotMatch(k.systemPrompt(PROJ), /Gedächtnis/, 'ausgeschaltet: nichts im Systemprompt');
    agent.chat.messages.push({ role: 'user', content: 'Noch etwas Neues für später merken bitte.', time: 9 }, { role: 'assistant', content: 'Gut, mache ich so.' });
    k.afterTurn(agent, PROJ, { type: 'done' });
    assert.strictEqual(calls, 1);
    assert.ok(fs.existsSync(path.join(t.dir, 'gedaechtnis.json')), 'nur in userData');
  } finally {
    t.done();
  }
});

test('Werkzeuge merken und vergessen; ausgeschaltet nicht angeboten', async () => {
  const t = temp();
  try {
    const memory = new Memory({ file: t.file });
    let on = true;
    let changed = 0;
    const ctx = { folder: PROJ, chat: 'c1', chatTitle: 'Test', knowledge: { memory, memoryOn: () => on, changed: () => changed++ } };
    let r = await runTool(null, 'merken', JSON.stringify({ text: 'Mag Tabellen statt Fliesstext.', bereich: 'konto', tags: ['stil'] }), ctx);
    assert.ok(r.ok, r.output);
    assert.match(r.output, /Gemerkt \(id m[0-9a-f]+, alle Projekte\)/);
    r = await runTool(null, 'merken', JSON.stringify({ text: 'Excel-Dateien liegen in daten/.', bereich: 'projekt' }), ctx);
    assert.strictEqual(memory.forProject(PROJ).length, 1);
    r = await runTool(null, 'merken', JSON.stringify({ text: 'Mein Passwort ist geheim123', bereich: 'konto' }), ctx);
    assert.strictEqual(r.ok, false);
    r = await runTool(null, 'vergessen', JSON.stringify({ text: 'tabellen statt' }), ctx);
    assert.ok(r.ok, r.output);
    r = await runTool(null, 'vergessen', JSON.stringify({ id: 'm00000000' }), ctx);
    assert.strictEqual(r.ok, false);
    assert.strictEqual(changed, 3);
    on = false;
    r = await runTool(null, 'merken', JSON.stringify({ text: 'Noch etwas Neues.', bereich: 'konto' }), ctx);
    assert.strictEqual(r.ok, false);
    assert.match(r.output, /ausgeschaltet/);
    assert.ok(toolSpecs().some((s) => s.function.name === 'merken'));
  } finally {
    t.done();
  }
});

test('Skill mit /name: Anleitung geht an das Modell, im Chat steht nur die Eingabe', () => {
  const msgs = apiMessages('System', [{ role: 'user', content: '/bericht Woche 47', skill: { name: 'bericht', text: 'Schreib eine Seite.' } }]);
  assert.match(msgs[1].content, /^\[Die Person ruft ihren Skill «bericht» auf\. Anleitung des Skills:\]\nSchreib eine Seite\.\n\[Ende der Anleitung\]\n\n\/bericht Woche 47$/);
});

test('Netz: gleiche Daten, gleiches Bild; Knoten und Kanten; Ausschnitt passt', async () => {
  const { buildGraph, layout, fit, neighbours } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'graph.mjs')).href);
  const items = [
    { id: 'a', text: 'A', tags: ['stil', 'python'], scope: 'konto', project: null },
    { id: 'b', text: 'B', tags: ['stil'], scope: 'projekt', project: 'C:\\P\\Budget' },
    { id: 'c', text: 'C', tags: [], scope: 'projekt', project: 'c:\\p\\budget' },
  ];
  const g = buildGraph(items, [{ folder: 'C:\\P\\Budget', name: 'Budget', color: '#0082B4' }]);
  assert.deepStrictEqual(g.nodes.map((n) => n.id).sort(), ['du', 'i:a', 'i:b', 'i:c', 'p:c:\\p\\budget', 't:python', 't:stil'].sort());
  assert.strictEqual(g.nodes.find((n) => n.kind === 'projekt').label, 'Budget');
  assert.ok(neighbours(g).get('t:stil').has('i:b'));
  const p1 = layout(g);
  const p2 = layout(buildGraph(items, [{ folder: 'C:\\P\\Budget', name: 'Budget', color: '#0082B4' }]));
  assert.deepStrictEqual(p1, p2, 'stabil');
  assert.deepStrictEqual(p1.du, { x: 0, y: 0 });
  for (const p of Object.values(p1)) assert.ok(Number.isFinite(p.x) && Number.isFinite(p.y));
  const v = fit(p1, 800, 600);
  assert.ok(v.k > 0 && v.k <= 1.15);
  // Grosses Gedächtnis: rechnet in vertretbarer Zeit
  const many = Array.from({ length: 400 }, (_, i) => ({ id: `x${i}`, text: `T${i}`, tags: [`t${i % 40}`, `u${i % 7}`], scope: i % 3 ? 'konto' : 'projekt', project: `C:\\P\\${i % 8}` }));
  const start = Date.now();
  const big = layout(buildGraph(many, []));
  assert.ok(Date.now() - start < 4000, `${Date.now() - start} ms`);
  assert.strictEqual(Object.keys(big).length, 1 + 400 + 47 + 8);
});

test('Arbeitsschleife: merken als Werkzeug, Skill nur an das Modell, done meldet Stopp und Fehler', async () => {
  const t = temp();
  const { Agent } = require('../src/main/agent');
  const { Completion } = require('../src/main/askapi');
  try {
    const memory = new Memory({ file: t.file });
    let on = true;
    const answers = [
      { tool_calls: [{ index: 0, id: 'c1', function: { name: 'merken', arguments: JSON.stringify({ text: 'Mag Tabellen.', bereich: 'konto' }) } }] },
      'Gemerkt.',
      Object.assign(new Error('ASK: kaputt'), { status: 500 }),
      'Fertig.',
    ];
    const requests = [];
    const ask = { async chat(req) {
      requests.push(req);
      const next = answers.shift();
      if (next instanceof Error) throw next;
      const c = new Completion(req.onDelta);
      c.add({ choices: [{ delta: typeof next === 'string' ? { content: next } : next, finish_reason: 'stop' }] });
      return c.end();
    } };
    const agent = new Agent({ ask, terminal: { state: 'gestoppt' }, getFolder: () => t.dir, getModel: () => 'm', getSystemPrompt: () => 'SYS',
      getCommandHint: () => '', getWebSearch: () => false, knowledge: { memory, memoryOn: () => on, changed() {} } });
    const done = [];
    agent.on('event', (e) => { if (e.type === 'done') done.push(e); });
    await agent.send('Merk dir, dass ich Tabellen mag.');
    assert.deepStrictEqual(memory.list().map((i) => [i.text, i.source]), [['Mag Tabellen.', 'merken']]);
    assert.ok(requests[0].tools.some((s) => s.function.name === 'merken'));
    await agent.send('Noch etwas');
    assert.deepStrictEqual(done.map((d) => [d.stopped, d.failed]), [[false, false], [false, true]]);
    on = false;
    await agent.send('/bericht KW 47', [], { skill: { name: 'bericht', text: 'Eine Seite.' } });
    const last = requests.at(-1);
    assert.ok(!last.tools.some((s) => ['merken', 'vergessen'].includes(s.function.name)), 'ausgeschaltet: keine Werkzeuge fürs Gedächtnis');
    assert.match(last.messages.at(-1).content, /Anleitung des Skills:\]\nEine Seite\.[\s\S]*\/bericht KW 47$/);
    assert.strictEqual(agent.transcript().filter((i) => i.role === 'user').at(-1).content, '/bericht KW 47');
  } finally {
    t.done();
  }
});
