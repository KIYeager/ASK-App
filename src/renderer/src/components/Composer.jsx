import { forwardRef, useEffect, useImperativeHandle, useLayoutEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { extOf, fileBadge, formatDuration, formatSize } from '../lib/format.js';
import { api } from '../api.js';

const THINK_LEVELS = ['standard', 'tief', 'mittel', 'maximal'];
const THINK_NAMES = { standard: 'Standard', tief: 'Tief', mittel: 'Mittel', maximal: 'Maximal' };
const MAX_ATTACH = 10;
const MAX_ATTACH_SIZE = 50 * 1024 * 1024;
const IMAGE_RE = /\.(png|jpe?g|gif|webp)$/i;
const TODO_SHOWN = 7;
const MAX_DRAFTS = 20; // Entwürfe anderer Chats, die das Eingabefeld behält
const MAX_SENT = 50; // frühere Eingaben für ↑
const revoke = (files) => { for (const f of files || []) if (f.url) URL.revokeObjectURL(f.url); };

// Aufgabenliste über der Eingabe wie in Claude Code (aus dem letzten update_tasks)
function Todo({ tasks, open, onToggle }) {
  if (!tasks?.length) return null;
  const done = tasks.filter((t) => t.status === 'erledigt').length;
  const current = tasks.find((t) => t.status === 'in_arbeit');
  let from = 0;
  if (tasks.length > TODO_SHOWN) {
    const at = Math.max(0, tasks.findIndex((t) => t.status !== 'erledigt'));
    from = Math.min(Math.max(0, at - 2), tasks.length - TODO_SHOWN);
  }
  const shown = tasks.slice(from, from + TODO_SHOWN);
  const rest = tasks.length - from - shown.length;
  const mark = { erledigt: '✓', in_arbeit: '▸', offen: '○' };
  return (
    <div id="todo" className={`todo-box${open ? ' open' : ''}${done === tasks.length ? ' done' : ''}`}>
      <button type="button" id="todo-head" className="todo-head" onClick={onToggle} title="Aufgaben ein- oder ausblenden (Ctrl+T)">
        <Icon name={open ? 'chevDown' : 'chevRight'} size={14} />
        <span className="todo-label">Aufgaben</span>
        <span className="todo-bar" aria-hidden="true"><span style={{ width: `${Math.round((done / tasks.length) * 100)}%` }} /></span>
        <span id="todo-count" className="todo-count">{done}/{tasks.length}</span>
        <span id="todo-current" className="todo-current">{done === tasks.length ? '✓ alle erledigt' : current ? current.titel : ''}</span>
      </button>
      {open ? (
        <ul id="todo-list" className="todo-list">
          {from > 0 ? <li className="more">… {from} erledigt</li> : null}
          {shown.map((t, i) => <li key={from + i} className={t.status}><span className="tmark" aria-hidden="true">{mark[t.status] || '○'}</span><span className="ttext">{t.titel}</span></li>)}
          {rest > 0 ? <li className="more">… und {rest} weitere</li> : null}
        </ul>
      ) : null}
    </div>
  );
}

// Was die Stufen bedeuten, in einer Zeile unter dem Regler. ASK kennt nur low, medium und xhigh (ENTSCHEIDE E62).
const THINK_INFO = {
  standard: 'Das Modell wählt selbst: Mittel zum Start, Maximal für Schwieriges, Tief für Einfaches.',
  tief: 'Schnelle Antworten, kaum Überlegen.',
  mittel: 'Ausgewogen, passt für die meisten Aufträge.',
  maximal: 'So gründlich wie möglich, deutlich langsamer.',
};
const THINK_STEPS = THINK_LEVELS.slice(1); // tief bis maximal
const LAST_STEP = THINK_STEPS.length - 1;

// Glühbirne für den Denkaufwand: je höher die Stufe, desto mehr Strahlen (-1: Automatisch, ohne Füllung)
function Bulb({ step = -1, size = 14 }) {
  const all = ['M12 1.6v1.6', 'M5 4.5l1.1 1.1', 'M19 4.5l-1.1 1.1', 'M2.6 10.2h1.6', 'M21.4 10.2h-1.6'];
  const rays = step >= LAST_STEP ? all : step >= 1 ? all.slice(0, 3) : [];
  return (
    <svg className="icon bulb" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"
      strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M9 17h6M10 20.5h4M8.5 14.5a5 5 0 1 1 7 0c-.8.7-1.5 1.5-1.5 2.5h-4c0-1-.7-1.8-1.5-2.5z"
        fill={step >= 0 ? 'currentColor' : 'none'} fillOpacity=".22" />
      {rays.map((d) => <path key={d} d={d} />)}
    </svg>
  );
}

// Pixel-Banner bei «Maximal»: Pixel gehen von links nach rechts an, Lichtwellen laufen zum Knopf,
// aus der Glühbirne steigen leuchtende Pixel. Farben aus --think-1 bis --think-4; ruhig bei reduzierter Bewegung.
function usePixelBanner(canvas, track, active) {
  useEffect(() => {
    const cv = canvas.current;
    const ctx = cv?.getContext?.('2d');
    if (!active || !ctx) return undefined;
    const still = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
    const hash = (a, b) => { const x = Math.sin(a * 127.1 + b * 311.7) * 43758.5453; return x - Math.floor(x); };
    const rgb = (c) => { const n = parseInt(c.trim().slice(1), 16); return [n >> 16 & 255, n >> 8 & 255, n & 255]; };
    const mix = (a, b, t) => a.map((v, i) => Math.round(v + (b[i] - v) * t));
    const css = getComputedStyle(track.current);
    const colors = [1, 2, 3, 4].map((i) => rgb(css.getPropertyValue(`--think-${i}`) || '#0082b4'));
    let raf = 0, t0 = 0, last = 0, parts = [];
    const frame = (t) => {
      if (!t0) t0 = t;
      const w = track.current.clientWidth, h = track.current.clientHeight, dpr = window.devicePixelRatio || 1;
      if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);
      const el = still ? 1e9 : t - t0, step = 6, cols = Math.ceil(w / step), rows = Math.ceil(h / step);
      const reveal = (el / 700) * cols;
      for (let x = 0; x < cols && x <= reveal; x++) {
        const pos = x / cols, band = Math.min(3, Math.floor(pos * 4));
        const fresh = Math.max(0, 1 - (reveal - x) / 6);
        const wave = still ? 0 : Math.max(0, Math.sin(pos * 9 - el / 380)) ** 4;
        for (let y = 0; y < rows; y++) {
          const j = hash(x, y);
          let lift = band * .1 + pos * .12 + j * .14 + wave * .28 + fresh * .5;
          if (!still && Math.sin(el / 230 + j * 60) > .985) lift += .5;
          ctx.fillStyle = `rgb(${mix(colors[band], [255, 255, 255], Math.min(.85, lift)).join(',')})`;
          ctx.fillRect(x * step, y * step, 5, 5);
        }
      }
      if (!still && el > 300 && t - last > 55) {
        last = t;
        parts.push({ x: w - h / 2 + (Math.random() - .5) * 8, y: h / 2 - 4, vx: -(.3 + Math.random() * 1.6), vy: -(.2 + Math.random() * .9), life: 1,
          c: Math.random() < .5 ? [255, 236, 170] : mix(colors[3], [255, 255, 255], .4) });
      }
      parts = parts.filter((p) => (p.life -= .02) > 0);
      for (const p of parts) {
        p.x += p.vx; p.y += p.vy; p.vy += .015;
        ctx.shadowColor = ctx.fillStyle = `rgba(${p.c.join(',')},${p.life})`;
        ctx.shadowBlur = 6;
        ctx.fillRect(Math.round(p.x / 2) * 2, Math.round(p.y / 2) * 2, 3, 3);
      }
      ctx.shadowBlur = 0;
      if (!still) raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);
    return () => { cancelAnimationFrame(raf); ctx.clearRect(0, 0, cv.width, cv.height); };
  }, [active, canvas, track]);
}

// Regler: Der Knopf mit der Glühbirne sitzt in der Spur und schiebt die Füllung vor sich her.
// Klicken gleitet zur Stufe, Ziehen folgt stufenlos und rastet beim Loslassen weich ein.
function ThinkSlider({ auto, pos, onPick }) {
  const track = useRef(null);
  const canvas = useRef(null);
  const [raw, setRaw] = useState(null); // Stelle 0..1 während des Ziehens
  const [dragging, setDragging] = useState(false);
  // Immer die aktuelle Auswahl-Funktion: die vom Drücken vergleicht mit der Stufe von damals, und zurück auf die
  // Ausgangsstufe ziehen schickte dann nichts (Mittel → Tief → Mittel blieb auf Tief)
  const pickRef = useRef(onPick);
  pickRef.current = onPick;
  const at = (x) => {
    const r = track.current.getBoundingClientRect(), inset = r.height / 2;
    return Math.max(0, Math.min(1, (x - r.left - inset) / (r.width - 2 * inset)));
  };
  const down = (e) => {
    if (e.button !== 0) return;
    const x0 = e.clientX;
    let f = at(x0), moved = false;
    e.currentTarget.setPointerCapture?.(e.pointerId);
    pickRef.current(Math.round(f * LAST_STEP));
    const target = e.currentTarget;
    const move = (ev) => {
      if (!moved && Math.abs(ev.clientX - x0) < 4) return;
      moved = true;
      setDragging(true);
      f = at(ev.clientX);
      setRaw(f);
      pickRef.current(Math.round(f * LAST_STEP));
    };
    const up = () => {
      if (moved) pickRef.current(Math.round(f * LAST_STEP)); // dort einrasten, wo losgelassen wurde
      setDragging(false); setRaw(null);
      target.removeEventListener('pointermove', move); target.removeEventListener('pointerup', up); target.removeEventListener('pointercancel', up);
    };
    target.addEventListener('pointermove', move); target.addEventListener('pointerup', up); target.addEventListener('pointercancel', up);
  };
  const key = (e) => {
    const k = { ArrowRight: 1, ArrowUp: 1, ArrowLeft: -1, ArrowDown: -1 }[e.key];
    if (k) onPick(auto ? pos : pos + k);
    else if (e.key === 'Home') onPick(0);
    else if (e.key === 'End') onPick(LAST_STEP);
    else if (e.key === 'Enter' || e.key === ' ') onPick(pos);
    else return;
    e.preventDefault();
  };
  const step = auto ? -1 : pos;
  const max = step === LAST_STEP;
  usePixelBanner(canvas, track, max && !dragging);
  const f = raw ?? (auto ? 0 : pos / LAST_STEP);
  return (
    <div className={`think-slider${dragging ? ' drag' : ''}${max ? ' max' : ''}`} data-step={step}>
      <div id="think-range" ref={track} className="think-track" role="slider" tabIndex={0} aria-label="Denkaufwand"
        aria-valuemin={0} aria-valuemax={LAST_STEP} aria-valuenow={pos} aria-valuetext={auto ? 'Automatisch' : THINK_NAMES[THINK_STEPS[pos]]}
        style={{ '--f': f }} onPointerDown={down} onKeyDown={key}>
        <div className="think-fill" />
        <canvas ref={canvas} className="think-pixels" aria-hidden="true" />
        <div className="think-knob"><Bulb step={step} size={16} /></div>
      </div>
      <div className="think-ticks">
        {THINK_STEPS.map((l, i) => (
          <button key={l} type="button" tabIndex={-1} className={!auto && i === pos ? 'sel' : ''}
            style={{ '--i': i / LAST_STEP }} onClick={() => { onPick(i); track.current?.focus(); }}>{THINK_NAMES[l]}</button>
        ))}
      </div>
    </div>
  );
}

function ThinkPicker({ level, autoLevel = '', onChange }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  useEffect(() => {
    if (!open) return undefined;
    const close = (e) => { if (!ref.current?.contains(e.target)) setOpen(false); };
    const esc = (e) => { if (e.key === 'Escape') { e.stopPropagation(); setOpen(false); } };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', esc, true);
    return () => { document.removeEventListener('mousedown', close); document.removeEventListener('keydown', esc, true); };
  }, [open]);
  useEffect(() => { if (open) document.getElementById('think-range')?.focus(); }, [open]);
  const auto = !THINK_STEPS.includes(level);
  const idx = THINK_STEPS.indexOf(level);
  const pos = idx < 0 ? 1 : idx; // Automatisch: Regler bei «Mittel», aber grau
  const autoStep = autoLevel ? THINK_STEPS.indexOf(autoLevel) : -1; // während eines Auftrags: Wahl des Modells
  // Mit der zuletzt gesendeten Stufe vergleichen: Beim schnellen Ziehen ist der Zustand der App noch nicht zurück
  const sent = useRef(level);
  useEffect(() => { sent.current = level; }, [level]);
  const pick = (i) => {
    const next = THINK_STEPS[Math.min(LAST_STEP, Math.max(0, i))];
    if (next !== sent.current) { sent.current = next; onChange(next); }
  };
  return (
    <div className="think-wrap" ref={ref}>
      <button type="button" id="think-btn" className={`chip think-chip${!auto ? ' on' : ''}`} data-step={auto ? -1 : idx}
        onClick={() => setOpen(!open)} title="Denkaufwand des Modells (/denken)" aria-expanded={open}>
        <Bulb step={auto ? autoStep : idx} /> <span id="think-label" className="chip-label">{auto ? (autoLevel ? `Auto · ${THINK_NAMES[autoLevel]}` : 'Auto') : THINK_NAMES[level]}</span>
      </button>
      {open ? (
        <div id="think-pop" className={`popover think-pop${auto ? ' auto' : ''}`} role="dialog" aria-label="Denkaufwand">
          <div className="think-head">
            <span>Denkaufwand</span>
            <button id="think-standard" type="button" role="switch" aria-checked={auto} className={`think-switch${auto ? ' on' : ''}`}
              onClick={() => onChange(auto ? 'mittel' : 'standard')} title="Das Modell entscheidet selbst">
              <span className="think-switch-text">Automatisch</span><span className="think-switch-track" aria-hidden="true"><span /></span>
            </button>
          </div>
          <ThinkSlider auto={auto} pos={pos} onPick={pick} />
          <p id="think-info" className="think-info">{THINK_INFO[auto ? 'standard' : level]}</p>
        </div>
      ) : null}
    </div>
  );
}

// Modus im Eingabefeld (ENTSCHEIDE E52); Shift+Tab wechselt der Reihe nach
export const MODES = [
  { id: 'auto', name: 'Auto', icon: 'bolt', text: 'ASK prüft jeden Befehl und fragt nur bei Gefährlichem.' },
  { id: 'nachfragen', name: 'Nachfragen', icon: 'hand', text: 'Fragt vor jedem Befehl und jeder Änderung.' },
  { id: 'plan', name: 'Plan', icon: 'map', text: 'Schaut sich nur um und legt einen Plan vor.' },
];

function ModePicker({ mode, onChange }) {
  const [open, setOpen] = useState(false);
  const ref = useRef(null);
  useEffect(() => {
    if (!open) return undefined;
    const close = (e) => { if (!ref.current?.contains(e.target)) setOpen(false); };
    const esc = (e) => { if (e.key === 'Escape') { e.stopPropagation(); setOpen(false); } };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', esc, true);
    return () => { document.removeEventListener('mousedown', close); document.removeEventListener('keydown', esc, true); };
  }, [open]);
  const cur = MODES.find((m) => m.id === mode) || MODES[0];
  return (
    <div className="mode-wrap" ref={ref}>
      <button type="button" id="mode-btn" className={`chip mode-chip mode-${cur.id}`} onClick={() => setOpen(!open)}
        title="Modus (Shift+Tab wechselt)" aria-expanded={open} aria-haspopup="listbox">
        <Icon name={cur.icon} size={14} /> <span id="mode-label" className="chip-label">{cur.name}</span>
      </button>
      {open ? (
        <div id="mode-pop" className="popover mode-pop" role="listbox" aria-label="Modus">
          {MODES.map((m) => (
            <button key={m.id} type="button" role="option" aria-selected={m.id === cur.id} className={`mode-opt${m.id === cur.id ? ' on' : ''}`}
              onClick={() => { onChange(m.id); setOpen(false); }}>
              <Icon name={m.icon} size={15} />
              <span className="mode-txt"><span className="mode-name">{m.name}</span><span className="muted">{m.text}</span></span>
              {m.id === cur.id ? <Icon name="check" size={14} className="mode-check" /> : null}
            </button>
          ))}
          <div className="mode-foot muted">Shift+Tab wechselt</div>
        </div>
      ) : null}
    </div>
  );
}

// @Datei: das Wort vor der Schreibstelle, wenn es mit @ beginnt
function mentionAt(text, caret) {
  const m = /(^|\s)@([^\s@"]*)$/.exec(text.slice(0, caret));
  return m ? { start: caret - m[2].length - 1, query: m[2] } : null;
}

const Composer = forwardRef(function Composer(props, ref) {
  const { state, chatId, free, running, block, fileBlock, goal, tasks, tasksLive, activity, runStarted, commands, pendingQuestion, pendingApproval,
    onSend, onStop, onAnswer, onGoalClear, setModel, setThinking, setWeb, notice, context, todoCollapsed, mode, setMode, autoEffort,
    sources, setStrictSources } = props;
  const [text, setText] = useState('');
  const [pending, setPending] = useState([]); // neue Anhänge { name, type, size, data, url }
  const [kept, setKept] = useState([]); // schon gespeicherte Anhänge einer zurückgenommenen Eingabe { path, name, image }
  const [queued, setQueued] = useState(null); // Enter während eines Auftrags: eine wartende Nachricht { chat, text, files, kept } (ENTSCHEIDE E34)
  const [slashIndex, setSlashIndex] = useState(0);
  const [mention, setMention] = useState(null); // { start, query, items, index }
  const [dropping, setDropping] = useState(false);
  const [todoOpen, setTodoOpen] = useState(true);
  const [todoPeek, setTodoPeek] = useState(null); // null: wie in den Einstellungen
  const [, tick] = useState(0);
  const input = useRef(null);
  const fileInput = useRef(null);
  const sent = useRef({ list: [], index: -1 });
  // Jeder Chat behält seinen Entwurf (Text, Anhänge, wartende Nachricht): Wer während eines Auftrags einen neuen
  // Chat beginnt oder den Chat wechselt, findet beim Zurückkommen vor, was dort stand, und nichts geht in den falschen Chat.
  const drafts = useRef(new Map()); // Chat → { text, pending, kept }
  const shownChat = useRef(chatId);
  const live = useRef({});
  live.current = { text, pending, kept, queued };
  const stash = (chat, d) => {
    if (!chat) return;
    const old = drafts.current.get(chat);
    drafts.current.delete(chat);
    const files = old ? [...d.pending, ...old.pending] : d.pending;
    revoke(files.slice(MAX_ATTACH)); // was nicht mehr Platz hat, auch freigeben
    const merged = { text: old ? [d.text, old.text].filter(Boolean).join('\n') : d.text, pending: files.slice(0, MAX_ATTACH), kept: old ? [...d.kept, ...old.kept] : d.kept };
    if (!merged.text && !merged.pending.length && !merged.kept.length) return;
    drafts.current.set(chat, merged);
    while (drafts.current.size > MAX_DRAFTS) { // ältesten Entwurf verwerfen, Vorschaubilder freigeben
      const [first, gone] = drafts.current.entries().next().value;
      revoke(gone.pending);
      drafts.current.delete(first);
    }
  };
  useLayoutEffect(() => {
    const prev = shownChat.current;
    if (prev === chatId) return;
    shownChat.current = chatId;
    // Erster Chat nach dem Start (noch keiner gezeigt): schon Getipptes und Angehängtes bleibt stehen
    if (prev == null && !drafts.current.has(chatId)) return;
    const cur = live.current;
    const q = cur.queued && cur.queued.chat === prev ? cur.queued : null;
    stash(prev, { text: [q?.text, cur.text].filter(Boolean).join('\n'), pending: [...(q?.files || []), ...cur.pending], kept: [...(q?.kept || []), ...cur.kept] });
    const next = drafts.current.get(chatId) || { text: '', pending: [], kept: [] };
    drafts.current.delete(chatId);
    setText(next.text); setPending(next.pending); setKept(next.kept); setQueued(null); setMention(null); setSlashIndex(0);
    sent.current.index = -1;
  }, [chatId]);
  useEffect(() => () => { revoke(live.current.pending); for (const d of drafts.current.values()) revoke(d.pending); }, []);
  // Etwas zurück ins Eingabefeld des Chats, zu dem es gehört (sichtbar: ins Feld, sonst zu dessen Entwurf)
  const giveBack = (chat, { text: t = '', files = [], kept: k = [] }) => {
    if (chat !== shownChat.current) { stash(chat, { text: t, pending: files, kept: k }); return; }
    if (t) setText((cur) => (cur ? `${t}\n${cur}` : t));
    if (files.length) setPending((p) => { const all = [...files, ...p]; revoke(all.slice(MAX_ATTACH)); return all.slice(0, MAX_ATTACH); });
    if (k.length) setKept((p) => [...k, ...p]);
  };

  useImperativeHandle(ref, () => ({
    focus: () => input.current?.focus(),
    setText: (t) => { setText(t); setTimeout(() => { input.current?.focus(); autosize(); }, 0); },
    getText: () => input.current?.value ?? '',
    addFiles: (list) => addFiles(list),
    toggleTodo: () => toggleTodo(),
    // Text und Anhänge zurück ins Eingabefeld (zurückgenommene Eingabe, Senden fehlgeschlagen)
    restore: (d = {}) => {
      giveBack(shownChat.current, d);
      setTimeout(() => input.current?.focus(), 0);
    },
    // Datei im Chat erwähnen: Pfad an der Schreibstelle einfügen
    insert: (t) => {
      const el = input.current;
      const at = el ? el.selectionStart ?? text.length : text.length;
      const next = text.slice(0, at) + (at && !/\s$/.test(text.slice(0, at)) ? ' ' : '') + t + ' ' + text.slice(at);
      setText(next);
      setTimeout(() => { el?.focus(); }, 0);
    },
    stop: () => stopRun(),
  }));

  // Wartende Nachricht geht nach dem Auftrag von selbst hinaus (ausser es ist gerade nichts möglich)
  useEffect(() => {
    // Nur im Chat, in dem sie geschrieben wurde; nach einem Wechsel liegt sie schon bei dessen Entwurf (oben)
    if (running || !queued || queued.chat !== chatId) return;
    const q = queued;
    setQueued(null);
    if (block) { giveBack(q.chat, q); return; }
    sendNow(q.text, q.files, q.kept);
  }, [running, chatId]);

  function restoreQueued() {
    if (queued) giveBack(queued.chat, queued);
    setQueued(null);
  }
  function stopRun() {
    restoreQueued();
    onStop();
  }

  useEffect(() => {
    if (!running) return undefined;
    const t = setInterval(() => tick((n) => n + 1), 500);
    return () => clearInterval(t);
  }, [running]);
  useEffect(() => { if (!running) setTodoPeek(null); }, [running]);
  useEffect(autosize, [text]);
  const caret = useRef(null);
  useLayoutEffect(() => {
    if (caret.current === null || !input.current) return;
    input.current.setSelectionRange(caret.current, caret.current);
    caret.current = null;
  }, [text]);

  // Vorschläge zu @Datei: Dateien im Projekt, die zum getippten Wort passen
  const mentionQuery = mention ? mention.query : null;
  useEffect(() => {
    if (mentionQuery === null) return undefined;
    let alive = true;
    const t = setTimeout(async () => {
      const found = mentionQuery ? await api().findFiles(mentionQuery) : [];
      if (alive) setMention((m) => (m && m.query === mentionQuery ? { ...m, items: (found || []).slice(0, 8), index: 0 } : m));
    }, mentionQuery ? 120 : 0);
    return () => { alive = false; clearTimeout(t); };
  }, [mentionQuery]);

  function updateMention(value, caret) {
    const at = free ? null : mentionAt(value, caret); // ohne Projekt gibt es keine Dateien zum Erwähnen
    setMention((m) => (at ? { ...at, items: m && m.query === at.query ? m.items : (m?.items || []), index: 0 } : null));
  }

  function pickMention(item) {
    if (!mention || !item) return;
    const rel = item.path.replaceAll('\\', '/');
    const token = /\s/.test(rel) ? `@"${rel}"` : `@${rel}`;
    const end = mention.start + 1 + mention.query.length;
    const next = `${text.slice(0, mention.start)}${token} ${text.slice(end)}`;
    setText(next);
    setMention(null);
    caret.current = mention.start + token.length + 1; // nach dem Rendern setzen (useLayoutEffect), sonst gehen schnelle Tasten verloren
  }

  function cycleMode() {
    const i = MODES.findIndex((m) => m.id === mode);
    setMode(MODES[(i + 1) % MODES.length].id);
  }

  function autosize() {
    const el = input.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 260)}px`;
  }

  function toggleTodo() {
    if (!tasks?.length) return;
    if (running && tasksLive) setTodoOpen((o) => !o);
    else setTodoPeek((o) => !(o ?? !todoCollapsed));
  }

  async function addFiles(list) {
    const next = [];
    for (const file of [...list]) {
      if (pending.length + kept.length + next.length >= MAX_ATTACH) { notice(`Höchstens ${MAX_ATTACH} Anhänge auf einmal.`); break; }
      if (file.size > MAX_ATTACH_SIZE) { notice(`${file.name} ist grösser als 50 MB und wurde nicht angehängt.`); continue; }
      let data;
      try { data = new Uint8Array(await file.arrayBuffer()); } catch { notice(`${file.name || 'Die Datei'} lässt sich nicht lesen (ein Ordner?).`); continue; }
      const image = file.type.startsWith('image/') || IMAGE_RE.test(file.name);
      next.push({ name: file.name, type: file.type, size: file.size, data, url: image ? URL.createObjectURL(file) : '' });
    }
    if (next.length) setPending((p) => [...p, ...next]);
    input.current?.focus();
  }

  const slashMatches = !text.startsWith('/') || text.includes(' ') || text.includes('\n') ? [] : commands.filter((c) => c.cmd.startsWith(text.toLowerCase()));

  function runSlash(c) {
    setText('');
    c.run();
  }

  async function submit() {
    // Nebenfrage (E86): geht neben dem Auftrag an ASK, auch während einer offenen Frage oder Rückfrage
    const btwCmd = /^\/btw(?:\s+([\s\S]*))?$/i.exec(text.trim());
    if (btwCmd) {
      const q = (btwCmd[1] || '').trim();
      if (!q) { notice('Schreib die Frage hinter /btw, zum Beispiel «/btw Was heisst Abstract?».'); return; }
      const typed = text;
      setText('');
      // Abgelehnt (z. B. noch nichts im Chat): die Frage zurück ins Eingabefeld, nichts geht verloren
      Promise.resolve(props.onBtw?.(q)).then((ok) => { if (ok === false) setText((now) => now || typed); });
      return;
    }
    if (pendingQuestion) {
      if (!text.trim()) return;
      onAnswer(pendingQuestion.id, text.trim());
      setText('');
      return;
    }
    const t = text.trim();
    if (!t && !pending.length && !kept.length) return; // leere Eingabe tut nichts, auch während eines Auftrags
    const goalCmd = /^\/goal(?:\s+([\s\S]*))?$/i.exec(t);
    if (goalCmd) {
      const arg = (goalCmd[1] || '').trim();
      if (running && arg && !/^(aus|stopp|stop|löschen|off|clear)$/i.test(arg)) { notice('Ein neues Ziel lässt sich erst setzen, wenn der Auftrag fertig ist.'); return; }
      setText('');
      props.onGoal(arg);
      return;
    }
    if (slashMatches.length) { runSlash(slashMatches[slashIndex] || slashMatches[0]); return; }
    // Chat ohne Projekt: Dateien (ausser Bildern) liest ASK über das Terminal; läuft es noch nicht, scheiterte das Lesen ohne Erklärung
    if (fileBlock && [...pending, ...kept].some((f) => !(f.url || f.image))) { notice(fileBlock); return; }
    if (running) {
      // Enter während eines Auftrags reiht ein, statt zu stoppen (stoppen: Esc oder ■)
      if (queued) { notice('Es wartet schon eine Nachricht. Mit × entfernen oder den Auftrag abwarten.'); return; }
      setQueued({ chat: chatId, text: t, files: pending, kept });
      setText(''); setPending([]); setKept([]);
      return;
    }
    if (block) return;
    const files = pending;
    const keptFiles = kept;
    setText(''); setPending([]); setKept([]);
    await sendNow(t, files, keptFiles);
  }

  async function sendNow(t, files = [], keptFiles = []) {
    const chat = chatId;
    if (t) { sent.current.list = [...sent.current.list, t].slice(-MAX_SENT); sent.current.index = -1; }
    const res = await onSend(t, files, keptFiles);
    // Nicht gesendet: zurück zum Chat, aus dem es kam (auch wenn inzwischen ein anderer sichtbar ist)
    if (res && res.ok === false) giveBack(chat, { text: t, files, kept: keptFiles });
    else revoke(files);
  }

  function onKey(e) {
    if (e.key === 'Tab' && e.shiftKey && !free) { e.preventDefault(); cycleMode(); return; }
    if (mention?.items?.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        const n = mention.items.length;
        setMention({ ...mention, index: (mention.index + (e.key === 'ArrowDown' ? 1 : -1) + n) % n });
        e.preventDefault();
        return;
      }
      if ((e.key === 'Enter' || e.key === 'Tab') && !e.shiftKey) { e.preventDefault(); pickMention(mention.items[mention.index]); return; }
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); setMention(null); return; }
    }
    if (slashMatches.length && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      setSlashIndex((i) => (i + (e.key === 'ArrowDown' ? 1 : -1) + slashMatches.length) % slashMatches.length);
      e.preventDefault();
      return;
    }
    if (slashMatches.length && e.key === 'Tab') {
      setText(slashMatches[Math.min(slashIndex, slashMatches.length - 1)].cmd);
      e.preventDefault();
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
      return;
    }
    const s = sent.current;
    if (e.key === 'ArrowUp' && !text && s.list.length) {
      s.index = s.index < 0 ? s.list.length - 1 : Math.max(s.index - 1, 0);
      setText(s.list[s.index]);
      e.preventDefault();
    }
  }

  function onPaste(e) {
    const files = [...(e.clipboardData?.files || [])];
    // Steht auch Text in der Zwischenablage (z. B. aus Excel kopierte Zellen), gilt der Text
    if (!files.length || e.clipboardData.types.includes('text/plain')) return;
    e.preventDefault();
    addFiles(files);
  }

  // Hineinziehen irgendwo ins Fenster
  useEffect(() => {
    const hasFiles = (e) => [...(e.dataTransfer?.types || [])].includes('Files');
    const over = (e) => { if (hasFiles(e)) { e.preventDefault(); setDropping(true); } };
    const leave = (e) => { if (!e.relatedTarget) setDropping(false); };
    const drop = (e) => {
      setDropping(false);
      if (!hasFiles(e)) return;
      e.preventDefault();
      if (document.querySelector('.overlay')) return;
      addFiles(e.dataTransfer.files);
    };
    document.addEventListener('dragover', over);
    document.addEventListener('dragleave', leave);
    document.addEventListener('drop', drop);
    return () => { document.removeEventListener('dragover', over); document.removeEventListener('dragleave', leave); document.removeEventListener('drop', drop); };
  });

  const showTodoOpen = running && tasksLive ? todoOpen : (todoPeek ?? !todoCollapsed);
  const answering = !!pendingQuestion;
  const placeholder = dropping ? 'Loslassen, um die Dateien anzuhängen'
    : answering ? 'Antworte auf die Frage von ASK …'
    : pendingApproval ? 'ASK wartet oben auf deine Entscheidung …'
    : running ? 'Folgenachricht (wird nach dem Auftrag gesendet) · Esc stoppt'
    : (block || (free ? 'Frag ASK etwas …' : mode === 'plan' ? 'Was soll ASK planen? …' : 'Auftrag beschreiben · @ für Dateien'));
  const sendDisabled = answering ? !text.trim() : (!running && !!block);
  const ctxKnown = context?.limit > 0;
  const ctxPct = ctxKnown ? Math.min(100, Math.round((context.tokens / context.limit) * 100)) : 0;
  const short = (n) => (n >= 1000 ? `${Math.round(n / 1000)}k` : String(Math.round(n)));
  const elapsed = runStarted ? formatDuration(Date.now() - runStarted) : '0 s';

  return (
    <div className="composer">
      <div className="column">
        {goal ? (
          <div id="goal-bar" className={`goal-bar${goal.done ? ' done' : ''}`}>
            <Icon name="target" size={14} /><span className="muted">Ziel:</span>
            <span id="goal-text" className="goal-text" title={goal.text}>{goal.text}</span>
            <span id="goal-state" className="goal-state" title={goal.summary || ''}>{goal.done ? '✓ erreicht' : running ? 'in Arbeit' : 'offen'}</span>
            <button id="goal-clear" type="button" className="icon-btn" title="Ziel aufheben (/goal aus)" aria-label="Ziel aufheben" onClick={onGoalClear}><Icon name="close" size={14} /></button>
          </div>
        ) : null}
        {running ? (
          <div id="activity" className="activity">
            <span className="spin" aria-hidden="true" />
            <span id="activity-text">{activity}</span>
            <span className="muted" title="Mit /btw stellst du eine Nebenfrage, ohne den Auftrag zu unterbrechen.">(<span id="activity-time">{elapsed}</span> · Esc zum Stoppen · /btw Nebenfrage)</span>
          </div>
        ) : null}
        <Todo tasks={tasks} open={showTodoOpen} onToggle={toggleTodo} />
        {slashMatches.length ? (
          <div id="slash" className="slash popover" role="listbox">
            {slashMatches.map((c, i) => (
              <div key={c.cmd} role="option" aria-selected={i === slashIndex} className={i === Math.min(slashIndex, slashMatches.length - 1) ? 'active' : ''}
                onMouseDown={(e) => { e.preventDefault(); runSlash(c); }}>
                <span className="cmd">{c.cmd}</span><span className="muted">{c.text}</span>
              </div>
            ))}
          </div>
        ) : null}
        {mention?.items?.length ? (
          <div id="mention" className="slash popover mention-pop" role="listbox" aria-label="Dateien">
            {mention.items.map((f, i) => (
              <div key={f.path} role="option" aria-selected={i === mention.index} className={i === mention.index ? 'active' : ''}
                onMouseDown={(e) => { e.preventDefault(); pickMention(f); }}>
                <Icon name={f.dir ? 'folder' : 'file'} size={14} />
                <span className="cmd">{f.name}</span>
                {f.template ? <span className="mention-tag" title="Vorlage der TF Bern aus C:\Vorlagen (nur lesen)">Vorlage</span> : null}
                <span className="muted path">{f.path.replaceAll('\\', '/').split('/').slice(0, -1).join('/')}</span>
              </div>
            ))}
          </div>
        ) : null}
        <div id="input-box" className={`input-box${block && !running && !answering ? ' disabled' : ''}${dropping ? ' dropping' : ''}${answering ? ' answering' : ''}`}>
          {queued ? (
            <div id="queue-row" className="attach-row">
              <span className="attach-chip queue-chip" title={queued.text}>
                <span className="name">Wartet: {queued.text || [...queued.files, ...queued.kept].map((f) => f.name || 'Bild').join(', ')}</span>
                <button type="button" className="remove" aria-label="Wartende Nachricht entfernen" title="Entfernen (Text zurück ins Eingabefeld)" onClick={restoreQueued}>
                  <Icon name="close" size={12} />
                </button>
              </span>
            </div>
          ) : null}
          {pending.length || kept.length ? (
            <div id="attach-row" className="attach-row">
              {kept.map((f, i) => (
                <span key={`k-${f.path}`} className="attach-chip" title={f.path}>
                  <span className={`badge ext-${extOf(f.name)}`}>{fileBadge(f.name)}</span>
                  <span className="name">{f.name}</span>
                  <button type="button" className="remove" aria-label={`${f.name} entfernen`} title="Entfernen"
                    onClick={() => setKept(kept.filter((_x, j) => j !== i))}><Icon name="close" size={12} /></button>
                </span>
              ))}
              {pending.map((f, i) => (
                <span key={`${f.name}-${i}`} className="attach-chip" title={`${f.name} (${formatSize(f.size)})`}>
                  {f.url ? <img src={f.url} alt="" /> : <span className={`badge ext-${extOf(f.name)}`}>{fileBadge(f.name)}</span>}
                  <span className="name">{f.name || 'Bild'}</span>
                  <button type="button" className="remove" aria-label={`${f.name} entfernen`} title="Entfernen"
                    onClick={() => { if (f.url) URL.revokeObjectURL(f.url); setPending(pending.filter((_x, j) => j !== i)); }}>
                    <Icon name="close" size={12} />
                  </button>
                </span>
              ))}
            </div>
          ) : null}
          <textarea id="input" ref={input} rows={1} spellCheck value={text} placeholder={placeholder} title={free ? 'Enter senden · Shift+Enter neue Zeile · / Befehle' : 'Enter senden · Shift+Enter neue Zeile · / Befehle · @ Datei · Shift+Tab Modus'}
            onChange={(e) => { setText(e.target.value); setSlashIndex(0); updateMention(e.target.value, e.target.selectionStart ?? e.target.value.length); }}
            onBlur={() => setMention(null)} onKeyDown={onKey} onPaste={onPaste} aria-label="Auftrag an ASK" />
          <div className="input-tools">
            <button type="button" id="attach-btn" className="chip icon-only" title="Dateien oder Bilder anhängen (auch mit Ctrl+V oder Hineinziehen)"
              aria-label="Anhängen" onClick={() => fileInput.current?.click()}><Icon name="clip" size={15} /></button>
            <input ref={fileInput} id="attach-file" type="file" multiple hidden onChange={(e) => { addFiles(e.target.files); e.target.value = ''; }} />
            {/* Quellen (E84): nur in Projekten mit Unterlagen; an heisst «Nur aus Quellen», dann ohne Web */}
            {!free && sources?.count > 0 ? (
              <button type="button" id="sources-btn" className={`chip${sources.strict ? ' on' : ''}`} aria-pressed={!!sources.strict}
                onClick={() => setStrictSources?.(!sources.strict)}
                title={sources.strict ? `Nur aus Quellen an: ASK antwortet nur aus den ${sources.count === 1 ? 'Unterlage' : `${sources.count} Unterlagen`} im Ordner Quellen, ohne Web (/quellen)`
                  : `Nur aus Quellen aus: ASK benutzt die ${sources.count === 1 ? 'Unterlage' : `${sources.count} Unterlagen`} im Ordner Quellen zuerst, darf aber auch eigenes Wissen und das Web nutzen (/quellen)`}>
                <Icon name="book" size={14} /> <span className="chip-label">Quellen</span>
              </button>
            ) : null}
            <button type="button" id="web-btn" className={`chip${state.webSearch && state.webSearchAvailable && !(sources?.strict && sources?.count > 0) ? ' on' : ''}`}
              disabled={!state.webSearchAvailable || (!free && sources?.strict && sources?.count > 0)}
              onClick={() => setWeb(!state.webSearch)}
              title={!state.webSearchAvailable ? 'Websuche ist in ASK für dein Konto nicht freigegeben' : (!free && sources?.strict && sources?.count > 0) ? 'Websuche aus, weil «Nur aus Quellen» an ist'
                : state.webSearch ? 'Websuche an: ASK sucht vor der Antwort im Web (/web)' : 'Websuche aus (/web)'}>
              <Icon name="globe" size={14} /> <span className="chip-label">Web</span>
            </button>
            <ThinkPicker level={state.thinking || 'standard'} autoLevel={autoEffort} onChange={setThinking} />
            {/* Ohne Projekt ändert ASK nichts: kein Modus zu wählen */}
            {free ? null : <ModePicker mode={mode} onChange={setMode} />}
            <span className="spacer" />
            {context?.tokens > 0 ? (
              <span id="ctx" className={`ctx${ctxKnown && ctxPct >= 80 ? ' warn' : ''}`}
                title={ctxKnown && ctxPct >= 80 ? `Der Kontext ist fast voll (${short(context.tokens)} von ${short(context.limit)} Token). ASK fasst ältere Aufträge bald von selbst zusammen; /zusammenfassen tut es sofort, /neu beginnt einen neuen Chat.`
                  : `Belegung des Kontextfensters${context.estimated ? ' (geschätzt)' : ''}${ctxKnown ? `: ${short(context.tokens)} von ${short(context.limit)} Token` : ''}`}>
                {ctxKnown ? <span className="ctx-ring" style={{ '--p': `${ctxPct}%` }} aria-hidden="true" /> : null}
                <span id="ctx-text">{ctxKnown ? `${ctxPct} %` : `${context.estimated ? '≈ ' : ''}${short(context.tokens)}`}</span>
              </span>
            ) : null}
            <label className="model-pick" title="Modell in ASK">
              <select id="model-select" value={state.model || ''} disabled={!state.models?.length} onChange={(e) => setModel(e.target.value)}>
                {(state.models || []).map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
              </select>
              <Icon name="chevDown" size={13} />
            </label>
            <button type="button" id="send-btn" className={`send${running && !answering ? ' stop' : ''}`} disabled={sendDisabled}
              onClick={() => (running && !answering ? stopRun() : submit())} title={answering ? 'Antwort senden (Enter)' : running ? 'Stoppen (Esc)' : 'Senden (Enter)'}
              aria-label={answering ? 'Antwort senden' : running ? 'Stoppen' : 'Senden'}>
              <Icon name={running && !answering ? 'stop' : 'send'} size={16} strokeWidth={2.2} />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
});

export default Composer;
export { THINK_LEVELS };
