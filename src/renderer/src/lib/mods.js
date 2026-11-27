// Mods in der Oberfläche (ENTSCHEIDE E72): lädt Aussehen (CSS) und Code (JavaScript) der eingeschalteten Mods und
// gibt dem Code die Schnittstelle `ASK`. Code eines Mods läuft mit vollem Zugriff auf die Oberfläche; heikle Schritte
// (Rückfragen erlauben, Modus «Auto», Tokens, Skills und Gedächtnis speichern, Mods einschalten) bestätigt die Person
// dann in einem Dialog von Windows, den der Code nicht bedienen kann (app.rs, `secure`).
//
// Jeder Mod bekommt seine eigene Schnittstelle: Beim Ausführen von mod.js zeigt `window.ASK` auf sie. Mods halten
// sie mit (function (ASK) { … })(ASK); fest, damit spätere Aufrufe (Ereignisse) die eigene Ablage treffen.
import { useSyncExternalStore } from 'react';
import { api } from '../api.js';
import { fillTemplate, modCommandFor } from './modCommands.mjs';

export { fillTemplate, modCommandFor };

const BASE = /Windows/i.test(navigator.userAgent) ? 'http://askmod.localhost/mod/' : 'askmod://mod/';
export const EVENTS = ['gesendet', 'antwort', 'fertig', 'fehler', 'frage', 'chat', 'theme'];

const listeners = new Map(); // Ereignis → Set von { mod, fn }
let store = { commands: [], panels: [], buttons: [], loaded: [] };
const subs = new Set();
// Hell oder Dunkel: folgt prefers-color-scheme (die Hülle setzt es aus den Einstellungen). Für Mods auch als
// data-theme="dark|light" am Wurzelelement, damit CSS beide Fälle ansprechen kann.
const darkQuery = typeof window.matchMedia === 'function' ? window.matchMedia('(prefers-color-scheme: dark)') : null;
const isDark = () => !!darkQuery?.matches;
function syncTheme() { document.documentElement.dataset.theme = isDark() ? 'dark' : 'light'; }
syncTheme();
darkQuery?.addEventListener?.('change', () => { syncTheme(); emitMod('theme', { dunkel: isDark() }); });

const host = { composer: null, send: null, chat: () => ({ id: '', titel: '', nachrichten: [] }), dark: isDark };

function update(patch) {
  store = { ...store, ...patch };
  subs.forEach((f) => f());
}

export function useMods() {
  return useSyncExternalStore((f) => { subs.add(f); return () => subs.delete(f); }, () => store);
}

/** App.jsx gibt die Teile der Oberfläche, die Mods benutzen dürfen */
export function setModHost(h) {
  Object.assign(host, h);
}

/** Ereignis an alle Mods; Fehler eines Mods stören die anderen und die App nicht */
export function emitMod(event, data = {}) {
  for (const { mod, fn } of listeners.get(event) || []) {
    try { fn(data); } catch (err) { console.error(`[Mod ${mod}]`, err); }
  }
}

function fileUrl(m, path) {
  const clean = String(path || '').replace(/^\.?\/+/, '');
  return `${BASE}${encodeURIComponent(m.id)}/${m.hash}/${clean.split('/').map(encodeURIComponent).join('/')}`;
}

function storageKey(id, key) {
  return `askmod:${id}:${key}`;
}

function apiFor(m) {
  const on = (event, fn) => {
    if (!EVENTS.includes(event) || typeof fn !== 'function') return () => {};
    const entry = { mod: m.id, fn };
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event).add(entry);
    return () => listeners.get(event)?.delete(entry);
  };
  return Object.freeze({
    mod: Object.freeze({ id: m.id, name: m.name }),
    on,
    eingabe: Object.freeze({
      lesen: () => host.composer?.getText?.() ?? '',
      setzen: (text) => host.composer?.setText(String(text ?? '')),
      einfuegen: (text) => host.composer?.insert(String(text ?? '')),
    }),
    senden: (text) => host.send?.(String(text ?? '')),
    chat: () => host.chat(),
    meldung: (text, { dauer = 4000 } = {}) => toast(String(text ?? ''), dauer),
    befehl: ({ name, beschreibung = '', ausfuehren } = {}) => {
      const cmd = String(name || '').toLowerCase().replace(/[^a-z0-9-]/g, '');
      if (!cmd || typeof ausfuehren !== 'function') return;
      update({ commands: [...store.commands.filter((c) => c.cmd !== `/${cmd}`), { cmd: `/${cmd}`, text: `${beschreibung} (Mod ${m.name})`, mod: m.id, js: ausfuehren }] });
    },
    panel: ({ id, titel, render } = {}) => {
      if (typeof render !== 'function') return;
      const key = `${m.id}:${id || titel || 'panel'}`;
      const el = document.createElement('div');
      el.className = `mod-panel mod-panel-${m.id}`;
      try { render(el); } catch (err) { console.error(`[Mod ${m.id}]`, err); }
      update({ panels: [...store.panels.filter((p) => p.key !== key), { key, titel: String(titel || m.name), el }] });
    },
    knopf: ({ titel, symbol, klick } = {}) => {
      if (typeof klick !== 'function') return;
      const key = `${m.id}:${store.buttons.length}`;
      update({ buttons: [...store.buttons, { key, titel: String(titel || m.name), symbol: String(symbol || '★').slice(0, 4), klick }] });
    },
    datei: (path) => fileUrl(m, path),
    speicher: Object.freeze({
      lesen: (key) => { try { return JSON.parse(localStorage.getItem(storageKey(m.id, key)) ?? 'null'); } catch { return null; } },
      schreiben: (key, value) => { try { localStorage.setItem(storageKey(m.id, key), JSON.stringify(value)); } catch { /* voll oder gesperrt */ } },
    }),
    dunkel: () => host.dark(),
  });
}

// Kleine Meldung unten in der Mitte
function toast(text, ms) {
  let box = document.getElementById('mod-toasts');
  if (!box) {
    box = document.createElement('div');
    box.id = 'mod-toasts';
    box.className = 'mod-toasts';
    box.setAttribute('role', 'status');
    document.body.appendChild(box);
  }
  const el = document.createElement('div');
  el.className = 'mod-toast';
  el.textContent = text.slice(0, 300);
  box.appendChild(el);
  setTimeout(() => el.remove(), Math.max(1000, Math.min(Number(ms) || 4000, 20000)));
}

function loadScript(src) {
  return new Promise((resolve) => {
    const s = document.createElement('script');
    s.src = src;
    s.async = false;
    s.onload = () => resolve(true);
    s.onerror = () => resolve(false);
    document.head.appendChild(s);
  });
}

let started = false;
/** Einmal beim Start: aktive Mods laden. Gibt die Liste zurück (für den Vergleich nach «mods:changed»). */
export async function loadMods() {
  if (started) return store.loaded;
  started = true;
  let list = [];
  try { list = (await api().modsActive?.()) || []; } catch { list = []; }
  const templates = [];
  for (const m of list) {
    if (m.style) {
      const link = document.createElement('link');
      link.rel = 'stylesheet';
      link.href = fileUrl(m, m.style);
      link.dataset.mod = m.id;
      document.head.appendChild(link);
    }
    for (const b of m.befehle || []) templates.push({ cmd: `/${b.name}`, text: `${b.beschreibung || 'Befehl'} (Mod ${m.name})`, mod: m.id, prompt: b.prompt });
  }
  update({ commands: [...templates, ...store.commands], loaded: list.map((m) => `${m.id}:${m.hash}`) });
  // Code nacheinander, damit `ASK` während der Ausführung zum richtigen Mod gehört
  for (const m of list.filter((x) => x.script)) {
    window.ASK = apiFor(m);
    const ok = await loadScript(fileUrl(m, m.script));
    if (!ok) toast(`Mod «${m.name}» liess sich nicht laden.`, 6000);
  }
  try { delete window.ASK; } catch { window.ASK = undefined; }
  return store.loaded;
}

/** Hat sich geändert, welche Mods aktiv sind? Dann muss die Oberfläche neu laden. */
export async function modsChanged() {
  let list = [];
  try { list = (await api().modsActive?.()) || []; } catch { return false; }
  const now = list.map((m) => `${m.id}:${m.hash}`).join('|');
  return now !== store.loaded.join('|');
}
