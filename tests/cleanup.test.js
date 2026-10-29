// Tests für das Aufräumen von Skripten und Zwischendateien in .ask (node --test tests/).
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { cleanupWorkDirs, KEEP_DAYS } = require('../src/main/cleanup');
const { loopReason } = require('../src/main/agent');

test('alte Skripte und Zwischendateien weg, neue und Chats bleiben', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-cleanup-'));
  try {
    const file = (rel, ageDays) => {
      const full = path.join(root, rel);
      fs.mkdirSync(path.dirname(full), { recursive: true });
      fs.writeFileSync(full, 'x');
      const t = new Date(Date.now() - ageDays * 86400000);
      fs.utimesSync(full, t, t);
      return full;
    };
    const oldScript = file('.ask/skripte/chat-a/alt.py', KEEP_DAYS + 1);
    const newScript = file('.ask/skripte/chat-b/neu.py', 1);
    const oldWork = file('.ask/arbeit/chat-a/versuch.docx', KEEP_DAYS + 5);
    const chat = file('.ask/chats/chat-a.json', 100);
    const result = file('ausgabe/bericht.docx', 100);
    assert.strictEqual(cleanupWorkDirs(root), 2);
    assert.ok(!fs.existsSync(oldScript) && !fs.existsSync(oldWork));
    assert.ok(!fs.existsSync(path.join(root, '.ask', 'skripte', 'chat-a')), 'leerer Chat-Ordner entfernt');
    assert.ok(fs.existsSync(path.join(root, '.ask', 'skripte')), 'Hauptordner bleibt');
    assert.ok(fs.existsSync(newScript) && fs.existsSync(chat) && fs.existsSync(result));
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('ohne .ask nichts zu tun', () => {
  assert.strictEqual(cleanupWorkDirs(path.join(os.tmpdir(), 'gibt-es-nicht-ask')), 0);
});

test('Schleifenschutz: dreimal derselbe Aufruf oder viele Fehler hintereinander', () => {
  const call = (name, args) => ({ role: 'assistant', content: '', tool_calls: [{ function: { name, arguments: JSON.stringify(args) } }] });
  const ok = { role: 'tool', ok: true, content: '' };
  const bad = { role: 'tool', ok: false, content: 'Fehler' };
  const same = [{ role: 'user', content: 'los' }, call('run_command', { command: 'ping' }), ok, call('run_command', { command: 'ping' }), ok];
  assert.strictEqual(loopReason(same), '');
  assert.match(loopReason([...same, call('run_command', { command: 'ping' }), ok]), /3-mal hintereinander genau denselben Schritt/);
  assert.strictEqual(loopReason([...same, call('run_command', { command: 'dir' }), ok]), '');
  const fails = [{ role: 'user', content: 'los' }];
  for (let i = 0; i < 8; i++) fails.push(call('read_file', { path: `d${i}.txt` }), bad);
  assert.match(loopReason(fails), /8 Werkzeugaufrufe hintereinander/);
  assert.strictEqual(loopReason([...fails, { role: 'user', content: 'neu' }]), '', 'zählt nur seit der letzten Eingabe');
});
