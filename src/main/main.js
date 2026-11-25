// ASK Desktop: eigene Chat-Oberfläche für ASK (Open WebUI der TF Bern) mit lokalem Arbeitsordner.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { app, ipcMain, dialog, shell, clipboard, protocol, net, Menu, safeStorage, nativeImage, nativeTheme, BrowserWindow, session, Notification } = require('electron');
const settings = require('./settings');
const paths = require('./paths');
const { checkFolder } = require('./folders');
const { Terminal } = require('./terminal');
const { AskSession, THINKING_LEVELS, sameOrigin } = require('./askapi');
const { Agent, NEW } = require('./agent');
const { Workspace } = require('./workspace');
const { AppWindow, BROWSER_PARTITION } = require('./window');
const { convertToPdf, officeInstalled } = require('./office');
const { cleanupOldVersions, isPortable } = require('./portable');
const { Updater } = require('./updater');
const { GitHub } = require('./github');
const { runTool } = require('./tools');
const { saveAttachments, imageUrl, nativeResize } = require('./attachments');
const { Snapshots } = require('./snapshot');
const { Visual } = require('./visual');
const { Web } = require('./web');
const { historyPrompt, search: searchChats } = require('./history');
const { Images } = require('./bild');
const { MODES: APPROVAL_MODES } = require('./approvals');
const { makeReviewer } = require('./review');
const usage = require('./usage');
const { Servers } = require('./servers');
const { request: terminalRequest } = require('./tools');
const { Knowledge } = require('./knowledge');
const { restoreLabel } = require('./lowlabel');

app.setAppUserModelId('ch.tfbern.ask');
// Nur für automatische Tests im Entwicklungsmodus: eigener Datenordner.
if (!app.isPackaged && process.env.ASK_TEST_USER_DATA) app.setPath('userData', process.env.ASK_TEST_USER_DATA);
// Nur für Tests: Windows zeichnet verdeckte Fenster sonst nicht neu, Bildschirmfotos wären veraltet.
if (!app.isPackaged && process.env.ASK_TEST_SCRIPT) app.commandLine.appendSwitch('disable-features', 'CalculateNativeWinOcclusion');
protocol.registerSchemesAsPrivileged([
  { scheme: 'askfile', privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true } },
]);

const singleInstance = app.requestSingleInstanceLock();
if (!singleInstance) {
  app.quit();
}

// Parallele Chats (ENTSCHEIDE E52): Jedes Projekt, in dem gearbeitet wird, hat sein eigenes Terminal und
// seine eigene Beobachtung des Ordners, jeder laufende Chat seinen eigenen Agenten. terminal, workspace und
// agent zeigen immer auf das sichtbare Projekt bzw. den sichtbaren Chat; was im Hintergrund läuft, bleibt in
// runtimes stehen, bis es fertig ist.
function makeTerminal() {
  const t = new Terminal();
  // Installierte App und einzelne EXE: Python vor dem ersten Terminal-Start entpacken (ENTSCHEIDE E39)
  if (app.isPackaged) {
    t.prepare = (report) => paths.pythonRuntime().ensure((percent) =>
      report(percent >= 100 ? '' : `Python wird eingerichtet (einmalig) … ${percent} %`));
  }
  return t;
}
const runtimes = new Map(); // Ordner (klein geschrieben) → { folder, terminal, workspace, servers, agents: Set }
let terminal = makeTerminal(); // ohne Projekt: ein Terminal, das nie startet
const ask = new AskSession();

// Prüfer für den Modus «Auto»: eigenes Modell aus den Einstellungen, sonst das Modell des Chats
const reviewer = makeReviewer({ ask, getModel: () => {
  const wanted = settings.get('reviewModel');
  return ask.models.some((m) => m.id === wanted) ? wanted : currentModel();
} });

let workspace = new Workspace();
const testMode = !app.isPackaged && !!process.env.ASK_TEST_SCRIPT;

// GitHub-Token verschlüsselt mit dem Windows-Konto (DPAPI) in den Einstellungen; nur hier lesbar.
const github = new GitHub({
  api: (testMode && process.env.ASK_TEST_GITHUB_API) || undefined,
  linkFile: (folder) => projectFile(folder, 'github.json', path.join('.ask', 'github.json')),
  store: {
    get() {
      const stored = settings.get('githubToken');
      if (!stored) return '';
      try { return safeStorage.decryptString(Buffer.from(stored, 'base64')); } catch { return ''; }
    },
    set(token) {
      settings.set({ githubToken: token ? safeStorage.encryptString(token).toString('base64') : '' });
    },
  },
  fetchImpl: (url, options) => net.fetch(url, options), // Netzwerk von Chromium, nutzt den Proxy von Windows
});

// Rückfrage vor Aktionen nach aussen (GitHub anlegen, hochladen). Mit options.checkbox ein Häkchen im
// Dialog; dann { ok, checked } statt true/false. In automatischen Tests: ja (Häkchen mit ASK_TEST_REMEMBER).
async function confirm(message, detail, options = {}) {
  const answer = (ok, checked) => (options.checkbox ? { ok, checked } : ok);
  if (testMode && process.env.ASK_TEST_AUTOCONFIRM === '1') return answer(true, process.env.ASK_TEST_REMEMBER === '1');
  if (!appWindow) return answer(false, false);
  const res = await dialog.showMessageBox(appWindow.win, { type: 'question', buttons: ['Ja', 'Nein'], defaultId: 0, cancelId: 1, message, detail,
    ...(options.checkbox ? { checkboxLabel: options.checkbox, checkboxChecked: false } : {}) });
  return answer(res.response === 0, res.response === 0 && !!res.checkboxChecked);
}

// Repositories, in die ohne Rückfrage hochgeladen wird (Häkchen im Dialog, zurücksetzen im GitHub-Dialog)
const githubTrust = {
  list: () => (Array.isArray(settings.get('githubTrusted')) ? settings.get('githubTrusted') : []),
  has: (repo) => githubTrust.list().some((r) => r.toLowerCase() === String(repo).toLowerCase()),
  add: (repo) => { if (!githubTrust.has(repo)) settings.set({ githubTrusted: [...githubTrust.list(), String(repo)] }); },
  remove: (repo) => settings.set({ githubTrusted: githubTrust.list().filter((r) => r.toLowerCase() !== String(repo).toLowerCase()) }),
};

// Aktionen aus dem Bereich «GitHub» für das sichtbare Projekt. Während das Modell dort arbeitet, nicht:
// Es könnte gleichzeitig abgleichen, und beide schrieben dieselbe Verbindung. Eine Aktion zur Zeit pro Ordner.
function ghRead(fn) {
  const folder = settings.get('folder');
  if (!folder) return Promise.resolve({ ok: false, message: 'Kein Projekt geöffnet.' });
  return Promise.resolve().then(() => fn(folder)).then((r) => ({ ok: true, ...r }), (err) => ({ ok: false, message: err.message }));
}
async function ghAction(fn) {
  const folder = settings.get('folder');
  if (!folder) return { ok: false, message: 'Kein Projekt geöffnet.' };
  if ([...(runtimes.get(path.resolve(folder).toLowerCase())?.agents || [])].some((a) => a.running || a.undoing)) {
    return { ok: false, message: 'ASK arbeitet gerade in diesem Projekt. Warte, bis der Auftrag fertig ist, oder stoppe ihn (Esc).' };
  }
  try {
    return { ok: true, ...(await github.exclusive(folder, () => fn(folder))) };
  } catch (err) {
    return { ok: false, message: err.message, pending: err.pending };
  }
}

// Internet für das Modell; lokale Adressen nur im Test (Mock-Server auf 127.0.0.1)
const web = new Web({ ask, fetchImpl: (url, options) => net.fetch(url, options), allowLocal: testMode });

// Updates (ENTSCHEIDE E37): fragt vor dem Herunterladen und vor dem Neustart, nie während eines Auftrags.
const updater = new Updater({
  app,
  config: process.env.ASK_NO_UPDATES === '1' ? null : settings.appConfig().updates, // aus für automatische Tests
  portable: isPortable(),
  beta: () => !!settings.get('betaUpdates'),
  busy: () => anyRunning(),
  ask: async (options) => {
    if (testMode || !appWindow) return -1;
    return (await dialog.showMessageBox(appWindow.win, options)).response;
  },
  openExternal: (url) => shell.openExternal(url),
  beforeInstall: async () => {
    quitting = true; // before-quit soll nicht ein zweites Mal aufräumen
    for (const a of allAgents()) a.stop();
    visual.closeRenderer();
    workspace.close();
    await Promise.all([terminal.stop(), ...[...runtimes.values()].map((rt) => stopRuntime(rt))]);
  },
  log: (m) => console.log(`[update] ${m}`),
});

// Bilder mit Flux über ASK (bild.js); angeboten, sobald ASK dem Konto das Werkzeug anbietet
const images = new Images({ ask, getModel: () => currentModel(), fetchImpl: (url, options) => net.fetch(url, options),
  allowLocal: testMode, resize: nativeResize(nativeImage) });

// Browser-Bereich der App für browser. Die Oberfläche öffnet ihn beim Start des Tests und meldet
// Grösse und Sichtbarkeit; ist er nach 2 s nicht sichtbar (App minimiert), testet visual.js unsichtbar.
async function browserPane() {
  if (!appWindow) return null;
  const view = appWindow.ensureBrowser();
  for (let i = 0; i < 40 && !appWindow.browserReady(); i++) await new Promise((r) => setTimeout(r, 50));
  if (!appWindow.browserReady()) return null;
  // focus tut nichts: Die Person tippt vielleicht gerade im Chat, die Eingaben des Modells erreichen die Seite auch so.
  // giveBack: nach Laden und Klicks des Modells den Tastaturfokus zurück an die Oberfläche der App (visual.js)
  const ui = appWindow.win.webContents;
  return { wc: view.webContents, mode: 'sichtbar', allow: (fileRoot) => { view.fileRoot = fileRoot; }, focus: () => {},
    giveBack: () => { if (!ui.isDestroyed()) ui.focus(); }, close: () => {} };
}

// Gedächtnis und eigene Skills, nur in userData (ENTSCHEIDE E50, E51)
const knowledge = new Knowledge({ dir: app.getPath('userData'), settings, ask, getModel: () => currentModel(),
  send: (channel, data) => appWindow?.send(channel, data), log: (m) => console.log(`[gedaechtnis] ${m}`) });

const visual = new Visual({ electron: { BrowserWindow, nativeImage }, convertToPdf, renderPage: path.join(__dirname, 'render', 'pdf.html'),
  getPane: browserPane, allowLocal: testMode });

function makeAgent(folder, rt) {
  const a = new Agent({
    github,
    githubTrust,
    visual,
    web,
    images,
    confirm,
    ask,
    terminal: rt.terminal,
    getFolder: () => folder,
    getModel: () => currentModel(),
    getSystemPrompt: (chat) => readInstruction('systemprompt.md', folder, chat) + projectNotes(folder)
      + knowledge.systemPrompt(folder) + (settings.get('chatMemory') ? historyPrompt(folder, chat) : '') + thinkingSwitch(),
    knowledge: knowledge.forAgent(),
    getChatMemory: () => !!settings.get('chatMemory'),
    getCommandHint: (chat) => readInstruction('befehle.md', folder, chat),
    getWebSearch: () => !!ask.webSearch && settings.get('webSearch'),
    getThinking: () => thinkingLevel(),
    getImageUrl: (attachment) => imageUrl(folder, attachment, nativeResize(nativeImage)),
    getVision: () => ask.models.find((m) => m.id === currentModel())?.vision,
    getSnapshots: () => snapshotsFor(folder),
    // Andere Chats, die gerade im selben Ordner arbeiten (Warnung, ENTSCHEIDE E46)
    getSiblings: () => [...rt.agents].filter((x) => x !== a && x.running).map((x) => x.chat.title || 'ohne Titel'),
    servers: rt.servers || null,
    // Modus im Eingabefeld: Auto lässt ASK jeden Befehl prüfen (review.js), Nachfragen fragt vor jedem
    getApprovalMode: () => 'alle',
    getMode: () => chatMode(),
    review: reviewer,
    fileExists: (rel) => !!rt.workspace.resolve(String(rel || '')),
    // In automatischen Tests: Rückfragen erlauben, Fragen mit der ersten Antwort beantworten
    autoApprove: testMode && process.env.ASK_TEST_AUTOCONFIRM === '1' ? async () => 'yes' : null,
    autoAnswer: testMode && process.env.ASK_TEST_AUTOCONFIRM === '1' ? async (q) => q.optionen[0] || 'ja' : null,
    // Kontextfenster: was ASK zum Modell angibt, sonst was ASK einmal in einer Fehlermeldung genannt hat
    getContextLimit: (model) => ask.models.find((m) => m.id === model)?.context || Number((settings.get('contextLimits') || {})[model]) || 0,
    onContextLimit: (model, limit) => settings.set({ contextLimits: { ...(settings.get('contextLimits') || {}), [model]: limit } }),
  });
  wireAgent(a, folder);
  rt.agents.add(a);
  return a;
}

// Laufzeit eines Projekts: Terminal und Beobachtung des Ordners (für Aufträge an die App wie PDF-Umwandlung)
function runtime(folder) {
  const key = path.resolve(folder).toLowerCase();
  let rt = runtimes.get(key);
  if (rt) return rt;
  migrateProject(folder);
  rt = { folder, terminal: makeTerminal(), workspace: new Workspace(), agents: new Set() };
  // Entwicklungsserver (ENTSCHEIDE E48) über die Terminal-API dieses Projekts; die Ports der Terminals gibt der
  // Browser nie frei
  rt.servers = new Servers({ request: (method, p, body) => terminalRequest(rt.terminal, method, p, body, 10000),
    available: () => rt.terminal.state === 'läuft', blockedPorts: terminalPorts });
  runtimes.set(key, rt);
  wireRuntime(rt);
  return rt;
}

const isActiveRt = (rt) => rt.terminal === terminal;
const activeRt = () => [...runtimes.values()].find(isActiveRt) || null;
const terminalPorts = () => [terminal.port, ...[...runtimes.values()].map((rt) => rt.terminal.port)].filter(Boolean);

// Projekt schliessen: zuerst seine Server stoppen (höchstens 3 s), dann das Terminal. Dessen Job-Objekt beendet,
// was danach noch läuft.
async function stopRuntime(rt) {
  await Promise.race([rt.servers.stopAll(), new Promise((r) => setTimeout(r, 3000))]);
  await rt.terminal.stop();
}
const allAgents = () => [...runtimes.values()].flatMap((rt) => [...rt.agents]);
const anyRunning = () => allAgents().some((a) => a.running);

// Laufende Chats für die Seitenleiste (auch die im Hintergrund)
function runningChats() {
  return allAgents().filter((a) => a.running).map((a) => ({ folder: a.getFolder(), id: a.chat.id, title: a.chat.title }));
}

// Projekte ohne laufenden Auftrag, die nicht sichtbar sind, wieder schliessen (Terminal beenden)
function releaseIdle() {
  for (const [key, rt] of runtimes) {
    if (isActiveRt(rt)) {
      // Im sichtbaren Projekt höchstens einen untätigen Agenten behalten: den sichtbaren
      for (const a of rt.agents) if (a !== agent && !a.running && !a.undoing) rt.agents.delete(a);
      continue;
    }
    if ([...rt.agents].some((a) => a.running || a.undoing)) continue;
    rt.agents.clear();
    rt.workspace.close();
    stopRuntime(rt);
    runtimes.delete(key);
  }
}

// Agent für einen Chat in einem Projekt: der, der ihn schon offen hat, sonst ein neuer
function agentFor(rt, id) {
  for (const a of rt.agents) if (a.chat.id === id) return a;
  return null;
}

// Ohne Projekt: ein Agent ohne Ordner (zeigt den leeren Chat)
let agent = makeAgent('', { terminal, workspace, agents: new Set() });

function approvalMode() {
  const mode = settings.get('approvals');
  return APPROVAL_MODES.includes(mode) && mode !== 'aus' ? mode : 'heikel';
}

// Modus im Eingabefeld (ENTSCHEIDE E52): auto (ohne Rückfragen), nachfragen, plan
const CHAT_MODES = ['auto', 'nachfragen', 'plan'];
function chatMode() {
  const mode = settings.get('mode');
  return CHAT_MODES.includes(mode) ? mode : 'auto';
}

const THEMES = { system: 'system', hell: 'light', dunkel: 'dark' };
function applyTheme() {
  nativeTheme.themeSource = THEMES[settings.get('theme')] || 'system';
}

// Datei oder Ordner der App zu einem Arbeitsordner, ausserhalb davon (paths.projectDataDir). Frühere
// Versionen legten Sicherung und GitHub-Verbindung in .ask ab: einmal übernehmen. Was von dort kommt,
// prüfen snapshot.js und github.js beim Lesen, denn das Modell konnte es ändern.
// Nur beim ersten Blick der App auf den Ordner (Marke «.uebernommen-<name>»), bevor ein Auftrag darin lief:
// Sonst könnte das Modell später eine eigene .ask/github.json anlegen und so bestimmen, wohin hochgeladen wird.
function projectFile(folder, name, legacy) {
  const target = path.join(paths.projectDataDir(folder), name);
  const old = path.join(folder, legacy);
  const marker = path.join(paths.projectDataDir(folder), `.uebernommen-${name}`);
  try {
    if (!fs.existsSync(marker)) {
      if (!fs.existsSync(target) && fs.existsSync(old)) {
        fs.mkdirSync(path.dirname(target), { recursive: true });
        fs.cpSync(old, target, { recursive: true });
      }
      fs.mkdirSync(path.dirname(marker), { recursive: true });
      fs.writeFileSync(marker, '');
    }
    if (fs.existsSync(old)) fs.rmSync(old, { recursive: true, force: true });
  } catch {
    // nicht übernommen: Die App beginnt eine neue Ablage
  }
  return target;
}

// Alte Ablagen eines Ordners übernehmen, sobald die App ihn öffnet (vor dem ersten Auftrag)
function migrateProject(folder) {
  projectFile(folder, 'github.json', path.join('.ask', 'github.json'));
  projectFile(folder, 'sicherung', path.join('.ask', 'sicherung'));
}

// Sicherung des Arbeitsordners (snapshot.js), eine Instanz pro Ordner
const snapshotsByFolder = new Map(); // eine Instanz pro Ordner (parallele Chats in mehreren Projekten)
function snapshotsFor(folder) {
  if (!folder) return null;
  const key = path.resolve(folder).toLowerCase();
  if (!snapshotsByFolder.has(key)) {
    snapshotsByFolder.set(key, new Snapshots(folder, projectFile(folder, 'sicherung', path.join('.ask', 'sicherung'))));
  }
  return snapshotsByFolder.get(key);
}

// Denkaufwand aus den Einstellungen; ältere Werte des früheren Schalters werden übernommen.
function thinkingLevel() {
  const value = settings.get('thinking');
  const old = { normal: 'standard', gruendlich: 'hoch', aus: 'tief' }[value];
  return old || (THINKING_LEVELS.includes(value) ? value : 'standard');
}

// Qwen-Schalter für hohen Denkaufwand am Ende des Systemprompts (siehe THINKING_PARAMS in askapi.js)
function thinkingSwitch() {
  return ['hoch', 'maximal'].includes(thinkingLevel()) ? '\n\n/think' : '';
}
let appWindow = null;
let quitting = false;

const TEXT_EXT = new Set(['.txt', '.md', '.csv', '.json', '.py', '.js', '.html', '.css', '.xml', '.log', '.yml',
  '.yaml', '.ini', '.toml', '.bat', '.ps1', '.sql', '.tsv', '.java', '.c', '.cpp', '.h', '.ts']);
const IMAGE_EXT = new Set(['.png', '.jpg', '.jpeg', '.gif', '.webp', '.svg', '.bmp']);
const OFFICE_EXT = new Set(['.docx', '.doc', '.xlsx', '.xls', '.pptx', '.ppt', '.odt', '.ods', '.odp']);
// Dateiarten, die ein Doppelklick ohne Rückfrage im zugehörigen Programm öffnet: Dokumente, Bilder, Medien,
// Text. Alles andere (exe, bat, cmd, lnk, js, vbs, hta, ps1, py, msi, reg, Makro-Dokumente, Unbekanntes) fragt nach.
const SAFE_OPEN_EXT = new Set(['.txt', '.md', '.csv', '.tsv', '.json', '.xml', '.yml', '.yaml', '.ini', '.toml', '.log', '.css', '.sql', '.html', '.htm', '.svg',
  '.pdf', '.docx', '.xlsx', '.pptx', '.odt', '.ods', '.odp', '.rtf',
  '.png', '.jpg', '.jpeg', '.gif', '.webp', '.bmp', '.ico', '.tif', '.tiff',
  '.mp3', '.wav', '.ogg', '.m4a', '.mp4', '.webm', '.mov', '.zip']);

function readStart(file, bytes) {
  const fd = fs.openSync(file, 'r');
  try {
    const buf = Buffer.alloc(bytes);
    const n = fs.readSync(fd, buf, 0, bytes, 0);
    return buf.subarray(0, n);
  } finally {
    fs.closeSync(fd);
  }
}

function readInstruction(name, folder, chat = 'allgemein') {
  try {
    const text = fs.readFileSync(path.join(paths.instructionsDir(), name), 'utf8');
    // Ersetzen mit Funktion: Ein «$» im Ordnernamen ist sonst ein Ersetzungsmuster
    return text.replaceAll('{ordner}', () => folder).replaceAll('{ordnername}', () => path.basename(folder)).replaceAll('{chat}', () => chat);
  } catch {
    return '';
  }
}

// Projektnotizen (.ask/projekt.md): was jeder Chat in diesem Ordner wissen soll, wie CLAUDE.md in
// Claude Code. Das Modell pflegt sie selbst, die Person kann sie im Editor ändern.
const NOTES = path.join('.ask', 'projekt.md');
const NOTES_MAX = 6000;

function projectNotes(folder) {
  let text = '';
  try { text = fs.readFileSync(path.join(folder, NOTES), 'utf8').trim(); } catch { return ''; }
  if (!text) return '';
  if (text.length > NOTES_MAX) text = `${text.slice(0, NOTES_MAX)}\n… (gekürzt, halte die Notizen kürzer)`;
  return `\n\n# Projektnotizen (.ask/projekt.md)\n${text}`;
}

function openNotes(folder) {
  const file = path.join(folder, NOTES);
  if (!fs.existsSync(file)) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, '# Projektnotizen\n\nWas das Modell in jedem Chat in diesem Ordner wissen soll (Aufbau, wichtige Dateien, Vorlagen, Wünsche).\n');
  }
  shell.openPath(file);
}

function state() {
  const s = settings.load();
  return {
    askUrl: s.askUrl,
    folder: s.folder,
    recent: s.recent,
    warningAccepted: s.warningAccepted,
    sidebarOpen: s.sidebarOpen,
    chatbarOpen: s.chatbarOpen,
    terminal: terminal.snapshot(),
    ask: ask.status,
    models: ask.models,
    model: currentModel(),
    webSearch: !!s.webSearch,
    chatMemory: !!s.chatMemory,
    memory: knowledge.memoryOn(),
    thinking: thinkingLevel(),
    webSearchAvailable: !!ask.webSearch,
    chat: agent.state(),
    running: runningChats(),
    servers: activeRt()?.servers.list() || [],
    update: updater.snapshot(),
    prefs: prefs(),
    version: app.getVersion(),
    approvals: approvalMode(),
    mode: chatMode(),
    reviewModel: s.reviewModel || '',
    theme: THEMES[s.theme] ? s.theme : 'system',
    project: settings.findProject(s.folder),
  };
}

// Einstellungen der Oberfläche, die sich die App merkt (Dialog «Einstellungen»)
// tourDone: Einführung gesehen (ENTSCHEIDE E49); notify: Benachrichtigungen (E52)
const PREFS = { verbose: false, todoCollapsed: true, notify: true, tourDone: false }; // wie bisher: die Aufgabenliste schrumpft nach dem Auftrag auf eine Zeile
function prefs() {
  const stored = settings.get('prefs') || {};
  return Object.fromEntries(Object.entries(PREFS).map(([k, v]) => [k, typeof stored[k] === typeof v ? stored[k] : v]));
}

function currentModel() {
  const wanted = settings.get('model');
  if (ask.models.some((m) => m.id === wanted)) return wanted;
  return ask.models[0]?.id || wanted || '';
}

function pushState() {
  appWindow?.send('state', state());
}

const sameFolder = (a, b) => !!a && !!b && path.resolve(a).toLowerCase() === path.resolve(b).toLowerCase();

// Linke Leiste: Projekte (ein Projekt = ein Arbeitsordner) mit ihren Chats. Ein Ordner, den es nicht
// mehr gibt, bleibt sichtbar (missing), damit man ihn entfernen kann.
function chatGroups() {
  const { folder, projects } = settings.load();
  return projects.map((p) => {
    const exists = fs.existsSync(p.folder);
    return { folder: p.folder, name: p.name, color: p.color, current: sameFolder(p.folder, folder), missing: !exists,
      chats: exists ? agent.list(p.folder) : [] };
  });
}

// Chat (id oder NEW) in einem Ordner öffnen; bei anderem Ordner zuerst wechseln.
let pendingChat = null;

// Sichtbaren Chat wechseln: einen laufenden Agenten zeigen oder einen untätigen im Projekt weiterverwenden
function showAgent(a) {
  if (a === agent) return;
  agent = a;
  releaseIdle();
  appWindow?.send('chat:changed', agent.state());
  pushState();
}

function openInRuntime(rt, id) {
  const existing = id === NEW ? null : agentFor(rt, id);
  if (existing) { showAgent(existing); return true; }
  // Läuft der sichtbare Agent, bleibt er im Hintergrund; der Chat bekommt einen eigenen
  const a = !agent.running && !agent.undoing && rt.agents.has(agent) ? agent : [...rt.agents].find((x) => !x.running && !x.undoing) || makeAgent(rt.folder, rt);
  const ok = id === NEW ? a.newChat() : a.open(id);
  if (ok) showAgent(a);
  return ok;
}

async function openChat(folder, id) {
  if (sameFolder(folder, settings.get('folder')) && terminal.state === 'läuft') {
    return { ok: openInRuntime(runtime(folder), id) };
  }
  pendingChat = id;
  const res = await openFolder(folder);
  if (!res.ok) pendingChat = null;
  return res;
}

// Projekt sichtbar machen. Laufende Aufträge in anderen Projekten arbeiten im Hintergrund weiter, jeder mit
// seinem eigenen Terminal und Ordner (ENTSCHEIDE E52).
const BUSY = 'Das Modell arbeitet noch in diesem Projekt. Zuerst stoppen (Esc).';

async function openFolder(folder, name = '') {
  const result = checkFolder(folder);
  if (!result.ok) return result;
  const rt = runtime(result.path);
  try {
    if (rt.workspace.root !== result.path) rt.workspace.open(result.path);
  } catch (err) {
    if (!isActiveRt(rt) && !rt.agents.size) runtimes.delete(path.resolve(result.path).toLowerCase());
    return { ok: false, reason: `Der Ordner kann nicht vorbereitet werden: ${err.message}` };
  }
  settings.addRecent(result.path, name);
  terminal = rt.terminal;
  workspace = rt.workspace;
  appWindow?.send('files', workspace.files());
  if (rt.terminal.state === 'läuft') {
    // schon offen (z. B. weil dort ein Auftrag im Hintergrund läuft): gewünschten oder letzten Chat zeigen
    const want = pendingChat;
    pendingChat = null;
    const running = [...rt.agents].find((a) => a.running);
    if (want) openInRuntime(rt, want);
    else if (running) showAgent(running);
    else openLatestIn(rt);
  } else {
    if (!rt.agents.size) makeAgent(rt.folder, rt);
    startTerminal();
  }
  releaseIdle();
  pushState();
  // Alte Sicherungen aufräumen, mit derselben Instanz, die auch sichert, und nicht beim Öffnen bremsen
  setTimeout(() => {
    try { if (sameFolder(settings.get('folder'), result.path)) snapshotsFor(result.path).prune(); } catch { /* beim nächsten Mal */ }
  }, 5000);
  return { ok: true, path: result.path };
}

// Neues Projekt: Ordner unter ASK-Projekte mit dem Namen des Projekts
function projectBase() {
  return path.join(process.env.USERPROFILE || app.getPath('home'), 'ASK-Projekte');
}

function safeFolderName(name) {
  const clean = String(name || '').replace(/[<>:"/\\|?*\x00-\x1f]/g, ' ').replace(/[. ]+$/, '').replace(/\s+/g, ' ').trim().slice(0, 60);
  return /^(con|prn|aux|nul|com\d|lpt\d)$/i.test(clean) ? `${clean}-Projekt` : clean;
}

async function newProject(name) {
  const base = projectBase();
  const title = String(name || '').trim() || `Projekt ${new Date().toISOString().slice(0, 10)}`;
  const folderName = safeFolderName(title) || 'Projekt';
  let target = path.join(base, folderName);
  for (let i = 2; fs.existsSync(target); i++) target = path.join(base, `${folderName} (${i})`);
  try {
    fs.mkdirSync(target, { recursive: true });
  } catch (err) {
    return { ok: false, reason: `Der Ordner kann nicht angelegt werden: ${err.message}` };
  }
  return openFolder(target, title);
}

// Letzten Chat eines Projekts zeigen (beim Öffnen des Projekts)
function openLatestIn(rt, wanted = null) {
  const a = [...rt.agents].find((x) => !x.running && !x.undoing) || makeAgent(rt.folder, rt);
  a.openLatest(wanted);
  showAgent(a);
  if (a === agent) { appWindow?.send('chat:changed', agent.state()); pushState(); }
}

function startTerminal() {
  const { folder, askUrl, port } = settings.load();
  if (!folder || !askUrl) return;
  terminal.start({
    folder,
    preferredPort: port,
    origin: new URL(askUrl).origin,
    systemPrompt: readInstruction('systemprompt.md', folder),
    executeDescription: readInstruction('befehle.md', folder),
  });
}

// Auftrag des Modells aus .ask/auftraege/<id>.json: Datei mit Office in PDF umwandeln.
const ordersInProgress = new Set();
async function handleOrder(file, ws = workspace) {
  const root = ws.root;
  if (!root || ordersInProgress.has(file)) return;
  ordersInProgress.add(file);
  const base = file.slice(0, -'.json'.length);
  let res;
  try {
    const order = JSON.parse(fs.readFileSync(file, 'utf8'));
    const source = order.typ === 'pdf' ? ws.resolve(String(order.quelle || '')) : null;
    if (!source) res = { fehler: 'Datei nicht gefunden oder ausserhalb des Arbeitsordners.' };
    else if (fs.statSync(source).size > 50 * 1024 * 1024) res = { fehler: 'Die Datei ist grösser als 50 MB.' };
    else res = await convertToPdf(root, source);
  } catch (err) {
    res = { fehler: `Auftrag nicht lesbar: ${err.message}` };
  }
  try {
    if (ws.root === root) {
      if (res.ok) fs.writeFileSync(`${base}.ok`, path.relative(root, res.target));
      else fs.writeFileSync(`${base}.fehler`, res.fehler);
      fs.rmSync(file, { force: true });
    }
  } finally {
    ordersInProgress.delete(file);
  }
}

// Eingabe in der Adressleiste: Adresse oder Suchbegriffe
function browserAddress(text) {
  const t = String(text || '').trim();
  if (/^https?:\/\//i.test(t)) return t;
  if (!t.includes(' ') && /^[\w-]+(\.[\w-]+)+(:\d+)?(\/.*)?$/.test(t)) return `https://${t}`;
  return `https://duckduckgo.com/?q=${encodeURIComponent(t)}`;
}

// Ordner aus der Liste nehmen. Beim aktuellen Ordner auch Terminal und Beobachtung beenden.
function forgetFolder(f) {
  const current = sameFolder(f, settings.get('folder'));
  const rt = runtimes.get(path.resolve(f).toLowerCase());
  if (rt && [...rt.agents].some((a) => a.running)) {
    appWindow?.send('folder:error', BUSY);
    return false;
  }
  settings.removeRecent(f);
  let stopped = rt ? null : Promise.resolve(); // Marke «Low» erst entfernen, wenn kein Terminal des Ordners mehr läuft
  if (current) {
    workspace.close();
    stopped = rt ? stopRuntime(rt) : terminal.stop();
    terminal = makeTerminal();
    workspace = new Workspace();
    agent = makeAgent('', { terminal, workspace, agents: new Set() });
    if (rt) runtimes.delete(path.resolve(f).toLowerCase());
  }
  if (stopped) {
    const others = settings.load().projects.map((p) => p.folder);
    Promise.resolve(stopped).then(() => restoreLabel(f, { otherFolders: others }))
      .catch((e) => console.error('Marke «Low» nicht entfernt:', e?.message || e));
  }
  pushState();
  appWindow?.send('chat:changed', agent.state());
  return true;
}

// Menü eines Projekts in der Seitenleiste (Rechtsklick oder «…»)
function projectMenu(folder) {
  const project = settings.findProject(folder);
  if (!project) return;
  const exists = fs.existsSync(folder);
  const items = [];
  if (exists) items.push({ label: 'Neuer Chat', click: () => openChat(folder, NEW).then((res) => { if (res && res.ok === false && res.reason) appWindow.send('folder:error', res.reason); }) });
  items.push({ label: 'Umbenennen …', click: () => appWindow.send('project:rename', folder) });
  items.push({ label: 'Farbe', submenu: settings.PROJECT_COLORS.map((c) => ({ label: COLOR_NAMES[c] || c, type: 'radio', checked: project.color === c,
    click: () => { settings.updateProject(folder, { color: c }); appWindow.send('chat:groups-changed'); } })) });
  items.push({ type: 'separator' });
  if (exists) {
    items.push({ label: 'Im Explorer öffnen', click: () => shell.openPath(folder) });
    items.push({ label: 'Projektnotizen bearbeiten', click: () => openNotes(folder) });
    if (sameFolder(folder, settings.get('folder'))) items.push({ label: 'GitHub …', click: () => appWindow.send('menu:github') });
    items.push({ type: 'separator' });
  }
  items.push({ label: 'Aus der Liste entfernen', click: async () => {
    if (await confirm('Projekt aus der Liste entfernen?', `${project.name}\n${folder}\n\nDer Ordner und seine Dateien bleiben erhalten. Du kannst ihn später wieder öffnen.`)) {
      forgetFolder(folder);
      appWindow.send('chat:groups-changed');
    }
  } });
  Menu.buildFromTemplate(items).popup({ window: appWindow.win });
}

const COLOR_NAMES = { '#0082B4': 'TF Blau', '#F28855': 'Orange', '#4EB68E': 'Grün', '#EE716D': 'Rot', '#60C5E2': 'Hellblau', '#A2CE8D': 'Hellgrün', '#A5ACAE': 'Grau' };

// Ordner-Menü. Ein Ordner, den es nicht mehr gibt, lässt sich nicht öffnen (das Terminal startet nicht):
// Er ist markiert, und ein Klick bietet an, ihn aus der Liste zu nehmen.
function folderMenuItems() {
  const { projects, folder } = settings.load();
  const recent = projects.map((p) => p.folder);
  const items = recent.map((f) => (fs.existsSync(f)
    ? { label: f, type: 'radio', checked: sameFolder(f, folder),
      click: () => openFolder(f).then((res) => { if (!res.ok) appWindow.send('folder:error', res.reason); }) }
    : { label: `${f} (nicht gefunden)`,
      click: async () => { if (await confirm('Ordner nicht gefunden', `${f}\n\nDiesen Eintrag aus der Liste entfernen?`)) forgetFolder(f); } }));
  if (items.length) items.push({ type: 'separator' });
  items.push({ label: 'Anderen Ordner wählen …', click: () => ipcMain.emit('folder:choose-from-menu') });
  items.push({ label: 'Neues Projekt …', click: () => appWindow.send('menu:new-folder') });
  items.push({ label: 'GitHub …', click: () => appWindow.send('menu:github') });
  if (folder || recent.length) items.push({ type: 'separator' });
  if (folder) {
    items.push({ label: 'Arbeitsordner im Explorer öffnen', click: () => shell.openPath(folder) });
    items.push({ label: 'Projektnotizen bearbeiten', click: () => openNotes(folder) });
  }
  if (recent.length) {
    items.push({ label: 'Aus der Liste entfernen', submenu: recent.map((f) => ({ label: f, click: () => forgetFolder(f) })) });
  }
  items.push({ type: 'separator' });
  items.push({ label: `Nach Updates suchen (ASK ${app.getVersion()})`, click: () => updater.check({ manual: true }) });
  items.push({ label: 'Beta-Versionen erhalten', type: 'checkbox', checked: !!settings.get('betaUpdates'), click: (item) => setBeta(item.checked) });
  return items;
}

// tell: mit /beta ausgelöst; das Menü zeigt den Zustand selbst als Häkchen.
async function setBeta(on, tell = false) {
  settings.set({ betaUpdates: !!on });
  pushState();
  if (tell && appWindow && !testMode) {
    await dialog.showMessageBox(appWindow.win, { type: 'info', buttons: ['OK'],
      message: on ? 'Beta-Versionen an' : 'Beta-Versionen aus',
      detail: on ? 'ASK bietet dir auch Vorabversionen zum Testen an. Sie können Fehler haben.'
        : 'ASK bietet dir nur noch normale Versionen an. Eine installierte Beta bleibt, bis eine neuere normale Version kommt.' });
  }
  if (on) updater.check(); // eine vorhandene Beta gleich anbieten
}

function setupIpc() {
  ipcMain.handle('update:action', () => updater.action());
  ipcMain.handle('update:later', () => updater.postpone());
  ipcMain.handle('update:beta', (_e, on, tell) => setBeta(on, !!tell));
  ipcMain.handle('file:pdf', async (_e, rel, preview) => {
    const source = workspace.resolve(rel);
    if (!source) return { fehler: 'Datei nicht gefunden.' };
    const res = await convertToPdf(workspace.root, source, { fresh: !!preview });
    return res.ok ? { ok: true, path: path.relative(workspace.root, res.target) } : res;
  });
  ipcMain.handle('state:get', () => state());
  ipcMain.handle('folder:choose', async () => {
    const suggested = path.join(process.env.USERPROFILE || app.getPath('home'), 'ASK-Projekte');
    const res = await dialog.showOpenDialog(appWindow.win, {
      title: 'Arbeitsordner wählen',
      defaultPath: fs.existsSync(suggested) ? suggested : process.env.USERPROFILE,
      properties: ['openDirectory', 'createDirectory'],
    });
    if (res.canceled || !res.filePaths[0]) return { ok: false, canceled: true };
    return openFolder(res.filePaths[0]);
  });
  ipcMain.handle('folder:new', (_e, name) => newProject(name));
  ipcMain.handle('folder:open', (_e, folder) => openFolder(folder));
  ipcMain.handle('folder:forget', (_e, folder) => forgetFolder(String(folder || '')));
  ipcMain.handle('project:menu', (_e, folder) => projectMenu(String(folder || '')));
  ipcMain.handle('project:update', (_e, folder, patch) => {
    const ok = settings.updateProject(String(folder || ''), patch || {});
    pushState();
    return ok;
  });
  ipcMain.handle('settings:set', (_e, values) => {
    const v = values || {};
    const patch = {};
    if (APPROVAL_MODES.includes(v.approvals)) patch.approvals = v.approvals;
    if (CHAT_MODES.includes(v.mode)) patch.mode = v.mode;
    if (typeof v.reviewModel === 'string') patch.reviewModel = v.reviewModel.slice(0, 200); // '' = Modell des Chats
    if (THEMES[v.theme]) patch.theme = v.theme;
    settings.set(patch);
    if (patch.theme) applyTheme();
    pushState();
  });
  knowledge.setupIpc({ ipcMain, dialog, getWindow: () => appWindow?.win, confirm, onSettings: pushState,
    projects: () => settings.load().projects.map((p) => ({ folder: p.folder, name: p.name, color: p.color })) });
  // Zuerst der sichtbare Chat: Fragen tragen die Kennung des Werkzeugaufrufs, die bei manchen Modellen in
  // jedem Chat gleich ist (call_0); die Oberfläche beantwortet immer die Frage des sichtbaren Chats.
  ipcMain.handle('chat:approve', (_e, id, answer) => agent.answerApproval(id, answer) || allAgents().some((a) => a.answerApproval(id, answer)));
  ipcMain.handle('chat:answer', (_e, id, text) => agent.answerQuestion(id, text) || allAgents().some((a) => a.answerQuestion(id, text)));
  ipcMain.handle('chat:stop-chat', (_e, folder, id) => {
    const rt = runtimes.get(path.resolve(String(folder || '')).toLowerCase());
    const a = rt && agentFor(rt, String(id || ''));
    if (a) a.stop();
  });
  ipcMain.handle('workspace:list', (_e, rel) => workspace.list(String(rel || '')));
  ipcMain.handle('workspace:find', (_e, query) => workspace.find(String(query || '')));
  ipcMain.handle('workspace:import', (_e, rel, files) => {
    try {
      return { ok: true, paths: workspace.import(String(rel || ''), files) };
    } catch (err) {
      return { ok: false, reason: `Nicht kopiert: ${err.message}` };
    }
  });
  ipcMain.handle('folder:menu', () => {
    Menu.buildFromTemplate(folderMenuItems()).popup({ window: appWindow.win });
  });
  ipcMain.on('folder:choose-from-menu', () => appWindow.send('menu:choose-folder'));
  ipcMain.handle('folder:show', () => {
    const { folder } = settings.load();
    if (folder) shell.openPath(folder);
  });
  ipcMain.handle('terminal:restart', () => startTerminal());
  ipcMain.handle('terminal:logs', () => terminal.logs.join('\n'));
  ipcMain.handle('warning:accept', () => {
    settings.set({ warningAccepted: true });
    pushState();
  });
  ipcMain.handle('ask:set-url', (_e, url) => {
    let parsed;
    try {
      parsed = new URL(String(url).trim());
    } catch {
      return { ok: false, reason: 'Das ist keine gültige Adresse.' };
    }
    if (parsed.protocol !== 'https:') return { ok: false, reason: 'Die Adresse muss mit https:// beginnen.' };
    settings.set({ askUrl: parsed.origin + parsed.pathname.replace(/\/$/, '') });
    loadAsk();
    startTerminal();
    pushState();
    return { ok: true };
  });
  ipcMain.handle('ask:connect', () => ask.check(true));
  ipcMain.handle('ask:logout', () => ask.logout());
  ipcMain.handle('ask:models', async () => {
    try { await ask.loadModels(); } catch { /* Status zeigt den Fehler */ }
    pushState();
    return ask.models;
  });
  ipcMain.handle('thinking:set', (_e, level) => {
    settings.set({ thinking: THINKING_LEVELS.includes(level) ? level : 'standard' });
    pushState();
  });
  ipcMain.handle('memory:set', (_e, on) => {
    settings.set({ chatMemory: !!on });
    pushState();
  });
  ipcMain.handle('web:set', (_e, on) => {
    settings.set({ webSearch: !!on });
    pushState();
  });
  ipcMain.handle('model:set', (_e, id) => {
    settings.set({ model: String(id) });
    if (!agent.running) agent.resetModes(); // ein anderes Modell kann Werkzeuge und Bilder wieder
    pushState();
  });
  // Anhänge zuerst nach .ask/anhaenge/<Chat>/ speichern; scheitert das, geht nichts an das Modell.
  // kept: Anhänge einer zurückgenommenen Eingabe (chat:rewind), die schon dort liegen.
  ipcMain.handle('chat:send', (_e, text, files, kept) => {
    if (agent.running) return { ok: false, reason: 'Es läuft schon ein Auftrag.' };
    if (agent.undoing) return { ok: false, reason: 'Der Ordner wird gerade zurückgesetzt. Einen Moment.' };
    let attachments = [];
    for (const k of Array.isArray(kept) ? kept.slice(0, 10) : []) {
      const rel = String(k?.path || '');
      const folder = agent.getFolder();
      // nur Dateien, die wirklich bei den Anhängen dieses Chats liegen
      if (!folder || !new RegExp(`^\\.ask/anhaenge/${agent.chat.id}/[^/\\\\]+$`).test(rel) || /\/\.{1,2}$/.test(rel)) continue;
      try {
        const size = fs.statSync(path.join(folder, ...rel.split('/'))).size;
        attachments.push({ path: rel, name: path.basename(rel), size, image: /\.(png|jpe?g|gif|webp)$/i.test(rel) });
      } catch { /* inzwischen gelöscht */ }
    }
    const before = attachments;
    if (Array.isArray(files) && files.length) {
      const folder = agent.getFolder();
      if (!folder) return { ok: false, reason: 'Zuerst einen Arbeitsordner wählen.' };
      try {
        attachments = [...before, ...saveAttachments(folder, files, agent.chat.id)];
      } catch (err) {
        return { ok: false, reason: `Anhänge nicht gespeichert: ${err.message}` };
      }
    }
    const skill = knowledge.skillFor(text); // «/name …»: eigener Skill (ENTSCHEIDE E51)
    agent.send(text, attachments, skill ? { skill } : {});
    return { ok: true };
  });
  ipcMain.handle('chat:undo', (_e, snapshot) => agent.undo(String(snapshot || '')));
  ipcMain.handle('chat:rewind', () => agent.rewind());
  // Läuft der Chat (auch im Hintergrund), ändert sein eigener Agent ihn: sonst überschriebe dessen nächstes
  // Speichern den neuen Titel wieder
  const chatOwner = (folder, id) => {
    const rt = runtimes.get(path.resolve(String(folder || '')).toLowerCase());
    return (rt && agentFor(rt, String(id || ''))) || agent;
  };
  ipcMain.handle('chat:rename', (_e, folder, id, title) => chatOwner(folder, id).rename(String(folder || ''), String(id || ''), title));
  ipcMain.handle('chat:pin', (_e, folder, id, on) => chatOwner(folder, id).pin(String(folder || ''), String(id || ''), !!on));
  // Suche über den Inhalt aller Chats der Ordner in der Liste: [{ folder, id, title, updated, snippets }]
  ipcMain.handle('chat:search', (_e, query) => {
    const hits = [];
    for (const group of chatGroups()) {
      try {
        for (const h of searchChats(group.folder, String(query || ''), null)) hits.push({ folder: group.folder, name: group.name, ...h });
      } catch { /* leere Anfrage oder Ordner nicht lesbar */ }
    }
    return hits.sort((a, b) => b.score - a.score || b.updated - a.updated).slice(0, 30);
  });
  // Nutzungsübersicht (ENTSCHEIDE E49): lokal aus den Chats aller Projekte; withAsk=false ohne Anfrage an ASK
  ipcMain.handle('usage:get', async (_e, withAsk = true) => {
    const result = usage.aggregate(settings.load().projects);
    result.ask = withAsk === false ? null : await usage.askUsage(ask).catch(() => null);
    return result;
  });
  ipcMain.handle('prefs:set', (_e, values) => {
    const next = { ...prefs() };
    for (const [k, v] of Object.entries(values || {})) if (k in PREFS && typeof v === typeof PREFS[k]) next[k] = v;
    settings.set({ prefs: next });
    pushState();
  });
  ipcMain.handle('chat:stop', () => agent.stop());
  ipcMain.handle('chat:goal', (_e, text) => agent.setGoal(text));
  ipcMain.handle('github:status', async () => {
    try {
      const folder = settings.get('folder');
      const s = folder ? await github.status(folder) : { linked: false, connected: github.connected(), login: github.connected() ? await github.whoami() : '' };
      return { ok: true, ...s, trusted: githubTrust.list() };
    } catch (err) {
      return { ok: false, connected: github.connected(), message: err.message };
    }
  });
  ipcMain.handle('github:connect', async (_e, token) => {
    try {
      return { ok: true, login: await github.connect(token) };
    } catch (err) {
      return { ok: false, message: err.message };
    }
  });
  ipcMain.handle('github:disconnect', () => github.disconnect());
  ipcMain.handle('github:untrust', (_e, repo) => githubTrust.remove(repo));
  // Bereich «GitHub» rechts: dieselben Abläufe wie die Werkzeuge des Modells. Hochladen und Anlegen laufen
  // über die Werkzeuge selbst, damit Rückfrage und «nicht mehr fragen» genau gleich gelten.
  const ghTool = (name, args) => ghAction((folder) => runTool(null, name, args, { github, githubTrust, confirm, folder, panel: true }));
  ipcMain.handle('github:branches', () => ghRead((folder) => github.branches(folder)));
  ipcMain.handle('github:pulls', () => ghRead(async (folder) => ({ pulls: await github.pullRequests(folder) })));
  ipcMain.handle('github:pull', () => ghAction((folder) => github.pull(folder)));
  ipcMain.handle('github:clone', (_e, repo) => ghAction((folder) => github.clone(folder, String(repo || ''))));
  ipcMain.handle('github:push', (_e, message, overwrite) => ghTool('github_push', { nachricht: String(message || '').trim() || 'Änderungen aus ASK', ueberschreiben: overwrite === true }));
  ipcMain.handle('github:create', (_e, name, privat) => ghTool('github_create', { name: String(name || '').trim(), privat: privat !== false }));
  ipcMain.handle('github:switch', (_e, name) => ghAction((folder) => github.switchBranch(folder, String(name || ''))));
  ipcMain.handle('github:branch-create', (_e, name) => ghAction((folder) => github.createBranch(folder, String(name || ''))));
  ipcMain.handle('github:pr-create', (_e, title, body) => ghAction((folder) => github.createPullRequest(folder, { title, body })));
  ipcMain.handle('chat:new', (_e, folder) => openChat(folder || settings.get('folder'), NEW));
  ipcMain.handle('chat:groups', () => chatGroups());
  ipcMain.handle('chat:open', (_e, folder, id) => openChat(folder, id));
  ipcMain.handle('chat:delete', async (_e, folder, id, title) => {
    const res = await dialog.showMessageBox(appWindow.win, {
      type: 'question', buttons: ['Löschen', 'Abbrechen'], defaultId: 1, cancelId: 1,
      message: 'Chat löschen?', detail: `«${title}» wird aus ${folder} entfernt. Die Dateien im Ordner bleiben.`,
    });
    const rt = runtimes.get(path.resolve(String(folder || '')).toLowerCase());
    const busy = rt && agentFor(rt, String(id || ''));
    if (busy && busy.running) return false;
    if (res.response !== 0) return false;
    const ok = agent.remove(folder, id);
    if (busy && busy !== agent) rt.agents.delete(busy);
    if (busy === agent) agent.newChat();
    return ok;
  });
  ipcMain.handle('chat:transcript', () => ({ ...agent.state(), items: agent.transcript() }));
  ipcMain.handle('ask:reload', () => {
    const wc = appWindow.view.webContents;
    const url = settings.get('askUrl');
    if (url && !sameOrigin(wc.getURL(), url)) wc.loadURL(url).catch(() => {}); // Fehler meldet did-fail-load
    else wc.reload();
  });
  ipcMain.handle('ui:chatbar', (_e, open) => {
    settings.set({ chatbarOpen: !!open });
    pushState();
  });
  ipcMain.handle('ui:sidebar', (_e, open) => {
    settings.set({ sidebarOpen: !!open });
    pushState();
  });
  ipcMain.on('ui:ask-bounds', (_e, bounds) => appWindow.setAskBounds(bounds));
  ipcMain.on('ui:ask-visible', (_e, visible) => appWindow.setAskVisible(visible));
  ipcMain.on('ui:browser-bounds', (_e, bounds) => appWindow.setBrowserBounds(bounds));
  ipcMain.on('ui:browser-visible', (_e, visible) => appWindow.setBrowserVisible(visible));
  // Entwicklungsserver des sichtbaren Projekts (ENTSCHEIDE E48). Die Person startet ohne Rückfrage: Sie tippt den Befehl selbst.
  ipcMain.handle('servers:list', () => activeRt()?.servers.list() || []);
  ipcMain.handle('servers:start', async (_e, command, name) => {
    const rt = activeRt();
    if (!rt || rt.terminal.state !== 'läuft') return { ok: false, reason: 'Das Terminal läuft nicht.' };
    try {
      return { ok: true, server: await rt.servers.start(String(command || ''), { name: String(name || ''), by: 'person' }) };
    } catch (err) {
      return { ok: false, reason: err.message };
    }
  });
  ipcMain.handle('servers:stop', async (_e, id) => {
    try {
      return { ok: !!(await activeRt()?.servers.stop(String(id || ''))) };
    } catch (err) {
      return { ok: false, reason: err.message };
    }
  });
  ipcMain.handle('servers:remove', (_e, id) => !!activeRt()?.servers.remove(String(id || '')));
  ipcMain.handle('servers:output', (_e, id) => activeRt()?.servers.output(String(id || '')) || []);
  ipcMain.handle('browser:show', () => { appWindow.ensureBrowser(); });
  ipcMain.handle('browser:go', (_e, text) => appWindow.ensureBrowser().webContents.loadURL(browserAddress(text)).catch(() => {}));
  ipcMain.handle('browser:back', () => { if (appWindow.browser) appWindow.browser.webContents.navigationHistory.goBack(); });
  ipcMain.handle('browser:forward', () => { if (appWindow.browser) appWindow.browser.webContents.navigationHistory.goForward(); });
  ipcMain.handle('browser:reload', () => { if (appWindow.browser) appWindow.browser.webContents.reload(); });
  ipcMain.handle('browser:close', () => {
    if (!appWindow.browser) return;
    appWindow.browser.webContents.loadURL('about:blank').catch(() => {});
    appWindow.setBrowserVisible(false);
  });
  // Anmeldungen im Browser der App löschen (Cookies, Speicher); nach Rückfrage
  ipcMain.handle('browser:clear', async () => {
    if (!(await confirm('Anmeldungen im Browser löschen?', 'Du wirst im Browser der App überall abgemeldet. ASK und dein Chrome sind davon nicht betroffen.'))) return false;
    await session.fromPartition(BROWSER_PARTITION).clearStorageData();
    if (appWindow.browser) appWindow.browser.webContents.reload();
    return true;
  });
  ipcMain.handle('clipboard:write', (_e, text) => clipboard.writeText(String(text)));
  ipcMain.handle('link:open', (_e, url) => {
    if (/^https?:\/\//i.test(String(url))) shell.openExternal(String(url));
  });
  ipcMain.handle('workspace:files', () => workspace.files());
  ipcMain.handle('file:preview', async (_e, rel) => {
    const full = workspace.resolve(rel);
    if (!full) return { kind: 'missing' };
    const ext = path.extname(full).toLowerCase();
    const url = `askfile://datei/${rel.split(path.sep).map(encodeURIComponent).join('/')}`;
    const { size, mtimeMs } = fs.statSync(full);
    if (IMAGE_EXT.has(ext)) return { kind: 'image', url: `${url}?v=${Math.round(mtimeMs)}` }; // neue Fassung, nicht die zwischengespeicherte
    if (ext === '.pdf') return { kind: 'pdf', url };
    // Mit installiertem Office zeigt die Vorschau ein PDF der Datei, ohne Klick auf «Als PDF anzeigen»
    if (OFFICE_EXT.has(ext)) return { kind: 'office', name: path.basename(full), auto: await officeInstalled(ext) };
    if (TEXT_EXT.has(ext) || size < 200000) {
      const buf = readStart(full, 200000); // nur den Anfang lesen, auch bei sehr grossen Dateien
      if (!buf.includes(0)) return { kind: 'text', text: buf.toString('utf8'), truncated: size > 200000 };
    }
    return { kind: 'other', name: path.basename(full) };
  });
  // Öffnen mit dem zugehörigen Programm läuft mit den normalen Rechten der Person, nicht im geschützten
  // Terminal. Programme und Skripte (auch solche, die das Modell angelegt hat) darum nie ohne Rückfrage.
  ipcMain.handle('file:open', async (_e, rel) => {
    const full = workspace.resolve(rel);
    if (!full) return 'nicht gefunden';
    if (!SAFE_OPEN_EXT.has(path.extname(full).toLowerCase())) {
      const res = testMode && process.env.ASK_TEST_AUTOCONFIRM === '1' ? { response: 0 } : await dialog.showMessageBox(appWindow.win, {
        type: 'warning', buttons: ['Im Explorer zeigen', 'Trotzdem öffnen', 'Abbrechen'], defaultId: 0, cancelId: 2,
        message: `«${path.basename(full)}» öffnen?`,
        detail: 'Diese Datei kann ein Programm oder Skript sein. Beim Öffnen läuft sie mit deinen Rechten auf dem ganzen Laptop, '
          + 'nicht nur im Arbeitsordner. Öffne sie nur, wenn du weisst, was sie tut. Hat das Modell sie angelegt, lass sie lieber vom Modell im Chat ausführen.',
      });
      if (res.response === 0) { shell.showItemInFolder(full); return ''; }
      if (res.response !== 1) return '';
    }
    return shell.openPath(full);
  });
  ipcMain.handle('file:show', (_e, rel) => {
    const full = workspace.resolve(rel);
    if (full) shell.showItemInFolder(full);
  });
}

function setupProtocol() {
  // Vorschau in der Seitenleiste: nur Dateien aus dem Arbeitsordner.
  protocol.handle('askfile', (request) => {
    const rel = decodeURIComponent(new URL(request.url).pathname.replace(/^\//, '')).replaceAll('/', path.sep);
    const full = workspace.resolve(rel);
    if (!full) return new Response('Nicht gefunden', { status: 404 });
    return net.fetch(pathToFileURL(full).toString());
  });
}

// Die ASK-Seite dient nur noch der Anmeldung; die App liest danach das Token (askapi.js).
function loadAsk() {
  const url = settings.get('askUrl');
  if (!url || !appWindow) return;
  ask.setBase(url);
  appWindow.loadAsk(url);
}

// Terminal und Ordner eines Projekts: Zustand nur für das sichtbare Projekt an die Oberfläche
function wireRuntime(rt) {
  let started = false;
  rt.terminal.on('port', (port) => settings.set({ port }));
  rt.terminal.on('state', (snap) => {
    if (snap.state === 'läuft' && !started) {
      started = true;
      if (isActiveRt(rt)) {
        const want = pendingChat;
        pendingChat = null;
        if (want) openInRuntime(rt, want); else openLatestIn(rt);
      }
    }
    if (snap.state !== 'läuft' && snap.state !== 'startet') started = false;
    if (isActiveRt(rt) && (snap.state === 'läuft' || snap.state === 'fehler')) pendingChat = null;
    if (isActiveRt(rt)) pushState();
  });
  // Server: nur die des sichtbaren Projekts an die Oberfläche. Ein beendetes Terminal nimmt seine Prozesse mit.
  rt.terminal.on('state', (snap) => {
    if (snap.state === 'gestoppt' || snap.state === 'startet' || (snap.state === 'fehler' && !rt.terminal.backend.running())) rt.servers.terminalGone();
  });
  rt.servers.on('changed', (list) => { if (isActiveRt(rt)) appWindow?.send('servers', list); });
  rt.workspace.on('auftrag', (file) => handleOrder(file, rt.workspace));
  rt.workspace.on('files', (files) => { if (rt.workspace === workspace) appWindow?.send('files', files); });
}

// Windows-Benachrichtigung (ENTSCHEIDE E52): wenn ASK fertig ist, fragt oder um Erlaubnis bittet und
// man gerade nicht hinschaut (Fenster im Hintergrund oder anderer Chat). Klick öffnet den Chat.
const NOTIFY_MIN_MS = 8000; // kurze Aufträge ohne Meldung
const shown = new Set(); // sonst räumt der Garbage Collector die Meldung weg, bevor jemand klickt
function notify(a, folder, ev) {
  if (!['done', 'question', 'approval'].includes(ev.type) || !prefs().notify || !appWindow || !Notification.isSupported()) return;
  const watching = appWindow.win.isFocused() && a === agent;
  if (watching) return;
  if (ev.type === 'done' && (Date.now() - (a.turnStarted || 0) < NOTIFY_MIN_MS || a.questions.size || a.approvals.size)) return;
  const title = a.chat.title || 'Neuer Chat';
  const body = ev.type === 'question' ? `Frage: ${ev.frage}` : ev.type === 'approval' ? `Wartet auf dich: ${ev.title}` : 'Fertig.';
  const n = new Notification({ title: `ASK · ${title}`.slice(0, 120), body: String(body).slice(0, 240), silent: ev.type === 'done' });
  const id = a.chat.id;
  shown.add(n);
  n.on('close', () => shown.delete(n));
  n.on('click', () => {
    shown.delete(n);
    if (!appWindow) return;
    if (appWindow.win.isMinimized()) appWindow.win.restore();
    appWindow.win.show();
    appWindow.win.focus();
    if (folder && a !== agent) openChat(folder, id);
  });
  n.show();
}

// Ereignisse eines Agenten: an die Oberfläche mit Chat und Ordner, damit sie nur den sichtbaren Chat zeigt
function wireAgent(a, folder) {
  a.on('event', (ev) => {
    // Frage oder Rückfrage: Fenster in der Taskleiste aufblinken lassen, wenn es im Hintergrund ist
    if ((ev.type === 'approval' || ev.type === 'question') && appWindow && !appWindow.win.isFocused()) appWindow.win.flashFrame(true);
    if (ev.type === 'user') a.turnStarted = Date.now();
    notify(a, folder, ev);
    appWindow?.send('chat:event', { ...ev, chatId: a.chat.id, folder });
    if (ev.type === 'done') knowledge.afterTurn(a, folder, ev); // im Hintergrund merken (ENTSCHEIDE E50)
  });
  a.on('changed', () => {
    if (a === agent) appWindow?.send('chat:changed', agent.state());
    pushState();
    appWindow?.send('chat:groups-changed');
    if (anyRunning()) updater.busyChanged();
    else updater.idle();
    if (!a.running) setTimeout(releaseIdle, 0);
  });
}

function wireEvents() {
  updater.on('status', () => pushState());
  ask.on('status', (s) => {
    if (s.state === 'angemeldet') images.discover();
    else if (s.state === 'nicht-angemeldet') images.forget();
    pushState();
  });
  updater.on('changed', () => {
    pushState();
    // Fortschritt auch in der Taskleiste: Download, danach der Countdown bis zum Neustart
    const u = updater.snapshot();
    const bar = u.state === 'lädt' ? u.percent / 100 : u.state === 'neustart' || u.state === 'installiert' ? 2 : -1;
    try { appWindow?.win?.setProgressBar(bar); } catch { /* Fenster schon zu */ }
  });
}

// Verknüpfung im Startmenü wieder anlegen, wenn sie fehlt. Der Installer legt sie bei Updates und bei einer
// Installation über eine bestehende nicht neu an, und Windows bietet an, sie zu löschen, wenn man während
// eines Updates darauf klickt (ENTSCHEIDE E39).
function ensureStartMenuLink() {
  try {
    const link = path.join(app.getPath('appData'), 'Microsoft', 'Windows', 'Start Menu', 'Programs', 'ASK.lnk');
    if (fs.existsSync(link)) return;
    shell.writeShortcutLink(link, 'create', { target: process.execPath, icon: process.execPath, iconIndex: 0,
      appUserModelId: 'ch.tfbern.ask', description: 'ASK Desktop' });
  } catch {
    // kein Startmenü (z. B. umgeleitete Ordner): ASK läuft trotzdem
  }
}

app.on('second-instance', () => {
  if (appWindow?.win) {
    if (appWindow.win.isMinimized()) appWindow.win.restore();
    appWindow.win.focus();
  }
});

// Wer ASK schon benutzt hat, sieht die Einführung nach dem Update nicht von selbst (E49), nur über die Einstellungen.
function skipTourForExistingUsers() {
  const s = settings.load();
  const stored = s.prefs || {};
  if (s.warningAccepted && typeof stored.tourDone !== 'boolean') settings.set({ prefs: { ...stored, tourDone: true } });
}

app.whenReady().then(() => {
  skipTourForExistingUsers();
  applyTheme();
  setupProtocol();
  setupIpc();
  appWindow = new AppWindow();
  ask.attach(appWindow.view);
  wireEvents();
  loadAsk();
  const s = settings.load();
  if (s.folder && s.askUrl) {
    openFolder(s.folder).then((res) => {
      if (!res.ok) {
        settings.set({ folder: '' });
        pushState();
      }
    });
  }
  appWindow.win.on('closed', () => {
    appWindow = null;
    visual.closeRenderer(); // unsichtbare Fenster halten die App sonst offen
    app.quit();
  });
  appWindow.win.on('focus', () => appWindow?.win.flashFrame(false));
  if (singleInstance) {
    cleanupOldVersions();
    updater.start();
    if (app.isPackaged && !isPortable()) ensureStartMenuLink();
  }
  if (!app.isPackaged && process.env.ASK_TEST_SCRIPT) {
    // Test: terminal, agent und workspace zeigen immer auf das sichtbare Projekt (parallele Chats)
    const live = (get) => new Proxy({}, {
      get: (_t, k) => { const v = get()[k]; return typeof v === 'function' ? v.bind(get()) : v; },
      set: (_t, k, v) => { get()[k] = v; return true; },
    });
    require(process.env.ASK_TEST_SCRIPT)({ app, appWindow, terminal: live(() => terminal), ask, agent: live(() => agent), workspace: live(() => workspace),
      settings, openFolder, pushState, folderMenuItems });
  }
});

// Beim Beenden den laufenden Auftrag stoppen und abwarten, bis er gespeichert ist (höchstens 5 s)
async function waitForAgent(ms = 5000) {
  const end = Date.now() + ms;
  while (anyRunning() && Date.now() < end) await new Promise((r) => setTimeout(r, 50));
}

app.on('before-quit', (event) => {
  if (quitting) return;
  event.preventDefault();
  quitting = true;
  for (const a of allAgents()) a.stop();
  updater.stop();
  waitForAgent().finally(() => {
    visual.closeRenderer();
    workspace.close();
    for (const rt of runtimes.values()) rt.workspace.close();
    Promise.allSettled([terminal.stop(), ...[...runtimes.values()].map((rt) => stopRuntime(rt))]).finally(() => app.quit());
  });
});

app.on('window-all-closed', () => app.quit());
