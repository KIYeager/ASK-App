// Werkzeuge web_search und web_fetch (ENTSCHEIDE E26). Laufen im Hauptprozess: Suche über die
// Websuche von ASK (dieselbe Suchmaschine wie auf der Webseite), sonst über DuckDuckGo; Seiten kommen
// als lesbarer Text statt als HTML. Vorher behalf sich das Modell mit curl und rohem HTML.
const path = require('node:path');
const fs = require('node:fs');

const MAX_RESULTS = 8;
const MAX_BYTES = 3 * 1024 * 1024;
const PAGE_CHARS = 12000; // Text pro web_fetch-Aufruf, weiter mit «ab»
const TIMEOUT = 20000;
const AGENT = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0 Safari/537.36 ASK-Desktop';

const ENTITIES = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", nbsp: ' ', shy: '', auml: 'ä', ouml: 'ö', uuml: 'ü', Auml: 'Ä', Ouml: 'Ö',
  Uuml: 'Ü', szlig: 'ß', eacute: 'é', egrave: 'è', agrave: 'à', ccedil: 'ç', laquo: '«', raquo: '»', ndash: '–', mdash: '—', hellip: '…',
  rsquo: '’', lsquo: '‘', rdquo: '”', ldquo: '“', copy: '©', reg: '®', euro: '€', middot: '·', bull: '•', times: '×' };

function decodeEntities(text) {
  return String(text).replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (m, e) => {
    if (e[0] === '#') {
      const code = e[1].toLowerCase() === 'x' ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return Number.isFinite(code) && code > 0 && code < 0x110000 ? String.fromCodePoint(code) : m;
    }
    return ENTITIES[e] ?? ENTITIES[e.toLowerCase()] ?? m;
  });
}

// HTML → lesbarer Text: ohne Skripte, Stile, Navigation, Kopf- und Fussbereich; Überschriften mit #,
// Listenpunkte mit -, Tabellenzellen mit |. Links bleiben nur als Text, damit er kurz bleibt.
// Ein Durchgang über den Text: Auch eine kaputte oder böswillige Seite (tausende offene Tags) darf den
// Hauptprozess nicht blockieren. Formulare bleiben drin: Manche Seiten stecken ganz in einem <form>.
const RAW = new Set(['script', 'style']); // Inhalt ist kein HTML, endet erst beim schliessenden Tag
const DROP = new Set(['script', 'style', 'noscript', 'svg', 'template', 'iframe', 'head', 'nav', 'footer', 'button', 'select']);
const BREAK = new Set(['p', 'div', 'section', 'article', 'tr', 'table', 'ul', 'ol', 'blockquote', 'pre', 'dd', 'dt', 'figure', 'main']);

function htmlToText(html, strict = true) {
  const h = String(html);
  const lower = h.toLowerCase();
  const out = { all: [], main: [], body: [] }; // main: Inhalt von <main>/<article>; body: ohne <header>
  let title = '';
  let dropped = []; // offene Tags, deren Inhalt wegfällt
  let inMain = 0;
  let inHeader = 0;
  let inTitle = false;
  const emit = (text) => {
    if (inTitle) { title += text; return; }
    if (dropped.length) return;
    out.all.push(text);
    if (inMain) out.main.push(text);
    if (!inHeader) out.body.push(text);
  };
  let i = 0;
  let gt = -1; // Position des nächsten «>», damit nicht jedes «<» den Rest neu absucht
  const ends = new Map(); // «</script» usw.: einmal nicht gefunden heisst nie mehr suchen
  const closing = (name, from) => {
    const key = `</${name}`;
    if (ends.get(key) === -1) return -1;
    const at = lower.indexOf(key, from);
    ends.set(key, at);
    return at;
  };
  while (i < h.length) {
    const lt = h.indexOf('<', i);
    if (lt < 0) { emit(h.slice(i)); break; }
    if (lt > i) emit(h.slice(i, lt));
    if (h.startsWith('<!--', lt)) {
      const end = ends.get('-->') === -1 ? -1 : h.indexOf('-->', lt + 4);
      ends.set('-->', end);
      if (end < 0) break; // offener Kommentar: der Rest gehört dazu
      i = end + 3;
      continue;
    }
    if (gt < lt && gt !== -2) { gt = h.indexOf('>', lt); if (gt < 0) gt = -2; }
    if ((h[lt + 1] === '!' || h[lt + 1] === '?') && gt >= 0) { i = gt + 1; continue; } // <!DOCTYPE …>, <?xml …?>
    const m = /^<(\/?)([a-zA-Z][a-zA-Z0-9]*)/.exec(h.slice(lt, lt + 40));
    if (!m || gt === -2) { emit(m ? '' : '<'); if (m) break; i = lt + 1; continue; } // kein Tag, oder Tag ohne Ende
    const close = m[1] === '/';
    const name = m[2].toLowerCase();
    i = gt + 1;
    if (!close && RAW.has(name)) {
      const end = closing(name, i);
      if (end < 0) break;
      i = end;
      continue;
    }
    if (name === 'title') { inTitle = !close && !title; continue; }
    if (DROP.has(name) && (strict || name === 'head')) {
      if (!close) { if (h[gt - 1] !== '/') dropped.push(name); } else if (dropped.includes(name)) dropped = dropped.slice(0, dropped.lastIndexOf(name));
      emit(' ');
      continue;
    }
    if (name === 'main' || name === 'article') inMain = Math.max(0, inMain + (close ? -1 : 1));
    if (name === 'header') { inHeader = Math.max(0, inHeader + (close ? -1 : 1)); emit(' '); continue; }
    if (/^h[1-6]$/.test(name)) emit(close ? '\n\n' : `\n\n${'#'.repeat(Number(name[1]))} `);
    else if (name === 'li' && !close) emit('\n- ');
    else if (name === 'br') emit('\n');
    else if ((name === 'td' || name === 'th') && !close) emit(' | ');
    else if (BREAK.has(name) && close) emit('\n');
    else emit(' ');
  }
  const clean = (parts) => decodeEntities(parts.join('')).replace(/[ \t\f\v\u00a0]+/g, ' ').split('\n').map((l) => l.trim())
    .filter((l, n, all) => l || (all[n - 1] && all[n - 1].trim())).join('\n').replace(/\n{3,}/g, '\n\n').trim();
  // Ein Bereich wie <nav> wurde nie geschlossen: Dann fiele der ganze Rest weg. Nochmals, ohne Bereiche auszulassen.
  if (strict && dropped.some((name) => name !== 'head')) return htmlToText(h, false);
  const main = clean(out.main);
  return { title: decodeEntities(title.replace(/\s+/g, ' ').trim()), text: main.length > 400 ? main : clean(out.body) };
}

// Ergebnisse von lite.duckduckgo.com: Links sind teils Weiterleitungen (…/l/?uddg=<Adresse>)
function parseDuckDuckGo(html) {
  const out = [];
  const links = [...String(html).matchAll(/<a[^>]*class=['"]result-link['"][^>]*>[\s\S]*?<\/a>/gi)];
  const snippets = [...String(html).matchAll(/<td[^>]*class=['"]result-snippet['"][^>]*>([\s\S]*?)<\/td>/gi)].map((m) => m[1]);
  links.forEach((m, i) => {
    let href = decodeEntities(/href=['"]([^'"]+)['"]/i.exec(m[0])?.[1] || '');
    const redirect = /[?&]uddg=([^&]+)/.exec(href);
    if (redirect) href = decodeURIComponent(redirect[1]);
    if (href.startsWith('//')) href = `https:${href}`;
    if (!/^https?:\/\//i.test(href) || /duckduckgo\.com\/(y\.js|l\/)/.test(href)) return;
    const strip = (s) => decodeEntities(String(s || '').replace(/<[^>]+>/g, ' ')).replace(/\s+/g, ' ').trim();
    out.push({ url: href, title: strip(m[0]), snippet: strip(snippets[i]) });
  });
  return out;
}

// Nur öffentliche http(s)-Adressen; keine Adressen im eigenen Rechner oder Schulnetz
function checkUrl(raw, allowLocal = false) {
  let url;
  try {
    url = new URL(String(raw).trim());
  } catch {
    throw new Error(`Keine gültige Adresse: ${raw}`);
  }
  if (!/^https?:$/.test(url.protocol)) throw new Error('Nur http- und https-Adressen.');
  const host = url.hostname.replace(/^\[|\]$/g, '').toLowerCase();
  // IPv4 in IPv6-Schreibweise (::ffff:7f00:1 ist 127.0.0.1) zählt wie die IPv4-Adresse
  const mapped = /^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$/.exec(host);
  const v4 = mapped ? [parseInt(mapped[1], 16) >> 8, parseInt(mapped[1], 16) & 255, parseInt(mapped[2], 16) >> 8, parseInt(mapped[2], 16) & 255].join('.') : host;
  const local = host === 'localhost' || host.endsWith('.localhost') || host.endsWith('.local') || host.endsWith('.internal') || host.endsWith('.lan')
    || host === '::1' || host === '::' || /^fe[89ab][0-9a-f]:/.test(host)
    || /^(127|10|0)\./.test(v4) || /^192\.168\./.test(v4) || /^169\.254\./.test(v4) || /^172\.(1[6-9]|2\d|3[01])\./.test(v4)
    || /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./.test(v4) // gemeinsam genutzter Bereich von Anbietern und VPN
    || /^(fc|fd)[0-9a-f]{2}:/.test(host) || !host.includes('.') && !host.includes(':');
  if (local && !allowLocal) throw new Error('Adressen im lokalen Netz oder auf diesem Rechner sind gesperrt.');
  return url;
}

async function readLimited(res) {
  const reader = res.body?.getReader();
  if (!reader) return Buffer.from(await res.arrayBuffer());
  const chunks = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.length;
    if (size > MAX_BYTES) { reader.cancel().catch(() => {}); throw new Error('Die Seite ist grösser als 3 MB.'); }
    chunks.push(Buffer.from(value));
  }
  return Buffer.concat(chunks);
}

function charsetOf(type, buf) {
  const m = /charset=([\w-]+)/i.exec(type) || /<meta[^>]+charset=['"]?([\w-]+)/i.exec(buf.subarray(0, 2048).toString('latin1'));
  const cs = (m?.[1] || 'utf-8').toLowerCase();
  return ['utf-8', 'utf8', 'iso-8859-1', 'latin1', 'windows-1252', 'utf-16le'].includes(cs) ? cs : 'utf-8';
}

class Web {
  // ask: AskSession (searchWeb, webSearch), fetchImpl: fetch mit Proxy von Windows (net.fetch)
  constructor({ ask, fetchImpl, allowLocal = false }) {
    this.ask = ask;
    this.fetch = fetchImpl;
    this.allowLocal = allowLocal;
    this.askFailed = false; // Websuche von ASK ging nicht: in dieser Sitzung gleich DuckDuckGo
  }

  async get(url, signal, accept = 'text/html,*/*;q=0.8') {
    const timeout = AbortSignal.timeout(TIMEOUT);
    const both = signal ? AbortSignal.any([signal, timeout]) : timeout;
    try {
      return await this.fetch(url, { signal: both, redirect: 'follow', headers: { 'User-Agent': AGENT, Accept: accept, 'Accept-Language': 'de-CH,de;q=0.9,en;q=0.7' } });
    } catch (err) {
      if (timeout.aborted && !signal?.aborted) throw new Error('Keine Antwort innerhalb von 20 Sekunden.');
      throw err;
    }
  }

  // { engine, results: [{ url, title, snippet }] }
  async search(query, { signal } = {}) {
    query = String(query || '').trim();
    if (!query) throw new Error('Leere Suchanfrage.');
    let note = '';
    if (this.ask?.webSearch && !this.askFailed) {
      try {
        const results = await this.ask.searchWeb(query, signal);
        if (results.length) return { engine: 'ASK', results: results.slice(0, MAX_RESULTS) };
        note = 'ASK fand nichts';
      } catch (err) {
        if (signal?.aborted) throw err;
        this.askFailed = true;
        note = err.message;
      }
    }
    const res = await this.get(`https://lite.duckduckgo.com/lite/?q=${encodeURIComponent(query)}&kl=ch-de`, signal);
    if (!res.ok) throw new Error(`DuckDuckGo antwortet mit HTTP ${res.status}${note ? ` (${note})` : ''}.`);
    const results = parseDuckDuckGo((await readLimited(res)).toString('utf8')).slice(0, MAX_RESULTS);
    return { engine: 'DuckDuckGo', results, note };
  }

  // Seite als Text. ab: ab diesem Zeichen weiterlesen. PDF wird gespeichert und mit read_file gelesen.
  async page(rawUrl, { signal, ab = 0, root, chat } = {}) {
    const url = checkUrl(rawUrl, this.allowLocal);
    const res = await this.get(url.href, signal);
    if (!res.ok) throw new Error(`Die Seite antwortet mit HTTP ${res.status}.`);
    const final = res.url || url.href;
    checkUrl(final, this.allowLocal); // Weiterleitung ins lokale Netz
    const type = res.headers.get('content-type') || '';
    const buf = await readLimited(res);
    if (/pdf/i.test(type) || buf.subarray(0, 5).toString('latin1') === '%PDF-') {
      if (!root || !chat) throw new Error('PDF kann hier nicht gespeichert werden.');
      const name = (path.basename(new URL(final).pathname).replace(/[^\w.-]+/g, '_') || 'dokument').replace(/(\.pdf)?$/i, '.pdf');
      const rel = `.ask/arbeit/${chat}/web/${name}`;
      fs.mkdirSync(path.join(root, '.ask', 'arbeit', chat, 'web'), { recursive: true });
      fs.writeFileSync(path.join(root, ...rel.split('/')), buf);
      return { url: final, title: name, text: '', pdf: rel, total: 0, from: 0, to: 0 };
    }
    if (!/html|xml|text|json/i.test(type) && type) throw new Error(`Kein Text, sondern ${type.split(';')[0]}.`);
    const raw = new TextDecoder(charsetOf(type, buf)).decode(buf);
    const { title, text } = /html|xml/i.test(type) || /<html|<body|<p[\s>]/i.test(raw.slice(0, 5000)) ? htmlToText(raw) : { title: '', text: raw.trim() };
    const from = Math.max(0, Math.min(Number(ab) || 0, text.length));
    return { url: final, title: title || new URL(final).hostname, text: text.slice(from, from + PAGE_CHARS), total: text.length, from, to: Math.min(text.length, from + PAGE_CHARS) };
  }
}

module.exports = { Web, htmlToText, parseDuckDuckGo, checkUrl, decodeEntities, PAGE_CHARS };
