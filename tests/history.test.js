const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { historyPrompt, search, read, chatText } = require('../src/main/history');
const { runTool } = require('../src/main/tools');

function folderWithChats() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-chats-'));
  const dir = path.join(root, '.ask', 'chats');
  fs.mkdirSync(dir, { recursive: true });
  const call = (name, args) => ({ id: 'c', type: 'function', function: { name, arguments: JSON.stringify(args) } });
  const chats = [
    { id: 'alt', title: 'Bericht Proxmox', updated: 1000, messages: [
      { role: 'user', content: 'Schreib einen Bericht über Proxmox Backups', attachments: [{ name: 'notizen.docx' }] },
      { role: 'assistant', content: '', tool_calls: [call('write_file', { path: 'bericht.docx' })] },
      { role: 'tool', content: 'Geschrieben' },
      { role: 'user', auto: true, note: 'view', content: '[Ansicht] Bilder' },
      { role: 'assistant', content: 'Der Bericht liegt in bericht.docx. Backups laufen täglich.' }] },
    { id: 'neu', title: 'Spiel', updated: 3000, messages: [{ role: 'user', content: 'Mach ein Spiel' }, { role: 'assistant', content: 'Fertig.' }] },
    { id: 'jetzt', title: 'Aktuell', updated: 5000, messages: [{ role: 'user', content: 'Proxmox Backups nochmals' }] },
  ];
  for (const c of chats) fs.writeFileSync(path.join(dir, `${c.id}.json`), JSON.stringify(c));
  return { root, done: () => fs.rmSync(root, { recursive: true, force: true }) };
}

test('Frühere Chats: Liste ohne den laufenden Chat, Text ohne Ansichten', () => {
  const f = folderWithChats();
  try {
    const prompt = historyPrompt(f.root, 'jetzt');
    assert.match(prompt, /# Frühere Chats in diesem Ordner/);
    assert.ok(prompt.indexOf('«Spiel» (id neu)') < prompt.indexOf('«Bericht Proxmox» (id alt)'), 'neueste zuerst');
    assert.ok(!prompt.includes('Aktuell'));
    const text = chatText(JSON.parse(fs.readFileSync(path.join(f.root, '.ask', 'chats', 'alt.json'), 'utf8')));
    assert.strictEqual(text, 'Person: Schreib einen Bericht über Proxmox Backups [Anhänge: notizen.docx]\n  [write_file bericht.docx]\nModell: Der Bericht liegt in bericht.docx. Backups laufen täglich.');
    assert.match(historyPrompt(os.tmpdir(), 'x'), /Keine\./);
  } finally {
    f.done();
  }
});

test('Suchen und lesen; nur mit Schalter', async () => {
  const f = folderWithChats();
  try {
    const hits = search(f.root, 'proxmox backups', 'jetzt');
    assert.deepStrictEqual(hits.map((h) => h.id), ['alt'], 'laufender Chat nicht, alle Wörter nötig');
    assert.ok(hits[0].snippets[0].includes('Proxmox Backups'));
    const r = read(f.root, 'alt');
    assert.strictEqual(r.title, 'Bericht Proxmox');
    assert.ok(r.text.includes('Backups laufen täglich'));
    assert.throws(() => read(f.root, '../x'), /Ungültige/);
    assert.throws(() => read(f.root, 'fehlt'), /Kein Chat/);

    const off = await runTool(null, 'chat_search', { anfrage: 'proxmox' }, { folder: f.root, chatMemory: false });
    assert.ok(!off.ok && /nicht freigegeben/.test(off.output));
    const on = await runTool(null, 'chat_search', { anfrage: 'proxmox' }, { folder: f.root, chatMemory: true, chat: 'jetzt' });
    assert.ok(on.ok && on.output.includes('«Bericht Proxmox» (id alt'), on.output);
    const readTool = await runTool(null, 'chat_read', { id: 'alt' }, { folder: f.root, chatMemory: true });
    assert.ok(readTool.ok && readTool.output.startsWith('Chat «Bericht Proxmox» vom'), readTool.output);
  } finally {
    f.done();
  }
});
