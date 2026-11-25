// Modus im Eingabefeld (Planmodus), automatisches Wiederholen, @Datei, Rückgängig ab einem früheren Auftrag
// (src/main/agent.js, src/renderer/src/lib/chatModel.mjs)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Agent, mentionsIn, apiMessages } = require('../src/main/agent');

function fakeAsk(replies) {
  const requests = [];
  return {
    requests,
    chat: async (req) => {
      requests.push(req);
      const r = replies.shift() || { content: 'Fertig.', calls: [] };
      if (r.throw) throw r.throw;
      return { content: r.content || '', calls: r.calls || [], sources: [], reasoning: '', stats: () => ({ tokens: 1, genMs: 1, totalMs: 1, firstMs: 1 }) };
    },
  };
}

function makeAgent(ask, opts = {}) {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-modi-'));
  const agent = new Agent({
    ask,
    terminal: { state: 'gestoppt', port: 0 },
    getFolder: () => folder,
    getModel: () => 'm',
    getSystemPrompt: () => 'sys',
    getCommandHint: () => '',
    getWebSearch: () => false,
    getApprovalMode: () => 'aus',
    fileExists: (rel) => (opts.files || []).includes(rel),
    getMode: () => opts.mode || 'auto',
    retryWaits: opts.retryWaits,
    getSnapshots: opts.getSnapshots,
    confirm: async () => true,
  });
  const events = [];
  agent.on('event', (ev) => events.push(ev));
  return { agent, events, folder };
}

const call = (id, name, args) => ({ id, name, arguments: JSON.stringify(args) });

test('Planmodus: nur Lesewerkzeuge, schreiben wird abgewiesen, plan_vorlegen beendet den Auftrag', async () => {
  const ask = fakeAsk([
    { calls: [call('a', 'write_file', { path: 'x.txt', content: 'hallo' })] },
    { calls: [call('b', 'plan_vorlegen', { plan: '# Website\n1. index.html anlegen' })] },
    { content: 'sollte nicht mehr kommen' },
  ]);
  const { agent, events } = makeAgent(ask, { mode: 'plan' });
  await agent.send('Mach mir eine Website');
  const names = ask.requests[0].tools.map((t) => t.function.name);
  assert.ok(names.includes('read_file') && names.includes('plan_vorlegen'));
  assert.ok(!names.includes('write_file') && !names.includes('run_command'), 'nichts, was ändert');
  assert.match(ask.requests[0].messages[0].content, /Planmodus/);
  const ends = events.filter((e) => e.type === 'tool-end');
  assert.strictEqual(ends[0].ok, false);
  assert.match(ends[0].summary, /Planmodus/);
  assert.strictEqual(ends[1].plan, '# Website\n1. index.html anlegen');
  assert.strictEqual(ask.requests.length, 2, 'nach dem Plan keine weitere Anfrage');
  const tool = agent.transcript().find((t) => t.role === 'tool' && t.plan);
  assert.ok(tool, 'Plan bleibt im gespeicherten Chat');
});

test('Auto-Modus: alle Werkzeuge, kein Planwerkzeug', async () => {
  const ask = fakeAsk([{ content: 'ok' }]);
  const { agent } = makeAgent(ask, { mode: 'auto' });
  await agent.send('Hallo');
  const names = ask.requests[0].tools.map((t) => t.function.name);
  assert.ok(names.includes('write_file'));
  assert.ok(!names.includes('plan_vorlegen'));
});

test('Verbindung weg: wartet und versucht es nochmals, mit Hinweis', async () => {
  const netErr = new Error('net::ERR_INTERNET_DISCONNECTED');
  const ask = fakeAsk([{ throw: netErr }, { throw: Object.assign(new Error('ASK: Bad Gateway'), { status: 502 }) }, { content: 'Geschafft.' }]);
  const { agent, events } = makeAgent(ask, { retryWaits: [0.01, 0.01, 0.01] });
  await agent.send('Hallo');
  const retries = events.filter((e) => e.type === 'retry');
  assert.strictEqual(retries.length, 2);
  assert.match(retries[0].text, /Neuer Versuch/);
  assert.ok(!events.some((e) => e.type === 'error'));
  assert.strictEqual(agent.chat.messages.at(-1).content, 'Geschafft.');
});

test('Wiederholen gibt nach den Versuchen auf; Fehler vom Modell werden nicht wiederholt', async () => {
  const netErr = () => new Error('fetch failed');
  const ask = fakeAsk([{ throw: netErr() }, { throw: netErr() }, { throw: netErr() }]);
  const { agent, events } = makeAgent(ask, { retryWaits: [0.01, 0.01] });
  await agent.send('Hallo');
  assert.strictEqual(events.filter((e) => e.type === 'retry').length, 2);
  assert.ok(events.some((e) => e.type === 'error' && /fetch failed/.test(e.text)));

  const ask2 = fakeAsk([{ throw: Object.assign(new Error('ASK: model not found'), { status: 404 }) }]);
  const b = makeAgent(ask2, { retryWaits: [0.01] });
  await b.agent.send('Hallo');
  assert.strictEqual(b.events.filter((e) => e.type === 'retry').length, 0);
  assert.strictEqual(ask2.requests.length, 1);
});

test('Stoppen während des Wartens bricht ab', async () => {
  const ask = fakeAsk([{ throw: new Error('socket hang up') }]);
  const { agent, events } = makeAgent(ask, { retryWaits: [30] });
  agent.on('event', (ev) => { if (ev.type === 'retry') setImmediate(() => agent.stop()); });
  const t = Date.now();
  await agent.send('Hallo');
  assert.ok(Date.now() - t < 5000, 'wartet nicht die ganzen 30 s');
  assert.ok(events.some((e) => e.type === 'notice' && e.text === 'Gestoppt.'));
});

test('@Datei: nur vorhandene Pfade, auch in Anführungszeichen, gehen als Hinweis ans Modell', async () => {
  const files = ['src/app.js', 'Mein Bericht.docx'];
  assert.deepStrictEqual(mentionsIn('Schau @src/app.js und @"Mein Bericht.docx", nicht @fehlt.txt oder mail@x.ch', (r) => files.includes(r)),
    ['src/app.js', 'Mein Bericht.docx']);
  const ask = fakeAsk([{ content: 'ok' }]);
  const { agent } = makeAgent(ask, { files });
  await agent.send('Erklär @src/app.js.');
  assert.deepStrictEqual(agent.chat.messages[0].mentions, ['src/app.js']);
  const user = apiMessages('sys', agent.chat.messages).find((m) => m.role === 'user');
  assert.match(user.content, /Mit @ erwähnt.*src\/app\.js/);
});

test('Rückgängig bei einem früheren Auftrag markiert auch die späteren', async () => {
  let n = 0;
  const snapshots = {
    take: async () => ({ id: `s${++n}`, skipped: [] }),
    diff: async () => ({ changed: ['a.txt'], added: [], removed: [] }),
    restore: async () => ({ failed: [] }),
  };
  const ask = fakeAsk([{ content: 'eins' }, { content: 'zwei' }, { content: 'drei' }]);
  const { agent } = makeAgent(ask, { getSnapshots: () => snapshots });
  await agent.send('Erster');
  await agent.send('Zweiter');
  await agent.send('Dritter');
  const res = await agent.undo('s2');
  assert.ok(res.ok, res.reason);
  const users = agent.chat.messages.filter((m) => m.role === 'user' && m.snapshot);
  assert.deepStrictEqual(users.map((m) => !!m.undone), [false, true, true]);
});

test('Anzeige: Plan als Karte, Wiederholen ersetzt die halbe Antwort, Rückgängig gilt ab dem Auftrag', async () => {
  const { applyEvent, buildTurns, emptyChat } = await import('../src/renderer/src/lib/chatModel.mjs');
  let m = emptyChat();
  for (const ev of [
    { type: 'user', text: 'Plan bitte' },
    { type: 'text', text: 'Halb' },
    { type: 'retry', text: 'Verbindung … 1/5' },
    { type: 'retry', text: 'Verbindung … 2/5' },
    { type: 'assistant-end', content: '', calls: [] },
    { type: 'tool-start', id: 'p', name: 'plan_vorlegen', label: 'Plan', detail: 'Website' },
    { type: 'tool-end', id: 'p', ok: true, summary: 'Plan vorgelegt', output: '', plan: '1. Schritt' },
    { type: 'done' },
  ]) m = applyEvent(m, ev);
  assert.ok(!m.entries.some((e) => e.k === 'assistant'), 'halbe Antwort verworfen');
  assert.strictEqual(m.entries.filter((e) => e.retry).length, 1, 'ein einziger Hinweis');
  assert.match(m.entries.find((e) => e.retry).text, /2\/5/);
  const turn = buildTurns(m)[0];
  assert.ok(turn.after.some((e) => e.plan === '1. Schritt'), 'Plan unter der Arbeit');

  let u = emptyChat();
  for (const ev of [
    { type: 'user', text: 'a' }, { type: 'changes', snapshot: 's1', changes: { changed: ['x'], added: [], removed: [] } }, { type: 'done' },
    { type: 'user', text: 'b' }, { type: 'changes', snapshot: 's2', changes: { changed: ['y'], added: [], removed: [] } }, { type: 'done' },
    { type: 'undo', snapshot: 's1', text: 'Rückgängig gemacht' },
  ]) u = applyEvent(u, ev);
  assert.deepStrictEqual(u.entries.filter((e) => e.k === 'changes').map((e) => e.undone), [true, true]);
});
