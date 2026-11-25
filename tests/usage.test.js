const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { summarizeChat, folderUsage, aggregate, askUsage, usageNumbers, dayKey } = require('../src/main/usage');

const DAY = 86400000;
const NOW = new Date(2026, 10, 25, 15, 0, 0).getTime();

const stats = (tokens, totalMs, estimated = false) => ({ tokens, genMs: totalMs - 500, totalMs, firstMs: 500, promptTokens: 4000, estimated });

function chatA() {
  return { id: 'a', title: 'Auswertung', created: NOW - 3 * DAY, updated: NOW - 2 * DAY, messages: [
    { role: 'user', content: 'Werte aus', time: NOW - 3 * DAY, changes: { changed: ['x.docx'], added: ['a.py', 'b.xlsx'], removed: [], counts: [1, 2, 0] } },
    { role: 'assistant', content: '', time: NOW - 3 * DAY + 1000, stats: stats(200, 4000), tool_calls: [{}, {}] },
    { role: 'tool', content: 'ok' },
    { role: 'tool', content: 'ok' },
    { role: 'assistant', content: 'Fertig', time: NOW - 3 * DAY + 9000, stats: stats(100, 2000, true) },
    { role: 'user', auto: true, content: 'Weiter am Ziel', time: NOW - 3 * DAY + 10000 },
    { role: 'user', content: 'Nochmals', time: NOW - 2 * DAY, undone: true, changes: { changed: [], added: ['c.txt'], removed: [], counts: [0, 1, 0] } },
    { role: 'assistant', content: 'Gut', time: NOW - 2 * DAY + 500, stats: stats(50, 1000) },
    { role: 'note', kind: 'notice', content: 'Hinweis' },
  ] };
}

function folderWith(chats) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-usage-'));
  const dir = path.join(root, '.ask', 'chats');
  fs.mkdirSync(dir, { recursive: true });
  for (const c of chats) fs.writeFileSync(path.join(dir, `${c.id}.json`), JSON.stringify(c));
  return root;
}

test('Nutzung: ein Chat ergibt Aufträge, Schritte, Token, Zeit und Dateien', () => {
  const s = summarizeChat(chatA());
  assert.strictEqual(s.chats, 1);
  assert.strictEqual(s.tasks, 2, 'automatische Nachrichten zählen nicht');
  assert.strictEqual(s.steps, 2);
  assert.strictEqual(s.tokens, 350);
  assert.strictEqual(s.estimatedTokens, 100);
  assert.strictEqual(s.modelMs, 7000);
  assert.deepStrictEqual(s.files, { added: 2, changed: 1, removed: 0 }, 'rückgängig gemachter Auftrag zählt nicht');
  assert.deepStrictEqual(s.days[dayKey(NOW - 3 * DAY)], { tasks: 1, tokens: 300 });
  assert.deepStrictEqual(s.days[dayKey(NOW - 2 * DAY)], { tasks: 1, tokens: 50 });
});

test('Nutzung: ohne counts zählen die Listen, ohne Zeit das Datum des Chats', () => {
  const s = summarizeChat({ id: 'b', updated: NOW, messages: [
    { role: 'user', content: 'x', changes: { changed: ['a'], added: ['b', 'c'], removed: ['d'] } },
    { role: 'assistant', stats: { tokens: 'kaputt', totalMs: -5 } },
  ] });
  assert.deepStrictEqual(s.files, { added: 2, changed: 1, removed: 1 });
  assert.strictEqual(s.tokens, 0);
  assert.strictEqual(s.modelMs, 0);
  assert.deepStrictEqual(s.days[dayKey(NOW)], { tasks: 1, tokens: 0 });
  assert.strictEqual(s.last, NOW);
});

test('Nutzung: Ordner mit kaputten Dateien und Zwischenspeicher', () => {
  const root = folderWith([chatA(), { id: 'leer', updated: NOW, messages: [] }]);
  try {
    fs.writeFileSync(path.join(root, '.ask', 'chats', 'kaputt.json'), '{ halb');
    fs.writeFileSync(path.join(root, '.ask', 'chats', 'notiz.txt'), 'kein Chat');
    const u = folderUsage(root);
    assert.strictEqual(u.chats, 2);
    assert.strictEqual(u.tasks, 2);
    // Geänderte Datei wird neu gelesen, unveränderte kommt aus dem Zwischenspeicher
    const file = path.join(root, '.ask', 'chats', 'leer.json');
    fs.writeFileSync(file, JSON.stringify({ id: 'leer', updated: NOW, messages: [{ role: 'user', content: 'neu', time: NOW }] }));
    const later = new Date(Date.now() + 5000);
    fs.utimesSync(file, later, later);
    assert.strictEqual(folderUsage(root).tasks, 3);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('Nutzung: Summe über Projekte und die letzten 30 Tage', () => {
  const one = folderWith([chatA()]);
  const two = folderWith([{ id: 'z', updated: NOW, messages: [
    { role: 'user', content: 'heute', time: NOW - 1000 },
    { role: 'assistant', content: 'ok', time: NOW - 500, stats: stats(40, 800) },
    { role: 'user', content: 'alt', time: NOW - 60 * DAY },
  ] }]);
  try {
    const r = aggregate([
      { folder: one, name: 'Eins', color: '#0082B4' },
      { folder: two, name: 'Zwei', color: '#F28855' },
      { folder: path.join(one, 'gibt-es-nicht'), name: 'Weg' },
      null,
    ], { now: NOW });
    assert.strictEqual(r.projects.length, 3);
    // meiste Aufträge zuerst, bei Gleichstand das zuletzt benutzte
    assert.deepStrictEqual(r.projects.map((p) => p.name), ['Zwei', 'Eins', 'Weg']);
    assert.strictEqual(r.projects[2].missing, true);
    assert.strictEqual(r.total.chats, 2);
    assert.strictEqual(r.total.tasks, 4);
    assert.strictEqual(r.total.tokens, 390);
    assert.strictEqual(r.total.files.added, 2);
    assert.strictEqual(r.days.length, 30);
    assert.strictEqual(r.days[29].date, dayKey(NOW));
    assert.deepStrictEqual(r.days[29], { date: dayKey(NOW), tasks: 1, tokens: 40 });
    assert.strictEqual(r.days[26].tasks, 1);
    assert.strictEqual(r.days.reduce((s, d) => s + d.tasks, 0), 3, 'Auftrag vor 60 Tagen nicht in den 30 Tagen');
    assert.strictEqual(r.projects[1].days, undefined, 'Tageswerte nur gesamt');
  } finally {
    fs.rmSync(one, { recursive: true, force: true });
    fs.rmSync(two, { recursive: true, force: true });
  }
});

test('Nutzung: viele Chats bleiben schnell', () => {
  const chats = Array.from({ length: 400 }, (_, i) => ({ ...chatA(), id: `c${i}` }));
  const root = folderWith(chats);
  try {
    folderUsage(root);
    const t = Date.now();
    const u = folderUsage(root);
    assert.strictEqual(u.chats, 400);
    assert.ok(Date.now() - t < 500, `zweiter Durchgang aus dem Zwischenspeicher: ${Date.now() - t} ms`);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('Nutzung von ASK: bekannte Zahlen, sonst nichts', () => {
  assert.deepStrictEqual(usageNumbers({ total_tokens: 1200, prompt_tokens: 800, completion_tokens: 400, x: 5 }), { tokens: 1200, inputTokens: 800, outputTokens: 400 });
  assert.deepStrictEqual(usageNumbers({ items: [{ message_count: 3 }, { message_count: 4 }] }), { messages: 7 });
  assert.deepStrictEqual(usageNumbers({ detail: 'Not Found' }), {});
  assert.deepStrictEqual(usageNumbers(null), {});
});

function fakeSession(routes) {
  const calls = [];
  return {
    calls,
    token: 't',
    raw: async (p) => {
      calls.push(p);
      const hit = Object.entries(routes).find(([k]) => p === k);
      if (!hit) return { ok: false, status: 404, json: async () => ({}) };
      if (hit[1] instanceof Error) throw hit[1];
      return { ok: true, status: 200, json: async () => hit[1] };
    },
  };
}

test('Nutzung von ASK: Chats der Webseite seitenweise, Fehler still', async () => {
  const sec = (ms) => Math.floor(ms / 1000);
  const page1 = Array.from({ length: 3 }, (_, i) => ({ id: `w${i}`, updated_at: sec(NOW - i * DAY) }));
  const page2 = [{ id: 'w9', updated_at: sec(NOW - 90 * DAY) }];
  const s = fakeSession({ '/api/v1/chats/?page=1': page1, '/api/v1/chats/?page=2': page2, '/api/v1/chats/?page=3': [],
    '/api/v1/users/user/usage': { total_tokens: 5000 } });
  const r = await askUsage(s, { now: NOW });
  assert.deepStrictEqual(r, { webChats: 4, webChats30: 3, tokens: 5000 });

  // Server ohne «page»: dieselbe Liste wieder, dann aufhören
  const same = fakeSession({ '/api/v1/chats/?page=1': page1, '/api/v1/chats/?page=2': page1 });
  assert.deepStrictEqual(await askUsage(same, { now: NOW }), { webChats: 3, webChats30: 3 });
  assert.ok(!same.calls.includes('/api/v1/chats/?page=3'));

  // Nichts erlaubt oder nicht erreichbar: null
  assert.strictEqual(await askUsage(fakeSession({ '/api/v1/chats/?page=1': new Error('offline') }), { now: NOW }), null);
  assert.strictEqual(await askUsage(fakeSession({}), { now: NOW }), null);
  assert.strictEqual(await askUsage({ token: '' }), null);
  assert.strictEqual(await askUsage(null), null);
});
