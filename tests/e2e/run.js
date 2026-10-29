// Startet Mock-ASK und die App im Entwicklungsmodus mit einem Testszenario.
// Aufruf: node tests/e2e/run.js <chat|anmelden|erststart|coworker> <testordner-basis>
// Benötigt ASK_BUILD_DIR (mit gebautem Python) und optional ASK_RUNTIME_DIR.
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const net = require('node:net');
const { spawn } = require('node:child_process');

const scenario = process.argv[2] || 'chat';
const base = process.argv[3] || path.join(os.homedir(), 'ASK-Test-e2e');
const root = path.join(__dirname, '..', '..');
const electron = require(path.join(root, 'node_modules', 'electron'));
const mockPort = 18900 + Math.floor(Math.random() * 90);
const askUrl = `http://127.0.0.1:${mockPort}`;

const userData = path.join(base, `userdata-${scenario}`);
const workdir = path.join(base, `ordner-${scenario}`);
const report = path.join(base, `bericht-${scenario}.json`);
const shots = path.join(base, 'bilder');
fs.rmSync(userData, { recursive: true, force: true });
fs.mkdirSync(userData, { recursive: true });
for (const dir of [workdir, `${workdir}-zwei`]) fs.rmSync(dir, { recursive: true, force: true });
fs.mkdirSync(workdir, { recursive: true });
fs.rmSync(report, { force: true });

if (scenario !== 'erststart') {
  fs.writeFileSync(path.join(userData, 'einstellungen.json'), JSON.stringify({
    askUrl, folder: workdir, recent: [workdir], warningAccepted: true, sidebarOpen: true, port: 0,
  }));
}

function portOpen(port) {
  return new Promise((resolve) => {
    const s = net.connect(port, '127.0.0.1');
    s.once('connect', () => { s.destroy(); resolve(true); });
    s.once('error', () => resolve(false));
  });
}

const mock = spawn(process.execPath, [path.join(__dirname, 'mock-ask.js'), String(mockPort),
  scenario === 'anmelden' ? 'abgemeldet' : 'angemeldet'], { stdio: 'inherit' });

const app = spawn(electron, [root], {
  stdio: ['ignore', 'pipe', 'pipe'],
  env: { ...process.env, ASK_TEST_USER_DATA: userData, ASK_TEST_SCRIPT: path.join(__dirname, 'app-test.js'),
    ASK_TEST_REPORT: report, ASK_TEST_SCENARIO: scenario, ASK_TEST_SHOTS: shots, ASK_TEST_ASK_URL: askUrl,
    ASK_TEST_GITHUB_API: `${askUrl}/github-api`, ASK_TEST_AUTOCONFIRM: '1' },
});
app.stdout.on('data', (d) => process.stdout.write(`[app] ${d}`));
app.stderr.on('data', (d) => process.stderr.write(`[app] ${d}`));

const timer = setTimeout(() => { console.error('Zeitüberschreitung, App wird beendet'); app.kill(); }, 240000);
app.on('exit', async (code) => {
  clearTimeout(timer);
  mock.kill();
  let result = { checks: [{ name: 'Bericht vorhanden', ok: false }] };
  try { result = JSON.parse(fs.readFileSync(report, 'utf8')); } catch { /* fehlt */ }
  const settingsFile = path.join(userData, 'einstellungen.json');
  const port = fs.existsSync(settingsFile) ? JSON.parse(fs.readFileSync(settingsFile, 'utf8')).port : 0;
  if (port) {
    await new Promise((r) => setTimeout(r, 1500));
    result.checks.push({ name: 'Terminal-Port nach dem Beenden geschlossen', ok: !(await portOpen(port)), detail: `Port ${port}` });
  }
  let failed = 0;
  for (const c of result.checks) {
    if (!c.ok) failed++;
    console.log(`${c.ok ? 'OK  ' : 'FEHLER'} ${c.name}${c.detail && !c.ok ? `\n      ${c.detail}` : ''}`);
  }
  console.log(`App-Exitcode ${code}; ${result.checks.length - failed} von ${result.checks.length} Prüfungen bestanden.`);
  process.exit(failed ? 1 : 0);
});
