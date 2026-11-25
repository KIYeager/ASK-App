// Tests für die Entwicklungsserver (src/main/servers.js) mit einem Ersatz für die Terminal-API.
const test = require('node:test');
const assert = require('node:assert');
const { Servers, detectAddress, stripAnsi, MAX_LINES } = require('../src/main/servers');

// Ersatz für open-terminal: Prozesse mit Ausgabe (Liste von Einträgen), Status und Exitcode
function fakeTerminal() {
  const t = { procs: new Map(), calls: [], next: 1 };
  t.add = (id, data) => t.procs.get(id).output.push({ type: 'stdout', data });
  t.request = async (method, path, body) => {
    t.calls.push(`${method} ${path}`);
    if (method === 'POST' && path.startsWith('/execute')) {
      const id = `p${t.next++}`;
      t.procs.set(id, { command: body.command, output: [], status: 'running', exit_code: null });
      return { status: 200, json: { id, status: 'running', exit_code: null, output: [], next_offset: 0 } };
    }
    const m = /^\/execute\/([^/?]+)(\/status\?wait=0&offset=(\d+))?$/.exec(path);
    const p = m && t.procs.get(m[1]);
    if (!p) return { status: 404, json: { detail: 'Process not found' } };
    if (method === 'DELETE') {
      p.status = 'killed';
      p.exit_code = -1;
      return { status: 200, json: { status: 'killed' } };
    }
    const offset = Number(m[3] || 0);
    return { status: 200, json: { id: m[1], status: p.status, exit_code: p.exit_code, output: p.output.slice(offset), next_offset: p.output.length } };
  };
  return t;
}

const make = (t, extra = {}) => new Servers({ request: t.request, probe: async () => true, pollMs: 10, ...extra });

test('Adresse aus der Ausgabe üblicher Entwicklungsserver', () => {
  const cases = [
    ['  \x1b[32m➜\x1b[39m  \x1b[1mLocal\x1b[22m:   \x1b[36mhttp://localhost:\x1b[1m5173\x1b[22m/\x1b[39m', 'http://localhost:5173/', 5173],
    ['   - Local:        http://localhost:3000', 'http://localhost:3000/', 3000],
    ['Serving HTTP on 0.0.0.0 port 8000 (http://0.0.0.0:8000/) ...', 'http://localhost:8000/', 8000],
    ['Serving HTTP on :: port 8080 (http://[::]:8080/) ...', 'http://localhost:8080/', 8080],
    [' * Running on http://127.0.0.1:5000', 'http://127.0.0.1:5000/', 5000],
    ['Server läuft auf http://127.0.0.1:8000/index.html.', 'http://127.0.0.1:8000/index.html', 8000],
    ['Server listening on port 4000', 'http://localhost:4000/', 4000],
    ['Listening on :3001', 'http://localhost:3001/', 3001],
    ['App running at https://localhost:8443/app', 'https://localhost:8443/app', 8443],
  ];
  for (const [line, url, port] of cases) assert.deepStrictEqual(detectAddress(line), { url, port }, line);
  for (const line of ['  ➜  Network: http://192.168.1.20:5173/', 'Siehe https://vitejs.dev/config', 'VITE v6.3.1  ready in 412 ms', 'Port 99999 is in use',
    'http://example.com:8080/']) {
    assert.strictEqual(detectAddress(line), null, line);
  }
  assert.strictEqual(stripAnsi('\x1b[1mfett\x1b[22m'), 'fett');
});

test('Start: Adresse erkannt, Zustand startet → läuft, Ausgabe gekürzt, Stopp über DELETE', async () => {
  const t = fakeTerminal();
  const s = make(t);
  const changes = [];
  s.on('changed', (list) => changes.push(list));
  const started = await s.start('npm run dev', { name: 'Webseite', by: 'modell' });
  assert.strictEqual(started.state, 'startet');
  assert.strictEqual(started.name, 'Webseite');
  assert.ok(t.calls[0].startsWith('POST /execute?wait=0'), 'ohne Warten starten');
  t.add(started.id, '\n  VITE v6.3.1  ready in 412 ms\n\n');
  t.add(started.id, '  ➜  Local:   http://localhost:5173/\n  ➜  Network: use --host to expose\n');
  const ready = await s.waitReady(started.id, 2000);
  assert.strictEqual(ready.state, 'läuft');
  assert.strictEqual(ready.url, 'http://localhost:5173/');
  assert.strictEqual(ready.port, 5173);
  assert.ok(ready.ready);
  assert.deepStrictEqual([...s.allowedPorts()], [5173]);
  assert.ok(changes.length >= 2);

  for (let i = 0; i < 250; i++) t.add(started.id, `Zeile ${i}\n`);
  await s.poll();
  const out = s.output(started.id);
  assert.strictEqual(out.length, MAX_LINES);
  assert.strictEqual(out.at(-1), 'Zeile 249');

  // Derselbe Befehl läuft schon: kein zweiter Start
  const again = await s.start('npm run dev');
  assert.strictEqual(again.id, started.id);
  assert.ok(again.existing);

  assert.ok(await s.stop(started.id));
  assert.ok(t.calls.includes(`DELETE /execute/${started.id}`));
  assert.strictEqual(s.get(started.id).state, 'gestoppt');
  assert.strictEqual(s.allowedPorts().size, 0, 'gestoppte Server geben keinen Port frei');
  await s.stopAll();
});

test('Server beendet sich selbst: Exitcode, fehler bei Code ungleich 0', async () => {
  const t = fakeTerminal();
  const s = make(t);
  const a = await s.start('node server.js');
  t.add(a.id, "Error: Cannot find module 'express'\n");
  Object.assign(t.procs.get(a.id), { status: 'done', exit_code: 1 });
  await s.poll();
  const snap = s.get(a.id);
  assert.strictEqual(snap.state, 'fehler');
  assert.strictEqual(snap.exitCode, 1);
  assert.match(snap.last, /Cannot find module/);
  const b = await s.start('python -m http.server');
  Object.assign(t.procs.get(b.id), { status: 'done', exit_code: 0 });
  await s.poll();
  assert.strictEqual(s.get(b.id).state, 'gestoppt');
  assert.ok(s.remove(a.id));
  assert.strictEqual(s.list().length, 1);
  await s.stopAll();
});

test('Port erst freigegeben, wenn der Server antwortet; Ports der Terminals bleiben gesperrt', async () => {
  const t = fakeTerminal();
  let open = false;
  const s = make(t, { probe: async () => open, blockedPorts: () => [24567] });
  const a = await s.start('python -m http.server 8000');
  t.add(a.id, 'Serving HTTP on 0.0.0.0 port 8000 (http://0.0.0.0:8000/) ...\n');
  await s.poll();
  assert.strictEqual(s.get(a.id).state, 'startet');
  assert.strictEqual(s.get(a.id).url, 'http://localhost:8000/');
  open = true;
  await s.poll();
  assert.strictEqual(s.get(a.id).state, 'läuft');
  const b = await s.start('python fake.py');
  t.add(b.id, 'Server listening on port 24567\n');
  await s.poll();
  assert.deepStrictEqual([...s.allowedPorts()].sort(), [8000]);
  await s.stopAll();
});

test('Ohne erkennbare Adresse gilt ein laufender Server nach einer Weile als gestartet', async () => {
  const t = fakeTerminal();
  let clock = 1000;
  const s = make(t, { now: () => clock });
  const a = await s.start('node bot.js');
  await s.poll();
  assert.strictEqual(s.get(a.id).state, 'startet');
  clock += 20000;
  await s.poll();
  assert.strictEqual(s.get(a.id).state, 'läuft');
  assert.strictEqual(s.get(a.id).url, '');
  assert.strictEqual(s.allowedPorts().size, 0);
  await s.stopAll();
});

test('Terminal beendet oder kennt den Prozess nicht mehr', async () => {
  const t = fakeTerminal();
  const s = make(t);
  const a = await s.start('npm start');
  t.procs.delete(a.id);
  await s.poll();
  assert.strictEqual(s.get(a.id).state, 'fehler');
  const b = await s.start('npm run dev');
  s.terminalGone();
  assert.strictEqual(s.get(b.id).state, 'gestoppt');
  assert.strictEqual(s.running().length, 0);
});

test('Ausgabe mit \\r-Fortschritt und über Stücke verteilten Zeilen', async () => {
  const t = fakeTerminal();
  const s = make(t);
  const a = await s.start('npm run dev');
  t.add(a.id, 'lädt 10%\rlädt 50%\rlädt 100%\r\nServer läuft auf http://loc');
  await s.poll();
  t.add(a.id, 'alhost:3000\n');
  await s.poll();
  assert.deepStrictEqual(s.output(a.id), ['lädt 100%', 'Server läuft auf http://localhost:3000']);
  assert.strictEqual(s.get(a.id).port, 3000);
  await s.stopAll();
});

test('Höchstens fünf laufende Server, leerer Befehl abgelehnt', async () => {
  const t = fakeTerminal();
  const s = make(t);
  for (let i = 0; i < 5; i++) await s.start(`node s${i}.js`);
  await assert.rejects(s.start('node s5.js'), /schon 5 Server/);
  await assert.rejects(s.start('  '), /Kein Befehl/);
  await s.stopAll();
  assert.strictEqual(s.running().length, 0);
});

test('Probe auf den Port: offen und geschlossen', async () => {
  const { probePort } = require('../src/main/servers');
  const srv = require('node:net').createServer((c) => c.end());
  await new Promise((r) => srv.listen(0, '127.0.0.1', r));
  const { port } = srv.address();
  try {
    assert.strictEqual(await probePort(port), true);
  } finally {
    await new Promise((r) => srv.close(r));
  }
  assert.strictEqual(await probePort(port), false);
});

test('Werkzeuge server_starten, server_liste, server_stoppen', async () => {
  const { runTool, toolSpecs } = require('../src/main/tools');
  const t = fakeTerminal();
  const names = toolSpecs().map((s) => s.function.name);
  for (const n of ['server_starten', 'server_stoppen', 'server_liste']) assert.ok(names.includes(n), n);
  const plain = t.request;
  // Vite meldet seine Adresse gleich nach dem Start
  const servers = make(t, { request: async (method, p, body) => {
    const res = await plain(method, p, body);
    if (method === 'POST') t.add(res.json.id, '  ➜  Local:   http://localhost:5173/\n');
    return res;
  } });
  const r = await runTool(null, 'server_starten', { befehl: 'npm run dev', name: 'Webseite' }, { servers });
  assert.ok(r.ok, r.output);
  assert.match(r.output, /p1 «Webseite»: läuft, http:\/\/localhost:5173\//);
  assert.match(r.output, /browser \(ziel: http:\/\/localhost:5173\/\)/);
  const list = await runTool(null, 'server_liste', {}, { servers });
  assert.match(list.output, /Local:\s+http:\/\/localhost:5173/);
  assert.strictEqual(list.summary, '1 läuft');
  const blocked = await runTool(null, 'server_starten', { befehl: 'start index.html' }, { servers });
  assert.strictEqual(blocked.summary, 'abgelehnt');
  const stopped = await runTool(null, 'server_stoppen', { id: 'p1' }, { servers });
  assert.ok(stopped.ok);
  assert.strictEqual(servers.get('p1').state, 'gestoppt');
  const none = await runTool(null, 'server_starten', { befehl: 'npm run dev' }, {});
  assert.match(none.output, /nicht verfügbar/);
});

test('stop() während der Probe: apply überschreibt gestoppt nicht mit läuft', async () => {
  const t = fakeTerminal();
  let release;
  const gate = new Promise((r) => { release = r; });
  const s = make(t, { probe: () => gate.then(() => true) });
  const started = await s.start('npm run dev');
  t.add(started.id, '  ➜  Local:   http://localhost:5173/\n');
  const polling = s.poll(); // bleibt in der Probe hängen
  await new Promise((r) => setTimeout(r, 20));
  assert.ok(await s.stop(started.id));
  assert.strictEqual(s.get(started.id).state, 'gestoppt');
  release();
  await polling;
  assert.strictEqual(s.get(started.id).state, 'gestoppt');
  assert.strictEqual(s.get(started.id).ready, false);
  assert.strictEqual(s.allowedPorts().size, 0);
  await s.stopAll();
});

test('Zwei gleichzeitige Starts desselben Befehls starten nur einen Prozess', async () => {
  const t = fakeTerminal();
  const s = make(t);
  const [a, b] = await Promise.all([s.start('node a.js'), s.start('node a.js')]);
  assert.strictEqual(a.id, b.id);
  assert.ok(a.existing || b.existing);
  assert.strictEqual(t.calls.filter((c) => c.startsWith('POST /execute')).length, 1);
  assert.strictEqual(s.running().length, 1);
  // Gleichzeitige Starts zählen auch gegen die Höchstzahl
  const many = await Promise.allSettled(Array.from({ length: 7 }, (_, i) => s.start(`node s${i}.js`)));
  assert.strictEqual(many.filter((r) => r.status === 'rejected').length, 3);
  assert.strictEqual(s.running().length, 5);
  await s.stopAll();
});

test('Ein Fehler in einem Zuhörer löst keine unbehandelte Ablehnung aus', async () => {
  const t = fakeTerminal();
  const s = make(t);
  const rejected = [];
  const onRejected = (err) => rejected.push(err);
  process.on('unhandledRejection', onRejected);
  try {
    const started = await s.start('node a.js');
    s.on('changed', () => { throw new Error('Zuhörer kaputt'); });
    t.add(started.id, 'Zeile\n');
    await new Promise((r) => setTimeout(r, 80));
    await new Promise((r) => setImmediate(r));
    assert.deepStrictEqual(rejected, []);
    assert.strictEqual(s.polling, false);
  } finally {
    process.off('unhandledRejection', onRejected);
    s.removeAllListeners('changed');
    await s.stopAll();
  }
});
