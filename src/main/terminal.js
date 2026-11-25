// Verwaltung des lokalen Terminals: Port, Key, Start, Health-Check, Neustart.
// Das eigentliche Starten übernimmt ein Backend (backend/*.js), damit sich die Isolationsvariante
// später austauschen lässt.
const crypto = require('node:crypto');
const net = require('node:net');
const http = require('node:http');
const { EventEmitter } = require('node:events');

const PORT_MIN = 20000;
const PORT_MAX = 39999;
const HEALTH_MISSES = 3; // so viele Proben hintereinander ohne Antwort, bevor das Terminal als ausgefallen gilt

function isPortFree(port) {
  return new Promise((resolve) => {
    const srv = net.createServer();
    srv.once('error', () => resolve(false));
    srv.listen(port, '127.0.0.1', () => srv.close(() => resolve(true)));
  });
}

async function choosePort(preferred) {
  if (preferred && await isPortFree(preferred)) return preferred;
  for (let i = 0; i < 50; i++) {
    const port = PORT_MIN + crypto.randomInt(PORT_MAX - PORT_MIN);
    if (await isPortFree(port)) return port;
  }
  throw new Error('Kein freier Port gefunden');
}

function getJson(port, path, key, timeout = 3000) {
  return new Promise((resolve, reject) => {
    const req = http.get({ host: '127.0.0.1', port, path, timeout,
      headers: key ? { Authorization: `Bearer ${key}` } : {} }, (res) => {
      let body = '';
      res.setEncoding('utf8');
      res.on('data', (c) => { body += c; });
      res.on('end', () => {
        if (res.statusCode !== 200) return reject(new Error(`HTTP ${res.statusCode}`));
        try { resolve(JSON.parse(body)); } catch (e) { reject(e); }
      });
    });
    req.on('timeout', () => req.destroy(new Error('Zeitüberschreitung')));
    req.on('error', reject);
  });
}

class Terminal extends EventEmitter {
  // backend: nur für Tests; sonst das Terminal mit Low Integrity
  constructor(backend = null) {
    super();
    this.backend = backend || new (require('./backend/native-low'))();
    this.queue = Promise.resolve(); // Starten und Stoppen laufen streng nacheinander
    this.misses = 0;
    this.unhealthy = false;
    this.state = 'gestoppt'; // gestoppt | startet | läuft | fehler
    this.error = '';
    this.port = 0;
    this.key = '';
    this.folder = '';
    this.info = null;
    this.logs = [];
    this.healthTimer = null;
    this.generation = 0;
  }

  url() {
    return this.port ? `http://127.0.0.1:${this.port}` : '';
  }

  setState(state, error = '') {
    this.state = state;
    this.error = error;
    this.emit('state', this.snapshot());
  }

  snapshot() {
    return { state: this.state, error: this.error, port: this.port, url: this.url(), folder: this.folder,
      lowIntegrity: this.info?.low_integrity ?? null };
  }

  log(line) {
    this.logs.push(line);
    if (this.logs.length > 300) this.logs.shift();
  }

  // options: { folder, preferredPort, origin, systemPrompt, executeDescription }
  // Mehrere Aufrufe kurz hintereinander (Doppelklick auf «Neu starten», zwei Ordnerwechsel) laufen
  // nacheinander, und nur der letzte startet wirklich: Sonst bleibt ein Terminal übrig, von dem die App
  // nichts mehr weiss, oder das Terminal läuft in einem anderen Ordner, als die App anzeigt.
  start(options) {
    const generation = ++this.generation;
    clearInterval(this.healthTimer);
    const run = this.queue.then(() => this.startNow(options, generation));
    this.queue = run.catch(() => {});
    return run;
  }

  async startNow(options, generation) {
    if (generation !== this.generation) return;
    await this.backend.stop();
    if (generation !== this.generation) return;
    this.folder = options.folder;
    this.info = null;
    this.misses = 0;
    this.unhealthy = false;
    this.setState('startet');
    try {
      this.port = await choosePort(options.preferredPort);
      this.key = crypto.randomBytes(32).toString('base64url');
      this.emit('port', this.port);
      await this.backend.start({
        workdir: options.folder,
        port: this.port,
        key: this.key,
        origin: options.origin,
        systemPrompt: options.systemPrompt,
        executeDescription: options.executeDescription,
        onEvent: (ev) => this.onBackendEvent(ev, generation),
      });
      await this.waitHealthy(generation);
    } catch (err) {
      if (generation !== this.generation) return;
      this.log(`Start fehlgeschlagen: ${err.message}`);
      await this.backend.stop();
      if (generation !== this.generation) return;
      this.setState('fehler', this.error || err.message);
    }
  }

  onBackendEvent(ev, generation) {
    if (generation !== this.generation) return;
    if (ev.event === 'log') this.log(ev.line);
    else this.log(JSON.stringify(ev));
    if (ev.event === 'error') {
      this.error = ev.code === 'label'
        ? `Der Ordner konnte nicht geschützt werden (${ev.message}). Bitte einen Ordner auf der internen Festplatte unter dem eigenen Benutzerprofil wählen.`
        : ev.message;
    }
    if (ev.event === 'exit' || ev.event === 'process-exit') {
      clearInterval(this.healthTimer);
      this.unhealthy = false;
      if (this.state === 'läuft' || this.state === 'startet') {
        this.setState('fehler', this.error || 'Das Terminal wurde unerwartet beendet.');
      }
    }
  }

  async waitHealthy(generation) {
    const deadline = Date.now() + 60000;
    while (Date.now() < deadline) {
      if (generation !== this.generation) return;
      if (this.state === 'fehler') throw new Error(this.error);
      if (!this.backend.running()) throw new Error(this.error || 'Das Terminal hat sich beim Start beendet.');
      try {
        this.info = await getJson(this.port, '/ask/info', this.key, 1500);
      } catch {
        await new Promise((r) => setTimeout(r, 400));
        continue;
      }
      if (generation !== this.generation) return;
      // Der Starter bricht ab, wenn er das Terminal nicht schützen kann; die App prüft es trotzdem selbst.
      if (this.info.low_integrity !== true) throw new Error('Das Terminal läuft nicht mit niedriger Integritätsstufe und wurde darum nicht freigegeben.');
      this.setState('läuft');
      this.healthTimer = setInterval(() => this.checkHealth(generation), 10000);
      return;
    }
    throw new Error('Das Terminal antwortet nicht.');
  }

  // Eine einzelne verpasste Probe (Laptop war kurz im Ruhezustand, ein Befehl lastet den Rechner aus) ist kein
  // Ausfall. Antwortet das Terminal später wieder, gilt es wieder als laufend, ohne dass die Person neu starten muss.
  async checkHealth(generation) {
    if (generation !== this.generation || (this.state !== 'läuft' && !this.unhealthy)) return;
    try {
      await getJson(this.port, '/health', null, 3000);
      if (generation !== this.generation) return;
      this.misses = 0;
      if (this.unhealthy && this.backend.running()) {
        this.unhealthy = false;
        this.setState('läuft');
      }
    } catch {
      if (generation !== this.generation || this.state !== 'läuft') return;
      if (++this.misses >= HEALTH_MISSES) {
        this.unhealthy = true;
        this.setState('fehler', 'Das Terminal antwortet nicht mehr.');
      }
    }
  }

  async stop() {
    const generation = ++this.generation;
    clearInterval(this.healthTimer);
    this.unhealthy = false;
    const run = this.queue.then(() => this.backend.stop());
    this.queue = run.catch(() => {});
    await run;
    if (generation === this.generation && this.state !== 'gestoppt') this.setState('gestoppt');
  }
}

module.exports = { Terminal, isPortFree, getJson };
