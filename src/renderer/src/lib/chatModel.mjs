// Datenmodell des Chats: Aus gespeicherten Chats (agent.transcript) und Live-Ereignissen des Agenten
// (chat:event) entsteht eine flache Liste von Einträgen. buildTurns gruppiert sie für die Anzeige in
// Aufträge: Eingabe, Arbeitsschritte (einklappbare Karte), Schlussantwort und alles danach.
// Reine Funktionen ohne DOM, damit sie sich mit node --test prüfen lassen.

export const READ_ONLY = new Set(['read_file', 'list_files', 'find_files', 'search_text', 'chat_search', 'chat_read', 'quellen_suchen', 'quelle_lesen']);

export function emptyChat() {
  return { entries: [], turnStats: [], seq: 0, streaming: null, btw: [] };
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

// Nebenfragen mit /btw (ENTSCHEIDE E86) stehen nicht in den Einträgen, sondern daneben (model.btw): Sonst würde eine
// Nebenantwort mitten in die laufende Antwort des Auftrags geraten. Jede gehört zu dem Auftrag, der beim Fragen lief
// (turn: Schlüssel seiner Eingabe, «start» vor der ersten Eingabe).
const lastUserKey = (model) => {
  for (let i = model.entries.length - 1; i >= 0; i--) if (model.entries[i].k === 'user') return model.entries[i].key;
  return 'start';
};

function upsertBtw(model, id, patch, create) {
  const list = model.btw || [];
  const i = list.findIndex((b) => b.id === id);
  const value = (b) => (typeof patch === 'function' ? patch(b) : patch);
  if (i < 0) {
    if (!create) return model;
    const fresh = { id, question: '', text: '', error: '', done: false, time: 0, turn: lastUserKey(model) };
    return { ...model, btw: [...list, { ...fresh, ...value(fresh) }] };
  }
  const next = list.slice();
  next[i] = { ...list[i], ...value(list[i]) };
  return { ...model, btw: next };
}

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
export const PLAN = 'plan_vorlegen';
const REMEMBER = 'merken';

// Hat sich das Modell mit «merken» etwas gemerkt, steht das sichtbar im Chat, mit «Vergessen» (ENTSCHEIDE E50).
// Kennung und Text kommen aus dem Ergebnis des Werkzeugs (memorytools.js).
function remembered(entry, result) {
  if (entry?.name !== REMEMBER || result.ok === false) return null;
  const m = /^Gemerkt \(id ([\w-]+), ([^)]*)\): ([\s\S]+)$/.exec(String(result.output || ''));
  return m ? { k: 'remembered', memoryId: m[1], scope: m[2], text: m[3].trim() } : null;
}

function finishWithMemory(model, i, result) {
  const entry = model.entries[i];
  const m = update(model, i, (e) => finishTool(e, result));
  const note = remembered(entry, result);
  return note ? add(m, note) : m;
}

function toolEntry(call) {
  const entry = { k: 'tool', id: call.id, name: call.name, label: call.label || call.name, detail: call.detail || '',
    ok: null, summary: '', output: '', progress: '', tasks: null, images: [] };
  // Frage des Modells: Frage und Antworten zum Anklicken; answer erst nach der Antwort
  if (call.name === QUESTION) entry.question = { frage: call.detail || '', optionen: call.optionen || [], mehrfach: false, open: false };
  return entry;
}

function finishTool(entry, { ok, summary, output, tasks, images, plan }) {
  return { ...(plan ? { plan } : {}), ok: ok !== false, summary: shortSummary(entry.name, ok, summary, output), rawSummary: summary || '',
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
    else if (item.role === 'undo') m = add(m, { k: 'notice', icon: 'undo', text: item.text, redo: item.redo || null, redone: !!item.redone });
    else if (item.role === 'redo') m = add(m, { k: 'notice', icon: 'redo', redoNote: true, text: item.text });
    // Gespeicherte Hinweise und Fehler eines Auftrags (auch «ab hier zusammengefasst»): Wer den Chat später öffnet,
    // sieht, warum ein Auftrag aufgehört hat
    else if (item.role === 'notice') m = add(m, { k: 'notice', text: item.text || '' });
    else if (item.role === 'error') m = add(m, { k: 'error', text: item.text || '' });
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
      if (i >= 0) m = finishWithMemory(m, i, item);
    } else if (item.role === 'btw') {
      m = upsertBtw(m, item.id || `btw-${m.seq}-${(m.btw || []).length}`, { question: item.question || '', text: item.answer || '', error: item.error || '', done: true, time: item.time || 0 }, true);
    }
  }
  // Werkzeuge ohne Ergebnis (App beendet während des Schritts)
  if (!running) {
    m = { ...m, entries: m.entries.map((e) => (e.k === 'tool' && e.ok === null ? { ...e, ok: false, summary: 'nicht beendet' } : e)) };
    m = endTurn(m, turn);
  }
  return m;
}

// Chat laden: gespeicherter Verlauf, dazu die offenen Fragen und Rückfragen eines Chats, der schon arbeitet (pending:
// Ereignisse «question» und «approval»). Ohne sie sähe man nach dem Wechsel in einen Chat, der im Hintergrund auf
// eine Antwort wartet, die Frage nicht mehr und könnte nur noch stoppen.
export function loadChat(items, running = false, pending = []) {
  return (running ? pending || [] : []).reduce(applyEvent, fromTranscript(items, running));
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
      return i < 0 ? model : finishWithMemory(model, i, ev);
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
      // Dieselbe Rückfrage nur einmal (beim Wechsel in einen Chat kann sie mit dem Verlauf und als Ereignis kommen):
      // Eine zweite Karte bliebe für immer offen
      if (model.entries.some((e) => e.k === 'approval' && e.id === ev.id)) return model;
      return add(model, { k: 'approval', id: ev.id, title: ev.title, detail: ev.detail || '', reason: ev.reason || '', reviewed: !!ev.reviewed, answer: null });
    case 'approval-end': {
      const i = findIndex(model, (e) => e.k === 'approval' && e.id === ev.id);
      return i < 0 ? model : update(model, i, { answer: ev.answer || 'no' });
    }
    case 'changes':
      return add(model, { k: 'changes', snapshot: ev.snapshot, changes: ev.changes, undone: false });
    case 'undo': {
      // Rückgängig bis zu diesem Auftrag: auch die Änderungen danach sind weg
      const from = model.entries.findIndex((e) => e.k === 'changes' && e.snapshot === ev.snapshot);
      const entries = model.entries.map((e, i) => (e.k === 'changes' && from >= 0 && i >= from ? { ...e, undone: true } : e));
      return add({ ...model, entries }, { k: 'notice', icon: 'undo', text: ev.text, redo: ev.redo || null, redone: false });
    }
    case 'redo': {
      // Rückgängig zurückgenommen: dessen Aufträge gelten wieder, Aufträge seither sind rückgängig gemacht
      const back = new Set(ev.snapshots || []);
      const later = new Set(ev.later || []);
      const entries = model.entries.map((e) => {
        if (e.k === 'changes' && back.has(e.snapshot)) return { ...e, undone: false };
        if (e.k === 'changes' && later.has(e.snapshot)) return { ...e, undone: true };
        if (e.k === 'notice' && e.redo && e.redo === ev.redo) return { ...e, redone: true };
        return e;
      });
      return add({ ...model, entries }, { k: 'notice', icon: 'redo', redoNote: true, text: ev.text });
    }
    case 'retry': {
      // Verbindung weg: halbe Antwort verwerfen, ein Hinweis, der beim nächsten Versuch nachgeführt wird
      let m = model;
      const s = streamingIndex(m);
      if (s >= 0) m = { ...m, entries: m.entries.filter((_x, j) => j !== s) };
      const last = m.entries[m.entries.length - 1];
      return last?.k === 'notice' && last.retry ? update(m, m.entries.length - 1, { text: ev.text }) : add(m, { k: 'notice', icon: 'reload', retry: true, text: ev.text });
    }
    // Nebenfrage (E86): live gestreamt neben dem Auftrag; das Ende bringt Frage und Antwort noch einmal mit, falls der
    // Chat inzwischen neu geladen wurde
    case 'btw-start': return upsertBtw(model, ev.id, { question: ev.question || '', time: ev.time || 0 }, true);
    case 'btw-delta': return upsertBtw(model, ev.id, (b) => ({ text: b.text + (ev.text || '') }), false);
    case 'btw-end': return upsertBtw(model, ev.id, (b) => ({ question: b.question || ev.question || '', time: b.time || ev.time || 0,
      text: ev.error ? b.text : (ev.text ?? b.text), error: ev.error || '', done: true }), true);
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
// Das Rückgängig, das sich noch zurücknehmen lässt: nur das letzte, und nur solange es nicht schon
// zurückgenommen ist (sonst stimmten die Markierungen der Aufträge nicht mehr)
export function redoable(model) {
  for (let i = model.entries.length - 1; i >= 0; i--) {
    const e = model.entries[i];
    if (e.k === 'notice' && e.redoNote) return null;
    if (e.k === 'notice' && e.icon === 'undo' && 'redo' in e) return e.redo && !e.redone ? e.redo : null;
  }
  return null;
}

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
const NO_BTW = Object.freeze([]);

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
  // Nebenfragen (E86) zu ihrem Auftrag; gehört eine zu keinem (mehr), dann zum letzten
  const btwOf = new Map();
  const keys = new Set(turns.map((t) => t.key));
  for (const b of model.btw || []) {
    const key = keys.has(b.turn) ? b.turn : turns.at(-1)?.key;
    if (!btwOf.has(key)) btwOf.set(key, []);
    btwOf.get(key).push(b);
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
      if (e.k === 'tool' && e.name === PLAN && e.plan) { group = null; after.push(e); return; } // Plan als Karte unter der Arbeit
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
    const tools = body.filter((e) => e.k === 'tool' && e.name !== QUESTION && e.name !== PLAN);
    return {
      key: t.key,
      user: t.user,
      body,
      steps: flatSteps,
      answer: answerIndex >= 0 ? body[answerIndex] : null,
      after,
      btw: btwOf.get(t.key) || NO_BTW,
      running: isRunning,
      toolCount: tools.length,
      failed: tools.filter((e) => e.ok === false).length,
      pending: tools.some((e) => e.ok === null),
    };
  });
}

// Unveränderte Aufträge behalten ihr Objekt von vorher: Einträge werden nie verändert, nur ersetzt, darum genügt der
// Vergleich der Einträge selbst. So zeichnet die Oberfläche bei jedem Stück Antwort nur den laufenden Auftrag neu,
// nicht den ganzen Chat (sonst wird ein langer Chat beim Schreiben der Antwort immer träger).
export function reuseTurns(prev, next) {
  if (!prev?.length) return next;
  const old = new Map(prev.map((t) => [t.key, t]));
  return next.map((t) => {
    const p = old.get(t.key);
    const same = (a = [], b = []) => a.length === b.length && a.every((e, i) => e === b[i]);
    return p && p.running === t.running && p.user === t.user && same(p.body, t.body) && same(p.btw, t.btw) ? p : t;
  });
}

// Zusammenfassung eines Auftrags: Schritte, Token, Token/s, Zeit des Modells (Zahlen wie in de-CH)
const num = (n) => Math.round(n).toLocaleString('de-CH');
const sec = (ms) => (ms / 1000).toLocaleString('de-CH', { minimumFractionDigits: 1, maximumFractionDigits: 1 });

// Dauer für Menschen: «45 s» (mit decimals «3,0 s»), ab einer Minute «16 min 40 s», ab einer Stunde «1 h 2 min»,
// damit nie «1000 s» dasteht
export function dauer(ms, { decimals = false } = {}) {
  const v = Math.max(0, Number(ms) || 0);
  const total = Math.round(v / 1000);
  if (total < 60) return decimals && v < 59950 ? `${sec(v)} s` : `${total} s`;
  if (total < 3600) return `${Math.floor(total / 60)} min${total % 60 ? ` ${total % 60} s` : ''}`;
  const min = Math.round(total / 60);
  return `${Math.floor(min / 60)} h${min % 60 ? ` ${min % 60} min` : ''}`;
}

export function statsText(list) {
  list = (list || []).filter(Boolean);
  if (!list.length) return null;
  const tokens = list.reduce((n, s) => n + (s.tokens || 0), 0);
  const gen = list.reduce((n, s) => n + (s.genMs || 0), 0);
  const total = list.reduce((n, s) => n + (s.totalMs || 0), 0);
  const approx = list.some((s) => s.estimated) ? '≈ ' : '';
  const parts = [`${list.length} ${list.length === 1 ? 'Schritt' : 'Schritte'}`, `${approx}${num(tokens)} Token`];
  if (gen > 0) parts.push(`${approx}${num(tokens / (gen / 1000))} Token/s`);
  parts.push(`Modell ${dauer(total, { decimals: true })}`);
  const steps = list.map((s, i) => `Schritt ${i + 1}: ${s.estimated ? '≈ ' : ''}${num(s.tokens || 0)} Token in ${dauer(s.genMs || 0, { decimals: true })}`
    + `${s.genMs > 0 ? ` (${num((s.tokens || 0) / (s.genMs / 1000))} Token/s)` : ''}, erste Antwort nach ${dauer(s.firstMs || 0, { decimals: true })}`
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
  // Quellen (E84): Suchen und gelesene Teile in den Unterlagen
  const sourceSearches = count(['quellen_suchen']);
  const sourceReads = count(['quelle_lesen']);
  const parts = [];
  if (sourceSearches) parts.push(`${sourceSearches === 1 ? 'In den Quellen gesucht' : `${sourceSearches}× in den Quellen gesucht`}`);
  if (sourceReads) parts.push(`${sourceReads} ${sourceReads === 1 ? 'Stelle' : 'Stellen'} in den Quellen gelesen`);
  if (reads) parts.push(`${reads} ${reads === 1 ? 'Datei' : 'Dateien'} gelesen`);
  if (lists) parts.push(`${lists} Ordner angesehen`);
  if (searches) parts.push(`${searches} ${searches === 1 ? 'Suche' : 'Suchen'}`);
  return parts.join(', ');
}
