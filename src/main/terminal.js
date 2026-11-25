// Verwaltung des lokalen Terminals: Port, Key, Start, Health-Check, Neustart.
// Das eigentliche Starten übernimmt ein Backend (backend/*.js), damit sich die Isolationsvariante
// später austauschen lässt.
const crypto = require('node:crypto');
const net = require('node:net');
const http = require('node:http');
const { EventEmitter } = require('node:events');
const NativeLowBackend = require('./backend/native-low');

const PORT_MIN = 20000;
const PORT_MAX = 39999;

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
  constructor() {
    super();
    this.backend = new NativeLowBackend();
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
  // Starts nacheinander: Zwei gleichzeitige Aufrufe (Doppelklick, Adresse und Neustart) liessen sonst
  // einen verwaisten Python-Prozess zurück.
  start(options) {
    this.queue = (this.queue || Promise.resolve()).then(() => this.startNow(options), () => this.startNow(options));
    return this.queue;
  }

  async startNow(options) {
    await this.stop();
    const generation = ++this.generation;
    this.folder = options.folder;
    this.info = null;
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
        const info = await getJson(this.port, '/ask/info', this.key, 1500);
        if (generation !== this.generation) return; // inzwischen neu gestartet oder gestoppt
        this.info = info;
        this.healthFails = 0;
        this.setState('läuft');
        clearInterval(this.healthTimer);
        this.healthTimer = setInterval(() => this.checkHealth(generation), 10000);
        return;
      } catch {
        await new Promise((r) => setTimeout(r, 400));
      }
    }
    throw new Error('Das Terminal antwortet nicht.');
  }

  // Erst nach drei verpassten Antworten in Folge gilt das Terminal als ausgefallen (es kann mit einer
  // grossen Umwandlung beschäftigt sein); antwortet es wieder, läuft es wieder.
  async checkHealth(generation) {
    if (generation !== this.generation) return;
    if (this.state !== 'läuft' && !(this.state === 'fehler' && this.healthDown)) return;
    try {
      await getJson(this.port, '/health', null, 5000);
      if (generation !== this.generation) return;
      this.healthFails = 0;
      if (this.state === 'fehler' && this.healthDown && this.backend.running()) {
        this.healthDown = false;
        this.setState('läuft');
      }
    } catch {
      if (generation !== this.generation || this.state !== 'läuft') return;
      this.healthFails = (this.healthFails || 0) + 1;
      if (this.healthFails >= 3) {
        this.healthDown = true;
        this.setState('fehler', 'Das Terminal antwortet nicht mehr.');
      }
    }
  }

  async stop() {
    this.generation++;
    this.healthDown = false;
    clearInterval(this.healthTimer);
    await this.backend.stop();
    if (this.state !== 'gestoppt') this.setState('gestoppt');
  }
}

module.exports = { Terminal, isPortFree, getJson };
