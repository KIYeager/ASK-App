const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { saveAttachments, imageUrl, cleanName } = require('../src/main/attachments');
const { apiMessages } = require('../src/main/agent');

const tmp = () => fs.mkdtempSync(path.join(os.tmpdir(), 'ask-anhang-'));
const bytes = (text) => new TextEncoder().encode(text);

test('Namen: ohne Pfad, ohne verbotene Zeichen, Bildschirmfoto mit Datum', () => {
  assert.strictEqual(cleanName('..\\..\\Windows\\win.ini'), 'win.ini');
  assert.strictEqual(cleanName('a:b*c?.txt'), 'a_b_c_.txt');
  assert.match(cleanName('image.png', 'image/png'), /^bild-\d{8}-\d{6}\.png$/);
  assert.match(cleanName('', 'image/jpeg'), /^bild-\d{8}-\d{6}\.jpg$/);
});

test('Anhänge landen in .ask/anhaenge/<Chat>/, nichts wird überschrieben', () => {
  const folder = tmp();
  try {
    const first = saveAttachments(folder, [{ name: 'bericht.docx', data: bytes('eins') }, { name: 'foto.PNG', data: bytes('png') }], 'c1');
    assert.deepStrictEqual(first.map((a) => [a.path, a.image]), [['.ask/anhaenge/c1/bericht.docx', false], ['.ask/anhaenge/c1/foto.PNG', true]]);
    const second = saveAttachments(folder, [{ name: 'bericht.docx', data: bytes('zwei') }], 'c1');
    assert.strictEqual(second[0].path, '.ask/anhaenge/c1/bericht (2).docx');
    assert.strictEqual(fs.readFileSync(path.join(folder, '.ask', 'anhaenge', 'c1', 'bericht.docx'), 'utf8'), 'eins');
    assert.strictEqual(saveAttachments(folder, [{ name: 'bericht.docx', data: bytes('drei') }], 'c2')[0].path, '.ask/anhaenge/c2/bericht.docx');
    assert.throws(() => saveAttachments(folder, Array.from({ length: 11 }, (_, i) => ({ name: `${i}.txt`, data: bytes('x') })), 'c1'), /Höchstens 10/);
    assert.throws(() => saveAttachments(folder, [], '../x'), /Ungültiger Chat/);
  } finally {
    fs.rmSync(folder, { recursive: true, force: true });
  }
});

test('Bild als data-URL, verkleinert wenn möglich, fehlende Datei ohne Fehler', () => {
  const folder = tmp();
  try {
    const [img, doc] = saveAttachments(folder, [{ name: 'a.png', data: bytes('PNGDATA') }, { name: 'b.txt', data: bytes('t') }], 'c1');
    assert.strictEqual(imageUrl(folder, img), `data:image/png;base64,${Buffer.from('PNGDATA').toString('base64')}`);
    assert.strictEqual(imageUrl(folder, img, () => ({ data: Buffer.from('klein'), type: 'image/jpeg' })), `data:image/jpeg;base64,${Buffer.from('klein').toString('base64')}`);
    assert.strictEqual(imageUrl(folder, doc), null);
    assert.strictEqual(imageUrl(folder, { path: '.ask/anhaenge/c1/weg.png' }), null);
  } finally {
    fs.rmSync(folder, { recursive: true, force: true });
  }
});

test('Verlauf: Pfade im Text, Bilder nur aus den letzten zwei Eingaben', () => {
  const att = (n, image = true) => ({ path: `eingang/${n}`, name: n, image });
  const messages = [
    { role: 'user', content: 'alt', attachments: [att('1.png')] },
    { role: 'assistant', content: 'ok' },
    { role: 'user', content: '', attachments: [att('2.png'), att('liste.xlsx', false)] },
    { role: 'assistant', content: 'ok' },
    { role: 'user', content: 'neu', attachments: [att('3.png')] },
  ];
  const url = (a) => `data:image/png;base64,${a.name}`;
  const out = apiMessages('sys', messages, false, url);
  assert.strictEqual(out[1].content, 'alt\n\n[Angehängt, liegt im Arbeitsordner: eingang/1.png]');
  assert.deepStrictEqual(out[3].content, [
    { type: 'text', text: 'Siehe Anhang.\n\n[Angehängt, liegt im Arbeitsordner: eingang/2.png, eingang/liste.xlsx]' },
    { type: 'image_url', image_url: { url: 'data:image/png;base64,2.png' } }]);
  assert.strictEqual(out[5].content[1].image_url.url, 'data:image/png;base64,3.png');
  // ohne Bildfunktion (Modell versteht keine Bilder): nur Text
  assert.ok(apiMessages('sys', messages, false, null).every((m) => typeof m.content === 'string'));
  // Bild nicht mehr lesbar: nur Text
  assert.strictEqual(typeof apiMessages('sys', messages, false, () => null)[5].content, 'string');
});
