// Interaktive Antworten (ENTSCHEIDE E80): Blöcke ```ask-ui (JSON, feste Bausteine) und ```ask-html (eigenes HTML,
// abgeschottet) in einer Antwort finden und prüfen. Das Format beschreibt der Skill interaktive-antworten.

export const UI_TYPES = ['hinweis', 'schritte', 'vergleich', 'diagramm', 'quiz', 'karteikarten', 'rechner', 'auswahl', 'formular'];

const LIMIT = { list: 60, text: 3000, html: 200 * 1024 };

// Antwort in Stücke teilen: Markdown und Bausteine. Ein noch offener Block am Ende ist `done: false`.
// Zäune wie in md.mjs: Zeile mit ``` und Sprache, Ende mit einer Zeile nur ```.
export function splitUi(text) {
  const lines = String(text || '').replace(/\r\n/g, '\n').split('\n');
  const parts = [];
  let md = [];
  const flush = () => { if (md.length) parts.push({ k: 'md', text: md.join('\n') }); md = []; };
  for (let i = 0; i < lines.length; i++) {
    const fence = /^\s*```\s*(ask-ui|ask-html)\s*$/i.exec(lines[i]);
    if (!fence) {
      // Gewöhnliche Codeblöcke unverändert übernehmen, damit ein ```ask-ui darin nicht zählt
      if (/^\s*```/.test(lines[i])) {
        md.push(lines[i]);
        for (i++; i < lines.length; i++) { md.push(lines[i]); if (/^\s*```\s*$/.test(lines[i])) break; }
        continue;
      }
      md.push(lines[i]);
      continue;
    }
    flush();
    const body = [];
    let done = false;
    for (i++; i < lines.length; i++) {
      if (/^\s*```\s*$/.test(lines[i])) { done = true; break; }
      body.push(lines[i]);
    }
    parts.push({ k: fence[1].toLowerCase() === 'ask-ui' ? 'ui' : 'html', src: body.join('\n'), done });
  }
  flush();
  return parts;
}

// Gleiche Stelle wieder als Markdown-Codeblock (wenn der Baustein nicht gezeigt werden kann oder ausgeschaltet ist)
export const asCode = (part) => `\`\`\`${part.k === 'ui' ? 'ask-ui' : 'ask-html'}\n${part.src}\n\`\`\``;

const str = (v, max = LIMIT.text) => (v === undefined || v === null ? '' : String(v).slice(0, max));
const arr = (v) => (Array.isArray(v) ? v.slice(0, LIMIT.list) : []);
const num = (v, d = 0) => (typeof v === 'number' && Number.isFinite(v) ? v : typeof v === 'string' && v.trim() !== '' && Number.isFinite(Number(v)) ? Number(v) : d);

class UiError extends Error {}
const need = (ok, msg) => { if (!ok) throw new UiError(msg); };

// Ein Baustein mit allen Feldern in der erwarteten Form; Fehler mit verständlicher Meldung
function normalize(b) {
  need(b && typeof b === 'object' && !Array.isArray(b), 'Jeder Baustein ist ein Objekt mit «typ».');
  const typ = str(b.typ || b.type).toLowerCase();
  need(UI_TYPES.includes(typ), `Unbekannter Typ «${typ || '(leer)'}». Möglich: ${UI_TYPES.join(', ')}.`);
  const titel = str(b.titel, 200);
  switch (typ) {
    case 'hinweis': {
      const art = ['info', 'tipp', 'warnung', 'erfolg'].includes(b.art) ? b.art : 'info';
      need(str(b.text), 'Hinweis braucht «text».');
      return { typ, titel, art, text: str(b.text) };
    }
    case 'schritte': {
      const schritte = arr(b.schritte).map((s) => (typeof s === 'string' ? { titel: str(s, 300), text: '' } : { titel: str(s?.titel, 300), text: str(s?.text) }))
        .filter((s) => s.titel || s.text);
      need(schritte.length, 'Schritte braucht «schritte».');
      return { typ, titel, schritte };
    }
    case 'vergleich': {
      const spalten = arr(b.spalten).slice(0, 4).map((c) => ({ titel: str(c?.titel, 120), untertitel: str(c?.untertitel, 200),
        punkte: arr(c?.punkte).map((p) => str(p, 400)), hervorheben: !!c?.hervorheben }));
      need(spalten.length >= 2, 'Vergleich braucht mindestens zwei «spalten».');
      return { typ, titel, spalten };
    }
    case 'diagramm': {
      const art = ['balken', 'linie', 'kreis'].includes(b.art) ? b.art : 'balken';
      const beschriftungen = arr(b.beschriftungen).map((l) => str(l, 60));
      const reihen = arr(b.reihen).slice(0, 6).map((r) => ({ name: str(r?.name, 60), werte: arr(r?.werte).map((v) => num(v)) }));
      need(beschriftungen.length && reihen.length, 'Diagramm braucht «beschriftungen» und «reihen».');
      need(reihen.every((r) => r.werte.length === beschriftungen.length), 'Jede Reihe braucht so viele «werte» wie «beschriftungen».');
      if (art === 'kreis') need(reihen[0].werte.every((v) => v >= 0), 'Ein Kreisdiagramm braucht Werte ab 0.');
      return { typ, titel, art, beschriftungen, reihen: art === 'kreis' ? reihen.slice(0, 1) : reihen, einheit: str(b.einheit, 20) };
    }
    case 'quiz': {
      const fragen = arr(b.fragen).map((f) => ({ frage: str(f?.frage, 600), antworten: arr(f?.antworten).slice(0, 8).map((a) => str(a, 300)),
        richtig: Math.trunc(num(f?.richtig, -1)), erklaerung: str(f?.erklaerung, 1200) }));
      need(fragen.length, 'Quiz braucht «fragen».');
      need(fragen.every((f) => f.frage && f.antworten.length >= 2 && f.richtig >= 0 && f.richtig < f.antworten.length),
        'Jede Quizfrage braucht «frage», mindestens zwei «antworten» und «richtig» (Nummer ab 0).');
      return { typ, titel, fragen };
    }
    case 'karteikarten': {
      const karten = arr(b.karten).map((k) => ({ vorne: str(k?.vorne, 600), hinten: str(k?.hinten, 1200) })).filter((k) => k.vorne && k.hinten);
      need(karten.length, 'Karteikarten brauchen «karten» mit «vorne» und «hinten».');
      return { typ, titel, karten };
    }
    case 'rechner': {
      const eingaben = arr(b.eingaben).slice(0, 12).map((e) => {
        const min = num(e?.min, 0);
        const max = num(e?.max, Math.max(100, min + 1));
        return { name: str(e?.name, 40), label: str(e?.label, 120), min, max: Math.max(max, min), schritt: num(e?.schritt, 1) || 1,
          wert: Math.min(Math.max(num(e?.wert, min), min), Math.max(max, min)), einheit: str(e?.einheit, 20) };
      });
      const ergebnisse = arr(b.ergebnisse).slice(0, 8).map((r) => ({ label: str(r?.label, 120), formel: str(r?.formel, 400),
        einheit: str(r?.einheit, 20), stellen: Math.min(Math.max(Math.trunc(num(r?.stellen, 2)), 0), 6) }));
      need(eingaben.length && ergebnisse.length, 'Rechner braucht «eingaben» und «ergebnisse».');
      need(eingaben.every((e) => /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(e.name)), 'Jede Eingabe braucht «name» aus Buchstaben, Ziffern oder _.');
      const vars = Object.fromEntries(eingaben.map((e) => [e.name, e.wert]));
      for (const r of ergebnisse) {
        try { evalFormula(r.formel, vars); } catch (err) { throw new UiError(`Formel «${r.formel}»: ${err.message}`); }
      }
      return { typ, titel, eingaben, ergebnisse };
    }
    case 'auswahl': {
      const optionen = arr(b.optionen).slice(0, 12).map((o) => str(o, 200)).filter(Boolean);
      need(optionen.length >= 2, 'Auswahl braucht mindestens zwei «optionen».');
      return { typ, titel, frage: str(b.frage, 400), optionen, mehrfach: !!b.mehrfach };
    }
    case 'formular': {
      const felder = arr(b.felder).slice(0, 12).map((f, i) => ({ name: str(f?.name, 40) || `feld${i + 1}`, label: str(f?.label, 120),
        art: ['text', 'zahl', 'auswahl', 'mehrzeilig'].includes(f?.art) ? f.art : 'text', optionen: arr(f?.optionen).map((o) => str(o, 200)),
        platzhalter: str(f?.platzhalter, 120), pflicht: !!f?.pflicht }));
      need(felder.length && felder.every((f) => f.label), 'Formular braucht «felder» mit «label».');
      need(felder.every((f) => f.art !== 'auswahl' || f.optionen.length), 'Ein Feld mit «art»: «auswahl» braucht «optionen».');
      return { typ, titel, felder, senden: str(b.senden, 40) || 'Senden' };
    }
    default:
      throw new UiError('Unbekannter Typ.');
  }
}

// JSON eines ```ask-ui-Blocks: ein Baustein oder eine Liste davon
export function parseUi(src) {
  let data;
  try { data = JSON.parse(String(src || '')); } catch (err) { return { ok: false, error: `Kein gültiges JSON (${err.message}).` }; }
  const list = Array.isArray(data) ? data : [data];
  if (!list.length) return { ok: false, error: 'Leerer Baustein.' };
  if (list.length > 8) return { ok: false, error: 'Höchstens 8 Bausteine pro Block.' };
  try {
    return { ok: true, blocks: list.map(normalize) };
  } catch (err) {
    if (err instanceof UiError) return { ok: false, error: err.message };
    throw err;
  }
}

export function checkHtml(src) {
  const html = String(src || '');
  if (!html.trim()) return { ok: false, error: 'Leeres HTML.' };
  if (html.length > LIMIT.html) return { ok: false, error: 'Das HTML ist zu gross (höchstens 200 KB).' };
  return { ok: true, html };
}

// ---- Formeln für den Rechner: + - * / % ^, Klammern, Zahlen, Namen der Eingaben, ein paar Funktionen.
// Eigener kleiner Parser, kein eval: aus einer Formel kann nie Code werden.
const FUNCS = {
  min: Math.min, max: Math.max, abs: Math.abs, sqrt: Math.sqrt, floor: Math.floor, ceil: Math.ceil, pow: Math.pow,
  round: (x, n = 0) => { const f = 10 ** Math.trunc(n); return Math.round(x * f) / f; },
  log: Math.log10, ln: Math.log, exp: Math.exp, sin: Math.sin, cos: Math.cos, tan: Math.tan,
};
const CONSTS = { pi: Math.PI, e: Math.E };

export function evalFormula(expr, vars = {}) {
  const src = String(expr || '');
  if (!src.trim()) throw new Error('leer');
  if (src.length > 400) throw new Error('zu lang');
  const toks = [];
  const re = /\s*(?:(\d+(?:[.,]\d+)?(?:e[+-]?\d+)?)|([a-zA-Z_][a-zA-Z0-9_]*)|(\*\*|[-+*/%^(),]))/y;
  let pos = 0;
  while (pos < src.length) {
    if (/^\s*$/.test(src.slice(pos))) break;
    re.lastIndex = pos;
    const m = re.exec(src);
    if (!m) throw new Error(`unerwartetes Zeichen «${src.slice(pos).trim()[0]}»`);
    pos = re.lastIndex;
    if (m[1]) toks.push({ t: 'n', v: Number(m[1].replace(',', '.')) });
    else if (m[2]) toks.push({ t: 'id', v: m[2] });
    else toks.push({ t: 'op', v: m[3] === '**' ? '^' : m[3] });
  }
  let i = 0;
  const peek = () => toks[i];
  const take = (v) => { if (toks[i]?.t === 'op' && toks[i].v === v) { i++; return true; } return false; };
  const expect = (v) => { if (!take(v)) throw new Error(`«${v}» fehlt`); };
  const own = (o, k) => Object.prototype.hasOwnProperty.call(o, k);
  function primary() {
    const t = peek();
    if (!t) throw new Error('unvollständig');
    if (t.t === 'n') { i++; return t.v; }
    if (t.t === 'id') {
      i++;
      if (take('(')) {
        if (!own(FUNCS, t.v)) throw new Error(`unbekannte Funktion «${t.v}»`);
        const args = [];
        if (!take(')')) { do args.push(sum()); while (take(',')); expect(')'); }
        return FUNCS[t.v](...args);
      }
      if (own(vars, t.v)) return Number(vars[t.v]);
      if (own(CONSTS, t.v)) return CONSTS[t.v];
      throw new Error(`unbekannter Name «${t.v}»`);
    }
    if (take('(')) { const v = sum(); expect(')'); return v; }
    throw new Error(`unerwartet «${t.v}»`);
  }
  function unary() {
    if (take('-')) return -unary();
    if (take('+')) return unary();
    return power();
  }
  function power() {
    const base = primary();
    if (take('^')) return base ** unary(); // rechts gebunden: 2^3^2 = 2^9
    return base;
  }
  function product() {
    let v = unary();
    for (;;) {
      if (take('*')) v *= unary();
      else if (take('/')) v /= unary();
      else if (take('%')) v %= unary();
      else return v;
    }
  }
  function sum() {
    let v = product();
    for (;;) {
      if (take('+')) v += product();
      else if (take('-')) v -= product();
      else return v;
    }
  }
  const v = sum();
  if (i < toks.length) throw new Error(`unerwartet «${toks[i].v}»`);
  return v;
}

// Zahl für Lernende: Schweizer Tausender (’), Komma bleibt Punkt wie in der Schweiz üblich
export function formatNumber(v, stellen = 2) {
  if (!Number.isFinite(v)) return '–';
  const fixed = v.toFixed(stellen);
  const [int, dec] = fixed.split('.');
  const withSep = int.replace(/\B(?=(\d{3})+(?!\d))/g, '’');
  return dec && /[1-9]/.test(dec) ? `${withSep}.${dec.replace(/0+$/, '')}` : withSep;
}
