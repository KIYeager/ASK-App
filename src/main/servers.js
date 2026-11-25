// Entwicklungsserver eines Projekts (ENTSCHEIDE E48): npm run dev, python -m http.server, node server.js usw.
// Sie laufen wie jeder Befehl im geschützten Terminal (Low Integrity, Job-Objekt), aber ohne dass jemand auf
// ihr Ende wartet. Die App merkt sich id, Befehl, Startzeit und die letzten Zeilen der Ausgabe, erkennt darin
// die Adresse (http://localhost:5173 usw.) und stoppt sie über das Terminal. Eine Instanz pro Projekt-Terminal.
const net = require('node:net');
const { EventEmitter } = require('node:events');

const MAX_LINES = 200;
const LINE_CHARS = 2000;
const MAX_SERVERS = 5; // laufende Server pro Projekt
const POLL_MS = 1000;
const READY_WITHOUT_URL_MS = 15000; // läuft er so lange ohne erkannte Adresse, gilt er trotzdem als gestartet
const MAX_MISSES = 5; // so viele Abfragen hintereinander ohne Antwort, dann gilt der Server als ausgefallen
const ACTIVE = new Set(['startet', 'läuft']);

// Farben und Steuerzeichen (Vite, Next.js färben ihre Ausgabe)
const stripAnsi = (text) => String(text).replace(/\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07]*(\x07|\x1b\\)|\x1b[@-Z\\-_]/g, '');

const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '0.0.0.0', '::1', '::', '[::1]', '[::]']);
const validPort = (n) => Number.isInteger(n) && n > 0 && n < 65536;

// Adresse eines lokalen Servers aus einer Zeile der Ausgabe, z. B.
//   «➜  Local:   http://localhost:5173/» (Vite), «- Local: http://localhost:3000» (Next.js),
//   «Serving HTTP on 0.0.0.0 port 8000 (http://0.0.0.0:8000/)» (Python), «Running on http://127.0.0.1:5000» (Flask),
//   «Server listening on port 3000» (Express und eigene Server). Adressen im Netz (Network: http://192.168…) zählen nicht.
// Liefert { url, port } mit localhost bzw. 127.0.0.1, oder null.
function detectAddress(line) {
  const text = stripAnsi(line);
  const urls = text.matchAll(/\bhttps?:\/\/(\[[0-9a-f:]+\]|[a-z0-9.-]+):(\d{1,5})(\/[^\s'"<>)\]]*)?/gi);
  for (const m of urls) {
    const host = m[1].toLowerCase();
    const port = Number(m[2]);
    if (!LOCAL_HOSTS.has(host) && !/^127\.\d+\.\d+\.\d+$/.test(host)) continue;
    if (!validPort(port)) continue;
    const scheme = /^https/i.test(m[0]) ? 'https' : 'http';
    // 0.0.0.0 und :: heissen «alle Adressen»; geöffnet wird der Server auf diesem Rechner
    const name = host === '127.0.0.1' || /^127\./.test(host) ? '127.0.0.1' : 'localhost';
    const rest = (m[3] || '/').replace(/[.,;:]+$/, '') || '/';
    return { url: `${scheme}://${name}:${port}${rest}`, port };
  }
  const serving = /Serving HTTP on (\S+) port (\d{1,5})/i.exec(text);
  if (serving && validPort(Number(serving[2]))) return { url: `http://localhost:${Number(serving[2])}/`, port: Number(serving[2]) };
  const listening = /\b(?:listening|läuft|running|started|lauscht)\b[^\n]{0,30}?\bport\b\s*:?\s*(\d{2,5})\b/i.exec(text)
    || /\blistening (?:on|at)\s+:?(\d{2,5})\b/i.exec(text);
  if (listening && validPort(Number(listening[1]))) return { url: `http://localhost:${Number(listening[1])}/`, port: Number(listening[1]) };
  return null;
}

// Antwortet etwas auf dem Port? (Der Server ist dann wirklich bereit, nicht nur angekündigt.)
function probePort(port, timeout = 800) {
  return new Promise((resolve) => {
    const socket = net.connect({ host: '127.0.0.1', port });
    const done = (ok) => { socket.destroy(); resolve(ok); };
    socket.setTimeout(timeout, () => done(false));
    socket.once('connect', () => done(true));
    socket.once('error', () => {
      // Server, die nur auf ::1 lauschen (localhost unter Windows)
      const v6 = net.connect({ host: '::1', port });
      const end = (ok) => { v6.destroy(); resolve(ok); };
      v6.setTimeout(timeout, () => end(false));
      v6.once('connect', () => end(true));
      v6.once('error', () => end(false));
    });
  });
}

const outputText = (json) => (json.output || []).map((o) => o.data ?? '').join('');

function shortName(command) {
  const c = String(command).replace(/\s+/g, ' ').trim();
  return c.length > 40 ? `${c.slice(0, 39)}…` : c;
}

class Servers extends EventEmitter {
  // request(method, path, body) → { status, json, text }: Terminal-API des Projekts (tools.js request)
  // probe(port) → Promise<bool>; blockedPorts() → Ports, die nie freigegeben werden (die Terminals der App);
  // available() → false, solange das Terminal nicht antwortet (dann wird nicht abgefragt und nichts gezählt)
  constructor({ request, probe = probePort, blockedPorts = () => [], available = () => true, now = () => Date.now(), pollMs = POLL_MS } = {}) {
    super();
    this.request = request;
    this.available = available;
    this.probe = probe;
    this.blockedPorts = blockedPorts;
    this.now = now;
    this.pollMs = pollMs;
    this.servers = new Map(); // id → Eintrag
    this.timer = null;
    this.polling = false;
    this.starting = new Map(); // Befehl → Promise der noch laufenden Starts (reserviert den Platz vor dem ersten await)
  }

  snapshot(s) {
    return { id: s.id, name: s.name, command: s.command, started: s.started, state: s.state, exitCode: s.exitCode,
      url: s.url, port: s.port, ready: s.ready, error: s.error, by: s.by, last: s.lines.at(-1) || '' };
  }

  list() {
    return [...this.servers.values()].map((s) => this.snapshot(s));
  }

  get(id) {
    const s = this.servers.get(String(id));
    return s ? this.snapshot(s) : null;
  }

  output(id, n = MAX_LINES) {
    const s = this.servers.get(String(id));
    if (!s) return [];
    return [...s.lines, ...(s.partial ? [s.partial] : [])].slice(-n);
  }

  running() {
    return [...this.servers.values()].filter((s) => ACTIVE.has(s.state));
  }

  // Ports, die der Browser für das Modell öffnen darf: nur von Servern, die die App gestartet hat und die noch laufen
  allowedPorts() {
    const blocked = new Set((this.blockedPorts() || []).map(Number));
    return new Set(this.running().map((s) => s.port).filter((p) => p && !blocked.has(p)));
  }

  changed() {
    this.emit('changed', this.list());
  }

  // Startet einen Befehl, ohne auf sein Ende zu warten. by: 'person' oder 'modell'
  async start(command, { name = '', by = 'person' } = {}) {
    const cmd = String(command || '').trim();
    if (!cmd) throw new Error('Kein Befehl angegeben.');
    const same = this.running().find((s) => s.command === cmd);
    if (same) return { ...this.snapshot(same), existing: true };
    // Zweiter Start desselben Befehls, während der erste noch läuft: auf dessen Ergebnis warten statt nochmals starten
    const pending = this.starting.get(cmd);
    if (pending) return { ...(await pending), existing: true };
    if (this.running().length + this.starting.size >= MAX_SERVERS) throw new Error(`Es laufen schon ${MAX_SERVERS} Server. Zuerst einen stoppen.`);
    const job = this.launch(cmd, name, by);
    this.starting.set(cmd, job);
    try {
      return await job;
    } finally {
      this.starting.delete(cmd);
    }
  }

  async launch(cmd, name, by) {
    // wait=0: sofort zurück (ohne wait wartet open-terminal bis zu 60 s auf das Ende)
    const res = await this.request('POST', '/execute?wait=0', { command: cmd });
    if (res.status !== 200 || !res.json?.id) {
      const detail = res.json?.detail;
      throw new Error(typeof detail === 'string' ? detail : `Der Server liess sich nicht starten (HTTP ${res.status}).`);
    }
    const s = { id: String(res.json.id), name: String(name || '').trim().slice(0, 60) || shortName(cmd), command: cmd, started: this.now(),
      state: 'startet', exitCode: null, url: '', port: 0, ready: false, error: '', by, lines: [], partial: '', offset: 0, misses: 0, stopping: false };
    this.servers.set(s.id, s);
    await this.apply(s, res.json);
    this.changed();
    this.schedule();
    return this.snapshot(s);
  }

  // Neue Ausgabe und Status aus einer Antwort des Terminals übernehmen
  async apply(s, json) {
    const text = outputText(json);
    if (text) this.addOutput(s, text);
    s.offset = json.next_offset ?? s.offset + (json.output || []).length;
    if (json.status && json.status !== 'running' && ACTIVE.has(s.state)) {
      s.exitCode = json.exit_code ?? null;
      s.state = s.stopping || json.status === 'killed' || s.exitCode === 0 ? 'gestoppt' : 'fehler';
      if (s.state === 'fehler') s.error = `Beendet mit Exitcode ${s.exitCode}`;
      s.ready = false;
      return;
    }
    if (s.state !== 'startet') return;
    if (s.port && !s.ready) {
      const ok = await this.probe(s.port);
      // Während der Probe kann stop() den Server beendet haben: den Zustand nicht überschreiben
      if (s.state !== 'startet' || s.stopping) return;
      if (ok) s.ready = true;
    }
    if (s.ready || (!s.port && this.now() - s.started >= READY_WITHOUT_URL_MS)) s.state = 'läuft';
  }

  addOutput(s, text) {
    const parts = (s.partial + text.replace(/\r\n/g, '\n')).split('\n');
    s.partial = parts.pop();
    // Fortschrittsanzeigen überschreiben ihre Zeile mit \r: nur der letzte Stand zählt
    const clean = (line) => stripAnsi(line.split('\r').filter(Boolean).at(-1) ?? '').slice(0, LINE_CHARS);
    for (const raw of parts) {
      const line = clean(raw);
      if (!s.port) {
        const found = detectAddress(line);
        if (found) { s.url = found.url; s.port = found.port; }
      }
      s.lines.push(line);
    }
    if (s.lines.length > MAX_LINES) s.lines.splice(0, s.lines.length - MAX_LINES);
    if (s.partial.length > LINE_CHARS) { s.lines.push(clean(s.partial)); s.partial = ''; }
    if (!s.port && s.partial) {
      const found = detectAddress(s.partial); // Zeile ohne Zeilenumbruch, z. B. eine Eingabeaufforderung
      if (found) { s.url = found.url; s.port = found.port; }
    }
  }

  schedule() {
    if (this.timer || !this.running().length) return;
    this.timer = setTimeout(() => { this.timer = null; this.poll().catch(() => {}).finally(() => this.schedule()); }, this.pollMs);
  }

  async poll() {
    if (this.polling || !this.available()) return;
    this.polling = true;
    try {
      for (const s of this.running()) {
        const before = JSON.stringify(this.snapshot(s));
        try {
          const res = await this.request('GET', `/execute/${encodeURIComponent(s.id)}/status?wait=0&offset=${s.offset}`);
          if (res.status === 404) {
            s.state = s.stopping ? 'gestoppt' : 'fehler';
            if (!s.stopping) s.error = 'Das Terminal kennt den Server nicht mehr.';
          } else if (res.status === 200 && res.json) {
            s.misses = 0;
            await this.apply(s, res.json);
          } else throw new Error(`HTTP ${res.status}`);
        } catch {
          if (++s.misses >= MAX_MISSES && ACTIVE.has(s.state)) { s.state = 'fehler'; s.error = 'Das Terminal antwortet nicht.'; }
        }
        if (JSON.stringify(this.snapshot(s)) !== before) this.changed();
      }
    } finally {
      this.polling = false;
    }
  }

  // Wartet, bis der Server eine Adresse hat und antwortet, beendet ist oder ms vorbei sind
  async waitReady(id, ms = 8000) {
    const end = this.now() + ms;
    for (;;) {
      const s = this.servers.get(String(id));
      if (!s || !ACTIVE.has(s.state) || s.ready || this.now() >= end) return s ? this.snapshot(s) : null;
      await new Promise((r) => setTimeout(r, Math.min(250, this.pollMs)));
      if (!this.timer && !this.polling) await this.poll();
    }
  }

  async stop(id) {
    const s = this.servers.get(String(id));
    if (!s) return false;
    if (ACTIVE.has(s.state)) {
      s.stopping = true;
      try {
        const res = await this.request('DELETE', `/execute/${encodeURIComponent(s.id)}`);
        if (res.status !== 200 && res.status !== 404) throw new Error(`HTTP ${res.status}`);
      } catch (err) {
        s.stopping = false;
        throw new Error(`Der Server liess sich nicht stoppen: ${err.message}`);
      }
      s.state = 'gestoppt';
      s.ready = false;
    }
    this.changed();
    return true;
  }

  // Beendete Server aus der Liste nehmen
  remove(id) {
    const s = this.servers.get(String(id));
    if (!s || ACTIVE.has(s.state)) return false;
    this.servers.delete(s.id);
    this.changed();
    return true;
  }

  // Alle stoppen (Projekt wird geschlossen, App wird beendet). Fehler zählen nicht: Das Terminal beendet beim
  // Stoppen ohnehin alle Prozesse seines Job-Objekts.
  async stopAll() {
    await Promise.allSettled(this.running().map((s) => this.stop(s.id)));
    clearTimeout(this.timer);
    this.timer = null;
  }

  // Das Terminal wurde beendet oder neu gestartet: Seine Prozesse sind weg
  terminalGone() {
    let any = false;
    for (const s of this.running()) {
      s.state = 'gestoppt';
      s.ready = false;
      s.error = 'Das Terminal wurde beendet.';
      any = true;
    }
    clearTimeout(this.timer);
    this.timer = null;
    if (any) this.changed();
  }
}

module.exports = { Servers, detectAddress, stripAnsi, probePort, MAX_LINES, MAX_SERVERS };
