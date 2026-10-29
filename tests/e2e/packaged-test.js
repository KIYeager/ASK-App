// Test der gepackten oder installierten App, von aussen gesteuert über das DevTools-Protokoll.
// Aufruf: node tests/e2e/packaged-test.js <pfad zu ASK.exe> <testordner-basis>
// Sichert vorhandene Einstellungen in %APPDATA%\ASK und stellt sie am Ende wieder her.
const fs = require('node:fs');
const os = require('node:os');
const net = require('node:net');
const http = require('node:http');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');

const exe = path.resolve(process.argv[2]);
const base = process.argv[3] || path.join(os.homedir(), 'ASK-Test-paket');
const pythonDir = path.join(path.dirname(exe), 'resources', 'python').toLowerCase();
const settingsFile = path.join(process.env.APPDATA, 'ASK', 'einstellungen.json');
const workdir = path.join(base, 'ordner-paket');
const mockPort = 18800 + Math.floor(Math.random() * 90);
const mockUrl = `http://127.0.0.1:${mockPort}`;
const debugPort = 9333;
const checks = [];
const check = (name, ok, detail = '') => {
  checks.push({ name, ok: !!ok, detail });
  console.log(`${ok ? 'OK  ' : 'FEHLER'} ${name}${!ok && detail ? `\n      ${detail}` : ''}`);
};
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function until(label, fn, timeout = 60000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try { last = await fn(); if (last) return last; } catch (e) { last = e.message; }
    await wait(400);
  }
  throw new Error(`Zeitüberschreitung: ${label} (${JSON.stringify(last)})`);
}

function get(url, headers = {}) {
  return new Promise((resolve, reject) => {
    http.get(url, { headers }, (res) => {
      let b = '';
      res.on('data', (c) => { b += c; });
      res.on('end', () => resolve({ status: res.statusCode, body: b }));
    }).on('error', reject);
  });
}

function canConnect(port, host) {
  return new Promise((resolve) => {
    const s = net.connect({ port, host, timeout: 2000 });
    s.once('connect', () => { s.destroy(); resolve(true); });
    s.once('error', () => resolve(false));
    s.once('timeout', () => { s.destroy(); resolve(false); });
  });
}

async function evaluate(match, expression) {
  const targets = JSON.parse((await get(`http://127.0.0.1:${debugPort}/json/list`)).body);
  const target = targets.find((t) => t.type === 'page' && match(t.url));
  if (!target) throw new Error('Ziel nicht gefunden');
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  const result = await new Promise((resolve, reject) => {
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id !== 1) return;
      if (msg.result?.exceptionDetails) reject(new Error(msg.result.exceptionDetails.text));
      else resolve(msg.result?.result?.value);
    };
    ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate',
      params: { expression, awaitPromise: true, returnByValue: true } }));
  });
  ws.close();
  return result;
}
const shell = (expr) => evaluate((u) => u.endsWith('index.html'), expr);

// Neuer Chat, Auftrag an das Mock-Modell; es führt «echo hallo aus ASK» aus und meldet «Fertig».
async function chatRoundtrip() {
  await shell('window.ask.chatNew()');
  await until('leerer Chat', () => shell('document.querySelectorAll("#messages .msg").length === 0'), 10000);
  await until('bereit', () => shell('!document.getElementById("send-btn").disabled'), 60000);
  await shell('window.ask.chatSend("Bitte teste die Werkzeuge")');
  return until('Antwort im Chat', () => shell(`[...document.querySelectorAll('.msg.tool .result')].some((r) => r.textContent.includes('hallo aus ASK'))
    && [...document.querySelectorAll('.msg.assistant .md')].some((m) => m.textContent.startsWith('Fertig'))`), 60000).catch(() => false);
}

function bundledPython() {
  const out = execFileSync('powershell.exe', ['-NoProfile', '-Command',
    'Get-CimInstance Win32_Process -Filter "Name=\'python.exe\'" | ForEach-Object { "$($_.ProcessId)|$($_.ExecutablePath)" }'],
  { encoding: 'utf8' });
  return out.split(/\r?\n/).filter(Boolean).map((l) => l.split('|'))
    .filter(([, p]) => p && p.toLowerCase().startsWith(pythonDir)).map(([pid]) => Number(pid));
}

function listening(port) {
  const out = execFileSync('netstat.exe', ['-ano', '-p', 'tcp'], { encoding: 'utf8' });
  return out.split(/\r?\n/).filter((l) => /LISTEN|ABH/i.test(l) && l.includes(`:${port} `))
    .map((l) => l.trim().split(/\s+/)[1]);
}

function launch() {
  const child = spawn(exe, [`--remote-debugging-port=${debugPort}`], { stdio: 'ignore' });
  return child;
}

async function waitRunning() {
  await until('Terminal läuft', () => shell('document.getElementById("terminal-text").textContent')
    .then((t) => t === 'Terminal läuft'), 90000);
  return JSON.parse(fs.readFileSync(settingsFile, 'utf8')).port;
}

async function main() {
  const backup = fs.existsSync(settingsFile) ? fs.readFileSync(settingsFile) : null;
  fs.mkdirSync(path.dirname(settingsFile), { recursive: true });
  fs.rmSync(workdir, { recursive: true, force: true });
  fs.mkdirSync(workdir, { recursive: true });
  fs.writeFileSync(settingsFile, JSON.stringify({ askUrl: mockUrl, folder: workdir, recent: [workdir],
    warningAccepted: true, sidebarOpen: true, autoConnect: true, port: 0 }));
  const mock = spawn(process.execPath, [path.join(__dirname, 'mock-ask.js'), String(mockPort), 'angemeldet'], { stdio: 'ignore' });

  try {
    let app = launch();
    const port = await waitRunning();
    check('Gepackte App startet, Terminal läuft', true);
    const addrs = listening(port);
    check('Terminal lauscht nur auf 127.0.0.1', addrs.length > 0 && addrs.every((a) => a.startsWith('127.0.0.1:')), addrs.join(', '));
    const lan = Object.values(os.networkInterfaces()).flat().filter((i) => !i.internal);
    for (const ip of lan) {
      check(`Von der Netzwerkadresse ${ip.address} nicht erreichbar`, !(await canConnect(port, ip.address)));
    }
    const health = await get(`http://127.0.0.1:${port}/health`);
    check('Health-Check', health.status === 200 && health.body.includes('ok'), health.body);
    check('Ohne Key: 401', (await get(`http://127.0.0.1:${port}/files/cwd`)).status === 401);
    check('Mit falschem Key: 401', (await get(`http://127.0.0.1:${port}/files/cwd`, { Authorization: 'Bearer ask-desktop' })).status === 401);

    await until('Bei ASK angemeldet', () => shell('document.getElementById("ask-text").textContent').then((t) => t === 'ASK: Lea Test'), 60000);
    check('Bei (Mock-)ASK angemeldet', true);
    check('Auftrag im Chat: Modell ruft Befehl im Terminal auf', await chatRoundtrip());

    for (const blocked of [process.env.OneDrive, 'C:\\Windows', process.env.USERPROFILE, 'C:\\'].filter(Boolean)) {
      const r = await shell(`window.ask.openFolder(${JSON.stringify(blocked)})`);
      check(`Ordnersperre: ${blocked}`, r && r.ok === false, JSON.stringify(r));
    }

    fs.writeFileSync(path.join(workdir, '.ask', 'tasks.json'),
      JSON.stringify({ auftrag: 'Paket-Test', aufgaben: [{ titel: 'A', status: 'erledigt' }, { titel: 'B', status: 'offen' }] }));
    await until('Aufgaben', () => shell('document.getElementById("tasks-count").textContent').then((t) => t === '1 von 2'), 10000);
    check('Seitenleiste zeigt tasks.json live', true);
    fs.writeFileSync(path.join(workdir, 'ergebnis.txt'), 'fertig');
    await until('Ausgabe', () => shell('document.getElementById("output-list").textContent').then((t) => t.includes('ergebnis.txt')), 10000);
    check('Seitenleiste zeigt neue Dateien live', true);
    check('Skills in .ask/skills kopiert', fs.existsSync(path.join(workdir, '.ask', 'skills', 'word', 'SKILL.md')));

    const pids = bundledPython();
    check('Python-Prozesse laufen (Starter und Server)', pids.length >= 2, pids.join(','));
    execFileSync('taskkill.exe', ['/PID', String(app.pid)]);
    await until('App beendet', () => app.exitCode !== null, 20000);
    await wait(2000);
    check('Nach normalem Schliessen: keine Python-Prozesse mehr', bundledPython().length === 0, bundledPython().join(','));
    check('Nach normalem Schliessen: Port geschlossen', !(await canConnect(port, '127.0.0.1')));

    app = launch();
    const port2 = await waitRunning();
    check('Neustart der App', true);
    check('Nach Neustart: letzter Chat wieder da', await until('Chat geladen', () => shell('document.querySelectorAll(".msg.tool").length >= 3'), 15000).catch(() => false));
    check('Nach Neustart: Auftrag im Chat funktioniert', await chatRoundtrip());
    execFileSync('taskkill.exe', ['/F', '/PID', String(app.pid)]);
    await wait(4000);
    check('Nach Absturz (hart beendet): keine Python-Prozesse mehr', bundledPython().length === 0, bundledPython().join(','));
    check('Nach Absturz: Port geschlossen', !(await canConnect(port2, '127.0.0.1')));
  } catch (err) {
    check('Ablauf', false, err.stack);
  } finally {
    mock.kill();
    if (backup) fs.writeFileSync(settingsFile, backup);
    else fs.rmSync(settingsFile, { force: true });
    const failed = checks.filter((c) => !c.ok).length;
    console.log(`${checks.length - failed} von ${checks.length} Prüfungen bestanden.`);
    process.exit(failed ? 1 : 0);
  }
}

main();
