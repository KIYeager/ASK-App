// Karte des Gedächtnisses (src/renderer/src/lib/graph.mjs)
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const load = () => import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'graph.mjs')).href);
const item = (id, text, tags, project = null, updated = 1) => ({ id, text, tags, scope: project ? 'projekt' : 'konto', project, updated });

test('Gedankenkarte: Äste für «Über dich» und jedes Projekt, jeder Eintrag als Zeile', async () => {
  const { mindmap, MAP } = await load();
  const projects = [{ folder: 'C:\\P\\Budget', name: 'Haushaltsbudget', color: '#0082B4' }, { folder: 'C:\\P\\Leer', name: 'Leer', color: '#fff' }];
  const items = [
    item('a', 'Mag kurze Antworten in Stichworten.', ['stil'], null, 5),
    item('b', 'Kennt Python gut.', ['python', 'kenntnisse'], null, 9),
    item('c', 'Beträge immer in CHF.', ['budget', 'stil'], 'c:\\p\\budget', 3),
    item('d', 'Gehört zu einem Ordner, den die Liste nicht kennt.', [], 'C:\\P\\Unbekannt', 1),
  ];
  const m = mindmap(items, projects);
  const groups = m.nodes.filter((n) => n.kind === 'gruppe');
  assert.deepEqual(groups.map((g) => [g.label, g.count]), [['Über dich', 2], ['Haushaltsbudget', 1], ['Unbekannt', 1]], 'Projekt ohne Einträge fehlt, Ordner ohne Projekt heisst wie der Ordner');
  // Jeder Eintrag hängt an genau einem Ast, auf dessen Seite, weiter aussen als der Ast
  const rows = m.nodes.filter((n) => n.kind === 'item');
  assert.equal(rows.length, 4);
  for (const r of rows) {
    const g = groups.find((x) => x.id === r.group);
    assert.equal(r.side, g.side);
    assert.ok(Math.abs(r.x) > Math.abs(g.x));
    assert.equal(m.links.filter((l) => l.to === r.id).length, 1);
  }
  // Neueste zuoberst, Zeilen überlappen nicht
  const about = rows.filter((r) => r.group === 'g:du');
  assert.deepEqual(about.map((r) => r.item.id), ['b', 'a']);
  assert.equal(about[1].y - about[0].y, MAP.row);
  for (const side of [1, -1]) {
    const ys = rows.filter((r) => r.side === side).map((r) => r.y).sort((x, y) => x - y);
    for (let i = 1; i < ys.length; i++) assert.ok(ys[i] - ys[i - 1] >= MAP.row, 'Zeilen einer Seite haben Abstand');
  }
  // Beide Seiten benutzt, Schlagworte nach Häufigkeit
  assert.ok(groups.some((g) => g.side === 1) && groups.some((g) => g.side === -1));
  assert.deepEqual(m.tags[0], ['stil', 2]);
  // Der Ausschnitt reicht bis ans Ende der längsten Zeile
  const far = Math.max(...m.extent.map((p) => Math.abs(p.x)));
  assert.ok(far > MAP.itemX + 100);
  // Beide Seiten zusammen passen in ein übliches Fenster, ohne dass die Schrift klein wird
  assert.ok(2 * (MAP.itemX + 16 + MAP.chars * MAP.charW) < 1100);
});

test('Gedankenkarte: lange Einträge gekürzt, eine einzige Gruppe steht rechts, verwandte Einträge über Schlagworte', async () => {
  const { mindmap, related, shorten, MAP } = await load();
  const long = 'Ein sehr langer Eintrag, der in der Karte nicht ganz Platz hat und darum mit drei Punkten endet.';
  const items = [item('a', long, ['x']), item('b', 'Kurz', ['x', 'y']), item('c', 'Allein', ['z'])];
  const m = mindmap(items, []);
  assert.ok(m.nodes.filter((n) => n.kind !== 'du').every((n) => n.side === 1));
  const row = m.nodes.find((n) => n.id === 'i:a');
  assert.ok(row.label.length <= MAP.chars && row.label.length > MAP.chars - 4);
  assert.ok(row.label.endsWith('…'));
  assert.equal(shorten('  viel   Leerraum  '), 'viel Leerraum');
  assert.deepEqual([...related(items, 'a')].sort(), ['a', 'b']);
  assert.deepEqual([...related(items, 'c')], ['c']);
  assert.deepEqual(mindmap([], []).nodes.map((n) => n.kind), ['du'], 'leer: nur die Mitte');
  // Viele Einträge über die Person: nach Themen (häufigste Schlagworte) in Äste geteilt, der Rest unter «Weiteres»
  const many = [...Array.from({ length: 5 }, (_, i) => item(`s${i}`, `Schule ${i} Eintrag`, ['schule', `x${i}`])), ...Array.from({ length: 4 }, (_, i) => item(`g${i}`, `Gerät ${i} Eintrag`, ['geräte'])),
    item('e1', 'Einzelner Eintrag eins', ['allein']), item('e2', 'Einzelner Eintrag zwei', [])];
  const split = mindmap(many, []).nodes.filter((n) => n.kind === 'gruppe');
  assert.deepEqual(split.map((g) => [g.label, g.count]).sort(), [['Weiteres', 2], ['geräte', 4], ['schule', 5]]);
  assert.ok(split.every((g) => g.about && g.topic));
  // Ohne gemeinsame Schlagworte bleibt es ein Ast «Über dich»
  const loose = Array.from({ length: 12 }, (_, i) => item(`l${i}`, `Loser Eintrag Nummer ${i}`, [`t${i}`]));
  assert.deepEqual(mindmap(loose, []).nodes.filter((n) => n.kind === 'gruppe').map((g) => g.label), ['Über dich']);
});
