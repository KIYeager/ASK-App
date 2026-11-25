// Rückfrage vor heiklen Schritten und Fragen des Modells (src/main/agent.js, approvals.js)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Agent } = require('../src/main/agent');

// ASK-Ersatz: liefert nacheinander die vorgegebenen Antworten
function fakeAsk(replies) {
  const requests = [];
  return {
    requests,
    chat: async (req) => {
      requests.push(req);
      const r = replies.shift() || { content: 'Fertig.', calls: [] };
      return { content: r.content || '', calls: r.calls || [], sources: [], reasoning: '', stats: () => ({ tokens: 1, genMs: 1, totalMs: 1, firstMs: 1 }) };
    },
  };
}

const del = (id) => ({ id, name: 'run_command', arguments: JSON.stringify({ command: 'del alt.txt' }) });

function makeAgent(ask, mode = 'heikel') {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-agent-'));
  const agent = new Agent({
    ask,
    terminal: { state: 'gestoppt', port: 0 }, // Befehle scheitern, das genügt hier
    getFolder: () => folder,
    getModel: () => 'm',
    getSystemPrompt: () => 'sys',
    getCommandHint: () => '',
    getWebSearch: () => false,
    getApprovalMode: () => mode,
    fileExists: () => false,
  });
  const events = [];
  agent.on('event', (ev) => {
    events.push(ev);
    if (ev.type === 'approval' && agent.answer) setImmediate(() => agent.answerApproval(ev.id, agent.answer.shift()));
  });
  return { agent, events, folder };
}

test('Abgelehnter Löschbefehl wird nicht ausgeführt, das Modell erfährt es', async () => {
  const ask = fakeAsk([{ calls: [del('a')] }]);
  const { agent, events } = makeAgent(ask);
  agent.answer = ['no'];
  await agent.send('Räum auf');
  const approval = events.find((e) => e.type === 'approval');
  assert.ok(approval, 'Rückfrage gestellt');
  assert.strictEqual(approval.kind, 'loeschen');
  assert.deepStrictEqual(events.find((e) => e.type === 'approval-end'), { type: 'approval-end', id: approval.id, answer: 'no' });
  const end = events.find((e) => e.type === 'tool-end');
  assert.strictEqual(end.ok, false);
  assert.strictEqual(end.summary, 'abgelehnt');
  const toolMsg = ask.requests[1].messages.find((m) => m.role === 'tool');
  assert.match(toolMsg.content, /abgelehnt/);
});

test('«Immer in diesem Chat» fragt danach nicht mehr, ein neuer Chat wieder', async () => {
  const ask = fakeAsk([{ calls: [del('a')] }, { calls: [del('b')] }]);
  const { agent, events } = makeAgent(ask);
  agent.answer = ['always'];
  await agent.send('Räum auf');
  assert.strictEqual(events.filter((e) => e.type === 'approval').length, 1);
  const ends = events.filter((e) => e.type === 'tool-end');
  assert.strictEqual(ends.length, 2);
  assert.ok(ends.every((e) => e.summary !== 'abgelehnt'), 'beide ausgeführt (das Terminal läuft hier nicht)');
  agent.newChat();
  assert.strictEqual(agent.allowed.size, 0);
});

test('Stoppen während der Rückfrage gilt als Nein', async () => {
  const ask = fakeAsk([{ calls: [del('a')] }]);
  const { agent, events } = makeAgent(ask);
  agent.on('event', (ev) => { if (ev.type === 'approval') setImmediate(() => agent.stop()); });
  await agent.send('Räum auf');
  assert.strictEqual(events.find((e) => e.type === 'approval-end').answer, 'no');
  assert.ok(events.some((e) => e.type === 'notice' && e.text === 'Gestoppt.'));
  assert.strictEqual(agent.running, false);
});

test('Stufe «aus» fragt nie', async () => {
  const ask = fakeAsk([{ calls: [del('a')] }]);
  const { agent, events } = makeAgent(ask, 'aus');
  await agent.send('Räum auf');
  assert.ok(!events.some((e) => e.type === 'approval'));
});

test('frage_stellen: Frage mit Antworten, die Antwort geht an das Modell', async () => {
  const ask = fakeAsk([{ calls: [{ id: 'q1', name: 'frage_stellen', arguments: JSON.stringify({ frage: 'Welche Datei?', optionen: ['a.csv', 'b.csv', ''] }) }] }]);
  const { agent, events } = makeAgent(ask);
  agent.on('event', (ev) => { if (ev.type === 'question') setImmediate(() => agent.answerQuestion(ev.id, 'b.csv')); });
  await agent.send('Werte aus');
  const q = events.find((e) => e.type === 'question');
  assert.deepStrictEqual({ frage: q.frage, optionen: q.optionen }, { frage: 'Welche Datei?', optionen: ['a.csv', 'b.csv'] });
  assert.strictEqual(events.find((e) => e.type === 'question-end').answer, 'b.csv');
  const tool = ask.requests[1].messages.find((m) => m.role === 'tool');
  assert.strictEqual(tool.content, 'Antwort der Person: b.csv');
  assert.ok(ask.requests[0].tools.some((t) => t.function.name === 'frage_stellen'));
  const call = agent.transcript().find((m) => m.role === 'assistant').calls[0];
  assert.deepStrictEqual([call.label, call.detail, call.optionen], ['Frage', 'Welche Datei?', ['a.csv', 'b.csv']]);
});

test('frage_stellen: Stoppen beendet die Frage ohne Antwort', async () => {
  const ask = fakeAsk([{ calls: [{ id: 'q1', name: 'frage_stellen', arguments: '{"frage":"Weiter?"}' }] }]);
  const { agent, events } = makeAgent(ask);
  agent.on('event', (ev) => { if (ev.type === 'question') setImmediate(() => agent.stop()); });
  await agent.send('x');
  assert.strictEqual(events.find((e) => e.type === 'question-end').answer, '');
  assert.strictEqual(events.find((e) => e.type === 'tool-end').summary, 'nicht beantwortet');
  assert.strictEqual(agent.questions.size, 0);
});

test('Argumente «null» bringen den Chat nicht durcheinander: jeder Aufruf bekommt ein Ergebnis', async () => {
  const ask = fakeAsk([{ calls: [{ id: 'n', name: 'run_command', arguments: 'null' }, { id: 'q', name: 'frage_stellen', arguments: '[1]' }] }]);
  const { agent } = makeAgent(ask);
  await agent.send('Los');
  const tools = ask.requests[1].messages.filter((m) => m.role === 'tool').map((m) => m.tool_call_id);
  assert.deepStrictEqual(tools, ['n', 'q']);
  const calls = ask.requests[1].messages.find((m) => m.tool_calls).tool_calls.map((c) => c.function.arguments);
  assert.deepStrictEqual(calls, ['{}', '{}']);
});

test('Gleicher Aufruf mit neuer Ausgabe ist keine Schleife, gleicher Aufruf mit gleichem Ergebnis schon', () => {
  const { loopReason } = require('../src/main/agent');
  const turn = (outputs) => [{ role: 'user', content: 'x' }, ...outputs.flatMap((o, i) => [
    { role: 'assistant', tool_calls: [{ id: `c${i}`, function: { name: 'command_status', arguments: '{"id":"1"}' } }] },
    { role: 'tool', tool_call_id: `c${i}`, content: o }])];
  assert.strictEqual(loopReason(turn(['a', 'ab', 'abc'])), '');
  assert.match(loopReason(turn(['a', 'a', 'a'])), /denselben Schritt/);
});

test('Während «Eingabe ändern» startet kein Auftrag und kein zweites Zurücknehmen', async () => {
  const ask = fakeAsk([{ content: 'Ok.' }]);
  const { agent } = makeAgent(ask);
  await agent.send('Erste Eingabe');
  let release;
  agent.chat.messages.find((m) => m.role === 'user').snapshot = 's1';
  agent.getSnapshots = () => ({ diff: async () => ({ changed: ['a.txt'], added: [], removed: [] }), restore: async () => ({ failed: [] }) });
  agent.confirm = () => new Promise((r) => { release = r; });
  const first = agent.rewind();
  await new Promise((r) => setImmediate(r));
  assert.strictEqual(agent.undoing, true);
  assert.strictEqual((await agent.rewind()).ok, false);
  await agent.send('Dazwischen');
  assert.strictEqual(ask.requests.length, 1, 'kein neuer Auftrag während des Zurücknehmens');
  release(true);
  assert.strictEqual((await first).ok, true);
  assert.strictEqual(agent.undoing, false);
});

test('Arbeitet ein anderer Chat im selben Ordner, warnt die App', async () => {
  const ask = fakeAsk([{ content: 'Ok.' }]);
  const { agent, events } = makeAgent(ask);
  agent.getSiblings = () => ['Bericht schreiben'];
  await agent.send('Los');
  assert.ok(events.some((e) => e.type === 'notice' && /«Bericht schreiben»/.test(e.text)));
});
