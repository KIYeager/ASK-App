const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { Web, htmlToText, parseDuckDuckGo, checkUrl } = require('../src/main/web');

const response = (body, type = 'text/html; charset=utf-8', status = 200, url = '') => ({
  ok: status < 400, status, url, headers: { get: (h) => (h.toLowerCase() === 'content-type' ? type : null) },
  body: null, arrayBuffer: async () => (Buffer.isBuffer(body) ? body : Buffer.from(body)),
});

test('HTML wird zu lesbarem Text', () => {
  const { title, text } = htmlToText(`<html><head><title>Test &amp; Co</title><style>p{}</style></head><body>
    <nav>Menü</nav><header>Kopf</header><h2>Abschnitt</h2><p>Grüsse&nbsp;aus &laquo;Bern&raquo; &#8211; &#x263A;</p>
    <ul><li>eins</li><li>zwei</li></ul><table><tr><td>A</td><td>B</td></tr></table><script>alert(1)</script><footer>Fuss</footer></body></html>`);
  assert.strictEqual(title, 'Test & Co');
  assert.match(text, /## Abschnitt/);
  assert.match(text, /Grüsse aus «Bern» – ☺/);
  assert.match(text, /- eins\n- zwei/);
  assert.match(text, /\| A \| B/);
  assert.ok(!/Menü|Kopf|Fuss|alert|p\{\}/.test(text), text);
});

test('Treffer von DuckDuckGo, Weiterleitungen aufgelöst', () => {
  const html = `<table><tr><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.proxmox.com%2Fde%2Fnews&amp;rut=x" class='result-link'>Proxmox <b>News</b></a></td></tr>
    <tr><td class='result-snippet'>Neue <b>Version</b> 9</td></tr>
    <tr><td><a rel="nofollow" href="https://pve.proxmox.com/wiki/Roadmap" class='result-link'>Roadmap</a></td></tr>
    <tr><td class='result-snippet'>Pläne</td></tr>
    <tr><td><a href="https://duckduckgo.com/y.js?ad=1" class='result-link'>Werbung</a></td></tr></table>`;
  assert.deepStrictEqual(parseDuckDuckGo(html), [
    { url: 'https://www.proxmox.com/de/news', title: 'Proxmox News', snippet: 'Neue Version 9' },
    { url: 'https://pve.proxmox.com/wiki/Roadmap', title: 'Roadmap', snippet: 'Pläne' },
  ]);
});

test('Nur öffentliche http(s)-Adressen', () => {
  assert.strictEqual(checkUrl('https://example.ch/a').hostname, 'example.ch');
  for (const bad of ['file:///C:/x', 'http://localhost:3000', 'http://127.0.0.1/', 'http://192.168.1.10', 'http://10.0.0.1', 'http://172.20.1.1',
    'http://[::1]/', 'http://intranet/', 'ftp://example.ch', 'kein link']) {
    assert.throws(() => checkUrl(bad), Error, bad);
  }
  assert.strictEqual(checkUrl('http://127.0.0.1:8080/x', true).port, '8080');
});

test('Suche: ASK zuerst, sonst DuckDuckGo; Seite als Text, PDF gespeichert, lokale Weiterleitung gesperrt', async () => {
  const calls = [];
  const ddg = `<a href="https://a.ch/" class='result-link'>A</a><td class='result-snippet'>a</td>`;
  const fetchImpl = async (url) => {
    calls.push(String(url));
    if (url.includes('duckduckgo')) return response(ddg);
    if (url.includes('pdf')) return response(Buffer.from('%PDF-1.4 test'), 'application/pdf', 200, url);
    if (url.includes('weiter')) return response('<p>x</p>', 'text/html', 200, 'http://192.168.1.1/');
    return response(`<title>Seite</title><main><p>${'Inhalt '.repeat(3000)}</p></main>`, 'text/html', 200, url);
  };
  const ask = { webSearch: true, searchWeb: async (q) => [{ url: 'https://ask.ch/x', title: `ASK ${q}`, snippet: '' }] };
  const web = new Web({ ask, fetchImpl });
  assert.deepStrictEqual(await web.search('test'), { engine: 'ASK', results: [{ url: 'https://ask.ch/x', title: 'ASK test', snippet: '' }] });
  assert.strictEqual(calls.length, 0);

  const failing = new Web({ ask: { webSearch: true, searchWeb: async () => { throw new Error('HTTP 403'); } }, fetchImpl });
  const r = await failing.search('test');
  assert.strictEqual(r.engine, 'DuckDuckGo');
  assert.strictEqual(r.results[0].url, 'https://a.ch/');
  await failing.search('noch');
  assert.strictEqual(calls.filter((c) => c.includes('duckduckgo')).length, 2, 'nach Fehler gleich DuckDuckGo');

  const p = await web.page('https://seite.ch/');
  assert.strictEqual(p.title, 'Seite');
  assert.strictEqual(p.from, 0);
  assert.strictEqual(p.text.length, 12000);
  assert.ok(p.total > 12000);
  const p2 = await web.page('https://seite.ch/', { ab: p.to });
  assert.strictEqual(p2.from, 12000);

  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-web-'));
  try {
    const pdf = await web.page('https://seite.ch/datei.pdf', { root, chat: 'c1' });
    assert.strictEqual(pdf.pdf, '.ask/arbeit/c1/web/datei.pdf');
    assert.ok(fs.readFileSync(path.join(root, '.ask', 'arbeit', 'c1', 'web', 'datei.pdf')).toString().startsWith('%PDF'));
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
  await assert.rejects(web.page('https://seite.ch/weiter'), /lokalen Netz/);
});
