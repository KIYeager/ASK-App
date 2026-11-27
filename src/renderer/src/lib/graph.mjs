// Karte des Gedächtnisses (ENTSCHEIDE E50, neu gezeichnet in E78): «Du» in der Mitte, ein Ast für «Über dich» und
// einer pro Projekt, daran jeder Eintrag als lesbare Zeile. Die frühere Anordnung mit Kräften zeigte die Einträge
// nur als Punkte; lesen konnte man erst beim Darüberfahren. Reine Funktionen ohne DOM (node --test).

const sameFolder = (a, b) => String(a || '').toLowerCase() === String(b || '').toLowerCase();
const baseName = (p) => String(p || '').split(/[\\/]/).filter(Boolean).pop() || String(p || '');

export const MAP = {
  row: 30, // Höhe einer Zeile mit einem Eintrag
  gap: 30, // Abstand zwischen zwei Ästen derselben Seite
  hubX: 150, // Abstand der Äste von der Mitte
  itemX: 265, // Abstand der Einträge von der Mitte
  hubChars: 18, // so viele Zeichen des Namens stehen im Ast
  split: 9, // hat «Über dich» mehr Einträge, wird es nach Themen (häufigste Schlagworte) in mehrere Äste geteilt
  // So viele Zeichen eines Eintrags stehen in der Karte (der ganze Text beim Darüberfahren und im Seitenfeld). Mit
  // den Abständen oben passen beide Seiten bei üblicher Fenstergrösse in lesbarer Schrift nebeneinander.
  chars: 40,
  charW: 6.5, // geschätzte Breite eines Zeichens der Beschriftung in px
};

export const shorten = (text, max = MAP.chars) => {
  const t = String(text || '').replace(/\s+/g, ' ').trim();
  return t.length > max ? `${t.slice(0, max - 1).trimEnd()}…` : t;
};

// Einträge nach Themen: immer das Schlagwort, das unter den übrigen am häufigsten ist (mindestens zweimal); was
// kein gemeinsames Schlagwort hat, kommt zu «Weiteres». So wird aus einer langen Spalte ein Bild mit Ästen.
function byTopic(list) {
  let rest = [...list];
  const out = [];
  while (rest.length) {
    const count = new Map();
    for (const i of rest) for (const t of i.tags || []) count.set(t, (count.get(t) || 0) + 1);
    const best = [...count.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], 'de'))[0];
    if (!best || best[1] < 2) break;
    out.push({ tag: best[0], items: rest.filter((i) => (i.tags || []).includes(best[0])) });
    rest = rest.filter((i) => !(i.tags || []).includes(best[0]));
  }
  if (rest.length) out.push({ tag: '', items: rest });
  return out;
}

// Gruppen: zuerst «Über dich», dann die Projekte in der Reihenfolge der Projektliste; ohne Einträge keine Gruppe
function groupsOf(items, projects) {
  const groups = [];
  const about = items.filter((i) => i.scope !== 'projekt' || !i.project);
  if (about.length > MAP.split) {
    const topics = byTopic(about);
    // Nur aufteilen, wenn es wirklich Themen gibt (sonst bliebe ein einziger Ast «Weiteres»)
    if (topics.length > 1) for (const t of topics) groups.push({ id: `g:du:${t.tag}`, label: t.tag || 'Weiteres', color: '', about: true, topic: true, items: t.items });
    else groups.push({ id: 'g:du', label: 'Über dich', color: '', about: true, items: about });
  } else if (about.length) groups.push({ id: 'g:du', label: 'Über dich', color: '', about: true, items: about });
  const rest = items.filter((i) => i.scope === 'projekt' && i.project);
  const order = [...projects.map((p) => p.folder), ...rest.map((i) => i.project)];
  const seen = new Set();
  for (const folder of order) {
    const key = String(folder || '').toLowerCase();
    if (!key || seen.has(key)) continue;
    seen.add(key);
    const list = rest.filter((i) => sameFolder(i.project, folder));
    if (!list.length) continue;
    const p = projects.find((x) => sameFolder(x.folder, folder));
    groups.push({ id: `g:${key}`, label: p?.name || baseName(folder), color: p?.color || '', about: false, folder, items: list });
  }
  // Neueste Einträge zuoberst
  for (const g of groups) g.items = [...g.items].sort((a, b) => (b.updated || 0) - (a.updated || 0));
  return groups;
}

// Karte: { nodes, links, tags, extent }.
// nodes: { id, kind: 'du' | 'gruppe' | 'item', x, y, side (1 rechts, -1 links), label, color, item?, group?, count? }
// links: { from, to, kind: 'ast' | 'zweig', color }; tags: [[Schlagwort, Anzahl]], häufigste zuerst
// extent: Punkte, die der Ausschnitt zeigen muss (mit dem Platz für die Beschriftungen)
export function mindmap(items = [], projects = []) {
  const groups = groupsOf(items, projects);
  const height = (g) => g.items.length * MAP.row;
  // Auf beide Seiten verteilen, sodass sie ähnlich hoch werden: «Über dich» rechts, dann die grössten zuerst
  const sides = { 1: [], '-1': [] };
  const total = { 1: 0, '-1': 0 };
  const sorted = [...groups].sort((a, b) => (b.about - a.about) || (height(b) - height(a)));
  for (const g of sorted) {
    const side = groups.length > 1 && total[-1] < total[1] ? -1 : 1;
    sides[side].push(g);
    total[side] += height(g) + MAP.gap;
  }
  const nodes = [{ id: 'du', kind: 'du', x: 0, y: 0, side: 0, label: 'Du' }];
  const links = [];
  const extent = [{ x: -40, y: -40 }, { x: 40, y: 40 }];
  for (const side of [1, -1]) {
    // In der Reihenfolge der Gruppen (nicht nach Grösse), damit ein Projekt seinen Platz behält, wenn Einträge dazukommen
    const list = groups.filter((g) => sides[side].includes(g));
    let y = -(list.reduce((n, g) => n + height(g), 0) + MAP.gap * Math.max(0, list.length - 1)) / 2;
    for (const g of list) {
      const h = height(g);
      nodes.push({ id: g.id, kind: 'gruppe', x: side * MAP.hubX, y: y + h / 2, side, label: g.label, color: g.color, about: g.about, topic: !!g.topic, count: g.items.length });
      links.push({ from: 'du', to: g.id, kind: 'ast', color: g.color });
      g.items.forEach((item, i) => {
        const id = `i:${item.id}`;
        const iy = y + MAP.row / 2 + i * MAP.row;
        const label = shorten(item.text);
        const width = 16 + label.length * MAP.charW; // Punkt und Text
        nodes.push({ id, kind: 'item', x: side * MAP.itemX, y: iy, side, label, width, color: g.color, item, group: g.id });
        links.push({ from: g.id, to: id, kind: 'zweig', color: g.color });
        extent.push({ x: side * (MAP.itemX + width), y: iy });
      });
      extent.push({ x: side * MAP.hubX, y: y - 6 }, { x: side * MAP.hubX, y: y + h + 6 });
      y += h + MAP.gap;
    }
  }
  const count = new Map();
  for (const item of items) for (const t of item.tags || []) count.set(t, (count.get(t) || 0) + 1);
  const tags = [...count.entries()].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0], 'de'));
  return { nodes, links, tags, extent };
}

// Einträge, die mit diesem verwandt sind: gleiches Schlagwort (für das Hervorheben beim Darüberfahren)
export function related(items, id) {
  const me = items.find((i) => i.id === id);
  const tags = new Set(me?.tags || []);
  return new Set(items.filter((i) => i.id === id || (i.tags || []).some((t) => tags.has(t))).map((i) => i.id));
}

// Ausschnitt, der alle Knoten zeigt: Verschiebung und Zoom für eine Fläche width × height
// minK: kleinster Zoom (die Karte bleibt lesbar; was nicht hineinpasst, erreicht man durch Ziehen)
export function fit(positions, width, height, pad = 60, minK = 0.35) {
  const pts = Object.values(positions);
  if (!pts.length || !width || !height) return { x: width / 2 || 0, y: height / 2 || 0, k: 1 };
  const minX = Math.min(...pts.map((p) => p.x)) - pad;
  const maxX = Math.max(...pts.map((p) => p.x)) + pad;
  const minY = Math.min(...pts.map((p) => p.y)) - pad;
  const maxY = Math.max(...pts.map((p) => p.y)) + pad;
  const k = Math.max(minK, Math.min(1.15, Math.min(width / (maxX - minX), height / (maxY - minY))));
  return { x: width / 2 - ((minX + maxX) / 2) * k, y: height / 2 - ((minY + maxY) / 2) * k, k };
}
