// Hauptfenster: eigene Oberfläche (Chat, Seitenleiste, Dialoge). Die ASK-Seite liegt als
// WebContentsView darüber und ist nur für die Anmeldung sichtbar.
const path = require('node:path');
const { BrowserWindow, WebContentsView, Menu, shell, session, clipboard } = require('electron');
const paths = require('./paths');

const ASK_PARTITION = 'persist:ask';
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
      title: 'ASK', icon: paths.iconFile(), backgroundColor: '#141414', show: false, autoHideMenuBar: true,
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
    const ui = () => this.win.webContents;
    const zoom = (delta) => ui().setZoomLevel(delta === 0 ? 0 : ui().getZoomLevel() + delta);
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

  // Browser-Bereich für browser_test (ENTSCHEIDE E27): eine echte Seite in der App, die das Modell steuert
  // und die Person sieht und selbst bedienen kann. Eigene flüchtige Sitzung, ohne Popups, Downloads und
  // Berechtigungen; Navigation nur innerhalb der geöffneten Datei (allowedUrl).
  ensureBrowser() {
    if (this.browser && !this.browser.webContents.isDestroyed()) return this.browser;
    this.browserBounds = this.browserBounds || null;
    this.browserVisible = !!this.browserVisible;
    this.browser = new WebContentsView({
      webPreferences: { partition: 'ask-browser', sandbox: true, contextIsolation: true, nodeIntegration: false,
        backgroundThrottling: false, spellcheck: false },
    });
    this.browser.setBackgroundColor('#000000');
    this.browser.setVisible(false);
    this.browser.allowedUrl = '';
    this.win.contentView.addChildView(this.browser);
    const wc = this.browser.webContents;
    wc.setWindowOpenHandler(() => ({ action: 'deny' }));
    wc.on('will-navigate', (e, url) => { if (url.split('#')[0] !== this.browser.allowedUrl) e.preventDefault(); });
    wc.session.setPermissionRequestHandler((_wc, _perm, cb) => cb(false));
    wc.session.setPermissionCheckHandler(() => false);
    wc.session.on('will-download', (e) => e.preventDefault());
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

module.exports = { AppWindow, ASK_PARTITION };
