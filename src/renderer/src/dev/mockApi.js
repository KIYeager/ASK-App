// Attrappe der Schnittstelle zum Hauptprozess für «npm run dev:ui» im Browser: Beispielprojekte,
// ein gespeicherter Chat und ein vorgespielter Auftrag mit Frage und Rückfrage. Nicht in Electron.
const listeners = {};
const on = (ch) => (cb) => {
  (listeners[ch] ||= new Set()).add(cb);
  return () => listeners[ch].delete(cb);
};
const emit = (ch, payload) => (listeners[ch] || []).forEach((cb) => cb(payload));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const params = new URLSearchParams(location.search);

const FOLDER = 'C:\\Users\\lernende\\ASK-Projekte\\Haushaltsbudget';
const stats = (t) => ({ tokens: t, genMs: t * 7, totalMs: t * 9, firstMs: 600, promptTokens: 4200 });

let state = {
  askUrl: 'https://ask.ict-tfbern.ch', folder: FOLDER, recent: [FOLDER], warningAccepted: true, sidebarOpen: true, chatbarOpen: true,
  terminal: { state: 'läuft', folder: FOLDER, lowIntegrity: true }, ask: { state: 'angemeldet', user: 'Lea Muster' },
  models: [{ id: 'qwen3-coder', name: 'Qwen3 Coder' }, { id: 'gpt-oss', name: 'GPT-OSS 120B' }], model: 'qwen3-coder',
  webSearch: false, chatMemory: false, memory: true, thinking: 'mittel', webSearchAvailable: true, approvals: 'heikel', mode: 'auto', theme: 'system',
  version: '0.2.3', update: { state: params.get('update') || 'verfügbar', current: '0.2.3', version: '0.2.4', percent: 42, countdown: 7, beta: false, enabled: true, portable: false, postponed: false },
  prefs: { verbose: false, todoCollapsed: true, tourDone: !params.get('tour') }, // ?tour=1 zeigt die Einführung
  chat: { id: 'c1', title: 'Ausgaben auswerten', running: false, goal: null, pinned: false, context: { tokens: 18400, estimated: false, limit: 131072 } },
};
if (params.get('offline')) state = { ...state, ask: { state: 'unbekannt', loadError: { code: -106, description: 'ERR_INTERNET_DISCONNECTED' } }, models: [], model: '' }; // ?offline=1
if (params.get('dns')) state = { ...state, secureDns: true, askUrlDefault: 'https://ask.ict-tfbern.ch', askUrl: 'https://ask.ict-tfbern.ch', ask: { state: 'unbekannt', loadError: { code: -105, description: 'ERR_NAME_NOT_RESOLVED', url: 'https://ask.ict-tfbern.ch/', proxy: 'DIRECT' } }, models: [], model: '' }; // ?dns=1
if (params.get('askfehler')) state = { ...state, ask: { state: 'fehler', user: 'Lea Muster', message: 'ASK bietet diesem Konto kein Modell an. Wer ASK betreut, muss deinem Konto zuerst ein Modell freigeben.' }, models: [], model: '' }; // ?askfehler=1
if (params.get('leer')) state.chat = { id: 'neu1', title: '', running: false, goal: null };
if (params.get('erst')) state = { ...state, folder: '', chat: { id: 'neu1', title: '', running: false } };

const projects = [
  { folder: FOLDER, name: 'Haushaltsbudget', color: '#0082B4', chats: [{ id: 'c1', title: 'Ausgaben auswerten', updated: Date.now() - 5 * 60000, count: 2 }, { id: 'c2', title: 'Budgetplan 2027', updated: Date.now() - 26 * 3600000, count: 4 }] },
  { folder: 'C:\\Users\\lernende\\ASK-Projekte\\Modul 293', name: 'Modul 293 · Webauftritt', color: '#F28855', chats: [{ id: 'c3', title: 'Startseite in HTML', updated: Date.now() - 3 * 86400000, count: 6 }, { id: 'c4', title: 'Bilder verkleinern', updated: Date.now() - 4 * 86400000, count: 1 }] },
  { folder: 'C:\\Users\\lernende\\ASK-Projekte\\Lernjournal', name: 'Lernjournal', color: '#4EB68E', chats: [{ id: 'c5', title: 'Wochenbericht KW 47', updated: Date.now() - 6 * 86400000, count: 3 }] },
];

const now = Date.now();
const tree = {
  '': [
    { path: 'daten', name: 'daten', dir: true, size: 0, modified: now - 86400000 },
    { path: 'bilder', name: 'bilder', dir: true, size: 0, modified: now - 3600000 },
    { path: 'ausgaben.csv', name: 'ausgaben.csv', dir: false, size: 2140, modified: now - 86400000 },
    { path: 'auswertung.py', name: 'auswertung.py', dir: false, size: 1530, modified: now - 300000 },
    { path: 'auswertung.xlsx', name: 'auswertung.xlsx', dir: false, size: 18800, modified: now - 280000 },
    { path: 'budget-notizen.docx', name: 'budget-notizen.docx', dir: false, size: 14200, modified: now - 270000 },
  ],
  daten: [{ path: 'daten\\kategorien.txt', name: 'kategorien.txt', dir: false, size: 120, modified: now - 86400000 }],
  bilder: [{ path: 'bilder\\diagramm.png', name: 'diagramm.png', dir: false, size: 84000, modified: now - 280000 },
    { path: 'bilder\\titelbild.png', name: 'titelbild.png', dir: false, size: 1200000, modified: now - 86400000 }],
};
const files = { top: tree[''], recent: tree[''].filter((f) => !f.dir) };

const saved = [
  { role: 'user', content: 'Werte ausgaben.csv aus: Summen pro Kategorie und ein Diagramm, alles in Excel.', time: now - 400000, snapshot: 's1',
    changes: { added: ['auswertung.py', 'auswertung.xlsx'], changed: ['budget-notizen.docx'], removed: ['entwurf_alt.xlsx'], counts: [1, 2, 1] } },
  { role: 'assistant', content: '', reasoning: 'Zuerst die CSV lesen, dann pandas für die Summen und openpyxl für das Diagramm.', stats: stats(180),
    calls: [{ id: 't1', name: 'read_file', label: 'Lesen', detail: 'ausgaben.csv' }, { id: 't1b', name: 'list_files', label: 'Ordner', detail: '.' }] },
  { role: 'tool', id: 't1', ok: true, summary: '61 Zeilen', output: 'Datum;Kategorie;Beschreibung;Betrag CHF\n02.08.2026;Bücher;Fachbuch Netzwerktechnik;32.75' },
  { role: 'tool', id: 't1b', ok: true, summary: '5 Einträge', output: 'ausgaben.csv\nbudget-notizen.docx' },
  { role: 'assistant', content: '', stats: stats(420), calls: [{ id: 't2', name: 'frage_stellen', label: 'Frage', detail: 'Soll das Diagramm ein Balken- oder ein Kuchendiagramm sein?', optionen: ['Balken', 'Kuchen'] }] },
  { role: 'tool', id: 't2', ok: true, summary: 'Balken', output: 'Frage: …\nAntwort: Balken' },
  { role: 'assistant', content: '', stats: stats(640), calls: [{ id: 't3', name: 'write_file', label: 'Schreiben', detail: 'auswertung.py' }, { id: 't4', name: 'run_command', label: 'Befehl', detail: 'python auswertung.py' }] },
  { role: 'tool', id: 't3', ok: true, summary: '38 Zeilen', output: 'import pandas as pd\n…' },
  { role: 'tool', id: 't4', ok: true, summary: '', output: 'auswertung.xlsx erstellt (6 Kategorien)' },
  { role: 'assistant', content: '', stats: stats(120), calls: [{ id: 't5', name: 'view_file', label: 'Ansehen', detail: 'auswertung.xlsx' }] },
  { role: 'tool', id: 't5', ok: true, summary: 'Tabelle und Diagramm geprüft', output: '' },
  { role: 'assistant', content: 'Fertig. In `auswertung.xlsx` stehen die Summen pro Kategorie und ein Balkendiagramm.\n\n| Kategorie | CHF |\n|---|---:|\n| ÖV-Abo | 500.00 |\n| Freizeit | 460.75 |\n| Bücher | 411.15 |\n| Material | 266.95 |\n| Handy | 150.00 |\n| Verpflegung | 145.70 |\n| **Total** | **1’934.55** |\n\nDen alten Entwurf `entwurf_alt.xlsx` habe ich nach deiner Bestätigung gelöscht.', stats: stats(300), calls: [] },
];

// Nutzung (E49): 30 Tage mit Beispielwerten, drei Projekte, Angaben von ASK
function mockUsage() {
  const days = Array.from({ length: 30 }, (_, i) => {
    const d = new Date(now - (29 - i) * 86400000);
    const weekend = d.getDay() === 0 || d.getDay() === 6;
    const tasks = weekend ? (i % 3 === 0 ? 1 : 0) : [3, 5, 2, 0, 7, 4, 6, 1, 8, 3][i % 10];
    return { date: `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`, tasks, tokens: tasks * 2350 + (tasks ? 410 : 0) };
  });
  const p = (i, chats, tasks, steps, tokens, est, added, changed, removed, min) => ({ folder: projects[i].folder, name: projects[i].name, color: projects[i].color, missing: false,
    chats, tasks, steps, tokens, estimatedTokens: est, modelMs: min * 60000, files: { added, changed, removed }, last: now - i * 86400000 });
  const list = [p(0, 14, 61, 412, 148200, 0, 23, 17, 4, 96), p(1, 9, 38, 301, 99650, 12400, 31, 22, 6, 71), p(2, 4, 11, 46, 18800, 0, 3, 5, 0, 14)];
  const total = list.reduce((t, x) => ({ chats: t.chats + x.chats, tasks: t.tasks + x.tasks, steps: t.steps + x.steps, tokens: t.tokens + x.tokens, estimatedTokens: t.estimatedTokens + x.estimatedTokens,
    modelMs: t.modelMs + x.modelMs, files: { added: t.files.added + x.files.added, changed: t.files.changed + x.files.changed, removed: t.files.removed + x.files.removed }, last: Math.max(t.last, x.last) }),
  { chats: 0, tasks: 0, steps: 0, tokens: 0, estimatedTokens: 0, modelMs: 0, files: { added: 0, changed: 0, removed: 0 }, last: 0 });
  return { projects: list, total, days };
}

let transcriptItems = params.get('leer') || params.get('erst') ? [] : saved;

// Entwicklungsserver (servers.js): ein laufender Vite-Server und einer, der mit Fehler beendet ist
const serverLines = {
  s1: ['> budget-web@0.1.0 dev', '> vite', '', '  VITE v6.3.1  ready in 412 ms', '', '  ➜  Local:   http://localhost:5173/', '  ➜  Network: use --host to expose'],
  s2: ['node:internal/modules/cjs/loader:1228', "Error: Cannot find module 'express'", 'Require stack:', '- C:\\Users\\lernende\\ASK-Projekte\\Haushaltsbudget\\api.js'],
};
let servers = params.get('server') === '0' ? [] : [
  { id: 's1', name: 'Webseite', command: 'npm run dev', started: now - 120000, state: 'läuft', exitCode: null, url: 'http://localhost:5173/', port: 5173, ready: true, error: '', by: 'modell', last: serverLines.s1.at(-1) },
  { id: 's2', name: 'node api.js', command: 'node api.js', started: now - 60000, state: 'fehler', exitCode: 1, url: '', port: 0, ready: false, error: 'Beendet mit Exitcode 1', by: 'person', last: serverLines.s2.at(-1) },
];
const pushServers = () => emit('servers', servers.map((x) => ({ ...x })));
async function mockServerStart(command) {
  const id = `s${Date.now()}`;
  serverLines[id] = [`> ${command}`];
  servers = [...servers, { id, name: command.slice(0, 40), command, started: Date.now(), state: 'startet', exitCode: null, url: '', port: 0, ready: false, error: '', by: 'person', last: '' }];
  pushServers();
  setTimeout(() => {
    serverLines[id].push('Serving HTTP on :: port 8000 (http://[::]:8000/) ...');
    servers = servers.map((x) => (x.id === id ? { ...x, state: 'läuft', url: 'http://localhost:8000/', port: 8000, ready: true, last: serverLines[id].at(-1) } : x));
    pushServers();
  }, 1200);
  return { ok: true, server: servers.at(-1) };
}
state.servers = servers;

const push = () => emit('state', { ...state, servers });

async function demoRun(text) {
  const ev = (e) => emit('chat:event', e);
  state = { ...state, chat: { ...state.chat, running: true, title: state.chat.title || text.slice(0, 60) } };
  push();
  ev({ type: 'user', text, attachments: [] });
  await wait(400);
  if (state.mode === 'plan') { await demoPlan(ev); return; }
  ev({ type: 'assistant-start' });
  for (const w of 'Ich räume den Ordner auf. '.split(' ')) { ev({ type: 'reasoning', text: `${w} ` }); await wait(40); }
  ev({ type: 'assistant-end', content: '', stats: stats(90), calls: [] });
  ev({ type: 'tool-start', id: 'd1', name: 'list_files', label: 'Ordner', detail: '.' });
  await wait(300);
  ev({ type: 'tool-end', id: 'd1', ok: true, summary: '6 Einträge', output: '' });
  ev({ type: 'tool-start', id: 'q1', name: 'frage_stellen', label: 'Frage', detail: 'Welche Dateien darf ich löschen?' });
  ev({ type: 'question', id: 'q1', frage: 'Welche Dateien darf ich löschen?', optionen: ['Nur Zwischendateien', 'Auch alte Entwürfe', 'Nichts löschen'] });
  const answer = await new Promise((r) => { pendingAnswer = r; });
  ev({ type: 'question-end', id: 'q1', answer });
  ev({ type: 'tool-end', id: 'q1', ok: true, summary: answer, output: `Antwort: ${answer}` });
  ev({ type: 'tool-start', id: 'd2', name: 'run_command', label: 'Befehl', detail: 'del /q entwurf_alt.xlsx' });
  ev({ type: 'approval', id: 'p1', kind: 'loeschen', title: 'Darf ASK Dateien löschen?', detail: 'del /q entwurf_alt.xlsx', reason: 'Der Befehl löscht Dateien oder Ordner. Sie kommen in die Sicherung und lassen sich mit «Rückgängig» zurückholen.' });
  const ok = await new Promise((r) => { pendingApproval = r; });
  ev({ type: 'approval-end', id: 'p1', answer: ok });
  ev({ type: 'tool-end', id: 'd2', ok: ok !== 'no', summary: ok === 'no' ? 'abgelehnt' : '', output: ok === 'no' ? 'Die Person hat diesen Schritt abgelehnt.' : '' });
  ev({ type: 'assistant-start' });
  for (const w of 'Erledigt. Ich habe den Ordner aufgeräumt; `auswertung.xlsx` bleibt unverändert.'.split(' ')) { ev({ type: 'text', text: `${w} ` }); await wait(50); }
  ev({ type: 'assistant-end', content: 'Erledigt. Ich habe den Ordner aufgeräumt; `auswertung.xlsx` bleibt unverändert.', stats: stats(70) });
  ev({ type: 'changes', snapshot: 's2', changes: { added: [], changed: [], removed: ok === 'no' ? [] : ['entwurf_alt.xlsx'], counts: [0, 0, ok === 'no' ? 0 : 1] } });
  ev({ type: 'done' });
  state = { ...state, chat: { ...state.chat, running: false } };
  push();
  emit('chat:changed', state.chat);
}
// Planmodus: umsehen, dann Plan vorlegen
async function demoPlan(ev) {
  ev({ type: 'assistant-start' });
  ev({ type: 'assistant-end', content: '', stats: stats(40), calls: [] });
  ev({ type: 'tool-start', id: 'r1', name: 'list_files', label: 'Ordner', detail: '.' });
  await wait(200);
  ev({ type: 'tool-end', id: 'r1', ok: true, summary: '6 Einträge', output: '' });
  ev({ type: 'retry', text: 'Verbindung zu ASK unterbrochen. Neuer Versuch in 2 s (1/5).' });
  await wait(300);
  ev({ type: 'tool-start', id: 'p1', name: 'plan_vorlegen', label: 'Plan', detail: 'Auswertung als Excel' });
  ev({ type: 'tool-end', id: 'p1', ok: true, summary: 'Plan vorgelegt', output: '',
    plan: '**Auswertung als Excel**\n\n1. `umfrage.csv` einlesen und prüfen\n2. Summen pro Kategorie berechnen\n3. `auswertung.xlsx` mit Tabelle und Diagramm anlegen\n4. Kurz zusammenfassen, was auffällt' });
  ev({ type: 'done' });
  state = { ...state, chat: { ...state.chat, running: false } };
  push();
}
// Gedächtnis und Skills (src/main/knowledge.js): Beispieldaten, ?gedaechtnis=leer zeigt den leeren Zustand
const M293 = 'C:\\Users\\lernende\\ASK-Projekte\\Modul 293';
const JOURNAL = 'C:\\Users\\lernende\\ASK-Projekte\\Lernjournal';
const day = 86400000;
const mem = (id, text, tags, project = null, ago = 1, extra = {}) => ({ id, text, tags, scope: project ? 'projekt' : 'konto', project, chat: 'c1', chatTitle: 'Ausgaben auswerten',
  chatFolder: project || FOLDER, source: 'auto', created: now - ago * day, updated: now - ago * day, ...extra });
let memoryItems = params.get('gedaechtnis') === 'leer' ? [] : [
  mem('m1', 'Lernt Informatik im zweiten Lehrjahr an der TF Bern.', ['ausbildung', 'tf bern'], null, 20, { source: 'hand' }),
  mem('m2', 'Mag kurze Antworten in Stichworten.', ['stil', 'antworten'], null, 2, { source: 'merken' }),
  mem('m3', 'Kennt Python gut, JavaScript erst wenig.', ['python', 'javascript', 'kenntnisse'], null, 6),
  mem('m4', 'Möchte Erklärungen mit einem kurzen Beispiel.', ['stil', 'lernen'], null, 4),
  mem('m5', 'Arbeitet mit Windows 11 und VS Code.', ['werkzeuge'], null, 9),
  mem('m6', 'Bereitet sich auf die Modulprüfung 293 im Januar vor.', ['ziele', 'prüfung'], null, 3),
  mem('m7', 'Beträge immer in CHF mit zwei Nachkommastellen.', ['budget', 'format'], FOLDER, 1),
  mem('m8', 'Kategorien stehen in daten/kategorien.txt.', ['budget', 'dateien'], FOLDER, 1),
  mem('m9', 'Diagramme als Balken, nicht als Kuchen.', ['diagramme', 'stil'], FOLDER, 0.2),
  mem('m10', 'Webauftritt ohne Framework, nur HTML, CSS und JavaScript.', ['javascript', 'web'], M293, 5),
  mem('m11', 'Farben aus dem CD der TF Bern verwenden.', ['web', 'design'], M293, 5),
  mem('m12', 'Startseite heisst index.html, Bilder liegen in img/.', ['web', 'dateien'], M293, 7),
  mem('m13', 'Wochenberichte nach der Vorlage der Schule, eine Seite.', ['berichte', 'format'], JOURNAL, 8),
];
let skillItems = [
  { slug: 'wochenbericht', description: 'Wochenbericht für das Lernjournal nach der Vorlage der Schule schreiben', body: '1. Lies die Notizen der Woche in notizen/.\n2. Schreib den Bericht nach vorlage.docx, höchstens eine Seite.\n3. Speichere ihn als berichte/KW<Nummer>.docx.', updated: now - 3 * day, size: 420 },
  { slug: 'code-erklaeren', description: 'Code Zeile für Zeile erklären, mit einem kleinen Beispiel am Schluss', body: 'Erkläre den Code in kurzen Abschnitten. Keine Fachbegriffe ohne Erklärung.', updated: now - 10 * day, size: 210 },
  { slug: 'quellen-pruefen', description: 'Angaben aus dem Internet mit zwei unabhängigen Quellen prüfen', body: 'Suche mit web_search, lies zwei Quellen, nenne beide.', updated: now - 12 * day, size: 160 },
];
const memoryApi = {
  memoryList: async () => ({ enabled: state.memory !== false, items: [...memoryItems].sort((a, b) => b.updated - a.updated), projects: projects.map(({ folder, name, color }) => ({ folder, name, color })) }),
  memoryAdd: async (v) => { const item = { id: `m${Date.now()}`, text: v.text, tags: (v.tags || []).map((t) => t.trim().toLowerCase()).filter(Boolean).slice(0, 3), scope: v.scope, project: v.project, chat: null, source: 'hand', created: Date.now(), updated: Date.now() }; memoryItems = [item, ...memoryItems]; return { ok: true, item }; },
  memoryUpdate: async (id, v) => { const i = memoryItems.find((x) => x.id === id); Object.assign(i, { text: v.text, tags: (v.tags || []).map((t) => t.trim().toLowerCase()).filter(Boolean).slice(0, 3), scope: v.scope, project: v.project, updated: Date.now() }); return { ok: true, item: i }; },
  memoryRemove: async (id) => { memoryItems = memoryItems.filter((x) => x.id !== id); return true; },
  memoryClear: async () => { if (!window.confirm('Alles vergessen?')) return false; memoryItems = []; return true; },
  setMemoryEnabled: async (on) => { state = { ...state, memory: !!on }; push(); },
  onMemoryChanged: on('memory:changed'),
  skillsList: async () => skillItems.map(({ slug, description, updated, size }) => ({ slug, name: slug, description, updated, size })),
  skillsGet: async (slug) => { const s = skillItems.find((x) => x.slug === slug); return s ? { ...s, name: s.slug } : null; },
  skillsSave: async (v, previous) => {
    const slug = String(v.name || '').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');
    if (!slug) return { ok: false, reason: 'Der Name fehlt.' };
    const skill = { slug, name: slug, description: v.description, body: v.body, updated: Date.now(), size: v.body.length };
    skillItems = [...skillItems.filter((s) => s.slug !== slug && s.slug !== previous), skill].sort((a, b) => a.slug.localeCompare(b.slug));
    emit('memory:changed', {});
    return { ok: true, skill };
  },
  skillsRemove: async (slug) => { if (!window.confirm(`Skill «${slug}» löschen?`)) return false; skillItems = skillItems.filter((s) => s.slug !== slug); emit('memory:changed', {}); return true; },
  skillsExport: async () => ({ ok: false, reason: 'Exportieren geht nur in der App.' }),
  skillsImport: async () => ({ ok: false, reason: 'Importieren geht nur in der App.' }),
};

let pendingAnswer = null;
let pendingApproval = null;

// GitHub: ?github=aus (nicht verbunden), ?github=leer (verbunden, kein Repository), sonst verbunden mit Beispieldaten
const ghMode = params.get('github') || '';
const gh = {
  connected: ghMode !== 'aus', linked: ghMode !== 'aus' && ghMode !== 'leer', repo: 'lea-muster/haushaltsbudget', branch: 'main',
  branches: ['main', 'diagramm-farben'], changed: ['auswertung.py'], added: ['auswertung.xlsx', 'bilder/diagramm.png'], deleted: ['entwurf_alt.xlsx'],
  remoteNewer: true, conflicts: [],
  pulls: [{ number: 3, title: 'Kategorien für 2027 ergänzt', url: 'https://github.com/lea-muster/haushaltsbudget/pull/3', head: 'kategorien-2027', base: 'main', user: 'lea-muster', draft: false }],
};
const ghUrl = () => `https://github.com/${gh.repo}`;
const ghPending = () => gh.changed.length + gh.added.length + gh.deleted.length;
const ghMock = {
  githubStatus: async () => {
    await wait(150);
    if (!gh.connected) return { ok: true, connected: false, linked: false, login: '' };
    if (!gh.linked) return { ok: true, connected: true, linked: false, login: 'lea-muster', trusted: [] };
    return { ok: true, connected: true, linked: true, login: 'lea-muster', repo: gh.repo, branch: gh.branch, url: ghUrl(), changed: gh.changed, added: gh.added,
      deleted: gh.deleted, remoteNewer: gh.remoteNewer, conflicts: gh.conflicts, trusted: [] };
  },
  githubConnect: async () => ({ ok: false, message: 'Nur in der App.' }),
  githubDisconnect: async () => { gh.connected = false; },
  githubUntrust: async () => {},
  githubBranches: async () => ({ ok: true, current: gh.branch, default: 'main', branches: gh.branches, url: ghUrl() }),
  githubPulls: async () => ({ ok: true, pulls: gh.pulls }),
  githubPull: async () => { await wait(500); const was = gh.remoteNewer; gh.remoteNewer = false; return { ok: true, written: was ? ['ausgaben.csv'] : [], removed: [], conflicts: [] }; },
  githubClone: async (repo) => { await wait(500); Object.assign(gh, { linked: true, repo, changed: [], added: [], deleted: [], remoteNewer: false }); return { ok: true, repo, written: ['README.md'], removed: [], conflicts: [] }; },
  githubCreate: async (name) => { await wait(500); Object.assign(gh, { linked: true, repo: `lea-muster/${name}`, changed: [], added: [], deleted: [], remoteNewer: false, pulls: [] }); return { ok: true, summary: `lea-muster/${name} · 6 Dateien` }; },
  githubPush: async () => {
    await wait(500);
    if (gh.remoteNewer) return { ok: false, output: 'Auf GitHub gibt es neuere Änderungen. Zuerst holen, dann nochmals hochladen.' };
    Object.assign(gh, { changed: [], added: [], deleted: [] });
    return { ok: true, summary: `3 hochgeladen · ${gh.repo}` };
  },
  githubSwitch: async (branch) => {
    if (ghPending()) return { ok: false, message: `Nicht gewechselt: Hier gibt es ${ghPending()} Änderungen, die noch nicht hochgeladen sind. Zuerst auf ${gh.branch} hochladen oder dafür einen neuen Branch anlegen.` };
    const from = gh.branch;
    gh.branch = branch;
    return { ok: true, branch, from, written: ['index.html'], removed: [], conflicts: [] };
  },
  githubBranchCreate: async (branch) => { gh.branches = [...gh.branches, branch]; gh.branch = branch; return { ok: true, branch, from: 'main' }; },
  githubPrCreate: async (title) => {
    const number = gh.pulls.length + 4;
    const url = `${ghUrl()}/pull/${number}`;
    gh.pulls = [...gh.pulls, { number, title, url, head: gh.branch, base: 'main', user: 'lea-muster', draft: false }];
    return { ok: true, number, url, head: gh.branch, base: 'main' };
  },
};

export function install() {
  // Rechtsklick ins Textfeld oder auf markierten Text: wie der Hauptprozess (context-menu → ui:menu)
  document.addEventListener('contextmenu', (e) => {
    if (e.defaultPrevented) return;
    const editable = e.target.closest('textarea, input');
    const text = String(window.getSelection());
    if (!editable && !text) return;
    e.preventDefault();
    setTimeout(() => emit('ui:menu', { id: Date.now(), items: editable
      ? [{ key: 'edit0', label: 'Ausschneiden', hint: 'Ctrl+X', enabled: true }, { key: 'edit1', label: 'Kopieren', hint: 'Ctrl+C', enabled: true }, { key: 'edit2', label: 'Einfügen', hint: 'Ctrl+V', enabled: true }, { key: 'edit3', label: 'Alles auswählen', hint: 'Ctrl+A', enabled: true }]
      : [{ key: 'edit1', label: 'Kopieren', enabled: true }] }), 0);
  });
  window.ask = {
    getState: async () => ({ ...state, servers }),
    onState: on('state'), onFiles: on('files'), onTasks: on('tasks'),
    onMenuChooseFolder: on('x1'), onMenuNewFolder: on('x2'), onMenuGithub: on('x3'), onFolderError: on('folder:error'),
    onProjectRename: on('project:rename'), onGroupsChanged: on('groups'), onChatEvent: on('chat:event'), onChatChanged: on('chat:changed'),
    onBrowserState: on('browser:state'), onBrowserNotice: on('browser:notice'), onServers: on('servers'),
    servers: async () => servers, serverStart: async (c) => mockServerStart(String(c || '').trim()), serverOutput: async (id) => serverLines[id] || [],
    serverStop: async (id) => { servers = servers.map((x) => (x.id === id ? { ...x, state: 'gestoppt', exitCode: 1, ready: false } : x)); pushServers(); return { ok: true }; },
    serverRemove: async (id) => { servers = servers.filter((x) => x.id !== id); pushServers(); return true; },
    chooseFolder: async () => ({ ok: false, canceled: true }), newFolder: async () => ({ ok: true }), openFolder: async () => ({ ok: true }),
    forgetFolder: async () => true, folderMenu: async () => emit('ui:menu', { id: Date.now(), items: [
      ...projects.map((p, i) => ({ key: `f${i}`, label: p.folder, type: 'radio', checked: p.folder === FOLDER, enabled: true })), { separator: true },
      { key: 'a', label: 'Anderen Ordner wählen …', enabled: true }, { key: 'b', label: 'Neues Projekt …', enabled: true }, { separator: true },
      { key: 'c', label: 'Aus der Liste entfernen', enabled: true, submenu: projects.map((p, i) => ({ key: `c.${i}`, label: p.folder, enabled: true })) }, { separator: true },
      { key: 'd', label: 'Nach Updates suchen (ASK 0.2.3)', enabled: true }, { key: 'e', label: 'Beta-Versionen erhalten', type: 'checkbox', checked: false, enabled: true }] }),
    showFolder: async () => {},
    projectMenu: async () => emit('ui:menu', { id: Date.now(), items: [
      { key: '0', label: 'Neuer Chat', enabled: true }, { key: '1', label: 'Umbenennen …', enabled: true },
      { key: '2', label: 'Farbe', enabled: true, submenu: ['#0082B4', '#F28855', '#4EB68E', '#EE716D'].map((c, i) => ({ key: `2.${i}`, label: ['TF Blau', 'Orange', 'Grün', 'Rot'][i], type: 'radio', checked: i === 0, swatch: c, enabled: true })) },
      { separator: true }, { key: '4', label: 'Im Explorer öffnen', enabled: true }, { key: '5', label: 'Projektnotizen bearbeiten', enabled: true }, { separator: true },
      { key: '7', label: 'Aus der Liste entfernen', enabled: true }] }),
    projectUpdate: async (folder, patch) => { const p = projects.find((x) => x.folder === folder); if (p && patch.name) p.name = patch.name; return true; },
    setSettings: async (v) => { state = { ...state, ...v }; push(); },
    updateAction: async () => emit('ui:ask', params.get('meldung')
      ? { id: Date.now(), type: 'warning', message: 'Die Suche nach Updates hat nicht geklappt.', detail: 'Ist der Laptop mit dem Internet verbunden?\n\nnet::ERR_INTERNET_DISCONNECTED', buttons: ['OK'] }
      : { id: Date.now(), type: 'question', message: 'ASK 0.2.4 ist da. Jetzt aktualisieren?', buttons: ['Jetzt aktualisieren', 'Später'], defaultId: 0, cancelId: 1,
        kind: 'update', version: '0.2.4', current: '0.2.3', beta: false,
        intro: 'Du hast 0.2.3. ASK lädt das Update im Hintergrund (Balken oben in der App), du kannst weiterarbeiten. Danach startet ASK von selbst neu, ausser während eines Auftrags. Chats, Ordner und Einstellungen bleiben.',
        notes: '• Die Tabs oben sehen aufgeräumter aus und zeigen die Farbe des Projekts.\n• Tabs lassen sich verschieben, umbenennen und mit Ctrl+W schliessen.\n• Updates erscheinen in einem eigenen Fenster der App.' }),
    onUiAsk: on('ui:ask'), onUiMenu: on('ui:menu'),
    uiAnswer: async (id, value) => {
      console.log('uiAnswer', id, value);
      // wie webContents.paste()/copy() im Hauptprozess: wirkt auf das Feld mit dem Fokus
      if (value === 'edit2') document.execCommand('insertText', false, 'EINGEFÜGT');
      if (value === 'edit1') console.log('kopiert:', String(window.getSelection()) || document.activeElement?.value?.slice(document.activeElement.selectionStart, document.activeElement.selectionEnd));
    }, updateLater: async () => {}, setBetaUpdates: async (on) => { state = { ...state, update: { ...state.update, beta: on } }; push(); },
    setPrefs: async (v) => { state = { ...state, prefs: { ...state.prefs, ...v } }; push(); },
    usageGet: async (withAsk = true) => (withAsk ? (await wait(600), { ...mockUsage(), ask: params.get('ohneask') ? null : { webChats: 42, webChats30: 12 } }) : { ...mockUsage(), ask: null }),
    chatRewind: async () => ({ ok: true, text: 'Räum den Ordner auf', attachments: [] }),
    chatRename: async (_f, id, title) => { const c = projects.flatMap((p) => p.chats).find((x) => x.id === id); if (c) c.title = title; return true; },
    chatPin: async (_f, id, on) => { const c = projects.flatMap((p) => p.chats).find((x) => x.id === id); if (c) c.pinned = on; return true; },
    chatSearch: async (q) => (/proxmox|budget/i.test(q) ? [{ folder: FOLDER, name: 'Haushaltsbudget', id: 'c2', title: 'Budgetplan 2027', updated: Date.now(), snippets: ['Person: Budget für 2027 aufstellen …'] }] : []),
    listDir: async (rel) => ({ ok: true, entries: tree[rel] || [] }),
    findFiles: async (q) => Object.values(tree).flat().filter((e) => e.path.toLowerCase().includes(String(q).toLowerCase())),
    importFiles: async () => ({ ok: true, paths: [] }),
    restartTerminal: async () => {}, terminalLogs: async () => 'open-terminal läuft auf 127.0.0.1', acceptWarning: async () => {},
    setAskUrl: async () => ({ ok: true }), connectAsk: async () => state.ask, logout: async () => {}, models: async () => state.models,
    askLoginWindow: async () => true, askOpenBrowser: async () => {}, askDefaultUrl: async () => true,
    askUseToken: async (t) => (String(t).trim() ? { ok: true } : { ok: false, reason: 'Das sieht nicht wie ein Token aus. Kopiere es ganz, ohne Leerzeichen.' }),
    setModel: async (id) => { state = { ...state, model: id }; push(); }, setWebSearch: async (v) => { state = { ...state, webSearch: v }; push(); },
    setChatMemory: async (v) => { state = { ...state, chatMemory: v }; push(); }, setThinking: async (v) => { state = { ...state, thinking: v }; push(); },
    chatSend: async (text) => { demoRun(text); return { ok: true }; }, chatUndo: async () => ({ ok: true }), chatStop: async () => {}, chatStopChat: async () => true,
    chatGoal: async () => {}, chatApprove: async (_id, a) => { pendingApproval?.(a); return true; }, chatAnswer: async (_id, t) => { pendingAnswer?.(t); return true; },
    ...ghMock,
    chatNew: async () => { state = { ...state, chat: { id: `neu${Date.now()}`, title: '', running: false } }; transcriptItems = []; push(); return { ok: true }; },
    chatGroups: async () => projects.map((p) => ({ ...p, current: p.folder === state.folder })),
    chatOpen: async (_f, id) => { const c = projects.flatMap((p) => p.chats).find((x) => x.id === id); state = { ...state, chat: { id, title: c?.title || '', running: false } }; transcriptItems = id === 'c1' ? saved : []; push(); return { ok: true }; },
    chatDelete: async (_f, _id, title) => { emit('ui:ask', { id: Date.now(), type: 'question', buttons: ['Löschen', 'Abbrechen'], defaultId: 1, cancelId: 1, dangerId: 0,
      message: 'Chat löschen?', detail: `«${title}» wird aus ${FOLDER} entfernt. Die Dateien im Ordner bleiben.` }); return false; }, chatTranscript: async () => ({ ...state.chat, items: transcriptItems }),
    reloadAsk: async () => {}, setSidebar: async (v) => { state = { ...state, sidebarOpen: v }; push(); }, setChatbar: async (v) => { state = { ...state, chatbarOpen: v }; push(); },
    setAskBounds: () => {}, setAskVisible: () => {}, setBrowserBounds: () => {}, setBrowserVisible: () => {},
    browserShow: async () => {}, browserGo: async () => {}, browserBack: async () => {}, browserForward: async () => {}, browserClear: async () => {}, browserReload: async () => {}, browserClose: async () => {},
    copy: async (t) => navigator.clipboard?.writeText(t).catch(() => {}), openLink: async (u) => window.open(u, '_blank'),
    files: async () => files, tasks: async () => null,
    preview: async (rel) => (/\.(csv|py|txt)$/.test(rel) ? { kind: 'text', text: rel.endsWith('.csv') ? 'Datum;Kategorie;Beschreibung;Betrag CHF\n02.08.2026;Bücher;Fachbuch Netzwerktechnik;32.75\n02.08.2026;Verpflegung;Mittagessen Mensa;15.85' : 'import pandas as pd\n\ndf = pd.read_csv("ausgaben.csv", sep=";", encoding="cp1252")\nsummen = df.groupby("Kategorie")["Betrag CHF"].sum()' } : { kind: 'office', name: rel }),
    openFile: async () => '', toPdf: async () => ({ fehler: 'Nur in der App.' }), showFile: async () => {},
    ...memoryApi,
  };
}
