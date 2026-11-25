// Tests für das Lesen der Antworten von ASK und den Verlauf an das Modell (node --test tests/).
const test = require('node:test');
const assert = require('node:assert');
const { AskSession, SseParser, ThinkSplitter, Completion, parseTextToolCalls, parseSources, webSearchAllowed, isStaleTerminal, nearestEffort } = require('../src/main/askapi');
const { apiMessages, shortArgs, TIGHT, TOO_LONG } = require('../src/main/agent');
const { parseArgs, clip, toolSpecs } = require('../src/main/tools');

test('SSE: Ereignisse über Teilstücke hinweg, [DONE] und Kommentare ignoriert', () => {
  const events = [];
  const p = new SseParser((e) => events.push(e));
  p.push('data: {"a":');
  p.push('1}\n\n: kommentar\ndata: [DONE]\r\n');
  p.push('data: {"b":2}\n');
  assert.deepStrictEqual(events, [{ a: 1 }, { b: 2 }]);
});

test('<think> wird getrennt, auch wenn die Marke geteilt ankommt', () => {
  const parts = { text: '', reasoning: '' };
  const s = new ThinkSplitter((kind, t) => { parts[kind] += t; });
  for (const piece of ['<thi', 'nk>ich ', 'denke</th', 'ink>Antwort <', 'b>']) s.push(piece);
  s.flush();
  assert.deepStrictEqual(parts, { text: 'Antwort <b>', reasoning: 'ich denke' });
});

test('Werkzeugaufrufe aus gestreamten Teilen zusammensetzen', () => {
  const c = new Completion();
  c.add({ choices: [{ delta: { tool_calls: [{ index: 0, id: 'a', function: { name: 'run_command', arguments: '' } }] } }] });
  c.add({ choices: [{ delta: { tool_calls: [{ index: 0, function: { arguments: '{"command":' } }] } }] });
  c.add({ choices: [{ delta: { tool_calls: [{ index: 0, function: { arguments: '"dir"}' } }, { index: 1, id: 'b', function: { name: 'read_file', arguments: '{"path":"x"}' } }] } }] });
  c.add({ choices: [{ delta: { reasoning_content: 'hm' }, finish_reason: 'tool_calls' }] });
  c.end();
  assert.deepStrictEqual(c.calls, [{ id: 'a', name: 'run_command', arguments: '{"command":"dir"}' }, { id: 'b', name: 'read_file', arguments: '{"path":"x"}' }]);
  assert.strictEqual(c.reasoning, 'hm');
  assert.strictEqual(c.finish, 'tool_calls');
});

test('Ganze Antwort ohne Streaming und Fehlermeldung', () => {
  const c = new Completion();
  c.add({ choices: [{ message: { content: 'Hallo', tool_calls: [{ id: 'x', function: { name: 'list_files', arguments: {} } }] } }] });
  c.end();
  assert.strictEqual(c.content, 'Hallo');
  assert.deepStrictEqual(c.calls, [{ id: 'x', name: 'list_files', arguments: '{}' }]);
  const e = new Completion();
  e.add({ error: { message: 'Modell nicht gefunden' } });
  assert.strictEqual(e.error, 'Modell nicht gefunden');
});

test('Tokenzahlen und Zeiten: gemessen oder geschätzt', () => {
  let t = 1000;
  const c = new Completion(() => {}, () => t);
  t = 1800; c.add({ choices: [{ delta: { reasoning_content: 'a'.repeat(36) } }] });
  t = 3800; c.add({ choices: [{ delta: { content: 'b'.repeat(36) } }] });
  t = 3900; c.add({ choices: [], usage: { prompt_tokens: 500, completion_tokens: 40 } });
  c.end();
  assert.deepStrictEqual(c.stats(), { tokens: 40, estimated: false, promptTokens: 500, genMs: 2000, firstMs: 800, totalMs: 2800 });
  const e = new Completion(() => {}, () => t);
  e.add({ choices: [{ delta: { content: 'x'.repeat(72) } }] });
  e.end();
  assert.strictEqual(e.stats().tokens, 20);
  assert.strictEqual(e.stats().estimated, true);
});

test('Websuche: Freigabe und Quellen aus ASK', () => {
  assert.strictEqual(webSearchAllowed({ features: { enable_web_search: true } }, { role: 'user', permissions: { features: { web_search: true } } }), true);
  assert.strictEqual(webSearchAllowed({ features: { enable_web_search: true } }, { role: 'user', permissions: { features: { web_search: false } } }), false);
  assert.strictEqual(webSearchAllowed({ features: { enable_web_search: false } }, { role: 'admin' }), false);
  assert.strictEqual(webSearchAllowed({ features: { enable_web_search: true } }, { role: 'admin', permissions: { features: { web_search: false } } }), true);
  const sources = parseSources([
    { source: { type: 'web_search' }, metadata: [{ source: 'https://a.ch/x', title: 'A' }, { source: 'https://a.ch/x', title: 'A doppelt' }, { source: 'datei.pdf' }] },
    { source: { urls: ['https://www.b.ch/seite'] } },
  ]);
  assert.deepStrictEqual(sources, [{ url: 'https://a.ch/x', title: 'A' }, { url: 'https://www.b.ch/seite', title: 'b.ch' }]);
  const c = new Completion();
  c.add({ sources: [{ metadata: [{ source: 'https://a.ch/x', title: 'A' }] }] });
  c.add({ sources: [{ metadata: [{ source: 'https://a.ch/x', title: 'A' }] }] });
  assert.strictEqual(c.sources.length, 1);
});

test('Alter Terminal-Eintrag: nur genau dieser gilt als alt', () => {
  const old = { name: 'ASK Desktop', key: 'ask-desktop', url: 'http://127.0.0.1:30853' };
  assert.strictEqual(isStaleTerminal(old), true);
  assert.strictEqual(isStaleTerminal({ ...old, key: 'echter-key' }), false);
  assert.strictEqual(isStaleTerminal({ ...old, name: 'Mein Terminal' }), false);
  assert.strictEqual(isStaleTerminal({ ...old, url: 'https://terminal.example.ch' }), false);
  assert.strictEqual(isStaleTerminal(null), false);
});

// AskSession mit nachgebauter ASK-Schnittstelle; zeichnet die Schreibzugriffe auf.
function cleanerWith(settingsResponse) {
  const ask = new AskSession();
  ask.base = 'https://ask.example.ch';
  const writes = [];
  ask.session = {
    fetch: async (url, options = {}) => {
      if (url.endsWith('/settings/update')) {
        writes.push(JSON.parse(options.body));
        return { ok: true };
      }
      return settingsResponse();
    },
  };
  return { ask, writes };
}

test('Alter Terminal-Eintrag: wird einmal entfernt, Anderes bleibt', async () => {
  const mine = { name: 'Mein Terminal', key: 'k', url: 'http://127.0.0.1:8000', enabled: false };
  const servers = [mine, { name: 'ASK Desktop', key: 'ask-desktop', url: 'http://127.0.0.1:30853', enabled: true }];
  const { ask, writes } = cleanerWith(() => ({ ok: true, json: async () => ({ ui: { terminalServers: servers } }) }));
  await ask.cleanStaleTerminal('token');
  await ask.cleanStaleTerminal('token');
  assert.deepStrictEqual(writes, [{ ui: { terminalServers: [mine] } }]);
});

test('Alter Terminal-Eintrag: ohne Eintrag oder bei Fehlern kein Schreiben, Anmeldung bleibt', async () => {
  const none = cleanerWith(() => ({ ok: true, json: async () => ({ ui: { terminalServers: [{ name: 'Mein Terminal', url: 'http://127.0.0.1:8000' }] } }) }));
  await none.ask.cleanStaleTerminal('token');
  assert.deepStrictEqual(none.writes, []);
  const empty = cleanerWith(() => ({ ok: true, json: async () => ({}) }));
  await empty.ask.cleanStaleTerminal('token');
  assert.deepStrictEqual(empty.writes, []);
  const denied = cleanerWith(() => ({ ok: false, status: 403 }));
  denied.ask.token = 'token';
  await denied.ask.cleanStaleTerminal('token');
  assert.deepStrictEqual(denied.writes, []);
  assert.strictEqual(denied.ask.token, 'token');
  const broken = cleanerWith(() => { throw new Error('offline'); });
  await broken.ask.cleanStaleTerminal('token');
  assert.deepStrictEqual(broken.writes, []);
});

test('Werkzeugaufrufe im Text (Qwen-Format)', () => {
  const r = parseTextToolCalls('Los geht es.\n<tool_call>\n{"name": "run_command", "arguments": {"command": "dir"}}\n</tool_call>\n<tool_call>kaputt</tool_call>');
  assert.deepStrictEqual(r.calls, [{ id: '', name: 'run_command', arguments: '{"command":"dir"}' }]);
  assert.strictEqual(r.text, 'Los geht es.\n\n<tool_call>kaputt</tool_call>');
});

test('Verlauf: ohne Denktext, alte Ergebnisse gekürzt, Textmodus als Text', () => {
  const big = 'x'.repeat(5000);
  const messages = [{ role: 'user', content: 'los' }];
  for (let i = 0; i < 10; i++) {
    messages.push({ role: 'assistant', content: '', reasoning: 'geheim', tool_calls: [{ id: `c${i}`, type: 'function', function: { name: 'read_file', arguments: '{}' } }] });
    messages.push({ role: 'tool', tool_call_id: `c${i}`, content: big });
  }
  messages.push({ role: 'assistant', content: 'a', textMode: true, raw: 'a<tool_call>{}</tool_call>' });
  messages.push({ role: 'tool', tool_call_id: 't', content: 'ok', textMode: true });
  const out = apiMessages('SYS', messages);
  assert.strictEqual(out[0].content, 'SYS');
  assert.ok(out.every((m) => !('reasoning' in m)));
  const tools = out.filter((m) => m.role === 'tool');
  assert.ok(tools[0].content.length < 2000, 'älteste Ergebnisse gekürzt');
  assert.strictEqual(tools[tools.length - 1].content.length, 5000, 'neueste Ergebnisse ganz');
  assert.deepStrictEqual(out.slice(-2), [{ role: 'assistant', content: 'a<tool_call>{}</tool_call>' }, { role: 'user', content: '<tool_response>\nok\n</tool_response>' }]);
});

test('Textmodus wandelt auch frühere Werkzeugaufrufe in Text um', () => {
  const out = apiMessages('SYS', [
    { role: 'user', content: 'los' },
    { role: 'assistant', content: 'Ich schaue.', tool_calls: [{ id: 'c1', type: 'function', function: { name: 'list_files', arguments: '{"path":"."}' } }] },
    { role: 'tool', tool_call_id: 'c1', content: 'a.txt' },
  ], true);
  assert.deepStrictEqual(out.map((m) => m.role), ['system', 'user', 'assistant', 'user']);
  assert.strictEqual(out[2].content, 'Ich schaue.\n<tool_call>\n{"name": "list_files", "arguments": {"path":"."}}\n</tool_call>');
  assert.ok(!('tool_calls' in out[2]));
  assert.strictEqual(out[3].content, '<tool_response>\na.txt\n</tool_response>');
});

test('Werkzeuge: Argumente, Kürzen, Beschreibungen', () => {
  assert.deepStrictEqual(parseArgs('{"a":1}'), { a: 1 });
  assert.deepStrictEqual(parseArgs(''), {});
  assert.throws(() => parseArgs('{kaputt'), /kein gültiges JSON/);
  const c = clip('a'.repeat(100) + 'b'.repeat(100), 50);
  assert.ok(c.startsWith('a'.repeat(15)) && c.endsWith('b'.repeat(35)) && c.includes('150 Zeichen ausgelassen'));
  const specs = toolSpecs(' Hinweis');
  assert.ok(specs.find((s) => s.function.name === 'run_command').function.description.endsWith(' Hinweis'));
  assert.ok(specs.every((s) => s.type === 'function' && s.function.parameters.type === 'object'));
});

test('Verlauf: lange Argumente früherer Aufrufe werden gekürzt, Hinweise gehen nicht an das Modell', () => {
  const big = 'x'.repeat(5000);
  const messages = [{ role: 'user', content: 'los' }];
  for (let n = 0; n < 12; n++) {
    messages.push({ role: 'assistant', content: '', tool_calls: [{ id: `c${n}`, type: 'function', function: { name: 'write_file', arguments: JSON.stringify({ path: `d${n}.txt`, content: big }) } }] });
    messages.push({ role: 'note', kind: 'notice', content: 'Hinweis für die Person' });
    messages.push({ role: 'tool', tool_call_id: `c${n}`, content: `Geschrieben ${big}` });
  }
  const out = apiMessages('SYS', messages);
  assert.ok(out.every((m) => ['system', 'user', 'assistant', 'tool'].includes(m.role)));
  const calls = out.filter((m) => m.tool_calls).map((m) => m.tool_calls[0].function.arguments);
  assert.strictEqual(calls.length, 12);
  assert.ok(calls[0].length < 1500 && JSON.parse(calls[0]).path === 'd0.txt', 'alter Aufruf gekürzt, bleibt gültiges JSON');
  assert.ok(JSON.parse(calls[0]).content.includes('gekürzt'));
  assert.strictEqual(JSON.parse(calls[11]).content, big, 'neuer Aufruf unverändert');
  assert.strictEqual(messages[1].tool_calls[0].function.arguments.length > 5000, true, 'der gespeicherte Chat bleibt vollständig');
  const size = (list) => JSON.stringify(list).length;
  assert.ok(size(apiMessages('SYS', messages, false, null, TIGHT)) < size(out) / 2);
  // Textmodus: frühere Aufrufe ebenfalls gekürzt
  const text = apiMessages('SYS', messages, true);
  assert.ok(text[1].role === 'user' && text[2].content.includes('<tool_call>') && text[2].content.length < 1500);
  assert.strictEqual(shortArgs('kein json '.repeat(500), 100).length, 5000);
  assert.strictEqual(shortArgs('{"a":"kurz"}', 100), '{"a":"kurz"}');
});

test('Meldungen «Verlauf zu lang» werden erkannt, andere nicht', () => {
  for (const m of ["This model's maximum context length is 32768 tokens. However, your request has 40211 input tokens.", 'context_length_exceeded',
    'prompt is too long: 210000 tokens > 200000 maximum', 'Please reduce the length of the messages.']) assert.ok(TOO_LONG.test(m), m);
  for (const m of ['tools is not supported', 'model does not support image input', 'Model not found']) assert.ok(!TOO_LONG.test(m), m);
});

test('Denkaufwand: abgelehnte Stufe durch die nächste erlaubte ersetzen', async () => {
  const msg = 'litellm.BadRequestError: OpenAIException - Unexpected reasoning effort high. Supported types are xhigh (default), medium, and low.';
  assert.strictEqual(nearestEffort(msg, 'high'), 'xhigh', 'bei Gleichstand die höhere');
  assert.strictEqual(nearestEffort(msg, 'low'), null, 'erlaubt: nichts ändern');
  assert.strictEqual(nearestEffort('Unrecognized request argument: reasoning_effort', 'high'), null, 'ohne Liste: keine Wahl');

  const ask = new AskSession();
  ask.base = 'https://ask.example.ch';
  ask.token = 't';
  const sent = [];
  const reply = (status, body) => ({ ok: status < 300, status, headers: { get: () => 'application/json' }, text: async () => JSON.stringify(body), json: async () => body });
  ask.session = {
    fetch: async (_url, options) => {
      const body = JSON.parse(options.body);
      sent.push(body.reasoning_effort);
      if (body.reasoning_effort === 'high') return reply(400, { detail: msg });
      return reply(200, { choices: [{ message: { content: 'ok' }, finish_reason: 'stop' }] });
    },
  };
  const first = await ask.chat({ model: 'm', messages: [], thinking: 'hoch' });
  assert.strictEqual(first.content, 'ok');
  await ask.chat({ model: 'm', messages: [], thinking: 'hoch' });
  assert.deepStrictEqual(sent, ['high', 'xhigh', 'xhigh'], 'einmal abgelehnt, danach gleich die erlaubte Stufe');
});

// Nachgebaute ASK-Seite (WebContents) für check()
function fakePage({ url = 'https://ask.example.ch/auth', read }) {
  const handlers = {};
  const page = { url, loads: [] };
  const wc = {
    on: (name, fn) => { handlers[name] = fn; },
    isDestroyed: () => false, isLoading: () => false, getURL: () => page.url,
    executeJavaScript: async () => read(),
    loadURL: async (u) => { page.loads.push(u); },
  };
  const ask = new AskSession();
  ask.setBase('https://ask.example.ch');
  ask.attach({ webContents: { ...wc, session: {} } });
  clearInterval(ask.timer);
  return { ask, handlers, page };
}

test('ASK-Seite lädt nicht: kein «Fehler», Status bleibt, damit der Anmeldebereich «nicht erreichbar» zeigt', async () => {
  const { ask, handlers } = fakePage({ read: () => { throw new Error('Script failed to execute'); } });
  const seen = [];
  ask.on('status', (s) => seen.push(s.state));
  handlers['did-fail-load']({}, -105, 'ERR_NAME_NOT_RESOLVED', 'https://ask.example.ch/', true);
  assert.strictEqual((await ask.check(true)).state, 'unbekannt');
  // auch ohne Meldung des Ladefehlers: Token nicht lesbar ist kein Fehler von ASK
  handlers['did-start-navigation']({ isMainFrame: true });
  assert.strictEqual((await ask.check(true)).state, 'unbekannt');
  assert.ok(!seen.includes('fehler') && !seen.includes('nicht-angemeldet'), seen.join());
});

test('ASK-Seite geladen, ohne Token: nicht angemeldet', async () => {
  const { ask } = fakePage({ read: () => '' });
  assert.strictEqual((await ask.check(true)).state, 'nicht-angemeldet');
});

test('Ladefehler steht im Status (auch wenn die Oberfläche das Ereignis verpasst) und verschwindet nach dem Laden', async () => {
  const { ask, handlers } = fakePage({ read: () => '' });
  handlers['did-fail-load']({}, -202, 'ERR_CERT_AUTHORITY_INVALID', 'https://ask.example.ch/', true);
  assert.deepStrictEqual(ask.status, { state: 'unbekannt', loadError: { code: -202, description: 'ERR_CERT_AUTHORITY_INVALID', url: 'https://ask.example.ch/' } });
  handlers['did-finish-load'](); // Electron: auch die Fehlerseite meldet «fertig geladen»; der Fehler bleibt
  assert.strictEqual(ask.status.loadError?.code, -202);
  handlers['did-fail-load']({}, -3, 'ERR_ABORTED', 'https://ask.example.ch/', true); // Abbruch ist kein Fehler
  handlers['did-start-navigation']({ isMainFrame: true, isSameDocument: false });
  handlers['did-finish-load']();
  await new Promise((r) => setTimeout(r, 0));
  assert.strictEqual(ask.status.loadError, undefined);
  assert.strictEqual(ask.status.state, 'nicht-angemeldet');
});

test('Nach einem Ladefehler lädt die Seite nach 15 s von selbst nochmals', async () => {
  const { ask, handlers, page } = fakePage({ read: () => '' });
  handlers['did-fail-load']({}, -106, 'ERR_INTERNET_DISCONNECTED', 'https://ask.example.ch/', true);
  ask.tick();
  assert.deepStrictEqual(page.loads, [], 'nicht sofort');
  ask.failedAt -= 16000;
  ask.tick();
  assert.deepStrictEqual(page.loads, ['https://ask.example.ch']);
});

test('Seite fertig, aber nicht auf ASK (z. B. Microsoft-Anmeldung): zeigen statt warten', async () => {
  const { ask } = fakePage({ url: 'https://login.microsoftonline.com/abc/oauth2/authorize', read: () => { throw new Error('nie gelesen'); } });
  assert.strictEqual((await ask.check(true)).state, 'nicht-angemeldet');
});

test('Token nicht lesbar: Status bleibt, Grund steht im Status', async () => {
  const { ask } = fakePage({ read: () => { throw new Error('Script failed to execute'); } });
  const st = await ask.check(true);
  assert.strictEqual(st.state, 'unbekannt');
  assert.match(st.detail, /Script failed to execute/);
});
