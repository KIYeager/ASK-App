// Nutzungsübersicht (ENTSCHEIDE E49): zählt aus den Chats in den Projektordnern (.ask/chats/*.json),
// was die Person mit ASK gemacht hat. Alles lokal; aggregate() schickt nichts irgendwohin.
// Nur askUsage() fragt ASK, und nur nach den eigenen Daten des angemeldeten Kontos.
const fs = require('node:fs');
const path = require('node:path');

const CHAT_DIR = path.join('.ask', 'chats');
const DAYS = 30;
const DAY_MS = 86400000;

// Tag als «JJJJ-MM-TT» in der Ortszeit des Laptops
function dayKey(ms) {
  const d = new Date(ms);
  const p = (n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

const empty = () => ({ chats: 0, tasks: 0, steps: 0, tokens: 0, estimatedTokens: 0, modelMs: 0, files: { added: 0, changed: 0, removed: 0 }, last: 0 });

function add(into, s) {
  into.chats += s.chats;
  into.tasks += s.tasks;
  into.steps += s.steps;
  into.tokens += s.tokens;
  into.estimatedTokens += s.estimatedTokens;
  into.modelMs += s.modelMs;
  into.files.added += s.files.added;
  into.files.changed += s.files.changed;
  into.files.removed += s.files.removed;
  into.last = Math.max(into.last, s.last);
  return into;
}

const n = (v) => (Number.isFinite(Number(v)) && Number(v) > 0 ? Number(v) : 0);

// Ein gespeicherter Chat → Zahlen und Tageswerte { 'JJJJ-MM-TT': { tasks, tokens } }.
// Aufträge sind Eingaben der Person (ohne automatische Nachrichten wie «Weiter am Ziel» oder Ansichten),
// Token die erzeugten Token (stats.tokens; geschätzte zählen mit und werden ausgewiesen),
// Dateien aus «changes» der Eingabe; ein rückgängig gemachter Auftrag zählt dort nicht mehr.
function summarizeChat(chat) {
  const s = { ...empty(), chats: 1, days: {} };
  const fallback = n(chat.updated) || n(chat.created);
  const day = (ms) => {
    const key = dayKey(n(ms) || fallback);
    return (s.days[key] ||= { tasks: 0, tokens: 0 });
  };
  for (const m of Array.isArray(chat.messages) ? chat.messages : []) {
    if (!m || typeof m !== 'object') continue;
    s.last = Math.max(s.last, n(m.time));
    if (m.role === 'user' && !m.auto) {
      s.tasks += 1;
      day(m.time).tasks += 1;
      const c = m.changes;
      if (c && !m.undone) {
        const counts = Array.isArray(c.counts) ? c.counts : [];
        s.files.changed += n(counts[0] ?? c.changed?.length);
        s.files.added += n(counts[1] ?? c.added?.length);
        s.files.removed += n(counts[2] ?? c.removed?.length);
      }
    } else if (m.role === 'assistant' && m.stats) {
      const t = n(m.stats.tokens);
      s.tokens += t;
      if (m.stats.estimated) s.estimatedTokens += t;
      s.modelMs += n(m.stats.totalMs);
      day(m.time).tokens += t;
    } else if (m.role === 'tool') {
      s.steps += 1;
    }
  }
  s.last = s.last || fallback;
  return s;
}

// Zwischenspeicher pro Datei: unveränderte Chats (gleiche Änderungszeit und Grösse) nicht neu lesen
const cache = new Map(); // Datei → { mtime, size, summary }

function folderUsage(folder) {
  const total = { ...empty(), days: {} };
  const dir = path.join(folder, CHAT_DIR);
  let names = [];
  try { names = fs.readdirSync(dir).filter((f) => f.endsWith('.json')); } catch { return total; }
  for (const name of names) {
    const file = path.join(dir, name);
    let summary = null;
    try {
      const st = fs.statSync(file);
      const hit = cache.get(file);
      if (hit && hit.mtime === st.mtimeMs && hit.size === st.size) summary = hit.summary;
      else {
        const chat = JSON.parse(fs.readFileSync(file, 'utf8'));
        if (!chat || !Array.isArray(chat.messages)) continue;
        summary = summarizeChat(chat);
        if (cache.size > 5000) cache.clear();
        cache.set(file, { mtime: st.mtimeMs, size: st.size, summary });
      }
    } catch {
      continue; // halb geschrieben oder nicht lesbar
    }
    add(total, summary);
    for (const [k, v] of Object.entries(summary.days)) {
      const d = (total.days[k] ||= { tasks: 0, tokens: 0 });
      d.tasks += v.tasks;
      d.tokens += v.tokens;
    }
  }
  return total;
}

// Alle Projekte: { projects: [...], total, days: [{ date, tasks, tokens }] (letzte 30 Tage, ältester zuerst) }
function aggregate(projects, { now = Date.now(), days = DAYS } = {}) {
  const total = empty();
  const byDay = {};
  const list = [];
  for (const p of Array.isArray(projects) ? projects : []) {
    if (!p || !p.folder) continue;
    const missing = !fs.existsSync(p.folder);
    const u = missing ? { ...empty(), days: {} } : folderUsage(p.folder);
    const { days: perDay, ...numbers } = u;
    list.push({ folder: p.folder, name: p.name || path.win32.basename(p.folder), color: p.color || '', missing, ...numbers });
    add(total, numbers);
    for (const [k, v] of Object.entries(perDay)) {
      const d = (byDay[k] ||= { tasks: 0, tokens: 0 });
      d.tasks += v.tasks;
      d.tokens += v.tokens;
    }
  }
  const series = [];
  const today = new Date(now);
  today.setHours(12, 0, 0, 0); // Mittag: Sommerzeit verschiebt keinen Tag
  for (let i = days - 1; i >= 0; i--) {
    const date = dayKey(today.getTime() - i * DAY_MS);
    series.push({ date, tasks: byDay[date]?.tasks || 0, tokens: byDay[date]?.tokens || 0 });
  }
  list.sort((a, b) => b.tasks - a.tasks || b.last - a.last);
  return { projects: list, total, days: series };
}

// ---------- Angaben von ASK (optional) ----------
// NICHT mit dem echten ASK geprüft. Ob ein normales Konto dort Nutzungszahlen abfragen darf, ist offen.
// Versucht werden bekannte Schnittstellen von Open WebUI; was nicht antwortet oder anders aussieht,
// wird still übergangen. Ohne Ergebnis: null, die Oberfläche zeigt den Abschnitt dann nicht.

const PAGES_MAX = 20;
// Kandidaten für Nutzungszahlen (Open WebUI je nach Version und Rolle); nur Zahlen mit bekannten Namen zählen
const USAGE_PATHS = ['/api/v1/chats/stats/usage', '/api/v1/users/user/usage', '/api/v1/analytics/summary'];
const USAGE_FIELDS = {
  messages: ['total_messages', 'message_count', 'messages'],
  tokens: ['total_tokens', 'tokens'],
  inputTokens: ['prompt_tokens', 'input_tokens'],
  outputTokens: ['completion_tokens', 'output_tokens'],
};

const seconds = (v) => (n(v) > 1e12 ? n(v) : n(v) * 1000); // Open WebUI zählt in Sekunden

async function getJson(session, apiPath, signal) {
  const res = await session.raw(apiPath, { signal });
  if (!res.ok) return null;
  return res.json().catch(() => null);
}

// Bekannte Zahlen aus einer Antwort lesen: oben, unter «data» oder als Summe über «items»
function usageNumbers(data) {
  if (!data || typeof data !== 'object') return {};
  const out = {};
  const sources = [data, data.data].filter((x) => x && typeof x === 'object' && !Array.isArray(x));
  const items = [data.items, data.data, Array.isArray(data) ? data : null].find(Array.isArray) || [];
  for (const [key, names] of Object.entries(USAGE_FIELDS)) {
    for (const src of sources) {
      const name = names.find((f) => typeof src[f] === 'number');
      if (name) { out[key] = src[name]; break; }
    }
    if (out[key] === undefined && items.length) {
      const name = names.find((f) => items.some((i) => i && typeof i[f] === 'number'));
      if (name) out[key] = items.reduce((sum, i) => sum + n(i?.[name]), 0);
    }
  }
  return out;
}

// session: AskSession (askapi.js). Ergebnis { webChats, webChats30, more, ...Zahlen } oder null.
async function askUsage(session, { now = Date.now(), timeoutMs = 5000 } = {}) {
  if (!session || !session.token) return null;
  const signal = AbortSignal.timeout(timeoutMs);
  const result = {};
  // Chats auf der ASK-Webseite: eigene Chatliste, seitenweise (ältere Versionen liefern alles auf einmal)
  try {
    const seen = new Set();
    let recent = 0;
    for (let page = 1; page <= PAGES_MAX; page++) {
      const list = await getJson(session, `/api/v1/chats/?page=${page}`, signal);
      if (!Array.isArray(list) || !list.length) break;
      const fresh = list.filter((c) => c && c.id && !seen.has(c.id));
      if (!fresh.length) break; // Server kennt «page» nicht und liefert dasselbe
      for (const c of fresh) {
        seen.add(c.id);
        if (now - seconds(c.updated_at || c.created_at) < DAYS * DAY_MS) recent += 1;
      }
      if (page === PAGES_MAX) result.more = true;
    }
    if (seen.size) Object.assign(result, { webChats: seen.size, webChats30: recent });
  } catch {
    // nicht erreichbar oder nicht erlaubt
  }
  for (const p of USAGE_PATHS) {
    if (signal.aborted) break;
    try {
      const nums = usageNumbers(await getJson(session, p, signal));
      if (Object.keys(nums).length) { Object.assign(result, nums); break; }
    } catch {
      // nächste Schnittstelle
    }
  }
  return Object.keys(result).length ? result : null;
}

module.exports = { summarizeChat, folderUsage, aggregate, askUsage, usageNumbers, dayKey, DAYS };
