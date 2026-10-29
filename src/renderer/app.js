// Oberfläche von ASK: Chat mit dem Modell, Seitenleiste, Dialoge, Anmeldung bei ASK.
const $ = (id) => document.getElementById(id);
const api = window.ask;

let state = null;
let files = { top: [], recent: [] };
let selected = null; // { path, modified }
let openDialog = null; // manuell geöffnet: 'ask' | 'terminal' | 'preview'
let startError = '';
let askFailed = false;
let chatId = null;
let running = false;
let runStarted = 0;
let activityLabel = 'Denkt nach …';
let current = null; // laufende Antwort { el, body, text, textEl, thinkEl, reasoning }
const toolRows = new Map();
let turnStats = []; // Tokenzahlen und Zeiten der Schritte des laufenden Auftrags
const sent = []; // eigene Eingaben für Pfeil nach oben
let sentIndex = -1;

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') node.className = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null && v !== false) node.setAttribute(k, v === true ? '' : v);
  }
  for (const c of children) if (c !== null && c !== undefined) node.append(c instanceof Node ? c : document.createTextNode(String(c)));
  return node;
}

function baseName(p) {
  return String(p || '').split(/[\\/]/).filter(Boolean).pop() || p;
}

function formatSize(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function formatTime(ms) {
  const diff = (Date.now() - ms) / 1000;
  if (diff < 60) return 'gerade eben';
  if (diff < 3600) return `vor ${Math.round(diff / 60)} Min.`;
  const d = new Date(ms);
  const sameDay = d.toDateString() === new Date().toDateString();
  return sameDay ? d.toLocaleTimeString('de-CH', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('de-CH', { day: '2-digit', month: '2-digit', year: 'numeric' });
}

// ---------- Status ----------

const TERMINAL_TEXT = {
  gestoppt: ['Terminal gestoppt', ''],
  startet: ['Terminal startet …', 'warn'],
  läuft: ['Terminal läuft', 'ok'],
  fehler: ['Terminal: Fehler', 'bad'],
};

function askText(a) {
  switch (a.state) {
    case 'angemeldet': return [`ASK: ${a.user || 'angemeldet'}`, 'ok'];
    case 'nicht-angemeldet': return ['ASK: nicht angemeldet', 'warn'];
    case 'fehler': return ['ASK: Fehler', 'bad'];
    case 'pruefen': return ['ASK: Anmeldung wird geprüft', 'warn'];
    default: return ['ASK: wird geprüft', ''];
  }
}

function ready() {
  return state && state.folder && state.terminal.state === 'läuft' && state.ask.state === 'angemeldet' && !!state.model;
}

function blocker() {
  if (!state?.folder) return 'Zuerst einen Arbeitsordner wählen.';
  if (state.ask.state !== 'angemeldet') return 'Zuerst bei ASK anmelden.';
  if (state.terminal.state === 'startet') return 'Das Terminal startet …';
  if (state.terminal.state !== 'läuft') return 'Das Terminal läuft nicht (unten auf «Terminal» klicken).';
  if (!state.model) return 'ASK bietet kein Modell an.';
  return '';
}

function renderStatus() {
  const s = state;
  $('folder-name').textContent = s.folder || 'Kein Ordner';
  $('folder-btn').title = s.folder ? `${s.folder}\nKlicken für weitere Ordner` : 'Arbeitsordner wählen';
  $('welcome-folder').textContent = s.folder ? `Ordner: ${s.folder}` : 'Noch kein Arbeitsordner gewählt.';

  const t = s.terminal;
  const [tText, tClass] = TERMINAL_TEXT[t.state] || [t.state, ''];
  $('terminal-text').textContent = tText;
  $('terminal-pill').className = `pill ${tClass}`;
  $('terminal-pill').title = t.state === 'läuft'
    ? `Läuft im Ordner ${t.folder}${t.lowIntegrity ? '. Ausserhalb des Ordners kann nichts geschrieben werden.' : ''}`
    : (t.error || 'Terminal');

  const [aText, aClass] = askText(s.ask);
  $('ask-text').textContent = aText;
  $('ask-pill').className = `pill ${aClass}`;
  $('ask-pill').title = s.ask.message || '';

  const select = $('model-select');
  const ids = s.models.map((m) => m.id).join('|');
  if (select.dataset.ids !== ids) {
    select.replaceChildren(...s.models.map((m) => el('option', { value: m.id }, m.name)));
    select.dataset.ids = ids;
  }
  select.value = s.model;
  select.disabled = s.models.length === 0;
  renderThinking(s.thinking || 'standard');
  const web = $('web-btn');
  web.disabled = !s.webSearchAvailable;
  web.classList.toggle('on', s.webSearch && s.webSearchAvailable);
  web.title = !s.webSearchAvailable ? 'Websuche ist in ASK für dein Konto nicht freigegeben'
    : s.webSearch ? 'Websuche an: ASK sucht vor der Antwort im Web (/web zum Ausschalten)' : 'Websuche aus (/web zum Einschalten)';

  const memory = $('memory-btn');
  memory.classList.toggle('on', !!s.chatMemory);
  memory.title = s.chatMemory ? 'Frühere Chats an: das Modell sieht die Liste der früheren Chats in diesem Ordner und kann sie durchsuchen und lesen (/chats zum Ausschalten)'
    : 'Frühere Chats aus: jeder Chat beginnt ohne Wissen über andere Chats (/chats zum Einschalten)';

  const setup = s.warningAccepted && !!s.askUrl;
  $('folder-btn').disabled = !setup;
  $('new-btn').disabled = running || !s.folder;
  document.body.classList.toggle('no-sidebar', !s.sidebarOpen);
  document.body.classList.toggle('no-chatbar', !s.chatbarOpen);
  renderComposer();
}

function renderComposer() {
  const block = blocker();
  const input = $('input');
  input.placeholder = running ? 'Das Modell arbeitet … (Esc stoppt)' : (block || 'Was soll ich im Arbeitsordner erledigen?');
  $('input-box').classList.toggle('disabled', !!block && !running);
  const send = $('send-btn');
  send.textContent = running ? '■' : '↵';
  send.title = running ? 'Stoppen (Esc)' : 'Senden (Enter)';
  send.classList.toggle('stop', running);
  send.disabled = !running && !!block;
  $('activity').hidden = !running;
  $('welcome').hidden = $('messages').childElementCount > 0;
  updateUndo();
  renderTodo();
  renderGoal();
}

// ---------- Denkaufwand (Regler wie «Reasoning Effort» in ASK) ----------

const THINK_LEVELS = ['standard', 'tief', 'mittel', 'hoch', 'maximal'];
const THINK_NAMES = { standard: 'Standard', tief: 'Tief', mittel: 'Mittel', hoch: 'Hoch', maximal: 'Maximal' };

function renderThinking(level) {
  $('think-label').textContent = THINK_NAMES[level] || 'Standard';
  $('think-value').textContent = THINK_NAMES[level] || 'Standard';
  $('think-pop').classList.toggle('standard', level === 'standard');
  if (level !== 'standard') $('think-range').value = String(THINK_LEVELS.indexOf(level));
}

function toggleThinkPop(open = $('think-pop').hidden) {
  $('think-pop').hidden = !open;
  if (open) $('think-range').focus();
  reportBrowser(); // die Browser-Ansicht läge sonst über dem Regler
}

// ---------- Ziel (/goal) ----------

let goal = null; // { text, done, summary } des offenen Chats

function renderGoal() {
  const bar = $('goal-bar');
  bar.hidden = !goal;
  if (!goal) return;
  $('goal-text').textContent = goal.text;
  $('goal-text').title = goal.text;
  bar.classList.toggle('done', !!goal.done);
  $('goal-state').textContent = goal.done ? '✓ erreicht' : running ? 'in Arbeit' : 'offen';
  $('goal-state').title = goal.summary || '';
}

async function handleGoal(arg) {
  if (!arg) {
    addMessage('notice', '◎', goal ? `Ziel: ${goal.text}${goal.done ? ' (erreicht)' : ''}` : 'Kein Ziel gesetzt. /goal <Ziel> setzt eines; das Modell arbeitet dann, bis es erreicht ist.');
    return;
  }
  if (/^(aus|stopp|stop|löschen|off|clear)$/i.test(arg)) {
    await api.chatGoal('');
    return;
  }
  if (blocker()) return;
  await api.chatGoal(arg);
  await sendText(arg);
}

// ---------- Anmeldung und Dialoge ----------

function activeDialog() {
  if (!state) return null;
  if (!state.warningAccepted) return 'warning';
  if (!state.askUrl) return 'url';
  if (!state.folder) return 'start';
  return openDialog;
}

// Bis die App weiss, ob man angemeldet ist, deckt ein Ladebildschirm alles ab. Die ASK-Seite selbst
// ist nur sichtbar, wenn sicher ist, dass man sich anmelden muss.
function needsLogin() {
  return !!state?.askUrl && ['nicht-angemeldet', 'unbekannt', 'pruefen'].includes(state.ask.state);
}

function showAskPage() {
  return state?.ask.state === 'nicht-angemeldet';
}

function renderDialogs() {
  const currentDialog = activeDialog();
  for (const name of ['warning', 'url', 'start', 'ask', 'github', 'terminal', 'preview']) {
    $(`dlg-${name}`).hidden = name !== currentDialog;
  }
  $('overlay').hidden = !currentDialog;
  const login = needsLogin() && state.warningAccepted;
  $('login-area').hidden = !login;
  $('login-area').classList.toggle('loading', login && !showAskPage());
  if (!askFailed) $('ask-placeholder').textContent = state.ask.state === 'pruefen' ? 'Anmeldung wird geprüft …' : 'Verbinde mit ASK …';
  api.setAskVisible(login && showAskPage() && !currentDialog && !askFailed);
  reportBrowser(); // Dialoge liegen sonst unter der Browser-Ansicht
  if (login) reportBounds();
  if (currentDialog === 'start') renderStart();
  if (currentDialog === 'ask') renderAskDialog();
  if (currentDialog === 'terminal') renderTerminalDialog();
  if (currentDialog === 'url') $('url-input').value = $('url-input').value || state.askUrl || '';
}

function renderStart() {
  const list = $('recent-list');
  list.replaceChildren();
  for (const folder of state.recent) {
    list.append(el('li', {},
      el('button', { class: 'link', title: folder, onclick: () => openFolder(() => api.openFolder(folder)) }, folder),
      el('button', { class: 'btn small', title: 'Aus der Liste entfernen', onclick: () => api.forgetFolder(folder) }, 'Entfernen')));
  }
  $('recent-title').hidden = state.recent.length === 0;
  const err = startError || (state.terminal.state === 'fehler' ? state.terminal.error : '');
  $('start-error').textContent = err;
  $('start-error').hidden = !err;
}

async function openFolder(action) {
  startError = '';
  const res = await action();
  if (res && !res.ok && !res.canceled) {
    startError = res.reason;
    if (state.folder) alertBox(res.reason);
  }
  render();
}

function alertBox(text) {
  openDialog = 'terminal';
  $('terminal-detail').textContent = text;
  $('terminal-logs').hidden = true;
  render();
}

function renderAskDialog() {
  const a = state.ask;
  const p = (text) => el('p', {}, text);
  const body = $('ask-body');
  body.replaceChildren(p(`Adresse: ${state.askUrl}`));
  if (a.state === 'angemeldet') {
    body.append(p(`Angemeldet als ${a.user || '(unbekannt)'}. ${state.models.length} Modell(e) verfügbar.`),
      p('Die App schickt deine Aufträge direkt an das gewählte Modell. Befehle und Dateizugriffe führt sie selbst auf diesem Laptop aus, nur im Arbeitsordner.'));
  } else if (a.state === 'fehler') {
    body.append(p(`ASK meldet einen Fehler: ${a.message || 'unbekannt'}`));
  } else {
    body.append(p('Nicht angemeldet. Die Anmeldeseite von ASK erscheint im Hauptfenster.'));
  }
  $('ask-logout').hidden = a.state !== 'angemeldet';
}

// ---------- GitHub (/github) ----------

async function showGithub() {
  openDialog = 'github';
  render();
  const s = await api.githubStatus();
  const p = (...parts) => el('p', {}, ...parts);
  const body = $('github-body');
  body.replaceChildren();
  if (s.connected && s.login) {
    body.append(p('Verbunden als ', el('strong', {}, s.login), '.'));
    if (s.linked) {
      body.append(p('Dieser Ordner gehört zu ', el('a', { href: s.url, onclick: (e) => { e.preventDefault(); api.openLink(s.url); } }, s.repo),
        `. Hier geändert: ${s.changed.length + s.added.length + s.deleted.length} Dateien${s.remoteNewer ? '; auf GitHub gibt es Neueres' : ''}.`));
    } else {
      body.append(p('Dieser Ordner ist mit keinem Repository verbunden. Sag dem Modell zum Beispiel «Hol das Repository besitzer/name» oder «Leg das Projekt auf GitHub an».'));
    }
    body.append(p(el('span', { class: 'muted small' }, 'Im Chat: «lade die Änderungen auf GitHub hoch», «hol die neuesten Änderungen von GitHub».')));
    if (s.trusted?.length) {
      body.append(p('Hochladen ohne Rückfrage:'), el('ul', { class: 'trusted' }, ...s.trusted.map((repo) => el('li', {}, `${repo} `,
        el('button', { class: 'link-btn', onclick: async () => { await api.githubUntrust(repo); showGithub(); } }, 'wieder fragen')))));
    }
  } else if (s.message) {
    body.append(p(`GitHub meldet: ${s.message}`));
  }
  const connected = !!(s.connected && s.login);
  $('github-connect').hidden = connected;
  $('github-ok').hidden = connected;
  $('github-disconnect').hidden = !connected;
  $('github-error').hidden = true;
  $('github-token').value = '';
}

async function renderTerminalDialog() {
  const t = state.terminal;
  if (t.state !== 'fehler' && $('terminal-logs').hidden) return;
  $('terminal-logs').hidden = false;
  $('terminal-detail').textContent = t.state === 'läuft'
    ? `Das Terminal läuft im Ordner ${t.folder}. ${t.lowIntegrity ? 'Ausserhalb des Ordners kann es nichts schreiben oder löschen.' : ''}`
    : (t.error || `Status: ${t.state}`);
  $('terminal-logs').textContent = await api.terminalLogs() || '(keine Meldungen)';
}

// ---------- Chats (linke Leiste) ----------

let groups = [];
const collapsed = new Set();

async function loadChats() {
  groups = await api.chatGroups();
  renderChats();
}

function shortTime(ms) {
  const diff = (Date.now() - ms) / 1000;
  if (diff < 60) return 'jetzt';
  if (diff < 3600) return `${Math.round(diff / 60)} Min.`;
  if (diff < 86400) return `${Math.round(diff / 3600)} Std.`;
  if (diff < 7 * 86400) return `${Math.round(diff / 86400)} T.`;
  return new Date(ms).toLocaleDateString('de-CH', { day: '2-digit', month: '2-digit' });
}

async function openChat(folder, id) {
  const res = await api.chatOpen(folder, id);
  if (res && res.ok === false && res.reason) alertBox(res.reason);
}

function chatItem(group, chat) {
  const active = group.current && chat.id === chatId;
  const item = el('li', {
    class: `chat-item${active ? ' active' : ''}`, title: `${chat.title}\n${chat.count} ${chat.count === 1 ? 'Auftrag' : 'Aufträge'} · ${formatTime(chat.updated)}\nRechtsklick: löschen`,
    onclick: () => { if (!active) openChat(group.folder, chat.id); },
    oncontextmenu: async (e) => {
      e.preventDefault();
      if (await api.chatDelete(group.folder, chat.id, chat.title)) loadChats();
    },
  },
  active && running ? el('span', { class: 'spinner', 'aria-hidden': 'true' }, '✻') : null,
  el('span', { class: 'ctitle' }, chat.title),
  el('span', { class: 'cwhen' }, shortTime(chat.updated)));
  return item;
}

function renderChats() {
  const query = $('chat-search').value.trim().toLowerCase();
  const nodes = [];
  for (const group of groups) {
    let chats = group.chats;
    // Der offene Chat ist erst gespeichert, wenn er eine Nachricht hat.
    if (group.current && !chats.some((c) => c.id === chatId)) {
      chats = [{ id: chatId, title: 'Neuer Chat', updated: Date.now(), count: 0, placeholder: true }, ...chats];
    }
    if (query) chats = chats.filter((c) => !c.placeholder && c.title.toLowerCase().includes(query));
    if (query && chats.length === 0) continue;
    const isCollapsed = !query && collapsed.has(group.folder);
    const list = el('ul', { class: 'chat-items' }, ...chats.map((c) => {
      const node = chatItem(group, c);
      if (c.placeholder) node.classList.add('placeholder');
      return node;
    }));
    if (!chats.length) list.append(el('li', { class: 'chat-empty' }, 'Noch keine Chats'));
    nodes.push(el('section', { class: `chat-group${group.current ? ' current' : ''}${isCollapsed ? ' collapsed' : ''}`, 'data-folder': group.folder },
      el('div', { class: 'group-head', title: group.folder, onclick: () => {
        if (collapsed.has(group.folder)) collapsed.delete(group.folder); else collapsed.add(group.folder);
        renderChats();
      } },
      el('span', { class: 'caret', 'aria-hidden': 'true' }, '▾'),
      el('span', { class: 'gname' }, group.name),
      el('button', { class: 'gnew', title: `Neuer Chat in ${group.name}`, onclick: (e) => {
        e.stopPropagation();
        api.chatNew(group.folder).then((res) => { if (res && res.ok === false && res.reason) alertBox(res.reason); });
      } }, '+')),
      list));
  }
  if (!nodes.length) nodes.push(el('div', { class: 'chat-empty' }, query ? 'Keine Treffer' : 'Noch keine Chats'));
  $('chat-groups').replaceChildren(...nodes);
}

// ---------- Chat ----------

const transcriptEl = () => $('transcript');

function nearBottom() {
  const t = transcriptEl();
  return t.scrollHeight - t.scrollTop - t.clientHeight < 120;
}

function scrollDown(force = false) {
  if (force || nearBottom()) requestAnimationFrame(() => { transcriptEl().scrollTop = transcriptEl().scrollHeight; });
}

function addMessage(cls, mark, ...content) {
  // Alles ausser Gedanken und weiteren Leseschritten beendet eine Gruppe von Leseschritten.
  if (!/^(assistant|tool group)/.test(cls)) closeGroup();
  const stick = nearBottom();
  const node = el('div', { class: `msg ${cls}` }, el('span', { class: 'mark' }, mark), el('div', { class: 'body' }, ...content));
  $('messages').append(node);
  renderComposer();
  scrollDown(stick);
  return node;
}

let turnNode = null; // Eingabe der Person, mit der der laufende Auftrag begann
let turnStart = 0;

function addUser(text, time = Date.now(), attachments = []) {
  turnNode = addMessage('user', '›', text || null, attachmentList(attachments));
  turnStart = time;
}

// Fertiger Auftrag: Arbeitsschritte zu einer Zeile zusammenklappen (wie «Worked for …» in Codex).
// Sichtbar bleiben die Schlussantwort, Hinweise, Fehler, Quellen und die Token-Zeile.
function foldTurn(endTime = Date.now()) {
  if (!turnNode) return;
  const nodes = [];
  for (let n = turnNode.nextElementSibling; n; n = n.nextElementSibling) nodes.push(n);
  const answer = [...nodes].reverse().find((n) => n.classList.contains('assistant') && n.querySelector('.md'));
  const steps = nodes.filter((n) => n !== answer && (n.classList.contains('tool') || n.classList.contains('assistant')
    || n.classList.contains('auto')));
  const tools = steps.filter((n) => n.classList.contains('tool'));
  turnNode = null;
  if (tools.length < 4) return;
  const failed = tools.filter((n) => n.classList.contains('bad')).length;
  const minutes = Math.round((endTime - turnStart) / 60000);
  const parts = [`${tools.length} Arbeitsschritte`];
  if (turnStart && endTime > turnStart) parts.push(minutes >= 1 ? `${minutes} Min.` : `${Math.max(1, Math.round((endTime - turnStart) / 1000))} s`);
  if (failed) parts.push(`${failed} fehlgeschlagen`);
  const fold = el('div', { class: 'msg fold', title: 'Klicken zeigt die einzelnen Schritte' },
    el('span', { class: 'mark' }, '▸'), el('div', { class: 'body' }, parts.join(' · ')));
  fold.addEventListener('click', () => {
    const open = fold.classList.toggle('open');
    fold.querySelector('.mark').textContent = open ? '▾' : '▸';
    steps.forEach((n) => n.classList.toggle('folded', !open));
  });
  steps[0].before(fold);
  steps.forEach((n) => n.classList.add('folded'));
}

// Datei im Arbeitsordner zu einem Inline-Code wie `berichte/bericht.docx` oder `bericht.docx`
function fileFor(text) {
  const rel = String(text).trim().replace(/^\.?[\\/]/, '').replaceAll('\\', '/').toLowerCase();
  if (!rel || rel.length > 260) return null;
  const all = [...files.recent, ...files.top].filter((f) => !f.dir);
  return all.find((f) => f.path.replaceAll('\\', '/').toLowerCase() === rel)
    || files.recent.find((f) => f.name.toLowerCase() === rel) || null;
}

function markFiles(root) {
  root.querySelectorAll('.md code').forEach((c) => {
    if (!c.closest('pre')) c.classList.toggle('file', !!fileFor(c.textContent));
  });
}

function markdown(target, text) {
  target.innerHTML = window.renderMarkdown(text);
  markFiles(target);
}

function ensureAssistant() {
  if (!current) current = { text: '', reasoning: '' };
  if (!current.el) {
    current.el = addMessage('assistant', '●');
    current.body = current.el.querySelector('.body');
  }
  return current;
}

function appendReasoning(text) {
  const c = ensureAssistant();
  c.reasoning += text;
  if (!c.thinkEl) {
    c.thinkPre = el('pre');
    c.thinkEl = el('details', { class: 'thinking', open: verbose }, el('summary', {}, '✻ Gedanken'), c.thinkPre);
    c.body.prepend(c.thinkEl);
  }
  c.thinkPre.textContent = c.reasoning;
  activityLabel = 'Denkt nach …';
  scrollDown();
}

let textFrame = 0;
function appendText(text) {
  const c = ensureAssistant();
  c.text += text;
  closeGroup();
  if (!c.textEl) {
    c.textEl = el('div', { class: 'md' });
    c.body.append(c.textEl);
  }
  activityLabel = 'Schreibt …';
  if (!textFrame) {
    textFrame = requestAnimationFrame(() => {
      textFrame = 0;
      if (current?.textEl) markdown(current.textEl, current.text);
      scrollDown();
    });
  }
}

function endAssistant(content) {
  if (!current && !content) return;
  const c = ensureAssistant();
  if (content) {
    closeGroup();
    if (!c.textEl) {
      c.textEl = el('div', { class: 'md' });
      c.body.append(c.textEl);
    }
    markdown(c.textEl, content);
    // Ganze Antwort kopieren (als Markdown, wie bei anderen Chat-Programmen)
    c.body.append(el('div', { class: 'msg-actions' }, el('button', { class: 'copy', type: 'button', 'data-text': content }, '⧉ Kopieren')));
  } else if (c.textEl) {
    c.textEl.remove();
    c.textEl = null;
  }
  if (!c.textEl && !c.thinkEl) c.el.remove();
  else if (!c.textEl) c.el.classList.add('thought-only');
  current = null;
}

// ---------- Werkzeugzeilen wie in Claude Code ----------
// Pro Aufruf eine Zeile mit dem Aufruf und eine mit dem Kurzresultat; Befehl, Ausgabe und Diff erst
// auf Klick (oder mit Ctrl+O für alle). Lese- und Suchschritte hintereinander ergeben eine Zeile.
const READ_ONLY = new Set(['read_file', 'list_files', 'find_files', 'search_text']);
let openGroup = null;

function closeGroup() {
  openGroup = null;
}

const firstLine = (text) => String(text || '').split('\n').map((l) => l.trim()).find((l) => l && l !== '(keine Ausgabe)') || '';

function shortSummary(name, ok, summary, output) {
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

function detailBlock(text, diff) {
  const pre = el('pre', { class: 'out' });
  const lines = String(text || '').replace(/\s+$/, '').split('\n');
  if (diff) {
    for (const line of lines) pre.append(el('span', { class: line.startsWith('+ ') ? 'add' : line.startsWith('- ') ? 'del' : '' }, line));
  } else {
    pre.textContent = lines.join('\n');
  }
  return pre;
}

function todoList(tasks) {
  const sym = { erledigt: '☒', in_arbeit: '▸', offen: '☐' };
  return el('ul', { class: 'todo' }, ...tasks.map((t) => el('li', { class: t.status }, `${sym[t.status] || '☐'} ${t.titel}`)));
}

function toolNode(cls) {
  const head = el('div', { class: 'call', title: 'Klicken für Details (Ctrl+O: alle)' });
  const summary = el('span', { class: 'summary' }, 'läuft …');
  const result = el('div', { class: 'result' }, el('span', { class: 'elbow' }, '⎿'), el('div', { class: 'res' }, summary));
  const details = el('div', { class: 'details' });
  const node = addMessage(cls, '●', head, result, details);
  node.addEventListener('click', (e) => {
    if (!e.target.closest('.details') && !window.getSelection().toString()) node.classList.toggle('expanded');
  });
  return { node, head, result, summary, details };
}

function callHead(label, detail) {
  return [el('span', { class: 'name' }, label), detail ? el('span', { class: 'arg' }, `(${detail})`) : ''];
}

function toolCall(id, name, label, detail) {
  activityLabel = `${label} …`;
  if (READ_ONLY.has(name)) return groupCall(id, name, label, detail);
  closeGroup();
  const row = { ...toolNode('tool running'), name, label };
  row.head.replaceChildren(...callHead(label, detail));
  if (detail) row.details.append(el('pre', { class: 'cmd' }, detail));
  toolRows.set(id, row);
  return row;
}

function groupCall(id, name, label, detail) {
  if (!openGroup) openGroup = { ...toolNode('tool group running'), items: [] };
  const group = openGroup;
  const item = { name, label, detail, ok: null, summary: '', sub: el('div', { class: 'sub' }) };
  group.items.push(item);
  group.details.append(item.sub);
  const row = { group, item, name, label };
  toolRows.set(id, row);
  renderGroup(group);
  return row;
}

function renderGroup(group) {
  const count = (names) => group.items.filter((i) => names.includes(i.name)).length;
  const reads = count(['read_file']);
  const lists = count(['list_files']);
  const searches = count(['find_files', 'search_text']);
  const runningAny = group.items.some((i) => i.ok === null);
  const fails = group.items.filter((i) => i.ok === false);
  group.node.classList.toggle('running', runningAny);
  group.node.classList.toggle('bad', !runningAny && fails.length > 0);
  for (const i of group.items) {
    i.sub.replaceChildren(...callHead(i.label, i.detail), el('span', { class: 'subsum' }, i.ok === null ? ' läuft …' : ` ⎿ ${i.summary}`));
    i.sub.classList.toggle('bad', i.ok === false);
  }
  if (group.items.length === 1) {
    const [i] = group.items;
    group.head.replaceChildren(...callHead(i.label, i.detail));
    group.summary.textContent = i.ok === null ? 'läuft …' : i.summary;
    return;
  }
  const parts = [];
  if (reads) parts.push(`${reads} ${reads === 1 ? 'Datei' : 'Dateien'} gelesen`);
  if (lists) parts.push(`${lists} Ordner angesehen`);
  if (searches) parts.push(`${searches} ${searches === 1 ? 'Suche' : 'Suchen'}`);
  group.head.replaceChildren(el('span', { class: 'name' }, parts.join(', ')));
  group.summary.textContent = runningAny ? 'läuft …'
    : fails.length ? `${fails.length} fehlgeschlagen · ${fails[0].summary}` : group.items.map((i) => i.detail).filter(Boolean).join(', ');
}

function toolProgress(row, output) {
  if (row.group) return;
  row.summary.textContent = firstLine(String(output).split('\n').filter((l) => l.trim()).slice(-1)[0]) || 'läuft …';
  const live = row.details.querySelector('pre.out') || row.details.appendChild(el('pre', { class: 'out' }));
  live.textContent = output;
}

// Bilder, die das Modell mit view_file oder browser_test angesehen hat
function shotList(images) {
  const fileUrl = (p) => `askfile://datei/${p.split('/').map(encodeURIComponent).join('/')}`;
  return el('div', { class: 'shots' }, ...images.map((p) => el('img', { src: fileUrl(p), alt: '', title: 'Klicken für die Vorschau',
    onclick: (e) => { e.stopPropagation(); showPreview({ path: p, name: baseName(p) }); } })));
}

function toolResult(row, { ok, summary, output, tasks, name, images }, live = false) {
  const short = shortSummary(name, ok, summary, output);
  if (row.group) {
    row.item.ok = ok !== false;
    row.item.summary = short;
    if (ok === false && output) row.item.sub.after(detailBlock(output));
    renderGroup(row.group);
    return;
  }
  row.node.classList.remove('running');
  row.node.classList.toggle('bad', ok === false);
  row.summary.textContent = short;
  if (tasks && tasks.length) {
    row.details.append(todoList(tasks));
    if (live) setTodo(tasks, true);
  }
  if (images && images.length) row.result.querySelector('.res').append(shotList(images));
  row.details.querySelector('pre.out')?.remove();
  if (output && !(tasks && tasks.length)) row.details.append(detailBlock(output, name === 'edit_file'));
}

// Ausführliche Ansicht (Ctrl+O): Gedanken und alle Details aufgeklappt
let verbose = false;
function setVerbose(on) {
  verbose = on;
  document.body.classList.toggle('verbose', on);
  document.querySelectorAll('details.thinking').forEach((d) => { d.open = on; });
}

const num = (n) => Math.round(n).toLocaleString('de-CH');
const sec = (ms) => (ms / 1000).toLocaleString('de-CH', { minimumFractionDigits: 1, maximumFractionDigits: 1 });

// Zusammenfassung eines Auftrags: Schritte, Ausgabetoken, Token pro Sekunde, Zeit des Modells.
// Aufgabenliste über dem Eingabefeld wie in Claude Code: aus dem letzten update_tasks des Chats.
// Während eines Auftrags offen (Ctrl+T blendet aus), danach eine Zeile, die sich aufklappen lässt.
let todo = { tasks: [], live: false }; // live: im laufenden Auftrag gesetzt
let todoOpen = true; // während eines Auftrags
let todoPeek = false; // nach dem Auftrag aufgeklappt
const TODO_SHOWN = 7;

function setTodo(tasks, live) {
  todo = { tasks: Array.isArray(tasks) ? tasks : [], live };
  renderTodo();
}

// Neuer Auftrag: eine fertige Liste verschwindet, eine offene bleibt, bis das Modell sie aktualisiert
function newTurnTodo() {
  todoPeek = false;
  setTodo(todo.tasks.every((t) => t.status === 'erledigt') ? [] : todo.tasks, false);
}

function toggleTodo() {
  if (!todo.tasks.length) return;
  if (running && todo.live) todoOpen = !todoOpen;
  else todoPeek = !todoPeek;
  renderTodo();
}

function renderTodo() {
  const box = $('todo');
  const tasks = todo.tasks;
  box.hidden = tasks.length === 0;
  if (!tasks.length) return;
  const done = tasks.filter((t) => t.status === 'erledigt').length;
  const current = tasks.find((t) => t.status === 'in_arbeit');
  const open = running && todo.live ? todoOpen : todoPeek;
  box.classList.toggle('open', open);
  box.classList.toggle('done', done === tasks.length);
  $('todo-caret').textContent = open ? '▾' : '▸';
  $('todo-fill').style.width = `${Math.round((done / tasks.length) * 100)}%`;
  $('todo-count').textContent = `${done}/${tasks.length}`;
  $('todo-current').textContent = done === tasks.length ? '✓ alle erledigt' : current ? current.titel : '';
  // Bei langen Listen: zwei erledigte vor der aktuellen, dann die nächsten
  let from = 0;
  if (tasks.length > TODO_SHOWN) {
    const at = Math.max(0, tasks.findIndex((t) => t.status !== 'erledigt'));
    from = Math.min(Math.max(0, at - 2), tasks.length - TODO_SHOWN);
  }
  const shown = tasks.slice(from, from + TODO_SHOWN);
  const mark = { erledigt: '✓', in_arbeit: '▸', offen: '○' };
  const items = shown.map((t) => el('li', { class: t.status }, el('span', { class: 'tmark', 'aria-hidden': 'true' }, mark[t.status] || '○'), el('span', { class: 'ttext' }, t.titel)));
  if (from > 0) items.unshift(el('li', { class: 'more' }, `… ${from} erledigt`));
  const rest = tasks.length - from - shown.length;
  if (rest > 0) items.push(el('li', { class: 'more' }, `… und ${rest} weitere`));
  $('todo-list').replaceChildren(...items);
}

// Was ein Auftrag im Ordner geändert hat. «Rückgängig» nur beim letzten Auftrag mit Änderungen.
function addChanges(snapshot, changes, undone = false) {
  if (!snapshot || !changes) return;
  const [changed, added, removed] = changes.counts || [changes.changed.length, changes.added.length, changes.removed.length];
  const parts = [];
  if (changed) parts.push(`${changed} geändert`);
  if (added) parts.push(`${added} neu`);
  if (removed) parts.push(`${removed} gelöscht`);
  const names = [...changes.added, ...changes.changed, ...changes.removed];
  const short = names.slice(0, 3).map(baseName).join(', ') + (names.length > 3 ? ', …' : '');
  const btn = el('button', { class: 'undo-btn', title: 'Ordner auf den Stand vor diesem Auftrag zurücksetzen' }, '↶ Rückgängig');
  const node = addMessage(`changes${undone ? ' undone' : ''}`, '⎿', el('span', {}, `Dateien: ${parts.join(' · ')}`),
    el('span', { class: 'muted' }, ` (${short})`), el('span', { class: 'undone-label muted' }, ' · rückgängig gemacht'), btn);
  node.dataset.snapshot = snapshot;
  node.title = [...changes.changed.map((f) => `geändert: ${f}`), ...changes.added.map((f) => `neu: ${f}`), ...changes.removed.map((f) => `gelöscht: ${f}`)].join('\n');
  btn.onclick = async () => {
    if (running) return;
    btn.disabled = true;
    const res = await api.chatUndo(snapshot);
    btn.disabled = false;
    if (res && !res.ok && !res.cancelled) addMessage('error', '✗', res.reason);
  };
  updateUndo();
}

function updateUndo() {
  const rows = [...document.querySelectorAll('.msg.changes')];
  const last = rows.filter((r) => !r.classList.contains('undone')).pop();
  for (const r of rows) r.querySelector('.undo-btn').hidden = r !== last || running;
}

function onUndo(ev) {
  document.querySelectorAll('.msg.changes').forEach((r) => { if (r.dataset.snapshot === ev.snapshot) r.classList.add('undone'); });
  addMessage('notice', '↶', ev.text);
  updateUndo();
}

function addStats(list) {
  list = list.filter(Boolean);
  if (!list.length) return;
  const tokens = list.reduce((n, s) => n + s.tokens, 0);
  const gen = list.reduce((n, s) => n + s.genMs, 0);
  const total = list.reduce((n, s) => n + s.totalMs, 0);
  const approx = list.some((s) => s.estimated) ? '≈ ' : '';
  const parts = [`${list.length} ${list.length === 1 ? 'Schritt' : 'Schritte'}`, `${approx}${num(tokens)} Token`];
  if (gen > 0) parts.push(`${approx}${num(tokens / (gen / 1000))} Token/s`);
  parts.push(`Modell ${sec(total)} s`);
  const steps = list.map((s, i) => `Schritt ${i + 1}: ${s.estimated ? '≈ ' : ''}${num(s.tokens)} Token in ${sec(s.genMs)} s`
    + `${s.genMs > 0 ? ` (${num(s.tokens / (s.genMs / 1000))} Token/s)` : ''}, erste Antwort nach ${sec(s.firstMs)} s`
    + `${s.promptTokens ? `, Eingabe ${num(s.promptTokens)} Token` : ''}`);
  if (approx) steps.push('≈ geschätzt aus der Textlänge: ASK hat keine Tokenzahlen geliefert.');
  const node = addMessage('stats', '✻', parts.join(' · '));
  node.title = steps.join('\n');
}

// Quellen der Websuche unter der Antwort
function addSources(sources) {
  if (!sources || !sources.length) return;
  const host = (url) => { try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return ''; } };
  addMessage('sources', '⎿', el('div', {}, `Websuche: ${sources.length} ${sources.length === 1 ? 'Quelle' : 'Quellen'}`),
    el('ol', {}, ...sources.slice(0, 10).map((s) => el('li', {},
      el('a', { href: s.url, title: s.url, onclick: (e) => { e.preventDefault(); api.openLink(s.url); } }, s.title),
      el('span', { class: 'host' }, ` · ${host(s.url)}`)))));
}

function onEvent(ev) {
  switch (ev.type) {
    case 'user':
      if (ev.auto) {
        addMessage('notice auto', '↻', 'Weiter am Ziel …');
        break;
      }
      turnStats = [];
      newTurnTodo();
      addUser(ev.text, Date.now(), ev.attachments);
      scrollDown(true);
      break;
    case 'assistant-start':
      current = null;
      activityLabel = ev.webSearch ? 'Sucht im Web …' : 'Denkt nach …';
      break;
    case 'reasoning': appendReasoning(ev.text); break;
    case 'text': appendText(ev.text); break;
    case 'assistant-end':
      endAssistant(ev.content);
      addSources(ev.sources);
      turnStats.push(ev.stats);
      break;
    case 'tool-start':
      toolCall(ev.id, ev.name, ev.label, ev.detail);
      break;
    case 'tool-progress': {
      const row = toolRows.get(ev.id);
      if (row) toolProgress(row, ev.output);
      scrollDown();
      break;
    }
    case 'tool-end': {
      const row = toolRows.get(ev.id);
      if (row) toolResult(row, { ...ev, name: row.name }, true);
      activityLabel = 'Denkt nach …';
      scrollDown();
      break;
    }
    case 'changes': addChanges(ev.snapshot, ev.changes); break;
    case 'live': onLive(ev); break;
    case 'undo': onUndo(ev); break;
    case 'notice': addMessage('notice', '⎿', ev.text); break;
    case 'error': addMessage('error', '✗', ev.text); break;
    case 'done':
      current = null;
      addStats(turnStats);
      turnStats = [];
      foldTurn();
      break;
    default:
  }
}

// Gespeicherten Chat anzeigen
function renderTranscript(items) {
  $('messages').replaceChildren();
  toolRows.clear();
  current = null;
  closeGroup();
  const names = new Map();
  let stats = [];
  let lastTime = 0;
  let turn = null; // Eingabe, mit der der Auftrag begann (Sicherung, Änderungen)
  let lastTasks = null;
  const endTurn = () => {
    if (turn) addChanges(turn.snapshot, turn.changes, turn.undone);
    addStats(stats);
  };
  turnNode = null;
  for (const item of items) {
    if (item.role === 'auto') {
      addMessage('notice auto', '↻', 'Weiter am Ziel …');
    } else if (item.role === 'undo') {
      addMessage('notice', '↶', item.text);
    } else if (item.role === 'user') {
      endTurn();
      turn = item;
      stats = [];
      foldTurn(lastTime);
      addUser(item.content, item.time || 0, item.attachments);
    } else if (item.role === 'assistant') {
      lastTime = item.time || lastTime;
      stats.push(item.stats);
      if (item.reasoning) appendReasoning(item.reasoning);
      endAssistant(item.content);
      addSources(item.sources);
      for (const call of item.calls) {
        names.set(call.id, call.name);
        toolCall(call.id, call.name, call.label, call.detail);
      }
    } else if (item.role === 'tool') {
      const row = toolRows.get(item.id);
      if (row) toolResult(row, { ...item, name: names.get(item.id) });
      if (item.tasks?.length) lastTasks = item.tasks;
    }
  }
  for (const row of toolRows.values()) {
    const open = row.group ? row.item.ok === null : row.node.classList.contains('running');
    if (open) toolResult(row, { ok: false, summary: 'nicht beendet', name: row.name });
  }
  if (!running) {
    endTurn();
    foldTurn(lastTime);
  }
  // Aufgabenliste: die zuletzt gesetzte, solange noch etwas offen ist
  setTodo(lastTasks && lastTasks.some((t) => t.status !== 'erledigt') ? lastTasks : [], running);
  renderComposer();
  scrollDown(true);
}

async function loadTranscript() {
  const t = await api.chatTranscript();
  chatId = t.id;
  running = t.running;
  renderTranscript(t.items);
  renderChats();
}

function tickActivity() {
  if (!running) return;
  const current = todo.live && todo.tasks.find((t) => t.status === 'in_arbeit');
  $('activity-text').textContent = current ? `${current.titel} · ${activityLabel}` : activityLabel;
  $('activity-time').textContent = `${Math.round((Date.now() - runStarted) / 1000)} s`;
}

// ---------- Eingabe ----------

const COMMANDS = [
  { cmd: '/neu', text: 'Neuen Chat beginnen', run: () => newChat() },
  { cmd: '/goal', text: 'Ziel setzen: /goal <Ziel>; das Modell arbeitet, bis es erreicht ist', run: () => { $('input').value = '/goal '; autosize(); $('input').focus(); } },
  { cmd: '/suchen', text: 'Chats durchsuchen', run: () => { if (!state.chatbarOpen) api.setChatbar(true); $('chat-search').focus(); } },
  { cmd: '/ordner', text: 'Anderen Arbeitsordner wählen', run: () => openFolder(api.chooseFolder) },
  { cmd: '/explorer', text: 'Arbeitsordner im Explorer öffnen', run: () => api.showFolder() },
  { cmd: '/github', text: 'Mit GitHub verbinden, Repository des Ordners', run: () => showGithub() },
  { cmd: '/modell', text: 'Modell wählen', run: () => $('model-select').focus() },
  { cmd: '/denken', text: 'Denkaufwand einstellen (Standard, Tief bis Maximal)', run: () => toggleThinkPop(true) },
  { cmd: '/web', text: 'Websuche ein- oder ausschalten', run: () => { if (state.webSearchAvailable) api.setWebSearch(!state.webSearch); } },
  { cmd: '/chats', text: 'Frühere Chats für das Modell ein- oder ausschalten', run: () => api.setChatMemory(!state.chatMemory) },
  { cmd: '/neustart', text: 'Terminal neu starten', run: () => api.restartTerminal() },
  { cmd: '/abmelden', text: 'Bei ASK abmelden', run: () => api.logout() },
];
let slashIndex = 0;

function slashMatches() {
  const v = $('input').value;
  if (!v.startsWith('/') || v.includes(' ') || v.includes('\n')) return [];
  return COMMANDS.filter((c) => c.cmd.startsWith(v.toLowerCase()));
}

function renderSlash() {
  const matches = slashMatches();
  const box = $('slash');
  box.hidden = matches.length === 0;
  slashIndex = Math.min(slashIndex, Math.max(matches.length - 1, 0));
  box.replaceChildren(...matches.map((c, i) => el('div', { class: i === slashIndex ? 'active' : '', onclick: () => runSlash(c) },
    el('span', { class: 'cmd' }, c.cmd), el('span', { class: 'muted' }, c.text))));
}

function runSlash(c) {
  $('input').value = '';
  autosize();
  renderSlash();
  c.run();
}

function autosize() {
  const input = $('input');
  input.style.height = 'auto';
  input.style.height = `${input.scrollHeight}px`;
}

async function submit() {
  if (running) {
    api.chatStop();
    return;
  }
  const input = $('input');
  const text = input.value.trim();
  if (!text && !pending.length) return;
  const goalCmd = /^\/goal(?:\s+([\s\S]*))?$/i.exec(text);
  if (goalCmd) {
    input.value = '';
    autosize();
    renderSlash();
    await handleGoal((goalCmd[1] || '').trim());
    return;
  }
  const matches = slashMatches();
  if (matches.length) {
    runSlash(matches[slashIndex] || matches[0]);
    return;
  }
  if (blocker()) return;
  input.value = '';
  autosize();
  const files = pending;
  pending = [];
  renderPending();
  await sendText(text, files);
}

async function sendText(text, files = []) {
  if (text) sent.push(text);
  sentIndex = -1;
  running = true;
  runStarted = Date.now();
  activityLabel = 'Denkt nach …';
  tickActivity();
  renderComposer();
  const res = await api.chatSend(text, files.map(({ name, type, data }) => ({ name, type, data })));
  if (res && res.ok === false) {
    // Nichts gesendet: Anhänge und Text zurück ins Eingabefeld
    running = false;
    pending = files;
    renderPending();
    if (text && !$('input').value) { $('input').value = text; autosize(); }
    addMessage('error', '✗', res.reason);
    renderComposer();
  } else {
    for (const f of files) if (f.url) URL.revokeObjectURL(f.url);
  }
}

// ---------- Anhänge ----------

const MAX_ATTACH = 10;
const MAX_ATTACH_SIZE = 50 * 1024 * 1024;
const IMAGE_RE = /\.(png|jpe?g|gif|webp)$/i;
let pending = []; // { name, type, size, data, url }

async function addFiles(list) {
  for (const file of [...list]) {
    if (pending.length >= MAX_ATTACH) {
      addMessage('notice', '⎿', `Höchstens ${MAX_ATTACH} Anhänge auf einmal.`);
      break;
    }
    if (file.size > MAX_ATTACH_SIZE) {
      addMessage('notice', '⎿', `${file.name} ist grösser als 50 MB und wurde nicht angehängt.`);
      continue;
    }
    let data;
    try {
      data = new Uint8Array(await file.arrayBuffer());
    } catch {
      addMessage('notice', '⎿', `${file.name || 'Die Datei'} lässt sich nicht lesen (ein Ordner?).`);
      continue;
    }
    const image = file.type.startsWith('image/') || IMAGE_RE.test(file.name);
    pending.push({ name: file.name, type: file.type, size: file.size, data, url: image ? URL.createObjectURL(file) : '' });
  }
  renderPending();
  $('input').focus();
}

function renderPending() {
  const row = $('attach-row');
  row.hidden = pending.length === 0;
  row.replaceChildren(...pending.map((f, i) => el('span', { class: 'attach-chip', title: `${f.name} (${formatSize(f.size)})` },
    f.url ? el('img', { src: f.url, alt: '' }) : el('span', { class: `badge ${extOf(f.name)}` }, (extOf(f.name) || 'Datei').slice(0, 4).toUpperCase()),
    el('span', { class: 'name' }, f.name || 'Bild'),
    el('button', { class: 'remove', title: 'Entfernen', 'aria-label': `${f.name} entfernen`, onclick: () => {
      if (f.url) URL.revokeObjectURL(f.url);
      pending.splice(i, 1);
      renderPending();
    } }, '×'))));
}

// Anhänge in einer Eingabe im Chat: Bilder als Vorschaubild, sonst Name; Klick zeigt die Vorschau.
function attachmentList(attachments) {
  if (!attachments?.length) return null;
  const fileUrl = (p) => `askfile://datei/${p.split('/').map(encodeURIComponent).join('/')}`;
  return el('div', { class: 'attachments' }, ...attachments.map((a) => {
    const open = () => showPreview({ path: a.path, name: a.name });
    return a.image
      ? el('img', { class: 'attach-thumb', src: fileUrl(a.path), alt: a.name, title: a.path, onclick: open })
      : el('span', { class: 'attach-chip', title: a.path, onclick: open },
        el('span', { class: `badge ${extOf(a.name)}` }, (extOf(a.name) || 'Datei').slice(0, 4).toUpperCase()), el('span', { class: 'name' }, a.name));
  }));
}

function wireAttachments() {
  $('attach-btn').onclick = () => $('attach-file').click();
  $('attach-file').onchange = (e) => {
    addFiles(e.target.files);
    e.target.value = '';
  };
  // Einfügen: Dateien und Bildschirmfotos. Steht auch Text in der Zwischenablage (z. B. aus Excel
  // kopierte Zellen, die zusätzlich als Bild kommen), gilt der Text.
  $('input').addEventListener('paste', (e) => {
    const files = [...(e.clipboardData?.files || [])];
    if (!files.length || e.clipboardData.types.includes('text/plain')) return;
    e.preventDefault();
    addFiles(files);
  });
  // Hineinziehen irgendwo ins Fenster
  const hasFiles = (e) => [...(e.dataTransfer?.types || [])].includes('Files');
  document.addEventListener('dragover', (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    $('input-box').classList.add('dropping');
  });
  document.addEventListener('dragleave', (e) => {
    if (!e.relatedTarget) $('input-box').classList.remove('dropping');
  });
  document.addEventListener('drop', (e) => {
    $('input-box').classList.remove('dropping');
    if (!hasFiles(e)) return;
    e.preventDefault();
    if (activeDialog()) return;
    addFiles(e.dataTransfer.files);
  });
}

async function newChat() {
  if (running) return;
  const res = await api.chatNew();
  if (res && res.ok === false && res.reason) alertBox(res.reason);
  $('input').focus();
}

function onKey(e) {
  const matches = slashMatches();
  if (matches.length && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
    slashIndex = (slashIndex + (e.key === 'ArrowDown' ? 1 : -1) + matches.length) % matches.length;
    renderSlash();
    e.preventDefault();
    return;
  }
  if (matches.length && e.key === 'Tab') {
    $('input').value = matches[slashIndex].cmd;
    renderSlash();
    e.preventDefault();
    return;
  }
  if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
    e.preventDefault();
    submit();
    return;
  }
  if (e.key === 'ArrowUp' && !$('input').value && sent.length) {
    sentIndex = sentIndex < 0 ? sent.length - 1 : Math.max(sentIndex - 1, 0);
    $('input').value = sent[sentIndex];
    autosize();
    e.preventDefault();
  }
}

// ---------- Seitenleiste ----------

// Browser-Bereich: die echte Seite, die browser_test öffnet und steuert (wie das Browser-Fenster in Claude).
// Die Seite selbst ist eine WebContentsView im Hauptprozess; hier liegen Kopfzeile, Platz und Konsole.
// Ist die App nicht sichtbar, testet sie unsichtbar und schickt Bilder (Modus «unsichtbar»).
let liveId = null;
let browserMode = '';

function browserOpen() {
  return !$('browser-pane').hidden;
}

function reportBrowser() {
  const r = $('browser-host').getBoundingClientRect();
  api.setBrowserBounds({ x: r.x, y: r.y, width: r.width, height: r.height });
  api.setBrowserVisible(browserOpen() && browserMode !== 'unsichtbar' && !activeDialog() && $('think-pop').hidden);
}

function openBrowser() {
  $('browser-pane').hidden = false;
  document.body.classList.add('browser-open');
  requestAnimationFrame(reportBrowser);
}

function closeBrowser() {
  $('browser-pane').hidden = true;
  document.body.classList.remove('browser-open', 'browser-big');
  api.browserClose();
  reportBrowser();
}

function toggleBrowserBig() {
  document.body.classList.toggle('browser-big');
  requestAnimationFrame(reportBrowser);
}

function onLive(ev) {
  if (ev.phase === 'start') {
    liveId = ev.id;
    browserMode = '';
    $('browser-pane').classList.add('running');
    $('browser-pane').dataset.mode = '';
    $('bp-path').textContent = ev.path;
    $('bp-status').textContent = 'lädt …';
    $('bp-log').replaceChildren();
    $('bp-frame').hidden = true;
    $('bp-frame').removeAttribute('src');
    openBrowser();
    return;
  }
  if (ev.id !== liveId) return;
  if (ev.mode) {
    browserMode = ev.mode;
    $('browser-pane').dataset.mode = ev.mode;
    $('bp-frame').hidden = ev.mode !== 'unsichtbar';
    reportBrowser();
  }
  if (ev.frame) $('bp-frame').src = ev.frame;
  if (ev.step) $('bp-status').textContent = `▸ ${ev.step}`;
  if (ev.log) {
    const log = $('bp-log');
    log.append(el('li', { class: ev.level === 'error' ? 'error' : 'warning' }, ev.log));
    while (log.children.length > 6) log.firstChild.remove();
  }
  if (ev.phase === 'end') {
    $('browser-pane').classList.remove('running');
    $('bp-status').textContent = `${ev.failed ? '✗' : '✓'} ${ev.summary}${ev.userInputs ? ` · ${ev.userInputs} eigene Eingaben` : ''}`;
  }
}

function extOf(name) {
  const m = /\.([a-z0-9]+)$/i.exec(name);
  return m ? m[1].toLowerCase() : '';
}

function fileItem(entry, meta) {
  const ext = entry.dir ? 'dir' : extOf(entry.name);
  return el('li', { title: entry.path, class: selected?.path === entry.path ? 'active' : '',
    onclick: () => (entry.dir ? api.showFile(entry.path) : showPreview(entry)),
    ondblclick: () => !entry.dir && api.openFile(entry.path) },
  el('span', { class: `badge ${ext}` }, entry.dir ? 'DIR' : (ext || 'Datei').slice(0, 4).toUpperCase()),
  el('span', { class: 'name' }, entry.name),
  el('span', { class: 'meta' }, meta));
}

function renderFiles() {
  $('output-list').replaceChildren(...files.recent.map((f) => fileItem({ ...f, name: f.path.replaceAll('\\', '/') }, formatTime(f.modified))));
  $('output-empty').hidden = files.recent.length > 0;
  const entries = files.top;
  $('files-list').replaceChildren(...entries.map((f) => fileItem(f, f.dir ? '' : formatSize(f.size))));
  $('files-empty').hidden = entries.length > 0;
  markFiles($('messages')); // neue Dateien im Chat als Link markieren
  if (selected) {
    const found = [...files.recent, ...files.top].find((f) => f.path === selected.path);
    if (!found) closePreview();
    else if (found.modified !== selected.modified) showPreview(found);
  }
}

async function fillPreview(container, entry) {
  const p = await api.preview(entry.path);
  container.replaceChildren();
  if (p.kind === 'image') container.append(el('img', { src: `${p.url}?t=${entry.modified}`, alt: entry.name }));
  else if (p.kind === 'pdf') container.append(el('iframe', { src: `${p.url}?t=${entry.modified}`, title: entry.name }));
  else if (p.kind === 'text') container.append(el('pre', {}, p.text + (p.truncated ? '\n\n… (gekürzt)' : '')));
  else if (p.kind === 'missing') container.append(el('div', { class: 'office' }, 'Die Datei ist nicht mehr vorhanden.'));
  else {
    const status = el('p', { class: 'muted small' });
    const box = el('div', { class: 'office' },
      el('p', {}, p.kind === 'office' ? 'Office-Dateien öffnest du in Word, Excel oder PowerPoint.' : 'Für diese Datei gibt es keine Vorschau.'),
      el('button', { class: 'btn primary', onclick: () => api.openFile(entry.path) }, 'Öffnen'),
      el('button', { class: 'btn', onclick: () => api.showFile(entry.path) }, 'Im Explorer zeigen'));
    if (p.kind === 'office' && /\.(docx|xlsx|pptx)$/i.test(entry.path)) {
      box.append(el('button', { class: 'btn', onclick: async () => {
        status.textContent = 'Wird mit Office in PDF umgewandelt …';
        const res = await api.toPdf(entry.path);
        if (!res.ok) { status.textContent = res.fehler; return; }
        const pdf = await api.preview(res.path);
        container.replaceChildren(el('iframe', { src: `${pdf.url}?t=${Date.now()}`, title: entry.name }));
      } }, 'Als PDF anzeigen'), status);
    }
    container.append(box);
  }
}

async function showPreview(entry) {
  selected = { path: entry.path, modified: entry.modified };
  if (!state.sidebarOpen) await api.setSidebar(true);
  $('preview-panel').hidden = false;
  $('preview-title').textContent = baseName(entry.path);
  $('preview-title').title = entry.path;
  document.querySelectorAll('.files li').forEach((li) => li.classList.toggle('active', li.title === entry.path));
  await fillPreview($('preview-body'), entry);
  if (openDialog === 'preview') await fillPreview($('big-body'), entry);
}

function closePreview() {
  selected = null;
  $('preview-panel').hidden = true;
  $('preview-body').replaceChildren();
  if (openDialog === 'preview') {
    openDialog = null;
    renderDialogs();
  }
}

// ---------- Ablauf ----------

function render() {
  if (!state) return;
  renderStatus();
  renderDialogs();
}

function reportBounds() {
  const r = $('ask-host').getBoundingClientRect();
  api.setAskBounds({ x: r.x, y: r.y, width: r.width, height: r.height });
}

function wire() {
  wireAttachments();
  $('todo-head').onclick = () => toggleTodo();
  $('bp-close').onclick = () => closeBrowser();
  $('bp-big').onclick = () => toggleBrowserBig();
  $('bp-reload').onclick = () => api.browserReload();
  $('folder-btn').onclick = () => api.folderMenu();
  $('sidebar-btn').onclick = () => api.setSidebar(!state.sidebarOpen);
  $('new-btn').onclick = () => newChat();
  $('chatbar-btn').onclick = () => api.setChatbar(!state.chatbarOpen);
  $('chat-search').addEventListener('input', () => renderChats());
  $('chat-search').addEventListener('keydown', (e) => { if (e.key === 'Escape') { e.target.value = ''; renderChats(); } });
  $('model-select').onchange = (e) => api.setModel(e.target.value);
  $('web-btn').onclick = () => api.setWebSearch(!state.webSearch);
  $('memory-btn').onclick = () => api.setChatMemory(!state.chatMemory);
  $('think-btn').onclick = (e) => { e.stopPropagation(); toggleThinkPop(); };
  $('think-pop').addEventListener('click', (e) => e.stopPropagation());
  $('think-range').addEventListener('input', (e) => api.setThinking(THINK_LEVELS[Number(e.target.value)]));
  $('think-standard').onclick = () => api.setThinking('standard');
  document.addEventListener('click', () => toggleThinkPop(false));
  $('terminal-pill').onclick = () => { openDialog = 'terminal'; $('terminal-logs').hidden = false; render(); };
  $('ask-pill').onclick = () => { openDialog = 'ask'; render(); };
  $('send-btn').onclick = () => submit();
  $('input').addEventListener('keydown', onKey);
  $('input').addEventListener('input', () => { autosize(); slashIndex = 0; renderSlash(); });
  document.querySelectorAll('.example').forEach((b) => {
    b.onclick = () => { $('input').value = b.textContent; autosize(); $('input').focus(); };
  });
  $('messages').addEventListener('click', async (e) => {
    const copy = e.target.closest('button.copy');
    if (copy) {
      const text = copy.dataset.text ?? copy.closest('.codeblock')?.querySelector('pre code')?.textContent ?? '';
      await api.copy(text);
      const label = copy.textContent;
      copy.textContent = '✓ Kopiert';
      setTimeout(() => { copy.textContent = label; }, 1500);
      return;
    }
    const link = e.target.closest('.md a');
    if (link) {
      e.preventDefault();
      api.openLink(link.getAttribute('href'));
      return;
    }
    const code = e.target.closest('.md code');
    if (code && !code.closest('pre')) {
      const found = fileFor(code.textContent);
      if (found) showPreview(found);
    }
  });
  $('warning-check').onchange = (e) => { $('warning-ok').disabled = !e.target.checked; };
  $('warning-ok').onclick = () => api.acceptWarning();
  $('url-ok').onclick = async () => {
    const res = await api.setAskUrl($('url-input').value);
    $('url-error').hidden = res.ok;
    $('url-error').textContent = res.reason || '';
  };
  $('url-input').onkeydown = (e) => { if (e.key === 'Enter') $('url-ok').click(); };
  $('start-new').onclick = () => openFolder(api.newFolder);
  $('start-choose').onclick = () => openFolder(api.chooseFolder);
  $('ask-retry').onclick = async () => { await api.connectAsk(); await api.models(); render(); };
  $('ask-logout').onclick = async () => { openDialog = null; await api.logout(); render(); };
  $('ask-close').onclick = () => { openDialog = null; render(); };
  $('login-reload').onclick = () => api.reloadAsk();
  $('terminal-restart').onclick = () => { api.restartTerminal(); openDialog = null; render(); };
  $('terminal-close').onclick = () => { openDialog = null; render(); };
  $('goal-clear').onclick = () => api.chatGoal('');
  $('preview-close').onclick = () => closePreview();
  $('preview-big').onclick = async () => {
    if (!selected) return;
    openDialog = 'preview';
    $('big-title').textContent = selected.path;
    render();
    await fillPreview($('big-body'), selected);
  };
  $('big-close').onclick = () => { openDialog = null; $('big-body').replaceChildren(); render(); };
  document.addEventListener('keydown', (e) => {
    if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 't') {
      e.preventDefault();
      toggleTodo();
    }
    if (e.ctrlKey && !e.shiftKey && !e.altKey && e.key.toLowerCase() === 'o') {
      e.preventDefault();
      setVerbose(!verbose);
    }
  });
  $('verbose-btn').onclick = () => setVerbose(!verbose);
  document.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    if (!$('think-pop').hidden) { toggleThinkPop(false); return; }
    if (openDialog) { openDialog = null; render(); return; }
    if (!$('slash').hidden) { $('input').value = ''; renderSlash(); return; }
    if (running) api.chatStop();
  });

  api.onState((s) => {
    const folderChanged = state && s.folder !== state.folder;
    state = s;
    goal = s.chat.goal;
    if (folderChanged) loadChats();
    if (s.chat.running !== running) {
      running = s.chat.running;
      if (running && !runStarted) runStarted = Date.now();
      if (!running) runStarted = 0;
    }
    if (s.chat.id !== chatId) loadTranscript();
    render();
  });
  api.onChatEvent(onEvent);
  api.onChatChanged((c) => {
    if (c.id !== chatId) loadTranscript();
    running = c.running;
    goal = c.goal;
    loadChats();
    if (!running) runStarted = 0;
    renderComposer();
    if (!running) $('input').focus();
  });
  api.onFiles((f) => { files = f; renderFiles(); });
  api.onMenuChooseFolder(() => openFolder(api.chooseFolder));
  api.onMenuNewFolder(() => openFolder(api.newFolder));
  api.onMenuGithub(() => showGithub());
  $('github-token-link').onclick = () => api.openLink('https://github.com/settings/tokens/new?scopes=repo&description=ASK%20Desktop');
  $('github-ok').onclick = async () => {
    const res = await api.githubConnect($('github-token').value);
    if (!res.ok) {
      $('github-error').textContent = res.message;
      $('github-error').hidden = false;
      return;
    }
    showGithub();
  };
  $('github-token').onkeydown = (e) => { if (e.key === 'Enter') $('github-ok').click(); };
  $('github-disconnect').onclick = async () => { await api.githubDisconnect(); showGithub(); };
  $('github-close').onclick = () => { openDialog = null; render(); };
  api.onFolderError((reason) => alertBox(reason));
  api.onAskLoaded(() => {
    askFailed = false;
    $('ask-placeholder').textContent = 'Verbinde mit ASK …';
    render();
  });
  api.onAskLoadFailed((info) => {
    askFailed = true;
    $('ask-placeholder').replaceChildren(el('div', {},
      el('p', {}, `ASK ist nicht erreichbar (${info.description}).`),
      el('p', {}, 'Internetverbindung prüfen und dann erneut laden.'),
      el('button', { class: 'btn primary', onclick: () => api.reloadAsk() }, 'Erneut laden')));
    render();
  });

  new ResizeObserver(reportBounds).observe($('ask-host'));
  window.addEventListener('resize', reportBounds);
  new ResizeObserver(reportBrowser).observe($('browser-host'));
  window.addEventListener('resize', reportBrowser);
  setInterval(tickActivity, 500);
}

(async function init() {
  wire();
  state = await api.getState();
  render();
  await loadTranscript();
  await loadChats();
  files = await api.files();
  renderFiles();
  $('input').focus();
})();
