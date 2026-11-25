// Prüfung von Word-, Excel- und PowerPoint-Dateien, bevor Office sie für die PDF-Umwandlung öffnet
// (office.js). Office läuft dabei mit den normalen Rechten der Person und holt verknüpfte Inhalte
// ohne Rückfrage, auch von fremden Rechnern (dabei gehen Anmeldedaten mit). Abgelehnt werden darum
// Makros, ActiveX, eingebettete Objekte, externe Verweise ausser gewöhnlichen Links, verknüpfte
// Felder und eingebettete Fremddokumente. Die Dateien sind ZIP-Archive mit XML; gelesen wird hier
// ohne Zusatzpaket, und alles, was sich nicht sicher lesen lässt, gilt als abgelehnt.
const zlib = require('node:zlib');

const MAX_ENTRIES = 5000;
const MAX_PART = 64 * 1024 * 1024; // entpackte Grösse eines geprüften Teils

class Rejected extends Error {}

// Inhaltsverzeichnis eines ZIP-Archivs: [{ name, method, compressed, size, offset }]
function zipEntries(buf) {
  let end = -1;
  for (let i = buf.length - 22; i >= Math.max(0, buf.length - 22 - 65535); i--) {
    if (buf.readUInt32LE(i) === 0x06054b50) { end = i; break; }
  }
  if (end < 0) throw new Rejected('Die Datei ist kein gültiges Office-Dokument.');
  const count = buf.readUInt16LE(end + 10);
  let pos = buf.readUInt32LE(end + 16);
  if (count === 0xffff || pos === 0xffffffff || count > MAX_ENTRIES) throw new Rejected('Die Datei ist zu gross für die Prüfung.');
  const entries = [];
  for (let n = 0; n < count; n++) {
    if (pos + 46 > buf.length || buf.readUInt32LE(pos) !== 0x02014b50) throw new Rejected('Die Datei ist beschädigt.');
    const flags = buf.readUInt16LE(pos + 8);
    const nameLength = buf.readUInt16LE(pos + 28);
    const entry = { method: buf.readUInt16LE(pos + 10), compressed: buf.readUInt32LE(pos + 20), size: buf.readUInt32LE(pos + 24),
      offset: buf.readUInt32LE(pos + 42), name: buf.toString('utf8', pos + 46, pos + 46 + nameLength).replace(/\\/g, '/') };
    if (flags & 1) throw new Rejected('Die Datei ist verschlüsselt.');
    if (entry.compressed === 0xffffffff || entry.size === 0xffffffff || entry.offset === 0xffffffff) throw new Rejected('Die Datei ist zu gross für die Prüfung.');
    entries.push(entry);
    pos += 46 + nameLength + buf.readUInt16LE(pos + 30) + buf.readUInt16LE(pos + 32);
  }
  return entries;
}

function readEntry(buf, entry) {
  const p = entry.offset;
  if (p + 30 > buf.length || buf.readUInt32LE(p) !== 0x04034b50) throw new Rejected('Die Datei ist beschädigt.');
  const start = p + 30 + buf.readUInt16LE(p + 26) + buf.readUInt16LE(p + 28);
  const data = buf.subarray(start, start + entry.compressed);
  if (entry.method === 0) return data;
  if (entry.method !== 8) throw new Rejected('Die Datei ist auf eine unbekannte Art gepackt.');
  try {
    return zlib.inflateRawSync(data, { maxOutputLength: MAX_PART });
  } catch {
    throw new Rejected('Ein Teil der Datei lässt sich nicht entpacken.');
  }
}

// XML als Text; UTF-16 erkennt man an der Marke am Anfang (sonst sähe die Prüfung nur Bytes)
function xmlText(data) {
  if (data.length >= 2 && data[0] === 0xff && data[1] === 0xfe) return data.subarray(2).toString('utf16le');
  if (data.length >= 2 && data[0] === 0xfe && data[1] === 0xff) return Buffer.from(data.subarray(2)).swap16().toString('utf16le');
  return data.toString('utf8');
}

const unescapeXml = (s) => s.replace(/&#x([0-9a-f]+);/gi, (_m, h) => String.fromCodePoint(parseInt(h, 16) || 32))
  .replace(/&#(\d+);/g, (_m, d) => String.fromCodePoint(parseInt(d, 10) || 32))
  .replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, '&');

// Attribute eines Tags, mit beiden Arten von Anführungszeichen und beliebigen Abständen
function attributes(tag) {
  const out = {};
  for (const m of tag.matchAll(/([\w:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')/g)) out[m[1]] = unescapeXml(m[2] ?? m[3]);
  return out;
}

// Diesen Verweis entfernt office.js aus der Kopie, bevor Word sie öffnet; er muss genau die Form haben, die dort erkannt wird
const TEMPLATE_REL = /^<Relationship [^>]*attachedTemplate[^>]*\/>$/;
const FIELDS = new Set(['DDE', 'DDEAUTO', 'INCLUDE', 'INCLUDETEXT', 'INCLUDEPICTURE', 'IMPORT', 'LINK', 'RD']);

function checkRels(name, xml) {
  for (const m of xml.matchAll(/<(?:[\w.-]+:)?Relationship\b[^>]*>/g)) {
    const a = attributes(m[0]);
    const type = String(a.Type || '');
    if (/\/aFChunk$/i.test(type)) throw new Rejected('Die Datei enthält ein eingebettetes Fremddokument.');
    if (String(a.TargetMode || '').trim().toLowerCase() !== 'external') continue;
    if (/\/hyperlink$/.test(type)) continue; // gewöhnlicher Link, wird erst beim Anklicken geöffnet
    if (name === 'word/_rels/settings.xml.rels' && /\/attachedTemplate$/.test(type) && TEMPLATE_REL.test(m[0])) continue;
    throw new Rejected('Die Datei verweist auf externe Inhalte.');
  }
}

// Felder in Word: <w:fldSimple w:instr="…"> oder fldChar begin … instrText … fldChar separate/end.
// Der Feldname ist das erste Wort der Anweisung, auch wenn Word sie auf mehrere Stücke verteilt.
function checkFields(xml) {
  if (/<w:altChunk\b/.test(xml)) throw new Rejected('Die Datei enthält ein eingebettetes Fremddokument.');
  const check = (instruction) => {
    const name = /^[\s\\]*([A-Za-z]+)/.exec(instruction)?.[1]?.toUpperCase();
    if (name && FIELDS.has(name)) throw new Rejected('Die Datei enthält verknüpfte Felder.');
  };
  const open = []; // Anweisungen der offenen Felder (Felder können verschachtelt sein)
  let loose = ''; // Anweisung ohne Feldanfang
  for (const m of xml.matchAll(/<w:fldSimple\b[^>]*>|<w:fldChar\b[^>]*>|<w:(?:instrText|delInstrText)\b[^>]*>([^<]*)/g)) {
    if (m[0].startsWith('<w:fldSimple')) check(attributes(m[0])['w:instr'] || '');
    else if (m[0].startsWith('<w:fldChar')) {
      const kind = attributes(m[0])['w:fldCharType'];
      if (kind === 'begin') open.push('');
      else if (open.length) check(open[open.length - 1]);
      if (kind === 'end') open.pop();
    } else if (open.length) open[open.length - 1] += unescapeXml(m[1] || '');
    else loose += unescapeXml(m[1] || '');
  }
  for (const instruction of open) check(instruction);
  check(loose);
}

// Fehlertext, wenn Office die Datei nicht öffnen soll; sonst ''.
function checkOoxml(buf) {
  try {
    const entries = zipEntries(buf);
    for (const e of entries) {
      if (/vbaProject|activeX|oleObject|embeddings\/.*\.(bin|exe|dll|js|vbs|ps1|bat|cmd)$/i.test(e.name)) throw new Rejected(`Die Datei enthält ${e.name}.`);
      if (/^xl\/externalLinks\//i.test(e.name)) throw new Rejected('Die Datei enthält verknüpfte Felder.');
      if (/\.rels$/i.test(e.name)) checkRels(e.name, xmlText(readEntry(buf, e)));
      else if (/^word\/.*\.xml$/i.test(e.name)) checkFields(xmlText(readEntry(buf, e)));
    }
    return '';
  } catch (err) {
    if (err instanceof Rejected) return err.message;
    return 'Die Datei lässt sich nicht prüfen.';
  }
}

module.exports = { checkOoxml, zipEntries, readEntry };
