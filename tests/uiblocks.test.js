// Interaktive Antworten (ENTSCHEIDE E80): Blöcke finden, Bausteine prüfen, Formeln ohne eval
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const load = () => import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'uiBlocks.mjs')).href);

test('Blöcke in der Antwort finden, offene am Ende erkennen', async () => {
  const { splitUi } = await load();
  const parts = splitUi('Text davor\n```ask-ui\n{"typ":"hinweis","text":"x"}\n```\nDazwischen\n```js\n```ask-ui\n```\nEnde\n```ask-html\n<p>');
  assert.deepStrictEqual(parts.map((p) => p.k), ['md', 'ui', 'md', 'html']);
  assert.equal(parts[1].done, true);
  assert.match(parts[2].text, /```js\n```ask-ui\n```/, 'ask-ui in einem gewöhnlichen Codeblock zählt nicht');
  assert.equal(parts[3].done, false);
  assert.equal(parts[3].src, '<p>');
  assert.deepStrictEqual(splitUi('nur Text').map((p) => p.k), ['md']);
});

test('Bausteine prüfen: gültige Beispiele aus dem Skill, verständliche Fehler', async () => {
  const { parseUi, UI_TYPES } = await load();
  const skill = fs.readFileSync('resources/standard-skills/interaktive-antworten/SKILL.md', 'utf8').replace(/\r\n/g, '\n');
  const examples = [...skill.matchAll(/^```(?:json|ask-ui)\n([\s\S]*?)\n```$/gm)].map((m) => m[1]);
  assert.ok(examples.length >= 6, 'Beispiele im Skill');
  const seen = new Set();
  for (const ex of examples) {
    const r = parseUi(ex);
    assert.ok(r.ok, `Beispiel im Skill ungültig: ${r.error}\n${ex}`);
    r.blocks.forEach((b) => seen.add(b.typ));
  }
  for (const t of UI_TYPES) assert.match(skill, new RegExp(`### ${t}\\n`), `Skill beschreibt ${t}`);
  assert.match(parseUi('{typ: x}').error, /JSON/);
  assert.match(parseUi('{"typ":"tabelle"}').error, /Unbekannter Typ/);
  assert.match(parseUi('{"typ":"diagramm","beschriftungen":["a","b"],"reihen":[{"werte":[1]}]}').error, /so viele/);
  assert.match(parseUi('{"typ":"quiz","fragen":[{"frage":"?","antworten":["a","b"],"richtig":2}]}').error, /richtig/);
  assert.match(parseUi('{"typ":"rechner","eingaben":[{"name":"a"}],"ergebnisse":[{"formel":"a + b"}]}').error, /unbekannter Name «b»/);
  assert.ok(parseUi('[{"typ":"hinweis","text":"a"},{"typ":"auswahl","optionen":["x","y"]}]').ok, 'Liste von Bausteinen');
  assert.match(parseUi(JSON.stringify(Array(9).fill({ typ: 'hinweis', text: 'a' }))).error, /Höchstens 8/);
});

test('Formeln: Rechnen ohne eval, nur bekannte Namen und Funktionen', async () => {
  const { evalFormula, formatNumber } = await load();
  assert.equal(evalFormula('1 + 2 * 3'), 7);
  assert.equal(evalFormula('(1 + 2) * 3'), 9);
  assert.equal(evalFormula('2 ^ 3 ^ 2'), 512, 'Potenz rechts gebunden');
  assert.equal(evalFormula('-2 ^ 2'), -4);
  assert.equal(evalFormula('round(betrag * 12 / 52, 2)', { betrag: 50 }), 11.54);
  assert.equal(evalFormula('max(a, b, 3)', { a: 1, b: 2 }), 3);
  assert.equal(evalFormula('1,5 * 2'), 3, 'Komma als Dezimalzeichen');
  assert.ok(Math.abs(evalFormula('start * (1 + zins / 100) ^ jahre', { start: 1000, zins: 2, jahre: 10 }) - 1218.99) < 0.01);
  for (const bad of ['alert(1)', 'constructor', 'a.b', 'x => 1', '"text"', '1 +', 'toString()', '__proto__']) {
    assert.throws(() => evalFormula(bad, {}), `${bad} wird abgelehnt`);
  }
  assert.equal(formatNumber(1234567.5, 2), '1’234’567.5');
  assert.equal(formatNumber(12, 2), '12');
  assert.equal(formatNumber(NaN), '–');
});

test('HTML-Bausteine: Vorspann ohne Netz, Protokoll askui in beiden CSP', () => {
  const prelude = fs.readFileSync('resources/ask-ui/vorspann.html', 'utf8');
  assert.match(prelude, /window\.ask = /);
  assert.ok(!/https?:\/\//.test(prelude), 'Vorspann lädt nichts von aussen');
  const conf = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8')).app.security.csp;
  assert.match(conf['frame-src'], /askui:/);
  assert.ok(!/askui/.test(conf['script-src']), 'askui darf keine Skripte in die Oberfläche liefern');
  // Jede Klasse des Bausatzes, die der Skill nennt, gibt es im Vorspann
  const skill = fs.readFileSync('resources/standard-skills/interaktive-antworten/SKILL.md', 'utf8');
  const named = new Set([...skill.matchAll(/`(ask-[a-z]+)`/g)].map((m) => m[1]).filter((c) => !['ask-ui', 'ask-html', 'ask-hilfe'].includes(c)));
  assert.ok(named.size >= 10, `Bausatz im Skill: ${[...named]}`);
  for (const c of named) assert.match(prelude, new RegExp(`\\.${c}\\b`), `Klasse ${c} fehlt im Vorspann`);
  const ui = fs.readFileSync('src/renderer/src/components/UiBlocks.jsx', 'utf8');
  assert.match(ui, /sandbox="allow-scripts allow-forms"/, 'iframe ohne allow-same-origin');
  assert.ok(!/allow-same-origin|allow-top-navigation|allow-popups/.test(ui));
});
