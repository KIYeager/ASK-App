// Attrappe der Schnittstelle zum Hauptprozess für «npm run dev:ui» im Browser: Beispielprojekte,
// ein gespeicherter Chat und ein vorgespielter Auftrag mit Frage und Rückfrage. Nicht in Electron.
const listeners = {};
const on = (ch) => (cb) => {
  (listeners[ch] ||= new Set()).add(cb);
  return () => listeners[ch].delete(cb);
};
const emit = (ch, payload) => (listeners[ch] || []).forEach((cb) => cb(payload));
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
import uiPrelude from '../../../../resources/ask-ui/vorspann.html?raw';

const params = new URLSearchParams(location.search);

const FOLDER = 'C:\\Users\\lernende\\ASK-Projekte\\Haushaltsbudget';
const stats = (t) => ({ tokens: t, genMs: t * 7, totalMs: t * 9, firstMs: 600, promptTokens: 4200 });

let state = {
  askUrl: 'https://ask.ict-tfbern.ch', folder: FOLDER, recent: [FOLDER], warningAccepted: true, sidebarOpen: true, chatbarOpen: true,
  terminal: { state: 'läuft', folder: FOLDER, lowIntegrity: true }, ask: { state: 'angemeldet', user: 'Lea Muster' },
  models: [{ id: 'qwen3-coder', name: 'Qwen3 Coder' }, { id: 'gpt-oss', name: 'GPT-OSS 120B' }], model: 'qwen3-coder',
  contextSize: 250000, contextReserve: 30000,
  webSearch: false, chatMemory: false, interactiveUi: params.get('interaktiv') !== 'aus', memory: true, thinking: 'mittel', webSearchAvailable: true, approvals: 'heikel', mode: 'auto', theme: 'system',
  version: '0.2.3', update: { state: params.get('update') || 'verfügbar', current: '0.2.3', version: '0.2.4', percent: 42, countdown: 7, beta: false, enabled: true, portable: false, postponed: false },
  prefs: { verbose: false, todoCollapsed: true, tourDone: !params.get('tour') }, // ?tour=1 zeigt die Einführung
  chat: { id: 'c1', title: 'Ausgaben auswerten', running: false, goal: null, pinned: false, context: { tokens: 18400, estimated: false, limit: 131072 } },
  modsCode: params.get('mods') === 'an', catalogs: [{ repo: 'KIYeager/ASK-Erweiterungen', eigen: false }, { repo: 'anthropics/skills', eigen: false }],
};
if (params.get('offline')) state = { ...state, ask: { state: 'unbekannt', loadError: { code: -106, description: 'ERR_INTERNET_DISCONNECTED' } }, models: [], model: '' }; // ?offline=1
if (params.get('dns')) state = { ...state, secureDns: true, askUrl: 'https://ask.ict-tfbern.ch', ask: { state: 'unbekannt', loadError: { code: -105, description: 'ERR_NAME_NOT_RESOLVED', url: 'https://ask.ict-tfbern.ch/', proxy: 'DIRECT' } }, models: [], model: '' }; // ?dns=1
if (params.get('askfehler')) state = { ...state, ask: { state: 'fehler', user: 'Lea Muster', message: 'ASK bietet diesem Konto kein Modell an. Wer ASK betreut, muss deinem Konto zuerst ein Modell freigeben.' }, models: [], model: '' }; // ?askfehler=1
if (params.get('leer')) state.chat = { id: 'neu1', title: '', running: false, goal: null };
// Chats ohne Projekt (E78): eigener Ordner in den Daten der App, kein Projekt. ?frei=1 startet dort, ?erst=1 wie beim ersten Start.
const FREE = 'C:\\Users\\lernende\\AppData\\Roaming\\ASK\\chats';
if (params.get('erst')) state = { ...state, folder: FREE, free: true, recent: [], chat: { id: 'neu1', title: '', running: false } };
if (params.get('frei')) state = { ...state, folder: FREE, free: true, chat: { id: 'f1', title: 'Was ist ein Subnetz?', running: false, context: { tokens: 900, estimated: false, limit: 131072 } } };
if (params.get('url')) state = { ...state, askUrl: '', folder: '', chat: { id: 'neu1', title: '', running: false } }; // ?url=1: Adresse fehlt noch

const allProjects = [
  { folder: FOLDER, name: 'Haushaltsbudget', color: '#0082B4', chats: [{ id: 'c1', title: 'Ausgaben auswerten', updated: Date.now() - 5 * 60000, count: 2 }, { id: 'c2', title: 'Budgetplan 2027', updated: Date.now() - 26 * 3600000, count: 4 }] },
  { folder: 'C:\\Users\\lernende\\ASK-Projekte\\Modul 293', name: 'Modul 293 · Webauftritt', color: '#F28855', chats: [{ id: 'c3', title: 'Startseite in HTML', updated: Date.now() - 3 * 86400000, count: 6 }, { id: 'c4', title: 'Bilder verkleinern', updated: Date.now() - 4 * 86400000, count: 1 }] },
  { folder: 'C:\\Users\\lernende\\ASK-Projekte\\Lernjournal', name: 'Lernjournal', color: '#4EB68E', chats: [{ id: 'c5', title: 'Wochenbericht KW 47', updated: Date.now() - 6 * 86400000, count: 3 }] },
];
const projects = params.get('erst') ? [] : allProjects;
const freeChats = params.get('erst') ? [] : [{ id: 'f1', title: 'Was ist ein Subnetz?', updated: Date.now() - 2 * 3600000, count: 1 }];

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
// Quellen (E84): Ordner «Quellen» mit einer Unterlage; ?quellen=1 zeigt dazu eine Antwort mit Verweisen
tree[''].unshift({ path: 'Quellen', name: 'Quellen', dir: true, size: 0, modified: now - 7200000 });
tree.Quellen = [{ path: 'Quellen\\Skript Budgetplanung.pdf', name: 'Skript Budgetplanung.pdf', dir: false, size: 412000, modified: now - 7200000 }];
files.quellen = 1;

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
if (params.get('quellen')) {
  saved.push({ role: 'user', content: 'Was ist laut Skript der Unterschied zwischen fixen und variablen Kosten?', time: now - 120000 },
    { role: 'assistant', content: '', stats: stats(150), calls: [{ id: 'q1', name: 'quellen_suchen', label: 'Quellen', detail: '«fixe variable Kosten»' }, { id: 'q2', name: 'quelle_lesen', label: 'Quelle', detail: 'Quellen/Skript Budgetplanung.pdf, S. 4' }] },
    { role: 'tool', id: 'q1', ok: true, summary: '3 Treffer', output: '[1] Quellen/Skript Budgetplanung.pdf · S. 4 · …' },
    { role: 'tool', id: 'q2', ok: true, summary: 'S. 4', output: 'Fixe Kosten fallen jeden Monat gleich an …' },
    { role: 'assistant', stats: stats(380), time: now - 100000,
      content: 'Laut Skript sind **fixe Kosten** Ausgaben, die jeden Monat gleich hoch sind, zum Beispiel Miete, ÖV-Abo oder Handy-Abo [Skript Budgetplanung, S. 4](quelle:Quellen/Skript%20Budgetplanung.pdf#seite=4). **Variable Kosten** schwanken je nach Verhalten, etwa Freizeit, Kleider oder Ausgang [Skript Budgetplanung, S. 5](quelle:Quellen/Skript%20Budgetplanung.pdf#seite=5).\n\nDas Skript empfiehlt, zuerst die fixen Kosten vom Lohn abzuziehen und den Rest auf die variablen Kosten und das Sparen zu verteilen [Skript Budgetplanung, S. 7](quelle:Quellen/Skript%20Budgetplanung.pdf#seite=7).',
      sources: [{ url: 'quelle:Quellen/Skript%20Budgetplanung.pdf#seite=4', title: 'Skript Budgetplanung, S. 4' }, { url: 'quelle:Quellen/Skript%20Budgetplanung.pdf#seite=5', title: 'Skript Budgetplanung, S. 5' },
        { url: 'quelle:Quellen/Skript%20Budgetplanung.pdf#seite=7', title: 'Skript Budgetplanung, S. 7' }] });
}

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

const savedFree = [
  { role: 'user', content: 'Was ist ein Subnetz?', time: now - 2 * 3600000 },
  { role: 'assistant', content: 'Ein **Subnetz** ist ein Teil eines grösseren Netzes. Die Subnetzmaske legt fest, welcher Teil der IP-Adresse das Netz bezeichnet und welcher den einzelnen Rechner.\n\nBeispiel: `192.168.1.0/24` hat 254 nutzbare Adressen.', stats: stats(160) },
];

// ?frei=1&interaktiv=demo: alle Bausteine der interaktiven Antworten (E80) im Chat ohne Projekt
const uiDemo = [
  'Hier die Übersicht als Diagramm, dazu ein kurzes Quiz.',
  '```ask-ui',
  JSON.stringify([
    { typ: 'diagramm', titel: 'Ausgaben pro Monat', art: 'balken', einheit: 'CHF', beschriftungen: ['Jan', 'Feb', 'Mär', 'Apr'], reihen: [{ name: '2025', werte: [420, 380, 510, 460] }, { name: '2026', werte: [450, 395, 470, 520] }] },
    { typ: 'diagramm', titel: 'Aufteilung', art: 'kreis', einheit: 'CHF', beschriftungen: ['ÖV-Abo', 'Freizeit', 'Bücher', 'Handy'], reihen: [{ werte: [500, 460.75, 411.15, 150] }] },
    { typ: 'quiz', titel: 'Kurz geprüft', fragen: [{ frage: 'Wie viele nutzbare Adressen hat ein /24-Netz?', antworten: ['256', '254', '255'], richtig: 1, erklaerung: 'Netz- und Broadcast-Adresse fallen weg: 256 − 2 = 254.' }] },
  ]),
  '```',
  '',
  '```ask-ui',
  JSON.stringify([
    { typ: 'hinweis', art: 'tipp', titel: 'Tipp', text: 'Mit dem Rechner kannst du die Werte selbst ausprobieren.' },
    { typ: 'rechner', titel: 'Sparrechner', eingaben: [{ name: 'betrag', label: 'Pro Monat', wert: 50, min: 0, max: 500, schritt: 10, einheit: 'CHF' }, { name: 'monate', label: 'Monate', wert: 12, min: 1, max: 60 }],
      ergebnisse: [{ label: 'Gespart', formel: 'betrag * monate', einheit: 'CHF', stellen: 0 }, { label: 'Pro Woche', formel: 'round(betrag * 12 / 52, 2)', einheit: 'CHF' }] },
    { typ: 'karteikarten', titel: 'Begriffe', karten: [{ vorne: 'Subnetzmaske', hinten: 'Legt fest, welcher Teil der IP-Adresse das Netz ist.' }, { vorne: 'Gateway', hinten: 'Der Router, über den ein Netz andere Netze erreicht.' }] },
    { typ: 'vergleich', titel: 'IPv4 oder IPv6', spalten: [{ titel: 'IPv4', untertitel: '32 Bit', punkte: ['etwa 4 Mrd. Adressen', 'mit NAT verbreitet'] }, { titel: 'IPv6', untertitel: '128 Bit', punkte: ['fast unbegrenzt viele Adressen', 'kein NAT nötig'], hervorheben: true }] },
    { typ: 'schritte', titel: 'Netz einrichten', schritte: [{ titel: 'Adressbereich wählen', text: 'z. B. 192.168.1.0/24' }, 'Gateway festlegen', 'DHCP einschalten'] },
    { typ: 'auswahl', frage: 'Womit machen wir weiter?', optionen: ['Mehr Übungen', 'Subnetze rechnen', 'Fertig'] },
    { typ: 'formular', titel: 'Dein Netz', felder: [{ name: 'geraete', label: 'Wie viele Geräte?', art: 'zahl', pflicht: true }, { name: 'ort', label: 'Wo?', art: 'auswahl', optionen: ['Zuhause', 'Schule'] }], senden: 'Plan erstellen' },
  ]),
  '```',
  '',
  'Und ein kleines Programm:',
  '```ask-html',
  '<div class="ask-tabs"><button data-tab="w">Würfel</button><button data-tab="n">Noten</button></div>',
  '<div data-panel="w" class="ask-stack"><div class="ask-card ask-row"><span class="ask-big" id="w">–</span><span class="ask-muted">Noch nicht gewürfelt</span></div>',
  '<div class="ask-row"><button class="ask-primary" id="b">Würfeln</button><button id="s">Ergebnis senden</button></div></div>',
  '<div data-panel="n" class="ask-bars"><div><span>Modul 129</span><span class="ask-bar" style="--wert:82"></span><span>82</span></div>',
  '<div><span>Allgemeinbildung und Sport</span><span class="ask-bar" style="--wert:74"></span><span>74</span></div></div>',
  '<script>let z=0;b.onclick=()=>{z=1+Math.floor(Math.random()*6);w.textContent=z};s.onclick=()=>ask.senden("Ich habe eine "+z+" gewürfelt.")</script>',
  '```',
].join('\n');
if (params.get('interaktiv') === 'demo') savedFree.push({ role: 'user', content: 'Zeig mir alles interaktiv.', time: now - 3600000 }, { role: 'assistant', content: uiDemo, stats: stats(400) });

// Chats der Attrappe: jeder mit eigenem Verlauf und eigenem Auftrag, damit parallele Chats und der Wechsel prüfbar sind
const store = new Map(); // id → { id, folder, items, running, started, pending, waits, stopped }
const chatOf = (id, folder = state.folder) => {
  if (!store.has(id)) store.set(id, { id, folder, items: id === 'c1' && !params.get('leer') ? [...saved] : id === 'f1' ? [...savedFree] : [], running: false, started: 0, pending: [], waits: {}, stopped: false });
  return store.get(id);
};
const listed = (folder) => (folder === FREE ? freeChats : projects.find((x) => x.folder === folder)?.chats || []);
const entryOf = (id) => [...freeChats, ...projects.flatMap((x) => x.chats)].find((x) => x.id === id);
const titleOf = (id) => entryOf(id)?.title || '';
const runningList = () => [...store.values()].filter((c) => c.running).map((c) => ({ folder: c.folder, id: c.id, title: titleOf(c.id) }));
const chatState = (c) => ({ id: c.id, title: titleOf(c.id), running: c.running, started: c.running ? c.started : 0, goal: null, pinned: !!entryOf(c.id)?.pinned,
  context: c.items.length ? { tokens: 900 + c.items.length * 2400, estimated: false, limit: 131072 } : null });
// Sichtbaren Chat wechseln (wie show_agent im Hauptprozess)
function show(id, folder) {
  const c = chatOf(id, folder);
  state = { ...state, folder: c.folder, free: c.folder === FREE, terminal: { ...state.terminal, folder: c.folder }, chat: chatState(c) };
  push();
  emit('chat:changed', state.chat);
}

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

const full = () => ({ ...state, running: runningList(), servers: state.free ? [] : servers });
const push = () => emit('state', full());

// Ein Auftrag gehört zu seinem Chat: Ereignisse tragen chatId und folder, der Verlauf wächst mit (für den Wechsel zurück)
const SPEED = params.get('langsam') ? 6 : 1;
async function demoRun(c, text) {
  const ev = (e) => emit('chat:event', { ...e, chatId: c.id, folder: c.folder });
  const sync = () => {
    if (state.chat.id === c.id) { state = { ...state, chat: chatState(c) }; push(); emit('chat:changed', state.chat); } else push();
    emit('groups');
  };
  const pause = (ms) => wait(ms * SPEED);
  // Frage oder Rückfrage: wartet auf die Person, bleibt offen (pending), auch wenn man den Chat wechselt
  const ask = (id, event) => new Promise((resolve) => { c.pending.push(event); c.waits[id] = (v) => { c.pending = c.pending.filter((x) => x.id !== id); delete c.waits[id]; resolve(v); }; ev(event); });
  const finish = (extra = {}) => { c.running = false; c.pending = []; c.waits = {}; ev({ type: 'done', ...extra }); sync(); };
  if (!entryOf(c.id)) listed(c.folder).unshift({ id: c.id, title: text.slice(0, 60), updated: Date.now(), count: 0 });
  entryOf(c.id).count += 1;
  entryOf(c.id).updated = Date.now();
  Object.assign(c, { running: true, started: Date.now(), stopped: false });
  c.items.push({ role: 'user', content: text, time: Date.now() });
  sync();
  ev({ type: 'user', text, attachments: [] });
  await pause(400);
  if (c.stopped) return finish({ stopped: true });
  if (/fehler/i.test(text)) {
    const msg = 'error sending request for url (https://ask.ict-tfbern.ch/api/chat/completions): dns error';
    c.items.push({ role: 'error', text: msg });
    ev({ type: 'error', text: msg });
    return finish({ failed: true });
  }
  if (c.folder === FREE) return demoFree(c, ev, pause, finish, text);
  if (state.mode === 'plan') return demoPlan(c, ev, pause, finish);
  ev({ type: 'assistant-start' });
  for (const w of 'Ich räume den Ordner auf. '.split(' ')) { ev({ type: 'reasoning', text: `${w} ` }); await pause(40); }
  ev({ type: 'assistant-end', content: '', stats: stats(90), calls: [] });
  c.items.push({ role: 'assistant', content: '', reasoning: 'Ich räume den Ordner auf.', stats: stats(90), calls: [{ id: 'd1', name: 'list_files', label: 'Ordner', detail: '.' }] });
  ev({ type: 'tool-start', id: 'd1', name: 'list_files', label: 'Ordner', detail: '.' });
  await pause(300);
  c.items.push({ role: 'tool', id: 'd1', ok: true, summary: '6 Einträge', output: '' });
  ev({ type: 'tool-end', id: 'd1', ok: true, summary: '6 Einträge', output: '' });
  if (c.stopped) return finish({ stopped: true });
  const q = { frage: 'Welche Dateien darf ich löschen?', optionen: ['Nur Zwischendateien', 'Auch alte Entwürfe', 'Nichts löschen'] };
  c.items.push({ role: 'assistant', content: '', stats: stats(40), calls: [{ id: 'q1', name: 'frage_stellen', label: 'Frage', detail: q.frage, optionen: q.optionen }] });
  ev({ type: 'tool-start', id: 'q1', name: 'frage_stellen', label: 'Frage', detail: q.frage });
  const answer = await ask('q1', { type: 'question', id: 'q1', ...q });
  if (c.stopped) { ev({ type: 'question-end', id: 'q1', answer: '' }); return finish({ stopped: true }); }
  ev({ type: 'question-end', id: 'q1', answer });
  c.items.push({ role: 'tool', id: 'q1', ok: true, summary: answer, output: `Antwort: ${answer}` });
  ev({ type: 'tool-end', id: 'q1', ok: true, summary: answer, output: `Antwort: ${answer}` });
  c.items.push({ role: 'assistant', content: '', stats: stats(30), calls: [{ id: 'd2', name: 'run_command', label: 'Befehl', detail: 'del /q entwurf_alt.xlsx' }] });
  ev({ type: 'tool-start', id: 'd2', name: 'run_command', label: 'Befehl', detail: 'del /q entwurf_alt.xlsx' });
  const ok = await ask('p1', { type: 'approval', id: 'p1', kind: 'loeschen', title: 'Darf ASK Dateien löschen?', detail: 'del /q entwurf_alt.xlsx', reason: 'Der Befehl löscht Dateien oder Ordner. Sie kommen in die Sicherung und lassen sich mit «Rückgängig» zurückholen.' });
  ev({ type: 'approval-end', id: 'p1', answer: ok || 'no' });
  const denied = !ok || ok === 'no';
  c.items.push({ role: 'tool', id: 'd2', ok: !denied, summary: denied ? 'abgelehnt' : '', output: denied ? 'Die Person hat diesen Schritt abgelehnt.' : '' });
  ev({ type: 'tool-end', id: 'd2', ok: !denied, summary: denied ? 'abgelehnt' : '', output: denied ? 'Die Person hat diesen Schritt abgelehnt.' : '' });
  if (c.stopped) return finish({ stopped: true });
  const reply = 'Erledigt. Ich habe den Ordner aufgeräumt; `auswertung.xlsx` bleibt unverändert.';
  ev({ type: 'assistant-start' });
  for (const w of reply.split(' ')) { ev({ type: 'text', text: `${w} ` }); await pause(50); }
  c.items.push({ role: 'assistant', content: reply, stats: stats(70) });
  ev({ type: 'assistant-end', content: reply, stats: stats(70) });
  ev({ type: 'changes', snapshot: 's2', changes: { added: [], changed: [], removed: denied ? [] : ['entwurf_alt.xlsx'], counts: [0, 0, denied ? 0 : 1] } });
  return finish();
}
// Chat ohne Projekt: nur eine Antwort, keine Werkzeuge
async function demoFree(c, ev, pause, finish, text) {
  const reply = /bild/i.test(text) ? 'Hier ist dein Bild. Soll ich etwas daran ändern?'
    : 'Gern. Kurz erklärt: Das ist ein Chat ohne Projekt. Ich kann hier antworten, im Web suchen und Bilder erstellen. Für Dateien und Befehle legst du links ein Projekt an.';
  ev({ type: 'assistant-start' });
  for (const w of reply.split(' ')) {
    if (c.stopped) return finish({ stopped: true });
    ev({ type: 'text', text: `${w} ` });
    await pause(60);
  }
  c.items.push({ role: 'assistant', content: reply, stats: stats(60) });
  ev({ type: 'assistant-end', content: reply, stats: stats(60) });
  return finish();
}
// Planmodus: umsehen, dann Plan vorlegen
async function demoPlan(c, ev, pause, finish) {
  ev({ type: 'assistant-start' });
  ev({ type: 'assistant-end', content: '', stats: stats(40), calls: [] });
  c.items.push({ role: 'assistant', content: '', stats: stats(40), calls: [{ id: 'r1', name: 'list_files', label: 'Ordner', detail: '.' }] });
  ev({ type: 'tool-start', id: 'r1', name: 'list_files', label: 'Ordner', detail: '.' });
  await pause(200);
  c.items.push({ role: 'tool', id: 'r1', ok: true, summary: '6 Einträge', output: '' });
  ev({ type: 'tool-end', id: 'r1', ok: true, summary: '6 Einträge', output: '' });
  ev({ type: 'retry', text: 'Verbindung zu ASK unterbrochen. Neuer Versuch in 2 s (1/5).' });
  await pause(300);
  const plan = '**Auswertung als Excel**\n\n1. `umfrage.csv` einlesen und prüfen\n2. Summen pro Kategorie berechnen\n3. `auswertung.xlsx` mit Tabelle und Diagramm anlegen\n4. Kurz zusammenfassen, was auffällt';
  c.items.push({ role: 'assistant', content: '', stats: stats(40), calls: [{ id: 'p1', name: 'plan_vorlegen', label: 'Plan', detail: 'Auswertung als Excel' }] });
  ev({ type: 'tool-start', id: 'p1', name: 'plan_vorlegen', label: 'Plan', detail: 'Auswertung als Excel' });
  c.items.push({ role: 'tool', id: 'p1', ok: true, summary: 'Plan vorgelegt', output: '', plan });
  ev({ type: 'tool-end', id: 'p1', ok: true, summary: 'Plan vorgelegt', output: '', plan });
  return finish();
}
// Stoppen: offene Frage oder Rückfrage endet ohne Antwort, der Auftrag hört beim nächsten Schritt auf
function stopChat(id) {
  const c = store.get(id);
  if (!c?.running) return;
  c.stopped = true;
  Object.values(c.waits).forEach((resolve) => resolve(null));
}
// Gedächtnis und Skills (src/main/knowledge.js): Beispieldaten, ?gedaechtnis=leer zeigt den leeren Zustand
const M293 = 'C:\\Users\\lernende\\ASK-Projekte\\Modul 293';
const JOURNAL = 'C:\\Users\\lernende\\ASK-Projekte\\Lernjournal';
const day = 86400000;
const mem = (id, text, tags, project = null, ago = 1, extra = {}) => ({ id, text, tags, scope: project ? 'projekt' : 'konto', project, chat: 'c1', chatTitle: 'Ausgaben auswerten',
  chatFolder: project || FOLDER, source: 'auto', created: now - ago * day, updated: now - ago * day, ...extra });
const manyMemories = () => [
  ['Lernt Informatik EFZ an der TF Bern, zweites Lehrjahr.', ['ausbildung', 'tfbern']], ['Schreibt jeden Tag einen Tagesbericht ins Lernjournal Kytt.', ['lernjournal', 'kytt']],
  ['Betreibt ein Homelab mit Proxmox und einem Mac mini.', ['homelab', 'geräte']], ['Nutzt vLLM für lokale Inferenz auf der eigenen Grafikkarte.', ['vllm', 'inferenz']],
  ['Baut Webprojekte mit Vite, React und WebGL.', ['webprojekte', 'stack']], ['Mag dunkle, ruhige Oberflächen mit wenig Farbe.', ['design', 'ui']],
  ['Will Antworten auf Hochdeutsch, nie in Mundart.', ['sprache', 'antworten']], ['Erwartet, dass Rückfragen zum Anklicken gestellt werden.', ['fragen', 'arbeitsweise']],
  ['Legt Wert auf Qualität: lieber prüfen als raten.', ['qualität', 'arbeitsweise']], ['Arbeitet mit mehreren KI-Abos und vergleicht die Modelle.', ['ki-abos', 'tools']],
  ['Überwacht seine Server mit Grafana und Uptime Kuma.', ['monitoring', 'homelab']], ['Verbindet sich von unterwegs per WireGuard mit dem Heimnetz.', ['vpn', 'homelab']],
  ['Hat eine Smartwatch und wertet die Daten selbst aus.', ['wearables', 'geräte']], ['Spielt gern Strategiespiele und baut kleine Spiele selbst.', ['spiele', 'interessen']],
  ['Präsentationen hält er lieber mit wenigen Folien und Live-Demo.', ['praesentationen', 'schule']], ['Dokumentiert Projekte in Markdown im Repository.', ['doku', 'arbeitsweise']],
  ['Lässt Agenten Aufgaben aufteilen und prüft die Ergebnisse selbst.', ['agenten', 'orchestrierung']], ['Nutzt Vorlagen der Schule für Berichte.', ['vorlage', 'schule']],
  ['Automatisiert Wiederkehrendes mit kleinen Skripten.', ['automationen', 'tools']], ['Wohnt im Kanton Bern.', ['wohnort']],
  ['Reflektiert am Freitag die Woche im Lernjournal.', ['reflexion', 'lernjournal']], ['Interessiert sich für Bildgenerierung mit Flux.', ['bildgenerierung', 'interessen']],
  ['Kauft Hardware gebraucht und rüstet sie auf.', ['kauf', 'geräte']], ['Module der Schule: 293, 164 und 319 laufen gerade.', ['module', 'schule']],
].map(([text, tags], i) => mem(`v${i}`, text, tags, null, i * 0.7, { source: i % 3 ? 'import' : 'auto' }));
let memoryItems = params.get('gedaechtnis') === 'leer' ? [] : params.get('gedaechtnis') === 'viel' ? [...manyMemories(), mem('vp', 'Die Szene rendert mit WebGL und einem eigenen Shader.', ['webgl', 'schwarzes-loch'], M293, 1)] : [
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
  memoryImportRead: async (text) => {
    await wait(700);
    const lines = String(text).split('\n').map((l) => l.replace(/^\s*[-*•\d.)]+\s*/, '').trim()).filter((l) => l.length >= 8);
    return { ok: true, byModel: true, items: lines.map((l, i) => ({ text: l, tags: [['ausbildung'], ['stil', 'antworten'], ['werkzeuge']][i % 3], known: memoryItems.some((m) => m.text === l) })) };
  },
  memoryImportSave: async (list) => {
    for (const v of list) memoryItems = [...memoryItems, mem(`m${Date.now()}${Math.random()}`, v.text, v.tags, null, 0, { source: 'import', chat: null })];
    emit('memory:changed', {});
    return { ok: true, added: list.length, failed: 0 };
  },
  memoryClear: async () => { if (!window.confirm('Alles vergessen?')) return false; memoryItems = []; return true; },
  setMemoryEnabled: async (on) => { state = { ...state, memory: !!on }; push(); },
  onMemoryChanged: on('memory:changed'),
  skillsList: async () => [
    ...skillItems.map(({ slug, description, updated, size }) => ({ slug, name: slug, description, updated, size, source: 'eigen', enabled: true, replaced: false })),
    ...standardSkills.map(([slug, description]) => ({ slug, name: slug, description, updated: now, size: 2000, source: 'standard', enabled: !skillsOff.has(slug), replaced: skillItems.some((x) => x.slug === slug) })),
  ].sort((a, b) => a.slug.localeCompare(b.slug)),
  skillsGet: async (slug, source) => {
    const std = standardSkills.find(([n]) => n === slug);
    if (source === 'standard' && std) return { slug, name: slug, description: std[1], body: '# Mitgelieferter Skill\n\n1. …', source: 'standard' };
    const s = skillItems.find((x) => x.slug === slug); return s ? { ...s, name: s.slug, source: 'eigen' } : null;
  },
  skillsEnable: async (slug, on) => { if (on) skillsOff.delete(slug); else skillsOff.add(slug); emit('memory:changed', {}); return true; },
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

const standardSkills = [
  ['code-erklaeren', 'Code Schritt für Schritt erklären, passend zum Wissensstand. Benutzen, wenn jemand Code verstehen will.'],
  ['code-review', 'Code auf Fehler, Lesbarkeit und Sicherheit prüfen und freundlich erklären, was besser geht.'],
  ['commit-nachricht', 'Aus den Änderungen eine gute Commit-Nachricht schreiben.'],
  ['fehler-suchen', 'Fehler systematisch finden: nachstellen, Meldung lesen, eingrenzen, beheben, prüfen.'],
  ['lernjournal', 'Lernjournal oder Wochenbericht schreiben: was gemacht, was gelernt, Probleme, nächste Schritte.'],
  ['mod-erstellen', 'Einen Mod für ASK erstellen: Aussehen, Befehle, Bereiche, Figuren oder Regeln.'],
  ['praesentation-planen', 'Eine Präsentation planen: Ziel, Publikum, Aufbau, eine Botschaft pro Folie.'],
  ['quellen-pruefen', 'Quellen und Behauptungen prüfen: wer, wann, Belege, Vergleich mit anderen Quellen.'],
  ['skill-erstellen', 'Einen eigenen Skill schreiben: wiederkehrenden Auftrag als Anleitung festhalten.'],
  ['tests-schreiben', 'Tests für vorhandenen Code schreiben und ausführen.'],
  ['webseite-gestalten', 'Gut aussehende Webseiten mit HTML und CSS gestalten.'],
];
const skillsOff = new Set();
let mods = [
  { id: 'robo-helfer', name: 'Robo-Helfer', version: '1.0.0', beschreibung: 'Ein kleiner Roboter, der auf ASK reagiert, Tipps gibt und sich verschieben lässt.', autor: 'ASK', style: 'style.css', script: 'mod.js', befehle: [], anweisungen: '', nachfragen: [], aktiv: params.get('mods') === 'an', geaendert: false, groesse: 6400, dateien: 3, hash: 'abc', fehler: '' },
  { id: 'vokabeln', name: 'Vokabeln und einfach erklärt', version: '1.0.0', beschreibung: 'Zwei Befehle zum Lernen: /vokabeln und /erklaer-einfach.', autor: 'ASK', style: '', script: '', befehle: [{ name: 'vokabeln', beschreibung: 'Vokabeln abfragen', prompt: 'Frag mich 10 Vokabeln ab: {{text}}' }], anweisungen: 'Korrigiere freundlich.', nachfragen: ['loeschen'], aktiv: true, geaendert: false, groesse: 700, dateien: 1, hash: 'def', fehler: '' },
];
let ownCatalogs = [];
const catalogsNow = () => [{ repo: 'KIYeager/ASK-Erweiterungen', eigen: false }, { repo: 'anthropics/skills', eigen: false }, ...ownCatalogs.map((repo) => ({ repo, eigen: true }))];
const modsChanged = () => { state = { ...state, modsCode: mods.some((m) => m.aktiv && m.script) }; emit('mods:changed', {}); push(); };
const extensionsApi = {
  modsList: async () => ({ mods, code: mods.some((m) => m.aktiv && m.script), folder: 'C:\\Users\\lernende\\AppData\\Roaming\\ASK\\mods' }),
  modsGet: async (id) => { const m = mods.find((x) => x.id === id); return m ? { ...m, files: [
    { path: 'mod.json', size: 300, text: JSON.stringify({ id: m.id, name: m.name, version: m.version, style: m.style, script: m.script }, null, 2) },
    ...(m.style ? [{ path: m.style, size: 900, text: '.robo-helfer-figur {\n  position: fixed;\n  right: 24px;\n  bottom: 120px;\n}' }] : []),
    ...(m.script ? [{ path: m.script, size: 5200, text: "(function (ASK) {\n  ASK.on('fertig', function () {\n    ASK.meldung('Geschafft!');\n  });\n})(ASK);" }] : []),
  ] } : null; },
  // ?mods=demo: Beispiel-Mods laden (die Dateien liefert der Test unter http://askmod.localhost/mod/…)
  modsActive: async () => (params.get('modtest') ? [{ id: params.get('modtest'), name: params.get('modtest'), hash: params.get('hash') || '',
    style: params.get('style') || '', script: params.get('script') || '', befehle: (() => { try { return JSON.parse(params.get('befehle') || '[]') || []; } catch { return []; } })() }]
    : params.get('mods') === 'demo' ? [
    { id: 'neon', name: 'Neon', hash: 'h', style: 'style.css', script: '', befehle: [] },
    { id: 'robo-helfer', name: 'Robo-Helfer', hash: 'h', style: 'style.css', script: 'mod.js', befehle: [] },
    { id: 'pomodoro', name: 'Pomodoro-Timer', hash: 'h', style: 'style.css', script: 'mod.js', befehle: [] },
    { id: 'vokabeln', name: 'Vokabeln', hash: 'h', style: '', script: '', befehle: [{ name: 'vokabeln', beschreibung: 'Vokabeln abfragen', prompt: 'Frag mich ab: {{text}}' }] },
  ] : []),
  modsEnable: async (id, on) => { mods = mods.map((m) => (m.id === id ? { ...m, aktiv: !!on } : m)); modsChanged(); return { ok: true }; },
  modsOffAll: async () => { mods = mods.map((m) => ({ ...m, aktiv: false })); modsChanged(); return { ok: true }; },
  modsRemove: async (id) => { if (!window.confirm('Mod löschen?')) return false; mods = mods.filter((m) => m.id !== id); modsChanged(); return true; },
  modsImport: async () => ({ ok: false, reason: 'Importieren geht nur in der App.' }),
  modsFolder: async () => {},
  modsGuide: async (action) => (action === 'kopieren' ? { ok: true, copied: true } : action === 'speichern' ? { ok: true, path: 'C:\\Users\\lernende\\Downloads\\SKILL.md' } : { ok: true, text: '---\nname: ask-mods\n---' }),
  onModsChanged: on('mods:changed'),
  catalogSources: async (list) => { if (Array.isArray(list)) ownCatalogs = list; state = { ...state, catalogs: catalogsNow() }; push(); return { ok: true, catalogs: catalogsNow() }; },
  catalogList: async (repo) => { await wait(500); return /ASK-Erweiterungen/.test(repo) ? { ok: false, reason: 'Das Repository gibt es nicht oder es ist nicht öffentlich.' } : { ok: true, entries: [
    { kind: 'skill', path: 'skills/frontend-design', name: 'frontend-design', description: 'Create distinctive, production-grade frontend interfaces with high design quality.', installiert: false },
    { kind: 'skill', path: 'skills/mcp-builder', name: 'mcp-builder', description: 'Guide for creating high-quality MCP servers.', installiert: false },
    { kind: 'skill', path: 'skills/webapp-testing', name: 'webapp-testing', description: 'Toolkit for interacting with and testing local web applications using Playwright.', installiert: true },
    { kind: 'mod', path: 'mods/neon', name: 'Neon', description: 'Ein dunkles Neon-Design mit leuchtenden Akzenten.', installiert: false },
    { kind: 'mod', path: 'mods/robo-helfer', name: 'Robo-Helfer', description: 'Ein kleiner Roboter, der auf ASK reagiert.', installiert: false },
  ] }; },
  catalogInstall: async () => { await wait(400); return { ok: true, kind: 'skill' }; },
  catalogLink: async (url) => (/github\.com|\//.test(url) ? { ok: true, kind: 'skill', skill: { slug: 'neu' } } : { ok: false, reason: 'Das ist kein GitHub-Repository. Beispiel: anthropics/skills' }),
  winMinimize: async () => {}, winMaximize: async () => {}, winClose: async () => {}, winDrag: () => {}, winState: async () => ({ maximized: false }), onWinState: on('win:state'),
};


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

function answerWait(id, value) {
  const c = [store.get(state.chat.id), ...store.values()].find((x) => x?.waits[id]);
  if (!c) return false;
  c.waits[id](value);
  return true;
}

// ASK Docs (ENTSCHEIDE E81): Docs im Speicher, wie docs.rs (?docs=leer: keine)
const mockDocs = {};
const mockComments = {};
let docSeq = 0;
const docHash = (t) => `h${t.length}-${[...t].reduce((a, c) => (a * 31 + c.charCodeAt(0)) | 0, 7)}`;
const docTitle = (t, p) => (/^# (.+)$/m.exec(t)?.[1] || p.split('/').pop().replace(/\.md$/, '')).trim();
if (!new URLSearchParams(location.search).get('docs')) {
  mockDocs['docs/lernzettel-photosynthese.md'] = { modified: Date.now() - 3600e3, content: '# Lernzettel Photosynthese\n\n## Worum es geht\n\nPflanzen bauen mit **Licht**, Wasser und CO₂ *Zucker* auf und geben Sauerstoff ab.\n\n## Formel\n\n6 CO₂ + 6 H₂O → C₆H₁₂O₆ + 6 O₂\n\n## Ablauf\n\n1. Lichtreaktion in den Thylakoiden\n   1. Wasser wird gespalten\n2. Calvin-Zyklus im Stroma\n\n| Teil | Ort |\n| --- | --- |\n| Lichtreaktion | Thylakoid |\n| Calvin-Zyklus | Stroma |\n\n> Merksatz: Ohne Licht kein Zucker.\n\nMehr dazu: [Wikipedia](https://de.wikipedia.org/wiki/Photosynthese)\n' };
  mockDocs['docs/projektplan.md'] = { modified: Date.now() - 86400e3, content: '# Projektplan\n\n## Ziel\n\n…\n' };
  mockComments['docs/lernzettel-photosynthese.md'] = [{ id: 'k1a2b3c', zitat: 'Calvin-Zyklus im Stroma', erledigt: false, zeit: Date.now() - 600e3,
    eintraege: [{ von: 'person', text: 'Kannst du hier ein Beispiel ergänzen?', zeit: Date.now() - 600e3 }, { von: 'ask', text: 'Beispiel mit Glukose ergänzt.', zeit: Date.now() - 500e3 }] }];
}
// Geteilte Docs (E82): ein Beispiel ist mit Tim und Mia geteilt
const mockShares = { 'docs/lernzettel-photosynthese.md': { geteilt: true, ordner: 'C:\\Users\\lea\\OneDrive - TF Bern\\Gruppe Bio\\lernzettel-photosynthese.askdoc', fehler: '',
  personen: [{ name: 'Tim', ich: false, zuletzt: Date.now() - 90e3 }, { name: 'Lea Muster', ich: true, zuletzt: Date.now() - 300e3 }, { name: 'Mia', ich: false, zuletzt: Date.now() - 3 * 3600e3 }],
  anderen: [{ name: 'Tim', zeit: Date.now() - 90e3, text: 'Teil Ort Lichtreaktion Thylakoid' }] } };
const mockMs = { name: new URLSearchParams(location.search).get('ms') === 'neu' ? '' : 'Lea Muster' };
const docMsg = (p, t) => `[Kommentar im Doc ${p}, ${t.id}] ${t.zitat ? `zur Stelle «${t.zitat}»` : 'zum ganzen Doc'}: ${[...t.eintraege].reverse().find((e) => e.von === 'person')?.text || ''}`;
const mockDocsApi = {
  docsList: async () => ({ ok: true, docs: Object.entries(mockDocs).map(([path, d]) => ({ path, title: docTitle(d.content, path), modified: d.modified, size: d.content.length,
    comments: (mockComments[path] || []).filter((c) => !c.erledigt).length, geteilt: !!mockShares[path] })).sort((a, b) => b.modified - a.modified) }),
  docsRead: async (path) => { const d = mockDocs[path]; return d ? { ok: true, path, title: docTitle(d.content, path), content: d.content, hash: docHash(d.content), modified: d.modified, comments: structuredClone(mockComments[path] || []), share: mockShares[path] || { geteilt: false } } : { ok: false, reason: `Das Doc «${path}» gibt es nicht.` }; },
  docsSave: async (path, content, base) => {
    const d = mockDocs[path];
    if (d && base && docHash(d.content) !== base && d.content !== content) return { ok: false, conflict: true, content: d.content, hash: docHash(d.content) };
    mockDocs[path] = { content, modified: Date.now() };
    return { ok: true, path, hash: docHash(content), title: docTitle(content, path) };
  },
  docsCreate: async (title) => { const path = `docs/${title.toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'doc'}${docSeq++ ? `-${docSeq}` : ''}.md`; mockDocs[path] = { content: `# ${title}\n\n`, modified: Date.now() }; return { ok: true, path }; },
  docsDelete: async (path) => { delete mockDocs[path]; delete mockComments[path]; return { ok: true }; },
  docsOpen: async () => null,
  docsShare: async (path) => { mockShares[path] = { geteilt: true, ordner: 'C:\\Users\\lea\\OneDrive - TF Bern\\Geteilt\\doc.askdoc', fehler: '', personen: [{ name: 'Lea Muster', ich: true, zuletzt: Date.now() }], anderen: [] }; return { ok: true, ...mockShares[path] }; },
  docsJoin: async () => ({ ok: false, canceled: true }),
  docsUnshare: async (path) => { delete mockShares[path]; return { ok: true }; },
  docsShareInfo: async (path) => mockShares[path] || { geteilt: false },
  docsShowFolder: async () => null,
  // Teilen per Link (E83): ?ms=aus ohne App-Registrierung, ?ms=neu nicht angemeldet
  docsMsStatus: async () => ({ eingerichtet: params.get('ms') !== 'aus', angemeldet: !!mockMs.name, name: mockMs.name }),
  docsMsLogin: async () => { await wait(500); mockMs.name = 'Lea Muster'; return { ok: true, name: mockMs.name }; },
  docsMsLogout: async () => { mockMs.name = ''; return { ok: true }; },
  docsShareLink: async (path) => { const link = 'https://tfbern-my.sharepoint.com/:f:/g/personal/lea_muster/EaBcD123'; mockShares[path] = { geteilt: true, link, ordner: 'OneDrive (Link)', fehler: '', personen: [{ name: 'Lea Muster', ich: true, zuletzt: Date.now() }], anderen: [] }; return { ok: true, ...mockShares[path] }; },
  docsJoinLink: async (link) => (/^https:\/\//.test(link) ? { ok: true, path: Object.keys(mockDocs)[0] } : { ok: false, reason: 'Das ist kein Link.' }),
  docsComments: async (path) => ({ ok: true, comments: structuredClone(mockComments[path] || []) }),
  docsCommentAdd: async (path, zitat, text, farbe = '') => { const t = { id: `k${Math.random().toString(16).slice(2, 8)}`, zitat, farbe, erledigt: false, zeit: Date.now(), eintraege: text ? [{ von: 'person', text, zeit: Date.now() }] : [] }; (mockComments[path] ||= []).push(t); return { ok: true, thread: structuredClone(t), message: docMsg(path, t) }; },
  docsCommentReply: async (path, id, text) => { const t = (mockComments[path] || []).find((x) => x.id === id); t.eintraege.push({ von: 'person', text, zeit: Date.now() }); return { ok: true, thread: { ...t }, message: docMsg(path, t) }; },
  docsCommentDone: async (path, id, done) => { const t = (mockComments[path] || []).find((x) => x.id === id); t.erledigt = !!done; return { ok: true, thread: { ...t } }; },
  docsCommentDelete: async (path, id) => { mockComments[path] = (mockComments[path] || []).filter((x) => x.id !== id); return { ok: true }; },
  docsExport: async (path, format) => ({ ok: true, path: `C:\\Users\\lea\\Downloads\\${path.split('/').pop().replace(/\.md$/, `.${format}`)}` }),
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
    getState: async () => { chatOf(state.chat.id); return full(); },
    onState: on('state'), onFiles: on('files'), onTasks: on('tasks'), onDocsSync: on('docs:sync'),
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
    projectUpdate: async (folder, patch) => {
      const p = projects.find((x) => x.folder === folder); if (p && patch.name) p.name = patch.name;
      if (p && typeof patch.nurQuellen === 'boolean') { p.nurQuellen = patch.nurQuellen; state = { ...state, project: { folder, name: p.name, color: p.color, nurQuellen: p.nurQuellen } }; push(); }
      return true;
    },
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
    whatsNewSeen: async () => { state = { ...state, whatsNew: null }; push(); },
    usageGet: async (withAsk = true) => (withAsk ? (await wait(600), { ...mockUsage(), ask: params.get('ohneask') ? null : { webChats: 42, webChats30: 12 } }) : { ...mockUsage(), ask: null }),
    chatRewind: async () => ({ ok: true, text: 'Räum den Ordner auf', attachments: [] }),
    chatRename: async (_f, id, title) => { const c = projects.flatMap((p) => p.chats).find((x) => x.id === id); if (c) c.title = title; return true; },
    chatPin: async (_f, id, on) => { const c = projects.flatMap((p) => p.chats).find((x) => x.id === id); if (c) c.pinned = on; return true; },
    chatSearch: async (q) => (/proxmox|budget/i.test(q) ? [{ folder: FOLDER, name: 'Haushaltsbudget', id: 'c2', title: 'Budgetplan 2027', updated: Date.now(), snippets: ['Person: Budget für 2027 aufstellen …'] }] : []),
    ...mockDocsApi,
    listDir: async (rel) => ({ ok: true, entries: state.free ? [] : tree[rel] || [] }),
    findFiles: async (q) => Object.values(tree).flat().filter((e) => e.path.toLowerCase().includes(String(q).toLowerCase())),
    importFiles: async () => ({ ok: true, paths: [] }),
    restartTerminal: async () => {}, terminalLogs: async () => 'open-terminal läuft auf 127.0.0.1', acceptWarning: async () => {},
    setAskUrl: async () => ({ ok: true }), connectAsk: async () => state.ask, logout: async () => {}, models: async () => state.models,
    askLoginWindow: async () => true, askOpenBrowser: async () => {},
    askUseToken: async (t) => (String(t).trim() ? { ok: true } : { ok: false, reason: 'Das sieht nicht wie ein Token aus. Kopiere es ganz, ohne Leerzeichen.' }),
    setModel: async (id) => { state = { ...state, model: id }; push(); }, setWebSearch: async (v) => { state = { ...state, webSearch: v }; push(); },
    setChatMemory: async (v) => { state = { ...state, chatMemory: v }; push(); },
    setInteractiveUi: async (v) => { state = { ...state, interactiveUi: v }; push(); },
    // Im Browser ohne App gibt es kein Protokoll askui: Vorspann und HTML als blob-Adresse (nur für die Entwicklung)
    uiPut: async (html) => URL.createObjectURL(new Blob([`${uiPrelude}\n${html}`], { type: 'text/html' })), setThinking: async (v) => { state = { ...state, thinking: v }; push(); },
    // Nebenfrage mit /btw (E86): Antwort Wort für Wort neben dem Auftrag, danach im Chat gespeichert
    chatBtw: async (question) => {
      const c = chatOf(state.chat.id);
      const id = `btw-${Date.now()}`;
      const time = Date.now();
      const ev = (e) => emit('chat:event', { ...e, chatId: c.id, folder: c.folder });
      ev({ type: 'btw-start', id, question, time });
      const answer = `Kurz nebenbei: Ein **Abstract** ist eine Zusammenfassung von wenigen Sätzen am Anfang eines Berichts. Der Auftrag läuft weiter.`;
      (async () => {
        for (const word of answer.split(/(?<= )/)) { await wait(60 * SPEED); ev({ type: 'btw-delta', id, text: word }); }
        c.items.push({ role: 'btw', id, question, answer, error: '', time });
        ev({ type: 'btw-end', id, question, time, text: answer, error: '' });
      })();
      return { ok: true, id };
    },
    chatSend: async (text) => { const c = chatOf(state.chat.id); if (c.running) return { ok: false, reason: 'Es läuft schon ein Auftrag.' }; demoRun(c, text); return { ok: true }; },
    // Rückgängig und Wiederherstellen wie in der App: Ereignis an den sichtbaren Chat
    chatUndo: async (snap) => { emit('chat:event', { type: 'undo', snapshot: snap, text: 'Rückgängig gemacht: 3 Dateien auf den Stand vor dem Auftrag gesetzt.', redo: `v-${snap}`, chatId: state.chat.id, folder: state.folder }); return { ok: true }; },
    chatRedo: async (id) => { emit('chat:event', { type: 'redo', redo: id, snapshots: [id.slice(2)], later: [], text: 'Wiederhergestellt: 3 Dateien wieder auf dem Stand vor dem Rückgängig.', chatId: state.chat.id, folder: state.folder }); return { ok: true }; }, chatStop: async () => stopChat(state.chat.id), chatStopChat: async (_f, id) => { stopChat(id); return true; },
    // wie im Hauptprozess: zuerst der sichtbare Chat, sonst der Chat, der diese Frage gestellt hat
    chatMove: async (from, id, to) => {
      const c = chatOf(id, from);
      if (c.running) return { ok: false, reason: 'Der Chat arbeitet noch. Zuerst stoppen, dann verschieben.' };
      const list = listed(from);
      const at = list.findIndex((x) => x.id === id);
      if (at < 0) return { ok: false, reason: 'Diesen Chat gibt es nicht mehr.' };
      listed(to).unshift(...list.splice(at, 1));
      c.folder = to;
      emit('groups');
      if (state.chat.id === id) show(id, to);
      return { ok: true };
    },
    chatExport: async (_f, id) => ({ ok: true, path: `C:\\Users\\lernende\\Documents\\${titleOf(id) || 'Chat'}.md` }),
    chatCompact: async () => {
      const c = chatOf(state.chat.id);
      if (c.items.filter((x) => x.role === 'user').length < 2) return { ok: false, reason: 'Der Chat ist noch kurz: Es gibt nichts zusammenzufassen.' };
      await wait(600);
      emit('chat:event', { type: 'notice', text: 'Der Chat ist lang geworden: Ältere Aufträge gehen ab hier als Zusammenfassung an das Modell. Du siehst weiterhin den ganzen Verlauf.', chatId: c.id, folder: c.folder });
      return { ok: true };
    },
    chatGoal: async () => {}, chatApprove: async (id, a) => answerWait(id, a), chatAnswer: async (id, t) => answerWait(id, t),
    ...ghMock,
    chatNew: async (folder) => { show(`neu${Date.now()}`, folder || state.folder); return { ok: true }; },
    chatGroups: async () => [{ folder: FREE, name: 'Chats', color: '', free: true, missing: false, current: state.folder === FREE, chats: [...freeChats] },
      ...projects.map((p) => ({ ...p, chats: [...p.chats], current: p.folder === state.folder }))],
    chatOpen: async (folder, id) => { show(id, entryOf(id) ? (freeChats.some((x) => x.id === id) ? FREE : projects.find((p) => p.chats.some((x) => x.id === id)).folder) : folder); return { ok: true }; },
    chatDelete: async (_f, _id, title) => { emit('ui:ask', { id: Date.now(), type: 'question', buttons: ['Löschen', 'Abbrechen'], defaultId: 1, cancelId: 1, dangerId: 0,
      message: 'Chat löschen?', detail: `«${title}» wird aus ${FOLDER} entfernt. Die Dateien im Ordner bleiben.` }); return false; },
    chatTranscript: async () => { const c = chatOf(state.chat.id); await wait(Number(params.get('verzug')) || 0); return { ...chatState(c), items: [...c.items], pending: [...c.pending] }; },
    reloadAsk: async () => {}, setSidebar: async (v) => { state = { ...state, sidebarOpen: v }; push(); }, setChatbar: async (v) => { state = { ...state, chatbarOpen: v }; push(); },
    setAskBounds: () => {}, setAskVisible: () => {}, setBrowserBounds: () => {}, setBrowserVisible: () => {},
    browserShow: async () => {}, browserGo: async () => {}, browserBack: async () => {}, browserForward: async () => {}, browserClear: async () => {}, browserReload: async () => {}, browserClose: async () => {},
    copy: async (t) => navigator.clipboard?.writeText(t).catch(() => {}), openLink: async (u) => window.open(u, '_blank'),
    files: async () => (state.free ? { top: [], recent: [] } : files), tasks: async () => null,
    preview: async (rel) => (/\.pdf$/i.test(rel) ? { kind: 'pdf', url: 'about:blank' } : /\.(csv|py|txt)$/.test(rel) ? { kind: 'text', text: rel.endsWith('.csv') ? 'Datum;Kategorie;Beschreibung;Betrag CHF\n02.08.2026;Bücher;Fachbuch Netzwerktechnik;32.75\n02.08.2026;Verpflegung;Mittagessen Mensa;15.85' : 'import pandas as pd\n\ndf = pd.read_csv("ausgaben.csv", sep=";", encoding="cp1252")\nsummen = df.groupby("Kategorie")["Betrag CHF"].sum()' } : { kind: 'office', name: rel }),
    openFile: async () => '', toPdf: async () => ({ fehler: 'Nur in der App.' }), showFile: async () => {},
    ...memoryApi,
    ...extensionsApi,
  };
}
