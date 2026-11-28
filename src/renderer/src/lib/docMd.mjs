// ASK Docs (ENTSCHEIDE E81): Markdown für den Editor im Reiter «Docs» in HTML und zurück. Gespeichert wird immer
// Markdown (docs/….md im Projekt); der Editor zeigt es formatiert wie in Word. Dasselbe Markdown versteht der Export nach
// Word (src-tauri/core/src/docx.rs): Überschriften, Absätze, Listen (verschachtelt), Zitate, Code, Tabellen, Trennlinien,
// **fett**, *kursiv*, ~~durchgestrichen~~, `Code`, [Links](https://…) und Bilder ![Beschreibung](bilder/name.png)
// (Pfad relativ zum Doc, E82; die Adresse zum Anzeigen liefert mdToHtml(md, { image })).
// mdToHtml erzeugt nur eigene, maskierte Tags. htmlToMd liest einen DOM-Baum (oder einen Nachbau mit nodeType, nodeName,
// childNodes, textContent, getAttribute), auch eingefügtes HTML aus Word oder dem Browser; was es nicht kennt, wird Text.
// Reine Funktionen ohne DOM, damit sie sich mit node --test prüfen lassen.

const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');
const SAFE_URL = /^(https?:\/\/|mailto:)/i;

// ---------- Markdown → Blöcke (wie parse_blocks in docx.rs) ----------

function heading(t) {
  const m = /^(#{1,6}) (.*)$/.exec(t);
  return m ? { level: m[1].length, text: m[2].trim().replace(/\s#+$/, '').trim() } : null;
}
function listItem(line) {
  const m = /^([ \t]*)([-*+]|\d{1,9}[.)]) (.*)$/.exec(line);
  if (!m) return null;
  const indent = [...m[1]].reduce((n, c) => n + (c === '\t' ? 4 : 1), 0);
  return { indent, ordered: /\d/.test(m[2]), text: m[3] };
}
const isRule = (t) => { const c = t.replace(/\s/g, ''); return c.length >= 3 && /^(-+|\*+|_+)$/.test(c); };
const isSep = (t) => /^[\s|:-]+$/.test(t) && t.includes('-') && t.includes('|');
function cells(line) {
  let t = line.trim();
  if (t.startsWith('|')) t = t.slice(1);
  if (t.endsWith('|') && !t.endsWith('\\|')) t = t.slice(0, -1);
  const out = [''];
  for (let i = 0; i < t.length; i++) {
    if (t[i] === '\\' && t[i + 1] === '|') { out[out.length - 1] += '|'; i++; } else if (t[i] === '|') out.push(''); else out[out.length - 1] += t[i];
  }
  return out.map((c) => c.trim());
}

export function parseBlocks(md) {
  const lines = String(md || '').replace(/\r\n/g, '\n').split('\n');
  const out = [];
  let para = null;
  let inList = false;
  let topOrdered = null;
  let indents = [];
  const flush = () => { if (para !== null) { out.push({ type: 'p', text: para }); para = null; } };
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const t = line.trim();
    if (t.startsWith('```') || t.startsWith('~~~')) {
      flush(); inList = false;
      const fence = t.slice(0, 3);
      const code = [];
      for (i++; i < lines.length && !lines[i].trim().startsWith(fence); i++) code.push(lines[i]);
      out.push({ type: 'code', text: code.join('\n') });
      continue;
    }
    if (!t) {
      flush();
      const next = lines.slice(i + 1).find((l) => l.trim());
      if (!next || !listItem(next)) inList = false;
      continue;
    }
    const h = heading(t);
    if (h) { flush(); inList = false; out.push({ type: 'h', ...h }); continue; }
    if (isRule(t) && para === null) { inList = false; out.push({ type: 'hr' }); continue; }
    if (t.startsWith('|') && i + 1 < lines.length && isSep(lines[i + 1])) {
      flush(); inList = false;
      const rows = [cells(t)];
      for (i += 2; i < lines.length && lines[i].trim().startsWith('|'); i++) rows.push(cells(lines[i]));
      i--;
      out.push({ type: 'table', rows });
      continue;
    }
    const item = listItem(line);
    if (item) {
      flush();
      if (!inList) indents = [];
      while (indents.length && item.indent < indents[indents.length - 1]) indents.pop();
      if (!indents.length || item.indent > indents[indents.length - 1]) indents.push(item.indent);
      const level = Math.min(indents.length - 1, 8);
      if (!inList || (level === 0 && topOrdered !== item.ordered)) out.push({ type: 'listbreak' });
      inList = true;
      if (level === 0) topOrdered = item.ordered;
      out.push({ type: 'li', level, ordered: item.ordered, text: item.text });
      continue;
    }
    if (t.startsWith('>')) {
      flush(); inList = false;
      const q = [t.slice(1).trimStart()];
      while (i + 1 < lines.length && lines[i + 1].trim().startsWith('>')) q.push(lines[++i].trim().slice(1).trimStart());
      for (const part of q.join('\n').split('\n\n')) out.push({ type: 'quote', text: part });
      continue;
    }
    if (inList && para === null && /^[ \t]/.test(line) && out[out.length - 1]?.type === 'li') {
      out[out.length - 1].text += ` ${t}`;
      continue;
    }
    inList = false;
    const hard = line.endsWith('  ') || line.endsWith('\\');
    const text = t.replace(/\\$/, '');
    if (para === null) para = text; else if (para.endsWith('\n')) para += text; else para += ` ${text}`;
    if (hard) para += '\n';
  }
  flush();
  return out;
}

// ---------- Text mit Auszeichnung → HTML ----------

const ESCAPABLE = '\\`*_~[]()#|>!-+.';

// Bildpfad im Doc: nur relativ, ohne Laufwerk, Schema oder Rückstrich (die Auflösung prüft die Oberfläche)
export const safeImagePath = (src) => !!src && !/^[a-z][a-z0-9+.-]*:|^[\\/]|\\/i.test(src) && src.length < 400;
// Adresse eines Bildes zum Anzeigen (während mdToHtml gesetzt)
let imageUrl = null;

function hasClosing(chars, from, marker) {
  for (let k = from; k + marker.length <= chars.length; k++) {
    if (chars.slice(k, k + marker.length).join('') === marker && chars[k - 1] !== '\\') return true;
  }
  return false;
}

export function inlineHtml(text) {
  const chars = [...String(text)];
  let out = '';
  const open = { b: false, i: false, s: false };
  const toggle = (k, tag) => { out += open[k] ? `</${tag}>` : `<${tag}>`; open[k] = !open[k]; };
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i];
    const n = chars[i + 1];
    if (c === '\\' && n && ESCAPABLE.includes(n)) { out += esc(n); i++; continue; }
    if (c === '\n') { out += '<br>'; continue; }
    if (c === '`') {
      const end = chars.indexOf('`', i + 1);
      if (end > i) { out += `<code>${esc(chars.slice(i + 1, end).join(''))}</code>`; i = end; continue; }
    }
    if (c === '!' && n === '[') {
      const mid = chars.indexOf(']', i + 2);
      if (mid > i && chars[mid + 1] === '(') {
        const end = chars.indexOf(')', mid + 2);
        if (end > mid) {
          const alt = chars.slice(i + 2, mid).join('');
          const src = chars.slice(mid + 2, end).join('').trim();
          if (safeImagePath(src)) {
            const url = imageUrl ? imageUrl(src) : '';
            out += `<img${url ? ` src="${esc(url)}"` : ''} alt="${esc(alt)}" data-src="${esc(src)}">`;
          } else {
            out += esc(alt);
          }
          i = end;
          continue;
        }
      }
    }
    if (c === '[') {
      const mid = chars.indexOf(']', i + 1);
      if (mid > i && chars[mid + 1] === '(') {
        const end = chars.indexOf(')', mid + 2);
        if (end > mid) {
          const label = inlineHtml(chars.slice(i + 1, mid).join(''));
          const url = chars.slice(mid + 2, end).join('').trim();
          out += SAFE_URL.test(url) ? `<a href="${esc(url)}">${label}</a>` : label;
          i = end;
          continue;
        }
      }
    }
    if ((c === '*' || c === '_') && n === c && (open.b || hasClosing(chars, i + 2, c + c))) { toggle('b', 'strong'); i++; continue; }
    if (c === '~' && n === '~' && (open.s || hasClosing(chars, i + 2, '~~'))) { toggle('s', 's'); i++; continue; }
    const inWord = i > 0 && /[\p{L}\p{N}]/u.test(chars[i - 1]) && /[\p{L}\p{N}]/u.test(n || '');
    if ((c === '*' || (c === '_' && !inWord)) && (open.i || (n && !/\s/.test(n) && hasClosing(chars, i + 1, c)))) { toggle('i', 'em'); continue; }
    out += esc(c);
  }
  if (open.s) out += '</s>';
  if (open.i) out += '</em>';
  if (open.b) out += '</strong>';
  return out;
}

// ---------- Markdown → HTML für den Editor ----------

export function mdToHtml(md, opts = {}) {
  imageUrl = typeof opts.image === 'function' ? opts.image : null;
  try { return blocksToHtml(parseBlocks(md)); } finally { imageUrl = null; }
}

function blocksToHtml(blocks) {
  let html = '';
  // Listen: Stapel offener <ul>/<ol>; ein <li> bleibt offen, bis der nächste Punkt gleicher oder kleinerer Tiefe kommt
  const stack = [];
  const closeLists = (to) => {
    while (stack.length > to) { const l = stack.pop(); html += `</li></${l}>`; }
  };
  for (const b of blocks) {
    if (b.type === 'listbreak') { closeLists(0); continue; }
    if (b.type !== 'li') closeLists(0);
    switch (b.type) {
      case 'h': html += `<h${b.level}>${inlineHtml(b.text) || '<br>'}</h${b.level}>`; break;
      case 'p': html += `<p>${inlineHtml(b.text)}</p>`; break;
      case 'quote': html += `<blockquote><p>${inlineHtml(b.text)}</p></blockquote>`; break;
      case 'code': html += `<pre><code>${esc(b.text)}</code></pre>`; break;
      case 'hr': html += '<hr>'; break;
      case 'table': {
        const [head, ...rows] = b.rows;
        const cols = Math.max(...b.rows.map((r) => r.length), 1);
        const row = (r, tag) => `<tr>${Array.from({ length: cols }, (_, k) => `<${tag}>${inlineHtml(r[k] || '') || '<br>'}</${tag}>`).join('')}</tr>`;
        html += `<table><thead>${row(head, 'th')}</thead><tbody>${rows.map((r) => row(r, 'td')).join('')}</tbody></table>`;
        break;
      }
      case 'li': {
        const tag = b.ordered ? 'ol' : 'ul';
        const depth = b.level + 1;
        if (stack.length >= depth) {
          closeLists(depth);
          if (stack[depth - 1] !== tag) { closeLists(depth - 1); html += `<${tag}>`; stack.push(tag); } else html += '</li>';
        }
        while (stack.length < depth) { html += `<${tag}>`; stack.push(tag); }
        html += `<li>${inlineHtml(b.text) || '<br>'}`;
        break;
      }
      default:
    }
  }
  closeLists(0);
  return html || '<p><br></p>';
}

// ---------- HTML (Editor, eingefügt) → Markdown ----------

const BLOCK = new Set(['P', 'DIV', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'UL', 'OL', 'LI', 'BLOCKQUOTE', 'PRE', 'TABLE', 'HR', 'SECTION', 'ARTICLE', 'HEADER', 'FOOTER', 'MAIN', 'FIGURE']);
const SKIP = new Set(['SCRIPT', 'STYLE', 'HEAD', 'TITLE', 'META', 'LINK', 'NOSCRIPT', 'TEMPLATE', 'SVG', 'BUTTON', 'IMG', 'VIDEO', 'AUDIO', 'IFRAME', 'OBJECT', 'INPUT', 'SELECT', 'TEXTAREA']);
const kids = (n) => Array.from(n.childNodes || []);
const nameOf = (n) => String(n.nodeName || '').toUpperCase();
const isBlock = (n) => n.nodeType === 1 && BLOCK.has(nameOf(n));

// Text so maskieren, dass er beim Lesen Text bleibt (keine ungewollte Auszeichnung)
function escText(t) {
  return t.replace(/[\\`*[\]]/g, '\\$&').replace(/~~/g, '\\~\\~')
    .replace(/(^|[^\p{L}\p{N}])_/gu, '$1\\_').replace(/_(?=[^\p{L}\p{N}]|$)/gu, '\\_');
}
// Am Anfang eines Absatzes und nach jedem Zeilenumbruch darin («\⏎»): nichts, was wie Überschrift, Liste,
// Zitat oder Tabelle aussieht
function escLine(t) {
  return t.replace(/^(\s*)(#{1,6} |>|[-+] |\|)/, (m, sp, x) => `${sp}\\${x}`).replace(/^(\s*\d{1,9})([.)]) /, '$1\\$2 ');
}
function escStart(t) {
  return t.split('\\\n').map(escLine).join('\\\n');
}

function wrap(inner, mark) {
  const m = /^(\s*)([\s\S]*?)(\s*)$/.exec(inner);
  if (!m[2]) return inner;
  return `${m[1]}${mark}${m[2]}${mark}${m[3]}`;
}

function style(n) {
  const s = String(n.getAttribute?.('style') || '').toLowerCase();
  return {
    bold: /font-weight\s*:\s*(bold|[6-9]00)/.test(s),
    italic: /font-style\s*:\s*italic/.test(s),
    strike: /text-decoration[^;]*line-through/.test(s),
  };
}

function inline(n, ctx = {}) {
  let out = '';
  for (const c of kids(n)) {
    if (c.nodeType === 3) {
      const t = String(c.textContent || '').replace(/\u00a0/g, ' ').replace(/[\r\n\t]+/g, ' ');
      out += ctx.code ? t : escText(t);
      continue;
    }
    if (c.nodeType !== 1) continue;
    const name = nameOf(c);
    if (name === 'IMG') {
      // Nur Bilder, die aus dem Doc kommen (data-src); eingefügte Bilder von Webseiten fallen weg
      const src = String(c.getAttribute?.('data-src') || '');
      if (safeImagePath(src)) out += `![${String(c.getAttribute?.('alt') || '').replace(/[[\]\\]/g, '')}](${src.replace(/[()\s]/g, encodeURIComponent)})`;
      continue;
    }
    if (SKIP.has(name)) continue;
    if (name === 'BR') { out += '\\\n'; continue; }
    if (name === 'CODE' || name === 'KBD' || name === 'SAMP') {
      const t = String(c.textContent || '').replace(/\n/g, ' ');
      if (t) out += t.includes('`') ? `\`\` ${t} \`\`` : `\`${t}\``;
      continue;
    }
    const inner = inline(c, ctx);
    const st = style(c);
    if (name === 'A') {
      const href = String(c.getAttribute?.('href') || '').trim();
      out += SAFE_URL.test(href) && inner.trim() ? `[${inner.trim()}](${href.replace(/[()\s]/g, encodeURIComponent)})` : inner;
    } else if (name === 'STRONG' || name === 'B' || st.bold) {
      out += wrap(inner, '**');
    } else if (name === 'EM' || name === 'I' || st.italic) {
      out += wrap(inner, '*');
    } else if (name === 'S' || name === 'DEL' || name === 'STRIKE' || st.strike) {
      out += wrap(inner, '~~');
    } else {
      out += isBlock(c) ? ` ${inner} ` : inner;
    }
  }
  return out;
}

const tidy = (s) => s.replace(/[ ]{2,}/g, ' ').replace(/ ?\\\n ?/g, '\\\n').replace(/(\\\n)+$/, '').trim();

function listMd(n, indent, out) {
  const ordered = nameOf(n) === 'OL';
  let k = Number(n.getAttribute?.('start')) || 1;
  for (const li of kids(n)) {
    if (li.nodeType !== 1) continue;
    if (nameOf(li) === 'UL' || nameOf(li) === 'OL') { listMd(li, indent + '  ', out); continue; }
    const marker = ordered ? `${k++}. ` : '- ';
    // Text des Punkts ohne darin verschachtelte Listen; Absätze darin werden zu einer Zeile
    const holder = { childNodes: kids(li).filter((c) => !(c.nodeType === 1 && (nameOf(c) === 'UL' || nameOf(c) === 'OL'))) };
    const text = tidy(inline(holder)).replace(/\\\n/g, ' ');
    out.push(`${indent}${marker}${text}`);
    for (const c of kids(li)) {
      if (c.nodeType === 1 && (nameOf(c) === 'UL' || nameOf(c) === 'OL')) listMd(c, indent + ' '.repeat(marker.length), out);
    }
  }
}

function cellText(c) {
  return tidy(inline(c)).replace(/\\\n/g, ' ').replace(/\|/g, '\\|');
}

function blocks(n, out) {
  let run = { childNodes: [] }; // Text und Inline-Elemente direkt im Block (contentEditable legt sie manchmal so ab)
  const flushRun = () => {
    const t = tidy(inline(run));
    if (t) out.push(escStart(t));
    run = { childNodes: [] };
  };
  for (const c of kids(n)) {
    if (c.nodeType === 3 || (c.nodeType === 1 && !isBlock(c) && (!SKIP.has(nameOf(c)) || nameOf(c) === 'IMG'))) {
      if (c.nodeType === 1 && nameOf(c) === 'BR' && !run.childNodes.length) continue;
      run.childNodes.push(c);
      continue;
    }
    flushRun();
    if (c.nodeType !== 1 || SKIP.has(nameOf(c))) continue;
    const name = nameOf(c);
    const h = /^H([1-6])$/.exec(name);
    if (h) {
      const t = tidy(inline(c)).replace(/\\\n/g, ' ');
      if (t) out.push(`${'#'.repeat(Number(h[1]))} ${t}`);
    } else if (name === 'UL' || name === 'OL') {
      const lines = [];
      listMd(c, '', lines);
      if (lines.length) out.push(lines.join('\n'));
    } else if (name === 'BLOCKQUOTE') {
      const inner = [];
      blocks(c, inner);
      if (inner.length) out.push(inner.join('\n\n').split('\n').map((l) => (l ? `> ${l}` : '>')).join('\n'));
    } else if (name === 'PRE') {
      const t = String(c.textContent || '').replace(/\u00a0/g, ' ').replace(/\n$/, '');
      out.push(`\`\`\`\n${t.replace(/```/g, '`\u200b``')}\n\`\`\``);
    } else if (name === 'HR') {
      out.push('---');
    } else if (name === 'TABLE') {
      const rows = [];
      const walk = (x) => { for (const r of kids(x)) { if (r.nodeType !== 1) continue; if (nameOf(r) === 'TR') rows.push(kids(r).filter((d) => d.nodeType === 1 && /^T[HD]$/.test(nameOf(d))).map(cellText)); else walk(r); } };
      walk(c);
      if (rows.length) {
        const cols = Math.max(...rows.map((r) => r.length), 1);
        const line = (r) => `| ${Array.from({ length: cols }, (_, k) => r[k] || '').join(' | ')} |`;
        out.push([line(rows[0]), `|${' --- |'.repeat(cols)}`, ...rows.slice(1).map(line)].join('\n'));
      }
    } else if (name === 'LI') {
      const lines = [];
      listMd({ nodeName: 'UL', childNodes: [c] }, '', lines);
      out.push(lines.join('\n'));
    } else if (name === 'P' || !kids(c).some(isBlock)) {
      const t = tidy(inline(c));
      if (t) out.push(escStart(t));
    } else {
      blocks(c, out); // DIV, SECTION … mit Blöcken darin
    }
  }
  flushRun();
}

export function htmlToMd(root) {
  const out = [];
  blocks(root, out);
  // Listen direkt hintereinander gleicher Art bleiben eine Liste; sonst trennt eine Leerzeile
  return out.length ? `${out.join('\n\n')}\n` : '';
}

// Titel aus dem Markdown (wie title_of in docs.rs)
export function titleOf(md, path = '') {
  for (const line of String(md || '').split('\n').slice(0, 40)) {
    const m = /^\s*# (.+)$/.exec(line);
    if (m) { const t = m[1].trim().replace(/^[*_`]+|[*_`]+$/g, '').trim(); if (t) return t; }
  }
  return String(path).split('/').pop().replace(/\.md$/i, '');
}

// Text für eine leere Vorlage des Editors
export const EMPTY_DOC = (title) => `# ${title}\n\n`;
