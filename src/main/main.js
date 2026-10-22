// ASK Desktop: Open WebUI der TF Bern mit lokalem Arbeitsordner und Terminal.
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { app, ipcMain, dialog, shell, clipboard, protocol, net, Menu } = require('electron');
const settings = require('./settings');
const paths = require('./paths');
const { checkFolder } = require('./folders');
const { Terminal } = require('./terminal');
const { Connection, PLACEHOLDER_KEY } = require('./connect');
const { Workspace } = require('./workspace');
const { AppWindow } = require('./window');
const { convertToPdf } = require('./office');

app.setAppUserModelId('ch.tfbern.ask');
// Nur für automatische Tests im Entwicklungsmodus: eigener Datenordner.
if (!app.isPackaged && process.env.ASK_TEST_USER_DATA) app.setPath('userData', process.env.ASK_TEST_USER_DATA);
protocol.registerSchemesAsPrivileged([
  { scheme: 'askfile', privileges: { standard: true, secure: true, supportFetchAPI: true, stream: true } },
]);

if (!app.requestSingleInstanceLock()) {
  app.quit();
}

const terminal = new Terminal();
const connection = new Connection();
const workspace = new Workspace();
let appWindow = null;
let quitting = false;

const TEXT_EXT = new Set(['.txt', '.md', '.csv', '.json', '.py', '.js', '.html', '.css', '.xml', '.log', '.yml',
  '.yaml', '.ini', '.toml', '.bat', '.ps1', '.sql', '.tsv', '.java', '.c', '.cpp', '.h', '.ts']);
const IMAGE_EXT = new Set(['.png', '.jpg', '.jpeg', '.gif', '.webp', '.svg', '.bmp']);
const OFFICE_EXT = new Set(['.docx', '.doc', '.xlsx', '.xls', '.pptx', '.ppt', '.odt', '.ods', '.odp']);

function readInstruction(name, folder) {
  try {
    const text = fs.readFileSync(path.join(paths.instructionsDir(), name), 'utf8');
    return text.replaceAll('{ordner}', folder).replaceAll('{ordnername}', path.basename(folder));
  } catch {
    return '';
  }
}

function state() {
  const s = settings.load();
  return {
    askUrl: s.askUrl,
    folder: s.folder,
    recent: s.recent,
    warningAccepted: s.warningAccepted,
    sidebarOpen: s.sidebarOpen,
    autoConnect: s.autoConnect,
    terminal: terminal.snapshot(),
    ask: connection.status,
    placeholderKey: PLACEHOLDER_KEY,
  };
}

function pushState() {
  appWindow?.send('state', state());
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
    const { recent, folder } = settings.load();
    const items = recent.map((f) => ({
      label: f, type: 'radio', checked: f === folder,
      click: () => openFolder(f).then((res) => { if (!res.ok) appWindow.send('folder:error', res.reason); }),
    }));
    if (items.length) items.push({ type: 'separator' });
    items.push({ label: 'Anderen Ordner wählen …', click: () => ipcMain.emit('folder:choose-from-menu') });
    items.push({ label: 'Neuen Projektordner anlegen', click: () => appWindow.send('menu:new-folder') });
    if (folder) {
      items.push({ type: 'separator' });
      items.push({ label: 'Arbeitsordner im Explorer öffnen', click: () => shell.openPath(folder) });
      items.push({ label: 'Aus der Liste entfernen', click: () => { settings.removeRecent(folder); workspace.close(); terminal.stop(); pushState(); } });
    }
    Menu.buildFromTemplate(items).popup({ window: appWindow.win });
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
    appWindow.loadAsk(settings.get('askUrl'));
    startTerminal();
    pushState();
    return { ok: true };
  });
  ipcMain.handle('ask:connect', () => connection.check(true));
  ipcMain.handle('ask:reload', () => {
    const wc = appWindow.view.webContents;
    const url = settings.get('askUrl');
    if (url && !wc.getURL().startsWith(new URL(url).origin)) wc.loadURL(url);
    else wc.reload();
  });
  ipcMain.handle('ui:sidebar', (_e, open) => {
    settings.set({ sidebarOpen: !!open });
    pushState();
  });
  ipcMain.on('ui:ask-bounds', (_e, bounds) => appWindow.setAskBounds(bounds));
  ipcMain.on('ui:ask-visible', (_e, visible) => appWindow.setAskVisible(visible));
  ipcMain.handle('clipboard:write', (_e, text) => clipboard.writeText(String(text)));
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

function wireEvents() {
  terminal.on('port', (port) => {
    settings.set({ port });
    appWindow.setTerminal(port, terminal.key);
  });
  terminal.on('state', (snap) => {
    if (snap.state === 'läuft') connection.setTerminal(terminal.url(), settings.get('autoConnect'));
    pushState();
  });
  connection.on('status', () => pushState());
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
  appWindow = new AppWindow({ onAskRequest: () => connection.markSeen() });
  connection.attach(appWindow.view, () => appWindow.askOrigin);
  wireEvents();
  const s = settings.load();
  if (s.askUrl) appWindow.loadAsk(s.askUrl);
  if (s.folder) {
    openFolder(s.folder).then((res) => {
      if (!res.ok) {
        settings.set({ folder: '' });
        pushState();
      }
    });
  }
  appWindow.win.on('closed', () => { appWindow = null; });
  if (!app.isPackaged && process.env.ASK_TEST_SCRIPT) {
    require(process.env.ASK_TEST_SCRIPT)({ app, appWindow, terminal, connection, workspace, settings, openFolder, pushState });
  }
});

app.on('before-quit', (event) => {
  if (quitting) return;
  event.preventDefault();
  quitting = true;
  workspace.close();
  terminal.stop().finally(() => app.quit());
});

app.on('window-all-closed', () => app.quit());
