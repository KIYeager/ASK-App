import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import GithubPane from './GithubPane.jsx';
import DocsPane from './DocsPane.jsx';
import { api } from '../api.js';
import ServerBar from './Servers.jsx';
import { baseName, extOf, fileBadge, fileUrl, formatSize } from '../lib/format.js';

const norm = (p) => String(p || '').replaceAll('\\', '/').replace(/^\.?\//, '').toLowerCase();
const MARK_TEXT = { neu: 'neu', geaendert: 'geändert', geloescht: 'gelöscht' };

// ---------- Dateien: wie der Explorer (Ordner betreten, über den Pfad zurück) ----------

const SORTS = { name: 'Name', modified: 'Geändert', size: 'Grösse' };

function when(ms) {
  const d = new Date(ms);
  if (d.toDateString() === new Date().toDateString()) return d.toLocaleTimeString('de-CH', { hour: '2-digit', minute: '2-digit' });
  return d.toLocaleDateString('de-CH', { day: '2-digit', month: '2-digit' });
}

function FileMenu({ menu, onClose, onOpen, onPreview, onMention, onShow }) {
  const ref = useRef(null);
  useEffect(() => {
    ref.current?.querySelector('button')?.focus();
    const away = (e) => { if (!ref.current?.contains(e.target)) onClose(); };
    const key = (e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } };
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('mousedown', away); document.removeEventListener('keydown', key, true); };
  }, [onClose]);
  const item = (label, fn) => <button type="button" role="menuitem" onClick={() => { onClose(); fn(); }}>{label}</button>;
  const left = Math.max(4, Math.min(menu.x, window.innerWidth - 200));
  const top = Math.max(4, Math.min(menu.y, window.innerHeight - 170));
  return (
    <div ref={ref} className="chat-menu popover" role="menu" aria-label={menu.entry.name} style={{ left, top }}>
      {item('Öffnen', onOpen)}
      {!menu.entry.dir ? item('Vorschau', onPreview) : null}
      {onMention ? item('Im Chat erwähnen', onMention) : null}
      {item('Im Explorer zeigen', onShow)}
    </div>
  );
}

function FileExplorer({ root, free, files, marks, removed, selected, onSelect, onPreview, onMention, notice }) {
  const rootName = free ? 'Ohne Projekt' : baseName(root);
  const [dir, setDir] = useState('');
  const [entries, setEntries] = useState(null);
  const [sort, setSort] = useState({ key: 'name', asc: true });
  const [query, setQuery] = useState('');
  const [results, setResults] = useState(null);
  const [menu, setMenu] = useState(null);
  const [dropping, setDropping] = useState(false);
  const load = useCallback(async (rel) => {
    const res = await api().listDir(rel);
    if (!res?.ok && rel) { setDir(''); return; } // Ordner gibt es nicht mehr
    setEntries(res?.entries || []);
  }, []);
  useEffect(() => { setDir(''); setQuery(''); setEntries(null); }, [root]);
  useEffect(() => { if (root) load(dir); }, [root, dir, files, load]);
  useEffect(() => {
    const q = query.trim();
    if (!q) { setResults(null); return undefined; }
    const t = setTimeout(async () => setResults(await api().findFiles(q)), 180);
    return () => clearTimeout(t);
  }, [query, files]);

  if (!root) return <div className="panel-empty">Kein Projekt geöffnet.</div>;
  const parts = dir ? dir.split(/[\\/]/) : [];
  const go = (rel) => { setDir(rel); setQuery(''); };
  const up = () => go(parts.slice(0, -1).join('\\'));
  const list = results || entries || [];
  const sorted = results ? list : [...list].sort((a, b) => {
    if (a.dir !== b.dir) return b.dir - a.dir;
    const k = sort.key;
    const v = k === 'name' ? a.name.localeCompare(b.name, 'de', { numeric: true }) : (a[k] || 0) - (b[k] || 0);
    return sort.asc ? v : -v;
  });
  const here = norm(dir);
  const gone = results ? [] : removed.filter((p) => { const n = norm(p); const i = n.lastIndexOf('/'); return (i < 0 ? '' : n.slice(0, i)) === here; });
  const open = (e) => (e.dir ? go(e.path) : api().openFile(e.path));
  const clickSort = (key) => setSort((s) => ({ key, asc: s.key === key ? !s.asc : key === 'name' }));

  // Dateien aus dem Explorer hineinziehen: in den offenen Ordner kopieren (nicht in den Chat)
  const onDrop = async (e) => {
    if (![...(e.dataTransfer?.types || [])].includes('Files')) return;
    e.preventDefault();
    e.stopPropagation();
    setDropping(false);
    const list2 = [];
    for (const f of [...e.dataTransfer.files]) {
      try { list2.push({ name: f.name, type: f.type, data: new Uint8Array(await f.arrayBuffer()) }); } catch { /* Ordner */ }
    }
    if (!list2.length) return;
    const res = await api().importFiles(dir, list2);
    if (!res?.ok) notice(res?.reason || 'Nicht kopiert.');
    else load(dir);
  };
  const onDragOver = (e) => {
    if (![...(e.dataTransfer?.types || [])].includes('Files')) return;
    e.preventDefault();
    e.stopPropagation();
    setDropping(true);
  };

  return (
    <div className={`explorer${dropping ? ' dropping' : ''}`} onDragOver={onDragOver} onDragLeave={(e) => { if (!e.currentTarget.contains(e.relatedTarget)) setDropping(false); }} onDrop={onDrop}>
      <div className="ex-search">
        <Icon name="search" size={14} />
        <input id="file-search" type="search" placeholder={free ? 'Dateien suchen' : 'Im Projekt suchen'} spellCheck={false} value={query} onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setQuery(''); } }} aria-label="Dateien im Projekt suchen" />
      </div>
      <div className="ex-crumbs">
        <button type="button" className="icon-btn" title="Eine Ebene hoch" aria-label="Eine Ebene hoch" disabled={!dir || !!results} onClick={up}><Icon name="arrowLeft" size={15} /></button>
        <nav className="crumbs" aria-label="Ordner">
          {results ? <span className="muted">{results.length} Treffer{free ? '' : ' im Projekt'}</span> : (
            <>
              <button type="button" className={`crumb-btn${!dir ? ' cur' : ''}`} onClick={() => go('')}>{rootName}</button>
              {parts.map((p, i) => (
                <span key={i} className="crumb-part"><Icon name="chevRight" size={12} />
                  <button type="button" className={`crumb-btn${i === parts.length - 1 ? ' cur' : ''}`} onClick={() => go(parts.slice(0, i + 1).join('\\'))}>{p}</button>
                </span>
              ))}
            </>
          )}
        </nav>
        <button type="button" className="icon-btn" title="Im Explorer öffnen" aria-label="Im Explorer öffnen" onClick={() => (dir ? api().showFile(dir) : api().showFolder())}><Icon name="folderOpen" size={15} /></button>
      </div>
      {!results ? (
        <div className="ex-head" role="row">
          {Object.entries(SORTS).map(([k, label]) => (
            <button key={k} type="button" className={`ex-col ${k}${sort.key === k ? ' on' : ''}`} onClick={() => clickSort(k)} aria-sort={sort.key === k ? (sort.asc ? 'ascending' : 'descending') : undefined}>
              {label}{sort.key === k ? <Icon name={sort.asc ? 'chevDown' : 'chevUp'} size={11} /> : null}
            </button>
          ))}
        </div>
      ) : null}
      <div className="ex-list" role="list">
        {entries === null && !results ? <div className="panel-empty">Lädt …</div> : null}
        {sorted.map((e) => {
          const mark = marks.get(norm(e.path));
          return (
            <div key={e.path} role="listitem" tabIndex={0} className={`ex-row${selected === e.path ? ' sel' : ''}${e.dir ? ' dir' : ''}`}
              title={e.path} onClick={() => (e.dir ? go(e.path) : onSelect(e))} onDoubleClick={() => { if (!e.dir) api().openFile(e.path); }}
              onKeyDown={(ev) => { if (ev.key === 'Enter') open(e); }}
              onContextMenu={(ev) => { ev.preventDefault(); setMenu({ entry: e, x: ev.clientX, y: ev.clientY }); }}>
              <span className="ex-name">
                {e.dir ? <Icon name="folder" size={16} className="ti dir" /> : <span className={`fi ext-${extOf(e.name)}`}>{fileBadge(e.name).slice(0, 3)}</span>}
                <span className="tn">{results ? e.path.replaceAll('\\', '/') : e.name}</span>
                {mark ? <span className={`mark ${mark}`}>{MARK_TEXT[mark]}</span> : null}
              </span>
              <span className="ex-c">{when(e.modified)}</span>
              <span className="ex-c">{e.dir ? '' : formatSize(e.size)}</span>
            </div>
          );
        })}
        {gone.map((p) => (
          <div key={`g-${p}`} className="ex-row gone" title={`${p} (gelöscht, «Rückgängig» holt sie zurück)`}>
            <span className="ex-name"><span className={`fi ext-${extOf(p)}`}>{fileBadge(p).slice(0, 3)}</span><span className="tn">{baseName(p)}</span><span className="mark geloescht">gelöscht</span></span>
            <span className="ex-c" /><span className="ex-c" />
          </div>
        ))}
        {entries && !results && !entries.length && !gone.length ? (
          <div className="panel-empty">{dir ? 'Der Ordner ist leer.' : free ? 'Hier erscheinen Bilder, die ASK in Chats ohne Projekt erstellt. Mit Dateien arbeitet ASK in einem Projekt.'
            : 'Das Projekt ist noch leer. Ziehe Dateien hierher, um sie hineinzukopieren.'}</div>
        ) : null}
        {results && !results.length ? <div className="panel-empty">Keine Datei mit diesem Namen.</div> : null}
      </div>
      {dropping ? <div className="ex-drop">In «{parts.length ? parts[parts.length - 1] : rootName}» kopieren</div> : null}
      {menu ? <FileMenu menu={menu} onClose={() => setMenu(null)} onOpen={() => open(menu.entry)} onPreview={() => onPreview(menu.entry)}
        onMention={free ? null : () => onMention(menu.entry.path)} onShow={() => api().showFile(menu.entry.path)} /> : null}
    </div>
  );
}

function Peek({ entry, onPreview, onMention, free }) {
  if (!entry) return null;
  const image = /\.(png|jpe?g|gif|webp|bmp)$/i.test(entry.path);
  return (
    <div className="peek">
      <div className="peek-head">
        <span className={`fi ext-${extOf(entry.name || entry.path)}`}>{fileBadge(entry.name || entry.path).slice(0, 3)}</span>
        <b className="ell" title={entry.path}>{baseName(entry.path)}</b>
        <span className="muted small">{entry.size ? formatSize(entry.size) : ''}</span>
      </div>
      {image ? <img className="peek-img" src={fileUrl(entry.path, Math.round(entry.modified || 0))} alt="" /> : null}
      <div className="peek-acts">
        <button type="button" className="btn small primary" onClick={() => api().openFile(entry.path)}>Öffnen</button>
        <button type="button" className="btn small" onClick={() => onPreview(entry)}>Vorschau</button>
        {free ? null : <button type="button" className="btn small" onClick={() => onMention(entry.path)}>Im Chat erwähnen</button>}
      </div>
    </div>
  );
}

// ---------- Vorschau ----------

export function PreviewBody({ entry, big = false }) {
  const [p, setP] = useState(null);
  const [status, setStatus] = useState('');
  const [pdf, setPdf] = useState(null);
  const shown = useRef(null); // Datei, die gerade gezeigt wird
  // Office-Datei als PDF zeigen (preview: ein aktuelles PDF von früher genügt). alive: Ergebnis einer anderen
  // Datei nicht mehr anzeigen, wenn inzwischen eine andere gewählt ist (ohne Angabe: dieselbe Datei wie beim Klick).
  const showPdf = async (path, preview, alive = () => shown.current === path) => {
    setStatus(preview ? 'Vorschau wird mit Office erstellt …' : 'Wird mit Office in PDF umgewandelt …');
    try {
      const res = await api().toPdf(path, preview);
      if (!alive()) return;
      if (!res.ok) { setStatus(res.fehler || 'Umwandeln nicht möglich.'); return; }
      const r = await api().preview(res.path);
      if (alive()) { setStatus(''); setPdf(r.url); }
    } catch (err) {
      if (alive()) setStatus(`Umwandeln nicht möglich: ${err?.message || err}`);
    }
  };
  useEffect(() => {
    let alive = true;
    shown.current = entry?.path ?? null;
    setP(null); setPdf(null); setStatus('');
    if (entry) {
      api().preview(entry.path).then((r) => {
        if (!alive) return;
        setP(r);
        if (r.kind === 'office' && r.auto && /\.(docx|xlsx|pptx)$/i.test(entry.path)) showPdf(entry.path, true, () => alive);
      }).catch(() => { if (alive) setP({ kind: 'error' }); });
    }
    return () => { alive = false; };
  }, [entry?.path, entry?.modified]);
  if (!entry) return <div className="panel-empty">Klicke im Dateibaum oder im Chat auf eine Datei, um sie hier anzusehen.</div>;
  if (!p) return <div className="panel-empty">Lädt …</div>;
  const v = Math.round(entry.modified || 0);
  if (pdf) return <iframe className={`preview-frame${big ? ' big' : ''}`} src={`${pdf}?t=${v}`} title={entry.name} />;
  if (p.kind === 'image') return <div className="preview-image"><img src={`${p.url}${p.url.includes('?') ? '&' : '?'}t=${v}`} alt={entry.name} /></div>;
  if (p.kind === 'pdf') return <iframe className={`preview-frame${big ? ' big' : ''}`} src={`${p.url}?t=${v}`} title={entry.name} />;
  if (p.kind === 'text') return <pre className="preview-text">{p.text + (p.truncated ? '\n\n… (gekürzt)' : '')}</pre>;
  if (p.kind === 'missing') return <div className="panel-empty">Die Datei ist nicht mehr vorhanden.</div>;
  if (p.kind === 'error') return <div className="panel-empty">Die Vorschau liess sich nicht laden.</div>;
  const office = p.kind === 'office';
  return (
    <div className="preview-office">
      <span className={`badge big ext-${extOf(entry.name)}`}>{fileBadge(entry.name)}</span>
      <p>{office ? 'Office-Dateien öffnest du in Word, Excel oder PowerPoint.' : 'Für diese Datei gibt es keine Vorschau.'}</p>
      <div className="btns">
        <button type="button" className="btn primary" onClick={() => api().openFile(entry.path)}>Öffnen</button>
        {office && /\.(docx|xlsx|pptx)$/i.test(entry.path) ? (
          <button type="button" className="btn" onClick={() => showPdf(entry.path, false)}>Als PDF anzeigen</button>
        ) : null}
        <button type="button" className="btn" onClick={() => api().showFile(entry.path)}>Im Explorer zeigen</button>
      </div>
      {status ? <p className="muted small">{status}</p> : null}
    </div>
  );
}

// ---------- Browser ----------

function BrowserTab({ browser, active, onClose, onBig, big, servers, root, notice }) {
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
        <button id="bp-big" type="button" className={`icon-btn${big ? ' on' : ''}`} title={big ? 'Wieder klein (Esc)' : 'Gross anzeigen'} aria-label={big ? 'Wieder klein' : 'Gross anzeigen'} aria-pressed={big} onClick={onBig}><Icon name="expand" size={15} /></button>
        <button id="bp-close" type="button" className="icon-btn" title="Browser schliessen" aria-label="Browser schliessen" onClick={onClose}><Icon name="close" size={15} /></button>
      </div>
      {root ? <ServerBar servers={servers} notice={notice} onOpen={(u) => api().browserGo(u)} /> : null}
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

// Bereich eines Mods: das Element bleibt bestehen (Zustand des Mods), es wird nur ein- und ausgehängt
function ModPanel({ el }) {
  const host = useRef(null);
  useEffect(() => {
    const box = host.current;
    box?.appendChild(el);
    return () => { if (el.parentNode === box) box.removeChild(el); };
  }, [el]);
  return <div ref={host} className="mod-panel-host" />;
}

// ---------- Panel ----------

export default function RightPanel({ tab, setTab, root, free, files, marks, removed, selected, onSelect, onBig, browser, browserActive, onBrowserClose, onBrowserBig, browserBig, onMention, notice,
  servers = [], running, dialogOpen, onGithub, modPanels = [], docs = {} }) {
  const serversUp = servers.some((s) => s.state === 'läuft' || s.state === 'startet');
  const entry = selected ? (files.recent.find((f) => f.path === selected.path) || selected) : null;
  return (
    <aside id="sidebar" className="sidebar">
      <div className="rt" role="tablist">
        <button type="button" role="tab" aria-selected={tab === 'dateien'} className={tab === 'dateien' ? 'on' : ''} onClick={() => setTab('dateien')}><Icon name="folder" size={15} /> Dateien</button>
        <button type="button" role="tab" aria-selected={tab === 'vorschau'} className={tab === 'vorschau' ? 'on' : ''} onClick={() => setTab('vorschau')}><Icon name="eye" size={15} /> Vorschau</button>
        <button id="tab-docs" type="button" role="tab" aria-selected={tab === 'docs'} className={tab === 'docs' ? 'on' : ''} onClick={() => setTab('docs')}><Icon name="doc" size={15} /> Docs</button>
        <button type="button" role="tab" aria-selected={tab === 'browser'} className={tab === 'browser' ? 'on' : ''} onClick={() => setTab('browser')}>
          <Icon name="browser" size={15} /> Browser{browser.running ? <span className="live-dot" aria-hidden="true" />
            : serversUp ? <span className="srv-dot up" title="Ein Server läuft" aria-hidden="true" /> : null}
        </button>
        {free ? null : <button id="tab-github" type="button" role="tab" aria-selected={tab === 'github'} className={tab === 'github' ? 'on' : ''} onClick={() => setTab('github')}><Icon name="github" size={15} /> GitHub</button>}
        {modPanels.map((p) => (
          <button key={p.key} type="button" role="tab" aria-selected={tab === `mod:${p.key}`} className={`mod-tab${tab === `mod:${p.key}` ? ' on' : ''}`}
            title={`${p.titel} (Mod)`} onClick={() => setTab(`mod:${p.key}`)}><Icon name="puzzle" size={14} /> {p.titel}</button>
        ))}
      </div>
      <div className="rp-body">
        {tab === 'dateien' ? (
          <div className="files-pane">
            <FileExplorer root={root} free={free} files={files} marks={marks} removed={removed} selected={selected?.path}
              onSelect={onSelect} onPreview={(e) => { onSelect(e); setTab('vorschau'); }} onMention={onMention} notice={notice} />
            <Peek entry={entry} free={free} onPreview={(e) => { onSelect(e); setTab('vorschau'); }} onMention={onMention} />
          </div>
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
        {tab === 'browser' ? <BrowserTab browser={browser} active={browserActive} onClose={onBrowserClose} onBig={onBrowserBig} big={browserBig}
          servers={servers} root={free ? '' : root} notice={notice} /> : null}
        {tab === 'docs' ? <DocsPane root={root} free={free} files={files} running={running} notice={notice} {...docs} /> : null}
        {tab === 'github' && !free ? <GithubPane root={root} files={files} running={running} dialogOpen={dialogOpen} onConnect={onGithub} /> : null}
        {modPanels.filter((p) => tab === `mod:${p.key}`).map((p) => <ModPanel key={p.key} el={p.el} />)}
      </div>
    </aside>
  );
}

export { norm };
