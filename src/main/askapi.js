// Verbindung mit ASK (Open WebUI) über dieselbe Schnittstelle, die auch die ASK-Webseite benutzt
// (ENTSCHEIDE E19). Angemeldet wird in der ASK-Seite, auch über das Microsoft-Konto. Danach liest
// die App das Token aus der Seite und spricht das Modell direkt an. Die Werkzeuge führt die App
// selbst aus (agent.js, tools.js); ASK braucht dafür keine Freigabe für Tool-Server.
const { EventEmitter } = require('node:events');

class AuthError extends Error {}

// Zerlegt einen Server-Sent-Events-Strom in JSON-Ereignisse («data: {...}»).
class SseParser {
  constructor(onEvent) {
    this.buffer = '';
    this.onEvent = onEvent;
  }

  push(text) {
    this.buffer += text;
    let i;
    while ((i = this.buffer.indexOf('\n')) >= 0) {
      const line = this.buffer.slice(0, i).replace(/\r$/, '');
      this.buffer = this.buffer.slice(i + 1);
      if (!line.startsWith('data:')) continue;
      const data = line.slice(5).trim();
      if (data === '[DONE]') { this.done = true; continue; }
      if (!data) continue;
      try {
        this.onEvent(JSON.parse(data));
      } catch {
        // kein JSON (Kommentar oder Statuszeile)
      }
    }
  }
}

// Trennt <think>…</think> aus dem Text, auch wenn die Marken über mehrere Teile verteilt ankommen.
class ThinkSplitter {
  constructor(emit) {
    this.emit = emit; // (kind, text) mit kind «text» oder «reasoning»
    this.inThink = false;
    this.pending = '';
  }

  push(text) {
    let rest = this.pending + text;
    this.pending = '';
    while (rest) {
      const tag = this.inThink ? '</think>' : '<think>';
      const i = rest.indexOf(tag);
      if (i >= 0) {
        if (i > 0) this.emit(this.inThink ? 'reasoning' : 'text', rest.slice(0, i));
        this.inThink = !this.inThink;
        rest = rest.slice(i + tag.length);
        continue;
      }
      // Ein angefangenes Tag am Ende zurückhalten
      let keep = 0;
      for (let n = Math.min(tag.length - 1, rest.length); n > 0; n--) {
        if (tag.startsWith(rest.slice(-n))) { keep = n; break; }
      }
      const out = rest.slice(0, rest.length - keep);
      if (out) this.emit(this.inThink ? 'reasoning' : 'text', out);
      this.pending = rest.slice(rest.length - keep);
      rest = '';
    }
  }

  flush() {
    if (this.pending) this.emit(this.inThink ? 'reasoning' : 'text', this.pending);
    this.pending = '';
  }
}

// Grobe Schätzung, wenn ASK keine Tokenzahlen liefert (Deutsch, Englisch und Code gemischt).
const CHARS_PER_TOKEN = 3.6;

// Sammelt eine (gestreamte oder ganze) Antwort im OpenAI-Format: Text, Denken, Werkzeugaufrufe,
// dazu Tokenzahlen (falls ASK sie liefert) und Zeiten für die Anzeige der Geschwindigkeit.
class Completion {
  constructor(onDelta = () => {}, now = Date.now) {
    this.content = '';
    this.reasoning = '';
    this.calls = [];
    this.finish = '';
    this.error = '';
    this.usage = null;
    this.sources = [];
    this.now = now;
    this.started = now();
    this.firstAt = 0;
    this.lastAt = 0;
    this.onDelta = onDelta;
    this.splitter = new ThinkSplitter((kind, text) => {
      if (kind === 'text') this.content += text;
      else this.reasoning += text;
      this.onDelta(kind, text);
    });
  }

  add(chunk) {
    if (!chunk || typeof chunk !== 'object') return;
    if (chunk.error) {
      const e = chunk.error;
      this.error = typeof e === 'string' ? e : e.message || e.detail || JSON.stringify(e);
      this.errorCode = Number(e.code ?? e.status ?? e.status_code ?? chunk.status_code) || 0;
      return;
    }
    if (typeof chunk.detail === 'string') {
      this.error = chunk.detail;
      return;
    }
    if (chunk.usage && (chunk.usage.completion_tokens || chunk.usage.prompt_tokens)) this.usage = chunk.usage;
    if (chunk.sources) this.sources.push(...parseSources(chunk.sources).filter((s) => !this.sources.some((o) => o.url === s.url)));
    const choice = chunk.choices?.[0];
    if (!choice) return;
    const d = choice.delta || choice.message || {};
    if (d.content || d.reasoning_content || d.reasoning || d.tool_calls) {
      this.lastAt = this.now();
      if (!this.firstAt) this.firstAt = this.lastAt;
    }
    const reasoning = d.reasoning_content ?? d.reasoning;
    if (reasoning) {
      this.reasoning += reasoning;
      this.onDelta('reasoning', reasoning);
    }
    if (d.content) this.splitter.push(d.content);
    const whole = !choice.delta && !!choice.message; // ganze Antwort: Aufrufe der Reihe nach
    (d.tool_calls || []).forEach((tc, n) => {
      const call = this.calls[this.slotFor(tc, whole ? n : null)] ||= { id: '', name: '', arguments: '' };
      if (tc.id) call.id = tc.id;
      const name = tc.function?.name;
      if (name && call.name !== name) call.name += name;
      const args = tc.function?.arguments;
      if (args) call.arguments += typeof args === 'string' ? args : JSON.stringify(args);
    });
    if (choice.finish_reason) this.finish = choice.finish_reason;
  }

  // Platz eines gestreamten Werkzeugaufrufs. Ohne «index» (manche Server) oder mit demselben index für
  // verschiedene Aufrufe entscheidet die Kennung: eine neue Kennung ist ein neuer Aufruf, Teile ohne
  // Kennung gehören zum letzten.
  slotFor(tc, fixed) {
    if (fixed !== null) return fixed;
    const byId = tc.id ? this.calls.findIndex((c) => c && c.id === tc.id) : -1;
    if (byId >= 0) return byId;
    const index = Number.isInteger(tc.index) ? tc.index : null;
    if (index !== null) {
      this.remap ||= new Map();
      const slot = this.remap.get(index) ?? index;
      const at = this.calls[slot];
      // gleicher index, aber eine andere Kennung: ein weiterer Aufruf; folgende Teile mit diesem index gehören zu ihm
      if (at && tc.id && at.id && at.id !== tc.id) {
        this.remap.set(index, this.calls.length);
        return this.calls.length;
      }
      return slot;
    }
    if (!this.calls.length || (tc.id && this.calls.at(-1).id)) return this.calls.length;
    return this.calls.length - 1;
  }

  end() {
    this.splitter.flush();
    this.calls = this.calls.filter((c) => c && c.name);
    return this;
  }

  // Ausgabetoken und Zeiten dieser Antwort. genMs: vom ersten bis zum letzten gestreamten Teil.
  stats() {
    const end = this.lastAt || this.now();
    const chars = this.content.length + this.reasoning.length + this.calls.reduce((n, c) => n + c.name.length + c.arguments.length, 0);
    const measured = this.usage?.completion_tokens;
    return {
      tokens: measured || Math.round(chars / CHARS_PER_TOKEN),
      estimated: !measured,
      promptTokens: this.usage?.prompt_tokens || 0,
      genMs: this.firstAt ? end - this.firstAt : 0,
      firstMs: this.firstAt ? this.firstAt - this.started : 0,
      totalMs: end - this.started,
    };
  }
}

// Werkzeugaufrufe im Text, wie Qwen sie schreibt, wenn der Server sie nicht selbst auswertet:
// <tool_call>{"name": "...", "arguments": {...}}</tool_call>
function parseTextToolCalls(content) {
  const calls = [];
  const text = String(content || '').replace(/<tool_call>\s*([\s\S]*?)\s*<\/tool_call>/g, (_m, body) => {
    try {
      const json = JSON.parse(body.replace(/^```(?:json)?\s*|\s*```$/g, ''));
      if (json && json.name) {
        const args = json.arguments ?? json.parameters ?? {};
        calls.push({ id: '', name: String(json.name), arguments: typeof args === 'string' ? args : JSON.stringify(args) });
        return '';
      }
    } catch {
      // kein gültiger Aufruf, Text stehen lassen
    }
    return _m;
  });
  return { text: text.trim(), calls };
}

// Denkaufwand (Regler wie «Reasoning Effort» in ASK). reasoning_effort verstehen OpenAI-kompatible
// Server und ASK selbst; für Hoch und Maximal zusätzlich vLLM (chat_template_kwargs) und Ollama (think).
// main.js setzt dazu den Qwen-Schalter /think. «standard» schickt nichts, das Modell entscheidet selbst.
const THINKING_LEVELS = ['standard', 'tief', 'mittel', 'hoch', 'maximal'];
const THINKING_PARAMS = {
  tief: { reasoning_effort: 'low' },
  mittel: { reasoning_effort: 'medium' },
  hoch: { reasoning_effort: 'high', think: true, chat_template_kwargs: { enable_thinking: true } },
  maximal: { reasoning_effort: 'high', think: true, chat_template_kwargs: { enable_thinking: true } },
};

// Lehnt ein Modell eine Stufe ab und nennt die erlaubten («Unexpected reasoning effort high. Supported
// types are xhigh (default), medium, and low»), die nächste erlaubte Stufe wählen, bei Gleichstand die höhere.
// null: keine Liste in der Meldung oder die Stufe ist schon erlaubt.
const EFFORTS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh'];
function nearestEffort(message, wanted) {
  const list = /supported[^:.]*?(?:are|:)\s*([^.]+)/i.exec(String(message))?.[1] || '';
  const allowed = EFFORTS.filter((e) => new RegExp(`\\b${e}\\b`, 'i').test(list));
  if (!allowed.length || allowed.includes(wanted)) return null;
  const at = EFFORTS.indexOf(wanted);
  return allowed.reduce((best, e) => {
    const d = Math.abs(EFFORTS.indexOf(e) - at);
    const bd = Math.abs(EFFORTS.indexOf(best) - at);
    return d < bd || (d === bd && EFFORTS.indexOf(e) > EFFORTS.indexOf(best)) ? e : best;
  });
}

// Websuche in ASK eingeschaltet und für dieses Konto erlaubt? (Admins dürfen immer.)
function webSearchAllowed(config, user) {
  if (config?.features?.enable_web_search !== true) return false;
  if (user?.role === 'admin') return true;
  return user?.permissions?.features?.web_search !== false;
}

// Quellen, die ASK bei der Websuche mitschickt («sources»), als Liste { title, url } ohne Doppelte.
function parseSources(list) {
  const seen = new Set();
  const out = [];
  for (const s of Array.isArray(list) ? list : []) {
    const metas = Array.isArray(s?.metadata) ? s.metadata : [];
    const urls = Array.isArray(s?.source?.urls) ? s.source.urls : [];
    const items = metas.length ? metas.map((m) => ({ url: m?.source || m?.url, title: m?.title || m?.name }))
      : urls.map((url) => ({ url, title: '' }));
    for (const item of items) {
      if (!item.url || !/^https?:\/\//i.test(item.url) || seen.has(item.url)) continue;
      seen.add(item.url);
      out.push({ url: item.url, title: String(item.title || '').trim() || item.url.replace(/^https?:\/\/(www\.)?/i, '').split('/')[0] });
    }
  }
  return out;
}

// Terminal-Einträge, die Versionen bis 29.10.2026 in den ASK-Einstellungen hinterlassen haben (E30).
// Nur genau dieser Eintrag zählt, eigene Terminals der Person bleiben unberührt.
function isStaleTerminal(s) {
  return s?.name === 'ASK Desktop' && s?.key === 'ask-desktop' && /^http:\/\/127\.0\.0\.1:\d+\/?$/.test(String(s?.url));
}

// Kontextfenster eines Modells aus den Angaben von /api/models. Je nach Modellserver steht es an anderer Stelle
// (Ollama: num_ctx, vLLM: max_model_len, andere: context_length oder context_window).
function contextOf(m) {
  const candidates = [m?.info?.params?.num_ctx, m?.context_length, m?.context_window, m?.max_model_len, m?.info?.meta?.context_length,
    m?.openai?.context_length, m?.openai?.context_window, m?.openai?.max_model_len, m?.top_provider?.context_length];
  const n = candidates.map(Number).find((v) => Number.isFinite(v) && v >= 1000);
  return n ? Math.round(n) : 0;
}

// Fehler, den ASK in einer Antwort mit Status 200 meldet (im Strom oder als JSON). Ohne Statuscode zählt
// er wie eine Ablehnung (400), damit «Chat zu lang» und die anderen Rückfallmodi greifen; Überlastung
// wie 503, damit die App nochmals versucht.
function inbandError(completion) {
  const message = completion.error;
  const code = completion.errorCode >= 400 && completion.errorCode < 600 ? completion.errorCode : 0;
  const status = code || (/rate.?limit|too many requests/i.test(message) ? 429
    : /overload|unavailable|temporar|try again|timeout|timed out/i.test(message) ? 503 : 400);
  return Object.assign(new Error(`ASK: ${message}`), { status, partial: completion.end() });
}

async function errorOf(res) {
  const text = await res.text().catch(() => '');
  let detail = text;
  try { detail = JSON.parse(text).detail || JSON.parse(text).error?.message || text; } catch { /* Text */ }
  if (typeof detail !== 'string') detail = JSON.stringify(detail);
  return Object.assign(new Error(`ASK: ${detail || `HTTP ${res.status}`}`), { status: res.status });
}

// Gehört die Adresse zu ASK? Verglichen wird die ganze Herkunft, nicht nur der Anfang der Adresse
// («https://ask.example.evil.org» beginnt auch mit «https://ask.example»).
function sameOrigin(url, base) {
  try { return !!url && !!base && new URL(url).origin === new URL(base).origin; } catch { return false; }
}

// Chromium: Name nicht gefunden (ERR_NAME_NOT_RESOLVED, ERR_NAME_RESOLUTION_FAILED)
const NAME_ERRORS = new Set([-105, -137]);
const isNameError = (err) => /ERR_NAME_(NOT_RESOLVED|RESOLUTION_FAILED)/.test(String(err?.message || err));
const netReason = (err) => (String(err?.message || err).match(/net::(ERR_[A-Z_]+)/) || [])[1] || String(err?.message || err);

// Token aus dem Browser bereinigen: «Bearer » und Leerzeichen weg; ein JWT (drei Teile) oder ein API-Schlüssel (sk-…)
function cleanToken(raw) {
  const t = String(raw || '').trim().replace(/^bearer\s+/i, '').replace(/^["']|["']$/g, '');
  if (!t || t.length > 8192 || /\s/.test(t)) return '';
  if (/^sk-[A-Za-z0-9_-]{8,}$/.test(t) || /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]*$/.test(t)) return t;
  return '';
}

const RETRY_LOAD_MS = 15000; // so oft lädt die ASK-Seite nach einem Ladefehler von selbst neu
const STALL_MS = 5 * 60 * 1000; // so lange darf ASK mitten in einer Antwort schweigen

const RECHECK_MS = { 'nicht-angemeldet': 3000, unbekannt: 1000, pruefen: 500, fehler: 30000, angemeldet: 120000 };

class AskSession extends EventEmitter {
  constructor() {
    super();
    this.base = '';
    this.view = null;
    this.session = null;
    this.token = '';
    this.status = { state: 'unbekannt' };
    this.models = [];
    this.user = null;
    this.timer = null;
    this.busy = false;
    this.lastCheck = 0;
    // Token aus dem Browser (eingefügt oder aus dem Anmeldefenster), falls die ASK-Seite in der App nicht lädt (ENTSCHEIDE E57)
    this.manualToken = '';
    this.tokenStore = null;
  }

  // Ablage für das Token aus dem Browser (verschlüsselt in den Einstellungen, main.js)
  setTokenStore(store) {
    this.tokenStore = store;
    try { this.manualToken = cleanToken(store?.get?.() || ''); } catch { this.manualToken = ''; }
  }

  rememberToken(token) {
    this.manualToken = token;
    try { this.tokenStore?.set?.(token); } catch { /* bleibt nur bis zum Beenden */ }
  }

  attach(view) {
    this.view = view;
    this.session = view.webContents.session;
    const wc = view.webContents;
    // Konnte die ASK-Seite nicht laden (offline, Proxy, Zertifikat), meldet getURL() trotzdem die Adresse von ASK,
    // aber in der Seite steckt die Fehlerseite von Chromium: Token lesen geht dort nicht. Dann nicht prüfen, sonst
    // wird daraus «ASK: Fehler», und die Oberfläche blendet den Anmeldebereich mit «ASK ist nicht erreichbar» aus.
    // Der Ladefehler steht im Status (loadError), nicht in einem Ereignis: Scheitert das Laden gleich beim Start,
    // ist die Oberfläche noch nicht bereit und verpasst ein Ereignis; den Status holt sie beim Start ab (ENTSCHEIDE E53).
    this.loadFailed = false;
    wc.on('did-fail-load', (_e, code, description, url, isMainFrame) => {
      if (!isMainFrame || code === -3) return;
      this.loadFailed = true;
      this.failedAt = Date.now();
      if (this.status.state !== 'angemeldet') this.publish({ ...this.status, loadError: { code, description, url } });
      if (NAME_ERRORS.has(code)) this.emit('name-not-resolved', url);
      // Für die Meldung: geht die App direkt oder über einen Proxy? Unterscheidet sich das vom Browser, ist das oft der Grund.
      Promise.resolve(this.session?.resolveProxy?.(url)).then((proxy) => {
        const e = this.status.loadError;
        if (proxy && e && e.url === url && e.code === code) this.publish({ ...this.status, loadError: { ...e, proxy: String(proxy) } });
      }).catch(() => {});
    });
    wc.on('did-start-navigation', (details) => { if (details?.isMainFrame !== false && !details?.isSameDocument) this.loadFailed = false; });
    // Nach einem Ladefehler kommt für die Fehlerseite von Chromium trotzdem did-finish-load: dann bleibt der Fehler
    wc.on('did-finish-load', () => {
      if (this.loadFailed) return;
      if (this.status.loadError) { const { loadError: _gone, ...rest } = this.status; this.publish(rest); }
      this.check(true);
    });
    wc.on('did-navigate', (_e, url) => this.navigated(url));
    wc.on('did-navigate-in-page', (_e, url) => this.navigated(url));
    clearInterval(this.timer);
    this.timer = setInterval(() => this.tick(), 2000);
  }

  // Alle 2 Sekunden: prüfen; konnte die Seite nicht laden, alle 15 Sekunden von selbst nochmals laden
  // (z. B. WLAN oder VPN beim Start der App noch nicht bereit)
  tick() {
    const wc = this.view?.webContents;
    if (this.loadFailed && wc && !wc.isDestroyed() && this.base && Date.now() - (this.failedAt || 0) > RETRY_LOAD_MS) {
      this.failedAt = Date.now();
      wc.loadURL(this.base).catch(() => {}); // scheitert es wieder, meldet das did-fail-load
      return;
    }
    this.check();
  }

  setBase(url) {
    this.base = String(url || '').replace(/\/$/, '');
  }

  publish(status) {
    this.status = status;
    this.emit('status', status);
  }

  // Verlässt die ASK-Seite die Anmeldung (auch nach dem Microsoft-Login), ist man vermutlich angemeldet:
  // sofort «prüfen» melden, damit die Oberfläche die Seite ausblendet und einen Ladebildschirm zeigt.
  navigated(url) {
    let target;
    try { target = new URL(url); } catch { return; }
    if (!sameOrigin(target.href, this.base)) return; // fremde Anmeldeseite bleibt sichtbar
    if (this.status.state === 'nicht-angemeldet' && !/^\/auth\b/.test(target.pathname)) this.publish({ state: 'pruefen' });
    this.check(true);
  }

  // Liest das Token aus der ASK-Seite und prüft es mit einer Anfrage an ASK.
  async check(force = false) {
    if (this.busy && force) this.again = true; // nach der laufenden Prüfung gleich nochmals
    if (!this.view || !this.base || this.busy) return this.status;
    const wc = this.view.webContents;
    const pageReady = !wc.isDestroyed() && !wc.isLoading() && !this.loadFailed && sameOrigin(wc.getURL(), this.base);
    if (!pageReady && !this.manualToken) {
      if (wc.isDestroyed() || wc.isLoading() || this.loadFailed) return this.status;
      // Fertig geladen, aber nicht auf ASK (z. B. gleich zur Microsoft-Anmeldung weitergeleitet, oder eine Seite des
      // Netzes): die Seite zeigen, statt hinter «Verbinde mit ASK …» zu warten. Angemeldet ist man dort nicht.
      if (['unbekannt', 'pruefen'].includes(this.status.state) && /^https?:/i.test(wc.getURL())) this.publish({ state: 'nicht-angemeldet' });
      return this.status;
    }
    const interval = RECHECK_MS[this.status.state] ?? 30000;
    if (!force && Date.now() - this.lastCheck < interval) return this.status;
    this.lastCheck = Date.now();
    this.busy = true;
    try {
      let token = '';
      if (pageReady) {
        try {
          token = await wc.executeJavaScript('localStorage.token || ""', true);
        } catch (err) {
          // Seite (noch) nicht lesbar, z. B. Fehlerseite: kein Fehler von ASK, später nochmals. Den Grund im Status
          // mitgeben, damit die Oberfläche ihn zeigen kann, wenn das Warten zu lange dauert.
          const detail = `Die ASK-Seite lässt sich nicht lesen: ${err?.message || err}`;
          if (!this.manualToken) {
            if (this.status.state !== 'angemeldet' && this.status.detail !== detail) this.publish({ ...this.status, detail });
            return this.status;
          }
        }
      }
      // Ohne Token auf der Seite gilt das Token aus dem Browser, falls eines da ist
      token = token || this.manualToken;
      if (!token) {
        this.token = '';
        if (this.status.state !== 'nicht-angemeldet') this.publish({ state: 'nicht-angemeldet' });
        return this.status;
      }
      if (token === this.token && this.status.state === 'angemeldet' && !force) return this.status;
      this.token = token;
      await this.signIn(token);
    } catch (err) {
      if (err instanceof AuthError) this.publish({ state: 'nicht-angemeldet' });
      else this.publish({ state: 'fehler', message: err.message });
    } finally {
      this.busy = false;
      if (this.again) {
        this.again = false;
        setTimeout(() => this.check(true), 0);
      }
    }
    return this.status;
  }

  // Token gilt: Konto, Modelle und Websuche von ASK holen und «angemeldet» melden
  async signIn(token) {
    const user = await this.json('/api/v1/auths/');
    const models = await this.loadModels();
    this.user = { name: user.name || user.email || '', role: user.role };
    this.webSearch = webSearchAllowed(await this.json('/api/config').catch(() => ({})), user);
    this.publish({ state: 'angemeldet', user: this.user.name });
    if (models.length === 0) this.publish({ state: 'fehler', message: 'ASK bietet diesem Konto kein Modell an. Wer ASK betreut, muss deinem Konto zuerst ein Modell freigeben.' });
    this.cleanStaleTerminal(token);
  }

  // Anmelden mit einem Token aus dem Browser (eingefügt oder aus dem Anmeldefenster). Das Token wird zuerst bei ASK
  // geprüft; nur ein gültiges bleibt gespeichert. Antwort { ok, reason }.
  async useToken(raw) {
    const token = cleanToken(raw);
    if (!token) return { ok: false, reason: 'Das sieht nicht wie ein Token aus. Kopiere es ganz, ohne Leerzeichen.' };
    if (!this.base || !this.session) return { ok: false, reason: 'Die Adresse von ASK fehlt.' };
    for (let i = 0; this.busy && i < 200; i++) await new Promise((r) => setTimeout(r, 50));
    this.busy = true;
    try {
      let res;
      try {
        res = await this.session.fetch(`${this.base}/api/v1/auths/`, { headers: { Authorization: `Bearer ${token}` } });
      } catch (err) {
        if (isNameError(err)) this.emit('name-not-resolved', this.base);
        return { ok: false, reason: `ASK ist für die App nicht erreichbar (${netReason(err)}). Die App versucht es auch von selbst wieder.` };
      }
      if (res.status === 401 || res.status === 403) return { ok: false, reason: 'ASK kennt dieses Token nicht oder es ist abgelaufen. Kopiere es nochmals aus ASK.' };
      if (!res.ok) return { ok: false, reason: `ASK antwortet mit HTTP ${res.status}.` };
      this.rememberToken(token);
      this.token = token;
      this.lastCheck = Date.now();
      try {
        await this.signIn(token);
      } catch (err) {
        if (err instanceof AuthError) return { ok: false, reason: 'ASK hat das Token gleich wieder abgelehnt.' };
        this.publish({ state: 'fehler', message: err.message });
      }
      return { ok: true };
    } finally {
      this.busy = false;
    }
  }

  // Anmeldung in einem eigenen Fenster mit derselben Sitzung wie die ASK-Seite: Sobald dort ein Token steht
  // (auch nach dem Microsoft-Login, das die Seite ohne neues Laden setzt), übernimmt es die App und meldet «login-done».
  watchLoginWindow(wc) {
    let tried = '';
    const grab = async () => {
      if (wc.isDestroyed() || wc.isLoading() || !sameOrigin(wc.getURL(), this.base)) return;
      let token = '';
      try { token = cleanToken(await wc.executeJavaScript('localStorage.token || ""', true)); } catch { return; }
      if (!token || token === tried) return;
      tried = token;
      const res = await this.useToken(token);
      if (res.ok) this.emit('login-done');
      else tried = ''; // z. B. ASK gerade nicht erreichbar: später nochmals
    };
    const timer = setInterval(grab, 1500);
    wc.on('did-finish-load', grab);
    wc.on('did-navigate-in-page', grab);
    wc.once('destroyed', () => clearInterval(timer));
    return () => clearInterval(timer);
  }

  async request(path, options = {}) {
    if (!this.token) throw new AuthError('Nicht bei ASK angemeldet.');
    const res = await this.session.fetch(this.base + path, {
      ...options,
      headers: { Authorization: `Bearer ${this.token}`, 'Content-Type': 'application/json', ...(options.headers || {}) },
    });
    // Nur 401 heisst «nicht mehr angemeldet». 403 kommt auch, wenn das Konto ein Modell oder eine Funktion
    // nicht benutzen darf; das meldet der Aufrufer als Fehler, die Anmeldung bleibt.
    if (res.status === 401) {
      if (this.token && this.token === this.manualToken) this.rememberToken('');
      this.token = '';
      this.publish({ state: 'nicht-angemeldet' });
      throw new AuthError('Die Anmeldung bei ASK ist abgelaufen. Bitte neu anmelden.');
    }
    return res;
  }

  // Anfrage an ASK ohne die Abmelde-Logik von request(), für Nebenwege wie Flux (bild.js), bei denen ein
  // 401 oder 403 auch «nicht freigegeben» heissen kann.
  async raw(path, options = {}) {
    if (!this.token) throw new AuthError('Nicht bei ASK angemeldet.');
    return this.session.fetch(this.base + path, {
      ...options,
      headers: { Authorization: `Bearer ${this.token}`, 'Content-Type': 'application/json', ...(options.headers || {}) },
    });
  }

  // Entfernt den alten Terminal-Eintrag aus den ASK-Einstellungen, einmal pro Anmeldung. Ohne die
  // Abmelde-Logik von request(): Schlägt das fehl, bleibt die Anmeldung gültig, und der nächste Start versucht es wieder.
  async cleanStaleTerminal(token) {
    if (this.cleanedFor === token) return;
    this.cleanedFor = token;
    try {
      const headers = { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' };
      const res = await this.session.fetch(`${this.base}/api/v1/users/user/settings`, { headers });
      if (!res.ok) return;
      const servers = (await res.json())?.ui?.terminalServers;
      if (!Array.isArray(servers) || !servers.some(isStaleTerminal)) return;
      await this.session.fetch(`${this.base}/api/v1/users/user/settings/update`, {
        method: 'POST', headers,
        body: JSON.stringify({ ui: { terminalServers: servers.filter((s) => !isStaleTerminal(s)) } }),
      });
    } catch {
      // nicht wichtig genug für eine Meldung
    }
  }

  // Websuche von ASK (Open WebUI /api/v1/retrieval/process/web/search) für das Werkzeug web_search.
  // Ohne die Abmelde-Logik von request(): ein 403 heisst hier «Websuche nicht freigegeben».
  // Ältere Open-WebUI-Versionen erwarten «query», neuere «queries»; beides geht mit.
  async searchWeb(query, signal) {
    if (!this.token) throw new AuthError('Nicht bei ASK angemeldet.');
    const res = await this.session.fetch(`${this.base}/api/v1/retrieval/process/web/search`, {
      method: 'POST', signal,
      headers: { Authorization: `Bearer ${this.token}`, 'Content-Type': 'application/json' },
      body: JSON.stringify({ query, queries: [query] }),
    });
    if (!res.ok) throw new Error(`Websuche von ASK: HTTP ${res.status}`);
    const data = await res.json();
    const items = Array.isArray(data.items) ? data.items : [];
    return items.map((i) => ({ url: i.link || i.url || '', title: String(i.title || '').trim(), snippet: String(i.snippet || i.content || '').trim() }))
      .filter((i) => /^https?:\/\//i.test(i.url));
  }

  async json(path) {
    const res = await this.request(path);
    if (!res.ok) throw new Error(`ASK antwortet mit HTTP ${res.status} (${path})`);
    return res.json();
  }

  async loadModels() {
    const data = await this.json('/api/models');
    this.models = (data.data || data || [])
      .filter((m) => m && m.id && !m.info?.meta?.hidden)
      // vision: true/false, wenn ASK angibt, ob das Modell Bilder versteht; sonst undefined
      // context: Grösse des Kontextfensters in Token, falls ASK oder der Modellserver sie nennt; sonst 0
      .map((m) => ({ id: m.id, name: m.name || m.id, vision: m.info?.meta?.capabilities?.vision, context: contextOf(m) }));
    this.emit('models', this.models);
    return this.models;
  }

  // Abmelden wie auf der Webseite: bei ASK abmelden, das Token aus der Seite und die Cookies von ASK löschen.
  // Ohne das meldet die Anmeldeseite nach einem Login über Microsoft gleich wieder an (Token im Cookie).
  async logout() {
    const token = this.token;
    this.token = '';
    this.rememberToken('');
    if (token && this.session) {
      await this.session.fetch(`${this.base}/api/v1/auths/signout`, { headers: { Authorization: `Bearer ${token}` } }).catch(() => {});
    }
    if (this.view) {
      await this.view.webContents.executeJavaScript('localStorage.removeItem("token")', true).catch(() => {});
      // auch localStorage: Lädt die Seite nicht, kann die App das Token dort nicht selbst entfernen
      await this.session.clearStorageData({ origin: new URL(this.base).origin, storages: ['cookies', 'localstorage'] }).catch(() => {});
      this.view.webContents.loadURL(`${this.base}/auth`).catch(() => {});
    }
    this.publish({ state: 'nicht-angemeldet' });
  }

  // Eine Antwort des Modells, gestreamt. onDelta(kind, text) für die Anzeige.
  // Bricht die Antwort ab (Stopp, Verbindung weg), trägt der Fehler unter «partial» mit, was bis dahin ankam.
  // Was ein Modell abgelehnt hat (Tokenzahlen im Strom, Denkparameter, Stufen des Denkaufwands), merkt sich
  // die App pro Modell: Ein Modell, das «reasoning_effort» nicht kennt, schaltet es für die anderen nicht ab.
  fallbacks(model) {
    this.fallbackByModel ||= new Map();
    if (!this.fallbackByModel.has(model)) this.fallbackByModel.set(model, { noUsage: false, noThinkParams: false, effortFor: null });
    return this.fallbackByModel.get(model);
  }

  // Für Tests: für alle Modelle zurücksetzen oder setzen
  set noUsage(v) { for (const f of this.fallbackByModel?.values() || []) f.noUsage = !!v; }
  set noThinkParams(v) { for (const f of this.fallbackByModel?.values() || []) f.noThinkParams = !!v; }

  async chat({ model, messages, tools, webSearch, thinking, signal: outer, onDelta }) {
    const completion = new Completion(onDelta);
    const fb = this.fallbacks(model);
    // Schweigt ASK mitten in der Antwort zu lange, hängt die Verbindung: abbrechen statt ewig warten
    const stall = new AbortController();
    const signal = outer ? AbortSignal.any([outer, stall.signal]) : stall.signal;
    const send = (withUsage) => {
      const body = { model, messages, stream: true };
      // Websuche macht ASK auf dem Server vor der Antwort und schickt die Quellen mit.
      if (webSearch) body.features = { web_search: true };
      if (!fb.noThinkParams && THINKING_PARAMS[thinking]) {
        Object.assign(body, THINKING_PARAMS[thinking]);
        const effort = fb.effortFor?.[body.reasoning_effort];
        if (effort) body.reasoning_effort = effort;
      }
      // Tokenzahlen am Ende des Stroms; manche Server lehnen das ab, dann ohne.
      if (withUsage) body.stream_options = { include_usage: true };
      if (tools && tools.length) body.tools = tools;
      return this.request('/api/chat/completions', { method: 'POST', body: JSON.stringify(body), signal });
    };
    let res = await send(!fb.noUsage);
    let error = res.ok ? null : await errorOf(res);
    // Lehnt der Server einen Zusatz ab, ohne ihn nochmals versuchen und ihn künftig weglassen.
    for (let retry = 0; error && res.status === 400 && retry < 3; retry++) {
      const thinkParams = !fb.noThinkParams && THINKING_PARAMS[thinking];
      const sent = thinkParams && (fb.effortFor?.[thinkParams.reasoning_effort] || thinkParams.reasoning_effort);
      const other = thinkParams && /reasoning[ _]effort/i.test(error.message) && nearestEffort(error.message, sent);
      if (!fb.noUsage && /stream_options|include_usage/i.test(error.message)) fb.noUsage = true;
      else if (other) fb.effortFor = { ...fb.effortFor, [thinkParams.reasoning_effort]: other }; // erlaubte Stufe statt gar keiner
      else if (thinkParams && /reasoning[ _]effort|chat_template_kwargs|enable_thinking|\bthink\b/i.test(error.message)) fb.noThinkParams = true;
      else break;
      res = await send(!fb.noUsage);
      error = res.ok ? null : await errorOf(res);
    }
    if (error) throw error;
    const type = res.headers.get('content-type') || '';
    if (!type.includes('text/event-stream')) {
      completion.add(await res.json().catch(() => null));
      if (completion.error) throw inbandError(completion);
      return completion.end();
    }
    const parser = new SseParser((chunk) => completion.add(chunk));
    const decoder = new TextDecoder();
    const reader = res.body.getReader();
    let timer = null;
    try {
      for (;;) {
        clearTimeout(timer);
        timer = setTimeout(() => stall.abort(), STALL_MS);
        const { done, value } = await reader.read();
        if (done) break;
        parser.push(decoder.decode(value, { stream: true }));
      }
    } catch (err) {
      const partial = completion.end();
      if (stall.signal.aborted && !outer?.aborted) throw Object.assign(new Error('ASK hat fünf Minuten lang nichts mehr gesendet. Die Verbindung wurde getrennt; schreib «weiter».'), { partial });
      throw Object.assign(err, { partial });
    } finally {
      clearTimeout(timer);
    }
    parser.push('\n');
    if (completion.error) throw inbandError(completion);
    // Strom zu Ende ohne [DONE] und ohne finish_reason: die Verbindung brach ab. Ein halber Werkzeugaufruf
    // (Argumente kein gültiges JSON) darf nicht ausgeführt werden: als Verbindungsfehler neu versuchen.
    if (!parser.done && !completion.finish) {
      const done = completion.end();
      const broken = done.calls.some((c) => { try { JSON.parse(c.arguments || '{}'); return false; } catch { return true; } });
      if (broken) throw Object.assign(new Error('ASK: Die Verbindung brach mitten in einem Werkzeugaufruf ab (network).'), { status: 502, partial: done });
      done.incomplete = true;
      return done;
    }
    return completion.end();
  }
}

module.exports = { inbandError, sameOrigin, cleanToken, isNameError, contextOf, AskSession, AuthError, SseParser, ThinkSplitter, Completion, parseTextToolCalls, parseSources, webSearchAllowed, isStaleTerminal, THINKING_PARAMS, THINKING_LEVELS, nearestEffort };
