// Hilfsagenten (src/main/helpers.js, Einbindung in agent.js)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Agent } = require('../src/main/agent');
const { runHelper, pool, HELPER_TOOLS } = require('../src/main/helpers');
const { apiMessages, textModePrompt } = require('../src/main/agent');

const stats = () => ({ tokens: 10, genMs: 1, totalMs: 1, firstMs: 1 });
const reply = (content, calls = []) => ({ content, calls, sources: [], reasoning: '', stats });
const call = (id, name, args) => ({ id, name, arguments: JSON.stringify(args) });
const isHelper = (req) => req.messages[0].content.startsWith('Du bist ein Hilfsagent');

test('Hilfsagent: nur lesende Werkzeuge, Ergebnis mit Schritten, Abbruch', async () => {
  const requests = [];
  const answers = [reply('', [call('a', 'write_file', { path: 'x.txt', content: 'x' }), call('b', 'list_files', {})]), reply('In a.txt steht die Antwort (Zeile 3).')];
  const ask = { chat: async (req) => { requests.push(req); return answers.shift(); } };
  const progress = [];
  const r = await runHelper(JSON.stringify({ titel: 'Suche', auftrag: 'Finde die Antwort' }), {
    ask, model: 'klein', terminal: { state: 'gestoppt', port: 0 }, folder: os.tmpdir(), apiMessages, textModePrompt, onProgress: (p) => progress.push(p) });
  assert.strictEqual(r.ok, true);
  assert.match(r.output, /Ergebnis des Hilfsagenten «Suche»:\nIn a\.txt steht die Antwort/);
  assert.match(r.summary, /^2 Schritte/);
  assert.match(r.display, /Schritte:\n✗ .*nicht erlaubt/);
  assert.strictEqual(requests[0].model, 'klein');
  assert.deepStrictEqual(requests[0].tools.map((t) => t.function.name).sort(), [...HELPER_TOOLS].sort());
  assert.ok(!requests[0].tools.some((t) => t.function.name === 'hilfsagent'), 'keine Hilfsagenten im Hilfsagenten');
  const toolMsg = requests[1].messages.find((m) => m.role === 'tool');
  assert.match(toolMsg.content, /nur diese Werkzeuge/);
  assert.ok(progress.length >= 2);

  const stopped = new AbortController();
  stopped.abort();
  const s = await runHelper(JSON.stringify({ auftrag: 'x' }), { ask, model: 'm', signal: stopped.signal, apiMessages, textModePrompt });
  assert.strictEqual(s.summary, 'abgebrochen');
  assert.strictEqual((await runHelper('{}', { ask, model: 'm', apiMessages, textModePrompt })).ok, false);
});

test('pool: höchstens limit gleichzeitig, Reihenfolge bleibt', async () => {
  let running = 0;
  let most = 0;
  const tasks = [30, 5, 20, 1, 10].map((ms, i) => async () => {
    running++; most = Math.max(most, running);
    await new Promise((r) => setTimeout(r, ms));
    running--;
    return i;
  });
  assert.deepStrictEqual(await pool(tasks, 3), [0, 1, 2, 3, 4]);
  assert.strictEqual(most, 3);
});

test('Agent: vier Hilfsagenten in einem Schritt, drei gleichzeitig, eigenes Modell, Ergebnisse in Reihenfolge', async () => {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-hilfe-'));
  let running = 0;
  let most = 0;
  const main = [reply('', [1, 2, 3, 4].map((n) => call(`h${n}`, 'hilfsagent', { titel: `Teil ${n}`, auftrag: `Teil ${n} klären` }))), reply('Alles zusammengetragen.')];
  const requests = [];
  const ask = {
    chat: async (req) => {
      requests.push(req);
      if (!isHelper(req)) return main.shift();
      running++; most = Math.max(most, running);
      await new Promise((r) => setTimeout(r, 20));
      running--;
      return reply(`Antwort zu ${req.messages.at(-1).content}`);
    },
  };
  const agent = new Agent({
    ask, terminal: { state: 'gestoppt', port: 0 }, getFolder: () => folder, getModel: () => 'gross', getHelperModel: () => 'klein',
    getSystemPrompt: () => 'sys', getCommandHint: () => '', getWebSearch: () => false, getApprovalMode: () => 'heikel', fileExists: () => false,
  });
  const events = [];
  agent.on('event', (e) => events.push(e));
  await agent.send('Finde alles heraus');
  assert.strictEqual(most, 3, 'höchstens drei gleichzeitig');
  assert.ok(requests.filter(isHelper).every((r) => r.model === 'klein'));
  assert.ok(requests.filter((r) => !isHelper(r)).every((r) => r.model === 'gross'));
  assert.ok(requests[0].tools.some((t) => t.function.name === 'hilfsagent'), 'Werkzeug angeboten');
  const last = requests.filter((r) => !isHelper(r)).at(-1);
  const results = last.messages.filter((m) => m.role === 'tool');
  assert.deepStrictEqual(results.map((m) => m.tool_call_id), ['h1', 'h2', 'h3', 'h4']);
  assert.match(results[2].content, /Antwort zu Teil 3 klären/);
  assert.strictEqual(events.filter((e) => e.type === 'tool-start').length, 4, 'jeder Hilfsagent einmal gestartet');
  assert.ok(events.some((e) => e.type === 'tool-progress' && e.id === 'h1'));
  assert.ok(!events.some((e) => e.type === 'approval'), 'keine Rückfrage');
});

test('Agent: Hilfsagenten auch im Planmodus; ohne eigenes Modell das Modell des Chats', async () => {
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-hilfe-'));
  const main = [reply('', [call('h', 'hilfsagent', { titel: 't', auftrag: 'a' })]), reply('Ok.')];
  const requests = [];
  const ask = { chat: async (req) => { requests.push(req); return isHelper(req) ? reply('Gefunden.') : main.shift(); } };
  const agent = new Agent({
    ask, terminal: { state: 'gestoppt', port: 0 }, getFolder: () => folder, getModel: () => 'gross', getMode: () => 'plan',
    getSystemPrompt: () => 'sys', getCommandHint: () => '', getWebSearch: () => false, getApprovalMode: () => 'heikel', fileExists: () => false,
  });
  await agent.send('Plane das');
  assert.ok(requests[0].tools.some((t) => t.function.name === 'hilfsagent'));
  const helper = requests.find(isHelper);
  assert.strictEqual(helper.model, 'gross');
  const result = requests.at(-1).messages.find((m) => m.role === 'tool');
  assert.match(result.content, /Gefunden\./);
});
