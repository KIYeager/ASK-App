// Tests für ASK Docs in der Oberfläche (ENTSCHEIDE E81): Markdown ↔ Editor (src/renderer/src/lib/docMd.mjs).
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

let mdToHtml, htmlToMd, titleOf, parseBlocks;
test.before(async () => {
  ({ mdToHtml, htmlToMd, titleOf, parseBlocks } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'docMd.mjs')).href));
});

// Kleiner HTML-Leser für die Tests: baut Knoten mit nodeType, nodeName, childNodes, textContent, getAttribute
const VOID = new Set(['br', 'hr', 'img', 'meta', 'input']);
const decode = (s) => s.replace(/&nbsp;/g, '\u00a0').replace(/&lt;/g, '<').replace(/&gt;/g, '>').replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&amp;/g, '&');
function parse(html) {
  const root = { nodeType: 1, nodeName: 'BODY', childNodes: [], attrs: {} };
  const stack = [root];
  const re = /<\/?([a-zA-Z0-9]+)((?:\s+[a-zA-Z-]+(?:="[^"]*")?)*)\s*\/?>|([^<]+)/g;
  let m;
  while ((m = re.exec(html))) {
    const top = stack[stack.length - 1];
    if (m[3] !== undefined) { top.childNodes.push({ nodeType: 3, nodeName: '#text', textContent: decode(m[3]) }); continue; }
    const name = m[1].toLowerCase();
    if (m[0].startsWith('</')) { while (stack.length > 1 && stack.pop().nodeName.toLowerCase() !== name); continue; }
    const attrs = {};
    for (const a of m[2].matchAll(/([a-zA-Z-]+)(?:="([^"]*)")?/g)) attrs[a[1].toLowerCase()] = decode(a[2] || '');
    const el = { nodeType: 1, nodeName: name.toUpperCase(), childNodes: [], attrs, getAttribute: (k) => attrs[k] ?? null };
    Object.defineProperty(el, 'textContent', { get() { return this.childNodes.map((c) => c.textContent).join(''); } });
    top.childNodes.push(el);
    if (!VOID.has(name)) stack.push(el);
  }
  return root;
}
const round = (md) => htmlToMd(parse(mdToHtml(md)));

test('Markdown wird zu sauberem HTML', () => {
  assert.strictEqual(mdToHtml('# Titel\n\nText **fett** und *kursiv*.'), '<h1>Titel</h1><p>Text <strong>fett</strong> und <em>kursiv</em>.</p>');
  assert.strictEqual(mdToHtml('- a\n  - b\n- c'), '<ul><li>a<ul><li>b</li></ul></li><li>c</li></ul>');
  assert.strictEqual(mdToHtml('1. eins\n2. zwei'), '<ol><li>eins</li><li>zwei</li></ol>');
  assert.strictEqual(mdToHtml('- a\n\n1. b'), '<ul><li>a</li></ul><ol><li>b</li></ol>', 'Wechsel der Listenart beginnt eine neue Liste');
  assert.strictEqual(mdToHtml('| A | B |\n|---|---|\n| 1 | 2 |'), '<table><thead><tr><th>A</th><th>B</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>');
  assert.strictEqual(mdToHtml('> Zitat'), '<blockquote><p>Zitat</p></blockquote>');
  assert.strictEqual(mdToHtml('```\n<b>x</b>\n```'), '<pre><code>&lt;b&gt;x&lt;/b&gt;</code></pre>');
  assert.strictEqual(mdToHtml(''), '<p><br></p>');
});

test('nichts Fremdes im HTML: Tags maskiert, nur Web-Links', () => {
  const html = mdToHtml('<script>alert(1)</script> [x](javascript:alert(1)) [y](https://tfbern.ch) <img src=x onerror=alert(1)>');
  assert.ok(!html.includes('<script') && !html.includes('<img') && !html.includes('javascript:'));
  assert.ok(html.includes('<a href="https://tfbern.ch">y</a>'));
  assert.ok(mdToHtml('[a](https://x.ch/"onmouseover="alert(1))').includes('&quot;'), 'Anführungszeichen in der Adresse bleiben maskiert');
});

test('hin und zurück bleibt das Markdown gleich', () => {
  for (const md of [
    '# Bericht\n\n## Einleitung\n\nText mit **fett**, *kursiv*, ~~weg~~ und `code`.\n',
    '- a\n  - b\n- c\n',
    '1. eins\n   1. unter eins\n2. zwei\n',
    '> Ein Zitat\n',
    '```\nlet x = 1;\n```\n',
    '| A | B |\n| --- | --- |\n| 1 | 2 \\| 3 |\n',
    'Text mit [Link](https://example.org).\n',
    'Zeile eins\\\nZeile zwei\n',
    '---\n',
  ]) {
    assert.strictEqual(round(md), md, md);
  }
});

test('Zeichen aus dem Text werden keine Auszeichnung', () => {
  const p = (t) => parse(`<p>${t}</p>`);
  assert.strictEqual(htmlToMd(p('3 * 4 = 12 und [Notiz]')), '3 \\* 4 = 12 und \\[Notiz\\]\n');
  assert.strictEqual(htmlToMd(p('datei_name_v2 und _x_')), 'datei_name_v2 und \\_x\\_\n');
  assert.strictEqual(htmlToMd(p('# keine Überschrift')), '\\# keine Überschrift\n');
  assert.strictEqual(htmlToMd(p('1. kein Punkt')), '1\\. kein Punkt\n');
  assert.strictEqual(htmlToMd(p('- kein Punkt')), '\\- kein Punkt\n');
  for (const t of ['3 * 4 = 12', '# keine Überschrift', '1. kein Punkt', '- kein Punkt', 'a_b_c', '~~nicht~~']) {
    assert.strictEqual(htmlToMd(parse(mdToHtml(htmlToMd(p(t))))), htmlToMd(p(t)), t);
    assert.ok(!/<(h1|li|s|em|strong)>/.test(mdToHtml(htmlToMd(p(t)))), t);
  }
});

test('eingefügtes HTML aus Word und Browser', () => {
  const word = '<div><p class="MsoTitle"><span style="font-weight:bold">Wichtig</span> und <span style="font-style:italic">schräg</span></p>'
    + '<ul><li><p>Punkt <b>eins</b></p></li><li>zwei</li></ul><script>x()</script><table><tr><td>a</td><td>b|c</td></tr></table></div>';
  assert.strictEqual(htmlToMd(parse(word)), '**Wichtig** und *schräg*\n\n- Punkt **eins**\n- zwei\n\n| a | b\\|c |\n| --- | --- |\n');
  // Text direkt im Editor (contentEditable), div als Absatz, nbsp
  assert.strictEqual(htmlToMd(parse('erste&nbsp;Zeile<div>zweite</div><div><br></div>')), 'erste Zeile\n\nzweite\n');
  assert.strictEqual(htmlToMd(parse('<p><b> fett </b>x</p>')), ' **fett** x\n'.trim() + '\n');
  assert.strictEqual(htmlToMd(parse('<p><a href="file:///C:/x">lokal</a></p>')), 'lokal\n', 'nur Web-Links bleiben Links');
  // nach einem Zeilenumbruch im Absatz wird «- » oder «# » nicht zur Liste oder Überschrift
  for (const md of ['Notiz\\\n\\- Punkt\n', 'a\\\n\\# b\n', 'x\\\n1\\. y\n']) assert.strictEqual(round(md), md);
  assert.strictEqual(htmlToMd(parse('<p>Notiz<br>- Punkt</p>')), 'Notiz\\\n\\- Punkt\n');
});

test('Titel und Blöcke', () => {
  assert.strictEqual(titleOf('Vorwort\n# **Titel**\n', 'docs/x.md'), 'Titel');
  assert.strictEqual(titleOf('', 'docs/plan.md'), 'plan');
  const b = parseBlocks('- a\n\n- b');
  assert.deepStrictEqual(b.filter((x) => x.type === 'li').map((x) => x.text), ['a', 'b']);
  assert.strictEqual(b.filter((x) => x.type === 'listbreak').length, 1, 'Leerzeile zwischen Punkten trennt die Liste nicht');
});

test('Bilder im Doc: nur relative Pfade, hin und zurück', async () => {
  const { safeImagePath } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'docMd.mjs')).href);
  const html = mdToHtml('Text\n\n![Zelle mit Kern](bilder/zelle.png)', { image: (src) => `askfile://datei/docs/${src}` });
  assert.ok(html.includes('<img src="askfile://datei/docs/bilder/zelle.png" alt="Zelle mit Kern" data-src="bilder/zelle.png">'), html);
  assert.strictEqual(round('Text\n\n![Zelle mit Kern](bilder/zelle.png)\n'), 'Text\n\n![Zelle mit Kern](bilder/zelle.png)\n');
  assert.strictEqual(round('Ein ![Logo](../bilder/logo.png) im Satz.\n'), 'Ein ![Logo](../bilder/logo.png) im Satz.\n');
  for (const bad of ['https://x.ch/a.png', 'C:/x.png', '/etc/x.png', 'file:///x.png', 'a\\b.png', 'javascript:alert(1)']) {
    assert.ok(!safeImagePath(bad), bad);
    assert.ok(!mdToHtml(`![x](${bad})`).includes('<img'), bad);
  }
  // Bilder von Webseiten (ohne data-src) fallen beim Einfügen weg
  assert.strictEqual(htmlToMd(parse('<p>a<img src="https://x.ch/t.png">b</p>')), 'ab\n');
});
