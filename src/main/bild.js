// Bilder mit Flux (ENTSCHEIDE E31). Flux ist in ASK ein Server-Werkzeug («Flux Bild», Funktion
// bild_erstellen). Die App kann es nicht selbst ausführen: Sie schickt ASK eine eigene, kurze Anfrage
// mit tool_ids, so wie die ASK-Webseite, und holt danach die Bilder ab. Getrennt vom normalen Chat,
// weil sich die Werkzeuge der App und die Server-Werkzeuge in einer Anfrage in die Quere kommen.
//
// Beim Bau liess sich Flux nicht ausprobieren (nur im Beta-Arbeitsbereich). Deshalb fragt die App wie
// die Webseite in einem vorübergehenden Chat in ASK an (dort legt ASK die Dateien ab, die das Werkzeug
// meldet) und sucht die Bilder an allen üblichen Stellen: im gestreamten Ergebnis, in den Dateien der
// Antwort, als Bild-Link oder eingebettet im Text. Der vorübergehende Chat wird danach gelöscht.
// Kommt kein Bild, legt die App die Antwort von ASK unter .ask/arbeit/<Chat>/ ab, damit sich der Weg
// anpassen lässt.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { SseParser } = require('./askapi');
const { checkUrl, decodeEntities } = require('./web');
const { imageUrl } = require('./attachments');

const FUNCTION = 'bild_erstellen';
const FORMATS = ['quadrat', 'hochformat', 'querformat', 'breitbild', 'handy', 'panorama'];
const QUALITIES = ['schnell', 'hoch', 'maximal'];
const MAX_IMAGES = 8;
const MAX_BYTES = 40 * 1024 * 1024;
const TIME_LIMIT = 5 * 60 * 1000;
const POLL_MS = 1500;
const AFTER_IMAGE_MS = 20000; // Bild da, Antworttext noch nicht: so lange höchstens noch warten
const TEMPLATE_TYPES = new Set(['.png', '.jpg', '.jpeg', '.webp', '.gif']);

const SYSTEM = 'Du erstellst Bilder für die App ASK Desktop. Rufe das Werkzeug bild_erstellen genau einmal auf, '
  + 'mit genau den Argumenten aus der Nachricht, ohne etwas wegzulassen oder hinzuzufügen. '
  + 'Gib danach das Ergebnis des Werkzeugs weiter, mit allen Bild-Links, und schreib höchstens einen kurzen Satz dazu.';

// Argumente für das Flux-Werkzeug: nur, was gesetzt ist, unbekannte Werte fallen weg
function fluxArguments(a, editing) {
  const out = { beschreibung: String(a.beschreibung || '').trim() };
  const pick = (v, list) => {
    const s = String(v || '').trim().toLowerCase();
    return list.includes(s) ? s : '';
  };
  if (editing) out.bearbeiten = true;
  const format = pick(a.format, FORMATS);
  if (format) out.format = format;
  const quality = pick(a.qualitaet, QUALITIES);
  if (quality) out.qualitaet = quality;
  if (String(a.stil || '').trim()) out.stil = String(a.stil).trim();
  const count = Math.round(Number(a.anzahl));
  if (count > 1) out.anzahl = Math.min(4, count);
  if (String(a.referenzen || '').trim() && !editing) out.referenzen = String(a.referenzen).trim();
  if (a.tf_branding === true || a.tf_branding === 'true') out.tf_branding = true;
  if (a.verwandeln === true || a.verwandeln === 'true') out.verwandeln = true;
  return out;
}

// Dateityp aus den ersten Bytes; null, wenn es kein Bild ist
function imageType(buf) {
  if (!Buffer.isBuffer(buf) || buf.length < 12) return null;
  if (buf[0] === 0x89 && buf.toString('ascii', 1, 4) === 'PNG') return '.png';
  if (buf[0] === 0xff && buf[1] === 0xd8 && buf[2] === 0xff) return '.jpg';
  if (buf.toString('ascii', 0, 4) === 'RIFF' && buf.toString('ascii', 8, 12) === 'WEBP') return '.webp';
  if (/^GIF8[79]a$/.test(buf.toString('ascii', 0, 6))) return '.gif';
  return null;
}

const DATA_RE = /data:image\/(?:png|jpe?g|webp|gif);base64,[A-Za-z0-9+/=]{100,}/g;
const ASK_RE = /(https?:\/\/[^\s"'<>()\\/]+)?\/(api\/v1\/files\/[\w-]+(?:\/content)?|cache\/image\/[^\s"'<>()\\?#]+)/g;
const MD_RE = /!\[[^\]]*\]\(\s*<?([^\s)>]+)>?(?:\s+"[^"]*")?\s*\)/g;
const WEB_RE = /https?:\/\/[^\s"'<>()\\]+\.(?:png|jpe?g|webp|gif)(?:\?[^\s"'<>()\\]*)?/gi;
const REFERENCE_HOSTS = /(^|\.)(wikipedia|wikimedia)\.org$/i; // Referenzfotos des Werkzeugs, nicht das Ergebnis

// Sucht Bilder in einer Antwort von ASK (beliebiges JSON oder Text). origin: Adresse von ASK.
// strong: sicher ein Ergebnis (Dateien von ASK, eingebettete Bilder, Dateien vom Typ Bild);
// weak: Bild-Links auf andere Server, nur wenn es sonst nichts gibt.
function findImageRefs(value, origin, found = { strong: [], weak: [] }) {
  const add = (list, ref) => {
    if (ref && !found.strong.includes(ref) && !found[list].includes(ref)) found[list].push(ref);
  };
  const askPath = (p) => `/${p.replace(/^api\/v1\/files\/([\w-]+)(?:\/content)?$/, 'api/v1/files/$1/content')}`;
  const web = (url, list) => {
    try {
      const u = new URL(url);
      if (/^https?:$/.test(u.protocol) && !REFERENCE_HOSTS.test(u.hostname)) add(list, u.href);
    } catch { /* keine Adresse */ }
  };
  const fromText = (raw) => {
    const text = decodeEntities(String(raw));
    for (const m of text.match(DATA_RE) || []) add('strong', m);
    for (const m of text.matchAll(ASK_RE)) {
      if (!m[1] || m[1] === origin) add('strong', askPath(m[2]));
    }
    for (const m of text.matchAll(MD_RE)) {
      if (/^https?:/i.test(m[1]) && !m[1].startsWith(`${origin}/`)) web(m[1], 'weak');
    }
    for (const m of text.match(WEB_RE) || []) if (!m.startsWith(`${origin}/`)) web(m, 'weak');
  };
  const walk = (v, depth) => {
    if (depth > 12 || v == null) return;
    if (typeof v === 'string') { if (v.length > 30) fromText(v); return; }
    if (Array.isArray(v)) { for (const x of v) walk(x, depth + 1); return; }
    if (typeof v !== 'object') return;
    // Datei vom Typ Bild, wie Werkzeuge sie über das Ereignis «files» melden
    if (/image/i.test(String(v.type || v.content_type || v.meta?.content_type || ''))) {
      const url = typeof v.url === 'string' ? v.url : typeof v.image_url?.url === 'string' ? v.image_url.url : '';
      if (url.startsWith('data:image/')) add('strong', url);
      else if (/^https?:/i.test(url) && !url.startsWith(`${origin}/`)) web(url, 'strong');
      else if (url) {
        const rel = url.startsWith(origin) ? url.slice(origin.length) : url.startsWith('/') ? url : `/${url}`;
        add('strong', rel.replace(/^\/api\/v1\/files\/([\w-]+)(?:\/content)?$/, '/api/v1/files/$1/content'));
      } else if (typeof v.id === 'string' && /^[\w-]+$/.test(v.id)) add('strong', `/api/v1/files/${v.id}/content`);
    }
    for (const x of Object.values(v)) walk(x, depth + 1);
  };
  walk(value, 0);
  return found;
}

// Lesbarer Antworttext ohne Bilder, Werkzeugblöcke und Markup, gekürzt
function plainText(content) {
  return decodeEntities(String(content || '')
    .replace(/<details[\s\S]*?<\/details>/gi, '')
    .replace(MD_RE, '')
    .replace(DATA_RE, '')
    .replace(/<[^>]+>/g, ''))
    .replace(/\n{3,}/g, '\n\n').trim().slice(0, 600);
}

function slug(text) {
  const s = String(text || '').toLowerCase()
    .replace(/ä/g, 'ae').replace(/ö/g, 'oe').replace(/ü/g, 'ue').replace(/ß/g, 'ss')
    .replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '');
  return s.slice(0, 40).replace(/-+$/, '') || 'bild';
}

// Pfad im Arbeitsordner (posix, relativ); Fehler, wenn er hinausführt
function inside(root, rel) {
  const full = path.resolve(root, String(rel || '').trim());
  const back = path.relative(root, full);
  if (!back || back.startsWith('..') || path.isAbsolute(back)) throw new Error(`Der Pfad «${rel}» liegt nicht im Arbeitsordner.`);
  return back.split(path.sep).join('/');
}

// Dateinamen für die Bilder, wie die App sonst im Ordner arbeitet (Rückgängig geht immer):
// - mit «datei» genau dort, auch wenn es die Datei schon gibt
// - beim Bearbeiten ersetzt das Ergebnis die Vorlage, ausser sie ist ein Anhang unter .ask
//   (dann kommt es nach bilder/) oder das Format ändert sich (dann daneben)
// - sonst unter bilder/ mit einem Namen aus der Beschreibung, ohne Bestehendes zu überschreiben
function saveNames(root, { datei, beschreibung, vorlage }, exts) {
  const sameType = (a, b) => a.toLowerCase().replace('jpeg', 'jpg') === b.toLowerCase().replace('jpeg', 'jpg');
  let dir = 'bilder';
  let stem;
  let given = ''; // Endung der gewünschten Datei, wenn sie zum Bild passt
  let replace = false;
  const target = String(datei || '').trim() ? inside(root, datei) : vorlage && !/^\.ask\//i.test(vorlage) ? vorlage : '';
  if (target) {
    dir = path.posix.dirname(target);
    given = path.posix.extname(target);
    stem = path.posix.basename(target, given);
    replace = exts.length === 1 && (!!String(datei || '').trim() || sameType(given, exts[0]));
  } else {
    stem = slug(vorlage ? path.posix.basename(vorlage, path.posix.extname(vorlage)) : beschreibung);
  }
  const taken = new Set();
  const join = (name) => (dir === '.' ? name : `${dir}/${name}`);
  return exts.map((ext, i) => {
    const end = given && sameType(given, ext) ? given : ext;
    const base = exts.length > 1 ? `${stem}-${i + 1}` : stem;
    let rel = join(`${base}${end}`);
    for (let n = 2; !replace && (fs.existsSync(path.join(root, rel)) || taken.has(rel)); n++) rel = join(`${base}-${n}${end}`);
    taken.add(rel);
    return rel;
  });
}

const sleep = (ms, signal) => new Promise((resolve, reject) => {
  if (signal?.aborted) return reject(Object.assign(new Error('abgebrochen'), { name: 'AbortError' }));
  const t = setTimeout(resolve, ms);
  signal?.addEventListener('abort', () => { clearTimeout(t); reject(Object.assign(new Error('abgebrochen'), { name: 'AbortError' })); }, { once: true });
});

class Images {
  // ask: AskSession (raw, base); getModel: Modell der App; fetchImpl: für Bilder auf anderen Servern
  constructor({ ask, getModel, fetchImpl, allowLocal = false, resize = null, pollMs = POLL_MS }) {
    this.ask = ask;
    this.pollMs = pollMs;
    this.getModel = getModel || (() => '');
    this.fetchImpl = fetchImpl || ((url, options) => fetch(url, options));
    this.allowLocal = allowLocal;
    this.resize = resize;
    this.tool = null; // { id, name } des Flux-Werkzeugs in ASK
  }

  available() {
    return !!this.tool;
  }

  // Bietet ASK diesem Konto das Flux-Werkzeug an? Sucht in den Werkzeugen nach bild_erstellen.
  async discover() {
    try {
      const res = await this.ask.raw('/api/v1/tools/');
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const data = await res.json();
      const list = Array.isArray(data) ? data : data?.items || data?.data || [];
      const found = list.find((t) => (t?.specs || []).some((s) => s?.name === FUNCTION))
        || list.find((t) => !Array.isArray(t?.specs) && /flux/i.test(`${t?.id} ${t?.name}`));
      this.tool = found ? { id: String(found.id), name: String(found.name || found.id) } : null;
    } catch {
      this.tool = null;
    }
    return this.tool;
  }

  forget() {
    this.tool = null;
  }

  origin() {
    return new URL(this.ask.base).origin;
  }

  async json(p, options) {
    const res = await this.ask.raw(p, options);
    if (!res.ok) throw Object.assign(new Error(`HTTP ${res.status}`), { status: res.status });
    return res.json();
  }

  // Vorlage zum Bearbeiten: Bild im Arbeitsordner als data-URL
  template(root, rel) {
    const clean = inside(root, rel);
    if (!TEMPLATE_TYPES.has(path.extname(clean).toLowerCase())) throw new Error('Die Vorlage muss ein Bild sein (PNG, JPG, WebP oder GIF).');
    if (!fs.existsSync(path.join(root, clean))) throw new Error(`Die Vorlage «${clean}» gibt es nicht.`);
    const url = imageUrl(root, { path: clean }, this.resize);
    if (!url) throw new Error('Die Vorlage ist zu gross oder nicht lesbar.');
    return { path: clean, url };
  }

  // Vorübergehender Chat in ASK, wie ihn die Webseite anlegt: Dort speichert ASK, was das Werkzeug
  // meldet (Dateien, Status), auch wenn die Antwort im Hintergrund läuft.
  async tempChat(model, text, template) {
    const user = crypto.randomUUID();
    const answer = crypto.randomUUID();
    const now = Math.floor(Date.now() / 1000);
    const files = template ? [{ type: 'image', url: template.url }] : [];
    const messages = {
      [user]: { id: user, parentId: null, childrenIds: [answer], role: 'user', content: text, files, timestamp: now, models: [model] },
      [answer]: { id: answer, parentId: user, childrenIds: [], role: 'assistant', content: '', model, modelName: model, timestamp: now, done: false },
    };
    const chat = { title: 'ASK Desktop: Bild', models: [model], messages: Object.values(messages), history: { messages, currentId: answer }, tags: [], timestamp: Date.now() };
    const data = await this.json('/api/v1/chats/new', { method: 'POST', body: JSON.stringify({ chat }) });
    if (!data?.id) throw new Error('kein Chat');
    return { id: String(data.id), answer };
  }

  async answerOf(chat) {
    const data = await this.json(`/api/v1/chats/${encodeURIComponent(chat.id)}`);
    const all = data?.chat?.history?.messages || {};
    return { message: all[chat.answer] || null, others: Object.values(all).filter((m) => m?.role === 'assistant' && m.id !== chat.answer) };
  }

  // Läuft im Hintergrund noch eine Antwort in diesem Chat? null: weiss ASK nicht zu sagen.
  async running(chat) {
    try {
      const data = await this.json(`/api/tasks/chat/${encodeURIComponent(chat.id)}`);
      return Array.isArray(data?.task_ids) ? data.task_ids.length > 0 : null;
    } catch {
      return null;
    }
  }

  // Erstellt Bilder und speichert sie im Arbeitsordner. Liefert { files, text }.
  async create(root, args, { chat = 'chat', signal, onProgress } = {}) {
    if (!this.tool) await this.discover();
    if (!this.tool) throw new Error('Flux ist in ASK für dein Konto nicht freigegeben, Bilder lassen sich darum nicht erstellen.');
    const model = this.getModel();
    if (!model) throw new Error('Kein Modell gewählt.');
    if (!String(args.beschreibung || '').trim()) throw new Error('Die Beschreibung des Bildes fehlt.');
    const template = String(args.vorlage || '').trim() ? this.template(root, args.vorlage) : null;
    if (String(args.datei || '').trim()) inside(root, args.datei); // falscher Pfad: melden, bevor Flux arbeitet
    const flux = fluxArguments(args, !!template);
    const text = `Rufe ${FUNCTION} mit genau diesen Argumenten auf:\n${JSON.stringify(flux, null, 2)}`
      + (template ? '\n\nDas Bild zum Bearbeiten ist dieser Nachricht angehängt.' : '');

    const controller = new AbortController();
    const abort = () => controller.abort();
    signal?.addEventListener('abort', abort, { once: true });
    const limit = setTimeout(abort, TIME_LIMIT);
    const started = Date.now();
    const trace = { zeit: new Date().toISOString(), werkzeug: this.tool, argumente: flux, ablauf: [] };
    let status = 'Flux erstellt das Bild';
    const progress = () => onProgress?.(`${status} … ${Math.round((Date.now() - started) / 1000)} s`);
    progress();
    const ticker = setInterval(progress, 1000);
    let temp = null;
    let run = null;
    try {
      try {
        temp = await this.tempChat(model, text, template);
        trace.ablauf.push(`Vorübergehender Chat ${temp.id}`);
      } catch (err) {
        trace.ablauf.push(`Ohne Chat (${err.message})`);
      }
      const found = { strong: [], weak: [] };
      const origin = this.origin();
      run = await this.request({ model, text, template, temp, signal: controller.signal, found, origin, trace });
      let content = run.content;
      if (temp) {
        // Hintergrund: warten, bis ASK fertig ist; sonst einmal nachlesen, was ASK im Chat abgelegt hat
        const answer = run.background ? await this.wait(temp, controller.signal, found, origin, trace, (s) => { status = s; })
          : await this.answerOf(temp).then((a) => a.message).catch(() => null);
        if (answer) {
          trace.antwort = answer;
          findImageRefs(answer, origin, found);
          content = content || String(answer.content || '');
        }
      }
      const refs = (found.strong.length ? found.strong : found.weak).filter((r) => r !== template?.url).slice(0, MAX_IMAGES);
      const images = [];
      const seen = new Set(); // dasselbe Bild unter zwei Adressen (Datei und Link) nur einmal
      for (const ref of refs) {
        try {
          const data = await this.download(ref, controller.signal);
          const ext = imageType(data);
          const hash = crypto.createHash('sha1').update(data).digest('hex');
          if (ext && !seen.has(hash)) images.push({ data, ext });
          else if (!ext) trace.ablauf.push(`Kein Bild: ${ref.slice(0, 120)}`);
          seen.add(hash);
        } catch (err) {
          if (controller.signal.aborted) throw err;
          trace.ablauf.push(`Nicht geladen: ${ref.slice(0, 120)} (${err.message})`);
        }
      }
      if (!images.length) {
        const saved = this.keepTrace(root, chat, trace, content);
        const said = plainText(content);
        const reason = run.toolCall ? 'ASK hat Flux nicht selbst ausgeführt, sondern den Aufruf zurückgegeben.'
          : run.error ? `ASK meldet: ${run.error}` : 'ASK hat kein Bild zurückgegeben.';
        throw new Error(`${reason}${said ? ` Antwort von ASK: «${said.slice(0, 300)}»` : ''}${saved ? ` Details: ${saved}` : ''}`);
      }
      const names = saveNames(root, { datei: args.datei, beschreibung: flux.beschreibung, vorlage: template?.path }, images.map((i) => i.ext));
      names.forEach((rel, i) => {
        const full = path.join(root, rel);
        fs.mkdirSync(path.dirname(full), { recursive: true });
        fs.writeFileSync(full, images[i].data);
      });
      return { files: names, text: plainText(content), seconds: Math.round((Date.now() - started) / 1000), template: template?.path || '' };
    } catch (err) {
      if (controller.signal.aborted && !signal?.aborted) throw new Error(`Flux hat nach ${TIME_LIMIT / 60000} Minuten kein Bild geliefert.`);
      throw err;
    } finally {
      clearInterval(ticker);
      clearTimeout(limit);
      signal?.removeEventListener('abort', abort);
      // Abgebrochen: die Antwort im Hintergrund in ASK ebenfalls anhalten
      if (controller.signal.aborted && run?.task) this.ask.raw(`/api/tasks/stop/${encodeURIComponent(run.task)}`, { method: 'POST' }).catch(() => {});
      if (temp) this.ask.raw(`/api/v1/chats/${encodeURIComponent(temp.id)}`, { method: 'DELETE' }).catch(() => {});
    }
  }

  // Die Anfrage an ASK. Liefert { content, background, toolCall, error }.
  async request({ model, text, template, temp, signal, found, origin, trace }) {
    const send = (withImage) => {
      const body = {
        model,
        stream: true,
        messages: [{ role: 'system', content: SYSTEM },
          { role: 'user', content: withImage ? [{ type: 'text', text }, { type: 'image_url', image_url: { url: template.url } }] : text }],
        tool_ids: [this.tool.id],
        features: { web_search: false, image_generation: false },
        background_tasks: { title_generation: false, tags_generation: false, follow_up_generation: false },
      };
      // Wie die Webseite: mit Chat, Nachricht und Sitzung führt ASK die Werkzeuge selbst aus
      if (temp) Object.assign(body, { chat_id: temp.id, id: temp.answer, session_id: `ask-desktop-${crypto.randomBytes(6).toString('hex')}` });
      return this.ask.raw('/api/chat/completions', { method: 'POST', body: JSON.stringify(body), signal });
    };
    const failure = async (res) => {
      const detail = await res.text().catch(() => '');
      let message = detail;
      try { message = JSON.parse(detail).detail || JSON.parse(detail).error?.message || detail; } catch { /* Text */ }
      if (typeof message !== 'string') message = JSON.stringify(message);
      return Object.assign(new Error(`ASK: ${message.slice(0, 300) || `HTTP ${res.status}`}`), { status: res.status });
    };
    let res = await send(!!template);
    if (!res.ok) {
      const err = await failure(res);
      // Nimmt das Modell keine Bilder an, liegt die Vorlage nur im Chat; das Werkzeug lädt sie selbst
      if (!(template && res.status === 400 && /image|vision|multimodal|modalit/i.test(err.message))) throw err;
      trace.ablauf.push('Modell nimmt keine Bilder an: Vorlage nur im Chat');
      res = await send(false);
      if (!res.ok) throw await failure(res);
    }
    const type = res.headers.get('content-type') || '';
    if (!type.includes('text/event-stream')) {
      const data = await res.json().catch(() => ({}));
      trace.ablauf.push(`Antwort als JSON: ${JSON.stringify(data).slice(0, 300)}`);
      if (data && (data.task_id || data.status === true) && !data.choices) return { content: '', background: true, task: data.task_id || '' };
      findImageRefs(data, origin, found);
      const message = data?.choices?.[0]?.message || {};
      return { content: String(message.content || ''), toolCall: (message.tool_calls || []).some((c) => c?.function?.name === FUNCTION),
        error: typeof data?.detail === 'string' ? data.detail : data?.error?.message };
    }
    trace.ablauf.push('Antwort gestreamt');
    trace.strom = [];
    let content = '';
    let toolCall = false;
    let error = '';
    const parser = new SseParser((chunk) => {
      if (trace.strom.length < 200) trace.strom.push(JSON.stringify(chunk).slice(0, 2000));
      findImageRefs(chunk, origin, found);
      const d = chunk?.choices?.[0]?.delta || chunk?.choices?.[0]?.message || {};
      if (typeof d.content === 'string') content += d.content;
      if ((d.tool_calls || []).some((c) => c?.function?.name === FUNCTION)) toolCall = true;
      if (chunk?.error) error = chunk.error.message || JSON.stringify(chunk.error);
    });
    const decoder = new TextDecoder();
    const reader = res.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      parser.push(decoder.decode(value, { stream: true }));
    }
    parser.push('\n');
    // Links kommen in kleinen Stücken: erst im ganzen Text auffindbar
    findImageRefs(content, origin, found);
    return { content, toolCall, error };
  }

  // Antwort im Hintergrund: den Chat in ASK lesen, bis die Antwort fertig ist
  async wait(temp, signal, found, origin, trace, onStatus) {
    let last = null;
    let lastContent = '';
    let firstImage = 0;
    for (let round = 0; ; round++) {
      await sleep(this.pollMs, signal);
      let answer;
      try {
        answer = await this.answerOf(temp);
      } catch (err) {
        trace.ablauf.push(`Chat nicht lesbar: ${err.message}`);
        if (round > 3) return last;
        continue;
      }
      const message = answer.message || answer.others[answer.others.length - 1] || null;
      if (!message) continue;
      last = message;
      const s = message.statusHistory?.[message.statusHistory.length - 1] || message.status;
      if (s?.description) onStatus(String(s.description).slice(0, 80));
      const before = found.strong.length;
      findImageRefs(message, origin, found);
      if (found.strong.length > before && !firstImage) firstImage = Date.now();
      const content = String(message.content || '');
      const busy = await this.running(temp);
      if (message.done === true || busy === false) return message;
      // ASK sagt nicht, ob es fertig ist: fertig, wenn der Text steht
      if (busy === null && content.trim() && content === lastContent) return message;
      if (firstImage && Date.now() - firstImage > AFTER_IMAGE_MS) return message;
      lastContent = content;
    }
  }

  // Bild laden: eingebettet, von ASK (mit Anmeldung) oder von einem anderen Server (ohne)
  async download(ref, signal) {
    if (ref.startsWith('data:')) return Buffer.from(ref.slice(ref.indexOf(',') + 1), 'base64');
    const origin = this.origin();
    const url = new URL(ref, origin);
    let res;
    if (url.origin === origin) res = await this.ask.raw(url.pathname + url.search, { signal });
    else res = await this.fetchImpl(checkUrl(url.href, this.allowLocal).href, { signal });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    if (Number(res.headers.get('content-length') || 0) > MAX_BYTES) throw new Error('grösser als 40 MB');
    const data = Buffer.from(await res.arrayBuffer());
    if (data.length > MAX_BYTES) throw new Error('grösser als 40 MB');
    return data;
  }

  // Antwort von ASK zum Nachsehen, wenn kein Bild kam (unter .ask/arbeit, mit dem Chat gelöscht)
  keepTrace(root, chat, trace, content) {
    try {
      const rel = `.ask/arbeit/${chat}/flux-${Date.now().toString(36)}.json`;
      fs.mkdirSync(path.dirname(path.join(root, rel)), { recursive: true });
      const clipData = (_k, v) => (typeof v === 'string' && v.startsWith('data:') ? `${v.slice(0, 60)}… (${v.length} Zeichen)` : v);
      fs.writeFileSync(path.join(root, rel), JSON.stringify({ ...trace, text: content }, clipData, 2));
      return rel;
    } catch {
      return '';
    }
  }
}

module.exports = { Images, fluxArguments, findImageRefs, imageType, saveNames, plainText, slug, FUNCTION, FORMATS, QUALITIES };
