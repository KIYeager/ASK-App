// Hauptfenster: eigene Oberfläche (Chat, Seitenleiste, Dialoge). Die ASK-Seite liegt als
// WebContentsView darüber und ist nur für die Anmeldung sichtbar.
const path = require('node:path');
const { BrowserWindow, WebContentsView, Menu, shell, session, clipboard } = require('electron');
const paths = require('./paths');
const { allowedNavigation } = require('./visual');
const { sameOrigin } = require('./askapi');

const ASK_PARTITION = 'persist:ask';
const BROWSER_PARTITION = 'persist:ask-browser';
// Berechtigungen, die ASK bekommen darf. Alles andere wird abgelehnt.
const ALLOWED_PERMISSIONS = new Set(['clipboard-read', 'clipboard-sanitized-write', 'notifications', 'media',
  'fullscreen', 'local-network-access', 'local-network', 'loopback-network', 'speaker-selection']);

class AppWindow {
  constructor() {
    this.askOrigin = '';
    this.askBounds = null;
    this.askVisible = false;

    this.win = new BrowserWindow({
      width: 1440, height: 920, minWidth: 960, minHeight: 600,
      title: 'ASK', icon: paths.iconFile(), backgroundColor: require('electron').nativeTheme.shouldUseDarkColors ? '#16191b' : '#ffffff', show: false, autoHideMenuBar: true,
      webPreferences: {
        preload: path.join(paths.preloadDir(), 'shell.js'),
        contextIsolation: true, sandbox: true, plugins: true, spellcheck: false,
      },
    });
    this.win.setMenuBarVisibility(false);
    this.win.once('ready-to-show', () => this.win.show());
    this.win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
    this.win.webContents.on('will-navigate', (e) => e.preventDefault());
    this.win.webContents.on('did-fail-load', (_e, code, desc, url, main) => {
      if (main && code !== -3) console.error(`Oberfläche nicht geladen (${desc}): ${url}. Zuerst «npm run build:ui».`);
    });
    // Entwicklung mit Vite: ASK_DEV_UI=http://localhost:5173 lädt die Oberfläche vom Dev-Server
    const devUi = !require('electron').app.isPackaged && /^http:\/\/(localhost|127\.0\.0\.1):\d+\/?$/.test(process.env.ASK_DEV_UI || '');
    if (devUi) this.win.loadURL(process.env.ASK_DEV_UI).catch(() => {});
    else this.win.loadFile(path.join(paths.rendererDir(), 'index.html')).catch(() => {});

    this.askSession = session.fromPartition(ASK_PARTITION);
    this.setupAskSession();
    this.view = new WebContentsView({
      webPreferences: { partition: ASK_PARTITION, contextIsolation: true, sandbox: true, spellcheck: true },
    });
    this.view.setBackgroundColor('#ffffff');
    this.view.setVisible(false);
    this.win.contentView.addChildView(this.view);
    this.setupAskView();
    this.setupMenu();
  }

  setupAskSession() {
    const ses = this.askSession;
    try {
      ses.setSpellCheckerLanguages(['de-CH', 'de']);
    } catch {
      // Wörterbuch nicht verfügbar
    }
    const allowed = (origin, permission) => !!this.askOrigin && origin === this.askOrigin
      && ALLOWED_PERMISSIONS.has(permission);
    ses.setPermissionRequestHandler((wc, permission, callback, details) => {
      let origin = '';
      try { origin = new URL(details.requestingUrl || wc.getURL()).origin; } catch { /* leer */ }
      callback(allowed(origin, permission));
    });
    ses.setPermissionCheckHandler((wc, permission, requestingOrigin) => {
      let origin = requestingOrigin;
      try { origin = new URL(requestingOrigin).origin; } catch { /* leer */ }
      return allowed(origin, permission);
    });
  }

  setupAskView() {
    const wc = this.view.webContents;
    wc.setWindowOpenHandler(({ url }) => {
      if (/^https?:/i.test(url)) shell.openExternal(url);
      return { action: 'deny' };
    });
    wc.on('context-menu', (_e, params) => this.showContextMenu(params));
    // Ladefehler der ASK-Seite meldet askapi.js im Status (loadError), nicht als Ereignis (ENTSCHEIDE E53)
  }

  showContextMenu(params, wc = this.view.webContents) {
    const items = contextMenuItems(params, wc);
    if (items.length) Menu.buildFromTemplate(items).popup({ window: this.win });
  }

  setupMenu() {
    // Unsichtbares Menü, damit die üblichen Tastenkürzel funktionieren.
    const ask = () => this.view.webContents;
    const ui = () => this.win.webContents;
    const zoom = (delta) => {
      ui().setZoomLevel(delta === 0 ? 0 : ui().getZoomLevel() + delta);
      setTimeout(() => { this.applyAskLayout(); this.applyBrowserLayout(); }, 50); // die Ansichten liegen in Pixeln über der Oberfläche
    };
    const menu = Menu.buildFromTemplate([
      { label: 'Bearbeiten', submenu: [
        { role: 'undo', label: 'Rückgängig' }, { role: 'redo', label: 'Wiederholen' }, { type: 'separator' },
        { role: 'cut', label: 'Ausschneiden' }, { role: 'copy', label: 'Kopieren' },
        { role: 'paste', label: 'Einfügen' }, { role: 'selectAll', label: 'Alles auswählen' },
      ] },
      { label: 'Ansicht', submenu: [
        { label: 'ASK-Anmeldeseite neu laden', accelerator: 'CmdOrCtrl+R', click: () => ask().reload() },
        { label: 'Vergrössern', accelerator: 'CmdOrCtrl+Plus', click: () => zoom(0.5) },
        { label: 'Vergrössern', accelerator: 'CmdOrCtrl+=', visible: false, click: () => zoom(0.5) },
        { label: 'Verkleinern', accelerator: 'CmdOrCtrl+-', click: () => zoom(-0.5) },
        { label: 'Originalgrösse', accelerator: 'CmdOrCtrl+0', click: () => zoom(0) },
        { label: 'Entwicklerwerkzeuge ASK', accelerator: 'CmdOrCtrl+Shift+I', click: () => ask().openDevTools({ mode: 'detach' }) },
        { label: 'Entwicklerwerkzeuge App', accelerator: 'CmdOrCtrl+Shift+J',
          click: () => this.win.webContents.openDevTools({ mode: 'detach' }) },
      ] },
    ]);
    this.win.setMenu(menu);
  }

  loadAsk(url) {
    this.askOrigin = new URL(url).origin;
    const current = this.view.webContents.getURL();
    if (!current || !sameOrigin(current, url)) this.view.webContents.loadURL(url).catch(() => {}); // did-fail-load zeigt den Fehler
  }

  setAskBounds(bounds) {
    this.askBounds = bounds;
    this.applyAskLayout();
  }

  setAskVisible(visible) {
    this.askVisible = visible;
    this.applyAskLayout();
  }

  applyAskLayout() {
    const b = this.askBounds;
    const show = this.askVisible && b && b.width > 0 && b.height > 0;
    if (show) {
      const z = this.win.webContents.getZoomFactor(); // Bereich in CSS-Pixeln der Oberfläche, wie beim Browser
      this.view.setBounds({ x: Math.round(b.x * z), y: Math.round(b.y * z), width: Math.round(b.width * z), height: Math.round(b.height * z) });
    }
    this.view.setVisible(!!show);
  }

  // Browser der App (ENTSCHEIDE E27, E29): eine echte Seite, die das Modell steuert und die Person sieht und
  // selbst bedienen kann. Eigene Sitzung getrennt von ASK und von Chrome, Anmeldungen bleiben gespeichert.
  // Webseiten sind frei, Dateien nur im Arbeitsordner (fileRoot). Keine Downloads, keine Berechtigungen.
  ensureBrowser() {
    if (this.browser && !this.browser.webContents.isDestroyed()) return this.browser;
    this.browserBounds = this.browserBounds || null;
    this.browserVisible = !!this.browserVisible;
    this.browser = new WebContentsView({
      webPreferences: { partition: BROWSER_PARTITION, sandbox: true, contextIsolation: true, nodeIntegration: false,
        backgroundThrottling: false, spellcheck: false },
    });
    this.browser.setBackgroundColor('#000000');
    this.browser.setVisible(false);
    this.browser.fileRoot = '';
    this.win.contentView.addChildView(this.browser);
    const wc = this.browser.webContents;
    // Neue Tabs öffnen in derselben Ansicht, echte Popups (Anmeldung über Microsoft usw.) als kleines Fenster
    wc.setWindowOpenHandler(({ url, disposition }) => {
      if (!/^https?:/i.test(url)) return { action: 'deny' };
      if (disposition === 'new-window') {
        return { action: 'allow', overrideBrowserWindowOptions: { parent: this.win, width: 520, height: 700, autoHideMenuBar: true,
          webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false } } };
      }
      wc.loadURL(url).catch(() => {});
      return { action: 'deny' };
    });
    wc.on('will-navigate', (e, url) => { if (!allowedNavigation(url, this.browser.fileRoot)) e.preventDefault(); });
    wc.on('context-menu', (_e, params) => this.showContextMenu(params, wc));
    wc.session.setPermissionRequestHandler((_wc, perm, cb) => cb(perm === 'fullscreen'));
    wc.session.setPermissionCheckHandler((_wc, perm) => perm === 'fullscreen');
    wc.session.on('will-download', (e, item) => {
      e.preventDefault();
      this.send('browser:notice', `Download blockiert: ${item.getFilename()}`);
    });
    const report = () => {
      if (wc.isDestroyed()) return;
      this.send('browser:state', { url: wc.getURL(), title: wc.getTitle(), back: wc.navigationHistory.canGoBack(),
        forward: wc.navigationHistory.canGoForward(), loading: wc.isLoading() });
    };
    for (const ev of ['did-navigate', 'did-navigate-in-page', 'did-start-loading', 'did-stop-loading', 'page-title-updated']) wc.on(ev, report);
    wc.on('did-fail-load', (_e, code, description, url, isMainFrame) => {
      if (isMainFrame && code !== -3) this.send('browser:notice', `Seite nicht erreichbar: ${description} (${url})`);
    });
    this.applyBrowserLayout();
    return this.browser;
  }

  setBrowserBounds(bounds) {
    this.browserBounds = bounds;
    this.applyBrowserLayout();
  }

  setBrowserVisible(visible) {
    this.browserVisible = !!visible;
    this.applyBrowserLayout();
  }

  // Ist der Browser-Bereich sichtbar und gross genug? (sonst testet visual.js unsichtbar)
  browserReady() {
    const b = this.browserBounds;
    return !!(this.browser && this.browserVisible && b && b.width >= 200 && b.height >= 150
      && this.win.isVisible() && !this.win.isMinimized());
  }

  applyBrowserLayout() {
    if (!this.browser) return;
    const b = this.browserBounds;
    const show = this.browserVisible && b && b.width > 0 && b.height > 0;
    if (show) {
      const z = this.win.webContents.getZoomFactor(); // Bereich in CSS-Pixeln der Oberfläche
      this.browser.setBounds({ x: Math.round(b.x * z), y: Math.round(b.y * z), width: Math.round(b.width * z), height: Math.round(b.height * z) });
    }
    this.browser.setVisible(!!show);
  }

  send(channel, payload) {
    if (!this.win.isDestroyed()) this.win.webContents.send(channel, payload);
  }
}

// Einträge des Rechtsklick-Menüs für Text, Links und Bilder (ASK-Seite, Browser und die eigene Oberfläche, ENTSCHEIDE E56).
// Mit click statt role, damit dieselben Einträge auch im Menü der Oberfläche funktionieren.
function contextMenuItems(params, wc) {
  const items = [];
  if (params.misspelledWord) {
    for (const s of params.dictionarySuggestions.slice(0, 5)) {
      items.push({ label: s, click: () => wc.replaceMisspelling(s) });
    }
    items.push({ label: 'Zum Wörterbuch hinzufügen',
      click: () => wc.session.addWordToSpellCheckerDictionary(params.misspelledWord) });
    items.push({ type: 'separator' });
  }
  // Nur Webadressen ins System geben: file:, search-ms:, ms-msdt: und andere Protokolle starten unter Windows Programme
  if (params.linkURL && /^https?:\/\//i.test(params.linkURL)) {
    items.push({ label: 'Link im Browser öffnen', click: () => shell.openExternal(params.linkURL) });
    items.push({ label: 'Link kopieren', click: () => clipboard.writeText(params.linkURL) });
    items.push({ type: 'separator' });
  }
  if (params.isEditable) {
    items.push({ label: 'Ausschneiden', hint: 'Ctrl+X', enabled: params.editFlags.canCut, click: () => wc.cut() });
    items.push({ label: 'Kopieren', hint: 'Ctrl+C', enabled: params.editFlags.canCopy, click: () => wc.copy() });
    items.push({ label: 'Einfügen', hint: 'Ctrl+V', enabled: params.editFlags.canPaste, click: () => wc.paste() });
    items.push({ label: 'Alles auswählen', hint: 'Ctrl+A', enabled: params.editFlags.canSelectAll !== false, click: () => wc.selectAll() });
  } else if (params.selectionText) {
    items.push({ label: 'Kopieren', hint: 'Ctrl+C', click: () => wc.copy() });
  }
  if (params.mediaType === 'image' && params.srcURL) {
    items.push({ label: 'Bild kopieren', click: () => wc.copyImageAt(params.x, params.y) });
  }
  while (items.length && items[items.length - 1].type === 'separator') items.pop();
  return items;
}

module.exports = { AppWindow, ASK_PARTITION, BROWSER_PARTITION, contextMenuItems };
