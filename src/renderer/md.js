// Kleiner Markdown-Renderer für Antworten des Modells: Überschriften, Absätze, Listen, Code,
// Tabellen, fett, kursiv, Links. Alles wird zuerst maskiert; es entsteht nie fremdes HTML.
(function () {
  const esc = (s) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

  function inline(text) {
    const codes = [];
    let s = text.replace(/`([^`\n]+)`/g, (_m, c) => {
      codes.push(c);
      return `\u0000${codes.length - 1}\u0000`;
    });
    s = esc(s)
      .replace(/\*\*([^*\n]+)\*\*/g, '<strong>$1</strong>')
      .replace(/(^|[^*\w])\*([^*\n]+)\*(?!\w)/g, '$1<em>$2</em>')
      .replace(/(^|[^_\w])_([^_\n]+)_(?!\w)/g, '$1<em>$2</em>')
      .replace(/\[([^\]\n]+)\]\((https?:[^)\s]+)\)/g, '<a href="$2" title="$2">$1</a>');
    return s.replace(/\u0000(\d+)\u0000/g, (_m, i) => `<code>${esc(codes[Number(i)])}</code>`);
  }

  function table(rows) {
    const cells = (line) => line.replace(/^\s*\|/, '').replace(/\|\s*$/, '').split('|').map((c) => c.trim());
    const head = cells(rows[0]);
    const body = rows.slice(2).map(cells);
    return `<table><thead><tr>${head.map((c) => `<th>${inline(c)}</th>`).join('')}</tr></thead><tbody>${
      body.map((r) => `<tr>${r.map((c) => `<td>${inline(c)}</td>`).join('')}</tr>`).join('')}</tbody></table>`;
  }

  function render(markdown) {
    const lines = String(markdown || '').replace(/\r\n/g, '\n').split('\n');
    const out = [];
    let para = [];
    let list = null; // { type: 'ul'|'ol', items: [] }
    const flushPara = () => {
      if (para.length) out.push(`<p>${para.map(inline).join('<br>')}</p>`);
      para = [];
    };
    const flushList = () => {
      if (list) out.push(`<${list.type}>${list.items.map((i) => `<li>${inline(i)}</li>`).join('')}</${list.type}>`);
      list = null;
    };
    for (let i = 0; i < lines.length; i++) {
      const line = lines[i];
      const fence = /^\s*```(\S*)\s*$/.exec(line);
      if (fence) {
        flushPara(); flushList();
        const code = [];
        for (i++; i < lines.length && !/^\s*```\s*$/.test(lines[i]); i++) code.push(lines[i]);
        out.push(`<div class="codeblock"><div class="codehead"><span>${esc(fence[1] || 'Code')}</span>`
          + `<button class="copy" type="button">Kopieren</button></div><pre class="code"><code>${esc(code.join('\n'))}</code></pre></div>`);
        continue;
      }
      if (/^\s*\|.*\|\s*$/.test(line) && /^\s*\|?\s*:?-{2,}/.test(lines[i + 1] || '')) {
        flushPara(); flushList();
        const rows = [];
        for (; i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i]); i++) rows.push(lines[i]);
        i--;
        out.push(table(rows));
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
        if (!list || list.type !== type) { flushList(); list = { type, items: [] }; }
        list.items.push(item[3]);
        continue;
      }
      if (!line.trim()) { flushPara(); flushList(); continue; }
      if (list && /^\s{2,}\S/.test(line)) { list.items[list.items.length - 1] += ` ${line.trim()}`; continue; }
      flushList();
      para.push(line);
    }
    flushPara(); flushList();
    return out.join('');
  }

  window.renderMarkdown = render;
})();
