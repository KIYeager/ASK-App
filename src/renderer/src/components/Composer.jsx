import { forwardRef, useEffect, useImperativeHandle, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { extOf, fileBadge, formatSize } from '../lib/format.js';

const THINK_LEVELS = ['standard', 'tief', 'mittel', 'hoch', 'maximal'];
const THINK_NAMES = { standard: 'Standard', tief: 'Tief', mittel: 'Mittel', hoch: 'Hoch', maximal: 'Maximal' };
const MAX_ATTACH = 10;
const MAX_ATTACH_SIZE = 50 * 1024 * 1024;
const IMAGE_RE = /\.(png|jpe?g|gif|webp)$/i;
const TODO_SHOWN = 7;

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

function ThinkPicker({ level, onChange }) {
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
  const idx = Math.max(1, THINK_LEVELS.indexOf(level));
  return (
    <div className="think-wrap" ref={ref}>
      <button type="button" id="think-btn" className={`chip${level !== 'standard' ? ' on' : ''}`} onClick={() => setOpen(!open)}
        title="Denkaufwand des Modells (/denken)" aria-expanded={open}>
        <Icon name="spark" size={14} /> <span id="think-label">{THINK_NAMES[level] || 'Standard'}</span>
      </button>
      {open ? (
        <div id="think-pop" className={`popover think-pop${level === 'standard' ? ' standard' : ''}`}>
          <div className="think-head"><span>Denkaufwand</span><span className="think-value">{THINK_NAMES[level]}</span></div>
          <div className="think-scale"><span>Tief</span><span>Maximal</span></div>
          <input id="think-range" type="range" min="1" max="4" step="1" value={idx} aria-label="Denkaufwand"
            onChange={(e) => onChange(THINK_LEVELS[Number(e.target.value)])} autoFocus />
          <div className="think-foot">
            <button id="think-standard" type="button" className="link" onClick={() => onChange('standard')}>Standard</button>
            <span className="muted">· das Modell entscheidet selbst</span>
          </div>
        </div>
      ) : null}
    </div>
  );
}

const Composer = forwardRef(function Composer(props, ref) {
  const { state, running, block, goal, tasks, tasksLive, activity, runStarted, commands, pendingQuestion, pendingApproval,
    onSend, onStop, onAnswer, onGoalClear, setModel, setThinking, setWeb, setMemory, notice } = props;
  const [text, setText] = useState('');
  const [pending, setPending] = useState([]);
  const [slashIndex, setSlashIndex] = useState(0);
  const [dropping, setDropping] = useState(false);
  const [todoOpen, setTodoOpen] = useState(true);
  const [todoPeek, setTodoPeek] = useState(false);
  const [, tick] = useState(0);
  const input = useRef(null);
  const fileInput = useRef(null);
  const sent = useRef({ list: [], index: -1 });

  useImperativeHandle(ref, () => ({
    focus: () => input.current?.focus(),
    setText: (t) => { setText(t); setTimeout(() => { input.current?.focus(); autosize(); }, 0); },
    addFiles: (list) => addFiles(list),
    toggleTodo: () => toggleTodo(),
    restore: (t, files) => { if (t && !input.current?.value) setText(t); setPending(files || []); },
  }));

  useEffect(() => {
    if (!running) return undefined;
    const t = setInterval(() => tick((n) => n + 1), 500);
    return () => clearInterval(t);
  }, [running]);
  useEffect(() => { if (!running) setTodoPeek(false); }, [running]);
  useEffect(autosize, [text]);

  function autosize() {
    const el = input.current;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${Math.min(el.scrollHeight, 260)}px`;
  }

  function toggleTodo() {
    if (!tasks?.length) return;
    if (running && tasksLive) setTodoOpen((o) => !o);
    else setTodoPeek((o) => !o);
  }

  async function addFiles(list) {
    const next = [];
    for (const file of [...list]) {
      if (pending.length + next.length >= MAX_ATTACH) { notice(`Höchstens ${MAX_ATTACH} Anhänge auf einmal.`); break; }
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
    if (pendingQuestion) {
      if (!text.trim()) return;
      onAnswer(pendingQuestion.id, text.trim());
      setText('');
      return;
    }
    if (running) { onStop(); return; }
    const t = text.trim();
    if (!t && !pending.length) return;
    const goalCmd = /^\/goal(?:\s+([\s\S]*))?$/i.exec(t);
    if (goalCmd) { setText(''); props.onGoal((goalCmd[1] || '').trim()); return; }
    if (slashMatches.length) { runSlash(slashMatches[slashIndex] || slashMatches[0]); return; }
    if (block) return;
    if (t) { sent.current.list.push(t); sent.current.index = -1; }
    const files = pending;
    setText('');
    setPending([]);
    const res = await onSend(t, files);
    if (res && res.ok === false) {
      setPending(files);
      if (t) setText((cur) => cur || t);
    } else {
      for (const f of files) if (f.url) URL.revokeObjectURL(f.url);
    }
  }

  function onKey(e) {
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

  const showTodoOpen = running && tasksLive ? todoOpen : todoPeek;
  const answering = !!pendingQuestion;
  const placeholder = answering ? 'Antworte auf die Frage von ASK …'
    : pendingApproval ? 'ASK wartet oben auf deine Entscheidung …'
    : running ? 'Das Modell arbeitet … (Esc stoppt)' : (block || 'Auftrag beschreiben …');
  const sendDisabled = answering ? !text.trim() : (!running && !!block);
  const seconds = runStarted ? Math.round((Date.now() - runStarted) / 1000) : 0;

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
            <span className="muted">(<span id="activity-time">{seconds} s</span> · Esc zum Stoppen)</span>
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
        <div id="input-box" className={`input-box${block && !running && !answering ? ' disabled' : ''}${dropping ? ' dropping' : ''}${answering ? ' answering' : ''}`}>
          {pending.length ? (
            <div id="attach-row" className="attach-row">
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
          <textarea id="input" ref={input} rows={1} spellCheck value={text} placeholder={placeholder} title="Enter senden · Shift+Enter neue Zeile · / Befehle · Ctrl+O Details"
            onChange={(e) => { setText(e.target.value); setSlashIndex(0); }} onKeyDown={onKey} onPaste={onPaste} aria-label="Auftrag an ASK" />
          <div className="input-tools">
            <button type="button" id="attach-btn" className="chip icon-only" title="Dateien oder Bilder anhängen (auch mit Ctrl+V oder Hineinziehen)"
              aria-label="Anhängen" onClick={() => fileInput.current?.click()}><Icon name="clip" size={15} /></button>
            <input ref={fileInput} id="attach-file" type="file" multiple hidden onChange={(e) => { addFiles(e.target.files); e.target.value = ''; }} />
            <button type="button" id="web-btn" className={`chip${state.webSearch && state.webSearchAvailable ? ' on' : ''}`} disabled={!state.webSearchAvailable}
              onClick={() => setWeb(!state.webSearch)}
              title={!state.webSearchAvailable ? 'Websuche ist in ASK für dein Konto nicht freigegeben' : state.webSearch ? 'Websuche an: ASK sucht vor der Antwort im Web (/web)' : 'Websuche aus (/web)'}>
              <Icon name="globe" size={14} /> Web
            </button>
            <ThinkPicker level={state.thinking || 'standard'} onChange={setThinking} />
            <span className="spacer" />
            <label className="model-pick" title="Modell in ASK">
              <select id="model-select" value={state.model || ''} disabled={!state.models?.length} onChange={(e) => setModel(e.target.value)}>
                {(state.models || []).map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
              </select>
              <Icon name="chevDown" size={13} />
            </label>
            <button type="button" id="send-btn" className={`send${running && !answering ? ' stop' : ''}`} disabled={sendDisabled}
              onClick={submit} title={answering ? 'Antwort senden (Enter)' : running ? 'Stoppen (Esc)' : 'Senden (Enter)'}
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
