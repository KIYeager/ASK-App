// Datenmodell der Chat-Anzeige (src/renderer/src/lib/chatModel.mjs)
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const load = () => import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'chatModel.mjs')).href);

const stats = { tokens: 100, genMs: 1000, totalMs: 1500, firstMs: 200 };

test('Live-Ereignisse ergeben Eingabe, Arbeitsschritte, Antwort und Statistik', async () => {
  const { emptyChat, applyEvent, buildTurns } = await load();
  let m = emptyChat();
  const evs = [
    { type: 'user', text: 'Werte aus' },
    { type: 'assistant-start' },
    { type: 'reasoning', text: 'Ich lese ' },
    { type: 'reasoning', text: 'zuerst.' },
    { type: 'assistant-end', content: '', stats, calls: [] },
    { type: 'tool-start', id: 'a', name: 'read_file', label: 'Lesen', detail: 'a.csv' },
    { type: 'tool-end', id: 'a', ok: true, summary: '60 Zeilen', output: 'x' },
    { type: 'tool-start', id: 'b', name: 'read_file', label: 'Lesen', detail: 'b.csv' },
    { type: 'tool-end', id: 'b', ok: true, summary: '3 Zeilen', output: 'x' },
    { type: 'tool-start', id: 'c', name: 'run_command', label: 'Befehl', detail: 'python x.py' },
    { type: 'tool-progress', id: 'c', output: 'läuft' },
    { type: 'tool-end', id: 'c', ok: false, summary: 'Exitcode 1', output: 'Traceback (most recent call last)' },
    { type: 'assistant-start' },
    { type: 'text', text: 'Fer' },
    { type: 'text', text: 'tig.' },
    { type: 'assistant-end', content: 'Fertig.', stats, sources: [{ url: 'https://a.ch', title: 'A' }] },
    { type: 'changes', snapshot: 's1', changes: { changed: ['a.csv'], added: [], removed: [] } },
    { type: 'done' },
  ];
  for (const ev of evs) m = applyEvent(m, ev);
  const turns = buildTurns(m, false);
  assert.strictEqual(turns.length, 1);
  const t = turns[0];
  assert.strictEqual(t.user.text, 'Werte aus');
  assert.strictEqual(t.answer.text, 'Fertig.');
  assert.deepStrictEqual(t.steps.map((s) => s.type), ['thought', 'group', 'tool']);
  assert.strictEqual(t.steps[1].entries.length, 2);
  assert.strictEqual(t.steps[2].entry.ok, false);
  assert.match(t.steps[2].entry.summary, /Exitcode 1 · Traceback/);
  assert.strictEqual(t.toolCount, 3);
  assert.strictEqual(t.failed, 1);
  assert.deepStrictEqual(t.after.map((e) => e.k), ['sources', 'changes', 'stats']);
  assert.strictEqual(t.after[2].list.length, 2);
});

test('Gespeicherter Chat entspricht dem Live-Ablauf, unbeendete Werkzeuge gelten als fehlgeschlagen', async () => {
  const { fromTranscript, buildTurns } = await load();
  const items = [
    { role: 'user', content: 'Hallo', time: 1, snapshot: 's1', changes: { changed: [], added: ['n.txt'], removed: [] } },
    { role: 'assistant', content: '', reasoning: '', stats, calls: [{ id: 'a', name: 'write_file', label: 'Schreiben', detail: 'n.txt' }] },
    { role: 'tool', id: 'a', ok: true, summary: '1 Zeile', output: 'hi' },
    { role: 'assistant', content: 'Erledigt', stats, calls: [] },
    { role: 'user', content: 'Noch was', time: 2 },
    { role: 'assistant', content: '', stats, calls: [{ id: 'b', name: 'run_command', label: 'Befehl', detail: 'dir' }] },
  ];
  const m = fromTranscript(items, false);
  const turns = buildTurns(m, false);
  assert.strictEqual(turns.length, 2);
  assert.strictEqual(turns[0].answer.text, 'Erledigt');
  assert.strictEqual(turns[0].steps[0].entry.summary, '1 Zeile geschrieben');
  assert.deepStrictEqual(turns[0].after.map((e) => e.k), ['changes', 'stats']);
  assert.strictEqual(turns[1].steps[0].entry.ok, false);
  assert.strictEqual(turns[1].steps[0].entry.summary, 'nicht beendet');
  assert.strictEqual(turns[1].answer, null);
});

test('Während der Arbeit: Text vor einem Werkzeug ist ein Zwischenschritt, Rückfragen werden beantwortet', async () => {
  const { emptyChat, applyEvent, buildTurns } = await load();
  let m = emptyChat();
  for (const ev of [
    { type: 'user', text: 'Lösch alt' },
    { type: 'assistant-start' },
    { type: 'text', text: 'Ich lösche die Datei.' },
    { type: 'assistant-end', content: 'Ich lösche die Datei.', stats },
    { type: 'tool-start', id: 'x', name: 'run_command', label: 'Befehl', detail: 'del alt.txt' },
    { type: 'approval', id: 'p1', title: 'Datei löschen?', detail: 'del alt.txt' },
  ]) m = applyEvent(m, ev);
  let t = buildTurns(m, true)[0];
  assert.strictEqual(t.answer, null);
  assert.strictEqual(t.steps[0].type, 'thought');
  assert.strictEqual(t.running, true);
  assert.strictEqual(t.after[0].k, 'approval');
  assert.strictEqual(t.after[0].answer, null);
  m = applyEvent(m, { type: 'approval-end', id: 'p1', answer: 'yes' });
  const answered = buildTurns(m, true)[0];
  assert.strictEqual(answered.after.length, 0);
  assert.strictEqual(answered.steps.find((s) => s.type === 'approval').entry.answer, 'yes');
  m = applyEvent(m, { type: 'approval', id: 'p2', title: 'Noch eine?' });
  m = applyEvent(m, { type: 'done' });
  t = buildTurns(m, false)[0];
  assert.strictEqual(t.steps.find((s) => s.entry.id === 'p2').entry.answer, 'no');
});

test('Rückgängig markiert die Änderungen und fügt einen Hinweis an', async () => {
  const { emptyChat, applyEvent } = await load();
  let m = emptyChat();
  m = applyEvent(m, { type: 'user', text: 'x' });
  m = applyEvent(m, { type: 'changes', snapshot: 's9', changes: { changed: ['a'], added: [], removed: [] } });
  m = applyEvent(m, { type: 'done' });
  m = applyEvent(m, { type: 'undo', snapshot: 's9', text: 'Rückgängig gemacht' });
  assert.strictEqual(m.entries.find((e) => e.k === 'changes').undone, true);
  assert.strictEqual(m.entries.at(-1).text, 'Rückgängig gemacht');
});

test('Statistikzeile und Gruppentitel', async () => {
  const { statsText, groupTitle } = await load();
  const s = statsText([stats, { ...stats, estimated: true }]);
  assert.match(s.text, /^2 Schritte · ≈ 200 Token · ≈ 100 Token\/s · Modell 3[.,]0 s$/);
  assert.match(s.title, /geschätzt/);
  assert.strictEqual(statsText([]), null);
  assert.strictEqual(groupTitle([{ name: 'read_file' }, { name: 'read_file' }, { name: 'list_files' }, { name: 'search_text' }]),
    '2 Dateien gelesen, 1 Ordner angesehen, 1 Suche');
  // Quellen (E84)
  assert.strictEqual(groupTitle([{ name: 'quellen_suchen' }, { name: 'quellen_suchen' }, { name: 'quelle_lesen' }]), '2× in den Quellen gesucht, 1 Stelle in den Quellen gelesen');
  assert.strictEqual(groupTitle([{ name: 'quellen_suchen' }]), 'In den Quellen gesucht');
});

test('Dauer: Sekunden, ab einer Minute Minuten und Sekunden, ab einer Stunde Stunden', async () => {
  const { dauer, statsText } = await load();
  assert.strictEqual(dauer(0), '0 s');
  assert.strictEqual(dauer(45_000), '45 s');
  assert.strictEqual(dauer(59_600), '1 min', 'nie «60 s»');
  assert.strictEqual(dauer(60_000), '1 min');
  assert.strictEqual(dauer(1_000_000), '16 min 40 s', 'nicht «1000 s»');
  assert.strictEqual(dauer(3_599_000), '59 min 59 s');
  assert.strictEqual(dauer(3_600_000), '1 h');
  assert.strictEqual(dauer(3_725_000), '1 h 2 min');
  assert.match(dauer(3000, { decimals: true }), /^3[.,]0 s$/);
  assert.strictEqual(dauer(59_970, { decimals: true }), '1 min', 'nie «60,0 s»');
  assert.strictEqual(dauer(-5), '0 s');
  const long = statsText([{ ...stats, totalMs: 1_234_500, genMs: 1_200_000 }]);
  assert.match(long.text, /Modell 20 min 35 s$/, long.text);
  assert.ok(!/\d{3,} s/.test(long.title), long.title);
});

test('Markdown bleibt maskiert und kennt Zitate', async () => {
  const { renderMarkdown } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'md.mjs')).href);
  const html = renderMarkdown('> **Hinweis** <script>x</script>\n\n`a.txt`');
  assert.match(html, /<blockquote><p><strong>Hinweis<\/strong> &lt;script&gt;/);
  assert.match(html, /<code>a.txt<\/code>/);
  assert.doesNotMatch(html, /<script>/);
});

test('Fragen des Modells stehen unter der Karte und zählen nicht als Arbeitsschritt', async () => {
  const { emptyChat, applyEvent, buildTurns, fromTranscript } = await load();
  let m = emptyChat();
  for (const ev of [
    { type: 'user', text: 'Werte aus' },
    { type: 'assistant-end', content: '', stats },
    { type: 'tool-start', id: 'q', name: 'frage_stellen', label: 'Frage', detail: 'Welche Datei?' },
    { type: 'question', id: 'q', frage: 'Welche Datei?', optionen: ['a.csv', 'b.csv'] },
  ]) m = applyEvent(m, ev);
  let t = buildTurns(m, true)[0];
  assert.strictEqual(t.toolCount, 0);
  assert.strictEqual(t.after[0].question.open, true);
  assert.deepStrictEqual(t.after[0].question.optionen, ['a.csv', 'b.csv']);
  m = applyEvent(m, { type: 'question-end', id: 'q', answer: 'b.csv' });
  m = applyEvent(m, { type: 'tool-end', id: 'q', ok: true, summary: 'b.csv', output: 'Frage: Welche Datei?\nAntwort: b.csv' });
  t = buildTurns(m, true)[0];
  assert.strictEqual(t.after.length, 0);
  assert.strictEqual(t.steps[0].type, 'question');
  assert.strictEqual(t.steps[0].entry.question.answer, 'b.csv');
  // gespeichert: Frage aus dem Aufruf, Antwort aus dem Ergebnis
  const saved = fromTranscript([{ role: 'user', content: 'x' }, { role: 'assistant', content: '', stats, calls: [{ id: 'q', name: 'frage_stellen', label: 'Frage', detail: 'Welche?', optionen: ['A'] }] },
    { role: 'tool', id: 'q', ok: true, summary: 'A', output: '' }]);
  const q = buildTurns(saved)[0].steps[0].entry;
  assert.deepStrictEqual([q.question.frage, q.question.optionen, q.summary], ['Welche?', ['A'], 'A']);
});

test('«merken» steht als eigener Eintrag im Chat, live und aus dem gespeicherten Chat', async () => {
  const { emptyChat, applyEvent, fromTranscript, buildTurns } = await load();
  const output = 'Gemerkt (id m1a2b3c4, alle Projekte): Mag Antworten in Stichworten.';
  let m = [{ type: 'user', text: 'Merk dir das' }, { type: 'assistant-start' }, { type: 'tool-start', id: 't1', name: 'merken', label: 'Merken', detail: 'x' },
    { type: 'tool-end', id: 't1', ok: true, summary: 'x', output }, { type: 'text', text: 'Gemerkt.' }, { type: 'assistant-end', content: 'Gemerkt.', stats }]
    .reduce(applyEvent, emptyChat());
  const note = m.entries.find((e) => e.k === 'remembered');
  assert.deepStrictEqual({ id: note.memoryId, scope: note.scope, text: note.text }, { id: 'm1a2b3c4', scope: 'alle Projekte', text: 'Mag Antworten in Stichworten.' });
  assert.ok(buildTurns(m)[0].after.some((e) => e.k === 'remembered'));
  m = fromTranscript([{ role: 'user', content: 'Merk dir das' }, { role: 'assistant', content: '', calls: [{ id: 't1', name: 'merken' }] },
    { role: 'tool', id: 't1', ok: true, output }]);
  assert.ok(m.entries.some((e) => e.k === 'remembered' && e.memoryId === 'm1a2b3c4'));
  const failed = [{ type: 'tool-start', id: 't2', name: 'merken' }, { type: 'tool-end', id: 't2', ok: false, output: 'Nicht gemerkt' }].reduce(applyEvent, emptyChat());
  assert.ok(!failed.entries.some((e) => e.k === 'remembered'));
});

test('Chat laden: offene Frage und Rückfrage eines arbeitenden Chats sind nach dem Wechsel wieder da', async () => {
  const { loadChat } = await import('../src/renderer/src/lib/chatModel.mjs');
  const items = [
    { role: 'user', content: 'Räum auf' },
    { role: 'assistant', content: '', calls: [{ id: 'q1', name: 'frage_stellen', label: 'Frage', detail: 'Was löschen?' }] },
  ];
  const question = { type: 'question', id: 'q1', frage: 'Was löschen?', optionen: ['Alles', 'Nichts'] };
  const approval = { type: 'approval', id: 'p1', title: 'Darf ASK löschen?', detail: 'del x' };
  const m = loadChat(items, true, [question, approval]);
  const q = m.entries.find((e) => e.id === 'q1');
  assert.equal(q.question.open, true);
  assert.deepEqual(q.question.optionen, ['Alles', 'Nichts']);
  assert.equal(m.entries.filter((e) => e.k === 'tool' && e.id === 'q1').length, 1, 'die Frage steht nicht doppelt da');
  assert.equal(m.entries.find((e) => e.k === 'approval').answer, null);
  // Arbeitet der Chat nicht mehr, ist nichts offen (veraltete Angaben zählen nicht)
  assert.equal(loadChat(items, false, [question]).entries.find((e) => e.id === 'q1').question.open, false);
});

test('Aufträge: unveränderte behalten ihr Objekt, nur der geänderte ist neu', async () => {
  const { applyEvent, buildTurns, emptyChat, reuseTurns } = await import('../src/renderer/src/lib/chatModel.mjs');
  let m = [{ type: 'user', text: 'eins' }, { type: 'text', text: 'Antwort eins' }, { type: 'assistant-end', content: 'Antwort eins' }, { type: 'done' },
    { type: 'user', text: 'zwei' }, { type: 'text', text: 'Ant' }].reduce(applyEvent, emptyChat());
  const first = reuseTurns([], buildTurns(m, true));
  m = applyEvent(m, { type: 'text', text: 'wort' });
  const second = reuseTurns(first, buildTurns(m, true));
  assert.equal(second.length, 2);
  assert.equal(second[0], first[0], 'der fertige Auftrag ist dasselbe Objekt');
  assert.notEqual(second[1], first[1], 'der laufende Auftrag ist neu');
  assert.equal(second[1].answer.text, 'Antwort');
  // Ende des Auftrags: «arbeitet» ändert sich, also ist der letzte Auftrag neu
  const third = reuseTurns(second, buildTurns(m, false));
  assert.equal(third[0], first[0]);
  assert.notEqual(third[1], second[1]);
});

test('Gespeicherte Hinweise und Fehler stehen nach dem Öffnen wieder im Chat; dieselbe Rückfrage nur einmal', async () => {
  const { applyEvent, buildTurns, fromTranscript, loadChat } = await load();
  const m = fromTranscript([
    { role: 'user', content: 'eins' },
    { role: 'error', text: 'ASK: kaputt' },
    { role: 'notice', text: 'Der Chat ist lang geworden: Ältere Aufträge gehen ab hier als Zusammenfassung an das Modell.' },
    { role: 'user', content: 'zwei' },
    { role: 'assistant', content: 'Antwort' },
  ]);
  const turns = buildTurns(m);
  assert.equal(turns.length, 2);
  assert.deepEqual(turns[0].after.map((e) => e.k), ['error', 'notice']);
  assert.equal(turns[0].after[0].text, 'ASK: kaputt');
  // Rückfrage kommt mit dem Verlauf und gleich nochmals als Ereignis: eine Karte, die sich beantworten lässt
  const approval = { type: 'approval', id: 'p1', title: 'Darf ASK löschen?' };
  let live = loadChat([{ role: 'user', content: 'x' }], true, [approval]);
  live = applyEvent(live, approval);
  assert.equal(live.entries.filter((e) => e.k === 'approval').length, 1);
  live = applyEvent(live, { type: 'approval-end', id: 'p1', answer: 'yes' });
  assert.equal(live.entries.find((e) => e.k === 'approval' && !e.answer), undefined);
});

test('Nebenfrage mit /btw: neben dem Auftrag, unterbricht die laufende Antwort nicht, bleibt nach dem Laden (E86)', async () => {
  const { applyEvent, buildTurns, emptyChat, fromTranscript, reuseTurns } = await load();
  let m = [
    { type: 'user', text: 'Schreib einen Bericht' },
    { type: 'assistant-start' },
    { type: 'text', text: 'Ich schreibe ' },
  ].reduce(applyEvent, emptyChat());
  m = applyEvent(m, { type: 'btw-start', id: 'b1', question: 'Was ist ein Abstract?', time: 5 });
  m = applyEvent(m, { type: 'btw-delta', id: 'b1', text: 'Eine kurze ' });
  // Die Antwort des Auftrags läuft im selben Eintrag weiter, nicht in einem neuen
  m = applyEvent(m, { type: 'text', text: 'den Bericht.' });
  assert.strictEqual(m.entries.filter((e) => e.k === 'assistant').length, 1);
  assert.strictEqual(m.entries.find((e) => e.k === 'assistant').text, 'Ich schreibe den Bericht.');
  m = applyEvent(m, { type: 'btw-delta', id: 'b1', text: 'Zusammenfassung.' });
  m = applyEvent(m, { type: 'btw-delta', id: 'unbekannt', text: 'x' });
  let turns = buildTurns(m, true);
  assert.strictEqual(turns.length, 1);
  assert.deepStrictEqual(turns[0].btw.map((b) => [b.question, b.text, b.done]), [['Was ist ein Abstract?', 'Eine kurze Zusammenfassung.', false]]);
  m = applyEvent(m, { type: 'btw-end', id: 'b1', question: 'Was ist ein Abstract?', text: 'Eine kurze Zusammenfassung.', error: '' });
  turns = buildTurns(m, true);
  assert.strictEqual(turns[0].btw[0].done, true);
  // Ende ohne Anfang (Chat inzwischen neu geladen): Frage und Antwort kommen mit dem Ende
  const fresh = applyEvent(emptyChat(), { type: 'btw-end', id: 'b2', question: 'Und?', text: 'So.', error: '' });
  assert.deepStrictEqual(fresh.btw.map((b) => [b.question, b.text, b.done]), [['Und?', 'So.', true]]);
  // Fehler
  const failed = applyEvent(applyEvent(emptyChat(), { type: 'btw-start', id: 'b3', question: 'Q' }), { type: 'btw-end', id: 'b3', error: 'Keine Antwort' });
  assert.strictEqual(failed.btw[0].error, 'Keine Antwort');
  // Gespeichert: gehört zum Auftrag, in dem sie gestellt wurde
  const saved = fromTranscript([
    { role: 'user', content: 'eins', time: 1 }, { role: 'assistant', content: 'A1', calls: [] },
    { role: 'btw', id: 'b9', question: 'nebenbei?', answer: 'ja', error: '', time: 2 },
    { role: 'user', content: 'zwei', time: 3 }, { role: 'assistant', content: 'A2', calls: [] },
  ]);
  const t2 = buildTurns(saved);
  assert.deepStrictEqual(t2.map((t) => t.btw.map((b) => b.text)), [['ja'], []]);
  assert.strictEqual(t2[0].answer.text, 'A1', 'die Nebenfrage verdrängt die Antwort nicht');
  // Unveränderte Aufträge behalten ihr Objekt, ein Auftrag mit neuer Nebenantwort nicht
  const again = reuseTurns(t2, buildTurns(applyEvent(saved, { type: 'btw-start', id: 'b10', question: 'noch was' })));
  assert.strictEqual(again[0], t2[0]);
  assert.notStrictEqual(again[1], t2[1]);
});

