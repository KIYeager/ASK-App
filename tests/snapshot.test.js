const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Snapshots } = require('../src/main/snapshot');

function folder() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-sicherung-'));
  const write = (rel, text) => {
    fs.mkdirSync(path.dirname(path.join(root, rel)), { recursive: true });
    fs.writeFileSync(path.join(root, rel), text);
  };
  const read = (rel) => fs.readFileSync(path.join(root, rel), 'utf8');
  const exists = (rel) => fs.existsSync(path.join(root, rel));
  return { root, write, read, exists, done: () => fs.rmSync(root, { recursive: true, force: true }) };
}

const objects = (root) => {
  const dir = path.join(root, '.ask', 'sicherung', 'objekte');
  return fs.existsSync(dir) ? fs.readdirSync(dir).flatMap((d) => fs.readdirSync(path.join(dir, d))).length : 0;
};

test('Sichern, Änderungen erkennen, zurücksetzen', async () => {
  const f = folder();
  try {
    f.write('bericht.docx', 'Fassung 1');
    f.write('daten/liste.csv', 'a;b');
    f.write('.ask/chats/x.json', '{}');
    f.write('.git/HEAD', 'ref');
    f.write('~$bericht.docx', 'Sperre');
    const s = new Snapshots(f.root);
    const before = await s.take();
    assert.strictEqual(before.count, 2, '.ask, .git und Office-Sperrdateien nicht gesichert');

    // Auftrag: ändern, neu anlegen (auch in neuem Ordner), löschen
    f.write('bericht.docx', 'Fassung 2 vom Modell');
    f.write('neu/spiel.html', '<html>');
    fs.rmSync(path.join(f.root, 'daten/liste.csv'));
    const d = await s.diff(before.id);
    assert.deepStrictEqual(d, { changed: ['bericht.docx'], added: ['neu/spiel.html'], removed: ['daten/liste.csv'] });

    const r = await s.restore(before.id);
    assert.deepStrictEqual(r.failed, []);
    assert.strictEqual(f.read('bericht.docx'), 'Fassung 1');
    assert.strictEqual(f.read('daten/liste.csv'), 'a;b');
    assert.ok(!f.exists('neu/spiel.html') && !f.exists('neu'), 'neue Datei und leerer Ordner weg');
    assert.ok(f.exists('.ask/chats/x.json') && f.exists('.git/HEAD'));
    assert.deepStrictEqual(await s.diff(before.id), { changed: [], added: [], removed: [] });

    // Rückgängig lässt sich rückgängig machen
    await s.restore(r.safety);
    assert.strictEqual(f.read('bericht.docx'), 'Fassung 2 vom Modell');
    assert.ok(f.exists('neu/spiel.html'));
  } finally {
    f.done();
  }
});

test('Jede Fassung nur einmal gespeichert, unveränderte Dateien nicht neu gelesen', async () => {
  const f = folder();
  try {
    f.write('a.txt', 'gleich');
    f.write('b.txt', 'gleich');
    f.write('c.txt', 'anders');
    const s = new Snapshots(f.root);
    await s.take();
    assert.strictEqual(objects(f.root), 2);
    await s.take();
    await new Snapshots(f.root).take(); // nach Neustart
    assert.strictEqual(objects(f.root), 2);
    f.write('c.txt', 'neu');
    await s.take();
    assert.strictEqual(objects(f.root), 3);
    // gleiche Grösse, anderer Inhalt und andere Zeit: diff liest die Datei nach
    const id = (await s.take()).id;
    const st = fs.statSync(path.join(f.root, 'a.txt'));
    fs.writeFileSync(path.join(f.root, 'a.txt'), 'GLEICH');
    fs.utimesSync(path.join(f.root, 'a.txt'), st.atime, new Date(st.mtimeMs + 5000));
    assert.deepStrictEqual((await s.diff(id)).changed, ['a.txt']);
    // nur berührt (neue Zeit, gleicher Inhalt): keine Änderung
    fs.utimesSync(path.join(f.root, 'b.txt'), st.atime, new Date(st.mtimeMs + 9000));
    assert.deepStrictEqual((await s.diff(id)).changed, ['a.txt']);
  } finally {
    f.done();
  }
});

test('Alte Stände und nicht mehr gebrauchte Fassungen werden gelöscht', async () => {
  const f = folder();
  try {
    f.write('a.txt', 'eins');
    const s = new Snapshots(f.root);
    const old = await s.take();
    f.write('a.txt', 'zweite Fassung'); // andere Länge: gleich lang und gleiche Zeitmarke gilt als unverändert
    const now = await s.take();
    assert.strictEqual(objects(f.root), 2);
    const file = path.join(f.root, '.ask', 'sicherung', 'staende', `${old.id}.json`);
    const stand = JSON.parse(fs.readFileSync(file, 'utf8'));
    fs.writeFileSync(file, JSON.stringify({ ...stand, time: Date.now() - 15 * 24 * 3600 * 1000 }));
    assert.strictEqual(s.prune(), 1);
    assert.strictEqual(objects(f.root), 1);
    assert.ok(!fs.existsSync(file));
    assert.deepStrictEqual(await s.diff(now.id), { changed: [], added: [], removed: [] });
  } finally {
    f.done();
  }
});
