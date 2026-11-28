// Kleiner Markdown-Renderer für Antworten des Modells: Überschriften, Absätze, Listen, Code,
// Tabellen, fett, kursiv, durchgestrichen, Links, Zitate. Alles wird zuerst maskiert; es entsteht nie fremdes HTML.
const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

// Hält fertig erzeugtes HTML (Code, Links, Adressen) zurück, damit Hervorhebung nicht hineinwirkt.
// Eigene NUL-Zeichen im Text werden vorher entfernt: sie dürfen nie wie Platzhalter wirken.
function inline(text) {
  const stash = [];
  const hold = (html) => `\u0000${stash.push(html) - 1}\u0000`;
  const urls = (t) => t.replace(/https?:\/\/[^\s]*[^\s*_]/g, (u) => hold(u));
  const emph = (t) => t
    .replace(/\*\*(?!\s)([^*\n]*[^*\s])\*\*/g, '<strong>$1</strong>')
    .replace(/~~(?!\s)([^~\n]*[^~\s])~~/g, '<del>$1</del>')
    .replace(/(^|[^*\w])\*(?!\s)([^*\n]*[^*\s])\*(?!\w)/g, '$1<em>$2</em>')
    .replace(/(^|[^_\w])_(?!\s)([^_\n]*[^_\s])_(?!\w)/g, '$1<em>$2</em>');
  let s = String(text).replace(/\u0000/g, '').replace(/`([^`\n]+)`/g, (_m, c) => hold(`<code>${esc(c)}</code>`));
  s = esc(s).replace(/\[([^\]\n]+)\]\((https?:[^)\s]+)\)/g, (_m, label, url) => hold(`<a href="${url}" title="${url}">${emph(urls(label))}</a>`));
  // Verweis auf eine Unterlage im Ordner «Quellen» (ENTSCHEIDE E84); ungültige Pfade bleiben Text
  s = s.replace(/\[([^\]\n]+)\]\((quelle:[^)\s]+)\)/g, (m, label, url) => {
    const ref = parseSourceLink(url.replace(/&amp;/g, '&'));
    return ref ? hold(`<a class="quelle" href="${url}" title="${esc(ref.path)}${ref.seite ? `, S. ${ref.seite}` : ''}">${emph(label)}</a>`) : m;
  });
  s = emph(urls(s));
  for (let n = 0; n < 10 && /\u0000\d+\u0000/.test(s); n++) s = s.replace(/\u0000(\d+)\u0000/g, (_m, i) => stash[Number(i)] ?? '');
  return s;
}

// Verweis `quelle:Quellen/Skript.pdf#seite=14` → { path, seite }; nur relative Pfade im Ordner «Quellen», ohne «..»
export function parseSourceLink(href) {
  const m = /^quelle:([^#?]+)(?:#seite=(\d{1,5}))?$/.exec(String(href || ''));
  if (!m) return null;
  let path;
  try { path = decodeURIComponent(m[1]).replace(/\\/g, '/'); } catch { return null; }
  const parts = path.split('/');
  if (!/^quellen$/i.test(parts[0]) || parts.length < 2 || parts.some((p) => !p || p === '.' || p === '..' || /[:\u0000]/.test(p))) return null;
  return { path, seite: m[2] ? Number(m[2]) : null };
}

// Zellen einer Tabellenzeile; ein «|» in Inline-Code oder als «\|» trennt nicht
const cells = (line) => line.replace(/[\u0001\u0002]/g, '')
  .replace(/`[^`\n]*`/g, (c) => c.replace(/\|/g, '\u0001')).replace(/\\\|/g, '\u0002')
  .replace(/^\s*\|/, '').replace(/\|\s*$/, '').split('|')
  .map((c) => c.trim().replace(/\u0001/g, '|').replace(/\u0002/g, '|'));
const isSep = (line) => /\|/.test(line) && /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)*\|?\s*$/.test(line);

function table(rows) {
  const head = cells(rows[0]);
  // Ausrichtung aus der Trennzeile: ---: rechts, :---: Mitte
  const align = cells(rows[1]).map((c) => (/^:-+:$/.test(c) ? 'center' : /-:$/.test(c) ? 'right' : ''));
  const attr = (i) => (align[i] ? ` class="${align[i]}"` : '');
  const body = rows.slice(2).map(cells);
  return `<table><thead><tr>${head.map((c, i) => `<th${attr(i)}>${inline(c)}</th>`).join('')}</tr></thead><tbody>${
    body.map((r) => `<tr>${r.map((c, i) => `<td${attr(i)}>${inline(c)}</td>`).join('')}</tr>`).join('')}</tbody></table>`;
}

function renderBlocks(markdown) {
  const lines = String(markdown || '').replace(/\r\n/g, '\n').split('\n');
  const out = [];
  let para = [];
  let list = null; // { type: 'ul'|'ol', items: [] }
  const flushPara = () => {
    if (para.length) out.push(`<p>${para.map(inline).join('<br>')}</p>`);
    para = [];
  };
  const flushList = () => {
    if (list) out.push(`<${list.type}${list.start > 1 ? ` start="${list.start}"` : ''}>${list.items.map((i) => `<li>${inline(i)}</li>`).join('')}</${list.type}>`);
    list = null;
  };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const fence = /^\s*```\s*([^\s`]*)[^`]*$/.exec(line); // Infozeile: erstes Wort, Rest ignoriert
    if (fence) {
      flushPara(); flushList();
      const code = [];
      for (i++; i < lines.length && !/^\s*```\s*$/.test(lines[i]); i++) code.push(lines[i]);
      out.push(`<div class="codeblock"><div class="codehead"><span>${esc(fence[1] || 'Code')}</span>`
        + `<button class="copy" type="button">Kopieren</button></div><pre class="code"><code>${esc(code.join('\n'))}</code></pre></div>`);
      continue;
    }
    if (line.includes('|') && isSep(lines[i + 1] || '')) {
      flushPara(); flushList();
      const rows = [line, lines[i + 1]];
      for (i += 2; i < lines.length && lines[i].trim() && lines[i].includes('|'); i++) rows.push(lines[i]);
      i--;
      out.push(table(rows));
      continue;
    }
    if (/^\s*(?:-{3,}|\*{3,}|_{3,})\s*$/.test(line)) { flushPara(); flushList(); out.push('<hr>'); continue; }
    if (/^\s*>\s?/.test(line)) {
      flushPara(); flushList();
      const quote = [];
      for (; i < lines.length && /^\s*>\s?/.test(lines[i]); i++) quote.push(lines[i].replace(/^\s*>\s?/, ''));
      i--;
      out.push(`<blockquote>${renderBlocks(quote.join('\n'))}</blockquote>`);
      continue;
    }
    const heading = /^(#{1,4})\s+(.*)$/.exec(line);
    if (heading) {
      flushPara(); flushList();
      out.push(`<h${heading[1].length + 2}>${inline(heading[2])}</h${heading[1].length + 2}>`);
      continue;
    }
    const item = /^\s*(?:([-*•])|(\d+)[.)])\s+(.*)$/.exec(line);
    if (item) {
      flushPara();
      const type = item[2] ? 'ol' : 'ul';
      if (!list || list.type !== type) { flushList(); list = { type, items: [], start: Number(item[2]) || 1 }; }
      list.items.push(item[3]);
      continue;
    }
    if (!line.trim()) { flushPara(); continue; } // Liste bleibt offen: dieselbe Liste geht nach der Leerzeile weiter
    if (list && /^\s{2,}\S/.test(line)) { list.items[list.items.length - 1] += ` ${line.trim()}`; continue; }
    flushList();
    para.push(line);
  }
  flushPara(); flushList();
  return out.join('');
}

// Darf nie werfen: im Notfall der maskierte Rohtext
function render(markdown) {
  try {
    return renderBlocks(markdown);
  } catch {
    return `<p>${esc(String(markdown || ''))}</p>`;
  }
}

export { render as renderMarkdown, esc };