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

test('Beispiel-Mods und Katalog sind gültig', () => {
  const fs = require('node:fs');
  const path = require('node:path');
  for (const base of ['resources/mods-beispiele', 'katalog/mods']) {
    for (const id of fs.readdirSync(base)) {
      const dir = path.join(base, id);
      const m = JSON.parse(fs.readFileSync(path.join(dir, 'mod.json'), 'utf8'));
      assert.strictEqual(m.id, id, `${dir}: id passt zum Ordner`);
      for (const f of [m.style, m.script].filter(Boolean)) assert.ok(fs.existsSync(path.join(dir, f)), `${dir}/${f} fehlt`);
      if (m.script) assert.match(fs.readFileSync(path.join(dir, m.script), 'utf8'), /\(function \(ASK\) \{[\s\S]*\}\)\(ASK\);\s*$/, `${dir}: hält ASK fest`);
    }
  }
  for (const name of fs.readdirSync('resources/standard-skills')) {
    const text = fs.readFileSync(path.join('resources/standard-skills', name, 'SKILL.md'), 'utf8');
    assert.match(text, new RegExp(`^---\\nname: ${name}\\ndescription: .{10,300}\\n`), `${name}: Kopf`);
    assert.ok(!text.replaceAll('«ß»', '').includes('ß'), `${name}: Schweizer Rechtschreibung`);
  }
});
