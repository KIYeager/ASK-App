// Updates aus dem öffentlichen Release-Repository (ENTSCHEIDE E37, E40). Die installierte App fragt einmal
// vor dem Herunterladen, zeigt den Download als Balken oben in der App und startet danach mit 10 s Countdown
// von selbst neu («Jetzt» oder «Später» im Balken); die einzelne EXE kann sich nicht selbst ersetzen und zeigt
// nur einen Hinweis mit Link zur neuen Version. Mit «Beta-Versionen erhalten» kommen auch
// Vorabversionen (Tags wie v0.3.0-beta.1). Während das Modell arbeitet, fragt die App nie.
const { EventEmitter } = require('node:events');

const FIRST_CHECK_MS = 20 * 1000;
const CHECK_EVERY_MS = 6 * 60 * 60 * 1000;
const NOTES_MAX = 700;
const RESTART_SECONDS = 10; // Countdown nach dem Download, bevor ASK für das Update neu startet

const releaseUrl = (config, version) => `https://github.com/${config.owner}/${config.repo}/releases/tag/v${version}`;

// Release-Text für den Dialog: ohne HTML, gekürzt. electron-updater liefert Text oder eine Liste.
function notesText(notes) {
  const raw = Array.isArray(notes) ? notes.map((n) => n?.note || '').join('\n') : String(notes || '');
  const text = raw
    .replace(/<\/(p|li|h\d)>|<br\s*\/?>/gi, '\n')
    .replace(/<li[^>]*>/gi, '• ')
    .replace(/<[^>]+>/g, '')
    .replace(/&amp;/g, '&').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'")
    .replace(/[ \t]+/g, ' ')
    .replace(/\n\s*\n+/g, '\n')
    .trim();
  return text.length > NOTES_MAX ? `${text.slice(0, NOTES_MAX).trimEnd()} …` : text;
}

const isBeta = (version) => /-/.test(String(version || ''));

class Updater extends EventEmitter {
  // app: Electron-App; config: { owner, repo } aus app-config.json; portable: läuft aus der einzelnen EXE;
  // beta(): Beta-Versionen gewünscht; busy(): Modell arbeitet; ask(options): Dialog, gibt den Knopf-Index;
  // openExternal(url); beforeInstall(): Terminal sauber beenden; updater: Ersatz für Tests.
  constructor({ app, config, portable = false, beta = () => false, busy = () => false, ask, openExternal, beforeInstall = async () => {}, updater = null, log = () => {},
    timers = { setInterval, clearInterval } }) {
    super();
    this.app = app;
    this.config = config && config.owner && config.repo ? config : null;
    this.portable = portable;
    this.beta = beta;
    this.busy = busy;
    this.ask = ask;
    this.openExternal = openExternal;
    this.beforeInstall = beforeInstall;
    this.log = log;
    this.enabled = !!(this.config && app?.isPackaged);
    this._updater = updater;
    this.state = this.enabled ? 'aktuell' : 'aus';
    this.info = null; // updateInfo der neuen Version
    this.percent = 0;
    this.error = '';
    this.dismissed = ''; // «Später» für diese Version: bis zum nächsten Start nicht mehr von selbst fragen
    this.pending = ''; // 'angebot' oder 'neustart', sobald das Modell fertig ist
    this.dialogOpen = false;
    this.timers = [];
    this.clock = timers;
    this.countdown = 0; // Sekunden bis zum Neustart (Zustand «neustart»)
    this.countdownTimer = null;
    this.postponed = false; // «Später» nach dem Download: beim Schliessen installieren
  }

  get updater() {
    if (this._updater) return this._updater;
    const { autoUpdater } = require('electron-updater');
    autoUpdater.autoDownload = false;
    autoUpdater.autoInstallOnAppQuit = true; // greift erst, wenn ein Update heruntergeladen ist
    autoUpdater.logger = { info: (m) => this.log(m), warn: (m) => this.log(m), error: (m) => this.log(m), debug: () => {} };
    // Die einzelne EXE hat kein app-update.yml (nur der Installer bekommt eins); sie prüft nur.
    if (this.portable) autoUpdater.setFeedURL({ provider: 'github', owner: this.config.owner, repo: this.config.repo });
    autoUpdater.on('download-progress', (p) => {
      this.percent = Math.round(p.percent || 0);
      this.emit('changed');
    });
    this._updater = autoUpdater;
    return autoUpdater;
  }

  snapshot() {
    return {
      state: this.state,
      current: this.app?.getVersion?.() || '',
      version: this.info?.version || '',
      percent: this.percent,
      error: this.error,
      portable: this.portable,
      beta: !!this.beta(),
      enabled: this.enabled,
      countdown: this.countdown,
      postponed: this.postponed,
    };
  }

  set(state, extra = {}) {
    Object.assign(this, { state, ...extra });
    this.emit('changed');
  }

  start() {
    if (!this.enabled) return;
    const first = setTimeout(() => this.check(), FIRST_CHECK_MS);
    const every = setInterval(() => this.check(), CHECK_EVERY_MS);
    first.unref?.();
    every.unref?.();
    this.timers.push(first, every);
  }

  stop() {
    for (const t of this.timers) clearTimeout(t);
    this.timers = [];
  }

  // manual: von der Person ausgelöst (Menü, /update, Klick unten): dann auch «kein Update» und Fehler melden.
  async check({ manual = false } = {}) {
    if (!this.enabled) {
      if (manual) await this.dialog({ type: 'info', message: 'Updates gibt es nur in der installierten App oder der ASK-EXE.', buttons: ['OK'] });
      return;
    }
    if (this.state === 'lädt' || this.state === 'neustart') return;
    if (this.state === 'geladen') {
      if (manual) await this.restartNow();
      return;
    }
    const hadOffer = this.state === 'verfügbar' && !!this.info;
    this.set('prüft', { error: '' });
    let result;
    try {
      this.updater.allowPrerelease = !!this.beta();
      result = await this.updater.checkForUpdates();
    } catch (err) {
      this.set('fehler', { error: String(err?.message || err).split('\n')[0] });
      if (manual) {
        await this.dialog({ type: 'warning', message: 'Die Suche nach Updates hat nicht geklappt.',
          detail: `Ist der Laptop mit dem Internet verbunden?\n\n${this.error}`, buttons: ['OK'] });
      }
      return;
    }
    if (result?.isUpdateAvailable && result.updateInfo?.version) {
      this.info = result.updateInfo;
      this.set('verfügbar');
      if (manual || this.dismissed !== this.info.version) await this.offer();
      return;
    }
    this.set(hadOffer ? 'verfügbar' : 'aktuell');
    if (manual) {
      await this.dialog({ type: 'info', message: `ASK ${this.app.getVersion()} ist die neueste Version.`,
        detail: this.beta() ? 'Beta-Versionen sind eingeschaltet.' : '', buttons: ['OK'] });
    }
  }

  // Klick auf «Update» in der Statuszeile oder «Jetzt neu starten» im Balken
  async action() {
    if (this.state === 'verfügbar') return this.offer();
    if (this.state === 'geladen' || this.state === 'neustart') return this.restartNow();
    if (this.state === 'lädt') return undefined;
    return this.check({ manual: true });
  }

  // Das Modell arbeitet wieder: einen laufenden Countdown anhalten, Neustart nach dem Auftrag.
  busyChanged() {
    if (this.busy() && this.state === 'neustart') {
      this.stopCountdown();
      this.pending = 'neustart';
      this.set('geladen');
    }
  }

  // Das Modell ist fertig: aufgeschobene Frage oder aufgeschobenen Neustart nachholen.
  idle() {
    const pending = this.pending;
    this.pending = '';
    if (pending === 'angebot' && this.state === 'verfügbar') this.offer();
    if (pending === 'neustart' && this.state === 'geladen') this.scheduleRestart();
  }

  async offer() {
    if (!this.info) return;
    if (this.busy()) {
      this.pending = 'angebot';
      return;
    }
    const { version } = this.info;
    const label = isBeta(version) ? `ASK ${version} (Beta)` : `ASK ${version}`;
    const notes = notesText(this.info.releaseNotes);
    if (this.portable) {
      const res = await this.dialog({ type: 'info', message: `${label} ist da.`,
        detail: `Du hast ${this.app.getVersion()}. Diese ASK-EXE aktualisiert sich nicht selbst: Lade die neue EXE herunter und starte sie. Chats und Einstellungen bleiben.${notes ? `\n\nNeu:\n${notes}` : ''}`,
        buttons: ['Download öffnen', 'Später'], defaultId: 0, cancelId: 1 });
      if (res === 0) this.openExternal(releaseUrl(this.config, version));
      else if (res === 1) this.dismissed = version;
      return;
    }
    const res = await this.dialog({ type: 'question', message: `${label} ist da. Jetzt aktualisieren?`,
      detail: `Du hast ${this.app.getVersion()}. ASK lädt das Update im Hintergrund (Balken oben in der App), du kannst weiterarbeiten. Danach startet ASK von selbst neu, ausser während eines Auftrags. Chats, Ordner und Einstellungen bleiben.${notes ? `\n\nNeu:\n${notes}` : ''}`,
      buttons: ['Jetzt aktualisieren', 'Später'], defaultId: 0, cancelId: 1 });
    if (res === 1) this.dismissed = version;
    if (res !== 0) return;
    await this.download();
  }

  async download() {
    this.set('lädt', { percent: 0, postponed: false });
    try {
      await this.updater.downloadUpdate();
    } catch (err) {
      this.set('verfügbar', { error: String(err?.message || err).split('\n')[0] });
      await this.dialog({ type: 'warning', message: 'Das Update konnte nicht heruntergeladen werden.',
        detail: `Später nochmals versuchen (unten auf «Update» klicken).\n\n${this.error}`, buttons: ['OK'] });
      return;
    }
    this.set('geladen', { percent: 100 });
    this.scheduleRestart();
  }

  // Nach dem Download: Countdown, dann Neustart. Während eines Auftrags erst danach.
  scheduleRestart() {
    if (this.postponed || this.state !== 'geladen') return;
    if (this.busy()) {
      this.pending = 'neustart';
      return;
    }
    this.stopCountdown();
    this.set('neustart', { countdown: RESTART_SECONDS });
    this.countdownTimer = this.clock.setInterval(() => {
      if (this.busy()) return this.busyChanged();
      this.countdown -= 1;
      if (this.countdown <= 0) return this.restartNow();
      this.emit('changed');
      return undefined;
    }, 1000);
  }

  stopCountdown() {
    if (this.countdownTimer) this.clock.clearInterval(this.countdownTimer);
    this.countdownTimer = null;
    this.countdown = 0;
  }

  // «Später» im Balken: nicht mehr von selbst neu starten; das Update installiert sich beim Schliessen.
  postpone() {
    if (this.state !== 'neustart' && this.state !== 'geladen') return;
    this.stopCountdown();
    this.pending = '';
    this.set('geladen', { postponed: true });
  }

  async restartNow() {
    this.stopCountdown();
    if (this.busy()) { // ein Auftrag läuft: danach
      this.pending = 'neustart';
      this.set('geladen');
      return;
    }
    this.set('installiert');
    await this.beforeInstall();
    // Nicht still: Der Installer zeigt seinen Fortschritt und startet ASK danach wieder. Still sah es aus,
    // als käme ASK nicht zurück, und ein Klick auf die Verknüpfung während des Installierens bot an, sie zu löschen.
    this.updater.quitAndInstall(false, true);
  }

  // Nie zwei Dialoge übereinander
  async dialog(options) {
    if (this.dialogOpen) return -1;
    this.dialogOpen = true;
    try {
      return await this.ask(options);
    } finally {
      this.dialogOpen = false;
    }
  }
}

module.exports = { Updater, notesText, releaseUrl, isBeta, FIRST_CHECK_MS, CHECK_EVERY_MS, RESTART_SECONDS };
