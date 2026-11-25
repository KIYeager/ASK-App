// Netz des Gedächtnisses (ENTSCHEIDE E50): Knoten «Du», Projekte, Schlagworte und Einträge, verbunden wie ein
// Mind-Web. Eigene kleine kräftebasierte Anordnung ohne Bibliothek: gleiche Daten ergeben immer dasselbe Bild
// (Zufall mit festem Startwert), sie rechnet eine feste Zahl Schritte und hält dann an.

const DU = 'du';
const sameFolder = (a, b) => String(a || '').toLowerCase() === String(b || '').toLowerCase();
const baseName = (p) => String(p || '').split(/[\\/]/).filter(Boolean).pop() || String(p || '');

// Knoten und Kanten aus Einträgen und Projekten. Projekte ohne Einträge erscheinen nicht.
export function buildGraph(items = [], projects = []) {
  const nodes = [{ id: DU, kind: 'du', label: 'Du' }];
  const links = [];
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const node = (n) => { if (!byId.has(n.id)) { byId.set(n.id, n); nodes.push(n); } return byId.get(n.id); };
  for (const item of items) {
    let anchor = DU;
    if (item.scope === 'projekt' && item.project) {
      const p = projects.find((x) => sameFolder(x.folder, item.project));
      const id = `p:${String(item.project).toLowerCase()}`;
      if (!byId.has(id)) {
        node({ id, kind: 'projekt', label: p?.name || baseName(item.project), color: p?.color || '', folder: item.project });
        links.push({ source: id, target: DU, kind: 'projekt' });
      }
      anchor = id;
    }
    const id = `i:${item.id}`;
    node({ id, kind: 'item', label: item.text, item, color: anchor === DU ? '' : byId.get(anchor).color });
    links.push({ source: id, target: anchor, kind: 'anker' });
    for (const tag of item.tags || []) {
      const tid = `t:${tag}`;
      node({ id: tid, kind: 'tag', label: tag });
      links.push({ source: id, target: tid, kind: 'tag' });
    }
  }
  return { nodes, links };
}

// Nachbarn je Knoten (für das Hervorheben beim Darüberfahren)
export function neighbours(graph) {
  const map = new Map(graph.nodes.map((n) => [n.id, new Set([n.id])]));
  for (const l of graph.links) {
    map.get(l.source)?.add(l.target);
    map.get(l.target)?.add(l.source);
  }
  return map;
}

function hash(text) {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) { h ^= text.charCodeAt(i); h = Math.imul(h, 16777619); }
  return h >>> 0;
}

// Zufallszahlen mit Startwert (mulberry32)
export function seeded(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const CHARGE = { du: 3200, projekt: 2000, tag: 1300, item: 520 };
const LENGTH = { projekt: 240, anker: 95, tag: 80 };
const STRENGTH = { projekt: 0.08, anker: 0.06, tag: 0.035 };

// Positionen { id: { x, y } } um «Du» bei (0, 0). Feste Zahl Schritte, danach steht das Bild.
export function layout(graph, { iterations } = {}) {
  const { nodes, links } = graph;
  const n = nodes.length;
  const rand = seeded(hash(nodes.map((x) => x.id).sort().join('|')));
  const steps = iterations ?? (n > 300 ? 180 : n > 120 ? 260 : 320);
  const index = new Map(nodes.map((x, i) => [x.id, i]));
  const x = new Float64Array(n);
  const y = new Float64Array(n);
  const vx = new Float64Array(n);
  const vy = new Float64Array(n);
  const q = nodes.map((nd) => CHARGE[nd.kind] || 400);
  // Beschriftete Knoten brauchen seitlich mehr Platz: Abstand in x zählt dort weniger (Ellipse statt Kreis)
  const wide = nodes.map((nd) => nd.kind === 'tag' || nd.kind === 'projekt' || nd.kind === 'du');
  // Start: Projekte auf einem Kreis, alles andere zufällig in der Nähe seines Ankers
  const projects = nodes.filter((nd) => nd.kind === 'projekt');
  const anchorOf = new Map(links.filter((l) => l.kind === 'anker').map((l) => [l.source, l.target]));
  nodes.forEach((nd, i) => {
    if (nd.kind === 'du') return;
    if (nd.kind === 'projekt') {
      const k = projects.indexOf(nd);
      const angle = (k / Math.max(projects.length, 1)) * Math.PI * 2 - Math.PI / 2 + 0.4;
      x[i] = Math.cos(angle) * 220; y[i] = Math.sin(angle) * 220;
      return;
    }
    const a = index.get(anchorOf.get(nd.id));
    const cx = a === undefined ? 0 : x[a];
    const cy = a === undefined ? 0 : y[a];
    const angle = rand() * Math.PI * 2;
    const r = 60 + rand() * 120;
    x[i] = cx + Math.cos(angle) * r; y[i] = cy + Math.sin(angle) * r;
  });
  const edges = links.map((l) => [index.get(l.source), index.get(l.target), LENGTH[l.kind] || 80, STRENGTH[l.kind] || 0.05]).filter(([a, b]) => a !== undefined && b !== undefined);
  for (let step = 0; step < steps; step++) {
    const alpha = 1 - step / steps;
    // Abstossung zwischen allen Knoten
    for (let i = 0; i < n; i++) {
      for (let j = i + 1; j < n; j++) {
        let dx = x[j] - x[i];
        let dy = y[j] - y[i];
        if (wide[i] || wide[j]) dx *= 0.6;
        let d2 = dx * dx + dy * dy;
        if (d2 < 1) { dx = (rand() - 0.5) || 0.1; dy = (rand() - 0.5) || 0.1; d2 = dx * dx + dy * dy; }
        if (d2 > 250000) continue; // weiter als 500 px: vernachlässigbar
        const f = (Math.sqrt(q[i] * q[j]) / d2) * alpha;
        const d = Math.sqrt(d2);
        const fx = (dx / d) * f;
        const fy = (dy / d) * f;
        vx[i] -= fx; vy[i] -= fy; vx[j] += fx; vy[j] += fy;
      }
    }
    // Federn entlang der Kanten
    for (const [a, b, len, k] of edges) {
      const dx = x[b] - x[a];
      const dy = y[b] - y[a];
      const d = Math.sqrt(dx * dx + dy * dy) || 0.01;
      const f = (d - len) * k * alpha;
      const fx = (dx / d) * f;
      const fy = (dy / d) * f;
      vx[a] += fx; vy[a] += fy; vx[b] -= fx; vy[b] -= fy;
    }
    for (let i = 0; i < n; i++) {
      if (nodes[i].kind === 'du') { vx[i] = 0; vy[i] = 0; continue; } // «Du» bleibt in der Mitte
      vx[i] -= x[i] * 0.006 * alpha; // leichter Zug zur Mitte, damit nichts davonfliegt
      vy[i] -= y[i] * 0.006 * alpha;
      vx[i] *= 0.55; vy[i] *= 0.55;
      const sp = Math.hypot(vx[i], vy[i]);
      if (sp > 40) { vx[i] *= 40 / sp; vy[i] *= 40 / sp; }
      x[i] += vx[i]; y[i] += vy[i];
    }
  }
  const out = {};
  nodes.forEach((nd, i) => { out[nd.id] = { x: Math.round(x[i] * 10) / 10, y: Math.round(y[i] * 10) / 10 }; });
  return out;
}

// Ausschnitt, der alle Knoten zeigt: Verschiebung und Zoom für eine Fläche width × height
export function fit(positions, width, height, pad = 60) {
  const pts = Object.values(positions);
  if (!pts.length || !width || !height) return { x: width / 2 || 0, y: height / 2 || 0, k: 1 };
  const minX = Math.min(...pts.map((p) => p.x)) - pad;
  const maxX = Math.max(...pts.map((p) => p.x)) + pad;
  const minY = Math.min(...pts.map((p) => p.y)) - pad;
  const maxY = Math.max(...pts.map((p) => p.y)) + pad;
  const k = Math.max(0.35, Math.min(1.15, Math.min(width / (maxX - minX), height / (maxY - minY))));
  return { x: width / 2 - ((minX + maxX) / 2) * k, y: height / 2 - ((minY + maxY) / 2) * k, k };
}
