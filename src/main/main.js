// ASK Desktop: eigene Chat-Oberfläche für ASK (Open WebUI der TF Bern) mit lokalem Arbeitsordner.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { app, ipcMain, dialog, shell, clipboard, protocol, net, Menu, safeStorage, nativeImage, BrowserWindow, session } = require('electron');
const settings = require('./settings');
const paths = require('./paths');
const { checkFolder } = require('./folders');
const { Terminal } = require('./terminal');
const { AskSession, THINKING_LEVELS } = require('./askapi');
const { Agent, NEW } = require('./agent');
const { Workspace } = require('./workspace');
const { AppWindow, BROWSER_PARTITION } = require('./window');
const { convertToPdf } = require('./office');
const { cleanupOldVersions } = require('./portable');
const { GitHub } = require('./github');
const { saveAttachments, imageUrl, nativeResize } = require('./attachments');
const { Snapshots } = require('./snapshot');
const { Visual } = require('./visual');
const { Web } = require('./web');
const { historyPrompt } = require('./history');

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

const terminal = new Terminal();
const ask = new AskSession();
const workspace = new Workspace();
const testMode = !app.isPackaged && !!process.env.ASK_TEST_SCRIPT;

// GitHub-Token verschlüsselt mit dem Windows-Konto (DPAPI) in den Einstellungen; nur hier lesbar.
const github = new GitHub({
  api: (testMode && process.env.ASK_TEST_GITHUB_API) || undefined,
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

// Internet für das Modell; lokale Adressen nur im Test (Mock-Server auf 127.0.0.1)
const web = new Web({ ask, fetchImpl: (url, options) => net.fetch(url, options), allowLocal: testMode });

// Browser-Bereich der App für browser. Die Oberfläche öffnet ihn beim Start des Tests und meldet
// Grösse und Sichtbarkeit; ist er nach 2 s nicht sichtbar (App minimiert), testet visual.js unsichtbar.
async function browserPane() {
  if (!appWindow) return null;
  const view = appWindow.ensureBrowser();
  for (let i = 0; i < 40 && !appWindow.browserReady(); i++) await new Promise((r) => setTimeout(r, 50));
  if (!appWindow.browserReady()) return null;
  // focus tut nichts: Die Person tippt vielleicht gerade im Chat, die Eingaben des Modells erreichen die Seite auch so
  return { wc: view.webContents, mode: 'sichtbar', allow: (fileRoot) => { view.fileRoot = fileRoot; }, focus: () => {}, close: () => {} };
}

const visual = new Visual({ electron: { BrowserWindow, nativeImage }, convertToPdf, renderPage: path.join(__dirname, 'render', 'pdf.html'),
  getPane: browserPane, allowLocal: testMode });

const agent = new Agent({
  github,
  githubTrust,
  visual,
  web,
  confirm,
  ask,
  terminal,
  getFolder: () => (terminal.state === 'läuft' ? terminal.folder : settings.get('folder')),
  getModel: () => currentModel(),
  getSystemPrompt: (chat) => readInstruction('systemprompt.md', settings.get('folder'), chat) + projectNotes(settings.get('folder'))
    + (settings.get('chatMemory') ? historyPrompt(settings.get('folder'), chat) : '') + thinkingSwitch(),
  getChatMemory: () => !!settings.get('chatMemory'),
  getCommandHint: (chat) => readInstruction('befehle.md', settings.get('folder'), chat),
  getWebSearch: () => !!ask.webSearch && settings.get('webSearch'),
  getThinking: () => thinkingLevel(),
  getImageUrl: (attachment) => imageUrl(agent.getFolder(), attachment, nativeResize(nativeImage)),
  getVision: () => ask.models.find((m) => m.id === currentModel())?.vision,
  getSnapshots: () => snapshotsFor(agent.getFolder()),
});

// Sicherung des Arbeitsordners (snapshot.js), eine Instanz pro Ordner
let snapshots = null;
function snapshotsFor(folder) {
  if (!folder) return null;
  if (!snapshots || !sameFolder(snapshots.root, folder)) snapshots = new Snapshots(folder);
  return snapshots;
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

function readInstruction(name, folder, chat = 'allgemein') {
  try {
    const text = fs.readFileSync(path.join(paths.instructionsDir(), name), 'utf8');
    return text.replaceAll('{ordner}', folder).replaceAll('{ordnername}', path.basename(folder)).replaceAll('{chat}', chat);
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
    thinking: thinkingLevel(),
    webSearchAvailable: !!ask.webSearch,
    chat: agent.state(),
  };
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

// Linke Leiste: Chats der zuletzt verwendeten Ordner, der aktuelle zuerst.
function chatGroups() {
  const { folder, recent } = settings.load();
  const folders = [folder, ...recent.filter((f) => !sameFolder(f, folder))].filter((f) => f && fs.existsSync(f));
  return folders.map((f) => ({ folder: f, name: path.basename(f), current: sameFolder(f, folder), chats: agent.list(f) }));
}

// Chat (id oder NEW) in einem Ordner öffnen; bei anderem Ordner zuerst wechseln.
let pendingChat = null;
async function openChat(folder, id) {
  if (agent.running) return { ok: false, reason: 'Das Modell arbeitet noch. Zuerst stoppen.' };
  if (sameFolder(folder, settings.get('folder')) && terminal.state === 'läuft') {
    return { ok: id === NEW ? agent.newChat() : agent.open(id) };
  }
  pendingChat = id;
  const res = await openFolder(folder);
  if (!res.ok) pendingChat = null;
  return res;
}

async function openFolder(folder) {
  const result = checkFolder(folder);
  if (!result.ok) return result;
  settings.addRecent(result.path);
  try {
    workspace.open(result.path);
  } catch (err) {
    return { ok: false, reason: `Der Ordner kann nicht vorbereitet werden: ${err.message}` };
  }
  pushState();
  startTerminal();
  return { ok: true, path: result.path };
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
async function handleOrder(file) {
  const root = workspace.root;
  if (!root || ordersInProgress.has(file)) return;
  ordersInProgress.add(file);
  const base = file.slice(0, -'.json'.length);
  let res;
  try {
    const order = JSON.parse(fs.readFileSync(file, 'utf8'));
    const source = order.typ === 'pdf' ? workspace.resolve(String(order.quelle || '')) : null;
    if (!source) res = { fehler: 'Datei nicht gefunden oder ausserhalb des Arbeitsordners.' };
    else if (fs.statSync(source).size > 50 * 1024 * 1024) res = { fehler: 'Die Datei ist grösser als 50 MB.' };
    else res = await convertToPdf(root, source);
  } catch (err) {
    res = { fehler: `Auftrag nicht lesbar: ${err.message}` };
  }
  try {
    if (workspace.root === root) {
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
  settings.removeRecent(f);
  if (current) {
    workspace.close();
    terminal.stop();
  }
  pushState();
}

// Ordner-Menü. Ein Ordner, den es nicht mehr gibt, lässt sich nicht öffnen (das Terminal startet nicht):
// Er ist markiert, und ein Klick bietet an, ihn aus der Liste zu nehmen.
function folderMenuItems() {
  const { recent, folder } = settings.load();
  const items = recent.map((f) => (fs.existsSync(f)
    ? { label: f, type: 'radio', checked: f === folder,
      click: () => openFolder(f).then((res) => { if (!res.ok) appWindow.send('folder:error', res.reason); }) }
    : { label: `${f} (nicht gefunden)`,
      click: async () => { if (await confirm('Ordner nicht gefunden', `${f}\n\nDiesen Eintrag aus der Liste entfernen?`)) forgetFolder(f); } }));
  if (items.length) items.push({ type: 'separator' });
  items.push({ label: 'Anderen Ordner wählen …', click: () => ipcMain.emit('folder:choose-from-menu') });
  items.push({ label: 'Neuen Projektordner anlegen', click: () => appWindow.send('menu:new-folder') });
  items.push({ label: 'GitHub …', click: () => appWindow.send('menu:github') });
  if (folder || recent.length) items.push({ type: 'separator' });
  if (folder) {
    items.push({ label: 'Arbeitsordner im Explorer öffnen', click: () => shell.openPath(folder) });
    items.push({ label: 'Projektnotizen bearbeiten', click: () => openNotes(folder) });
  }
  if (recent.length) {
    items.push({ label: 'Aus der Liste entfernen', submenu: recent.map((f) => ({ label: f, click: () => forgetFolder(f) })) });
  }
  return items;
}

function setupIpc() {
  ipcMain.handle('file:pdf', async (_e, rel) => {
    const source = workspace.resolve(rel);
    if (!source) return { fehler: 'Datei nicht gefunden.' };
    const res = await convertToPdf(workspace.root, source);
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
  ipcMain.handle('folder:new', async () => {
    const base = path.join(process.env.USERPROFILE || app.getPath('home'), 'ASK-Projekte');
    const date = new Date().toISOString().slice(0, 10);
    let target = path.join(base, `Projekt ${date}`);
    for (let i = 2; fs.existsSync(target); i++) target = path.join(base, `Projekt ${date} (${i})`);
    fs.mkdirSync(target, { recursive: true });
    return openFolder(target);
  });
  ipcMain.handle('folder:open', (_e, folder) => openFolder(folder));
  ipcMain.handle('folder:forget', (_e, folder) => {
    settings.removeRecent(folder);
    pushState();
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
      parsed = new URL(url.trim());
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
    pushState();
  });
  // Anhänge zuerst nach .ask/anhaenge/<Chat>/ speichern; scheitert das, geht nichts an das Modell.
  ipcMain.handle('chat:send', (_e, text, files) => {
    if (agent.running) return { ok: false, reason: 'Es läuft schon ein Auftrag.' };
    let attachments = [];
    if (Array.isArray(files) && files.length) {
      const folder = agent.getFolder();
      if (!folder) return { ok: false, reason: 'Zuerst einen Arbeitsordner wählen.' };
      try {
        attachments = saveAttachments(folder, files, agent.chat.id);
      } catch (err) {
        return { ok: false, reason: `Anhänge nicht gespeichert: ${err.message}` };
      }
    }
    agent.send(text, attachments);
    return { ok: true };
  });
  ipcMain.handle('chat:undo', (_e, snapshot) => agent.undo(String(snapshot || '')));
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
  ipcMain.handle('chat:new', (_e, folder) => openChat(folder || settings.get('folder'), NEW));
  ipcMain.handle('chat:groups', () => chatGroups());
  ipcMain.handle('chat:open', (_e, folder, id) => openChat(folder, id));
  ipcMain.handle('chat:delete', async (_e, folder, id, title) => {
    const res = await dialog.showMessageBox(appWindow.win, {
      type: 'question', buttons: ['Löschen', 'Abbrechen'], defaultId: 1, cancelId: 1,
      message: 'Chat löschen?', detail: `«${title}» wird aus ${folder} entfernt. Die Dateien im Ordner bleiben.`,
    });
    return res.response === 0 && agent.remove(folder, id);
  });
  ipcMain.handle('chat:transcript', () => ({ ...agent.state(), items: agent.transcript() }));
  ipcMain.handle('ask:reload', () => {
    const wc = appWindow.view.webContents;
    const url = settings.get('askUrl');
    if (url && !wc.getURL().startsWith(new URL(url).origin)) wc.loadURL(url);
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
  ipcMain.handle('workspace:tasks', () => workspace.tasks());
  ipcMain.handle('file:preview', (_e, rel) => {
    const full = workspace.resolve(rel);
    if (!full) return { kind: 'missing' };
    const ext = path.extname(full).toLowerCase();
    const url = `askfile://datei/${rel.split(path.sep).map(encodeURIComponent).join('/')}`;
    const size = fs.statSync(full).size;
    if (IMAGE_EXT.has(ext)) return { kind: 'image', url };
    if (ext === '.pdf') return { kind: 'pdf', url };
    if (OFFICE_EXT.has(ext)) return { kind: 'office', name: path.basename(full) };
    if (TEXT_EXT.has(ext) || size < 200000) {
      const buf = fs.readFileSync(full).subarray(0, 200000);
      if (!buf.includes(0)) return { kind: 'text', text: buf.toString('utf8'), truncated: size > 200000 };
    }
    return { kind: 'other', name: path.basename(full) };
  });
  ipcMain.handle('file:open', (_e, rel) => {
    const full = workspace.resolve(rel);
    if (full) return shell.openPath(full);
    return 'nicht gefunden';
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

let chatFolder = null;

function wireEvents() {
  terminal.on('port', (port) => settings.set({ port }));
  terminal.on('state', (snap) => {
    if (snap.state === 'läuft' && snap.folder !== chatFolder) {
      chatFolder = snap.folder;
      agent.openLatest(pendingChat);
      pendingChat = null;
    }
    pushState();
  });
  ask.on('status', () => pushState());
  agent.on('event', (ev) => appWindow?.send('chat:event', ev));
  agent.on('changed', () => {
    appWindow?.send('chat:changed', agent.state());
    pushState();
  });
  workspace.on('auftrag', (file) => handleOrder(file));
  workspace.on('files', (files) => appWindow?.send('files', files));
  workspace.on('tasks', (tasks) => appWindow?.send('tasks', tasks));
}

app.on('second-instance', () => {
  if (appWindow?.win) {
    if (appWindow.win.isMinimized()) appWindow.win.restore();
    appWindow.win.focus();
  }
});

app.whenReady().then(() => {
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
  if (singleInstance) cleanupOldVersions();
  if (!app.isPackaged && process.env.ASK_TEST_SCRIPT) {
    require(process.env.ASK_TEST_SCRIPT)({ app, appWindow, terminal, ask, agent, workspace, settings, openFolder, pushState, folderMenuItems });
  }
});

app.on('before-quit', (event) => {
  if (quitting) return;
  event.preventDefault();
  quitting = true;
  agent.stop();
  visual.closeRenderer();
  workspace.close();
  terminal.stop().finally(() => app.quit());
});

app.on('window-all-closed', () => app.quit());
