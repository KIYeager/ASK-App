// Arbeitsschleife (ENTSCHEIDE E19): Auftrag an das Modell in ASK, Werkzeugaufrufe lokal ausführen,
// Ergebnisse zurückgeben, bis das Modell ohne Werkzeug antwortet. Chats liegen im Arbeitsordner
// unter .ask/chats/<id>.json, damit sie zum Projekt gehören.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { EventEmitter } = require('node:events');
const { toolSpecs, runTool, describe, clip } = require('./tools');
const { parseTextToolCalls } = require('./askapi');
const { assessRisk } = require('./approvals');
const { MEMORY_TOOLS } = require('./memorytools');

// Keine feste Schrittgrenze für normale Aufträge; der Schleifenschutz (loopReason) hält ein
// festgefahrenes Modell an. MAX_STEPS ist nur die Notbremse.
const MAX_STEPS = 500;
const SAME_CALL_LIMIT = 3; // derselbe Aufruf so oft hintereinander
const FAIL_LIMIT = 8; // so viele fehlgeschlagene Werkzeugaufrufe hintereinander
const GOAL_IDLE_LIMIT = 2; // Ziel-Modus: so oft ohne Arbeitsschritt anstossen
const GOAL_NUDGE = 'Das Ziel ist noch nicht erreicht. Arbeite selbständig weiter. Ist es erreicht, rufe goal_done auf. '
  + 'Brauchst du wirklich eine Antwort der Person, frag mit frage_stellen.';
const GOAL_TOOL = {
  type: 'function',
  function: {
    name: 'goal_done',
    description: 'Meldet, dass das Ziel dieses Chats vollständig erreicht ist. Erst aufrufen, wenn alles erledigt und geprüft ist.',
    parameters: { type: 'object', properties: { zusammenfassung: { type: 'string', description: 'Was erreicht wurde, ein bis zwei Sätze' } }, required: ['zusammenfassung'] },
  },
};

// Werkzeug frage_stellen: Das Modell fragt die Person im Chat und wartet auf die Antwort (wie
// AskUserQuestion in Claude Code). Die App zeigt die Frage mit Antworten zum Anklicken und einem Feld
// für eine eigene Antwort; auch eine Eingabe im normalen Eingabefeld gilt als Antwort.
const QUESTION_TOOL = {
  type: 'function',
  function: {
    name: 'frage_stellen',
    description: 'Stellt der Person eine Frage und wartet auf ihre Antwort. Nur, wenn für den Auftrag wirklich eine Entscheidung oder Angabe der Person fehlt. '
      + 'Gib wenn möglich 2 bis 4 kurze Antworten zum Anklicken an; die Person kann immer auch frei antworten.',
    parameters: {
      type: 'object',
      properties: {
        frage: { type: 'string', description: 'Die Frage, kurz und klar, in der Du-Form' },
        optionen: { type: 'array', items: { type: 'string' }, description: 'Antworten zum Anklicken, je höchstens ein paar Wörter (optional, höchstens 6)' },
        mehrfach: { type: 'boolean', description: 'true, wenn mehrere Antworten gewählt werden dürfen' },
      },
      required: ['frage'],
    },
  },
};

// Planmodus (ENTSCHEIDE E52): Das Modell schaut sich nur um (lesen, suchen, Web) und legt dann mit
// plan_vorlegen einen Plan vor. Der Auftrag endet dort; die Person setzt ihn um oder ändert ihn.
const PLAN_TOOL = {
  type: 'function',
  function: {
    name: 'plan_vorlegen',
    description: 'Legt der Person den fertigen Plan vor und beendet den Auftrag. Nur im Planmodus.',
    parameters: {
      type: 'object',
      properties: { plan: { type: 'string', description: 'Der Plan in Markdown: kurzer Titel, nummerierte Schritte, betroffene Dateien' } },
      required: ['plan'],
    },
  },
};
// Werkzeuge, die im Planmodus erlaubt sind: nichts, was den Ordner oder etwas ausserhalb verändert
const PLAN_ALLOWED = new Set(['read_file', 'list_files', 'find_files', 'search_text', 'view_file', 'web_search', 'web_fetch',
  'chat_search', 'chat_read', 'github_status', 'server_liste', 'skill_lesen', 'update_tasks', 'frage_stellen', 'plan_vorlegen']);
const PLAN_PROMPT = '\n\n# Planmodus\nDie Person will zuerst einen Plan sehen. Ändere nichts: keine Befehle, keine Dateien schreiben. '
  + 'Sieh dir mit den Lese- und Suchwerkzeugen an, was du brauchst, frag bei Unklarem mit frage_stellen, '
  + 'und lege dann mit plan_vorlegen einen kurzen, konkreten Plan vor (Schritte, betroffene Dateien). Danach hörst du auf.';

// Verbindung zu ASK weg oder ASK kurz überlastet: so lange warten und nochmals versuchen (Sekunden)
const RETRY_WAITS = [2, 5, 10, 20, 30];
function retryable(err) {
  if (!err || err.name === 'AbortError' || err.name === 'AuthError' || err.constructor?.name === 'AuthError') return false;
  if (/fünf Minuten lang nichts/.test(err.message || '')) return false; // hing schon lange, die Person entscheidet
  if (err.status) return [429, 502, 503, 504].includes(err.status);
  return /net::|fetch failed|ECONN|ETIMEDOUT|ENOTFOUND|EAI_AGAIN|socket|network|terminated|other side closed/i.test(err.message || '');
}

// Wartet ms, bricht beim Stoppen ab
function pause(ms, signal) {
  return new Promise((resolve, reject) => {
    if (signal.aborted) { reject(Object.assign(new Error('abgebrochen'), { name: 'AbortError' })); return; }
    const t = setTimeout(() => { signal.removeEventListener('abort', onAbort); resolve(); }, ms);
    const onAbort = () => { clearTimeout(t); reject(Object.assign(new Error('abgebrochen'), { name: 'AbortError' })); };
    signal.addEventListener('abort', onAbort, { once: true });
  });
}

function goalPrompt(goal) {
  return `\n\nZiel dieses Chats (von der Person mit /goal gesetzt): ${goal.text}\n`
    + 'Arbeite selbständig und ohne Rückfragen weiter, bis das Ziel vollständig erreicht und geprüft ist. '
    + 'Kündige Arbeit nicht nur an, sondern führe sie mit den Werkzeugen aus. Ist das Ziel erreicht, rufe goal_done auf.';
}
const KEEP_FULL_TOOL_RESULTS = 8; // ältere Werkzeugergebnisse gehen gekürzt an das Modell
// Wie viel vom Verlauf an das Modell geht. Ältere Schritte werden gekürzt: Ergebnisse auf «clip» Zeichen,
// lange Argumente früherer Aufrufe (der Inhalt einer geschriebenen Datei) auf «args» Zeichen; die Datei
// selbst liegt ja im Ordner. TIGHT gilt, sobald ASK meldet, dass der Verlauf zu lang für das Modell ist.
const NORMAL = { keep: KEEP_FULL_TOOL_RESULTS, clip: 1500, args: 1200 };
const TIGHT = { keep: 3, clip: 400, args: 200 };
const TOO_LONG = /context length|context window|maximum context|context_length|too many tokens|input tokens|prompt is too long|exceeds? the (model'?s? )?(maximum|max)|reduce the length/i;
const IMAGE_TURNS = 2; // Bilder aus so vielen der letzten Eingaben gehen an das Modell
const VIEW_TURNS = 2; // ebenso aus so vielen der letzten Ansichten (view_file, browser)
const CHAT_DIR = path.join('.ask', 'chats');
const NEW = 'neu'; // openLatest: leeren Chat beginnen
const { WORK_DIRS } = require('./cleanup'); // unter .ask, je ein Unterordner pro Chat

const normRel = (p) => String(p || '').trim().replace(/^\.?[\\/]+/, '').replaceAll('\\', '/').toLowerCase();
const newId = () => `${new Date().toISOString().slice(0, 19).replace(/[-:T]/g, '')}-${crypto.randomBytes(3).toString('hex')}`;

// Argumente eines Werkzeugaufrufs: «null», eine Zahl oder eine Liste werden zu {}, damit kein Schritt daran
// scheitert. Unlesbares JSON bleibt, wie es ist: runTool meldet dem Modell den Fehler.
function cleanArgs(raw) {
  const text = typeof raw === 'string' ? raw : raw && typeof raw === 'object' ? JSON.stringify(raw) : '';
  if (!text.trim()) return '{}';
  try {
    const v = JSON.parse(text);
    return v && typeof v === 'object' && !Array.isArray(v) ? text : '{}';
  } catch {
    return text;
  }
}

// Steckt das Modell in einer Schleife? Betrachtet nur den laufenden Auftrag (seit der letzten Eingabe).
function loopReason(messages) {
  const start = messages.map((m) => (m.role === 'user' && m.note !== 'view' ? 'user' : '')).lastIndexOf('user');
  const calls = [];
  const results = [];
  const output = new Map(); // Ergebnis pro Aufruf: Abfragen wie command_status mit neuer Ausgabe sind keine Schleife
  for (const m of messages.slice(start + 1)) {
    if (m.role === 'tool') { results.push(m.ok !== false); output.set(m.tool_call_id, String(m.content ?? '').slice(0, 4000)); }
  }
  for (const m of messages.slice(start + 1)) {
    if (m.role === 'assistant') for (const c of m.tool_calls || []) calls.push(`${c.function.name} ${c.function.arguments}\u0000${output.get(c.id) ?? ''}`);
  }
  const last = calls.slice(-SAME_CALL_LIMIT);
  if (last.length === SAME_CALL_LIMIT && last.every((c) => c === last[0])) {
    return `Das Modell hat ${SAME_CALL_LIMIT}-mal hintereinander genau denselben Schritt mit demselben Ergebnis ausgeführt (${last[0].split(' ')[0]}).`;
  }
  const fails = results.slice(-FAIL_LIMIT);
  if (fails.length === FAIL_LIMIT && fails.every((ok) => !ok)) {
    return `${FAIL_LIMIT} Werkzeugaufrufe hintereinander sind fehlgeschlagen.`;
  }
  return '';
}

// Werkzeuge im Systemprompt, falls ASK den Parameter «tools» nicht weitergibt (Textmodus).
function textModePrompt(specs) {
  const list = specs.map((s) => JSON.stringify(s.function)).join('\n');
  return `\n\n# Werkzeuge\nDu kannst diese Werkzeuge aufrufen:\n<tools>\n${list}\n</tools>\n`
    + 'Für einen Aufruf schreibst du genau:\n<tool_call>\n{"name": "<werkzeug>", "arguments": {...}}\n</tool_call>\n'
    + 'Danach wartest du auf das Ergebnis in <tool_response>.';
}

// Text einer Eingabe mit Anhängen: das Modell erfährt, wo die Dateien liegen.
// Mit /name aufgerufener Skill (skills.js): Die Anleitung geht vor dem Text an das Modell, die Person sieht nur ihre Eingabe.
function userText(m) {
  const files = (m.attachments || []).map((a) => a.path);
  const skill = m.skill?.text ? `[Die Person ruft ihren Skill «${m.skill.name}» auf. Anleitung des Skills:]\n${m.skill.text}\n[Ende der Anleitung]\n\n` : '';
  let text = m.content;
  if (files.length) text = `${m.content || 'Siehe Anhang.'}\n\n[Angehängt, liegt im Arbeitsordner: ${files.join(', ')}]`;
  if (m.mentions?.length) text += `\n\n[Mit @ erwähnt, liegt im Arbeitsordner: ${m.mentions.join(', ')}. Lies es, wenn du es brauchst.]`;
  return skill + text;
}

// @Datei im Eingabefeld (@pfad oder @"pfad mit Leerzeichen"): nur Pfade, die es im Ordner gibt
function mentionsIn(text, exists) {
  const out = [];
  for (const m of String(text || '').matchAll(/(?:^|\s)@(?:"([^"\n]+)"|([^\s"]+))/g)) {
    const rel = (m[1] || m[2]).replace(/[.,;:!?)]+$/, '');
    if (rel && !out.includes(rel) && exists(rel)) out.push(rel);
  }
  return out.slice(0, 20);
}

// Verlauf für die Anfrage: ohne Denktexte, alte Ergebnisse gekürzt, im Textmodus ohne tool-Rollen
// (auch für frühere Schritte, die noch mit Werkzeugliste liefen). imageUrl(anhang) liefert Bilder
// als data-URL; sie gehen nur bei den letzten IMAGE_TURNS Eingaben mit, ältere nur als Pfad.
// Lange Texte in den Argumenten eines früheren Aufrufs kürzen; das Ergebnis bleibt gültiges JSON.
function shortArgs(raw, max) {
  if (!raw || raw.length <= max) return raw;
  try {
    const args = JSON.parse(raw);
    if (!args || typeof args !== 'object') return raw;
    const cut = (v) => (typeof v === 'string' && v.length > max ? `${v.slice(0, Math.floor(max / 2))}\n… (für den Verlauf gekürzt, ${v.length} Zeichen; den heutigen Stand zeigt read_file)` : v);
    return JSON.stringify(Object.fromEntries(Object.entries(args).map(([k, v]) => [k, cut(v)])));
  } catch {
    return raw;
  }
}

function apiMessages(system, messages, textMode = false, imageUrl = null, budget = NORMAL) {
  const toolIndexes = messages.map((m, i) => (m.role === 'tool' ? i : -1)).filter((i) => i >= 0);
  const recentList = toolIndexes.slice(-budget.keep);
  const recent = new Set(recentList);
  const firstRecent = recentList.length ? recentList[0] : messages.length;
  // Aufrufe eines Schritts, dessen Ergebnisse schon gekürzt sind: auch die Argumente kürzen
  const calls = (m, i) => (i + m.tool_calls.length < firstRecent
    ? m.tool_calls.map((c) => ({ ...c, function: { ...c.function, arguments: shortArgs(c.function.arguments, budget.args) } })) : m.tool_calls);
  // Bilder: Anhänge der letzten IMAGE_TURNS Eingaben, Ansichten (view_file, browser) der letzten VIEW_TURNS
  const withImage = (view) => messages.map((m, i) => (m.role === 'user' && (m.note === 'view') === view && m.attachments?.some((a) => a.image) ? i : -1))
    .filter((i) => i >= 0);
  const withImages = imageUrl ? new Set([...withImage(false).slice(-IMAGE_TURNS), ...withImage(true).slice(-VIEW_TURNS)]) : new Set();
  const out = [{ role: 'system', content: system }];
  for (const [i, m] of messages.entries()) {
    if (m.role === 'user') {
      const urls = withImages.has(i) ? m.attachments.filter((a) => a.image).map(imageUrl).filter(Boolean) : [];
      const text = m.note === 'view' ? (urls.length ? m.content : `${m.content}: nicht mehr mitgeschickt.`) : userText(m);
      out.push({ role: 'user', content: urls.length ? [{ type: 'text', text }, ...urls.map((url) => ({ type: 'image_url', image_url: { url } }))] : text });
    }
    else if (m.role === 'assistant') {
      const old = !!m.tool_calls?.length && i + m.tool_calls.length < firstRecent;
      if (m.textMode && !old) out.push({ role: 'assistant', content: m.raw || m.content || '' });
      else if ((textMode || m.textMode) && m.tool_calls?.length) {
        const written = calls(m, i).map((c) => `<tool_call>\n{"name": ${JSON.stringify(c.function.name)}, "arguments": ${c.function.arguments || '{}'}}\n</tool_call>`);
        out.push({ role: 'assistant', content: [m.content, ...written].filter(Boolean).join('\n') });
      } else {
        const msg = { role: 'assistant', content: m.content || '' };
        if (m.tool_calls?.length) msg.tool_calls = calls(m, i);
        out.push(msg);
      }
    } else if (m.role === 'tool') {
      const content = recent.has(i) ? m.content : clip(m.content, budget.clip);
      if (m.textMode || textMode) out.push({ role: 'user', content: `<tool_response>\n${content}\n</tool_response>` });
      else out.push({ role: 'tool', tool_call_id: m.tool_call_id, content });
    }
  }
  return out;
}

class Agent extends EventEmitter {
  constructor({ ask, terminal, getFolder, getModel, getSystemPrompt, getCommandHint, getWebSearch, getThinking, github, githubTrust, visual, web, images, confirm, getImageUrl, getVision, getSnapshots, getChatMemory, getContextLimit, onContextLimit, getApprovalMode, fileExists, autoApprove, autoAnswer, getMode, retryWaits, servers, review, knowledge, getSiblings }) {
    super();
    // Modus «Auto»: Prüfer in ASK (review.js), beurteilt Befehle und heikle Schritte
    this.review = review || null;
    this.reviewedSafe = new Set(); // in diesem Chat schon als harmlos beurteilte Schritte
    this.getMode = getMode || (() => 'nachfragen'); // auto | nachfragen | plan (Eingabefeld)
    this.getSiblings = getSiblings || (() => []);
    this.retryWaits = retryWaits || RETRY_WAITS;
    this.getContextLimit = getContextLimit || (() => 0); // Kontextfenster des Modells in Token, 0: unbekannt
    this.onContextLimit = onContextLimit || (() => {}); // ASK hat die Grenze in einer Fehlermeldung genannt
    // Rückfrage vor heiklen Schritten (approvals.js): Stufe aus den Einstellungen, offene Fragen nach id
    this.getApprovalMode = getApprovalMode || (() => 'aus');
    this.fileExists = fileExists || (() => false);
    this.autoApprove = autoApprove || null; // nur in automatischen Tests
    this.approvals = new Map();
    this.questions = new Map(); // offene Fragen des Modells (frage_stellen) nach Werkzeug-id
    this.autoAnswer = autoAnswer || null; // nur in automatischen Tests
    this.knowledge = knowledge || null; // Gedächtnis und eigene Skills (memorytools.js); memoryOn() aus: ohne merken/vergessen
    this.allowed = new Set(); // «Immer in diesem Chat», pro Art (loeschen, ueberschreiben, …)
    this.madeThisTurn = new Set(); // in diesem Auftrag vom Modell geschriebene Dateien
    this.images = images || null; // bild_erstellen mit Flux (bild.js), nur wenn ASK das Werkzeug anbietet
    this.getChatMemory = getChatMemory || (() => false); // Schalter «Chats»: frühere Chats lesen (history.js)
    this.web = web || null; // web_search und web_fetch (web.js)
    this.githubTrust = githubTrust || null;
    this.visual = visual || null; // view_file und browser (visual.js)
    this.servers = servers || null; // Entwicklungsserver des Projekts (servers.js)
    this.getSnapshots = getSnapshots || (() => null); // Sicherung des Arbeitsordners (snapshot.js)
    this.lastSkipped = '';
    this.github = github;
    this.confirm = confirm || (async () => false);
    this.getImageUrl = getImageUrl || null; // Anhang → data-URL für das Modell
    this.getVision = getVision || (() => undefined); // false: Modell versteht laut ASK keine Bilder
    this.noImagesFor = new Set(); // Modelle, bei denen ASK Bilder abgelehnt hat
    this.ask = ask;
    this.terminal = terminal;
    this.getFolder = getFolder;
    this.getModel = getModel;
    this.getSystemPrompt = getSystemPrompt;
    this.getCommandHint = getCommandHint;
    this.getWebSearch = getWebSearch;
    this.getThinking = getThinking || (() => 'normal');
    this.textModeFor = new Set(); // Modelle, die Werkzeuge nur als Text bekommen
    this.tightFor = new Set(); // Chats, deren Verlauf für das Modell zu lang wurde
    this.running = false;
    this.controller = null;
    this.chat = this.blank();
  }

  // Bilder und Werkzeugliste lehnt ein bestimmtes Modell ab, nicht ASK als Ganzes: Nach einem Wechsel
  // des Modells gilt wieder der normale Weg.
  get textMode() { return this.textModeFor.has(this.getModel()); }

  set textMode(on) { if (on) this.textModeFor.add(this.getModel()); else this.textModeFor.delete(this.getModel()); }

  get noImages() { return this.noImagesFor.has(this.getModel()); }

  set noImages(on) { if (on) this.noImagesFor.add(this.getModel()); else this.noImagesFor.delete(this.getModel()); }

  blank() {
    return { id: newId(), title: '', created: Date.now(), updated: Date.now(), messages: [] };
  }

  // Hinweise und Fehler eines Auftrags bleiben im Chat stehen (das Modell bekommt sie nicht): Wer den
  // Chat später öffnet, sieht, warum ein Auftrag aufgehört hat.
  event(payload) {
    if (this.running && (payload.type === 'notice' || payload.type === 'error') && payload.text) {
      this.push({ role: 'note', kind: payload.type, content: String(payload.text), time: Date.now() });
    }
    this.emit('event', payload);
  }

  state() {
    return { id: this.chat.id, title: this.chat.title, running: this.running, undoing: !!this.undoing, textMode: this.textMode, goal: this.chat.goal || null,
      pinned: !!this.chat.pinned, context: this.context() };
  }

  // Wie voll ist das Kontextfenster? Aus den Tokenzahlen der letzten Antwort (Eingabe und Ausgabe); liefert ASK
  // keine, grob aus der Länge des Verlaufs geschätzt. limit 0: Die App kennt die Grösse des Fensters nicht.
  context() {
    const last = this.chat.messages.findLast((m) => m.role === 'assistant' && m.stats);
    const limit = Number(this.getContextLimit(this.getModel())) || 0;
    if (last?.stats.promptTokens) return { tokens: last.stats.promptTokens + (last.stats.tokens || 0), estimated: false, limit };
    if (!last) return { tokens: 0, estimated: true, limit };
    const n = this.chat.messages.length;
    if (!this.sized || this.sized.chat !== this.chat.id || this.sized.n !== n) {
      const chars = this.chat.messages.reduce((sum, m) => (m.role === 'note' ? sum : sum + String(m.content || '').length
        + (m.tool_calls || []).reduce((c, t) => c + String(t.function?.arguments || '').length, 0)), 0);
      this.sized = { chat: this.chat.id, n, tokens: Math.round(chars / 3.6) };
    }
    return { tokens: this.sized.tokens, estimated: true, limit };
  }

  // /goal: Ziel des Chats. Das Modell arbeitet weiter, bis es goal_done aufruft (siehe send).
  setGoal(text) {
    text = String(text || '').trim();
    this.chat.goal = text ? { text, done: false, since: Date.now() } : null;
    this.save();
    this.emit('changed');
  }

  chatDir() {
    const folder = this.getFolder();
    return folder ? path.join(folder, CHAT_DIR) : '';
  }

  save() {
    const dir = this.chatDir();
    if (!dir || this.chat.messages.length === 0) return;
    this.chat.updated = Date.now();
    try {
      fs.mkdirSync(dir, { recursive: true });
      const file = path.join(dir, `${this.chat.id}.json`);
      const text = JSON.stringify(this.chat);
      // Erst daneben schreiben, dann umbenennen: Ein Absturz mitten im Schreiben zerstört den Chat nicht
      const tmp = `${file}.tmp`;
      try {
        fs.writeFileSync(tmp, text);
        fs.renameSync(tmp, file);
      } catch {
        fs.rmSync(tmp, { force: true });
        fs.writeFileSync(file, text);
      }
    } catch {
      // Ordner nicht beschreibbar: Chat bleibt nur im Speicher
    }
  }

  // Chats eines Ordners (Standard: aktueller Ordner), neueste zuerst. Zusammenfassungen werden pro
  // Datei zwischengespeichert, solange sie sich nicht ändert; Chats mit Spielen können gross sein.
  list(folder = this.getFolder()) {
    const dir = folder ? path.join(folder, CHAT_DIR) : '';
    if (!dir || !fs.existsSync(dir)) return [];
    this.summaries = this.summaries || new Map();
    return fs.readdirSync(dir).filter((n) => n.endsWith('.json')).map((n) => {
      const file = path.join(dir, n);
      try {
        const mtime = fs.statSync(file).mtimeMs;
        const cached = this.summaries.get(file);
        if (cached && cached.mtime === mtime) return cached.summary;
        const c = JSON.parse(fs.readFileSync(file, 'utf8'));
        const summary = { id: c.id, title: c.title || '(ohne Titel)', updated: c.updated, pinned: !!c.pinned,
          count: c.messages.filter((m) => m.role === 'user' && !m.auto).length };
        this.summaries.set(file, { mtime, summary });
        return summary;
      } catch {
        return null;
      }
    }).filter(Boolean).sort((a, b) => (b.pinned - a.pinned) || (b.updated - a.updated)); // angeheftete zuerst
  }

  // Titel oder Anheften eines gespeicherten Chats ändern (auch in einem anderen Ordner als dem offenen).
  patch(folder, id, change) {
    if (!/^[\w-]+$/.test(String(id)) || !folder) return false;
    const current = id === this.chat.id && path.resolve(folder).toLowerCase() === path.resolve(this.getFolder() || '').toLowerCase();
    try {
      if (current) {
        change(this.chat);
        this.save();
      } else {
        const file = path.join(folder, CHAT_DIR, `${id}.json`);
        const chat = JSON.parse(fs.readFileSync(file, 'utf8'));
        change(chat);
        fs.writeFileSync(`${file}.tmp`, JSON.stringify(chat));
        fs.renameSync(`${file}.tmp`, file);
      }
    } catch {
      return false;
    }
    this.emit('changed');
    return true;
  }

  rename(folder, id, title) {
    const clean = String(title || '').replace(/\s+/g, ' ').trim().slice(0, 80);
    return !!clean && this.patch(folder, id, (chat) => { chat.title = clean; });
  }

  pin(folder, id, on) {
    return this.patch(folder, id, (chat) => { if (on) chat.pinned = true; else delete chat.pinned; });
  }

  // Letzte Eingabe zurücknehmen, um sie zu ändern oder neu zu schicken: Der Chat endet wieder davor, und der
  // Ordner geht (nach Rückfrage) auf den Stand vor diesem Auftrag. Liefert Text und Anhänge für das Eingabefeld.
  async rewind() {
    if (this.running) return { ok: false, reason: 'Es läuft gerade ein Auftrag.' };
    if (this.undoing) return { ok: false, reason: 'Es wird schon zurückgesetzt.' };
    // Während Rückfrage und Zurücksetzen startet kein Auftrag (send prüft undoing) und kein zweites Rückgängig
    this.undoing = true;
    this.emit('changed');
    try {
      return await this.rewindNow();
    } catch (err) {
      return { ok: false, reason: `Zurücknehmen nicht möglich: ${err.message}` };
    } finally {
      this.undoing = false;
      this.emit('changed');
    }
  }

  async rewindNow() {
    const index = this.chat.messages.findLastIndex((m) => m.role === 'user' && !m.auto);
    if (index < 0) return { ok: false, reason: 'In diesem Chat gibt es keine Eingabe.' };
    const message = this.chat.messages[index];
    const snapshots = this.getSnapshots();
    let restored = null;
    if (message.snapshot && snapshots && !message.undone) {
      let d = null;
      try { d = await snapshots.diff(message.snapshot); } catch { /* Sicherung gibt es nicht mehr: nur den Chat zurücknehmen */ }
      const total = d ? d.changed.length + d.added.length + d.removed.length : 0;
      if (total) {
        const names = [...d.changed, ...d.added, ...d.removed];
        const answer = await this.confirm('Auch die Dateien auf den Stand vor dieser Eingabe zurücksetzen?',
          `Seit dieser Eingabe geändert, angelegt oder gelöscht:\n${names.slice(0, 12).map((f) => `  ${f}`).join('\n')}${names.length > 12 ? `\n  … und ${names.length - 12} weitere` : ''}`
          + `\n\nJa: Der Ordner ist danach wie vor der Eingabe. Nein: Die Eingabe wird nicht zurückgenommen.${this.siblingWarning()}`);
        if (!answer) return { ok: false, cancelled: true };
        restored = await snapshots.restore(message.snapshot);
      }
    }
    this.chat.messages = this.chat.messages.slice(0, index);
    if (!this.chat.messages.length) this.chat.title = ''; // der Titel kam aus dieser Eingabe
    if (this.chat.goal?.done) this.chat.goal = { ...this.chat.goal, done: false };
    this.sized = null;
    if (this.chat.messages.length) this.save();
    else { try { fs.rmSync(path.join(this.chatDir(), `${this.chat.id}.json`), { force: true }); } catch { /* bleibt */ } }
    this.emit('changed');
    return { ok: true, text: message.content || '', attachments: message.attachments || [], failed: restored?.failed || [] };
  }

  // Neuer Chat oder anderer Chat: Rückfallmodi neu prüfen (Textmodus, keine Bilder)
  resetModes() {
    this.textMode = false;
    this.noImages = false;
  }

  newChat() {
    if (this.running || this.undoing) return false;
    this.allowed.clear();
    this.reviewedSafe.clear();
    this.resetModes();
    this.chat = this.blank();
    this.emit('changed');
    return true;
  }

  // Beim Ordnerwechsel: gewünschten Chat (id oder NEW) oder sonst den letzten Chat des Ordners öffnen.
  openLatest(wanted = null) {
    if (this.running || this.undoing) return; // Ordnerwechsel sind während eines Auftrags gesperrt (main.js)
    if (wanted && wanted !== NEW && this.open(wanted)) return;
    const latest = wanted === NEW ? null : this.list()[0];
    if (!latest || !this.open(latest.id)) {
      this.chat = this.blank();
      this.emit('changed');
    }
  }

  // Löscht einen gespeicherten Chat. Ist es der offene, beginnt ein neuer.
  remove(folder, id) {
    if (!/^[\w-]+$/.test(id)) return false;
    const current = id === this.chat.id && path.resolve(folder).toLowerCase() === path.resolve(this.getFolder() || '').toLowerCase();
    if (current && this.running) return false;
    try {
      fs.rmSync(path.join(folder, CHAT_DIR, `${id}.json`));
    } catch {
      return false;
    }
    // Skripte und Zwischendateien dieses Chats gehören mit dazu
    for (const sub of WORK_DIRS) fs.rmSync(path.join(folder, '.ask', sub, id), { recursive: true, force: true });
    if (current) this.newChat();
    return true;
  }

  open(id) {
    if (this.running || this.undoing || !/^[\w-]+$/.test(id)) return false;
    try {
      this.allowed.clear();
    this.reviewedSafe.clear();
      this.resetModes();
      this.chat = JSON.parse(fs.readFileSync(path.join(this.chatDir(), `${id}.json`), 'utf8'));
      this.emit('changed');
      return true;
    } catch {
      return false;
    }
  }

  stop() {
    if (this.controller) this.controller.abort();
  }

  push(message) {
    this.chat.messages.push(message);
  }

  // attachments: bereits in .ask/anhaenge/<Chat>/ gespeicherte Anhänge (siehe attachments.js)
  // extra.skill: { name, text } eines mit /name aufgerufenen Skills
  async send(text, attachments = [], extra = {}) {
    text = String(text || '').trim();
    if ((!text && !attachments.length) || this.running || this.undoing) return;
    if (!this.chat.title) this.chat.title = (text || attachments.map((a) => a.name).join(', ')).replace(/\s+/g, ' ').slice(0, 80);
    const message = { role: 'user', content: text, time: Date.now() };
    if (attachments.length) message.attachments = attachments;
    const mentions = mentionsIn(text, (rel) => this.fileExists(rel));
    if (mentions.length) message.mentions = mentions;
    if (extra?.skill?.text) message.skill = { name: String(extra.skill.name), text: String(extra.skill.text) };
    this.push(message);
    this.event({ type: 'user', text, attachments });
    this.turnSources = { read: [], found: [] }; // web_fetch und web_search in diesem Auftrag
    this.madeThisTurn = new Set();
    this.running = true;
    this.controller = new AbortController();
    const { signal } = this.controller;
    this.emit('changed');
    const others = this.getSiblings();
    if (others.length) {
      this.event({ type: 'notice', text: `Im selben Projekt arbeitet gerade auch ${others.map((t) => `«${clip(t, 60)}»`).join(', ')}. `
        + 'Beide Chats ändern dieselben Dateien, und «Rückgängig» setzt die Änderungen beider zurück.' });
    }
    const snapshots = this.getSnapshots();
    let failed = false;
    try {
      // Stand des Ordners vor dem Auftrag, für «Rückgängig»
      if (snapshots) await this.backup(snapshots, message);
      let idle = 0; // Anstösse ohne Arbeitsschritt hintereinander (Ziel-Modus)
      for (let step = 0; step < MAX_STEPS; step++) {
        if (signal.aborted) break;
        // Websuche nur bei der ersten Anfrage eines Auftrags, sonst sucht ASK bei jedem Werkzeugschritt neu.
        const done = await this.step(signal, step === 0 && !!this.getWebSearch?.());
        this.save();
        if (done) {
          // Ziel-Modus: aufgehört, ohne das Ziel als erreicht zu melden? Dann anstossen, höchstens
          // zweimal ohne Arbeitsschritt dazwischen; danach wartet das Modell offenbar auf die Person.
          const goal = this.chat.goal;
          // Im Planmodus fehlt goal_done: dort wartet der Plan auf die Person, nicht auf einen Anstoss
          if (!goal || goal.done || signal.aborted || this.getMode() === 'plan') break;
          if (idle >= GOAL_IDLE_LIMIT) {
            this.event({ type: 'notice', text: 'Das Modell arbeitet nicht weiter am Ziel und wartet offenbar auf dich. Antworte oder schreib «weiter».' });
            break;
          }
          idle++;
          this.push({ role: 'user', content: GOAL_NUDGE, auto: true, time: Date.now() });
          this.event({ type: 'user', text: GOAL_NUDGE, auto: true });
          continue;
        }
        idle = 0;
        const loop = loopReason(this.chat.messages);
        if (loop) {
          this.event({ type: 'notice', text: `Angehalten: ${loop} Schreib, wie es weitergehen soll, oder «weiter».` });
          break;
        }
        if (step === MAX_STEPS - 1) this.event({ type: 'notice', text: `Nach ${MAX_STEPS} Schritten angehalten. Schreib «weiter», um fortzufahren.` });
      }
    } catch (err) {
      failed = true;
      if (!signal.aborted && err.name !== 'AbortError') this.event({ type: 'error', text: err.message });
    } finally {
      if (signal.aborted) this.event({ type: 'notice', text: 'Gestoppt.' });
      if (snapshots && message.snapshot) await this.recordChanges(snapshots, message);
      this.running = false;
      this.controller = null;
      this.save();
      this.event({ type: 'done', stopped: signal.aborted, failed }); // main.js: danach im Hintergrund merken (memory.js)
      this.emit('changed');
    }
  }

  // Eine Antwort des Modells und die verlangten Werkzeuge. true, wenn das Modell fertig ist.
  async step(signal, webSearch = false) {
    const model = this.getModel();
    if (!model) throw new Error('Kein Modell gewählt.');
    const goal = this.chat.goal && !this.chat.goal.done ? this.chat.goal : null;
    const memory = !!this.getChatMemory();
    const flux = !!this.images?.available();
    const remember = !this.knowledge?.memoryOn || !!this.knowledge.memoryOn();
    const offered = (name) => (memory || !name.startsWith('chat_')) && (flux || name !== 'bild_erstellen') && (remember || !MEMORY_TOOLS.includes(name));
    const plan = this.getMode() === 'plan';
    let specs = [...toolSpecs(this.getCommandHint(this.chat.id)).filter((s) => offered(s.function.name)), QUESTION_TOOL, ...(goal ? [GOAL_TOOL] : [])];
    if (plan) specs = [...specs, PLAN_TOOL].filter((s) => PLAN_ALLOWED.has(s.function.name));
    let sentImages = false;
    const attempt = () => {
      const system = this.getSystemPrompt(this.chat.id) + (goal ? goalPrompt(goal) : '') + (plan ? PLAN_PROMPT : '') + (this.textMode ? textModePrompt(specs) : '');
      const imageUrl = this.canSee() ? this.getImageUrl : null;
      const messages = apiMessages(system, this.chat.messages, this.textMode, imageUrl, this.tightFor.has(this.chat.id) ? TIGHT : NORMAL);
      sentImages = messages.some((m) => Array.isArray(m.content));
      this.event({ type: 'assistant-start', webSearch });
      return this.ask.chat({
        model,
        messages,
        tools: this.textMode ? null : specs,
        webSearch,
        thinking: this.getThinking(),
        signal,
        onDelta: (kind, text) => this.event({ type: kind, text }),
      });
    };
    // Bricht die Verbindung ab, wartet die App und versucht es nochmals (die halbe Antwort verfällt)
    const request = async () => {
      for (let n = 0; ; n++) {
        try {
          return await attempt();
        } catch (err) {
          if (signal.aborted || n >= this.retryWaits.length || !retryable(err)) throw err;
          const wait = this.retryWaits[n];
          this.event({ type: 'retry', attempt: n + 1, of: this.retryWaits.length, wait,
            text: `Verbindung zu ASK unterbrochen. Neuer Versuch in ${wait} s (${n + 1}/${this.retryWaits.length}).` });
          await pause(wait * 1000, signal);
        }
      }
    };
    let completion;
    try {
      completion = await request();
    } catch (err) {
      this.keepPartial(err);
      // Der Verlauf ist zu lang für das Modell: ältere Schritte stärker kürzen und nochmals
      const limit = err.status >= 400 && TOO_LONG.test(err.message) ? Number(/(?:context length|context window|maximum)[^\d]{0,40}(\d{4,8})/i.exec(err.message)?.[1]) : 0;
      if (limit) this.onContextLimit(model, limit); // für die Anzeige, wie voll der Verlauf ist
      if (err.status >= 400 && TOO_LONG.test(err.message) && !this.tightFor.has(this.chat.id)) {
        this.tightFor.add(this.chat.id);
        this.event({ type: 'notice', text: 'Der Chat ist für das Modell zu lang geworden. Ältere Arbeitsschritte gehen ab jetzt stark gekürzt an das Modell; für ein neues Thema besser einen neuen Chat beginnen (/neu).' });
        completion = await request();
      } else if (err.status >= 400 && TOO_LONG.test(err.message)) {
        throw new Error('Der Chat ist zu lang für das Modell, auch gekürzt. Beginne einen neuen Chat (/neu); mit dem Schalter «Chats» kann das Modell diesen hier nachlesen.');
      } else if (sentImages && err.status >= 400 && /image|vision|multimodal|modalit/i.test(err.message)) {
        // Nimmt das Modell keine Bilder an, gehen sie künftig nur als Pfad mit.
        this.noImages = true;
        this.event({ type: 'notice', text: 'Das Modell in ASK nimmt keine Bilder an. Die Bilder liegen als Datei bei den Anhängen, das Modell kann sie nur so bearbeiten, nicht ansehen.' });
        completion = await request();
      } else if (!this.textMode && err.status === 400 && /tool/i.test(err.message)) {
        // Manche Server lehnen «tools» ab: dann Werkzeuge im Text beschreiben.
        this.textMode = true;
        this.event({ type: 'notice', text: 'ASK nimmt keine Werkzeugliste an, die App beschreibt die Werkzeuge im Text.' });
        completion = await request();
      } else {
        throw err;
      }
    }

    let { content } = completion;
    let calls = completion.calls;
    let textMode = false;
    if (calls.length === 0 && content.includes('<tool_call>')) {
      const parsed = parseTextToolCalls(content);
      if (parsed.calls.length) {
        calls = parsed.calls;
        textMode = true;
        this.textMode = true;
        content = parsed.text;
      }
    }
    const toolCalls = calls.map((c) => ({ id: c.id || `call_${crypto.randomBytes(6).toString('hex')}`, type: 'function',
      function: { name: String(c.name || ''), arguments: cleanArgs(c.arguments) } }));
    const message = { role: 'assistant', content, time: Date.now(), stats: completion.stats() };
    // Quellen unter der Schlussantwort: Websuche von ASK und die Seiten, die das Modell mit web_fetch
    // gelesen hat (ohne gelesene Seiten die ersten Treffer von web_search)
    let sources = completion.sources;
    if (!toolCalls.length && this.turnSources) {
      const web = this.turnSources.read.length ? this.turnSources.read : this.turnSources.found.slice(0, 5);
      sources = [...sources, ...web].filter((s, i, all) => s.url && all.findIndex((t) => t.url === s.url) === i).slice(0, 10);
    }
    if (sources.length) message.sources = sources;
    if (completion.reasoning) message.reasoning = completion.reasoning;
    if (toolCalls.length) message.tool_calls = toolCalls;
    if (textMode) { message.textMode = true; message.raw = completion.content; }
    this.push(message);
    this.event({ type: 'assistant-end', content, stats: message.stats, sources: message.sources || [], calls: toolCalls.map((c) => ({ id: c.id, name: c.function.name, ...this.describeCall(c) })) });
    if (completion.incomplete) {
      this.event({ type: 'notice', text: 'Die Antwort von ASK endete ohne Abschluss; sie ist vielleicht unvollständig. Schreib «weiter», falls etwas fehlt.' });
    }
    if (completion.finish === 'length') {
      this.event({ type: 'notice', text: 'ASK hat die Antwort wegen der Längengrenze abgeschnitten. Schreib «weiter», oder lass grosse Dateien in mehreren Teilen schreiben.' });
    }
    if (toolCalls.length === 0) {
      if (!content.trim()) this.event({ type: 'notice', text: 'Das Modell hat keine Antwort geschrieben.' });
      return true;
    }

    const seen = []; // Bilder aus view_file und browser, gehen danach als Nachricht an das Modell
    const canSee = this.canSee();
    let planned = false;
    for (const call of toolCalls) {
      if (signal.aborted) {
        this.push({ role: 'tool', tool_call_id: call.id, name: call.function.name, content: 'Von der Person abgebrochen.', ok: false, summary: 'abgebrochen', textMode });
        continue;
      }
      const started = Date.now();
      this.event({ type: 'tool-start', id: call.id, name: call.function.name, ...this.describeCall(call) });
      const name = call.function.name;
      // Ein Fehler in der App darf den Verlauf nicht kaputt machen: Jeder Aufruf bekommt ein Ergebnis,
      // sonst lehnt ASK jede weitere Anfrage dieses Chats ab (tool_calls ohne Antwort)
      let result;
      try {
        const verdict = plan && !PLAN_ALLOWED.has(name) ? 'plan' : await this.approve(call, signal);
        result = verdict === 'plan'
          ? { ok: false, summary: 'im Planmodus nicht erlaubt', output: 'Im Planmodus änderst du nichts. Lege mit plan_vorlegen einen Plan vor.' }
          : name === 'plan_vorlegen'
          ? this.proposePlan(call.function.arguments)
          : verdict !== 'yes'
          ? { ok: false, summary: 'abgelehnt', output: signal.aborted ? 'Von der Person abgebrochen.'
            : 'Die Person hat diesen Schritt abgelehnt. Mach ohne ihn weiter oder frag kurz nach, was sie stattdessen möchte.' }
          : call.function.name === 'goal_done'
          ? this.finishGoal(call.function.arguments)
          : call.function.name === 'frage_stellen'
          ? await this.askQuestion(call, signal)
          : await runTool(this.terminal, call.function.name, call.function.arguments, {
            signal,
            onProgress: (output) => this.event({ type: 'tool-progress', id: call.id, output: clip(output, 4000) }),
            github: this.github,
            githubTrust: this.githubTrust,
            visual: this.visual,
            servers: this.servers,
            web: this.web,
            images: flux ? this.images : null,
            chatMemory: memory,
            onLive: (data) => this.event({ type: 'live', id: call.id, ...data }),
            canSee,
            chat: this.chat.id,
            chatTitle: this.chat.title,
            knowledge: this.knowledge,
            folder: this.getFolder(),
            confirm: this.confirm,
          });
      } catch (err) {
        result = { ok: false, summary: 'Fehler in der App', output: `Fehler in der App: ${err?.message || err}` };
      }
      const images = result.images || [];
      if (result.ok !== false && call.function.name === 'write_file') {
        try { this.madeThisTurn.add(normRel(JSON.parse(call.function.arguments || '{}').path)); } catch { /* ohne Pfad */ }
      }
      if (result.sources && this.turnSources) this.turnSources[call.function.name === 'web_fetch' ? 'read' : 'found'].push(...result.sources);
      if (result.plan) planned = true;
      this.push({ role: 'tool', tool_call_id: call.id, name: call.function.name, content: result.output, ok: result.ok,
        summary: result.summary, display: clip(result.display ?? result.output, 6000), tasks: result.tasks, images, textMode, ms: Date.now() - started,
        ...(result.plan ? { plan: result.plan } : {}) });
      this.event({ type: 'tool-end', id: call.id, ok: result.ok, summary: result.summary,
        output: clip(result.display ?? result.output, 6000), tasks: result.tasks, images, ms: Date.now() - started, plan: result.plan || null });
      if (canSee && images.length && result.view !== false) seen.push({ call: `${call.function.name}(${this.describeCall(call).detail})`, images });
    }
    if (seen.length && !signal.aborted) {
      this.push({ role: 'user', auto: true, note: 'view', time: Date.now(),
        content: `[Ansicht] Bilder zu ${seen.map((s) => s.call).join(', ')}`,
        attachments: seen.flatMap((s) => s.images).map((p) => ({ path: p, name: path.basename(p), image: true })) });
    }
    return planned; // nach plan_vorlegen ist der Auftrag zu Ende, die Person entscheidet
  }

  // Werkzeug plan_vorlegen: Plan im Chat zeigen (Knöpfe «Umsetzen» und «Ändern» in der Oberfläche)
  proposePlan(rawArgs) {
    let text = '';
    try { text = String(JSON.parse(rawArgs || '{}').plan || '').trim(); } catch { /* unten */ }
    if (!text) return { ok: false, output: 'plan fehlt.', summary: 'ohne Plan' };
    return { ok: true, plan: clip(text, 20000), summary: 'Plan vorgelegt',
      output: 'Der Plan ist der Person vorgelegt. Hör jetzt auf und warte auf ihre Antwort.' };
  }

  // Rückfrage vor heiklen Schritten: 'yes' oder 'no'. Wartet auf die Antwort der Person im Chat
  // (answerApproval); Stoppen gilt als Nein.
  async approve(call, signal) {
    let args = {};
    try { args = JSON.parse(call.function.arguments || '{}'); } catch { return 'yes'; } // runTool meldet den Fehler
    const ctx = { exists: (rel) => this.fileExists(rel), madeThisTurn: (rel) => this.madeThisTurn.has(normRel(rel)) };
    // Auto: der Prüfer entscheidet; Nachfragen: Stufe aus den Einstellungen (vor jedem Befehl)
    const risk = this.getMode() === 'auto'
      ? await this.autoRisk(call, args, ctx, signal)
      : assessRisk(call.function.name, args, { ...ctx, mode: this.getApprovalMode() });
    if (signal.aborted) return 'no';
    if (!risk || this.allowed.has(risk.kind)) return 'yes';
    const id = `frage_${crypto.randomBytes(4).toString('hex')}`;
    this.event({ type: 'approval', id, kind: risk.kind, title: risk.title, detail: risk.detail, reason: risk.reason, reviewed: !!risk.reviewed });
    let answer;
    if (this.autoApprove) answer = await this.autoApprove(risk);
    else {
      answer = await new Promise((resolve) => {
        const onAbort = () => resolve('no');
        signal.addEventListener('abort', onAbort, { once: true });
        this.approvals.set(id, (a) => { signal.removeEventListener('abort', onAbort); resolve(a); });
      });
    }
    this.approvals.delete(id);
    if (answer === 'always') this.allowed.add(risk.kind);
    const verdict = answer === 'yes' || answer === 'always' ? 'yes' : 'no';
    this.event({ type: 'approval-end', id, answer: verdict === 'yes' ? answer : 'no' });
    return verdict;
  }

  // Modus «Auto»: Befehle und heikle Schritte (Löschen, ausserhalb, Überschreiben) prüft ein Modell in
  // ASK. Harmlos: ohne Rückfrage. Gefährlich: Rückfrage mit dem Grund. Ist die Prüfung nicht möglich
  // (Fehler, keine verwertbare Antwort), gelten die festen Regeln (approvals.js, Stufe «heikel»).
  async autoRisk(call, args, ctx, signal) {
    const name = call.function.name;
    const rule = assessRisk(name, args, { ...ctx, mode: 'heikel' });
    const command = name === 'run_command' ? args.command : name === 'server_starten' ? (args.befehl || args.command) : null;
    if (command === null && !rule) return null; // Lesen, Suchen, neue Dateien im Projekt: ohne Prüfung
    if (!this.review) return rule;
    const key = `${name}:${command ?? args.path ?? ''}`;
    if (this.reviewedSafe.has(key)) return null;
    this.event({ type: 'review', id: call.id });
    let verdict = null;
    try {
      const task = [...this.chat.messages].reverse().find((m) => m.role === 'user' && !m.auto)?.content || '';
      verdict = await this.review({ name, args, task, folder: this.getFolder(), signal });
    } catch {
      verdict = null;
    }
    if (signal.aborted) return null;
    if (!verdict) return rule && { ...rule, reason: [rule.reason, 'Die Prüfung durch ASK war nicht möglich.'].filter(Boolean).join(' ') };
    if (!verdict.dangerous) { this.reviewedSafe.add(key); return null; }
    const detail = rule?.detail || String(command ?? args.path ?? '').replace(/\s+/g, ' ').slice(0, 300);
    return { kind: rule?.kind ? `${rule.kind}:${key}` : `gefahr:${key}`, title: rule?.title || (command !== null ? 'Diesen Befehl ausführen?' : 'Diesen Schritt ausführen?'),
      detail, reason: verdict.reason || 'ASK hält den Schritt für gefährlich.', reviewed: true };
  }

  // Werkzeug frage_stellen: Frage im Chat zeigen und auf die Antwort warten (answerQuestion)
  async askQuestion(call, signal) {
    let args = {};
    try { args = JSON.parse(call.function.arguments || '{}'); } catch { /* unten */ }
    const frage = String(args.frage || args.question || '').trim();
    if (!frage) return { ok: false, output: 'frage fehlt.', summary: 'ohne Frage' };
    const optionen = (Array.isArray(args.optionen) ? args.optionen : Array.isArray(args.options) ? args.options : [])
      .map((o) => String(typeof o === 'object' && o ? o.text || o.label || '' : o).trim()).filter(Boolean).slice(0, 6);
    const id = call.id;
    this.event({ type: 'question', id, frage, optionen, mehrfach: !!args.mehrfach });
    let answer;
    if (this.autoAnswer) answer = await this.autoAnswer({ frage, optionen });
    else {
      answer = await new Promise((resolve) => {
        const onAbort = () => resolve(null);
        signal.addEventListener('abort', onAbort, { once: true });
        this.questions.set(id, (a) => { signal.removeEventListener('abort', onAbort); resolve(a); });
      });
    }
    this.questions.delete(id);
    if (answer === null || answer === undefined) {
      this.event({ type: 'question-end', id, answer: '' });
      return { ok: false, output: 'Die Person hat nicht geantwortet (gestoppt).', summary: 'nicht beantwortet' };
    }
    const text = String(answer).trim() || '(leer)';
    this.event({ type: 'question-end', id, answer: text });
    return { ok: true, output: `Antwort der Person: ${text}`, summary: clip(text, 200), display: `Frage: ${frage}\nAntwort: ${text}` };
  }

  // Antwort der Person auf eine Frage des Modells
  answerQuestion(id, text) {
    const resolve = this.questions.get(String(id));
    if (!resolve) return false;
    resolve(String(text ?? '').slice(0, 4000));
    return true;
  }

  // Offene Frage oder Rückfrage (für die Oberfläche nach dem Neuladen)
  pending() {
    return { questions: [...this.questions.keys()], approvals: [...this.approvals.keys()] };
  }

  // Antwort der Person auf eine Rückfrage: 'yes' | 'always' | 'no'
  answerApproval(id, answer) {
    const resolve = this.approvals.get(String(id));
    if (!resolve) return false;
    resolve(['yes', 'always', 'no'].includes(answer) ? answer : 'no');
    return true;
  }

  // Was vom Modell schon angekommen war, als die Antwort abbrach (Stopp, Verbindung weg), bleibt im Chat:
  // Die Person hat es gesehen, und das Modell soll beim nächsten Auftrag wissen, wie weit es war.
  keepPartial(err) {
    const p = err?.partial;
    if (!p || !(String(p.content || '').trim() || String(p.reasoning || '').trim())) return;
    const message = { role: 'assistant', content: p.content || '', time: Date.now(), stats: p.stats(), stopped: true };
    if (p.reasoning) message.reasoning = p.reasoning;
    this.push(message);
  }

  // Darf das Modell Bilder bekommen? (ASK meldet Bildfähigkeit nicht ab und hat keine abgelehnt)
  canSee() {
    return !!this.getImageUrl && !this.noImages && this.getVision() !== false;
  }

  async backup(snapshots, message) {
    try {
      const stand = await snapshots.take(this.chat.title);
      message.snapshot = stand.id;
      const skipped = stand.skipped.join('\n');
      if (skipped && skipped !== this.lastSkipped) {
        this.event({ type: 'notice', text: `Nicht gesichert (grösser als 100 MB oder gesperrt): ${stand.skipped.slice(0, 5).join(', ')}`
          + `${stand.skipped.length > 5 ? ` und ${stand.skipped.length - 5} weitere` : ''}. Änderungen daran lassen sich nicht rückgängig machen.` });
      }
      this.lastSkipped = skipped;
    } catch (err) {
      this.event({ type: 'notice', text: `Der Ordner wurde nicht gesichert: ${err.message} Änderungen dieses Auftrags lassen sich nicht rückgängig machen.` });
    }
  }

  // Nach dem Auftrag: was hat sich im Ordner geändert? Anzeige mit Knopf «Rückgängig».
  async recordChanges(snapshots, message) {
    try {
      const d = await snapshots.diff(message.snapshot);
      if (!d.changed.length && !d.added.length && !d.removed.length) return;
      message.changes = { changed: d.changed.slice(0, 50), added: d.added.slice(0, 50), removed: d.removed.slice(0, 50),
        counts: [d.changed.length, d.added.length, d.removed.length] };
      this.event({ type: 'changes', snapshot: message.snapshot, changes: message.changes });
    } catch {
      // Sicherung verschwunden: kein Rückgängig
    }
  }

  // Ordner auf den Stand vor einem Auftrag zurücksetzen (nach Rückfrage). Das Modell erfährt es im Verlauf.
  // Während des Zurücksetzens (Rückfrage, Sicherung, Kopieren) startet kein Auftrag und kein anderer Chat.
  async undo(id) {
    if (this.running) return { ok: false, reason: 'Es läuft gerade ein Auftrag.' };
    if (this.undoing) return { ok: false, reason: 'Es wird schon zurückgesetzt.' };
    this.undoing = true;
    try {
      return await this.undoNow(id);
    } catch (err) {
      return { ok: false, reason: `Zurücksetzen nicht möglich: ${err.message}` };
    } finally {
      this.undoing = false;
      this.emit('changed');
    }
  }

  async undoNow(id) {
    const message = this.chat.messages.find((m) => m.role === 'user' && m.snapshot === id);
    const snapshots = this.getSnapshots();
    if (!message || !snapshots) return { ok: false, reason: 'Zu diesem Auftrag gibt es keine Sicherung.' };
    let d;
    try {
      d = await snapshots.diff(id);
    } catch {
      return { ok: false, reason: 'Die Sicherung gibt es nicht mehr (sie wird nach 14 Tagen gelöscht) oder ist nicht lesbar.' };
    }
    const total = d.changed.length + d.added.length + d.removed.length;
    if (!total) return { ok: false, reason: 'Seit diesem Auftrag hat sich im Ordner nichts geändert.' };
    const list = (title, files) => (files.length ? `${title}:\n${files.slice(0, 12).map((f) => `  ${f}`).join('\n')}${files.length > 12 ? `\n  … und ${files.length - 12} weitere` : ''}` : '');
    const detail = [list('Wird auf den alten Stand gesetzt', d.changed), list('Wird gelöscht (neu seit dem Auftrag)', d.added),
      list('Wird wiederhergestellt', d.removed)].filter(Boolean).join('\n\n');
    // Rückgängig bis zu einem früheren Schritt: Was spätere Aufträge geändert haben, geht mit
    const later = this.chat.messages.slice(this.chat.messages.indexOf(message) + 1).filter((m) => m.role === 'user' && m.snapshot && !m.undone);
    const warn = later.length ? `Auch die Änderungen ${later.length === 1 ? 'des Auftrags' : `der ${later.length} Aufträge`} danach gehen dabei verloren.`
      : 'Auch spätere Änderungen an diesen Dateien gehen dabei verloren.';
    if (!(await this.confirm('Ordner auf den Stand vor diesem Auftrag zurücksetzen?', `${detail}\n\n${warn}${this.siblingWarning()}`))) {
      return { ok: false, cancelled: true };
    }
    const r = await snapshots.restore(id);
    message.undone = true;
    for (const m of later) m.undone = true;
    const done = total - r.failed.length;
    const text = `Rückgängig gemacht: ${done} ${done === 1 ? 'Datei' : 'Dateien'} auf den Stand vor dem Auftrag gesetzt.`
      + (r.failed.length ? ` Nicht möglich (wohl geöffnet): ${r.failed.slice(0, 5).join(', ')}.` : '');
    this.push({ role: 'user', auto: true, note: 'undo', summary: text, time: Date.now(),
      content: `[Hinweis der App: Die Person hat den Auftrag «${clip(message.content || 'Anhang', 80)}» rückgängig gemacht. Der Arbeitsordner ist wieder auf dem Stand vor diesem Auftrag; was dabei geändert oder angelegt wurde, gibt es so nicht mehr.]` });
    this.save();
    this.event({ type: 'undo', snapshot: id, text });
    return { ok: true, text };
  }

  // Zusatz für die Rückfrage vor dem Zurücksetzen, wenn ein anderer Chat gerade im selben Ordner arbeitet
  siblingWarning() {
    const others = this.getSiblings();
    return others.length ? `\n\nAchtung: Gerade arbeitet auch ${others.map((t) => `«${clip(t, 60)}»`).join(', ')} in diesem Ordner. `
      + 'Was dieser Chat geändert hat, wird ebenfalls zurückgesetzt.' : '';
  }

  // Werkzeug goal_done: Ziel als erreicht markieren
  finishGoal(rawArgs) {
    let summary = '';
    try { summary = String(JSON.parse(rawArgs || '{}').zusammenfassung || ''); } catch { /* ohne */ }
    if (!this.chat.goal) return { ok: false, output: 'Es ist kein Ziel gesetzt.', summary: 'kein Ziel' };
    this.chat.goal = { ...this.chat.goal, done: true, summary, doneAt: Date.now() };
    this.emit('changed');
    return { ok: true, output: 'Ziel als erreicht markiert. Schreib jetzt die kurze Schlussantwort.', summary: summary || 'Ziel erreicht' };
  }

  describeCall(call) {
    let args = {};
    try { args = JSON.parse(call.function.arguments || '{}'); } catch { /* roh */ }
    if (call.function.name === 'frage_stellen') {
      const optionen = Array.isArray(args.optionen) ? args.optionen.map((o) => String(o).trim()).filter(Boolean).slice(0, 6) : [];
      return { label: 'Frage', detail: String(args.frage || ''), optionen };
    }
    if (call.function.name === 'plan_vorlegen') {
      const first = String(args.plan || '').split('\n').map((l) => l.replace(/^#+\s*/, '').trim()).find(Boolean) || '';
      return { label: 'Plan', detail: clip(first, 120) };
    }
    return describe(call.function.name, args);
  }

  // Für die Anzeige eines gespeicherten Chats.
  transcript() {
    return this.chat.messages.filter((m) => m.note !== 'view').map((m) => {
      if (m.role === 'note') return { role: m.kind === 'error' ? 'error' : 'notice', text: m.content };
      if (m.role === 'assistant') {
        return { role: 'assistant', content: m.content, reasoning: m.reasoning || '', stats: m.stats || null, sources: m.sources || [], time: m.time, stopped: !!m.stopped,
          calls: (m.tool_calls || []).map((c) => ({ id: c.id, name: c.function.name, ...this.describeCall(c) })) };
      }
      if (m.role === 'tool') return { role: 'tool', id: m.tool_call_id, ok: m.ok !== false, summary: m.summary || '', output: m.display ?? m.content, tasks: m.tasks, images: m.images || [], plan: m.plan || null };
      if (m.note === 'undo') return { role: 'undo', text: m.summary };
      return { role: m.auto ? 'auto' : m.role, content: m.content, time: m.time, attachments: m.attachments || [],
        snapshot: m.snapshot || null, changes: m.changes || null, undone: !!m.undone };
    });
  }
}

module.exports = { Agent, mentionsIn, apiMessages, shortArgs, textModePrompt, loopReason, NEW, NORMAL, TIGHT, TOO_LONG };
