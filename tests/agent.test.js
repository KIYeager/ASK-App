// Tests für die Arbeitsschleife (src/main/agent.js) mit einem Ersatz für ASK.
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Agent } = require('../src/main/agent');
const { Completion } = require('../src/main/askapi');

// answers: Funktionen (request) → Text der Antwort, { calls } oder ein Fehler, der Reihe nach
function setup(answers) {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-agent-'));
  const requests = [];
  let model = 'modell-a';
  const ask = {
    async chat(request) {
      requests.push(request);
      const next = answers.shift();
      const result = typeof next === 'function' ? await next(request) : next;
      if (result instanceof Error) throw result;
      const completion = new Completion(request.onDelta);
      completion.add({ choices: [{ delta: typeof result === 'string' ? { content: result } : result, finish_reason: 'stop' }] });
      return completion.end();
    },
  };
  const agent = new Agent({ ask, terminal: { state: 'gestoppt' }, getFolder: () => folder, getModel: () => model,
    getSystemPrompt: () => 'SYS', getCommandHint: () => '', getWebSearch: () => false });
  const events = [];
  agent.on('event', (e) => events.push(e));
  return { agent, requests, events, folder, setModel: (m) => { model = m; }, done: () => fs.rmSync(folder, { recursive: true, force: true }) };
}

const call = (name, args) => ({ tool_calls: [{ index: 0, id: 'c1', function: { name, arguments: JSON.stringify(args) } }] });

test('Hinweise und Fehler bleiben im Chat, gehen aber nicht an das Modell', async () => {
  const t = setup([Object.assign(new Error('ASK: kaputt'), { status: 500 }), 'Hallo']);
  try {
    await t.agent.send('eins');
    assert.deepStrictEqual(t.agent.transcript().map((i) => i.role), ['user', 'error']);
    await t.agent.send('zwei');
    assert.deepStrictEqual(t.requests[1].messages.map((m) => m.role), ['system', 'user', 'user']);
    assert.deepStrictEqual(t.agent.transcript().map((i) => i.role), ['user', 'error', 'user', 'assistant']);
    const saved = JSON.parse(fs.readFileSync(path.join(t.folder, '.ask', 'chats', `${t.agent.chat.id}.json`), 'utf8'));
    assert.strictEqual(saved.messages[1].role, 'note');
    assert.ok(!fs.existsSync(path.join(t.folder, '.ask', 'chats', `${t.agent.chat.id}.json.tmp`)));
  } finally {
    t.done();
  }
});

test('Gestoppte Antwort: was schon da war, bleibt im Chat', async () => {
  const t = setup([async (request) => {
    request.onDelta('text', 'Ich habe angefangen');
    const partial = new Completion();
    partial.add({ choices: [{ delta: { content: 'Ich habe angefangen' } }] });
    t.agent.stop();
    return Object.assign(new Error('abgebrochen'), { name: 'AbortError', partial: partial.end() });
  }]);
  try {
    await t.agent.send('los');
    const items = t.agent.transcript();
    assert.deepStrictEqual(items.map((i) => i.role), ['user', 'assistant', 'notice']);
    assert.strictEqual(items[1].content, 'Ich habe angefangen');
    assert.strictEqual(items[1].stopped, true);
    assert.strictEqual(items[2].text, 'Gestoppt.');
  } finally {
    t.done();
  }
});

test('Verlauf zu lang: einmal stärker gekürzt nochmals, sonst klare Meldung', async () => {
  const long = () => Object.assign(new Error("ASK: This model's maximum context length is 8192 tokens."), { status: 400 });
  const t = setup([long(), 'jetzt geht es', long()]);
  try {
    await t.agent.send('eins');
    assert.strictEqual(t.requests.length, 2);
    assert.ok(t.events.some((e) => e.type === 'notice' && /zu lang geworden/.test(e.text)));
    assert.strictEqual(t.agent.transcript().at(-1).content, 'jetzt geht es');
    await t.agent.send('zwei');
    assert.strictEqual(t.requests.length, 3);
    assert.ok(t.events.some((e) => e.type === 'error' && /zu lang für das Modell, auch gekürzt/.test(e.text)));
    assert.ok(!t.agent.textMode, 'eine Meldung zur Länge schaltet nicht in den Textmodus');
  } finally {
    t.done();
  }
});

test('Textmodus gilt für das Modell, das die Werkzeugliste abgelehnt hat', async () => {
  const t = setup([Object.assign(new Error('ASK: tools are not supported'), { status: 400 }), 'ok', 'ok']);
  try {
    await t.agent.send('eins');
    assert.strictEqual(t.agent.textMode, true);
    assert.strictEqual(t.requests[1].tools, null);
    t.setModel('modell-b');
    assert.strictEqual(t.agent.textMode, false);
    await t.agent.send('zwei');
    assert.ok(Array.isArray(t.requests[2].tools));
  } finally {
    t.done();
  }
});

test('Abgebrochene Werkzeugaufrufe gelten als fehlgeschlagen', async () => {
  const t = setup([async () => { t.agent.stop(); return call('list_files', { path: '.' }); }]);
  try {
    await t.agent.send('los');
    const tool = t.agent.transcript().find((i) => i.role === 'tool');
    assert.strictEqual(tool.ok, false);
    assert.strictEqual(tool.summary, 'abgebrochen');
  } finally {
    t.done();
  }
});
