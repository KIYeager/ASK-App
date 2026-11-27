// Fenster «Hilfe» (ENTSCHEIDE E79): Inhalt vollständig, Suche findet über Stichwörter und ohne Umlaute
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const load = () => import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'helpTopics.mjs')).href);

test('Hilfe: jedes Thema hat Fragen mit Antwort, Ziele sind bekannt', async () => {
  const { HELP_TOPICS, HELP_TARGETS } = await load();
  const ids = new Set();
  for (const t of HELP_TOPICS) {
    assert.ok(t.id && t.title && t.icon, `Thema unvollständig: ${t.id}`);
    assert.ok(!ids.has(t.id), `Thema doppelt: ${t.id}`);
    ids.add(t.id);
    assert.ok(t.items.length > 0, `Thema ohne Fragen: ${t.id}`);
    for (const it of t.items) {
      assert.ok(it.q && it.a, `Frage ohne Antwort in ${t.id}`);
      if (it.open) assert.ok(HELP_TARGETS[it.open], `Unbekanntes Ziel ${it.open} in ${t.id}`);
    }
  }
  assert.ok(!ids.has('start'), '«start» ist die Übersicht');
});

test('Hilfe: Ziele für Einstellungen nennen echte Bereiche', async () => {
  const fs = require('node:fs');
  const { HELP_TARGETS } = await load();
  const settings = fs.readFileSync(path.join(__dirname, '..', 'src', 'renderer', 'src', 'components', 'Settings.jsx'), 'utf8');
  for (const key of Object.keys(HELP_TARGETS).filter((k) => k.startsWith('settings:'))) {
    assert.match(settings, new RegExp(`id: '${key.slice(9)}'`), `Bereich fehlt: ${key}`);
  }
});

test('Hilfe: Suche', async () => {
  const { searchHelp, HELP_TOPICS } = await load();
  assert.equal(searchHelp('').length, HELP_TOPICS.length);
  const qs = (q) => searchHelp(q).flatMap((t) => t.items.map((it) => it.q));
  assert.ok(qs('dunkel').some((q) => /Hell oder dunkel/.test(q)));
  assert.ok(qs('Rückgängig').some((q) => /rückgängig/.test(q)));
  assert.ok(qs('ruckgangig').some((q) => /rückgängig/.test(q)), 'ohne Umlaute');
  assert.ok(qs('dark mode').length >= 1, 'über Stichwörter');
  assert.ok(qs('mod ausschalten').length >= 1, 'alle Wörter, egal wo');
  assert.equal(searchHelp('xyzzy').length, 0);
});
