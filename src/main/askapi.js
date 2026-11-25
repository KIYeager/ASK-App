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
      if (!data || data === '[DONE]') continue;
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
    if (chunk.error) {
      this.error = chunk.error.message || chunk.error.detail || JSON.stringify(chunk.error);
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
    (d.tool_calls || []).forEach((tc, n) => {
      const index = tc.index ?? n;
      const call = this.calls[index] || (this.calls[index] = { id: '', name: '', arguments: '' });
      if (tc.id) call.id = tc.id;
      const name = tc.function?.name;
      if (name && call.name !== name) call.name += name;
      const args = tc.function?.arguments;
      if (args) call.arguments += typeof args === 'string' ? args : JSON.stringify(args);
    });
    if (choice.finish_reason) this.finish = choice.finish_reason;
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

async function errorOf(res) {
  const text = await res.text().catch(() => '');
  let detail = text;
  try { detail = JSON.parse(text).detail || JSON.parse(text).error?.message || text; } catch { /* Text */ }
  if (typeof detail !== 'string') detail = JSON.stringify(detail);
  return Object.assign(new Error(`ASK: ${detail || `HTTP ${res.status}`}`), { status: res.status });
}

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
  }

  attach(view) {
    this.view = view;
    this.session = view.webContents.session;
    const wc = view.webContents;
    wc.on('did-finish-load', () => this.check(true));
    wc.on('did-navigate', (_e, url) => this.navigated(url));
    wc.on('did-navigate-in-page', (_e, url) => this.navigated(url));
    clearInterval(this.timer);
    this.timer = setInterval(() => this.check(), 2000);
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
    if (!this.base || target.origin !== new URL(this.base).origin) return; // fremde Anmeldeseite bleibt sichtbar
    if (this.status.state === 'nicht-angemeldet' && !/^\/auth\b/.test(target.pathname)) this.publish({ state: 'pruefen' });
    this.check(true);
  }

  // Liest das Token aus der ASK-Seite und prüft es mit einer Anfrage an ASK.
  async check(force = false) {
    if (this.busy && force) this.again = true; // nach der laufenden Prüfung gleich nochmals
    if (!this.view || !this.base || this.busy) return this.status;
    const wc = this.view.webContents;
    if (wc.isDestroyed() || wc.isLoading() || !wc.getURL().startsWith(new URL(this.base).origin)) return this.status;
    const interval = RECHECK_MS[this.status.state] ?? 30000;
    if (!force && Date.now() - this.lastCheck < interval) return this.status;
    this.lastCheck = Date.now();
    this.busy = true;
    try {
      const token = await wc.executeJavaScript('localStorage.token || ""', true);
      if (!token) {
        this.token = '';
        if (this.status.state !== 'nicht-angemeldet') this.publish({ state: 'nicht-angemeldet' });
        return this.status;
      }
      if (token === this.token && this.status.state === 'angemeldet' && !force) return this.status;
      this.token = token;
      const user = await this.json('/api/v1/auths/');
      const models = await this.loadModels();
      this.user = { name: user.name || user.email || '', role: user.role };
      this.webSearch = webSearchAllowed(await this.json('/api/config').catch(() => ({})), user);
      this.publish({ state: 'angemeldet', user: this.user.name });
      if (models.length === 0) this.publish({ state: 'fehler', message: 'ASK bietet diesem Konto kein Modell an.' });
      this.cleanStaleTerminal(token);
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

  async request(path, options = {}) {
    if (!this.token) throw new AuthError('Nicht bei ASK angemeldet.');
    const res = await this.session.fetch(this.base + path, {
      ...options,
      headers: { Authorization: `Bearer ${this.token}`, 'Content-Type': 'application/json', ...(options.headers || {}) },
    });
    if (res.status === 401 || res.status === 403) {
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
      .map((m) => ({ id: m.id, name: m.name || m.id, vision: m.info?.meta?.capabilities?.vision }));
    this.emit('models', this.models);
    return this.models;
  }

  async logout() {
    this.token = '';
    if (this.view) {
      await this.view.webContents.executeJavaScript('localStorage.removeItem("token")', true).catch(() => {});
      this.view.webContents.loadURL(`${this.base}/auth`);
    }
    this.publish({ state: 'nicht-angemeldet' });
  }

  // Eine Antwort des Modells, gestreamt. onDelta(kind, text) für die Anzeige.
  async chat({ model, messages, tools, webSearch, thinking, signal, onDelta }) {
    const completion = new Completion(onDelta);
    const send = (withUsage) => {
      const body = { model, messages, stream: true };
      // Websuche macht ASK auf dem Server vor der Antwort und schickt die Quellen mit.
      if (webSearch) body.features = { web_search: true };
      if (!this.noThinkParams && THINKING_PARAMS[thinking]) {
        Object.assign(body, THINKING_PARAMS[thinking]);
        const effort = this.effortFor?.[body.reasoning_effort];
        if (effort) body.reasoning_effort = effort;
      }
      // Tokenzahlen am Ende des Stroms; manche Server lehnen das ab, dann ohne.
      if (withUsage) body.stream_options = { include_usage: true };
      if (tools && tools.length) body.tools = tools;
      return this.request('/api/chat/completions', { method: 'POST', body: JSON.stringify(body), signal });
    };
    let res = await send(!this.noUsage);
    let error = res.ok ? null : await errorOf(res);
    // Lehnt der Server einen Zusatz ab, ohne ihn nochmals versuchen und ihn künftig weglassen.
    for (let retry = 0; error && res.status === 400 && retry < 3; retry++) {
      const thinkParams = !this.noThinkParams && THINKING_PARAMS[thinking];
      const sent = thinkParams && (this.effortFor?.[thinkParams.reasoning_effort] || thinkParams.reasoning_effort);
      const other = thinkParams && /reasoning[ _]effort/i.test(error.message) && nearestEffort(error.message, sent);
      if (!this.noUsage && /stream_options|include_usage/i.test(error.message)) this.noUsage = true;
      else if (other) this.effortFor = { ...this.effortFor, [thinkParams.reasoning_effort]: other }; // erlaubte Stufe statt gar keiner
      else if (thinkParams && /reasoning[ _]effort|chat_template_kwargs|enable_thinking|\bthink\b/i.test(error.message)) this.noThinkParams = true;
      else break;
      res = await send(!this.noUsage);
      error = res.ok ? null : await errorOf(res);
    }
    if (error) throw error;
    const type = res.headers.get('content-type') || '';
    if (!type.includes('text/event-stream')) {
      completion.add(await res.json());
      return completion.end();
    }
    const parser = new SseParser((chunk) => completion.add(chunk));
    const decoder = new TextDecoder();
    const reader = res.body.getReader();
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      parser.push(decoder.decode(value, { stream: true }));
    }
    parser.push('\n');
    if (completion.error) throw new Error(`ASK: ${completion.error}`);
    return completion.end();
  }
}

module.exports = { AskSession, AuthError, SseParser, ThinkSplitter, Completion, parseTextToolCalls, parseSources, webSearchAllowed, isStaleTerminal, THINKING_PARAMS, THINKING_LEVELS, nearestEffort };
