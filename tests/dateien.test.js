// Dateibereich: Inhalt eines Ordners, Suche im Projekt, Hineinziehen (src/main/workspace.js)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Workspace } = require('../src/main/workspace');

function project() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-dateien-'));
  fs.mkdirSync(path.join(root, 'bilder'));
  fs.mkdirSync(path.join(root, '.ask'));
  fs.writeFileSync(path.join(root, 'bilder', 'diagramm.png'), 'x');
  fs.writeFileSync(path.join(root, 'ausgaben.csv'), 'a;b');
  fs.writeFileSync(path.join(root, '.ask', 'geheim.json'), '{}');
  const w = new Workspace();
  w.root = root;
  return { w, root };
}

test('Ordnerinhalt: Ordner zuerst, .ask ausgeblendet, Unterordner über den relativen Pfad', () => {
  const { w } = project();
  assert.deepStrictEqual(w.list('').entries.map((e) => e.name), ['bilder', 'ausgaben.csv']);
  assert.deepStrictEqual(w.list('bilder').entries.map((e) => e.path), [path.join('bilder', 'diagramm.png')]);
  assert.strictEqual(w.list('..').ok, false);
});

test('Suche im ganzen Projekt nach Teilen des Namens, ohne .ask', () => {
  const { w } = project();
  assert.deepStrictEqual(w.find('diag').map((e) => e.path), [path.join('bilder', 'diagramm.png')]);
  assert.deepStrictEqual(w.find('geheim'), []);
  assert.deepStrictEqual(w.find(''), []);
});

test('Hineinziehen kopiert in den gewählten Ordner und überschreibt nie', () => {
  const { w, root } = project();
  const a = w.import('bilder', [{ name: 'diagramm.png', data: new Uint8Array([1, 2]) }]);
  assert.deepStrictEqual(a, [path.join('bilder', 'diagramm (2).png')]);
  assert.strictEqual(fs.readFileSync(path.join(root, 'bilder', 'diagramm.png'), 'utf8'), 'x');
  assert.deepStrictEqual(w.import('', [{ name: '..\\..\\boese.bat', data: [] }]), ['boese.bat']);
  assert.throws(() => w.import('..', [{ name: 'x.txt', data: [] }]));
});

test('Office: Vorschau nimmt ein aktuelles PDF ohne neue Umwandlung; ohne Windows kein Office', async () => {
  const fsx = require('node:fs');
  const osx = require('node:os');
  const px = require('node:path');
  const { convertToPdf, officeInstalled, pdfTarget } = require('../src/main/office');
  const root = fsx.mkdtempSync(px.join(osx.tmpdir(), 'ask-office-'));
  try {
    const source = px.join(root, 'Bericht.docx');
    fsx.writeFileSync(source, 'x');
    const target = pdfTarget(root, source);
    assert.strictEqual(target, px.join(root, '.ask', 'pdf', 'Bericht.pdf'));
    fsx.mkdirSync(px.dirname(target), { recursive: true });
    fsx.writeFileSync(target, '%PDF');
    const res = await convertToPdf(root, source, { fresh: true });
    assert.deepStrictEqual(res, { ok: true, target, cached: true });
    assert.strictEqual(await officeInstalled('.docx', { platform: 'linux' }), false);
    assert.strictEqual(await officeInstalled('.txt', { platform: 'win32' }), false);
    assert.strictEqual(await officeInstalled('.xlsx', { platform: 'win32', run: async () => true }), true);
  } finally {
    fsx.rmSync(root, { recursive: true, force: true });
  }
});
