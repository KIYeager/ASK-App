// Oberfläche von ASK Desktop: Statusleiste, Seitenleiste, Dialoge.
const $ = (id) => document.getElementById(id);
const api = window.ask;

let state = null;
let files = { top: [], output: [] };
let selected = null; // { path, modified }
let openDialog = null; // manuell geöffneter Dialog: 'connect' | 'terminal' | 'preview'
let startError = '';
let askFailed = false;

function el(tag, attrs = {}, ...children) {
  const node = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs)) {
    if (k === 'class') node.className = v;
    else if (k.startsWith('on')) node.addEventListener(k.slice(2), v);
    else if (v !== undefined && v !== null) node.setAttribute(k, v);
  }
  for (const c of children) node.append(c instanceof Node ? c : document.createTextNode(String(c)));
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

// ---------- Statusleiste ----------

const TERMINAL_TEXT = {
  gestoppt: ['Terminal gestoppt', ''],
  startet: ['Terminal startet …', 'warn'],
  läuft: ['Terminal läuft', 'ok'],
  fehler: ['Terminal: Fehler', 'bad'],
};

const ASK_TEXT = {
  unbekannt: ['ASK: wird geprüft', ''],
  'nicht-angemeldet': ['ASK: nicht angemeldet', 'warn'],
  'nicht-freigegeben': ['ASK: Terminal nicht freigegeben', 'bad'],
  'nicht-eingerichtet': ['ASK: nicht verbunden', 'warn'],
  eingerichtet: ['ASK: eingerichtet', 'warn'],
  verbunden: ['Mit ASK verbunden', 'ok'],
  'zu-alt': ['ASK-Version zu alt', 'bad'],
  fehler: ['ASK: Fehler', 'bad'],
};

function renderStatus() {
  const s = state;
  $('folder-name').textContent = s.folder ? baseName(s.folder) : 'Kein Ordner';
  $('folder-btn').title = s.folder || 'Arbeitsordner wählen';

  const t = s.terminal;
  const [tText, tClass] = TERMINAL_TEXT[t.state] || [t.state, ''];
  $('terminal-text').textContent = tText;
  $('terminal-pill').className = `pill ${tClass}`;
  $('terminal-pill').title = t.state === 'läuft'
    ? `Läuft auf ${t.url}${t.lowIntegrity ? ', Schreibschutz ausserhalb des Ordners aktiv' : ''}`
    : (t.error || 'Terminal');

  const a = s.terminal.state === 'läuft' ? s.ask : { state: 'unbekannt' };
  const [aText, aClass] = ASK_TEXT[a.state] || [a.state, ''];
  $('ask-text').textContent = s.terminal.state === 'läuft' ? aText : 'ASK: nicht verbunden';
  $('ask-pill').className = `pill ${s.terminal.state === 'läuft' ? aClass : ''}`;
  $('ask-pill').title = a.state === 'eingerichtet' ? 'Eingerichtet, aber ASK hat das Terminal noch nicht benutzt.' : '';

  const ready = s.warningAccepted && !!s.askUrl;
  $('restart-btn').disabled = !s.folder || !ready;
  $('choose-btn').disabled = !ready;
  $('folder-btn').disabled = !ready;
  document.body.classList.toggle('no-sidebar', !s.sidebarOpen);
}

// ---------- Dialoge ----------

function activeDialog() {
  if (!state) return null;
  if (!state.warningAccepted) return 'warning';
  if (!state.askUrl) return 'url';
  if (!state.folder) return 'start';
  return openDialog;
}

function renderDialogs() {
  const current = activeDialog();
  for (const name of ['warning', 'url', 'start', 'connect', 'terminal', 'preview']) {
    $(`dlg-${name}`).hidden = name !== current;
  }
  $('overlay').hidden = !current;
  api.setAskVisible(!current && !!state?.askUrl && !askFailed);
  if (current === 'start') renderStart();
  if (current === 'connect') renderConnect();
  if (current === 'terminal') renderTerminalDialog();
  if (current === 'url') $('url-input').value = $('url-input').value || state.askUrl || '';
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

function step(text, ...code) {
  return el('li', {}, text, ...code);
}

function renderConnect() {
  const a = state.ask;
  const body = $('connect-body');
  body.replaceChildren();
  const manual = el('ol', { class: 'steps' },
    step('In ASK oben auf das Profil klicken, «Einstellungen» öffnen, Reiter «Integrationen».'),
    step('Bei «Open Terminal» auf «Verbindung hinzufügen» (+) klicken.'),
    step('URL und Key von unten kopieren und einfügen, speichern und die Verbindung aktivieren.'),
    step('Im Chat im Eingabefeld auf das Terminal-Symbol klicken und «ASK Desktop» wählen.'));
  const p = (text) => el('p', {}, text);
  switch (a.state) {
    case 'nicht-freigegeben':
      body.append(
        p('ASK erlaubt deinem Konto keine persönliche Terminal-Verbindung. Das kann nur die ASK-Administration ändern.'),
        p(`Fehlt: ${[!a.directIntegrations && '«Direkte Integrationen» (Admin-Einstellung)',
          !a.permission && 'Berechtigung «Direkte Tool-Server» für dein Konto',
          !a.toolServers && 'Tool-Server sind in ASK ganz ausgeschaltet'].filter(Boolean).join(', ')}.`),
        p('ASK funktioniert weiterhin, aber ohne Zugriff auf den Arbeitsordner.'));
      break;
    case 'nicht-angemeldet':
      body.append(p('Melde dich zuerst in ASK an. Danach richtet die App die Verbindung selbst ein.'));
      break;
    case 'eingerichtet':
    case 'verbunden':
      body.append(p(a.state === 'verbunden'
        ? 'ASK ist mit dem Terminal verbunden.'
        : 'Die Verbindung ist in ASK eingerichtet. Sobald ASK das Terminal benutzt, zeigt die Statusleiste «Mit ASK verbunden».'),
      p('Im Chat muss das Terminal «ASK Desktop» ausgewählt sein (Terminal-Symbol im Eingabefeld). Die App wählt es beim Einrichten vor.'));
      break;
    case 'zu-alt':
      body.append(p(`Diese ASK-Version (${a.version}) unterstützt persönliche Terminals noch nicht vollständig. Nötig ist mindestens Open WebUI 0.8.11.`));
      break;
    default:
      body.append(p(a.message ? `Automatisch einrichten ging nicht: ${a.message}` : 'Die Verbindung ist noch nicht eingerichtet.'),
        p('Du kannst sie auch von Hand einrichten:'), manual);
  }
  if (['nicht-freigegeben', 'eingerichtet', 'verbunden', 'zu-alt', 'nicht-angemeldet'].includes(a.state)) {
    body.append(el('details', {}, el('summary', {}, 'Von Hand einrichten'), manual));
  }
  $('connect-url').textContent = state.terminal.url || '(Terminal läuft nicht)';
  $('connect-key').textContent = state.placeholderKey;
}

async function renderTerminalDialog() {
  const t = state.terminal;
  if (t.state !== 'fehler' && $('terminal-logs').hidden) return;
  $('terminal-logs').hidden = false;
  const detail = t.state === 'läuft'
    ? `Das Terminal läuft auf ${t.url} im Ordner ${t.folder}. ${t.lowIntegrity ? 'Ausserhalb des Ordners kann es nichts schreiben oder löschen.' : ''}`
    : (t.error || `Status: ${t.state}`);
  $('terminal-detail').textContent = detail;
  $('terminal-logs').textContent = await api.terminalLogs() || '(keine Meldungen)';
}

// ---------- Seitenleiste ----------

function renderTasks(tasks) {
  const list = $('tasks-list');
  list.replaceChildren();
  const items = tasks?.aufgaben || [];
  for (const t of items) {
    list.append(el('li', { class: t.status }, el('span', { class: 'mark', 'aria-hidden': 'true' }),
      el('span', {}, t.titel)));
  }
  const done = items.filter((t) => t.status === 'erledigt').length;
  $('tasks-count').textContent = items.length ? `${done} von ${items.length}` : '';
  $('tasks-goal').textContent = tasks?.auftrag || tasks?.fehler || '';
  $('tasks-goal').hidden = !(tasks?.auftrag || tasks?.fehler);
  $('tasks-empty').hidden = items.length > 0;
}

function extOf(name) {
  const m = /\.([a-z0-9]+)$/i.exec(name);
  return m ? m[1].toLowerCase() : '';
}

function fileItem(entry, meta) {
  const ext = entry.dir ? 'dir' : extOf(entry.name);
  const li = el('li', { title: entry.path, class: selected?.path === entry.path ? 'active' : '',
    onclick: () => (entry.dir ? api.showFile(entry.path) : showPreview(entry)),
    ondblclick: () => !entry.dir && api.openFile(entry.path) },
  el('span', { class: `badge ${ext}` }, entry.dir ? 'Ordner' : (ext || 'Datei').slice(0, 4).toUpperCase()),
  el('span', { class: 'name' }, entry.name),
  el('span', { class: 'meta' }, meta));
  return li;
}

function renderFiles() {
  const out = $('output-list');
  out.replaceChildren(...files.output.map((f) => fileItem({ ...f, name: f.path.replace(/^ausgabe[\\/]/, '') },
    formatTime(f.modified))));
  $('output-empty').hidden = files.output.length > 0;
  const top = $('files-list');
  const entries = files.top.filter((f) => f.name !== 'ausgabe');
  top.replaceChildren(...entries.map((f) => fileItem(f, f.dir ? '' : formatSize(f.size))));
  $('files-empty').hidden = entries.length > 0;
  if (selected) {
    const current = [...files.output, ...files.top].find((f) => f.path === selected.path);
    if (!current) closePreview();
    else if (current.modified !== selected.modified) showPreview(current);
  }
}

async function fillPreview(container, entry) {
  const p = await api.preview(entry.path);
  container.replaceChildren();
  if (p.kind === 'image') container.append(el('img', { src: `${p.url}?t=${entry.modified}`, alt: entry.name }));
  else if (p.kind === 'pdf') container.append(el('iframe', { src: `${p.url}?t=${entry.modified}`, title: entry.name }));
  else if (p.kind === 'text') {
    container.append(el('pre', {}, p.text + (p.truncated ? '\n\n… (gekürzt)' : '')));
  } else if (p.kind === 'missing') {
    container.append(el('div', { class: 'office' }, 'Die Datei ist nicht mehr vorhanden.'));
  } else {
    const status = el('p', { class: 'muted small' });
    const box = el('div', { class: 'office' },
      el('p', {}, p.kind === 'office' ? 'Office-Dateien öffnest du in Word, Excel oder PowerPoint.'
        : 'Für diese Datei gibt es keine Vorschau.'),
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
  const r = $('ask-area').getBoundingClientRect();
  api.setAskBounds({ x: r.x, y: r.y, width: r.width, height: r.height });
}

function wire() {
  $('folder-btn').onclick = () => api.folderMenu();
  $('choose-btn').onclick = () => openFolder(api.chooseFolder);
  $('restart-btn').onclick = () => api.restartTerminal();
  $('sidebar-btn').onclick = () => api.setSidebar(!state.sidebarOpen);
  $('terminal-pill').onclick = () => { openDialog = 'terminal'; $('terminal-logs').hidden = false; render(); };
  $('ask-pill').onclick = () => { openDialog = 'connect'; render(); };
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
  $('connect-retry').onclick = async () => { await api.connectAsk(); };
  $('connect-close').onclick = () => { openDialog = null; render(); };
  $('terminal-restart').onclick = () => { api.restartTerminal(); openDialog = null; render(); };
  $('terminal-close').onclick = () => { openDialog = null; render(); };
  $('preview-close').onclick = () => closePreview();
  $('preview-big').onclick = async () => {
    if (!selected) return;
    openDialog = 'preview';
    $('big-title').textContent = selected.path;
    render();
    await fillPreview($('big-body'), selected);
  };
  $('big-close').onclick = () => { openDialog = null; $('big-body').replaceChildren(); render(); };
  document.querySelectorAll('[data-copy]').forEach((b) => {
    b.onclick = () => api.copy($(b.dataset.copy).textContent);
  });
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && openDialog) { openDialog = null; render(); }
  });

  api.onState((s) => { state = s; render(); });
  api.onFiles((f) => { files = f; renderFiles(); });
  api.onTasks((t) => renderTasks(t));
  api.onMenuChooseFolder(() => openFolder(api.chooseFolder));
  api.onMenuNewFolder(() => openFolder(api.newFolder));
  api.onFolderError((reason) => alertBox(reason));
  api.onAskLoaded(() => {
    askFailed = false;
    $('ask-placeholder').textContent = 'ASK wird geladen …';
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

  new ResizeObserver(reportBounds).observe($('ask-area'));
  window.addEventListener('resize', reportBounds);
}

(async function init() {
  wire();
  state = await api.getState();
  render();
  reportBounds();
  files = await api.files();
  renderFiles();
  renderTasks(await api.tasks());
})();
