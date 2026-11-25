// Datenmodell des Chats: Aus gespeicherten Chats (agent.transcript) und Live-Ereignissen des Agenten
// (chat:event) entsteht eine flache Liste von Einträgen. buildTurns gruppiert sie für die Anzeige in
// Aufträge: Eingabe, Arbeitsschritte (einklappbare Karte), Schlussantwort und alles danach.
// Reine Funktionen ohne DOM, damit sie sich mit node --test prüfen lassen.

export const READ_ONLY = new Set(['read_file', 'list_files', 'find_files', 'search_text', 'chat_search', 'chat_read']);

export function emptyChat() {
  return { entries: [], turnStats: [], seq: 0, streaming: null };
}

const firstLine = (text) => String(text || '').split('\n').map((l) => l.trim()).find((l) => l && l !== '(keine Ausgabe)') || '';

// Kurzresultat einer Werkzeugzeile wie in Claude Code: «84 Zeilen gelesen», «+10 −4 Zeilen»
export function shortSummary(name, ok, summary, output) {
  if (ok === false) {
    const line = firstLine(output).replace(/^Fehler:\s*/, '');
    return name === 'run_command' && summary ? `${summary} · ${line}` : (line || summary || 'Fehler');
  }
  switch (name) {
    case 'read_file': return summary ? `${summary} gelesen` : 'gelesen';
    case 'write_file': return summary ? `${summary} geschrieben` : 'geschrieben';
    case 'edit_file': {
      const lines = String(output || '').split('\n');
      return `+${lines.filter((l) => l.startsWith('+ ')).length} −${lines.filter((l) => l.startsWith('- ')).length} Zeilen`;
    }
    case 'run_command': {
      const lines = String(output || '').split('\n').filter((l) => l.trim() && l.trim() !== '(keine Ausgabe)');
      const text = lines.length === 0 ? 'fertig' : lines.length === 1 ? lines[0].trim() : `${lines.length} Zeilen Ausgabe`;
      return summary ? `${summary} · ${text}` : text;
    }
    default: return summary || 'erledigt';
  }
}

function add(model, entry) {
  const seq = model.seq + 1;
  return { ...model, seq, entries: [...model.entries, { key: `e${seq}`, ...entry }] };
}

function update(model, index, patch) {
  const entries = model.entries.slice();
  entries[index] = { ...entries[index], ...(typeof patch === 'function' ? patch(entries[index]) : patch) };
  return { ...model, entries };
}

const findIndex = (model, pred) => {
  for (let i = model.entries.length - 1; i >= 0; i--) if (pred(model.entries[i])) return i;
  return -1;
};

// Laufende Antwort: der letzte Eintrag, wenn er eine nicht beendete Antwort ist
function streamingIndex(model) {
  const i = model.entries.length - 1;
  return i >= 0 && model.entries[i].k === 'assistant' && !model.entries[i].done ? i : -1;
}

function ensureAssistant(model) {
  const i = streamingIndex(model);
  if (i >= 0) return [model, i];
  const next = add(model, { k: 'assistant', text: '', reasoning: '', done: false });
  return [next, next.entries.length - 1];
}

export const QUESTION = 'frage_stellen';

function toolEntry(call) {
  const entry = { k: 'tool', id: call.id, name: call.name, label: call.label || call.name, detail: call.detail || '',
    ok: null, summary: '', output: '', progress: '', tasks: null, images: [] };
  // Frage des Modells: Frage und Antworten zum Anklicken; answer erst nach der Antwort
  if (call.name === QUESTION) entry.question = { frage: call.detail || '', optionen: call.optionen || [], mehrfach: false, open: false };
  return entry;
}

function finishTool(entry, { ok, summary, output, tasks, images }) {
  return { ok: ok !== false, summary: shortSummary(entry.name, ok, summary, output), rawSummary: summary || '',
    output: output || '', progress: '', tasks: tasks && tasks.length ? tasks : null, images: images || [] };
}

function endAssistant(model, content, reasoning) {
  let [m, i] = ensureAssistant(model);
  const e = m.entries[i];
  const text = content !== undefined && content !== null ? content : e.text;
  if (!String(text || '').trim() && !String(reasoning ?? e.reasoning).trim()) {
    return { ...m, entries: m.entries.filter((_x, j) => j !== i) };
  }
  m = update(m, i, { text: text || '', reasoning: reasoning ?? e.reasoning, done: true });
  return m;
}

function endTurn(model, turn) {
  let m = model;
  if (turn && turn.snapshot && turn.changes) m = add(m, { k: 'changes', snapshot: turn.snapshot, changes: turn.changes, undone: !!turn.undone });
  const list = m.turnStats.filter(Boolean);
  if (list.length) m = add(m, { k: 'stats', list });
  return { ...m, turnStats: [] };
}

// Gespeicherten Chat in Einträge umwandeln (entspricht dem Ablauf live)
export function fromTranscript(items, running = false) {
  let m = emptyChat();
  let turn = null;
  for (const item of items || []) {
    if (item.role === 'auto') m = add(m, { k: 'auto' });
    else if (item.role === 'undo') m = add(m, { k: 'notice', icon: 'undo', text: item.text });
    else if (item.role === 'user') {
      m = endTurn(m, turn);
      turn = item;
      m = add(m, { k: 'user', text: item.content || '', attachments: item.attachments || [], time: item.time || 0, snapshot: item.snapshot || null });
    } else if (item.role === 'assistant') {
      m = { ...m, turnStats: [...m.turnStats, item.stats] };
      m = add(m, { k: 'assistant', text: item.content || '', reasoning: item.reasoning || '', done: true, time: item.time || 0 });
      if (!String(item.content || '').trim() && !String(item.reasoning || '').trim()) m = { ...m, entries: m.entries.slice(0, -1) };
      if (item.sources?.length) m = add(m, { k: 'sources', sources: item.sources });
      for (const call of item.calls || []) m = add(m, toolEntry(call));
    } else if (item.role === 'tool') {
      const i = findIndex(m, (e) => e.k === 'tool' && e.id === item.id);
      if (i >= 0) m = update(m, i, (e) => finishTool(e, item));
    }
  }
  // Werkzeuge ohne Ergebnis (App beendet während des Schritts)
  if (!running) {
    m = { ...m, entries: m.entries.map((e) => (e.k === 'tool' && e.ok === null ? { ...e, ok: false, summary: 'nicht beendet' } : e)) };
    m = endTurn(m, turn);
  }
  return m;
}

// Ein Ereignis des Agenten anwenden
export function applyEvent(model, ev) {
  switch (ev.type) {
    case 'user':
      if (ev.auto) return add(model, { k: 'auto' });
      return add({ ...model, turnStats: [] }, { k: 'user', text: ev.text || '', attachments: ev.attachments || [], time: Date.now() });
    case 'assistant-start':
      return { ...model, webSearch: !!ev.webSearch };
    case 'reasoning': {
      const [m, i] = ensureAssistant(model);
      return update(m, i, (e) => ({ reasoning: e.reasoning + ev.text }));
    }
    case 'text': {
      const [m, i] = ensureAssistant(model);
      return update(m, i, (e) => ({ text: e.text + ev.text }));
    }
    case 'assistant-end': {
      let m = endAssistant(model, ev.content);
      m = { ...m, turnStats: [...m.turnStats, ev.stats] };
      if (ev.sources?.length) m = add(m, { k: 'sources', sources: ev.sources });
      return m;
    }
    case 'tool-start': {
      const i = findIndex(model, (e) => e.k === 'tool' && e.id === ev.id);
      if (i >= 0) return model;
      return add(model, toolEntry(ev));
    }
    case 'tool-progress': {
      const i = findIndex(model, (e) => e.k === 'tool' && e.id === ev.id);
      return i < 0 ? model : update(model, i, { progress: String(ev.output || '') });
    }
    case 'tool-end': {
      const i = findIndex(model, (e) => e.k === 'tool' && e.id === ev.id);
      return i < 0 ? model : update(model, i, (e) => finishTool(e, ev));
    }
    case 'question': {
      const i = findIndex(model, (e) => e.k === 'tool' && e.id === ev.id);
      const question = { frage: ev.frage, optionen: ev.optionen || [], mehrfach: !!ev.mehrfach, open: true };
      return i < 0 ? add(model, { ...toolEntry({ id: ev.id, name: QUESTION, label: 'Frage', detail: ev.frage }), question })
        : update(model, i, { question });
    }
    case 'question-end': {
      const i = findIndex(model, (e) => e.k === 'tool' && e.id === ev.id);
      return i < 0 ? model : update(model, i, (e) => ({ question: { ...e.question, open: false, answer: ev.answer } }));
    }
    case 'approval':
      return add(model, { k: 'approval', id: ev.id, title: ev.title, detail: ev.detail || '', reason: ev.reason || '', answer: null });
    case 'approval-end': {
      const i = findIndex(model, (e) => e.k === 'approval' && e.id === ev.id);
      return i < 0 ? model : update(model, i, { answer: ev.answer || 'no' });
    }
    case 'changes':
      return add(model, { k: 'changes', snapshot: ev.snapshot, changes: ev.changes, undone: false });
    case 'undo': {
      const entries = model.entries.map((e) => (e.k === 'changes' && e.snapshot === ev.snapshot ? { ...e, undone: true } : e));
      return add({ ...model, entries }, { k: 'notice', icon: 'undo', text: ev.text });
    }
    case 'notice': return add(model, { k: 'notice', text: ev.text });
    case 'error': return add(model, { k: 'error', text: ev.text });
    case 'local-error': return add(model, { k: 'error', text: ev.text });
    case 'done': {
      let m = model;
      const i = streamingIndex(m);
      if (i >= 0) m = endAssistant(m);
      // offene Rückfragen sind mit dem Ende des Auftrags erledigt
      m = { ...m, entries: m.entries.map((e) => (e.k === 'approval' && !e.answer ? { ...e, answer: 'no' }
        : e.question?.open ? { ...e, question: { ...e.question, open: false } } : e)) };
      return endTurn(m, null);
    }
    default: return model;
  }
}

// Letzte Aufgabenliste des Chats (update_tasks), solange noch etwas offen ist
export function lastTasks(model) {
  for (let i = model.entries.length - 1; i >= 0; i--) {
    const e = model.entries[i];
    if (e.k === 'tool' && e.tasks?.length) return e.tasks;
  }
  return null;
}

// Einträge in Aufträge gruppieren.
// turn = { key, user, steps, answer, after, running, toolCount, failed, start, end }
// steps: { type: 'tool', entry } | { type: 'group', key, entries } | { type: 'thought', entry } | { type: 'auto', entry }
export function buildTurns(model, running = false) {
  const turns = [];
  let cur = null;
  const open = (user) => {
    cur = { key: user ? user.key : 'start', user, body: [] };
    turns.push(cur);
  };
  for (const e of model.entries) {
    if (e.k === 'user') open(e);
    else {
      if (!cur) open(null);
      cur.body.push(e);
    }
  }
  return turns.map((t, ti) => {
    const last = ti === turns.length - 1;
    const isRunning = running && last;
    const body = t.body;
    // Schlussantwort: letzte Antwort mit Text, nach der kein Werkzeug mehr kommt
    let answerIndex = -1;
    for (let i = body.length - 1; i >= 0; i--) {
      const e = body[i];
      if ((e.k === 'tool' && e.name !== QUESTION) || e.k === 'auto') break;
      if (e.k === 'assistant' && String(e.text || '').trim()) { answerIndex = i; break; }
    }
    const steps = [];
    const after = [];
    let group = null;
    body.forEach((e, i) => {
      if (i === answerIndex) return;
      if (e.k === 'tool' && e.name === QUESTION) {
        group = null;
        // Offene Fragen gut sichtbar unter der Karte, beantwortete als Schritt in der Karte
        if (e.question?.open) after.push(e);
        else steps.push({ type: 'question', entry: e });
        return;
      }
      if (e.k === 'tool') {
        if (READ_ONLY.has(e.name)) {
          if (!group) { group = { type: 'group', key: `g-${e.key}`, entries: [] }; steps.push(group); }
          group.entries.push(e);
          return;
        }
        group = null;
        steps.push({ type: 'tool', entry: e });
      } else if (e.k === 'assistant') {
        group = null;
        steps.push({ type: 'thought', entry: e });
      } else if (e.k === 'approval' && e.answer) {
        group = null;
        steps.push({ type: 'approval', entry: e }); // erledigte Rückfrage als Schritt
      } else if (e.k === 'auto') {
        group = null;
        steps.push({ type: 'auto', entry: e });
      } else {
        after.push(e);
      }
    });
    // Eine Gruppe mit nur einem Eintrag ist eine normale Zeile
    const flatSteps = steps.map((s) => (s.type === 'group' && s.entries.length === 1 ? { type: 'tool', entry: s.entries[0] } : s));
    const tools = body.filter((e) => e.k === 'tool' && e.name !== QUESTION);
    return {
      key: t.key,
      user: t.user,
      steps: flatSteps,
      answer: answerIndex >= 0 ? body[answerIndex] : null,
      after,
      running: isRunning,
      toolCount: tools.length,
      failed: tools.filter((e) => e.ok === false).length,
      pending: tools.some((e) => e.ok === null),
    };
  });
}

// Zusammenfassung eines Auftrags: Schritte, Token, Token/s, Zeit des Modells (Zahlen wie in de-CH)
const num = (n) => Math.round(n).toLocaleString('de-CH');
const sec = (ms) => (ms / 1000).toLocaleString('de-CH', { minimumFractionDigits: 1, maximumFractionDigits: 1 });

export function statsText(list) {
  list = (list || []).filter(Boolean);
  if (!list.length) return null;
  const tokens = list.reduce((n, s) => n + (s.tokens || 0), 0);
  const gen = list.reduce((n, s) => n + (s.genMs || 0), 0);
  const total = list.reduce((n, s) => n + (s.totalMs || 0), 0);
  const approx = list.some((s) => s.estimated) ? '≈ ' : '';
  const parts = [`${list.length} ${list.length === 1 ? 'Schritt' : 'Schritte'}`, `${approx}${num(tokens)} Token`];
  if (gen > 0) parts.push(`${approx}${num(tokens / (gen / 1000))} Token/s`);
  parts.push(`Modell ${sec(total)} s`);
  const steps = list.map((s, i) => `Schritt ${i + 1}: ${s.estimated ? '≈ ' : ''}${num(s.tokens || 0)} Token in ${sec(s.genMs || 0)} s`
    + `${s.genMs > 0 ? ` (${num((s.tokens || 0) / (s.genMs / 1000))} Token/s)` : ''}, erste Antwort nach ${sec(s.firstMs || 0)} s`
    + `${s.promptTokens ? `, Eingabe ${num(s.promptTokens)} Token` : ''}`);
  if (approx) steps.push('≈ geschätzt aus der Textlänge: ASK hat keine Tokenzahlen geliefert.');
  return { text: parts.join(' · '), title: steps.join('\n') };
}

// Überschrift einer Gruppe von Lese- und Suchschritten
export function groupTitle(entries) {
  const count = (names) => entries.filter((i) => names.includes(i.name)).length;
  const reads = count(['read_file', 'chat_read']);
  const lists = count(['list_files']);
  const searches = count(['find_files', 'search_text', 'chat_search']);
  const parts = [];
  if (reads) parts.push(`${reads} ${reads === 1 ? 'Datei' : 'Dateien'} gelesen`);
  if (lists) parts.push(`${lists} Ordner angesehen`);
  if (searches) parts.push(`${searches} ${searches === 1 ? 'Suche' : 'Suchen'}`);
  return parts.join(', ');
}
