// Frühere Chats eines Ordners für das Modell (ENTSCHEIDE E28), nur wenn die Person den Schalter
// «Chats» einschaltet: eine Liste im Systemprompt und die Werkzeuge chat_search und chat_read.
// Das Modell bekommt Eingaben und Antworten als Text, Werkzeugaufrufe nur als kurze Zeile.
const fs = require('node:fs');
const path = require('node:path');

const CHAT_DIR = path.join('.ask', 'chats');
const LIST_MAX = 15;
const READ_CHARS = 15000;
const RESULTS = 8;

const date = (ms) => {
  const d = new Date(ms || 0);
  const p = (n) => String(n).padStart(2, '0');
  return `${p(d.getDate())}.${p(d.getMonth() + 1)}.${d.getFullYear()} ${p(d.getHours())}:${p(d.getMinutes())}`;
};

function loadChats(folder) {
  const dir = path.join(folder, CHAT_DIR);
  let names = [];
  try { names = fs.readdirSync(dir).filter((n) => n.endsWith('.json')); } catch { return []; }
  return names.map((n) => {
    try { return JSON.parse(fs.readFileSync(path.join(dir, n), 'utf8')); } catch { return null; }
  }).filter((c) => c && c.id && Array.isArray(c.messages)).sort((a, b) => (b.updated || 0) - (a.updated || 0));
}

// Verlauf als Text: Eingaben, Antworten und Werkzeugaufrufe als Kurzzeile
function chatText(chat) {
  const lines = [];
  for (const m of chat.messages) {
    if (m.role === 'user') {
      if (m.note === 'view' || m.auto) continue;
      const files = (m.attachments || []).map((a) => a.name).join(', ');
      lines.push(`Person: ${m.content || ''}${files ? ` [Anhänge: ${files}]` : ''}`);
    } else if (m.role === 'assistant') {
      for (const c of m.tool_calls || []) {
        let args = {};
        try { args = JSON.parse(c.function.arguments || '{}'); } catch { /* roh */ }
        const detail = args.path || args.command || args.anfrage || args.url || args.nachricht || args.auftrag || '';
        lines.push(`  [${c.function.name}${detail ? ` ${String(detail).replace(/\s+/g, ' ').slice(0, 120)}` : ''}]`);
      }
      if (m.content && m.content.trim()) lines.push(`Modell: ${m.content.trim()}`);
    }
  }
  return lines.join('\n');
}

// Liste für den Systemprompt (ohne den laufenden Chat)
function historyPrompt(folder, current) {
  const chats = loadChats(folder).filter((c) => c.id !== current && c.messages.some((m) => m.role === 'user'));
  if (!chats.length) return '\n\n# Frühere Chats in diesem Ordner\nKeine.';
  const list = chats.slice(0, LIST_MAX).map((c) => `- ${date(c.updated)} «${String(c.title || '(ohne Titel)').slice(0, 80)}» (id ${c.id})`);
  return `\n\n# Frühere Chats in diesem Ordner (neueste zuerst${chats.length > LIST_MAX ? `, ${LIST_MAX} von ${chats.length}` : ''})\n${list.join('\n')}\n`
    + 'Bezieht sich die Person auf frühere Arbeit («wie letztes Mal», «der Bericht von gestern»), suche mit chat_search oder lies den Chat mit chat_read.';
}

function search(folder, query, current) {
  const words = String(query || '').toLowerCase().split(/\s+/).filter((w) => w.length > 1);
  if (!words.length) throw new Error('Leere Suchanfrage.');
  const hits = [];
  for (const c of loadChats(folder)) {
    if (c.id === current) continue;
    const text = chatText(c);
    const lower = `${String(c.title || '').toLowerCase()}\n${text.toLowerCase()}`;
    const score = words.reduce((n, w) => n + (lower.split(w).length - 1), 0);
    if (!score || !words.every((w) => lower.includes(w))) continue;
    const snippets = text.split('\n').filter((l) => words.some((w) => l.toLowerCase().includes(w))).slice(0, 3)
      .map((l) => (l.length > 220 ? `${l.slice(0, 220)} …` : l));
    hits.push({ id: c.id, title: c.title || '(ohne Titel)', updated: c.updated, score, snippets });
  }
  return hits.sort((a, b) => b.score - a.score || b.updated - a.updated).slice(0, RESULTS);
}

function read(folder, id, ab = 0) {
  if (!/^[\w-]+$/.test(String(id))) throw new Error('Ungültige Chat-id.');
  const chat = loadChats(folder).find((c) => c.id === id);
  if (!chat) throw new Error(`Kein Chat mit id ${id} in diesem Ordner.`);
  const text = chatText(chat);
  const from = Math.max(0, Math.min(Number(ab) || 0, text.length));
  return { title: chat.title || '(ohne Titel)', updated: chat.updated, text: text.slice(from, from + READ_CHARS), from, to: Math.min(text.length, from + READ_CHARS), total: text.length };
}

module.exports = { historyPrompt, search, read, chatText, date };
