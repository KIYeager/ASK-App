// Tests für die Prüfung von Office-Dateien vor der PDF-Umwandlung (src/main/ooxml.js).
const test = require('node:test');
const assert = require('node:assert');
const zlib = require('node:zlib');
const { checkOoxml } = require('../src/main/ooxml');

// Kleines ZIP-Archiv aus { name: text | Buffer }; store: ungepackt ablegen
function zip(files, { store = false } = {}) {
  const local = [];
  const central = [];
  let offset = 0;
  for (const [name, content] of Object.entries(files)) {
    const raw = Buffer.isBuffer(content) ? content : Buffer.from(content);
    const data = store ? raw : zlib.deflateRawSync(raw);
    const n = Buffer.from(name);
    const head = Buffer.alloc(30);
    head.writeUInt32LE(0x04034b50, 0); head.writeUInt16LE(20, 4); head.writeUInt16LE(store ? 0 : 8, 8);
    head.writeUInt32LE(data.length, 18); head.writeUInt32LE(raw.length, 22); head.writeUInt16LE(n.length, 26);
    const dir = Buffer.alloc(46);
    dir.writeUInt32LE(0x02014b50, 0); dir.writeUInt16LE(20, 4); dir.writeUInt16LE(20, 6); dir.writeUInt16LE(store ? 0 : 8, 10);
    dir.writeUInt32LE(data.length, 20); dir.writeUInt32LE(raw.length, 24); dir.writeUInt16LE(n.length, 28); dir.writeUInt32LE(offset, 42);
    local.push(head, n, data);
    central.push(dir, n);
    offset += 30 + n.length + data.length;
  }
  const dirSize = central.reduce((sum, b) => sum + b.length, 0);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0); end.writeUInt16LE(Object.keys(files).length, 8); end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(dirSize, 12); end.writeUInt32LE(offset, 16);
  return Buffer.concat([...local, ...central, end]);
}

const REL = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships';
const rels = (...items) => `<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">${items.join('')}</Relationships>`;
const doc = (body) => `<?xml version="1.0"?><w:document xmlns:w="x"><w:body>${body}</w:body></w:document>`;
const field = (...parts) => `<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>${parts.map((p) => `<w:r><w:instrText xml:space="preserve">${p}</w:instrText></w:r>`).join('')}`
  + '<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>x</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>';
const docx = (documentXml, relsXml = rels(), extra = {}) => zip({ '[Content_Types].xml': '<Types/>', 'word/document.xml': documentXml, 'word/_rels/document.xml.rels': relsXml, ...extra });

test('Gewöhnliche Dokumente gehen durch', () => {
  assert.strictEqual(checkOoxml(docx(doc('<w:p><w:r><w:t>Hallo</w:t></w:r></w:p>'))), '');
  assert.strictEqual(checkOoxml(zip({ 'word/document.xml': doc('<w:p/>') }, { store: true })), '');
  // Links, Inhaltsverzeichnis, Seitenzahl und ein Link, dessen Adresse «link» oder «dde» enthält
  const ok = doc(field(' TOC \\o "1-3" ') + field(' PAGE ') + field(' HYPERLINK "https://x.ch/link/dde" ') + field(' HYPER', 'LINK "https://x.ch" ')
    + '<w:fldSimple w:instr=" DATE \\@ &quot;dd.MM.yyyy&quot; "/>');
  assert.strictEqual(checkOoxml(docx(ok, rels(`<Relationship Id="r1" Type="${REL}/hyperlink" Target="https://x.ch" TargetMode="External"/>`))), '');
  // Verweis auf eine Vorlage in der Form, die office.js aus der Kopie entfernt
  assert.strictEqual(checkOoxml(docx(doc('<w:p/>'), rels(), {
    'word/_rels/settings.xml.rels': rels(`<Relationship Id="r1" Type="${REL}/attachedTemplate" Target="file:///C:/v.dotx" TargetMode="External"/>`) })), '');
});

test('Externe Verweise werden abgelehnt, auch in ungewohnter Schreibweise', () => {
  const image = `${REL}/image`;
  for (const rel of [
    `<Relationship Id="r1" Type="${image}" Target="\\\\boese\\freigabe\\x.png" TargetMode="External"/>`,
    `<Relationship Id="r1" Type="${image}" Target="https://boese.example/hyperlink" TargetMode="External"/>`, // Ziel endet auf /hyperlink
    `<Relationship Id='r1' Type='${image}' Target='\\\\boese\\x.png' TargetMode='External'/>`, // einfache Anführungszeichen
    `<Relationship\n\tId="r1" Type="${image}" Target="\\\\boese\\x.png" TargetMode="External"/>`, // Zeilenumbruch nach dem Namen
    `<Relationship Id="r1" Type="${image}" Target="\\\\boese\\x.png" TargetMode = "External" />`, // Abstände um =
    `<Relationship Id="r1" Type="${image}" Target="\\\\boese\\x.png" TargetMode="external"/>`,
    `<Relationship Id="r1" Type="${REL}/hyperlink/x" Target="\\\\boese\\x" TargetMode="External"/>`,
    `<pr:Relationship xmlns:pr="p" Id="r1" Type="${image}" Target="\\\\boese\\x.png" TargetMode="External"/>`,
    `<Relationship Id="r1" Type="${REL}/aFChunk" Target="fremd.html"/>`,
  ]) assert.match(checkOoxml(docx(doc('<w:p/>'), rels(rel))), /externe Inhalte|Fremddokument/, rel);
  // Vorlage in einer Form, die das Entfernen in office.js nicht erkennt
  assert.match(checkOoxml(docx(doc('<w:p/>'), rels(), {
    'word/_rels/settings.xml.rels': rels(`<Relationship Id='r1' Type='${REL}/attachedTemplate' Target='\\\\boese\\v.dotx' TargetMode='External'></Relationship>`) })), /externe Inhalte/);
  // als UTF-16 gespeichert
  const utf16 = Buffer.concat([Buffer.from([0xff, 0xfe]), Buffer.from(rels(`<Relationship Id="r1" Type="${image}" Target="\\\\boese\\x.png" TargetMode="External"/>`), 'utf16le')]);
  assert.match(checkOoxml(docx(doc('<w:p/>'), utf16)), /externe Inhalte/);
});

test('Verknüpfte Felder, Makros und Fremddokumente werden abgelehnt', () => {
  for (const body of [
    field(' INCLUDEPICTURE "\\\\\\\\boese\\\\x.png" \\d '),
    field(' includepicture "\\\\\\\\boese\\\\x.png" '), // klein geschrieben
    field(' INCLUDE', 'PICTURE "\\\\\\\\boese\\\\x.png" '), // auf zwei Stücke verteilt
    field(' DDEAUTO c:\\\\windows\\\\system32\\\\cmd.exe "/k calc" '),
    field(' LINK Excel.Sheet.12 "\\\\\\\\boese\\\\a.xlsx" '),
    field(' IF 1 = 1 "', field(' INCLUDETEXT "\\\\\\\\boese\\\\a.docx" '), '" '), // verschachtelt
    '<w:fldSimple w:instr=\' INCLUDETEXT "x" \'/>',
    '<w:fldSimple w:instr=" RD &quot;\\\\boese\\a.docx&quot; "/>',
    '<w:p><w:r><w:instrText> INCLUDEPICTURE "x" </w:instrText></w:r></w:p>', // ohne Feldanfang
    '<w:altChunk r:id="r1"/>',
  ]) assert.match(checkOoxml(docx(doc(body))), /verknüpfte Felder|Fremddokument/, body);
  // auch in Fussnoten, nicht nur im Haupttext
  assert.match(checkOoxml(docx(doc('<w:p/>'), rels(), { 'word/footnotes.xml': doc(field(' INCLUDEPICTURE "x" ')) })), /verknüpfte Felder/);
  assert.match(checkOoxml(docx(doc('<w:p/>'), rels(), { 'word/vbaProject.bin': 'x' })), /vbaProject/);
  assert.match(checkOoxml(zip({ 'xl/workbook.xml': '<x/>', 'xl/externalLinks/externalLink1.xml': '<x/>' })), /verknüpfte Felder/);
});

test('Was sich nicht lesen lässt, wird abgelehnt', () => {
  assert.notStrictEqual(checkOoxml(Buffer.from('kein zip')), '');
  assert.notStrictEqual(checkOoxml(Buffer.alloc(0)), '');
  const broken = docx(doc('<w:p/>'));
  broken.fill(0xff, 40, 60); // gepackte Daten zerstört
  assert.notStrictEqual(checkOoxml(broken), '');
});
