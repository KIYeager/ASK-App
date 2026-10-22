// Hauptfenster: eigene Oberfläche (Statusleiste, Seitenleiste, Dialoge) und ASK als WebContentsView.
const path = require('node:path');
const { BrowserWindow, WebContentsView, Menu, shell, session, clipboard } = require('electron');
const paths = require('./paths');

const ASK_PARTITION = 'persist:ask';
// Berechtigungen, die ASK bekommen darf. Alles andere wird abgelehnt.
const ALLOWED_PERMISSIONS = new Set(['clipboard-read', 'clipboard-sanitized-write', 'notifications', 'media',
  'fullscreen', 'local-network-access', 'local-network', 'loopback-network', 'speaker-selection']);

class AppWindow {
  constructor({ onAskRequest }) {
    this.onAskRequest = onAskRequest;
    this.askOrigin = '';
    this.terminalPort = 0;
    this.terminalKey = '';
    this.askBounds = null;
    this.askVisible = false;

    this.win = new BrowserWindow({
      width: 1440, height: 920, minWidth: 960, minHeight: 600,
      title: 'ASK', icon: paths.iconFile(), backgroundColor: '#f4f5f7', show: false, autoHideMenuBar: true,
      webPreferences: {
        preload: path.join(paths.preloadDir(), 'shell.js'),
        contextIsolation: true, sandbox: true, plugins: true, spellcheck: false,
      },
    });
    this.win.setMenuBarVisibility(false);
    this.win.once('ready-to-show', () => this.win.show());
    this.win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
    this.win.webContents.on('will-navigate', (e) => e.preventDefault());
    this.win.loadFile(path.join(paths.rendererDir(), 'index.html'));

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

  // Echter Key wird nur hier eingesetzt; ASK selbst kennt nur den Platzhalter.
  setTerminal(port, key) {
    this.terminalPort = port;
    this.terminalKey = key;
    const filter = { urls: [`http://127.0.0.1:${port}/*`, `ws://127.0.0.1:${port}/*`] };
    this.askSession.webRequest.onBeforeSendHeaders(filter, (details, callback) => {
      const headers = details.requestHeaders;
      const name = Object.keys(headers).find((k) => k.toLowerCase() === 'authorization');
      const fromAsk = details.webContentsId === this.view.webContents.id;
      if (fromAsk && this.terminalKey && (name || details.resourceType === 'webSocket')) {
        headers[name || 'Authorization'] = `Bearer ${this.terminalKey}`;
      }
      callback({ requestHeaders: headers });
    });
    this.askSession.webRequest.onCompleted({ urls: [`http://127.0.0.1:${port}/*`] }, (details) => {
      if (details.method !== 'OPTIONS' && details.statusCode < 400 && !/\/(health|api\/config)$/.test(details.url)) {
        this.onAskRequest();
      }
    });
  }

  setupAskView() {
    const wc = this.view.webContents;
    wc.setWindowOpenHandler(({ url }) => {
      if (/^https?:/i.test(url)) shell.openExternal(url);
      return { action: 'deny' };
    });
    wc.on('context-menu', (_e, params) => this.showContextMenu(params));
    wc.on('did-fail-load', (_e, code, description, url, isMainFrame) => {
      if (isMainFrame && code !== -3) this.send('ask:load-failed', { code, description, url });
    });
    wc.on('did-finish-load', () => this.send('ask:loaded', { url: wc.getURL() }));
    wc.on('page-title-updated', (_e, title) => this.win.setTitle(title ? `ASK – ${title.replace(/ ?[•|-] ?Open WebUI$/, '')}` : 'ASK'));
  }

  showContextMenu(params) {
    const wc = this.view.webContents;
    const items = [];
    if (params.misspelledWord) {
      for (const s of params.dictionarySuggestions.slice(0, 5)) {
        items.push({ label: s, click: () => wc.replaceMisspelling(s) });
      }
      items.push({ label: 'Zum Wörterbuch hinzufügen',
        click: () => wc.session.addWordToSpellCheckerDictionary(params.misspelledWord) });
      items.push({ type: 'separator' });
    }
    if (params.linkURL) {
      items.push({ label: 'Link im Browser öffnen', click: () => shell.openExternal(params.linkURL) });
      items.push({ label: 'Link kopieren', click: () => clipboard.writeText(params.linkURL) });
      items.push({ type: 'separator' });
    }
    if (params.isEditable) {
      items.push({ label: 'Ausschneiden', role: 'cut', enabled: params.editFlags.canCut });
      items.push({ label: 'Kopieren', role: 'copy', enabled: params.editFlags.canCopy });
      items.push({ label: 'Einfügen', role: 'paste', enabled: params.editFlags.canPaste });
      items.push({ label: 'Alles auswählen', role: 'selectAll' });
    } else if (params.selectionText) {
      items.push({ label: 'Kopieren', role: 'copy' });
    }
    if (params.mediaType === 'image' && params.srcURL) {
      items.push({ label: 'Bild kopieren', click: () => wc.copyImageAt(params.x, params.y) });
    }
    if (items.length) Menu.buildFromTemplate(items).popup({ window: this.win });
  }

  setupMenu() {
    // Unsichtbares Menü, damit die üblichen Tastenkürzel funktionieren.
    const ask = () => this.view.webContents;
    const zoom = (delta) => ask().setZoomLevel(delta === 0 ? 0 : ask().getZoomLevel() + delta);
    const menu = Menu.buildFromTemplate([
      { label: 'Bearbeiten', submenu: [
        { role: 'undo', label: 'Rückgängig' }, { role: 'redo', label: 'Wiederholen' }, { type: 'separator' },
        { role: 'cut', label: 'Ausschneiden' }, { role: 'copy', label: 'Kopieren' },
        { role: 'paste', label: 'Einfügen' }, { role: 'selectAll', label: 'Alles auswählen' },
      ] },
      { label: 'Ansicht', submenu: [
        { label: 'ASK neu laden', accelerator: 'CmdOrCtrl+R', click: () => ask().reload() },
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
    if (!current || !current.startsWith(this.askOrigin)) this.view.webContents.loadURL(url);
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
      this.view.setBounds({ x: Math.round(b.x), y: Math.round(b.y), width: Math.round(b.width), height: Math.round(b.height) });
    }
    this.view.setVisible(!!show);
  }

  send(channel, payload) {
    if (!this.win.isDestroyed()) this.win.webContents.send(channel, payload);
  }
}

module.exports = { AppWindow, ASK_PARTITION };
