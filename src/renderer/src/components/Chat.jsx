import { memo, useState } from 'react';
import Icon, { TOOL_ICONS } from './Icon.jsx';
import Markdown from './Markdown.jsx';
import { groupTitle, statsText } from '../lib/chatModel.mjs';
import { baseName, extOf, fileBadge, fileUrl } from '../lib/format.js';

// ---------- Eingabe der Person ----------

function Attachments({ list, ctx }) {
  if (!list?.length) return null;
  return (
    <div className="attachments">
      {list.map((a) => (a.image
        ? <img key={a.path} className="attach-thumb" src={fileUrl(a.path)} alt={a.name} title={a.path} onClick={() => ctx.openFile(a.path)} />
        : (
          <button key={a.path} type="button" className="attach-chip" title={a.path} onClick={() => ctx.openFile(a.path)}>
            <span className={`badge ext-${extOf(a.name)}`}>{fileBadge(a.name)}</span><span className="name">{a.name}</span>
          </button>
        )))}
    </div>
  );
}

function UserMessage({ entry, ctx, editable }) {
  return (
    <div className="msg user">
      <div className="bubble">
        {entry.text ? <div className="text">{entry.text}</div> : null}
        <Attachments list={entry.attachments} ctx={ctx} />
      </div>
      {editable ? (
        // Nur die letzte Eingabe, nur ohne laufenden Auftrag (ENTSCHEIDE E36)
        <div className="user-actions">
          <button type="button" className="ghost-btn user-edit" title="Die Nachricht zurücknehmen und im Eingabefeld ändern" onClick={() => ctx.editLast(false)}>
            <Icon name="pen" size={13} /> Bearbeiten
          </button>
          <button type="button" className="ghost-btn user-resend" title="Die Nachricht zurücknehmen und gleich nochmals senden" disabled={!!ctx.blocker} onClick={() => ctx.editLast(true)}>
            <Icon name="reload" size={13} /> Neu senden
          </button>
        </div>
      ) : null}
    </div>
  );
}

// ---------- Arbeitsschritte ----------

function Detail({ entry, hidden }) {
  const out = String(entry.progress || entry.output || '').replace(/\s+$/, '');
  return (
    <div className="details" hidden={hidden}>
      {entry.detail && entry.name === 'run_command' ? <pre className="cmd">{entry.detail}</pre> : null}
      {entry.tasks?.length ? (
        <ul className="todo">
          {entry.tasks.map((t, i) => <li key={i} className={t.status}>{{ erledigt: '☒', in_arbeit: '▸', offen: '☐' }[t.status] || '☐'} {t.titel}</li>)}
        </ul>
      ) : out ? (
        <pre className="out">
          {entry.name === 'edit_file'
            ? out.split('\n').map((l, i) => <span key={i} className={l.startsWith('+ ') ? 'add' : l.startsWith('- ') ? 'del' : ''}>{l}{'\n'}</span>)
            : out}
        </pre>
      ) : null}
    </div>
  );
}

function Shots({ images, big, ctx }) {
  if (!images?.length) return null;
  return (
    <div className={big ? 'shots big' : 'shots'}>
      {images.map((p) => (
        <img key={p} src={fileUrl(p, big ? String(p.length) : '')} alt={big ? baseName(p) : ''} title={`${p} · klicken für die Vorschau`}
          onClick={(e) => { e.stopPropagation(); ctx.openFile(p); }} onLoad={ctx.onMediaLoad} />
      ))}
    </div>
  );
}

const ToolRow = memo(function ToolRow({ entry, ctx }) {
  const [open, setOpen] = useState(false);
  const expanded = open || ctx.verbose;
  const state = entry.ok === null ? 'running' : entry.ok === false ? 'bad' : 'ok';
  const summary = entry.ok === null ? (lastLine(entry.progress) || 'läuft …') : entry.summary;
  return (
    <div className={`msg tool ${state}${expanded ? ' expanded' : ''}`}>
      <button type="button" className="call" onClick={() => { if (!window.getSelection()?.toString()) setOpen(!open); }} title="Klicken für Details (Ctrl+O: alle)">
        <span className="ic"><Icon name={TOOL_ICONS[entry.name] || 'spark'} /></span>
        <span className="name">{entry.label}</span>
        {entry.detail ? <span className="arg">{entry.detail}</span> : null}
        <span className="result">{state === 'running' ? <span className="spin" aria-hidden="true" /> : null}<span className="res">{summary}</span></span>
      </button>
      <Shots images={entry.images} big={entry.name === 'bild_erstellen'} ctx={ctx} />
      <Detail entry={entry} hidden={!expanded} />
    </div>
  );
});

function lastLine(text) {
  return String(text || '').split('\n').map((l) => l.trim()).filter(Boolean).pop() || '';
}

function GroupRow({ step, ctx }) {
  const [open, setOpen] = useState(false);
  const entries = step.entries;
  const running = entries.some((e) => e.ok === null);
  const fails = entries.filter((e) => e.ok === false);
  const expanded = open || ctx.verbose;
  return (
    <div className={`msg tool group ${running ? 'running' : fails.length ? 'bad' : 'ok'}${expanded ? ' expanded' : ''}`}>
      <button type="button" className="call" onClick={() => setOpen(!open)}>
        <span className="ic"><Icon name="read" /></span>
        <span className="name">{groupTitle(entries)}</span>
        <span className="arg">{entries.map((e) => e.detail).filter(Boolean).join(', ')}</span>
        <span className="result">{running ? <span className="spin" aria-hidden="true" /> : null}
          <span className="res">{running ? 'läuft …' : fails.length ? `${fails.length} fehlgeschlagen` : ''}</span></span>
      </button>
      {(
        <div className="details" hidden={!expanded}>
          {entries.map((e) => (
            <div key={e.key} className={`sub${e.ok === false ? ' bad' : ''}`}>
              <span className="name">{e.label}</span> <span className="arg">{e.detail}</span>
              <span className="subsum"> · {e.ok === null ? 'läuft …' : e.summary}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function Thought({ entry, ctx }) {
  return (
    <div className="msg thought">
      {entry.reasoning ? <Reasoning text={entry.reasoning} verbose={ctx.verbose} /> : null}
      {String(entry.text || '').trim() ? <Markdown text={entry.text} isFile={ctx.isFile} onFile={ctx.openFileByName} onLink={ctx.openLink} onCopy={ctx.copy} /> : null}
    </div>
  );
}

function Reasoning({ text, verbose }) {
  return (
    <details className="thinking" open={verbose || undefined}>
      <summary><Icon name="spark" size={13} /> Gedanken</summary>
      <pre>{text}</pre>
    </details>
  );
}

function WorkCard({ turn, ctx }) {
  const [userOpen, setUserOpen] = useState(null);
  // Eingeklappt ab vier Schritten, sobald der Auftrag fertig ist; Ctrl+O klappt alles auf
  const auto = turn.running || turn.toolCount < 4 || turn.pending || ctx.verbose;
  const open = userOpen ?? auto;
  const parts = [];
  if (turn.toolCount) parts.push(`${turn.toolCount} ${turn.toolCount === 1 ? 'Arbeitsschritt' : 'Arbeitsschritte'}`);
  else parts.push('Überlegungen');
  if (turn.failed) parts.push(`${turn.failed} fehlgeschlagen`);
  return (
    <div className={`work${open ? ' open' : ''}${turn.running ? ' running' : ''}`}>
      <button type="button" className="work-head msg fold" onClick={() => setUserOpen(!open)} aria-expanded={open}>
        <span className={`state ${turn.running ? 'run' : turn.failed ? 'bad' : 'ok'}`}>
          {turn.running ? <span className="spin" aria-hidden="true" /> : <Icon name={turn.failed ? 'alert' : 'check'} size={15} />}
        </span>
        <span className="title">{turn.running ? 'Arbeitet' : parts[0]}</span>
        {turn.running ? <span className="muted">{parts[0]}</span> : parts.slice(1).map((p) => <span key={p} className="muted">· {p}</span>)}
        <span className="chev"><Icon name={open ? 'chevDown' : 'chevRight'} size={15} /></span>
      </button>
      <div className="work-body" hidden={!open}>
          {turn.steps.map((s) => {
            if (s.type === 'tool') return <ToolRow key={s.entry.key} entry={s.entry} ctx={ctx} />;
            if (s.type === 'group') return <GroupRow key={s.key} step={s} ctx={ctx} />;
            if (s.type === 'approval') return <Approval key={s.entry.key} entry={s.entry} ctx={ctx} />;
            if (s.type === 'question') return <Question key={s.entry.key} entry={s.entry} ctx={ctx} />;
            if (s.type === 'auto') return <div key={s.entry.key} className="msg notice auto"><Icon name="reload" size={13} /> Weiter am Ziel …</div>;
            return <Thought key={s.entry.key} entry={s.entry} ctx={ctx} />;
          })}
      </div>
      {/* Eingeklappt: erstellte Bilder bleiben sichtbar */}
      {!open ? <CollapsedImages turn={turn} ctx={ctx} /> : null}
    </div>
  );
}

function CollapsedImages({ turn, ctx }) {
  const big = turn.steps.filter((s) => s.type === 'tool' && s.entry.name === 'bild_erstellen' && s.entry.images?.length);
  if (!big.length) return null;
  return <div className="work-images">{big.map((s) => <Shots key={s.entry.key} images={s.entry.images} big ctx={ctx} />)}</div>;
}

// ---------- Antwort und was danach kommt ----------

function Answer({ entry, streaming, ctx }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className={`msg assistant${streaming ? ' streaming' : ''}`}>
      {entry.reasoning ? <Reasoning text={entry.reasoning} verbose={ctx.verbose} /> : null}
      <Markdown text={entry.text} isFile={ctx.isFile} onFile={ctx.openFileByName} onLink={ctx.openLink} onCopy={ctx.copy} />
      {!streaming ? (
        <div className="msg-actions">
          <button type="button" className="copy ghost-btn" data-text={entry.text}
            onClick={() => { ctx.copy(entry.text); setCopied(true); setTimeout(() => setCopied(false), 1500); }}>
            <Icon name={copied ? 'check' : 'copy'} size={14} /> {copied ? 'Kopiert' : 'Kopieren'}
          </button>
        </div>
      ) : null}
    </div>
  );
}

function Sources({ entry, ctx }) {
  const host = (url) => { try { return new URL(url).hostname.replace(/^www\./, ''); } catch { return ''; } };
  return (
    <div className="msg sources">
      <div className="label"><Icon name="globe" size={14} /> {entry.sources.length} {entry.sources.length === 1 ? 'Quelle' : 'Quellen'}</div>
      <ol>
        {entry.sources.slice(0, 10).map((s) => (
          <li key={s.url}><a href={s.url} title={s.url} onClick={(e) => { e.preventDefault(); ctx.openLink(s.url); }}>{s.title || host(s.url)}</a>
            <span className="host"> · {host(s.url)}</span></li>
        ))}
      </ol>
    </div>
  );
}

function Changes({ entry, ctx }) {
  const c = entry.changes;
  const [changed, added, removed] = c.counts || [c.changed.length, c.added.length, c.removed.length];
  const parts = [];
  if (added) parts.push(`${added} neu`);
  if (changed) parts.push(`${changed} geändert`);
  if (removed) parts.push(`${removed} gelöscht`);
  const names = [...c.added, ...c.changed, ...c.removed];
  const title = [...c.changed.map((f) => `geändert: ${f}`), ...c.added.map((f) => `neu: ${f}`), ...c.removed.map((f) => `gelöscht: ${f}`)].join('\n');
  // Rückgängig geht bei jedem Auftrag mit Sicherung; bei früheren setzt es auch alles danach zurück
  const latest = ctx.undoSnapshot === entry.snapshot;
  const canUndo = !entry.undone && !!entry.snapshot && !ctx.running;
  return (
    <div className={`msg changes${entry.undone ? ' undone' : ''}${latest ? ' latest' : ''}`} title={title}>
      <Icon name="file" size={14} />
      <span className="sum">Dateien: {parts.join(', ')}</span>
      <span className="names">
        {names.slice(0, 3).map((n, i) => <span key={n}>{i ? ', ' : ''}<button type="button" className="fname" onClick={() => ctx.openFile(n)}>{baseName(n)}</button></span>)}
        {names.length > 3 ? ` und ${names.length - 3} weitere` : ''}
      </span>
      {entry.undone ? <span className="muted undone-label">rückgängig gemacht</span> : null}
      <button type="button" className="undo-btn ghost-btn" hidden={!canUndo} title={latest ? 'Ordner auf den Stand vor diesem Auftrag zurücksetzen' : 'Ordner auf den Stand vor diesem Auftrag zurücksetzen, mit allem danach'}
        onClick={() => ctx.undo(entry.snapshot)}>
        <Icon name="undo" size={14} /> Rückgängig
      </button>
    </div>
  );
}

function Stats({ entry }) {
  const s = statsText(entry.list);
  if (!s) return null;
  return <div className="msg stats" title={s.title}><Icon name="spark" size={12} /> {s.text}</div>;
}

const ANSWER_TEXT = { yes: 'Erlaubt', always: 'Erlaubt für diesen Chat', no: 'Abgelehnt' };

function Approval({ entry, ctx }) {
  if (entry.answer) {
    return (
      <div className={`msg approval done ${entry.answer === 'no' ? 'no' : 'yes'}`}>
        <Icon name={entry.answer === 'no' ? 'close' : 'check'} size={14} />
        <span>{ANSWER_TEXT[entry.answer]}:</span> <span className="arg">{entry.detail || entry.title}</span>
      </div>
    );
  }
  return (
    <div className="msg approval open" role="group" aria-label={entry.title}>
      <span className="ic"><Icon name="alert" size={18} /></span>
      <div className="content">
        <div className="title">{entry.title}</div>
        {entry.detail ? <code className="cmdline">{entry.detail}</code> : null}
        {entry.reason ? <div className="reason">{entry.reason}</div> : null}
        {entry.reviewed ? <div className="reviewed"><Icon name="shield" size={12} /> Von ASK geprüft</div> : null}
        <div className="btns">
          <button type="button" className="btn primary" onClick={() => ctx.approve(entry.id, 'yes')}>Erlauben</button>
          <button type="button" className="btn" onClick={() => ctx.approve(entry.id, 'always')}>Immer in diesem Chat</button>
          <button type="button" className="btn" onClick={() => ctx.approve(entry.id, 'no')}>Ablehnen</button>
        </div>
      </div>
    </div>
  );
}

function Question({ entry, ctx }) {
  const q = entry.question;
  const [picked, setPicked] = useState([]);
  const [text, setText] = useState('');
  const answer = q.answer ?? (entry.ok !== null ? entry.summary : '');
  if (!q.open) {
    return (
      <div className="msg question done">
        <span className="ic"><Icon name="help" size={16} /></span>
        <div className="content">
          <div className="q">{q.frage}</div>
          <div className="a">{answer && entry.ok !== false ? <><Icon name="check" size={13} /> {answer}</> : <span className="muted">nicht beantwortet</span>}</div>
        </div>
      </div>
    );
  }
  const send = (value) => { if (String(value).trim()) ctx.answer(entry.id, String(value).trim()); };
  return (
    <div className="msg question open" role="group" aria-label="Frage von ASK">
      <span className="ic"><Icon name="help" size={18} /></span>
      <div className="content">
        <div className="eyebrow">ASK fragt</div>
        <div className="q">{q.frage}</div>
        {q.optionen.length ? (
          <div className="options">
            {q.optionen.map((o) => (q.mehrfach ? (
              <label key={o} className={`option check${picked.includes(o) ? ' on' : ''}`}>
                <input type="checkbox" checked={picked.includes(o)} onChange={() => setPicked(picked.includes(o) ? picked.filter((x) => x !== o) : [...picked, o])} />
                {o}
              </label>
            ) : (
              <button key={o} type="button" className="option" onClick={() => send(o)}>{o}</button>
            )))}
          </div>
        ) : null}
        <form className="free" onSubmit={(e) => { e.preventDefault(); send(q.mehrfach && picked.length ? [...picked, text].filter(Boolean).join(', ') : text); }}>
          <input value={text} onChange={(e) => setText(e.target.value)} placeholder={q.optionen.length ? 'Andere Antwort …' : 'Deine Antwort …'} aria-label="Eigene Antwort" />
          <button type="submit" className="btn primary" disabled={!text.trim() && !(q.mehrfach && picked.length)}>Antworten</button>
        </form>
      </div>
    </div>
  );
}

// Plan aus dem Planmodus: lesen, dann umsetzen (wechselt auf Auto) oder im Eingabefeld ändern lassen
function Plan({ entry, ctx }) {
  const isLast = ctx.lastPlanKey === entry.key;
  return (
    <div className="msg plan" role="group" aria-label="Plan">
      <div className="plan-head"><Icon name="map" size={15} /> <span>Plan</span></div>
      <div className="plan-body"><Markdown text={entry.plan} isFile={ctx.isFile} onFile={ctx.openFileByName} onLink={ctx.openLink} onCopy={ctx.copy} /></div>
      {isLast && !ctx.running ? (
        <div className="btns">
          <button type="button" className="btn primary" onClick={() => ctx.implementPlan()}>Umsetzen</button>
          <button type="button" className="btn" onClick={() => ctx.revisePlan()}>Ändern</button>
        </div>
      ) : null}
    </div>
  );
}

function After({ entry, ctx }) {
  switch (entry.k) {
    case 'sources': return <Sources entry={entry} ctx={ctx} />;
    case 'changes': return <Changes entry={entry} ctx={ctx} />;
    case 'stats': return <Stats entry={entry} />;
    case 'approval': return <Approval entry={entry} ctx={ctx} />;
    case 'error': return <div className="msg error"><Icon name="alert" size={14} /> <span>{entry.text}</span></div>;
    case 'notice': return <div className={`msg notice${entry.retry ? ' retry' : ''}`}><Icon name={entry.icon || 'chevRight'} size={13} /> <span>{entry.text}</span></div>;
    case 'tool': return entry.plan ? <Plan entry={entry} ctx={ctx} /> : entry.question ? <Question entry={entry} ctx={ctx} /> : <ToolRow entry={entry} ctx={ctx} />;
    default: return null;
  }
}

export const Turn = memo(function Turn({ turn, ctx, last }) {
  const streaming = turn.running && last && turn.answer && !turn.answer.done;
  return (
    <section className="turn">
      {turn.user ? <UserMessage entry={turn.user} ctx={ctx} editable={last && !ctx.running} /> : null}
      {turn.steps.length ? <WorkCard turn={turn} ctx={ctx} /> : null}
      {turn.running && !turn.steps.length && !turn.answer ? (
        <div className="work running"><div className="work-head"><span className="state run"><span className="spin" aria-hidden="true" /></span><span className="title">{ctx.activity}</span></div></div>
      ) : null}
      {turn.answer ? <Answer entry={turn.answer} streaming={streaming} ctx={ctx} /> : null}
      {turn.after.map((e) => <After key={e.key} entry={e} ctx={ctx} />)}
    </section>
  );
});
