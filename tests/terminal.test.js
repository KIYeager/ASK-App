// Tests für Start, Stopp und Überwachung des Terminals (src/main/terminal.js) mit einem Ersatz-Backend.
const test = require('node:test');
const assert = require('node:assert');
const http = require('node:http');
const { Terminal } = require('../src/main/terminal');

// Ersatz für backend/native-low.js: ein kleiner HTTP-Server, Start und Stopp dauern einen Moment
function fakeBackend({ lowIntegrity = true } = {}) {
  const b = { server: null, starts: [], alive: 0, maxAlive: 0, healthy: true };
  b.running = () => !!b.server;
  b.start = async ({ workdir, port }) => {
    await new Promise((r) => setTimeout(r, 30));
    const server = http.createServer((req, res) => {
      if (!b.healthy && req.url === '/health') { req.socket.destroy(); return; }
      res.setHeader('Content-Type', 'application/json');
      res.end(JSON.stringify({ low_integrity: lowIntegrity, workdir }));
    });
    await new Promise((resolve, reject) => { server.once('error', reject); server.listen(port, '127.0.0.1', resolve); });
    b.server = server;
    b.starts.push(workdir);
    b.alive++;
    b.maxAlive = Math.max(b.maxAlive, b.alive);
  };
  b.stop = async () => {
    const server = b.server;
    if (!server) return;
    b.server = null;
    await new Promise((r) => setTimeout(r, 30));
    server.closeAllConnections();
    await new Promise((r) => server.close(r));
    b.alive--;
  };
  return b;
}

test('Mehrere Starts kurz hintereinander: nur der letzte läuft, kein Terminal bleibt übrig', async () => {
  const backend = fakeBackend();
  const t = new Terminal(backend);
  try {
    await Promise.all([t.start({ folder: 'A' }), t.start({ folder: 'B' }), t.start({ folder: 'C' })]);
    assert.strictEqual(t.state, 'läuft');
    assert.strictEqual(t.folder, 'C');
    assert.deepStrictEqual(backend.starts, ['C']);
    await t.start({ folder: 'D' });
    t.start({ folder: 'E' }); // gleich wieder gestoppt
    await t.stop();
    assert.strictEqual(t.state, 'gestoppt');
    assert.strictEqual(backend.alive, 0);
    assert.strictEqual(backend.maxAlive, 1);
  } finally {
    await t.stop();
  }
});

test('Terminal ohne niedrige Integritätsstufe wird nicht freigegeben', async () => {
  const t = new Terminal(fakeBackend({ lowIntegrity: false }));
  try {
    await t.start({ folder: 'A' });
    assert.strictEqual(t.state, 'fehler');
    assert.match(t.error, /niedriger Integritätsstufe/);
  } finally {
    await t.stop();
  }
});

test('Überwachung: eine verpasste Probe ist kein Ausfall, nach drei gilt es als ausgefallen und erholt sich', async () => {
  const backend = fakeBackend();
  const t = new Terminal(backend);
  try {
    await t.start({ folder: 'A' });
    const generation = t.generation;
    backend.healthy = false;
    await t.checkHealth(generation);
    await t.checkHealth(generation);
    assert.strictEqual(t.state, 'läuft');
    await t.checkHealth(generation);
    assert.strictEqual(t.state, 'fehler');
    backend.healthy = true;
    await t.checkHealth(generation);
    assert.strictEqual(t.state, 'läuft');
  } finally {
    await t.stop();
  }
});
