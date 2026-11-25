// Tests für das Entpacken des mitgelieferten Pythons (src/main/python.js) und das Packen (scripts/pack-python.js).
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { EventEmitter } = require('node:events');
const { PythonRuntime, runtimeRoot, DONE } = require('../src/main/python');
const { pythonId, countEntries } = require('../scripts/pack-python');

function setup({ id = 'abcdef0123456789', entries = 4, tar = 'ok' } = {}) {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-python-'));
  const packDir = path.join(base, 'python-pack');
  const root = path.join(base, 'python');
  fs.mkdirSync(packDir, { recursive: true });
  fs.writeFileSync(path.join(packDir, 'python.json'), JSON.stringify({ id, files: 3, entries }));
  fs.writeFileSync(path.join(packDir, 'python.zip'), '');
  const calls = [];
  // Nachgebautes tar.exe: legt die Dateien im Zielordner an und meldet jede Zeile wie bsdtar -v
  const spawnImpl = (exe, args) => {
    calls.push({ exe, args });
    const child = new EventEmitter();
    child.stdout = new EventEmitter();
    child.stderr = new EventEmitter();
    const dest = args[args.indexOf('-C') + 1];
    setImmediate(() => {
      if (tar === 'fail') {
        child.stderr.emit('data', 'tar.exe: Error opening archive: Disk full\n');
        child.emit('exit', 1);
        return;
      }
      fs.mkdirSync(path.join(dest, 'Lib'), { recursive: true });
      if (tar !== 'no-exe') fs.writeFileSync(path.join(dest, 'python.exe'), 'exe');
      fs.writeFileSync(path.join(dest, 'Lib', 'os.py'), '');
      child.stderr.emit('data', 'x ./\nx ./Lib/\n');
      child.stderr.emit('data', 'x ./Lib/os.py\nx ./python.exe\n');
      child.emit('exit', 0);
    });
    return child;
  };
  const rt = new PythonRuntime({ packDir, root, tarExe: 'tar.exe', spawnImpl });
  return { base, root, rt, calls };
}

test('Ordner unter %LOCALAPPDATA%\\ASK-Desktop\\python', () => {
  assert.strictEqual(runtimeRoot({ LOCALAPPDATA: 'C:\\Users\\lea\\AppData\\Local' }),
    path.join('C:\\Users\\lea\\AppData\\Local', 'ASK-Desktop', 'python'));
  assert.strictEqual(runtimeRoot({}), '');
});

test('entpackt beim ersten Mal über <id>.neu, mit Fortschritt', async () => {
  const { root, rt, calls } = setup();
  const progress = [];
  const dir = await rt.ensure((p) => progress.push(p));
  assert.strictEqual(dir, path.join(root, 'abcdef0123456789'));
  assert.ok(fs.existsSync(path.join(dir, 'python.exe')));
  assert.ok(fs.existsSync(path.join(dir, DONE)));
  assert.ok(!fs.existsSync(`${dir}.neu`));
  assert.strictEqual(rt.exe(), path.join(dir, 'python.exe'));
  assert.deepStrictEqual(calls[0].args.slice(0, 3), ['-x', '-v', '-f']);
  assert.ok(calls[0].args.includes(`${dir}.neu`));
  assert.strictEqual(progress[0], 0);
  assert.strictEqual(progress.at(-1), 100);
  assert.ok(progress.includes(50)); // 2 von 4 Einträgen
});

test('schon entpackt: kein tar, ältere Fassungen werden entfernt', async () => {
  const { root, rt, calls } = setup();
  await rt.ensure();
  fs.mkdirSync(path.join(root, '1111111111111111', 'Lib'), { recursive: true });
  fs.mkdirSync(path.join(root, 'abcdef0123456789.neu'), { recursive: true });
  await rt.ensure();
  assert.strictEqual(calls.length, 1);
  await new Promise((r) => setTimeout(r, 50));
  assert.deepStrictEqual(fs.readdirSync(root), ['abcdef0123456789']);
});

test('zwei Starts gleichzeitig entpacken nur einmal', async () => {
  const { rt, calls } = setup();
  const [a, b] = await Promise.all([rt.ensure(), rt.ensure()]);
  assert.strictEqual(a, b);
  assert.strictEqual(calls.length, 1);
});

test('halb entpackter Ordner ohne .fertig wird ersetzt', async () => {
  const { root, rt, calls } = setup();
  const dir = path.join(root, 'abcdef0123456789');
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'python.exe'), 'alt');
  assert.strictEqual(rt.ready(), false);
  await rt.ensure();
  assert.strictEqual(calls.length, 1);
  assert.strictEqual(fs.readFileSync(path.join(dir, 'python.exe'), 'utf8'), 'exe');
});

test('Fehler: tar scheitert oder python.exe fehlt, nichts bleibt liegen', async () => {
  for (const mode of ['fail', 'no-exe']) {
    const { root, rt } = setup({ tar: mode });
    await assert.rejects(rt.ensure(), /Python konnte nicht entpackt werden/);
    assert.deepStrictEqual(fs.existsSync(root) ? fs.readdirSync(root) : [], []);
    assert.strictEqual(rt.ready(), false);
  }
});

test('ohne Archiv in der App: verständlicher Fehler', async () => {
  const base = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-python-'));
  const rt = new PythonRuntime({ packDir: path.join(base, 'fehlt'), root: path.join(base, 'python') });
  assert.strictEqual(rt.exe(), '');
  await assert.rejects(rt.ensure(), /neu installieren/);
});

test('Fassung hängt nur von Build-Skript und requirements.txt ab', () => {
  const files = { 'scripts/build-python.ps1': 'a', 'python/requirements.txt': 'numpy==1' };
  const id = pythonId((f) => files[f]);
  assert.match(id, /^[0-9a-f]{16}$/);
  assert.strictEqual(pythonId((f) => files[f]), id);
  assert.notStrictEqual(pythonId((f) => ({ ...files, 'python/requirements.txt': 'numpy==2' })[f]), id);
  assert.match(pythonId(), /^[0-9a-f]{16}$/); // echte Dateien im Repository
});

test('Dateien und Ordner zählen', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-count-'));
  fs.mkdirSync(path.join(dir, 'a', 'b'), { recursive: true });
  fs.writeFileSync(path.join(dir, 'x'), '');
  fs.writeFileSync(path.join(dir, 'a', 'b', 'y'), '');
  assert.deepStrictEqual(countEntries(dir), { files: 2, dirs: 2 });
});
