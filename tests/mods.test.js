// Befehle von Mods (src/renderer/src/lib/modCommands.mjs, ENTSCHEIDE E72)
const test = require('node:test');
const assert = require('node:assert');

test('Vorlagen und Zuordnung der Befehle von Mods', async () => {
  const { fillTemplate, modCommandFor } = await import('../src/renderer/src/lib/modCommands.mjs');
  assert.strictEqual(fillTemplate('Frag mich ab: {{text}}.', '  Englisch  '), 'Frag mich ab: Englisch.');
  assert.strictEqual(fillTemplate('Erkläre einfach.', 'Photosynthese'), 'Erkläre einfach.\n\nPhotosynthese');
  assert.strictEqual(fillTemplate('Nur das.', ''), 'Nur das.');
  const cmds = [{ cmd: '/vokabeln', prompt: 'x' }, { cmd: '/timer', js: () => {} }, { cmd: '/nur-text' }];
  assert.deepStrictEqual(modCommandFor('/Vokabeln Tiere\nund Pflanzen', cmds), { cmd: cmds[0], rest: 'Tiere\nund Pflanzen' });
  assert.strictEqual(modCommandFor('/timer', cmds).rest, '');
  assert.strictEqual(modCommandFor('/nur-text a', cmds), null, 'ohne Vorlage und Code');
  assert.strictEqual(modCommandFor('Hallo /vokabeln', cmds), null);
  assert.strictEqual(modCommandFor('/unbekannt', cmds), null);
});

test('Beispiel-Mods und mitgelieferte Skills sind gültig', () => {
  const fs = require('node:fs');
  const path = require('node:path');
  for (const base of ['resources/mods-beispiele']) {
    for (const id of fs.readdirSync(base)) {
      const dir = path.join(base, id);
      const m = JSON.parse(fs.readFileSync(path.join(dir, 'mod.json'), 'utf8'));
      assert.strictEqual(m.id, id, `${dir}: id passt zum Ordner`);
      for (const f of [m.style, m.script].filter(Boolean)) assert.ok(fs.existsSync(path.join(dir, f)), `${dir}/${f} fehlt`);
      if (m.script) assert.match(fs.readFileSync(path.join(dir, m.script), 'utf8'), /\(function \(ASK\) \{[\s\S]*\}\)\(ASK\);\s*$/, `${dir}: hält ASK fest`);
    }
  }
  for (const name of fs.readdirSync('resources/standard-skills')) {
    const text = fs.readFileSync(path.join('resources/standard-skills', name, 'SKILL.md'), 'utf8').replace(/\r\n/g, '\n'); // Windows checkt mit CRLF aus
    assert.match(text, new RegExp(`^---\\nname: ${name}\\ndescription: .{10,300}\\n`), `${name}: Kopf`);
    assert.ok(!text.replaceAll('«ß»', '').includes('ß'), `${name}: Schweizer Rechtschreibung`);
  }
});

test('Anleitung für andere KI nennt die ganze Schnittstelle ASK', () => {
  const fs = require('node:fs');
  const guide = fs.readFileSync('resources/mod-anleitung/SKILL.md', 'utf8').replace(/\r\n/g, '\n');
  assert.match(guide, /^---\nname: ask-mods\ndescription: .{20,300}\n---\n/);
  const runtime = fs.readFileSync('src/renderer/src/lib/mods.js', 'utf8');
  const api = runtime.slice(runtime.indexOf('return Object.freeze({'), runtime.indexOf('// Kleine Meldung'));
  const names = [...api.matchAll(/^ {4}([a-z]+):/gm)].map((m) => m[1]);
  assert.ok(names.length >= 10, names.join(','));
  for (const n of names) assert.ok(guide.includes(`ASK.${n}`), `ASK.${n} fehlt in der Anleitung`);
  const events = /EVENTS = \[([^\]]+)\]/.exec(runtime)[1].match(/'([a-z]+)'/g).map((e) => e.slice(1, -1));
  for (const e of events) assert.ok(guide.includes(`\`${e}\``), `Ereignis ${e} fehlt in der Anleitung`);
  const reserved = /RESERVED: &\[&str\] = &\[([^\]]+)\]/.exec(fs.readFileSync('src-tauri/core/src/skills.rs', 'utf8'))[1].match(/"([a-z]+)"/g).map((x) => x.slice(1, -1));
  for (const r of reserved) assert.ok(guide.includes(` ${r}`) || guide.includes(`\`${r}`), `gesperrter Befehl ${r} fehlt in der Anleitung`);
});
