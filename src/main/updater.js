// Updates (ENTSCHEIDE E34). Die Adresse steht als «updateUrl» in resources/app-config.json; dort liegen die
// Dateien, die electron-builder mit publish «generic» erzeugt (latest.yml, ASK-Setup-<Version>.exe, ASK-<Version>.exe).
// Installierte App: electron-updater lädt im Hintergrund, die Person startet neu, wann sie will.
// Einzelne EXE: nur ein Hinweis mit Link, denn die EXE ersetzt sich nicht selbst.
// Ohne updateUrl oder im Entwicklungsmodus ist die Prüfung aus; die Version wird trotzdem angezeigt.
const { EventEmitter } = require('node:events');

const CHECK_EVERY_MS = 4 * 60 * 60 * 1000;

// Versionen wie 1.2.3 oder 1.2.3-beta.1 vergleichen: >0 wenn a neuer ist
function compareVersions(a, b) {
  const parse = (v) => String(v || '').trim().replace(/^v/i, '').split('-');
  const [ma, pa] = parse(a);
  const [mb, pb] = parse(b);
  const na = ma.split('.').map((n) => Number(n) || 0);
  const nb = mb.split('.').map((n) => Number(n) || 0);
  for (let i = 0; i < Math.max(na.length, nb.length, 3); i++) {
    const d = (na[i] || 0) - (nb[i] || 0);
    if (d) return d;
  }
  if (pa && !pb) return -1; // Vorabversion ist älter als die fertige
  if (!pa && pb) return 1;
  return String(pa || '').localeCompare(String(pb || ''), 'en', { numeric: true });
}

// Version aus latest.yml («version: 1.2.3»)
function versionFromYaml(text) {
  const m = /^version:\s*['"]?([^'"\s]+)['"]?\s*$/m.exec(String(text || ''));
  return m ? m[1] : '';
}

class Updater extends EventEmitter {
  constructor({ version, url, packaged, portable, fetchImpl, loadAutoUpdater }) {
    super();
    this.version = version;
    this.url = String(url || '').replace(/\/+$/, '');
    this.packaged = !!packaged;
    this.portable = !!portable;
    this.fetchImpl = fetchImpl;
    this.loadAutoUpdater = loadAutoUpdater || (() => require('electron-updater').autoUpdater);
    this.timer = null;
    this.status = { state: this.enabled() ? 'bereit-zu-pruefen' : 'aus', version, available: '', progress: 0, message: '', download: '' };
  }

  enabled() {
    return this.packaged && /^https:\/\//i.test(this.url);
  }

  set(patch) {
    this.status = { ...this.status, ...patch };
    this.emit('status', this.status);
  }

  start() {
    if (!this.enabled()) return;
    if (!this.portable) this.setupAutoUpdater();
    setTimeout(() => this.check(), 15000); // nicht beim Start bremsen
    this.timer = setInterval(() => this.check(), CHECK_EVERY_MS);
    this.timer.unref?.();
  }

  setupAutoUpdater() {
    const au = this.loadAutoUpdater();
    this.autoUpdater = au;
    au.autoDownload = true;
    au.autoInstallOnAppQuit = true;
    au.setFeedURL({ provider: 'generic', url: this.url });
    au.on('checking-for-update', () => this.set({ state: 'pruefen', message: '' }));
    au.on('update-not-available', () => this.set({ state: 'aktuell', message: '' }));
    au.on('update-available', (info) => this.set({ state: 'laedt', available: info?.version || '', progress: 0 }));
    au.on('download-progress', (p) => this.set({ state: 'laedt', progress: Math.round(p?.percent || 0) }));
    au.on('update-downloaded', (info) => this.set({ state: 'bereit', available: info?.version || this.status.available, progress: 100 }));
    au.on('error', (err) => this.set({ state: 'fehler', message: String(err?.message || err).split('\n')[0].slice(0, 200) }));
  }

  async check() {
    if (!this.enabled() || ['laedt', 'bereit'].includes(this.status.state)) return this.status;
    if (this.autoUpdater) {
      try { await this.autoUpdater.checkForUpdates(); } catch (err) { this.set({ state: 'fehler', message: String(err.message || err).slice(0, 200) }); }
      return this.status;
    }
    // Einzelne EXE: latest.yml lesen und auf die neue EXE hinweisen
    this.set({ state: 'pruefen', message: '' });
    try {
      const res = await this.fetchImpl(`${this.url}/latest.yml?t=${Date.now()}`);
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const latest = versionFromYaml(await res.text());
      if (latest && compareVersions(latest, this.version) > 0) {
        this.set({ state: 'verfuegbar', available: latest, download: `${this.url}/ASK-${latest}.exe` });
      } else {
        this.set({ state: 'aktuell' });
      }
    } catch (err) {
      this.set({ state: 'fehler', message: `Keine Verbindung zum Update-Server (${err.message})` });
    }
    return this.status;
  }

  // Heruntergeladenes Update installieren und neu starten
  install() {
    if (this.status.state !== 'bereit' || !this.autoUpdater) return false;
    setImmediate(() => this.autoUpdater.quitAndInstall(true, true));
    return true;
  }

  stop() {
    clearInterval(this.timer);
  }
}

module.exports = { Updater, compareVersions, versionFromYaml };
