// Tests für den Markdown-Renderer der Oberfläche (node --test tests/).
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

// Der Renderer ist ein ES-Modul der Oberfläche (src/renderer/src/lib/md.mjs)
let md;
test.before(async () => {
  ({ renderMarkdown: md } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'md.mjs')).href));
});

test('nummerierte Liste behält die Zahlen über Leerzeile und Codeblock', () => {
  assert.strictEqual(md('1. a\n\n2. b'), '<ol><li>a</li><li>b</li></ol>');
  const html = md('1. a\n\n2. b\n```\nx\n```\n3. c');
  assert.match(html, /^<ol><li>a<\/li><li>b<\/li><\/ol><div class="codeblock">/);
  assert.match(html, /<ol start="3"><li>c<\/li><\/ol>$/);
  assert.strictEqual(md('5. x\n6. y'), '<ol start="5"><li>x</li><li>y</li></ol>');
});

test('Codezaun mit Infozeile aus mehreren Wörtern', () => {
  const html = md('```python title=x\nprint(1)\n```\nnachher **fett**');
  assert.match(html, /<span>python<\/span>/);
  assert.match(html, /<code>print\(1\)<\/code>/);
  assert.match(html, /<p>nachher <strong>fett<\/strong><\/p>$/);
});

test('Hervorhebung wirkt nicht in Adressen und nicht bei «2 * 3 * 4»', () => {
  assert.strictEqual(md('siehe https://x/_foo_/ hier'), '<p>siehe https://x/_foo_/ hier</p>');
  assert.strictEqual(md('[l](https://x/_foo_/)'), '<p><a href="https://x/_foo_/" title="https://x/_foo_/">l</a></p>');
  assert.strictEqual(md('2 * 3 * 4'), '<p>2 * 3 * 4</p>');
  assert.strictEqual(md('*schräg* und _auch_ und **fett**'), '<p><em>schräg</em> und <em>auch</em> und <strong>fett</strong></p>');
  assert.strictEqual(md('snake_case_name'), '<p>snake_case_name</p>');
  assert.match(md('[*a*](https://x/)'), /<a [^>]*><em>a<\/em><\/a>/);
});

test('Tabelle ohne führenden Strich in der Trennzeile, «|» in Inline-Code', () => {
  assert.strictEqual(md('a|b\n---|---\n1|2'),
    '<table><thead><tr><th>a</th><th>b</th></tr></thead><tbody><tr><td>1</td><td>2</td></tr></tbody></table>');
  const html = md('|a|b|\n---|---\n|`x|y`|2|');
  assert.match(html, /<td><code>x\|y<\/code><\/td><td>2<\/td>/);
  assert.match(md('|a|b|\n|---|---|\n|1|2|\n|3|4|'), /<td>3<\/td><td>4<\/td>/);
});

test('NUL-Zeichen im Text lassen den Renderer nicht werfen', () => {
  for (const s of ['x \u00005\u0000 y', '\u00000\u0000', '`a` \u00000\u0000 \u00009\u0000', '|\u00001\u0000|\n---|---\n\u00005\u0000|b']) {
    assert.doesNotThrow(() => md(s));
    assert.ok(!md(s).includes('\u0000'));
  }
  assert.strictEqual(md('x \u00005\u0000 y'), '<p>x 5 y</p>');
  assert.strictEqual(md('`a` \u00000\u0000'), '<p><code>a</code> 0</p>');
});

test('wirft nie, auch bei seltsamer Eingabe', () => {
  for (const s of [undefined, null, 42, '', '```', '|', '|\n|', '[](', '**', '_ _', '\u0000', '1.', '- ', '#']) assert.doesNotThrow(() => md(s));
});

test('Trennlinie für eine Zeile nur aus ---', () => {
  assert.strictEqual(md('a\n\n---\n\nb'), '<p>a</p><hr><p>b</p>');
  assert.strictEqual(md('---'), '<hr>');
});

test('Maskierung bleibt: kein fremdes HTML, keine javascript:-Links, kein Ausbruch aus Anführungszeichen', () => {
  const html = md('<img src=x onerror=alert(1)> <script>x</script>');
  assert.ok(!/<img|<script/i.test(html));
  assert.match(html, /&lt;img/);
  assert.ok(!/<a /.test(md('[x](javascript:alert(1))')));
  const link = md('[a](https://a"onmouseover="x)');
  assert.ok(!/"onmouseover/.test(link));
  assert.match(link, /href="https:\/\/a&quot;onmouseover=&quot;x"/);
  assert.ok(!/<b onerror|<b>/.test(md('```<b\n<b onerror=1>\n```')));
  assert.match(md('`<b>`'), /<code>&lt;b&gt;<\/code>/);
  assert.ok(!/<b>/.test(md('| <b>x</b> |\n|---|\n| <i> |')));
});

test('Verweise auf Unterlagen (Quellen, E84): nur Pfade im Ordner Quellen, ohne «..» und Laufwerk', async () => {
  const { parseSourceLink } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'md.mjs')).href);
  assert.strictEqual(md('[Skript, S. 2](quelle:Quellen/Skript%20Bio.pdf#seite=2)'),
    '<p><a class="quelle" href="quelle:Quellen/Skript%20Bio.pdf#seite=2" title="Quellen/Skript Bio.pdf, S. 2">Skript, S. 2</a></p>');
  assert.deepStrictEqual(parseSourceLink('quelle:Quellen/Woche%202/Blatt.docx'), { path: 'Quellen/Woche 2/Blatt.docx', seite: null });
  assert.deepStrictEqual(parseSourceLink('quelle:quellen/a.pdf#seite=14'), { path: 'quellen/a.pdf', seite: 14 });
  for (const bad of ['quelle:Quellen/../geheim.txt', 'quelle:Quellen/..%2Fgeheim.txt', 'quelle:C:/Windows/win.ini', 'quelle:/etc/passwd', 'quelle:Andere/a.pdf',
    'quelle:Quellen/', 'quelle:Quellen/a.pdf#seite=x', 'quelle:Quellen/%E0%A4%A', 'https://x.ch', '']) {
    assert.strictEqual(parseSourceLink(bad), null, bad);
  }
  assert.ok(!/<a /.test(md('[x](quelle:Quellen/../geheim.txt)')), 'ungültiger Verweis bleibt Text');
  const quoted = md('[a](quelle:Quellen/a"onmouseover="x.pdf)');
  assert.ok(!/"onmouseover/.test(quoted), quoted);
  assert.ok(!/<img/.test(md('[<img src=x>](quelle:Quellen/a.pdf)')), 'Text des Verweises bleibt maskiert');
});
