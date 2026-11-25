// Eigene Skills (src/main/skills.js, Werkzeuge in memorytools.js, Aufruf mit /name in knowledge.js)
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Skills, slugify, parse, format, invocation, MAX_FILE } = require('../src/main/skills');
const { runTool } = require('../src/main/tools');
const { Knowledge } = require('../src/main/knowledge');

function temp() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-skills-'));
  return { dir, done: () => fs.rmSync(dir, { recursive: true, force: true }) };
}

const SKILL = '---\nname: wochenbericht\ndescription: Wochenbericht nach der Vorlage schreiben\n---\n\n1. Notizen lesen.\n2. Bericht schreiben.\n';

test('Namen werden zu sicheren Ordnernamen', () => {
  assert.strictEqual(slugify('Wöchentlicher Bericht'), 'woechentlicher-bericht');
  assert.strictEqual(slugify('  Grüsse & Strasse!  '), 'gruesse-strasse');
  assert.strictEqual(slugify('Café Crème'), 'cafe-creme');
  assert.strictEqual(slugify('../../etc/passwd'), 'etc-passwd');
  assert.strictEqual(slugify('..\\..\\AppData\\Startup'), 'appdata-startup');
  assert.strictEqual(slugify('CON'), 'con');
  assert.strictEqual(slugify('a'.repeat(80)).length, 40);
  assert.strictEqual(slugify('---'), '');
  assert.strictEqual(slugify('名前'), '');
  assert.strictEqual(slugify('x-'.repeat(30)).endsWith('-'), false);
});

test('Kopf lesen: name, description, Anleitung; Fehler mit klarer Meldung', () => {
  assert.deepStrictEqual(parse(SKILL), { name: 'wochenbericht', description: 'Wochenbericht nach der Vorlage schreiben', body: '1. Notizen lesen.\n2. Bericht schreiben.' });
  // Windows-Zeilenenden, BOM, Anführungszeichen, eingerückte Fortsetzung
  const crlf = '\uFEFF---\r\nname: "code-erklaeren"\r\ndescription: Erklärt Code\r\n  Zeile für Zeile\r\nlicense: MIT\r\n---\r\nText\r\n';
  assert.deepStrictEqual(parse(crlf), { name: 'code-erklaeren', description: 'Erklärt Code Zeile für Zeile', body: 'Text' });
  assert.throws(() => parse('# Nur Text'), /Kopf/);
  assert.throws(() => parse('---\ndescription: x\n---\nText'), /name/);
  assert.throws(() => parse('---\nname: x\n---\nText'), /description/);
  assert.throws(() => parse('---\nname: x\ndescription: y\n---\n   \n'), /leer/);
  assert.deepStrictEqual(parse(format({ name: 'a', description: 'b\nc', body: 'd' })), { name: 'a', description: 'b c', body: 'd' });
});

test('Speichern, Liste, umbenennen, löschen, nur in userData', () => {
  const t = temp();
  try {
    const s = new Skills({ dir: path.join(t.dir, 'skills') });
    assert.deepStrictEqual(s.list(), []);
    assert.strictEqual(s.prompt(), '');
    const saved = s.save({ name: 'Wochenbericht', description: 'Bericht schreiben', body: 'Schritte' });
    assert.strictEqual(saved.slug, 'wochenbericht');
    assert.ok(fs.existsSync(path.join(t.dir, 'skills', 'wochenbericht', 'SKILL.md')));
    assert.throws(() => s.save({ name: 'wochenbericht', description: 'x', body: 'y' }), /schon/);
    assert.throws(() => s.save({ name: 'neu', description: 'x', body: 'y' }), /Befehl der App/);
    assert.throws(() => s.save({ name: '!!!', description: 'x', body: 'y' }), /Name fehlt/);
    assert.throws(() => s.save({ name: 'leer', description: ' ', body: 'y' }), /Beschreibung/);
    assert.throws(() => s.save({ name: 'gross', description: 'x', body: 'y'.repeat(MAX_FILE) }), /100 KB/);
    // Umbenennen: alter Ordner verschwindet
    s.save({ name: 'bericht', description: 'Bericht schreiben', body: 'Neu' }, { previous: 'wochenbericht' });
    assert.deepStrictEqual(s.list().map((x) => x.slug), ['bericht']);
    assert.strictEqual(s.get('Bericht').body, 'Neu');
    // Kaputte Dateien und fremde Ordnernamen erscheinen nicht
    fs.mkdirSync(path.join(t.dir, 'skills', 'kaputt'));
    fs.writeFileSync(path.join(t.dir, 'skills', 'kaputt', 'SKILL.md'), 'ohne Kopf');
    fs.mkdirSync(path.join(t.dir, 'skills', 'Gross Mit Leer'));
    fs.writeFileSync(path.join(t.dir, 'skills', 'Gross Mit Leer', 'SKILL.md'), SKILL);
    assert.deepStrictEqual(s.list().map((x) => x.slug), ['bericht']);
    assert.strictEqual(s.exportText('bericht'), '---\nname: bericht\ndescription: Bericht schreiben\n---\n\nNeu\n');
    assert.ok(s.remove('bericht'));
    assert.ok(!s.remove('bericht'));
    assert.strictEqual(s.get('../bericht'), null);
  } finally {
    t.done();
  }
});

test('Import prüft Grösse, Text, Kopf und Namen', () => {
  assert.deepStrictEqual(Skills.readImport(Buffer.from(SKILL)), { name: 'wochenbericht', description: 'Wochenbericht nach der Vorlage schreiben', body: '1. Notizen lesen.\n2. Bericht schreiben.' });
  assert.strictEqual(Skills.readImport(Buffer.from(SKILL.replace('name: wochenbericht', 'name: ../../Mein Bericht'))).name, 'mein-bericht');
  assert.throws(() => Skills.readImport(Buffer.alloc(MAX_FILE + 1, 'a')), /100 KB/);
  assert.throws(() => Skills.readImport(Buffer.from('')), /leer/);
  assert.throws(() => Skills.readImport(Buffer.from([0x2d, 0x2d, 0x2d, 0x00, 0x0a])), /keine Textdatei/);
  assert.throws(() => Skills.readImport(Buffer.from([0x2d, 0x2d, 0x2d, 0x0a, 0xff, 0xfe, 0x0a])), /UTF-8/);
  assert.throws(() => Skills.readImport(Buffer.from('# Anleitung ohne Kopf')), /Kopf/);
  assert.throws(() => Skills.readImport(Buffer.from(SKILL.replace('name: wochenbericht', 'name: skills'))), /Befehl/);
  assert.throws(() => Skills.readImport(Buffer.from(SKILL.replace('name: wochenbericht', 'name: ***'))), /Name fehlt/);
});

test('Liste im Systemprompt ist kurz, auch bei vielen Skills', () => {
  const t = temp();
  try {
    const s = new Skills({ dir: t.dir });
    s.save({ name: 'bericht', description: 'Bericht schreiben', body: 'x' });
    assert.match(s.prompt(), /# Eigene Skills\n.*skill_lesen.*\n- bericht: Bericht schreiben$/);
    for (let i = 0; i < 40; i++) s.save({ name: `skill-${i}`, description: `Beschreibung ${'lang '.repeat(60)}`, body: 'x' });
    const p = s.prompt();
    assert.ok(p.length < 1900, `${p.length}`);
    assert.match(p, /… und \d+ weitere/);
  } finally {
    t.done();
  }
});

test('Aufruf mit /name: nur vorhandene Skills, Anleitung für diesen Auftrag', () => {
  const t = temp();
  try {
    assert.deepStrictEqual(invocation('/bericht Woche 47'), { slug: 'bericht', rest: 'Woche 47' });
    assert.deepStrictEqual(invocation('  /Bericht'), { slug: 'bericht', rest: '' });
    assert.strictEqual(invocation('bitte /bericht'), null);
    assert.strictEqual(invocation('/'), null);
    const k = new Knowledge({ dir: t.dir, settings: { get: () => undefined, set() {} }, ask: null, getModel: () => '' });
    k.skills.save({ name: 'bericht', description: 'Bericht', body: 'Schreib eine Seite.' });
    assert.deepStrictEqual(k.skillFor('/bericht Woche 47'), { name: 'bericht', text: 'Schreib eine Seite.' });
    assert.strictEqual(k.skillFor('/unbekannt etwas'), null);
    assert.strictEqual(k.skillFor('Hallo'), null);
    assert.match(k.systemPrompt(''), /- bericht: Bericht/);
  } finally {
    t.done();
  }
});

test('Werkzeuge skill_lesen und skill_erstellen (mit Rückfrage)', async () => {
  const t = temp();
  try {
    const skills = new Skills({ dir: t.dir });
    const asked = [];
    let answer = false;
    const ctx = { knowledge: { skills, changed() {} }, confirm: async (m, d) => { asked.push([m, d]); return answer; } };
    let r = await runTool(null, 'skill_lesen', JSON.stringify({ name: 'bericht' }), ctx);
    assert.strictEqual(r.ok, false);
    assert.match(r.output, /noch keine/);
    const args = JSON.stringify({ name: 'Mein Bericht', beschreibung: 'Bericht nach Vorlage', anleitung: '1. Lesen\n2. Schreiben' });
    r = await runTool(null, 'skill_erstellen', args, ctx);
    assert.strictEqual(r.ok, false, 'abgelehnt');
    assert.match(asked[0][0], /Skill «mein-bericht» speichern\?/);
    assert.deepStrictEqual(skills.list(), []);
    answer = true;
    r = await runTool(null, 'skill_erstellen', args, ctx);
    assert.ok(r.ok, r.output);
    assert.match(r.output, /\/mein-bericht/);
    r = await runTool(null, 'skill_erstellen', args, ctx);
    assert.match(asked[2][0], /ersetzen\?/);
    r = await runTool(null, 'skill_lesen', JSON.stringify({ name: 'Mein Bericht' }), ctx);
    assert.ok(r.ok);
    assert.match(r.output, /Bericht nach Vorlage\n\n1\. Lesen\n2\. Schreiben/);
    r = await runTool(null, 'skill_erstellen', JSON.stringify({ name: 'github', beschreibung: 'x', anleitung: 'y' }), ctx);
    assert.strictEqual(r.ok, false);
    assert.match(r.output, /Befehl der App/);
  } finally {
    t.done();
  }
});
