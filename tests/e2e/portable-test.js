// Test der einzelnen EXE (npm run dist:portable), von aussen gesteuert über das DevTools-Protokoll.
// Aufruf: node tests/e2e/portable-test.js <pfad zu ASK-<Version>.exe> <testordner-basis> [startbild.png]
// Prüft ersten Start (Entpacken mit Startbild), spätere Starts aus dem Zwischenspeicher, einen
// zweiten Start bei laufender App, abgebrochenes Entpacken und das Aufräumen älterer Stände.
// Sichert vorhandene Einstellungen in %APPDATA%\ASK und stellt sie am Ende wieder her. Der Stand
// dieser EXE in %LOCALAPPDATA%\ASK-Desktop\app wird für den Test neu entpackt und bleibt danach
// liegen, wie nach einem normalen Start.
const fs = require('node:fs');
const os = require('node:os');
const net = require('node:net');
const http = require('node:http');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');

const exe = path.resolve(process.argv[2]);
const base = process.argv[3] || path.join(os.homedir(), 'ASK-Test-portable');
const splashPng = process.argv[4] || '';
const settingsFile = path.join(process.env.APPDATA, 'ASK', 'einstellungen.json');
const cacheRoot = path.join(process.env.LOCALAPPDATA, 'ASK-Desktop', 'app');
const workdir = path.join(base, 'ordner-portable');
const mockPort = 18800 + Math.floor(Math.random() * 90);
const mockUrl = `http://127.0.0.1:${mockPort}`;
const debugPort = 9334;
const checks = [];
const check = (name, ok, detail = '') => {
  checks.push({ name, ok: !!ok, detail });
  console.log(`${ok ? 'OK  ' : 'FEHLER'} ${name}${detail ? `\n      ${detail}` : ''}`);
};
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
// Oberfläche der Tauri-Fassung: http://tauri.localhost/ (WebView2, mit WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS steuerbar)
const isUi = (u) => /^https?:\/\/tauri\.localhost\//.test(String(u));
const ps = (command) => execFileSync('powershell.exe', ['-NoProfile', '-Command', command], { encoding: 'utf8' }).trim();

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

async function shell(expression) {
  const targets = JSON.parse((await get(`http://127.0.0.1:${debugPort}/json/list`)).body);
  const target = targets.find((t) => t.type === 'page' && isUi(t.url));
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

// Neuer Chat, Auftrag an das Mock-Modell; es führt «echo hallo aus ASK» aus und meldet «Fertig».
async function chatRoundtrip() {
  await shell('window.ask.chatNew()');
  await until('leerer Chat', () => shell('document.querySelectorAll("#messages .msg").length === 0'), 10000);
  await until('bereit', () => shell('!document.getElementById("send-btn").disabled'), 60000);
  await shell('window.ask.chatSend("Bitte teste die Werkzeuge")');
  return until('Antwort im Chat', () => shell(`[...document.querySelectorAll('.msg.tool .result')].some((r) => r.textContent.includes('hallo aus ASK'))
    && [...document.querySelectorAll('.msg.assistant .md')].some((m) => m.textContent.startsWith('Fertig'))`), 60000).catch(() => false);
}

function processes(name) {
  return ps(`Get-CimInstance Win32_Process -Filter "Name='${name}'" | ForEach-Object { "$($_.ProcessId)|$($_.ParentProcessId)|$($_.ExecutablePath)" }`)
    .split(/\r?\n/).filter(Boolean).map((l) => {
      const [pid, parent, file] = l.split('|');
      return { pid: Number(pid), parent: Number(parent), file: file || '' };
    });
}

// Hauptprozesse von ASK (ohne Renderer und andere Hilfsprozesse)
function mainAsk() {
  const all = processes('ASK.exe');
  return all.filter((p) => !all.some((q) => q.pid === p.parent));
}

// Python entpackt die App nach %LOCALAPPDATA%\ASK-Desktop\python\<id> (ENTSCHEIDE E39), nicht in den Stand der EXE
const pythonRoot = path.join(process.env.LOCALAPPDATA, 'ASK-Desktop', 'python').toLowerCase();
const pythonIn = () => processes('python.exe').filter((p) => p.file.toLowerCase().startsWith(pythonRoot));

// Sucht ein sichtbares Fenster des Starters im Format des Startbilds (16:9) und hält es auf Wunsch fest.
const SPLASH_PS = `param([int]$procId, [int]$seconds, [string]$png)
Add-Type @"
using System; using System.Runtime.InteropServices;
public static class W {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] static extern bool EnumWindows(P f, IntPtr l);
  [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint p);
  [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out R r);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public struct R { public int L, T, Rt, B; }
  public static int[] Find(uint pid) {
    int[] found = null;
    EnumWindows(delegate (IntPtr h, IntPtr l) {
      uint p; GetWindowThreadProcessId(h, out p);
      if (p != pid || !IsWindowVisible(h)) return true;
      R r; GetWindowRect(h, out r);
      int w = r.Rt - r.L, ht = r.B - r.T;
      if (w >= 300 && ht > 0 && Math.Abs((double)w / ht - 480.0 / 270) < 0.05) { found = new int[] { r.L, r.T, w, ht }; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
}
"@
# Bildschirmpixel für Fensterposition und Aufnahme, sonst passen sie bei Skalierung nicht zusammen
[W]::SetProcessDPIAware() | Out-Null
$end = (Get-Date).AddSeconds($seconds)
while ((Get-Date) -lt $end -and (Get-Process -Id $procId -ErrorAction SilentlyContinue)) {
  $r = [W]::Find($procId)
  if ($r) {
    Start-Sleep -Milliseconds 500
    if ($png) {
      Add-Type -AssemblyName System.Drawing
      $b = New-Object Drawing.Bitmap $r[2], $r[3]
      [Drawing.Graphics]::FromImage($b).CopyFromScreen($r[0], $r[1], 0, 0, $b.Size)
      $b.Save($png)
    }
    "$($r[2])x$($r[3])"; exit 0
  }
  Start-Sleep -Milliseconds 150
}
"kein Startbild"`;

function watchSplash(stubPid, png = '') {
  const script = path.join(os.tmpdir(), `ask-splash-${process.pid}-${stubPid}.ps1`);
  fs.writeFileSync(script, SPLASH_PS);
  return new Promise((resolve) => {
    const p = spawn('powershell.exe', ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', script,
      '-procId', String(stubPid), '-seconds', '60', '-png', png], { stdio: ['ignore', 'pipe', 'pipe'] });
    let out = '';
    p.stdout.on('data', (c) => { out += c; });
    p.stderr.on('data', (c) => { out += c; });
    p.on('exit', () => { fs.rmSync(script, { force: true }); resolve(out.trim()); });
  });
}

function launch() {
  const child = spawn(exe, [], { stdio: 'ignore', env: { ...process.env, ASK_NO_UPDATES: '1', WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${debugPort}` } });
  child.started = Date.now();
  return child;
}

async function waitRunning(stub) {
  await until('Terminal läuft', () => shell('document.getElementById("terminal-text").textContent')
    .then((t) => t === 'Terminal läuft'), 300000);
  const seconds = ((Date.now() - stub.started) / 1000).toFixed(1);
  const port = JSON.parse(fs.readFileSync(settingsFile, 'utf8')).port;
  const [app] = await until('ASK-Hauptprozess', () => (mainAsk().length === 1 ? mainAsk() : null), 10000);
  return { pid: app.pid, dir: path.dirname(app.file), port, seconds };
}

async function closeApp() {
  for (const p of mainAsk()) execFileSync('taskkill.exe', ['/PID', String(p.pid)]);
  await until('App beendet', () => processes('ASK.exe').length === 0, 30000);
}

const created = (dir) => fs.statSync(dir).birthtimeMs;

async function main() {
  if (processes('ASK.exe').length) {
    console.log('ASK läuft bereits. Bitte zuerst schliessen.');
    process.exit(1);
  }
  const buildId = ps(`(Get-Item -LiteralPath '${exe}').VersionInfo.ProductVersion`);
  const appDir = path.join(cacheRoot, buildId);
  const backup = fs.existsSync(settingsFile) ? fs.readFileSync(settingsFile) : null;
  fs.mkdirSync(path.dirname(settingsFile), { recursive: true });
  fs.rmSync(workdir, { recursive: true, force: true });
  fs.mkdirSync(workdir, { recursive: true });
  fs.writeFileSync(settingsFile, JSON.stringify({ askUrl: mockUrl, folder: workdir, recent: [workdir],
    warningAccepted: true, sidebarOpen: true, autoConnect: true, port: 0 }));
  const mock = spawn(process.execPath, [path.join(__dirname, 'mock-ask.js'), String(mockPort), 'angemeldet'], { stdio: 'ignore' });
  fs.rmSync(appDir, { recursive: true, force: true });
  fs.rmSync(`${appDir}.neu`, { recursive: true, force: true });

  try {
    // Erster Start: Startbild, Entpacken in den Zwischenspeicher, Terminal und Verbindung
    let stub = launch();
    const splash = await watchSplash(stub.pid, splashPng);
    check('Erster Start: Startbild erscheint während des Entpackens', /^\d+x\d+$/.test(splash), splash);
    const first = await waitRunning(stub);
    check(`Erster Start bis «Terminal läuft» in ${first.seconds} s`, true);
    await until('Starter beendet', () => stub.exitCode !== null, 20000);
    check('Starter beendet sich, die App läuft weiter', mainAsk().length === 1);
    check('App läuft aus dem Zwischenspeicher', first.dir.toLowerCase() === appDir.toLowerCase(), first.dir);
    check('Kein Ordner .neu übrig', !fs.existsSync(`${appDir}.neu`));
    check('Python läuft aus dem entpackten Python der App', pythonIn().length >= 2);
    check('Health-Check', (await get(`http://127.0.0.1:${first.port}/health`)).status === 200);
    await until('Bei ASK angemeldet', () => shell('document.getElementById("ask-text").textContent').then((t) => t === 'ASK: Lea Test'), 60000);
    check('Bei (Mock-)ASK angemeldet; Auftrag im Chat führt Befehl im Terminal aus', await chatRoundtrip());
    const extractedAt = created(appDir);
    await closeApp();
    check('Nach Schliessen: keine Python-Prozesse mehr', pythonIn().length === 0);
    check('Nach Schliessen: Port geschlossen', !(await canConnect(first.port, '127.0.0.1')));
    check('Nach Schliessen: Zwischenspeicher bleibt', fs.existsSync(path.join(appDir, 'ASK.exe')));

    // Späterer Start: kein Entpacken, kein Startbild
    stub = launch();
    const splash2 = await watchSplash(stub.pid);
    const later = await waitRunning(stub);
    check(`Späterer Start bis «Terminal läuft» in ${later.seconds} s`, Number(later.seconds) < Number(first.seconds));
    check('Späterer Start: ohne Startbild, nicht neu entpackt',
      splash2 === 'kein Startbild' && created(appDir) === extractedAt, splash2);

    // Zweiter Start bei laufender App
    const again = launch();
    await until('Zweiter Starter beendet', () => again.exitCode !== null, 20000);
    await wait(4000);
    check('Zweiter Start: nur eine ASK-Instanz, erste läuft weiter', mainAsk().length === 1
      && (await get(`http://127.0.0.1:${later.port}/health`)).status === 200, mainAsk().map((p) => p.pid).join(','));
    await closeApp();

    // Ältere Stände und abgebrochene Entpackungen räumt der nächste Start weg
    const old = path.join(cacheRoot, '0.0.1-alt');
    fs.mkdirSync(old, { recursive: true });
    fs.copyFileSync(path.join(appDir, 'ASK.exe'), path.join(old, 'ASK.exe'));
    const oldNeu = path.join(cacheRoot, '0.0.2-alt.neu');
    const youngNeu = path.join(cacheRoot, '0.0.3-jung.neu');
    fs.mkdirSync(oldNeu, { recursive: true });
    fs.mkdirSync(youngNeu, { recursive: true });
    ps(`(Get-Item -LiteralPath '${oldNeu}').CreationTime = (Get-Date).AddHours(-2)`);
    stub = launch();
    await waitRunning(stub);
    await until('Alte Stände gelöscht', () => !fs.existsSync(old) && !fs.existsSync(oldNeu), 30000)
      .then(() => check('Älterer Stand (mit ASK.exe) und alter .neu-Ordner gelöscht', true))
      .catch((e) => check('Älterer Stand (mit ASK.exe) und alter .neu-Ordner gelöscht', false, e.message));
    check('Junger .neu-Ordner und eigener Stand bleiben', fs.existsSync(youngNeu) && fs.existsSync(path.join(appDir, 'ASK.exe')));
    fs.rmSync(youngNeu, { recursive: true, force: true });
    await closeApp();

    // Abgebrochenes Entpacken (z. B. Herunterfahren): gilt nicht als fertig, der nächste Start entpackt neu
    fs.rmSync(appDir, { recursive: true, force: true });
    stub = launch();
    await until('Entpacken läuft', () => fs.existsSync(path.join(`${appDir}.neu`, 'skills')), 120000);
    await wait(2000);
    execFileSync('taskkill.exe', ['/F', '/PID', String(stub.pid)]);
    await until('Starter beendet', () => stub.exitCode !== null, 10000);
    check('Abgebrochen: kein fertiger Stand, kein App-Prozess', !fs.existsSync(appDir) && processes('ASK.exe').length === 0);
    stub = launch();
    const resumed = await waitRunning(stub);
    check(`Nach Abbruch: neu entpackt und gestartet in ${resumed.seconds} s`,
      resumed.dir.toLowerCase() === appDir.toLowerCase() && !fs.existsSync(`${appDir}.neu`));
    await closeApp();
  } catch (err) {
    check('Ablauf', false, err.stack);
  } finally {
    await closeApp().catch(() => {});
    mock.kill();
    if (backup) fs.writeFileSync(settingsFile, backup);
    else fs.rmSync(settingsFile, { force: true });
    const failed = checks.filter((c) => !c.ok).length;
    console.log(`${checks.length - failed} von ${checks.length} Prüfungen bestanden.`);
    process.exit(failed ? 1 : 0);
  }
}

main();
