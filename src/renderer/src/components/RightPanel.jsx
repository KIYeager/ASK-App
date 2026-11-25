import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { baseName, extOf, fileBadge, formatSize, formatTime } from '../lib/format.js';

const norm = (p) => String(p || '').replaceAll('\\', '/').replace(/^\.?\//, '').toLowerCase();
const MARK_TEXT = { neu: 'neu', geaendert: 'geändert', geloescht: 'gelöscht' };

// ---------- Dateibaum ----------

function TreeNode({ entry, depth, marks, expanded, toggle, children, selected, onSelect, onOpen }) {
  const mark = marks.get(norm(entry.path));
  const isOpen = expanded.has(entry.path);
  return (
    <>
      <div className={`tree-row${selected === entry.path ? ' sel' : ''}`} style={{ paddingLeft: 8 + depth * 16 }} role="treeitem"
        aria-expanded={entry.dir ? isOpen : undefined} aria-selected={selected === entry.path} tabIndex={0}
        title={`${entry.path}${entry.dir ? '' : ` · ${formatSize(entry.size)} · ${formatTime(entry.modified)}`}`}
        onClick={() => (entry.dir ? toggle(entry.path) : onSelect(entry))}
        onDoubleClick={() => { if (!entry.dir) onOpen(entry.path); }}
        onKeyDown={(e) => { if (e.key === 'Enter') { if (entry.dir) toggle(entry.path); else onOpen(entry.path); } }}>
        {entry.dir ? <Icon name={isOpen ? 'chevDown' : 'chevRight'} size={13} className="tw" /> : <span className="tw" />}
        <Icon name={entry.dir ? (isOpen ? 'folderOpen' : 'folder') : 'file'} size={15} className={`ti${entry.dir ? ' dir' : ''}`} />
        <span className="tn">{entry.name}</span>
        {mark ? <span className={`mark ${mark}`}>{MARK_TEXT[mark]}</span> : null}
      </div>
      {entry.dir && isOpen ? children : null}
    </>
  );
}

function FileTree({ root, files, marks, removed, selected, onSelect, onOpen, onShowFolder }) {
  const [dirs, setDirs] = useState({}); // rel → entries
  const [expanded, setExpanded] = useState(() => new Set());
  const load = useCallback(async (rel) => {
    const res = await api().listDir(rel);
    setDirs((d) => ({ ...d, [rel]: res?.entries || [] }));
  }, []);
  // Neu laden, wenn sich im Ordner etwas ändert (files kommt vom Hauptprozess) oder das Projekt wechselt
  useEffect(() => { setDirs({}); setExpanded(new Set()); }, [root]);
  useEffect(() => {
    if (!root) return;
    load('');
    for (const rel of expanded) load(rel);
  }, [root, files, load]); // eslint-disable-line react-hooks/exhaustive-deps
  const toggle = (rel) => {
    const next = new Set(expanded);
    if (next.has(rel)) next.delete(rel);
    else { next.add(rel); load(rel); }
    setExpanded(next);
  };
  const render = (rel, depth) => (dirs[rel] || []).map((e) => (
    <TreeNode key={e.path} entry={e} depth={depth} marks={marks} expanded={expanded} toggle={toggle} selected={selected} onSelect={onSelect} onOpen={onOpen}>
      {render(e.path, depth + 1)}
    </TreeNode>
  ));
  if (!root) return <div className="panel-empty">Kein Projekt geöffnet.</div>;
  const top = dirs[''];
  return (
    <div className="tree" role="tree" aria-label="Dateien im Projekt">
      <div className="tree-root" title={root}>
        <Icon name="folder" size={15} /><b>{baseName(root)}</b>
        <button type="button" className="icon-btn" title="Im Explorer öffnen (/explorer)" onClick={onShowFolder}><Icon name="folderOpen" size={14} /></button>
      </div>
      {top && !top.length ? <div className="panel-empty">Der Ordner ist noch leer. Lege die Dateien hinein, mit denen ASK arbeiten soll, oder ziehe sie in den Chat.</div> : null}
      {render('', 0)}
      {removed.length ? (
        <div className="tree-removed">
          {removed.map((p) => (
            <div key={p} className="tree-row gone" style={{ paddingLeft: 8 }} title={`${p} (gelöscht, «Rückgängig» holt sie zurück)`}>
              <span className="tw" /><Icon name="file" size={15} className="ti" /><span className="tn">{p}</span><span className="mark geloescht">gelöscht</span>
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

// ---------- Vorschau ----------

export function PreviewBody({ entry, big = false }) {
  const [p, setP] = useState(null);
  const [status, setStatus] = useState('');
  const [pdf, setPdf] = useState(null);
  useEffect(() => {
    let alive = true;
    setP(null); setPdf(null); setStatus('');
    if (entry) api().preview(entry.path).then((r) => { if (alive) setP(r); });
    return () => { alive = false; };
  }, [entry?.path, entry?.modified]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!entry) return <div className="panel-empty">Klicke im Dateibaum oder im Chat auf eine Datei, um sie hier anzusehen.</div>;
  if (!p) return <div className="panel-empty">Lädt …</div>;
  const v = Math.round(entry.modified || 0);
  if (pdf) return <iframe className={`preview-frame${big ? ' big' : ''}`} src={`${pdf}?t=${Date.now()}`} title={entry.name} />;
  if (p.kind === 'image') return <div className="preview-image"><img src={`${p.url}${p.url.includes('?') ? '&' : '?'}t=${v}`} alt={entry.name} /></div>;
  if (p.kind === 'pdf') return <iframe className={`preview-frame${big ? ' big' : ''}`} src={`${p.url}?t=${v}`} title={entry.name} />;
  if (p.kind === 'text') return <pre className="preview-text">{p.text + (p.truncated ? '\n\n… (gekürzt)' : '')}</pre>;
  if (p.kind === 'missing') return <div className="panel-empty">Die Datei ist nicht mehr vorhanden.</div>;
  const office = p.kind === 'office';
  return (
    <div className="preview-office">
      <span className={`badge big ext-${extOf(entry.name)}`}>{fileBadge(entry.name)}</span>
      <p>{office ? 'Office-Dateien öffnest du in Word, Excel oder PowerPoint.' : 'Für diese Datei gibt es keine Vorschau.'}</p>
      <div className="btns">
        <button type="button" className="btn primary" onClick={() => api().openFile(entry.path)}>Öffnen</button>
        {office && /\.(docx|xlsx|pptx)$/i.test(entry.path) ? (
          <button type="button" className="btn" onClick={async () => {
            setStatus('Wird mit Office in PDF umgewandelt …');
            const res = await api().toPdf(entry.path);
            if (!res.ok) { setStatus(res.fehler || 'Umwandeln nicht möglich.'); return; }
            const r = await api().preview(res.path);
            setPdf(r.url);
          }}>Als PDF anzeigen</button>
        ) : null}
        <button type="button" className="btn" onClick={() => api().showFile(entry.path)}>Im Explorer zeigen</button>
      </div>
      {status ? <p className="muted small">{status}</p> : null}
    </div>
  );
}

// ---------- Browser ----------

function BrowserTab({ browser, active, onClose, onBig, big }) {
  const host = useRef(null);
  const [url, setUrl] = useState('');
  const editing = useRef(false);
  useEffect(() => { if (!editing.current) setUrl(browser.url === 'about:blank' ? '' : browser.url || ''); }, [browser.url]);
  // Grösse und Sichtbarkeit der echten Seite (WebContentsView im Hauptprozess) melden
  const report = useCallback(() => {
    const el = host.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    api().setBrowserBounds({ x: r.x, y: r.y, width: r.width, height: r.height });
    api().setBrowserVisible(active && browser.mode !== 'unsichtbar');
  }, [active, browser.mode]);
  useLayoutEffect(() => {
    report();
    const ro = new ResizeObserver(report);
    if (host.current) ro.observe(host.current);
    window.addEventListener('resize', report);
    const t = setTimeout(report, 250); // nach Übergängen
    return () => { ro.disconnect(); window.removeEventListener('resize', report); clearTimeout(t); };
  }, [report, big]);
  useEffect(() => () => api().setBrowserVisible(false), []);
  return (
    <section id="browser-pane" className={`browser-pane${browser.running ? ' running' : ''}${browser.loading ? ' loading' : ''}`} data-mode={browser.mode || ''}>
      <div className="bp-head">
        <button id="bp-back" type="button" className="icon-btn" title="Zurück" aria-label="Zurück" disabled={!browser.back} onClick={() => api().browserBack()}><Icon name="arrowLeft" size={15} /></button>
        <button id="bp-forward" type="button" className="icon-btn" title="Vorwärts" aria-label="Vorwärts" disabled={!browser.forward} onClick={() => api().browserForward()}><Icon name="arrowRight" size={15} /></button>
        <button id="bp-reload" type="button" className="icon-btn" title="Neu laden" aria-label="Neu laden" onClick={() => api().browserReload()}><Icon name="reload" size={15} /></button>
        <form id="bp-form" className="bp-form" onSubmit={(e) => { e.preventDefault(); const t = url.trim(); if (!t) return; e.target.querySelector('input').blur(); api().browserGo(t); }}>
          <input id="bp-url" className="bp-url" spellCheck={false} autoComplete="off" placeholder="Adresse oder Suchbegriffe" aria-label="Adresse"
            value={url} onChange={(e) => setUrl(e.target.value)} onFocus={(e) => { editing.current = true; e.target.select(); }} onBlur={() => { editing.current = false; }} />
        </form>
        <button id="bp-big" type="button" className="icon-btn" title="Gross" aria-label="Gross" onClick={onBig}><Icon name="expand" size={15} /></button>
        <button id="bp-close" type="button" className="icon-btn" title="Browser schliessen" aria-label="Browser schliessen" onClick={onClose}><Icon name="close" size={15} /></button>
      </div>
      {browser.status ? <div className="bp-status-row"><span className="live-dot" aria-hidden="true" /><span id="bp-status" className="bp-status">{browser.status}</span></div> : null}
      <div id="browser-host" ref={host} className="browser-host">
        {browser.mode === 'unsichtbar' && browser.frame ? <img id="bp-frame" className="bp-frame" alt="" src={browser.frame} /> : null}
        {!browser.url && !browser.running ? <div className="panel-empty">Gib oben eine Adresse ein. ASK öffnet hier auch Webseiten und Spiele, die es für dich testet.</div> : null}
      </div>
      {browser.log.length ? <ul id="bp-log" className="live-log">{browser.log.map((l, i) => <li key={i} className={l.level === 'error' ? 'error' : 'warning'}>{l.text}</li>)}</ul> : null}
      <div className="bp-hint muted small">Du kannst selbst klicken, tippen und dich anmelden. Wichtige Konten (E-Banking) hier besser nicht.{' '}
        <button id="bp-clear" type="button" className="link" onClick={() => api().browserClear()}>Anmeldungen löschen</button></div>
    </section>
  );
}

// ---------- Panel ----------

export default function RightPanel({ tab, setTab, root, files, marks, removed, selected, onSelect, onBig, browser, browserActive, onBrowserClose, onBrowserBig, browserBig }) {
  const entry = selected ? (files.recent.find((f) => f.path === selected.path) || selected) : null;
  return (
    <aside id="sidebar" className="sidebar">
      <div className="rt" role="tablist">
        <button type="button" role="tab" aria-selected={tab === 'dateien'} className={tab === 'dateien' ? 'on' : ''} onClick={() => setTab('dateien')}><Icon name="folder" size={15} /> Dateien</button>
        <button type="button" role="tab" aria-selected={tab === 'vorschau'} className={tab === 'vorschau' ? 'on' : ''} onClick={() => setTab('vorschau')}><Icon name="eye" size={15} /> Vorschau</button>
        <button type="button" role="tab" aria-selected={tab === 'browser'} className={tab === 'browser' ? 'on' : ''} onClick={() => setTab('browser')}>
          <Icon name="browser" size={15} /> Browser{browser.running ? <span className="live-dot" aria-hidden="true" /> : null}
        </button>
      </div>
      <div className="rp-body">
        {tab === 'dateien' ? (
          <FileTree root={root} files={files} marks={marks} removed={removed} selected={selected?.path}
            onSelect={(e) => { onSelect(e); setTab('vorschau'); }} onOpen={(p) => api().openFile(p)} onShowFolder={() => api().showFolder()} />
        ) : null}
        {tab === 'vorschau' ? (
          <div className="preview">
            {entry ? (
              <div className="preview-head">
                <Icon name="file" size={15} /><b className="ell" title={entry.path}>{baseName(entry.path)}</b>
                <span className="spacer" />
                <button type="button" className="icon-btn" title="Gross anzeigen" aria-label="Gross anzeigen" onClick={onBig}><Icon name="expand" size={14} /></button>
                <button type="button" className="btn small" onClick={() => api().openFile(entry.path)}>Öffnen</button>
                <button type="button" className="icon-btn" title="Vorschau schliessen" aria-label="Vorschau schliessen" onClick={() => onSelect(null)}><Icon name="close" size={14} /></button>
              </div>
            ) : null}
            <div id="preview-body" className="preview-body"><PreviewBody entry={entry} /></div>
          </div>
        ) : null}
        {tab === 'browser' ? <BrowserTab browser={browser} active={browserActive} onClose={onBrowserClose} onBig={onBrowserBig} big={browserBig} /> : null}
      </div>
    </aside>
  );
}

export { norm };
