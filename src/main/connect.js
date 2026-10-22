// Verbindung zwischen ASK (Open WebUI) und dem lokalen Terminal.
//
// Läuft über die angemeldete ASK-Seite: Der Code unten wird in der Seite ausgeführt und nutzt
// das Token, das ASK selbst in localStorage hält. Geschrieben wird nur, wenn ASK die persönliche
// Terminal-Verbindung für diesen Nutzer freigegeben hat (siehe docs/ENTSCHEIDE.md, E8).
const { EventEmitter } = require('node:events');

const PLACEHOLDER_KEY = 'ask-desktop';
const ENTRY_NAME = 'ASK Desktop';
const MIN_VERSION = [0, 8, 11]; // ab hier holt Open WebUI den Systemprompt des Terminals

// Wird als Funktionsquelltext in die ASK-Seite gegeben; darf nichts aus diesem Modul verwenden.
async function pageProbe(terminalUrl, placeholderKey, entryName, apply) {
  const token = localStorage.token;
  if (!token) return { state: 'nicht-angemeldet' };
  const headers = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };
  const getJson = async (url) => {
    const res = await fetch(url, { headers });
    if (res.status === 401 || res.status === 403) throw Object.assign(new Error('auth'), { auth: true });
    if (!res.ok) throw new Error(`${url}: HTTP ${res.status}`);
    return res.json();
  };
  try {
    const config = await getJson('/api/config');
    const me = await getJson('/api/v1/auths/');
    const features = config.features || {};
    const isAdmin = me.role === 'admin';
    const permitted = isAdmin || me.permissions?.features?.direct_tool_servers === true;
    const result = {
      version: config.version || '',
      role: me.role,
      toolServers: features.enable_tool_servers !== false,
      directIntegrations: features.enable_direct_integrations === true,
      permission: permitted,
    };
    if (!result.toolServers || !result.directIntegrations || !permitted) {
      return { ...result, state: 'nicht-freigegeben' };
    }
    const settings = await getJson('/api/v1/users/user/settings');
    const servers = Array.isArray(settings?.ui?.terminalServers) ? settings.ui.terminalServers : [];
    const ours = servers.find((s) => s.url === terminalUrl);
    const ready = ours && ours.enabled && ours.key === placeholderKey;
    const selected = localStorage.selectedTerminalId === terminalUrl;
    if (ready && (selected || !apply)) return { ...result, state: 'eingerichtet', selected };
    if (!apply) return { ...result, state: 'nicht-eingerichtet' };

    // Alte Einträge der App (anderer Port) entfernen, andere Terminals nur deaktivieren:
    // Open WebUI erlaubt immer nur eine aktive persönliche Terminal-Verbindung.
    const others = servers
      .filter((s) => s.url !== terminalUrl && s.name !== entryName)
      .map((s) => ({ ...s, enabled: false }));
    const entry = { ...(ours || {}), url: terminalUrl, key: placeholderKey, name: entryName, enabled: true,
      auth_type: 'bearer', path: '/openapi.json', config: ours?.config || {} };
    const res = await fetch('/api/v1/users/user/settings/update', {
      method: 'POST', headers, body: JSON.stringify({ ui: { terminalServers: [...others, entry] } }),
    });
    if (!res.ok) return { ...result, state: 'fehler', message: `Speichern in ASK fehlgeschlagen (HTTP ${res.status})` };
    localStorage.selectedTerminalId = terminalUrl;
    return { ...result, state: 'eingerichtet', changed: true, selected: true };
  } catch (err) {
    if (err.auth) return { state: 'nicht-angemeldet' };
    return { state: 'fehler', message: String(err.message || err) };
  }
}

function versionTooOld(version) {
  const parts = String(version).split('.').map((n) => parseInt(n, 10));
  if (parts.some(Number.isNaN) || parts.length < 2) return false;
  for (let i = 0; i < 3; i++) {
    if ((parts[i] || 0) > MIN_VERSION[i]) return false;
    if ((parts[i] || 0) < MIN_VERSION[i]) return true;
  }
  return false;
}

class Connection extends EventEmitter {
  constructor() {
    super();
    this.view = null;
    this.terminalUrl = '';
    this.status = { state: 'unbekannt' };
    this.pendingApply = false;
    this.autoConnect = true;
    this.lastSeen = 0;
    this.timer = null;
    this.busy = false;
  }

  attach(view, getAskOrigin) {
    this.view = view;
    this.getAskOrigin = getAskOrigin;
    const wc = view.webContents;
    wc.on('did-finish-load', () => this.check());
    wc.on('did-navigate-in-page', () => this.check());
    clearInterval(this.timer);
    this.timer = setInterval(() => this.check(), 8000);
  }

  // Neuer Terminal-Start: Verbindung in ASK beim nächsten Check einrichten bzw. prüfen.
  setTerminal(url, autoConnect) {
    if (url !== this.terminalUrl) this.lastSeen = 0;
    this.terminalUrl = url;
    this.autoConnect = autoConnect;
    this.pendingApply = !!url && autoConnect;
    this.check();
  }

  // Erfolgreiche Anfrage von ASK an das Terminal beobachtet (aus window.js).
  markSeen() {
    const first = !this.lastSeen;
    this.lastSeen = Date.now();
    if (first) this.publish({ ...this.status, state: 'verbunden' });
  }

  connected() {
    return this.lastSeen > 0;
  }

  publish(status) {
    if (this.connected() && ['eingerichtet', 'nicht-eingerichtet'].includes(status.state)) {
      status = { ...status, state: 'verbunden' };
    }
    if (status.version && versionTooOld(status.version)) status = { ...status, state: 'zu-alt' };
    this.status = status;
    this.emit('status', this.status);
  }

  async check(force = false) {
    if (!this.view || !this.terminalUrl || this.busy) return this.status;
    const wc = this.view.webContents;
    if (wc.isDestroyed() || wc.isLoading()) return this.status;
    const origin = this.getAskOrigin();
    if (!origin || !wc.getURL().startsWith(origin + '/')) return this.status;
    const apply = force || this.pendingApply;
    this.busy = true;
    try {
      const source = `(${pageProbe.toString()})(${JSON.stringify(this.terminalUrl)}, ${JSON.stringify(PLACEHOLDER_KEY)}, ${JSON.stringify(ENTRY_NAME)}, ${apply})`;
      const result = await wc.executeJavaScript(source, true);
      if (apply && ['eingerichtet', 'nicht-freigegeben', 'zu-alt'].includes(result.state)) this.pendingApply = false;
      this.publish(result);
      if (result.changed) wc.reload();
    } catch (err) {
      this.publish({ state: 'fehler', message: err.message });
    } finally {
      this.busy = false;
    }
    return this.status;
  }
}

module.exports = { Connection, PLACEHOLDER_KEY, ENTRY_NAME, versionTooOld, pageProbe };
