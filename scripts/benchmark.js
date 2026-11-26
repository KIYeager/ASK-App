// Vergleich Electron-Fassung gegen Tauri-Fassung (ENTSCHEIDE E57): Startzeiten, Speicher, Grössen.
// Läuft unter Windows (Workflow .github/workflows/benchmark.yml oder von Hand). Beide Apps werden nacheinander
// gestartet, jede zuerst einmal zum Aufwärmen (Python entpacken, WebView2 laden; nicht gezählt), danach `runs` Mal.
// Gemessen ab dem Start des Prozesses:
//   ui:       Oberfläche geladen (Seite der Oberfläche im DevTools-Protokoll, #root gefüllt, window.ask da)
//   terminal: Zustand «Terminal läuft» (getState() der Oberfläche)
//   speicher: Arbeitsspeicher aller Prozesse der App (ASK.exe und Kinder, auch WebView2 bzw. Chromium) nach 20 s
// Aufruf: node scripts/benchmark.js --electron <ASK.exe> --tauri <ASK.exe> [--runs 10] [--out bericht.json]
//         [--sizes name=datei-oder-ordner ...]
// Die Einstellungen in %APPDATA%\ASK werden gesichert und am Ende wiederhergestellt.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const http = require('node:http');
const { spawn, execFileSync } = require('node:child_process');

const args = process.argv.slice(2);
const opt = (name, def) => { const i = args.indexOf(`--${name}`); return i >= 0 ? args[i + 1] : def; };
const apps = { electron: opt('electron'), tauri: opt('tauri') };
const runs = Number(opt('runs', 10));
const out = opt('out', path.join(os.tmpdir(), 'ask-benchmark.json'));
const sizesAt = args.indexOf('--sizes');
const sizes = sizesAt >= 0 ? args.slice(sizesAt + 1).filter((a) => a.includes('=') && !a.startsWith('--')) : [];

const debugPort = 9444;
const mockPort = 18700 + Math.floor(Math.random() * 90);
const settingsDir = path.join(process.env.APPDATA || os.homedir(), 'ASK');
const settingsFile = path.join(settingsDir, 'einstellungen.json');
const workdir = path.join(os.homedir(), 'ASK-Benchmark', 'ordner');
const wait = (ms) => new Promise((r) => setTimeout(r, ms));

function get(url) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, (res) => {
      let body = '';
      res.on('data', (c) => { body += c; });
      res.on('end', () => resolve(body));
    });
    req.on('error', reject);
    req.setTimeout(2000, () => req.destroy(new Error('Zeitüberschreitung')));
  });
}

// Oberfläche: Electron lud index.html aus dem Paket, Tauri liefert http://tauri.localhost/
const isUi = (u) => /index\.html$/.test(u) || /^https?:\/\/tauri\.localhost\//.test(u);

async function evaluate(expression) {
  const targets = JSON.parse(await get(`http://127.0.0.1:${debugPort}/json/list`));
  const target = targets.find((t) => t.type === 'page' && isUi(t.url));
  if (!target) return undefined;
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  const value = await new Promise((resolve) => {
    ws.onmessage = (m) => {
      const msg = JSON.parse(m.data);
      if (msg.id === 1) resolve(msg.result?.result?.value);
    };
    ws.send(JSON.stringify({ id: 1, method: 'Runtime.evaluate', params: { expression, awaitPromise: true, returnByValue: true } }));
  });
  ws.close();
  return value;
}

async function until(fn, timeout) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    try { if (await fn()) return true; } catch { /* noch nicht bereit */ }
    await wait(50);
  }
  return false;
}

// Arbeitsspeicher (Working Set) eines Prozessbaums in MB
function treeMemory(pid) {
  const ps = `$all = Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,WorkingSetSize
$ids = @(${pid}); $sum = 0; $i = 0
while ($i -lt $ids.Count) { $p = $ids[$i]; $i++
  foreach ($x in $all) { if ($x.ParentProcessId -eq $p) { $ids += $x.ProcessId } }
  $me = $all | Where-Object { $_.ProcessId -eq $p }; if ($me) { $sum += $me.WorkingSetSize } }
[math]::Round($sum / 1MB)`;
  try { return Number(execFileSync('powershell', ['-NoProfile', '-Command', ps], { encoding: 'utf8' }).trim()); } catch { return NaN; }
}

function killTree(pid) {
  try { execFileSync('taskkill', ['/PID', String(pid), '/T', '/F'], { stdio: 'ignore' }); } catch { /* schon beendet */ }
}

async function startOnce(kind, exe, measureMemory) {
  const env = { ...process.env, ASK_NO_UPDATES: '1' };
  const argv = [];
  if (kind === 'electron') argv.push(`--remote-debugging-port=${debugPort}`);
  else env.WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = `--remote-debugging-port=${debugPort}`;
  const t0 = Date.now();
  const child = spawn(exe, argv, { stdio: 'ignore', env, detached: false });
  const result = { ui: null, terminal: null, speicher: null };
  try {
    if (await until(async () => (await evaluate('!!window.ask && !!document.querySelector("#root")?.children.length')) === true, 60000)) {
      result.ui = Date.now() - t0;
    }
    if (await until(async () => (await evaluate('window.ask.getState().then((s) => s.terminal && s.terminal.state)')) === 'läuft', 300000)) {
      result.terminal = Date.now() - t0;
    }
    if (measureMemory) {
      await wait(Math.max(0, 20000 - (Date.now() - t0)));
      result.speicher = treeMemory(child.pid);
    }
  } finally {
    killTree(child.pid);
    await wait(3000); // Terminal und WebView2 sauber weg, Port frei
  }
  return result;
}

function dirSize(p) {
  const st = fs.statSync(p);
  if (!st.isDirectory()) return st.size;
  return fs.readdirSync(p).reduce((n, f) => n + dirSize(path.join(p, f)), 0);
}

const median = (xs) => {
  const v = xs.filter((x) => Number.isFinite(x)).sort((a, b) => a - b);
  if (!v.length) return null;
  return v.length % 2 ? v[(v.length - 1) / 2] : Math.round((v[v.length / 2 - 1] + v[v.length / 2]) / 2);
};

async function main() {
  if (process.platform !== 'win32') throw new Error('Nur unter Windows sinnvoll (WebView2, Windows-Terminal).');
  for (const [k, exe] of Object.entries(apps)) if (!exe || !fs.existsSync(exe)) throw new Error(`--${k} <ASK.exe> fehlt oder existiert nicht: ${exe}`);
  const backup = fs.existsSync(settingsFile) ? fs.readFileSync(settingsFile) : null;
  fs.mkdirSync(workdir, { recursive: true });
  fs.mkdirSync(settingsDir, { recursive: true });
  const mock = spawn(process.execPath, [path.join(__dirname, '..', 'tests', 'e2e', 'mock-ask.js'), String(mockPort), 'angemeldet'], { stdio: 'ignore' });
  const report = { datum: new Date().toISOString(), rechner: `${os.cpus()[0]?.model || '?'}, ${Math.round(os.totalmem() / 2 ** 30)} GB`, runs, apps: {}, groessen: {} };
  try {
    await wait(1000);
    for (const [kind, exe] of Object.entries(apps)) {
      const settings = { askUrl: `http://127.0.0.1:${mockPort}`, folder: workdir, recent: [workdir], warningAccepted: true, sidebarOpen: true, port: 0,
        prefs: { tourDone: true } };
      fs.writeFileSync(settingsFile, JSON.stringify(settings));
      console.log(`${kind}: Aufwärmen …`);
      const warm = await startOnce(kind, exe, false);
      const list = [];
      for (let i = 0; i < runs; i++) {
        const r = await startOnce(kind, exe, true);
        console.log(`${kind} #${i + 1}: Oberfläche ${r.ui} ms, Terminal ${r.terminal} ms, Speicher ${r.speicher} MB`);
        list.push(r);
      }
      report.apps[kind] = { exe, erster: warm, laeufe: list, median: {
        ui: median(list.map((r) => r.ui)), terminal: median(list.map((r) => r.terminal)), speicher: median(list.map((r) => r.speicher)) } };
    }
    for (const s of sizes) {
      const [name, p] = [s.slice(0, s.indexOf('=')), s.slice(s.indexOf('=') + 1)];
      if (fs.existsSync(p)) report.groessen[name] = Math.round(dirSize(p) / 2 ** 20);
    }
  } finally {
    mock.kill();
    if (backup) fs.writeFileSync(settingsFile, backup); else fs.rmSync(settingsFile, { force: true });
  }
  fs.writeFileSync(out, JSON.stringify(report, null, 2));
  const e = report.apps.electron.median;
  const t = report.apps.tauri.median;
  const pct = (a, b) => (a && b ? `${Math.round(((b - a) / a) * 100)} %` : '–');
  const lines = [
    `## Electron gegen Tauri (${runs} Starts nach dem Aufwärmen, Median)`, '',
    `Rechner: ${report.rechner}`, '',
    '| Messung | Electron | Tauri | Unterschied |', '|---|---|---|---|',
    `| Oberfläche geladen | ${e.ui} ms | ${t.ui} ms | ${pct(e.ui, t.ui)} |`,
    `| Terminal läuft | ${e.terminal} ms | ${t.terminal} ms | ${pct(e.terminal, t.terminal)} |`,
    `| Arbeitsspeicher nach 20 s | ${e.speicher} MB | ${t.speicher} MB | ${pct(e.speicher, t.speicher)} |`,
    `| Erster Start (Oberfläche / Terminal) | ${report.apps.electron.erster.ui} / ${report.apps.electron.erster.terminal} ms | ${report.apps.tauri.erster.ui} / ${report.apps.tauri.erster.terminal} ms | |`,
    ...Object.entries(report.groessen).map(([k, v]) => `| Grösse ${k} | ${v} MB | | |`),
    '',
  ];
  console.log(lines.join('\n'));
  if (process.env.GITHUB_STEP_SUMMARY) fs.appendFileSync(process.env.GITHUB_STEP_SUMMARY, `${lines.join('\n')}\n`);
}

main().catch((err) => {
  console.error(err.message);
  process.exit(1);
});
