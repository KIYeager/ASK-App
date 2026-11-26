import { useCallback, useEffect, useLayoutEffect, useMemo, useReducer, useRef, useState } from 'react';
import { api } from './api.js';
import Icon from './components/Icon.jsx';
import Sidebar from './components/Sidebar.jsx';
import Tabs from './components/Tabs.jsx';
import AppAsk from './components/AppAsk.jsx';
import PopMenu from './components/PopMenu.jsx';
import Composer from './components/Composer.jsx';
import RightPanel, { norm } from './components/RightPanel.jsx';
import Dialogs, { activeDialog } from './components/Dialogs.jsx';
import { useSkillCommands } from './components/Knowledge.jsx';
import { Turn } from './components/Chat.jsx';
import { applyEvent, buildTurns, emptyChat, fromTranscript, lastTasks } from './lib/chatModel.mjs';
import { baseName } from './lib/format.js';

// Einträge eines Menüs aus dem Hauptprozess (nur Texte und Schlüssel) für PopMenu; ein Klick meldet den Schlüssel zurück
function menuItems(list, pick) {
  return (list || []).map((it) => (it.separator ? { separator: true } : {
    label: it.label, type: it.type, checked: it.checked, enabled: it.enabled, swatch: it.swatch, hint: it.hint,
    submenu: it.submenu ? menuItems(it.submenu, pick) : undefined, run: () => pick(it.key),
  }));
}

// Karte unten links während eines Updates: Download, Countdown, «Jetzt neu starten», «Später» (ENTSCHEIDE E40).
// Schwebt über der Projektleiste, damit sie weder Tabs noch Chat verschiebt und nie unter Browser oder ASK-Seite liegt.
function UpdateBar({ u }) {
  const st = u?.state || '';
  if (!['lädt', 'neustart', 'geladen', 'installiert'].includes(st)) return null;
  const name = `ASK ${u.version}${/-/.test(u.version) ? ' (Beta)' : ''}`;
  let title = `${name} ist bereit`;
  let text = '';
  let fill = null;
  let now = true;
  let later = false;
  if (st === 'lädt') { title = `Update auf ${name}`; text = `Wird geladen … ${u.percent} %. Du kannst weiterarbeiten.`; fill = u.percent; now = false; }
  else if (st === 'neustart') { text = `ASK startet in ${u.countdown} s neu.`; fill = Math.round(((10 - u.countdown) / 10) * 100); later = true; }
  else if (st === 'installiert') { title = `${name} wird installiert`; text = 'ASK schliesst sich. Gleich ist ASK wieder da.'; now = false; }
  else if (u.postponed) text = 'Installiert sich, wenn du ASK schliesst.';
  else text = 'ASK startet neu, sobald der Auftrag fertig ist.';
  return (
    <div id="update-bar" className={`update-bar${st === 'lädt' ? ' loading' : ''}`} role="status">
      <span className="update-icon" aria-hidden="true"><Icon name={st === 'lädt' || st === 'installiert' ? 'down' : 'reload'} size={16} strokeWidth={2} /></span>
      <div className="update-main">
        <strong className="ell">{title}</strong>
        <span id="update-bar-text">{text}</span>
        {fill !== null ? <span id="update-bar-track" className="update-track"><span id="update-bar-fill" style={{ width: `${fill}%` }} /></span> : null}
        {now || later ? (
          <span className="update-acts">
            {later ? <button id="update-bar-later" type="button" className="btn small" onClick={() => api().updateLater()}>Später</button> : null}
            {now ? <button id="update-bar-now" type="button" className="btn small primary" onClick={() => api().updateAction()}>Jetzt neu starten</button> : null}
          </span>
        ) : null}
      </div>
    </div>
  );
}

function reducer(model, action) {
  if (action.type === 'load') return fromTranscript(action.items, action.running);
  if (action.type === 'events') return action.evs.reduce(applyEvent, model);
  return model;
}

// Kurze Vorschläge; ein Klick setzt den ganzen Auftrag ins Eingabefeld
const EXAMPLES = [
  ['list', 'Dateien zeigen', 'Welche Dateien liegen im Projekt?'],
  ['file', 'Word-Anleitung', 'Erstelle aus den Stichworten in meiner Textdatei eine Word-Anleitung mit Checkliste.'],
  ['list', 'CSV auswerten', 'Werte die CSV-Datei aus: Summen pro Kategorie und ein Diagramm, alles in Excel.'],
  ['term', 'Python-Skript', 'Schreibe ein Python-Skript, das prüft, ob ein Passwort sicher ist, und teste es.'],
];

// Breiten der Seitenleisten (ziehbar, pro Rechner gespeichert)
const PANELS = { chatbar: { css: '--left-w', min: 200, max: 460, def: 260, grows: 1 }, sidebar: { css: '--right-w', min: 280, max: 900, def: 360, grows: -1 } };
const CHAT_MIN = 380;
function loadWidths() {
  try { return JSON.parse(localStorage.getItem('ask.breiten') || '{}') || {}; } catch { return {}; }
}

const BROWSER0 = { url: '', back: false, forward: false, loading: false, running: false, mode: '', status: '', log: [], frame: '', liveId: null };

export default function App() {
  const [state, setState] = useState(null);
  const [model, dispatch] = useReducer(reducer, undefined, emptyChat);
  const [chatId, setChatId] = useState(null);
  const [running, setRunning] = useState(false);
  const [runStarted, setRunStarted] = useState(0);
  const [groups, setGroups] = useState([]);
  const [files, setFiles] = useState({ top: [], recent: [] });
  const [dialog, setDialog] = useState(null);
  const [startError, setStartError] = useState('');
  const [selected, setSelected] = useState(null);
  const [tab, setTab] = useState('dateien');
  const [verbose, setVerbose] = useState(false);
  const [query, setQuery] = useState('');
  const [renaming, setRenaming] = useState(null);
  const [activity, setActivity] = useState('Denkt nach …');
  const [browser, setBrowser] = useState(BROWSER0);
  const [browserBig, setBrowserBig] = useState(false);
  const [servers, setServers] = useState([]); // Entwicklungsserver des sichtbaren Projekts
  const [tabs, setTabs] = useState([]);
  const [uiAsk, setUiAsk] = useState(null); // Dialog des Hauptprozesses, z. B. «Update ist da»
  const uiAskRef = useRef(null);
  uiAskRef.current = uiAsk;
  const answerUiAsk = useCallback((value) => { const q = uiAskRef.current; if (q) api().uiAnswer(q.id, value); uiAskRef.current = null; setUiAsk(null); }, []);
  // Menü des Hauptprozesses (Projekt, Ordner): an der Stelle des letzten Rechtsklicks bzw. unter dem angeklickten Knopf
  const [uiMenu, setUiMenu] = useState(null);
  const uiMenuRef = useRef(null);
  uiMenuRef.current = uiMenu;
  const pointer = useRef({ x: 80, y: 60 });
  // Fokus und Markierung vor dem Menü: zurück, bevor die Wahl an den Hauptprozess geht, sonst fügt «Einfügen» ins Leere ein
  const menuFocus = useRef(null);
  const restoreFocus = () => {
    const f = menuFocus.current;
    menuFocus.current = null;
    if (f?.text) { const sel = window.getSelection(); sel.removeAllRanges(); sel.addRange(f.text); } // markierter Text im Chat
    if (!f?.el?.isConnected) return;
    f.el.focus({ preventScroll: true });
    if (f.range && typeof f.el.setSelectionRange === 'function') { try { f.el.setSelectionRange(...f.range); } catch { /* Feld ohne Markierung */ } }
  };
  const closeUiMenu = useCallback(() => { const m = uiMenuRef.current; if (m && !m.done) { restoreFocus(); api().uiAnswer(m.id, null); } uiMenuRef.current = null; setUiMenu(null); }, []);
  const pickUiMenu = useCallback((key) => { const m = uiMenuRef.current; if (m) { m.done = true; restoreFocus(); api().uiAnswer(m.id, key); } }, []);
  useEffect(() => {
    const note = (e) => {
      const btn = e.type === 'contextmenu' ? null : e.target.closest?.('button');
      if (btn) { const r = btn.getBoundingClientRect(); pointer.current = { x: r.left, y: r.bottom + 4, right: r.right }; }
      else pointer.current = { x: e.clientX, y: e.clientY };
    };
    document.addEventListener('pointerdown', note, true);
    document.addEventListener('contextmenu', note, true);
    return () => { document.removeEventListener('pointerdown', note, true); document.removeEventListener('contextmenu', note, true); };
  }, []);
  const closedTabs = useRef([]); // zuletzt geschlossene Tabs, für Ctrl+Shift+T
  const [widths, setWidths] = useState(loadWidths);
  const [follow, setFollow] = useState(true);
  const [showJump, setShowJump] = useState(false);

  const stateRef = useRef(null);
  const chatIdRef = useRef(null);
  const runningRef = useRef(false);
  const composer = useRef(null);
  const transcript = useRef(null);
  const followRef = useRef(true);
  const lastScroll = useRef(0);
  const buf = useRef([]);
  const flushTimer = useRef(0);
  const activityRef = useRef('Denkt nach …');

  const setAct = (text) => { if (activityRef.current !== text) { activityRef.current = text; setActivity(text); } };

  // ---------- Laden ----------

  const loadChats = useCallback(async () => setGroups(await api().chatGroups()), []);
  const loadTranscript = useCallback(async () => {
    const t = await api().chatTranscript();
    // Anderer Chat: noch gesammelte Ereignisse gehören zum vorherigen, verwerfen, sonst landen sie im neuen
    if (t.id !== chatIdRef.current) {
      clearTimeout(flushTimer.current);
      flushTimer.current = 0;
      buf.current = [];
    }
    chatIdRef.current = t.id;
    runningRef.current = t.running;
    setChatId(t.id);
    setRunning(t.running);
    dispatch({ type: 'load', items: t.items, running: t.running });
    followRef.current = true;
    setFollow(true);
  }, []);

  const flush = useCallback(() => {
    clearTimeout(flushTimer.current);
    flushTimer.current = 0;
    if (!buf.current.length) return;
    const evs = buf.current;
    buf.current = [];
    dispatch({ type: 'events', evs });
  }, []);

  // Browser-Bereich: Live-Ereignisse von browser (visual.js)
  const onLive = useCallback((ev) => {
    if (ev.phase === 'start') {
      setBrowser((b) => ({ ...b, liveId: ev.id, running: true, mode: '', status: 'lädt …', log: [], frame: '', url: ev.path || b.url }));
      setTab('browser');
      if (!stateRef.current?.sidebarOpen) api().setSidebar(true);
      return;
    }
    setBrowser((b) => {
      if (ev.id !== b.liveId) return b;
      const n = { ...b };
      if (ev.mode) n.mode = ev.mode;
      if (ev.frame) n.frame = ev.frame;
      if (ev.step) n.status = `▸ ${ev.step}`;
      if (ev.log) n.log = [...b.log, { text: ev.log, level: ev.level }].slice(-6);
      if (ev.phase === 'end') {
        n.running = false;
        n.status = `${ev.failed ? '✗' : '✓'} ${ev.summary}${ev.userInputs ? ` · ${ev.userInputs} eigene Eingaben` : ''}`;
      }
      return n;
    });
  }, []);

  const onChatEvent = useCallback((ev) => {
    // Parallele Chats: Ereignisse anderer Chats laufen im Hintergrund weiter, die Seitenleiste zeigt sie
    if (ev.chatId && ev.chatId !== chatIdRef.current) return;
    if (ev.type === 'live') { onLive(ev); return; }
    switch (ev.type) {
      case 'assistant-start': setAct(ev.webSearch ? 'Sucht im Web …' : 'Denkt nach …'); break;
      case 'reasoning': setAct('Denkt nach …'); break;
      case 'text': setAct('Schreibt …'); break;
      case 'tool-start': setAct(ev.name === 'frage_stellen' ? 'Wartet auf deine Antwort' : `${ev.label} …`); break;
      case 'tool-end': setAct('Denkt nach …'); break;
      case 'approval': setAct('Wartet auf deine Entscheidung'); break;
      case 'review': setAct('Prüft, ob der Schritt sicher ist …'); break;
      case 'approval-end': setAct('Denkt nach …'); break;
      case 'user':
        if (!ev.auto) { followRef.current = true; setFollow(true); }
        break;
      default:
    }
    buf.current.push(ev);
    if (ev.type === 'text' || ev.type === 'reasoning' || ev.type === 'tool-progress') {
      if (!flushTimer.current) flushTimer.current = setTimeout(flush, 80);
    } else {
      flush();
    }
  }, [flush, onLive]);

  useEffect(() => {
    const a = api();
    const offs = [
      a.onState((s) => {
        const prev = stateRef.current;
        stateRef.current = s;
        setState(s);
        if (s.servers) setServers(s.servers);
        if (prev && s.folder !== prev.folder) loadChats();
        if (s.chat.running !== runningRef.current) {
          runningRef.current = s.chat.running;
          setRunning(s.chat.running);
        }
        if (s.chat.id !== chatIdRef.current) loadTranscript();
      }),
      a.onChatEvent(onChatEvent),
      a.onChatChanged((c) => {
        if (c.id !== chatIdRef.current) loadTranscript();
        runningRef.current = c.running;
        setRunning(c.running);
        loadChats();
        if (!c.running) composer.current?.focus();
      }),
      a.onFiles((f) => setFiles(f)),
      a.onGroupsChanged?.(() => loadChats()),
      a.onProjectRename?.((folder) => { setRenaming(folder); if (!stateRef.current?.chatbarOpen) a.setChatbar(true); }),
      a.onMenuChooseFolder(() => openFolder(() => a.chooseFolder())),
      a.onMenuNewFolder(() => setDialog({ name: 'project' })),
      a.onMenuGithub(() => setDialog({ name: 'github' })),
      a.onUiAsk?.((q) => setUiAsk(q)),
      a.onUiMenu?.((m) => {
        const el = document.activeElement;
        const sel = window.getSelection();
        menuFocus.current = {
          el: el && el !== document.body ? el : null,
          range: el && 'selectionStart' in el && el.selectionStart != null ? [el.selectionStart, el.selectionEnd] : null,
          text: sel && sel.rangeCount && !sel.isCollapsed ? sel.getRangeAt(0).cloneRange() : null,
        };
        setUiMenu({ ...m, ...pointer.current });
      }),
      a.onFolderError((reason) => setDialog({ name: 'terminal', text: reason })),
      a.onBrowserState((s) => setBrowser((b) => ({ ...b, ...s }))),
      a.onServers?.((list) => setServers(list || [])),
      a.onBrowserNotice((text) => setBrowser((b) => ({ ...b, log: [...b.log, { text, level: 'warning' }].slice(-6) }))),
    ].filter(Boolean);
    (async () => {
      const s = await a.getState();
      stateRef.current = s;
      setState(s);
      if (s.servers) setServers(s.servers);
      await loadTranscript();
      await loadChats();
      setFiles(await a.files());
      composer.current?.focus();
    })();
    return () => offs.forEach((off) => typeof off === 'function' && off());
  }, []);

  useEffect(() => {
    if (running) { setRunStarted((t) => t || Date.now()); setAct('Denkt nach …'); } else setRunStarted(0);
  }, [running]);

  // Ausführliche Ansicht aus den gespeicherten Einstellungen (einmal beim Start)
  const prefsLoaded = useRef(false);
  useEffect(() => {
    if (!state?.prefs || prefsLoaded.current) return;
    prefsLoaded.current = true;
    setVerbose(!!state.prefs.verbose);
  }, [state?.prefs]);

  // Anderer Chat: eine wartende Nachricht kommt zurück ins Eingabefeld, statt im falschen Chat loszugehen
  useEffect(() => { composer.current?.restoreQueued(); }, [chatId]);

  // Tabs: zuletzt geöffnete Chats dieser Sitzung
  useEffect(() => {
    if (!state?.folder || !state.chat?.id || !state.chat.title) return;
    setTabs((t) => {
      const item = { folder: state.folder, id: state.chat.id, title: state.chat.title };
      const rest = t.filter((x) => x.id !== item.id);
      const i = t.findIndex((x) => x.id === item.id);
      const next = i >= 0 ? t.map((x) => (x.id === item.id ? item : x)) : [item, ...rest];
      return next.slice(0, 6);
    });
  }, [state?.folder, state?.chat?.id, state?.chat?.title]);

  // ---------- Abgeleitete Werte ----------

  const turns = useMemo(() => buildTurns(model, running), [model, running]);
  const tasksAll = useMemo(() => lastTasks(model), [model]);
  const lastTurn = turns[turns.length - 1];
  const tasksLive = running && !!lastTurn && lastTurn.steps.some((s) => s.type === 'tool' && s.entry.tasks?.length);
  const tasks = tasksAll && (tasksLive || tasksAll.some((t) => t.status !== 'erledigt')) ? tasksAll : null;
  const pendingQuestion = useMemo(() => [...model.entries].reverse().find((e) => e.question?.open) || null, [model]);
  const pendingApproval = useMemo(() => model.entries.find((e) => e.k === 'approval' && !e.answer) || null, [model]);
  // Knöpfe «Umsetzen» und «Ändern» nur beim letzten Plan, solange danach nichts eingegeben wurde
  const lastPlanKey = useMemo(() => { const e = [...model.entries].reverse().find((x) => x.k === 'user' || x.plan); return e?.plan ? e.key : null; }, [model]);
  const lastChanges = useMemo(() => [...model.entries].reverse().find((e) => e.k === 'changes' && !e.undone) || null, [model]);

  // Markierungen im Dateibaum: was der letzte Auftrag geändert hat; während der Arbeit, was gerade geändert wurde
  const { marks, removed } = useMemo(() => {
    const m = new Map();
    const c = lastChanges?.changes;
    if (c) {
      for (const p of c.added) m.set(norm(p), 'neu');
      for (const p of c.changed) m.set(norm(p), 'geaendert');
    }
    if (running && runStarted) for (const f of files.recent) if (f.modified >= runStarted && !m.has(norm(f.path))) m.set(norm(f.path), 'geaendert');
    // Ordner mit markierten Dateien ebenfalls markieren
    for (const [p, v] of [...m]) {
      const parts = p.split('/');
      for (let i = 1; i < parts.length; i++) { const d = parts.slice(0, i).join('/'); if (!m.has(d)) m.set(d, v === 'neu' ? 'neu' : 'geaendert'); }
    }
    return { marks: m, removed: c ? c.removed : [] };
  }, [lastChanges, files, running, runStarted]);

  const allFiles = useMemo(() => [...files.recent, ...files.top].filter((f) => !f.dir), [files]);
  const findFile = useCallback((text) => {
    const rel = norm(String(text).trim());
    if (!rel || rel.length > 260) return null;
    const changed = lastChanges ? [...lastChanges.changes.added, ...lastChanges.changes.changed] : [];
    return allFiles.find((f) => norm(f.path) === rel) || files.recent.find((f) => f.name.toLowerCase() === rel)
      || (changed.find((p) => norm(p) === rel) ? { path: changed.find((p) => norm(p) === rel), name: baseName(rel) } : null);
  }, [allFiles, files, lastChanges]);

  // Eigener Dialog des Hauptprozesses (Updates) zählt wie ein Dialog: Browser und ASK-Seite treten zurück
  const dialogName = uiAsk ? 'app-ask' : activeDialog(state, dialog);
  const ask = state?.ask || {};
  const needsLogin = !!state?.askUrl && !!state?.warningAccepted && ['nicht-angemeldet', 'unbekannt', 'pruefen'].includes(ask.state);
  const showAskPage = ask.state === 'nicht-angemeldet';
  const loadFailed = ask.loadError || null; // Ladefehler der ASK-Seite, aus dem Status (askapi.js)
  const [slow, setSlow] = useState(false); // «Verbinde mit ASK …» dauert ungewöhnlich lange
  const waiting = needsLogin && !showAskPage && !loadFailed;
  useEffect(() => {
    setSlow(false);
    if (!waiting) return undefined;
    const t = setTimeout(() => setSlow(true), 15000);
    return () => clearTimeout(t);
  }, [waiting]);
  const leftOpen = state?.chatbarOpen !== false;
  const rightOpen = state?.sidebarOpen !== false;
  const browserActive = rightOpen && tab === 'browser' && !dialogName && !needsLogin && !uiMenu; // Menüs lägen sonst unter der Browser-Ansicht

  // ---------- Aktionen ----------

  const openFolder = async (action) => {
    setStartError('');
    const res = await action();
    if (res && !res.ok && !res.canceled) {
      if (stateRef.current?.folder) setDialog({ name: 'terminal', text: res.reason });
      else setStartError(res.reason);
    }
    return res;
  };

  const openFile = useCallback((p) => {
    const found = allFiles.find((f) => norm(f.path) === norm(p));
    setSelected(found || { path: p, name: baseName(p), modified: Date.now() });
    setTab('vorschau');
    if (!stateRef.current?.sidebarOpen) api().setSidebar(true);
  }, [allFiles]);

  const send = async (text, fileList, kept = []) => {
    runningRef.current = true;
    setRunning(true);
    setRunStarted(Date.now());
    let res;
    try {
      res = await api().chatSend(text, fileList.map(({ name, type, data }) => ({ name, type, data })), kept);
    } catch (err) {
      res = { ok: false, reason: err?.message || String(err) };
    }
    if (res && res.ok === false) {
      runningRef.current = false;
      setRunning(false);
      dispatch({ type: 'events', evs: [{ type: 'local-error', text: res.reason || 'Senden fehlgeschlagen.' }] });
    }
    return res;
  };

  // Letzte Eingabe zurücknehmen: ändern (zurück ins Eingabefeld) oder gleich neu senden (ENTSCHEIDE E36)
  const editLast = async (resend) => {
    if (runningRef.current) return;
    const res = await api().chatRewind();
    if (!res || res.cancelled) return;
    if (!res.ok) { setDialog({ name: 'terminal', text: res.reason || 'Die Nachricht lässt sich nicht zurücknehmen.' }); return; }
    await loadTranscript();
    if (res.failed?.length) notice(`Nicht zurückgesetzt: ${res.failed.map((f) => baseName(f?.path || f)).join(', ')}`);
    if (resend && !blocker) { await send(res.text || '', [], res.attachments || []); return; }
    composer.current?.restore({ text: res.text || '', kept: res.attachments || [] });
  };

  const setVerboseSaved = (on) => { setVerbose(on); api().setPrefs?.({ verbose: on }); };

  const notice = (text) => dispatch({ type: 'events', evs: [{ type: 'notice', text }] });

  const blocker = !state?.folder ? 'Zuerst ein Projekt wählen.'
    : ask.state === 'fehler' ? 'ASK meldet einen Fehler (Hinweis über dem Eingabefeld).'
    : ask.state !== 'angemeldet' ? 'Zuerst bei ASK anmelden.'
    : state.terminal.state === 'startet' ? 'Das Terminal startet …'
    : state.terminal.state !== 'läuft' ? 'Das Terminal läuft nicht (unten links auf «Terminal» klicken).'
    : !state.model ? 'ASK bietet kein Modell an.' : '';

  const newChat = async (folder) => {
    if (runningRef.current) return;
    const res = await api().chatNew(folder);
    if (res && res.ok === false && res.reason) setDialog({ name: 'terminal', text: res.reason });
    composer.current?.focus();
  };

  const openChat = async (folder, id) => {
    const res = await api().chatOpen(folder, id);
    if (res && res.ok === false && res.reason) setDialog({ name: 'terminal', text: res.reason });
  };

  const handleGoal = async (arg) => {
    if (!arg) {
      const g = stateRef.current?.chat?.goal;
      notice(g ? `Ziel: ${g.text}${g.done ? ' (erreicht)' : ''}` : 'Kein Ziel gesetzt. /goal <Ziel> setzt eines; das Modell arbeitet dann, bis es erreicht ist.');
      return;
    }
    if (/^(aus|stopp|stop|löschen|off|clear)$/i.test(arg)) { await api().chatGoal(''); return; }
    if (blocker) return;
    await api().chatGoal(arg);
    await send(arg, []);
  };

  const skillCommands = useSkillCommands({ composer, stateRef, notice }); // eigene Skills als /name (ENTSCHEIDE E51)

  const commands = [
    { cmd: '/neu', text: 'Neuen Chat beginnen', run: () => newChat() },
    { cmd: '/projekt', text: 'Neues Projekt anlegen', run: () => setDialog({ name: 'project' }) },
    { cmd: '/goal', text: 'Ziel setzen: /goal <Ziel>; das Modell arbeitet, bis es erreicht ist', run: () => composer.current?.setText('/goal ') },
    { cmd: '/suchen', text: 'Chats durchsuchen', run: () => { if (!leftOpen) api().setChatbar(true); setTimeout(() => document.getElementById('chat-search')?.focus(), 50); } },
    { cmd: '/ordner', text: 'Vorhandenen Ordner als Projekt öffnen', run: () => openFolder(() => api().chooseFolder()) },
    { cmd: '/explorer', text: 'Projektordner im Explorer öffnen', run: () => api().showFolder() },
    { cmd: '/dateien', text: 'Dateien des Projekts zeigen', run: () => { setTab('dateien'); if (!rightOpen) api().setSidebar(true); } },
    { cmd: '/github', text: 'Mit GitHub verbinden, Repository des Projekts', run: () => setDialog({ name: 'github' }) },
    { cmd: '/plan', text: 'Planmodus: ASK legt zuerst einen Plan vor', run: () => api().setSettings({ mode: 'plan' }) },
    { cmd: '/auto', text: 'Auto: ASK arbeitet ohne Rückfragen', run: () => api().setSettings({ mode: 'auto' }) },
    { cmd: '/nachfragen', text: 'Nachfragen: ASK fragt vor heiklen Schritten', run: () => api().setSettings({ mode: 'nachfragen' }) },
    { cmd: '/modell', text: 'Modell wählen', run: () => document.getElementById('model-select')?.focus() },
    { cmd: '/denken', text: 'Denkaufwand einstellen (Standard, Tief bis Maximal)', run: () => document.getElementById('think-btn')?.click() },
    { cmd: '/web', text: 'Websuche ein- oder ausschalten', run: () => { if (state.webSearchAvailable) api().setWebSearch(!state.webSearch); } },
    { cmd: '/chats', text: 'Frühere Chats für das Modell ein- oder ausschalten', run: () => api().setChatMemory(!state.chatMemory) },
    { cmd: '/einstellungen', text: 'Einstellungen: Darstellung, Rückfragen, Version', run: () => setDialog({ name: 'settings' }) },
    { cmd: '/nutzung', text: 'Nutzung: Aufträge, Token und Dateien pro Projekt', run: () => setDialog({ name: 'usage' }) },
    { cmd: '/update', text: 'Nach einer neuen Version von ASK suchen', run: () => api().updateAction() },
    { cmd: '/beta', text: 'Beta-Versionen erhalten ein- oder ausschalten', run: () => api().setBetaUpdates(!state.update?.beta, true) },
    { cmd: '/neustart', text: 'Terminal neu starten', run: () => api().restartTerminal() },
    { cmd: '/abmelden', text: 'Bei ASK abmelden', run: () => api().logout() },
    { cmd: '/gedaechtnis', text: 'Gedächtnis: was ASK sich über dich und deine Projekte merkt', run: () => setDialog({ name: 'memory' }) },
    { cmd: '/skills', text: 'Eigene Skills ansehen, erstellen, teilen', run: () => setDialog({ name: 'skills' }) },
    ...skillCommands,
  ];

  // ---------- Tastatur ----------

  useEffect(() => {
    const onKey = (e) => {
      const k = e.key.toLowerCase();
      if (e.ctrlKey && !e.shiftKey && !e.altKey && k === 't') { e.preventDefault(); composer.current?.toggleTodo(); }
      if (e.ctrlKey && !e.shiftKey && !e.altKey && k === 'o') { e.preventDefault(); setVerboseSaved(!verbose); }
      if (e.ctrlKey && !e.shiftKey && !e.altKey && k === 'n') { e.preventDefault(); newChat(); }
      // Tabs: Ctrl+W schliesst, Ctrl+Tab wechselt, Ctrl+1 bis 9 springt, Ctrl+Shift+T holt den zuletzt geschlossenen zurück
      if (e.ctrlKey && !e.shiftKey && !e.altKey && k === 'w') { e.preventDefault(); if (!dialog) closeTab(shownTabs.find((t) => t.id === chatId)); }
      if (e.ctrlKey && !e.altKey && e.key === 'Tab') { e.preventDefault(); switchTab(e.shiftKey ? -1 : 1); }
      if (e.ctrlKey && e.shiftKey && !e.altKey && k === 't') { e.preventDefault(); reopenTab(); }
      if (e.ctrlKey && !e.shiftKey && !e.altKey && /^[1-9]$/.test(e.key)) {
        const t = e.key === '9' ? shownTabs[shownTabs.length - 1] : shownTabs[Number(e.key) - 1];
        if (t) { e.preventDefault(); if (t.id !== chatId && !t.placeholder) openChat(t.folder, t.id); }
      }
      if (e.key === 'Escape') {
        if (dialog) { setDialog(null); return; }
        if (browserBig) { setBrowserBig(false); return; }
        if (runningRef.current && !document.querySelector('.popover')) composer.current?.stop();
      }
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  });

  // ---------- Scrollen: der Ansicht folgen, bis die Person hochscrollt ----------

  const fromBottom = (t) => t.scrollHeight - t.scrollTop - t.clientHeight;
  useLayoutEffect(() => {
    const t = transcript.current;
    if (t && followRef.current) t.scrollTop = t.scrollHeight;
  });
  const onScroll = () => {
    const t = transcript.current;
    if (!t) return;
    if (t.scrollTop < lastScroll.current - 1 && fromBottom(t) > 4) { followRef.current = false; setFollow(false); }
    else if (t.scrollTop > lastScroll.current && fromBottom(t) < 4) { followRef.current = true; setFollow(true); }
    lastScroll.current = t.scrollTop;
    setShowJump(!followRef.current && fromBottom(t) > 40);
  };
  const onWheel = (e) => {
    if (e.deltaY < 0) { followRef.current = false; setFollow(false); }
  };
  const jump = () => { followRef.current = true; setFollow(true); const t = transcript.current; if (t) t.scrollTop = t.scrollHeight; setShowJump(false); };
  const onMediaLoad = () => { const t = transcript.current; if (t && followRef.current) t.scrollTop = t.scrollHeight; };

  // ---------- Anmeldeseite von ASK (WebContentsView über #ask-host) ----------

  const askHost = useRef(null);
  useLayoutEffect(() => {
    const report = () => {
      const el = askHost.current;
      if (!el) return;
      const r = el.getBoundingClientRect();
      api().setAskBounds({ x: r.x, y: r.y, width: r.width, height: r.height });
    };
    report();
    api().setAskVisible(needsLogin && showAskPage && !dialogName && !loadFailed && !uiMenu);
    const ro = new ResizeObserver(report);
    if (askHost.current) ro.observe(askHost.current);
    window.addEventListener('resize', report);
    return () => { ro.disconnect(); window.removeEventListener('resize', report); };
  }, [needsLogin, showAskPage, dialogName, loadFailed, uiMenu]);

  // ---------- Breiten ----------

  const clampWidth = useCallback((name, w) => {
    const other = name === 'chatbar' ? (rightOpen ? widths.sidebar || PANELS.sidebar.def : 0) : (leftOpen ? widths.chatbar || PANELS.chatbar.def : 0);
    const room = window.innerWidth - other - CHAT_MIN;
    return Math.round(Math.max(PANELS[name].min, Math.min(PANELS[name].max, room, w)));
  }, [widths, leftOpen, rightOpen]);
  const startResize = (name) => (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const handle = e.currentTarget;
    const startX = e.clientX;
    const startW = (name === 'chatbar' ? document.getElementById('chatbar') : document.getElementById('sidebar'))?.getBoundingClientRect().width || PANELS[name].def;
    handle.setPointerCapture(e.pointerId);
    document.body.classList.add('resizing');
    let last = widths;
    const move = (ev) => { last = { ...last, [name]: clampWidth(name, startW + PANELS[name].grows * (ev.clientX - startX)) }; setWidths(last); };
    const end = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', end);
      handle.removeEventListener('lostpointercapture', end);
      document.body.classList.remove('resizing');
      try { localStorage.setItem('ask.breiten', JSON.stringify(last)); } catch { /* nur für diese Sitzung */ }
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', end);
    handle.addEventListener('lostpointercapture', end);
  };
  const resetWidth = (name) => () => { const w = { ...widths }; delete w[name]; setWidths(w); try { localStorage.setItem('ask.breiten', JSON.stringify(w)); } catch { /* egal */ } };
  const keyResize = (name) => (e) => {
    if (e.key !== 'ArrowLeft' && e.key !== 'ArrowRight') return;
    e.preventDefault();
    const cur = widths[name] || PANELS[name].def;
    const w = { ...widths, [name]: clampWidth(name, cur + (e.key === 'ArrowRight' ? 1 : -1) * PANELS[name].grows * (e.shiftKey ? 64 : 16)) };
    setWidths(w);
    try { localStorage.setItem('ask.breiten', JSON.stringify(w)); } catch { /* egal */ }
  };

  if (!state) return <div className="boot"><img src="./tf-mark.png" alt="" className="logo-mark" /> ASK startet …</div>;

  const ctx = {
    verbose, running, activity, blocker,
    editLast,
    isFile: (t) => !!findFile(t),
    openFile,
    openFileByName: (t) => { const f = findFile(t); if (f) openFile(f.path); },
    openLink: (u) => api().openLink(u),
    copy: (t) => api().copy(t),
    approve: (id, a) => api().chatApprove(id, a),
    answer: (id, t) => api().chatAnswer(id, t),
    forgetMemory: async (id) => { try { return !!(await api().memoryRemove(id)); } catch { return false; } },
    undo: async (snap) => { const res = await api().chatUndo(snap); if (res && !res.ok && !res.cancelled) dispatch({ type: 'events', evs: [{ type: 'local-error', text: res.reason }] }); },
    undoSnapshot: lastChanges?.snapshot,
    lastPlanKey,
    // Plan umsetzen: auf Auto wechseln und loslegen; ändern: Wunsch ins Eingabefeld schreiben lassen
    implementPlan: async () => { await api().setSettings({ mode: 'auto' }); send('Setze den Plan um.', []); },
    revisePlan: () => composer.current?.setText('Ändere den Plan: '),
    onMediaLoad,
  };

  const project = groups.find((g) => g.current);
  // Tabs oben: der aktuelle Chat steht immer drin, ein neuer noch ohne Titel als «Neuer Chat»
  const shownTabs = tabs.some((t) => t.id === chatId) ? tabs : [{ id: chatId, folder: state.folder, title: 'Neuer Chat', placeholder: true }, ...tabs];
  const closeTabs = (list, keep) => {
    const ids = new Set(list.filter((t) => !t.placeholder).map((t) => t.id));
    closedTabs.current = [...list.filter((t) => !t.placeholder), ...closedTabs.current].slice(0, 10);
    setTabs((all) => all.filter((x) => !ids.has(x.id)));
    if (list.some((t) => t.id === chatId) && keep && keep.id !== chatId) openChat(keep.folder, keep.id);
  };
  const closeTab = (t, i = shownTabs.findIndex((x) => x.id === t.id)) => {
    if (shownTabs.length < 2) return;
    closeTabs([t], t.id === chatId ? shownTabs[i + 1] || shownTabs[i - 1] : null); // aktiven Tab schliessen: zum Nachbarn wechseln
  };
  const reopenTab = () => {
    const t = closedTabs.current.shift();
    if (t) openChat(t.folder, t.id);
  };
  const moveTab = (id, overId, after) => setTabs((all) => {
    const moving = all.find((x) => x.id === id);
    if (!moving) return all;
    const rest = all.filter((x) => x.id !== id);
    const at = rest.findIndex((x) => x.id === overId);
    if (at < 0) return all;
    rest.splice(after ? at + 1 : at, 0, moving);
    return rest;
  });
  const renameTab = async (t, title) => {
    await api().chatRename(t.folder, t.id, title);
    setTabs((all) => all.map((x) => (x.id === t.id ? { ...x, title } : x)));
    loadChats();
  };
  const switchTab = (delta) => {
    if (shownTabs.length < 2) return;
    const i = shownTabs.findIndex((x) => x.id === chatId);
    const t = shownTabs[(i + delta + shownTabs.length) % shownTabs.length];
    if (t && !t.placeholder) openChat(t.folder, t.id);
  };
  const cols = [leftOpen ? `${clampWidth('chatbar', widths.chatbar || PANELS.chatbar.def)}px` : '0px', leftOpen ? '5px' : '0px', 'minmax(0, 1fr)',
    rightOpen ? '5px' : '0px', rightOpen ? `${clampWidth('sidebar', widths.sidebar || PANELS.sidebar.def)}px` : '0px'];

  return (
    <div className={`app${browserBig && rightOpen && tab === 'browser' ? ' browser-big' : ''}${verbose ? ' verbose' : ''}`} style={{ '--left-w': leftOpen ? cols[0] : '324px' }}>
      <header className="topbar">
        <div className="brand" style={{ width: leftOpen ? `calc(${cols[0]} - 8px)` : 'auto' }}>
          <button type="button" id="chatbar-btn" className="icon-btn" title="Projekte ein- oder ausblenden" aria-label="Projekte ein- oder ausblenden"
            onClick={() => api().setChatbar(!leftOpen)}><Icon name="sidebar" size={17} /></button>
          <img src="./tf-mark.png" alt="" className="logo-mark" />
          <span className="wordmark">ASK</span>
        </div>
        <Tabs tabs={shownTabs} chatId={chatId} onNew={() => newChat()} onOpen={(t) => openChat(t.folder, t.id)}
          colorOf={(t) => groups.find((g) => g.folder.toLowerCase() === (t.folder || '').toLowerCase())?.color || 'var(--dim)'}
          isBusy={(t) => (t.id === chatId && running) || (state.running || []).some((r) => r.id === t.id)}
          onStop={(t) => (t.id === chatId ? composer.current?.stop() : api().chatStopChat(t.folder, t.id))}
          onClose={closeTab} onCloseMany={closeTabs} onMove={moveTab} onRename={renameTab} />
        <span className="spacer" />
        <button type="button" id="folder-btn" className="crumb" hidden={leftOpen} title={state.folder ? `${state.folder}\nKlicken für weitere Projekte und Aktionen` : 'Projekt wählen'}
          onClick={() => api().folderMenu()} disabled={!state.warningAccepted || !state.askUrl}>
          {project ? <span className="swatch" style={{ background: project.color }} /> : null}
          <span id="folder-name">{project?.name || (state.folder ? baseName(state.folder) : 'Kein Projekt')}</span>
          <Icon name="chevDown" size={13} />
        </button>
        <button type="button" id="sidebar-btn" className={`icon-btn${rightOpen ? ' on' : ''}`} title="Dateien, Vorschau und Browser ein- oder ausblenden" aria-label="Seitenleiste rechts"
          onClick={() => api().setSidebar(!rightOpen)}><Icon name="panel" size={17} /></button>
      </header>

      <UpdateBar u={state.update} />
      <main className="layout" style={{ gridTemplateColumns: cols.join(' ') }}>
        {leftOpen ? (
          <Sidebar groups={groups} chatId={chatId} running={running} runningChats={state.running || []} query={query} setQuery={setQuery}
            renaming={renaming} onRenameDone={async (folder, name) => { setRenaming(null); if (name.trim()) await api().projectUpdate(folder, { name }); loadChats(); }}
            onNewChat={() => newChat()} onOpenChat={openChat}
            onDeleteChat={async (folder, id, title) => { if (await api().chatDelete(folder, id, title)) loadChats(); }}
            onNewProject={() => setDialog({ name: 'project' })} onProjectMenu={(folder) => api().projectMenu(folder)}
            onNewInProject={(folder) => newChat(folder)} state={state}
            onSettings={() => setDialog({ name: 'settings' })} onStatus={(name) => setDialog({ name })}
            update={state.update} onUpdate={() => api().updateAction()}
            onRefresh={loadChats} />
        ) : <span />}
        {leftOpen ? <div className="resizer" data-panel="chatbar" role="separator" aria-orientation="vertical" tabIndex={0} aria-label="Breite der Projekte"
          title="Ziehen: Breite ändern · Doppelklick: Standardbreite" onPointerDown={startResize('chatbar')} onDoubleClick={resetWidth('chatbar')} onKeyDown={keyResize('chatbar')} /> : <span />}

        <section className={`chat-area${!turns.length && !needsLogin ? ' empty' : ''}`}>
          {needsLogin ? (
            <div id="login-area" className={`login-area${showAskPage ? '' : ' loading'}`}>
              <div className="login-head">
                <span><img src="./tf-mark.png" alt="" className="logo-mark" /> Bei ASK anmelden. Danach arbeitest du hier im Chat; die Seite verschwindet wieder.</span>
                <button id="login-reload" type="button" className="btn small" onClick={() => api().reloadAsk()}>Seite neu laden</button>
              </div>
              <div id="ask-host" ref={askHost} className="ask-host">
                <div id="ask-placeholder" className="ask-placeholder">
                  {loadFailed ? (
                    <div id="ask-unreachable">
                      <p>ASK ist nicht erreichbar ({loadFailed.description}).</p>
                      <p>Internetverbindung prüfen und dann erneut laden. Die App versucht es auch von selbst wieder.</p>
                      <button type="button" className="btn primary" onClick={() => api().reloadAsk()}>Erneut laden</button>
                    </div>
                  ) : slow ? (
                    <div id="ask-slow">
                      <p><span className="spin" aria-hidden="true" /> {ask.state === 'pruefen' ? 'Anmeldung wird geprüft …' : 'Verbinde mit ASK …'}</p>
                      <p>Das dauert ungewöhnlich lange. Internetverbindung prüfen und die Seite neu laden.</p>
                      {ask.detail ? <p className="muted small">{ask.detail}</p> : null}
                      <button type="button" className="btn primary" onClick={() => { setSlow(false); api().reloadAsk(); }}>Seite neu laden</button>
                    </div>
                  ) : <><span className="spin" aria-hidden="true" /> {ask.state === 'pruefen' ? 'Anmeldung wird geprüft …' : 'Verbinde mit ASK …'}</>}
                </div>
              </div>
            </div>
          ) : null}
          <div id="transcript" ref={transcript} className="transcript" onScroll={onScroll} onWheel={onWheel} hidden={needsLogin}>
            <div className="column">
              {!turns.length ? (
                <div id="welcome" className="welcome">
                  <img src="./tf-mark.png" alt="" className="welcome-mark logo-mark" />
                  <h1>Woran arbeiten wir{project ? ` in ${project.name}` : ''}?</h1>
                </div>
              ) : null}
              <div id="messages" className="messages">
                {turns.map((t, i) => <Turn key={t.key} turn={t} ctx={ctx} last={i === turns.length - 1} />)}
              </div>
            </div>
          </div>
          {<button id="jump-btn" type="button" className="jump-btn" hidden={!(showJump && !follow)} onClick={jump}><Icon name="chevDown" size={14} /> nach unten</button>}
          {ask.state === 'fehler' && !needsLogin ? (
            // Angemeldet, aber ASK lässt nicht arbeiten (z. B. kein Modell für das Konto): Fehler zeigen und einen Weg zurück zur Anmeldung
            <div id="ask-error" className="column ask-error" role="alert">
              <div className="ask-error-box">
                <p><strong>ASK meldet einen Fehler:</strong> {ask.message || 'unbekannt'}</p>
                <div className="ask-error-actions">
                  <button id="ask-error-retry" type="button" className="btn small" onClick={async () => { await api().connectAsk(); await api().models(); }}>Erneut prüfen</button>
                  <button id="ask-error-relogin" type="button" className="btn small" onClick={() => api().logout()}><Icon name="logout" size={14} /> Abmelden und neu anmelden</button>
                </div>
              </div>
            </div>
          ) : null}
          {/* auch während der Anmeldung da (unter der Anmeldeseite), mit Hinweis im Platzhalter */}
          {(
            <Composer ref={composer} state={state} running={running} block={blocker} goal={state.chat?.goal} tasks={tasks} tasksLive={tasksLive}
              context={state.chat?.context} todoCollapsed={state.prefs?.todoCollapsed ?? true}
              activity={(tasksLive && tasks?.find((t) => t.status === 'in_arbeit')?.titel) ? `${tasks.find((t) => t.status === 'in_arbeit').titel} · ${activity}` : activity}
              runStarted={runStarted} commands={commands} pendingQuestion={pendingQuestion} pendingApproval={pendingApproval}
              onSend={send} onStop={() => api().chatStop()} onAnswer={(id, t) => api().chatAnswer(id, t)} onGoal={handleGoal}
              onGoalClear={() => api().chatGoal('')} setModel={(id) => api().setModel(id)} setThinking={(l) => api().setThinking(l)}
              setWeb={(on) => api().setWebSearch(on)} notice={notice}
              mode={state.mode || 'auto'} setMode={(mode) => api().setSettings({ mode })} />
          )}
          {!turns.length && !needsLogin ? (
            // Leerer Chat: Eingabe in der Mitte, Vorschläge darunter (wie in Claude)
            <div className="column examples-wrap">
              <div className="examples">
                {EXAMPLES.map(([icon, label, text]) => (
                  <button key={label} type="button" className="example" title={text} onClick={() => composer.current?.setText(text)}><Icon name={icon} size={14} />{label}</button>
                ))}
              </div>
            </div>
          ) : null}
        </section>

        {rightOpen ? <div className="resizer" data-panel="sidebar" role="separator" aria-orientation="vertical" tabIndex={0} aria-label="Breite der Seitenleiste"
          title="Ziehen: Breite ändern · Doppelklick: Standardbreite" onPointerDown={startResize('sidebar')} onDoubleClick={resetWidth('sidebar')} onKeyDown={keyResize('sidebar')} /> : <span />}
        {rightOpen ? (
          <RightPanel tab={tab} setTab={setTab} root={state.folder} files={files} marks={marks} removed={removed} selected={selected}
            onSelect={setSelected} onBig={() => setDialog({ name: 'preview' })} browser={browser} browserActive={browserActive}
            onBrowserClose={() => { api().browserClose(); setBrowser(BROWSER0); setBrowserBig(false); setTab('dateien'); }}
            onBrowserBig={() => setBrowserBig(!browserBig)} browserBig={browserBig}
            onMention={(p) => { const rel = String(p).replaceAll('\\', '/'); composer.current?.insert(/\s/.test(rel) ? `@"${rel}"` : `@${rel}`); }} notice={notice} servers={servers}
            running={running} dialogOpen={!!dialogName} onGithub={() => setDialog({ name: 'github' })} />
        ) : <span />}
      </main>

      <Dialogs state={state} dialog={dialog} setDialog={setDialog} selected={selected} startError={startError} onVerbose={setVerboseSaved} />
      {uiAsk ? <AppAsk key={uiAsk.id} ask={uiAsk} onAnswer={answerUiAsk} /> : null}
      {uiMenu ? <PopMenu key={uiMenu.id} x={uiMenu.right && uiMenu.right > window.innerWidth - 280 ? uiMenu.right - 260 : uiMenu.x} y={uiMenu.y} label="Menü"
        onClose={closeUiMenu} items={menuItems(uiMenu.items, pickUiMenu)} /> : null}
    </div>
  );
}
