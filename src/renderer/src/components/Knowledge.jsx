// Gedächtnis (ENTSCHEIDE E50) und eigene Skills (E51): Dialoge «Gedächtnis» mit Netz und Liste und «Skills»,
// dazu die Skills als Befehle im Eingabefeld (/name).
import { useEffect, useMemo, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { baseName, formatTime } from '../lib/format.js';
import { buildGraph, fit, layout, neighbours } from '../lib/graph.mjs';
import '../styles/knowledge.css';

const same = (a, b) => String(a || '').toLowerCase() === String(b || '').toLowerCase();

function Shell({ id, title, className = '', head, children, actions, onClose }) {
  return (
    <div id={id} className={`dialog ${className}`} role="dialog" aria-modal="true" aria-labelledby={`${id}-title`}>
      <div className="dialog-head">
        <h1 id={`${id}-title`}>{title}</h1>
        <span className="spacer" />
        {head}
        {onClose ? <button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={16} /></button> : null}
      </div>
      <div className="dialog-body">{children}</div>
      {actions ? <div className="actions">{actions}</div> : null}
    </div>
  );
}

// Skills als Befehle im Eingabefeld; das Gedächtnis meldet Neues im Chat (still, als Hinweis)
export function useSkillCommands({ composer, stateRef, notice }) {
  const [skills, setSkills] = useState([]);
  useEffect(() => {
    const a = api();
    const load = () => a.skillsList?.().then((l) => setSkills(Array.isArray(l) ? l : [])).catch(() => {});
    load();
    const off = a.onMemoryChanged?.((d) => {
      load();
      if (d?.added?.length && d.chatId && d.chatId === stateRef.current?.chat?.id) notice(`Gemerkt: ${d.added.join(' · ')}`);
    });
    return () => { if (typeof off === 'function') off(); };
  }, []);
  // Eigene und eingeschaltete mitgelieferte (E70); ein eigener mit gleichem Namen ersetzt den mitgelieferten
  return skills.filter((s) => s.enabled !== false && !s.replaced)
    .map((s) => ({ cmd: `/${s.slug}`, text: s.description, skill: true, run: () => composer.current?.setText(`/${s.slug} `) }));
}

// ---------- Gedächtnis ----------

function scopeLabel(item, projects) {
  if (item.scope !== 'projekt') return 'Über dich';
  return projects.find((p) => same(p.folder, item.project))?.name || baseName(item.project);
}

function scopeColor(item, projects) {
  if (item.scope !== 'projekt') return 'var(--text)';
  return projects.find((p) => same(p.folder, item.project))?.color || 'var(--dim)';
}

// Neuer oder geänderter Eintrag: Satz, Schlagworte, Bereich
function ItemForm({ item, projects, folder, onDone, onCancel, autoFocus = true }) {
  const [text, setText] = useState(item?.text || '');
  const [tags, setTags] = useState((item?.tags || []).join(', '));
  const [scope, setScope] = useState(item ? (item.scope === 'projekt' ? item.project : '') : '');
  const [error, setError] = useState('');
  const save = async (e) => {
    e?.preventDefault();
    const v = { text, tags: tags.split(','), scope: scope ? 'projekt' : 'konto', project: scope || null };
    const res = item ? await api().memoryUpdate(item.id, v) : await api().memoryAdd(v);
    if (res?.ok === false) { setError(res.reason || 'Nicht gespeichert.'); return; }
    onDone(res?.item);
  };
  const options = [...projects];
  if (folder && !options.some((p) => same(p.folder, folder))) options.unshift({ folder, name: baseName(folder) });
  if (item?.project && !options.some((p) => same(p.folder, item.project))) options.push({ folder: item.project, name: baseName(item.project) });
  return (
    <form className="mem-form" onSubmit={save}>
      <label className="mem-field"><span>Eintrag</span>
        <input id="mem-text" value={text} onChange={(e) => setText(e.target.value)} placeholder="z. B. Mag kurze Antworten in Stichworten." maxLength={240} autoFocus={autoFocus} />
      </label>
      <div className="mem-row">
        <label className="mem-field"><span>Schlagworte</span>
          <input id="mem-tags" value={tags} onChange={(e) => setTags(e.target.value)} placeholder="stil, antworten" />
        </label>
        <label className="mem-field"><span>Gilt für</span>
          <select id="mem-scope" value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="">Alle Projekte (über dich)</option>
            {options.map((p) => <option key={p.folder} value={p.folder}>Projekt {p.name}</option>)}
          </select>
        </label>
      </div>
      {error ? <p className="error">{error}</p> : null}
      <div className="mem-form-btns">
        <button type="button" className="btn small" onClick={onCancel}>Abbrechen</button>
        <button type="submit" className="btn small primary" disabled={text.trim().length < 3}>Speichern</button>
      </div>
    </form>
  );
}

// Seitenfeld zum gewählten Eintrag im Netz
function ItemPanel({ item, projects, folder, onChanged, onClose }) {
  const [editing, setEditing] = useState(false);
  useEffect(() => setEditing(false), [item.id]);
  const forget = async () => { await api().memoryRemove(item.id); onChanged(null); };
  const source = item.chat ? (item.chatTitle ? `Chat «${item.chatTitle}»` : 'einem Chat') : item.source === 'hand' ? 'von dir eingetragen' : '';
  return (
    <aside id="mem-panel" className="mem-panel" aria-label="Eintrag">
      <div className="mem-panel-head">
        <span className="mem-scope"><span className="dot" style={{ background: scopeColor(item, projects) }} />{scopeLabel(item, projects)}</span>
        <button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={14} /></button>
      </div>
      {editing ? <ItemForm item={item} projects={projects} folder={folder} onDone={(i) => { setEditing(false); onChanged(i); }} onCancel={() => setEditing(false)} /> : (
        <>
          <p className="mem-panel-text">{item.text}</p>
          {item.tags.length ? <div className="mem-tags">{item.tags.map((t) => <span key={t} className="mem-tag">{t}</span>)}</div> : null}
          <p className="muted small">
            {item.source === 'merken' ? 'Gemerkt auf Wunsch' : item.source === 'hand' ? 'Eingetragen' : 'Automatisch gemerkt'}
            {source && item.source !== 'hand' ? <> aus {item.chat && item.chatFolder ? (
              <button type="button" className="link" onClick={() => api().chatOpen(item.chatFolder, item.chat)}>{source}</button>
            ) : source}</> : null}
            {' · '}{formatTime(item.updated)}
          </p>
          <div className="mem-panel-btns">
            <button type="button" className="btn small" onClick={() => setEditing(true)}><Icon name="pen" size={13} /> Bearbeiten</button>
            <button type="button" id="mem-forget" className="btn small danger" onClick={forget}><Icon name="trash" size={13} /> Vergessen</button>
          </div>
        </>
      )}
    </aside>
  );
}

const R = { du: 20, projekt: 12, tag: 4.5, item: 5.5 };

function MemoryNet({ items, projects, folder, selected, onSelect, onChanged }) {
  const graph = useMemo(() => buildGraph(items, projects), [items, projects]);
  const near = useMemo(() => neighbours(graph), [graph]);
  const computed = useMemo(() => layout(graph), [graph]);
  const [pos, setPos] = useState(computed);
  const [view, setView] = useState(null);
  const [hover, setHover] = useState(null);
  const box = useRef(null);
  const drag = useRef(null);
  const viewRef = useRef(view);
  viewRef.current = view;

  useEffect(() => { setPos(computed); }, [computed]);
  const fitView = () => {
    const r = box.current?.getBoundingClientRect();
    if (r) setView(fit(computed, r.width, r.height));
  };
  // Erster Ausschnitt, sobald die Fläche ihre Grösse hat; neue Einträge verschieben ihn nicht
  useEffect(() => { if (!view) fitView(); }, [computed]);

  // Das Seitenfeld nimmt rechts 300 px weg: Ausschnitt um die Hälfte mitschieben, damit die Mitte bleibt
  const panelOpen = useRef(false);
  useEffect(() => {
    const open = !!selected;
    if (open !== panelOpen.current && view) setView((v) => ({ ...v, x: v.x + (open ? -150 : 150) }));
    panelOpen.current = open;
  }, [selected]);

  // Zoom mit dem Mausrad um den Zeiger (passiv: false, damit die Seite nicht scrollt)
  useEffect(() => {
    const el = box.current;
    if (!el) return undefined;
    const onWheel = (e) => {
      e.preventDefault();
      const v = viewRef.current;
      if (!v) return;
      const r = el.getBoundingClientRect();
      const mx = e.clientX - r.left;
      const my = e.clientY - r.top;
      const k = Math.max(0.25, Math.min(3, v.k * Math.exp(-e.deltaY * 0.0015)));
      setView({ k, x: mx - ((mx - v.x) / v.k) * k, y: my - ((my - v.y) / v.k) * k });
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, []);

  const zoom = (f) => {
    const r = box.current.getBoundingClientRect();
    const v = view;
    const k = Math.max(0.25, Math.min(3, v.k * f));
    setView({ k, x: r.width / 2 - ((r.width / 2 - v.x) / v.k) * k, y: r.height / 2 - ((r.height / 2 - v.y) / v.k) * k });
  };

  const down = (e, id = null) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    box.current.setPointerCapture?.(e.pointerId);
    drag.current = { id, sx: e.clientX, sy: e.clientY, moved: false, view, start: id ? pos[id] : null };
  };
  const move = (e) => {
    const d = drag.current;
    if (!d) return;
    const dx = e.clientX - d.sx;
    const dy = e.clientY - d.sy;
    if (!d.moved && Math.hypot(dx, dy) < 3) return;
    d.moved = true;
    if (d.id) setPos((p) => ({ ...p, [d.id]: { x: d.start.x + dx / view.k, y: d.start.y + dy / view.k } }));
    else setView({ ...d.view, x: d.view.x + dx, y: d.view.y + dy });
  };
  const up = () => {
    const d = drag.current;
    drag.current = null;
    if (!d || d.moved) return;
    const n = d.id && graph.nodes.find((x) => x.id === d.id);
    if (n?.kind === 'item') onSelect(n.item.id);
    else if (!d.id) onSelect(null);
  };

  const lit = hover ? near.get(hover) : selected ? near.get(`i:${selected}`) : null;
  const selItem = selected ? items.find((i) => i.id === selected) : null;
  const hoverNode = hover ? graph.nodes.find((n) => n.id === hover) : null;
  const tip = hoverNode?.kind === 'item' && view && pos[hover] ? { x: pos[hover].x * view.k + view.x, y: pos[hover].y * view.k + view.y, text: hoverNode.label } : null;
  // «Du» und Einträge über dich neutral, damit sie sich von Projekten in TF Blau abheben
  const fill = (n) => (n.kind === 'du' ? 'var(--text)' : n.kind === 'projekt' ? n.color || 'var(--dim)' : n.kind === 'tag' ? 'var(--pane)' : n.color || 'var(--muted)');

  return (
    <div className="mem-net">
      <div ref={box} id="mem-graph" className="mem-canvas" onPointerDown={(e) => down(e)} onPointerMove={move} onPointerUp={up} onPointerLeave={() => setHover(null)}>
        {view ? (
          <svg width="100%" height="100%" role="img" aria-label={`Netz mit ${items.length} Einträgen`}>
            <g transform={`translate(${view.x} ${view.y}) scale(${view.k})`}>
              {graph.links.map((l) => {
                const a = pos[l.source];
                const b = pos[l.target];
                if (!a || !b) return null;
                const on = lit && lit.has(l.source) && lit.has(l.target) && (l.source === hover || l.target === hover || l.source === `i:${selected}` || l.target === `i:${selected}` || !hover);
                return <line key={`${l.source}-${l.target}`} x1={a.x} y1={a.y} x2={b.x} y2={b.y} className={`mem-link ${l.kind}${lit ? (on ? ' lit' : ' dim') : ''}`} />;
              })}
              {graph.nodes.map((n) => {
                const p = pos[n.id];
                if (!p) return null;
                const dim = lit && !lit.has(n.id);
                const isSel = n.kind === 'item' && n.item.id === selected;
                return (
                  <g key={n.id} transform={`translate(${p.x} ${p.y})`} className={`mem-node ${n.kind}${dim ? ' dim' : ''}${isSel ? ' sel' : ''}`}
                    onPointerDown={(e) => down(e, n.id)} onPointerEnter={() => setHover(n.id)} onPointerLeave={() => setHover((h) => (h === n.id ? null : h))}>
                    {n.kind === 'item' ? <circle r={12} className="hit" /> : null}
                    <circle r={R[n.kind]} style={{ fill: fill(n) }} />
                    {n.kind === 'du' ? <text className="mem-label du" dy="0.35em" textAnchor="middle">Du</text> : null}
                    {n.kind === 'projekt' ? <text className="mem-label projekt" y={R.projekt + 14} textAnchor="middle">{n.label.length > 26 ? `${n.label.slice(0, 25)}…` : n.label}</text> : null}
                    {n.kind === 'tag' ? <text className="mem-label tag" x={8} dy="0.35em">{n.label}</text> : null}
                  </g>
                );
              })}
            </g>
          </svg>
        ) : null}
        {tip ? <div className="mem-tip" style={{ left: tip.x, top: tip.y }}>{tip.text}</div> : null}
        <div className="mem-zoom">
          <button type="button" className="icon-btn" aria-label="Vergrössern" onClick={() => zoom(1.25)} onPointerDown={(e) => e.stopPropagation()}><Icon name="plus" size={15} /></button>
          <button type="button" className="icon-btn" aria-label="Verkleinern" onClick={() => zoom(0.8)} onPointerDown={(e) => e.stopPropagation()}><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true"><path d="M5 12h14" /></svg></button>
          <button type="button" className="icon-btn" aria-label="Alles zeigen" onClick={() => { setPos(computed); fitView(); }} onPointerDown={(e) => e.stopPropagation()}><Icon name="expand" size={14} /></button>
        </div>
        <div className="mem-legend" aria-hidden="true">
          <span><i className="dot" style={{ background: 'var(--text)' }} />Du</span>
          <span><i className="dot" style={{ background: 'var(--acc)' }} />Projekt</span>
          <span><i className="dot ring" />Schlagwort</span>
          <span><i className="dot small" style={{ background: 'var(--muted)' }} />Eintrag</span>
        </div>
      </div>
      {selItem ? <ItemPanel item={selItem} projects={projects} folder={folder} onChanged={onChanged} onClose={() => onSelect(null)} /> : null}
    </div>
  );
}

function MemoryList({ items, projects, folder, adding, setAdding, onChanged }) {
  const [query, setQuery] = useState('');
  const [editing, setEditing] = useState(null);
  const q = query.trim().toLowerCase();
  const shown = q ? items.filter((i) => `${i.text} ${i.tags.join(' ')} ${scopeLabel(i, projects)}`.toLowerCase().includes(q)) : items;
  return (
    <div className="mem-list-view">
      <div className="mem-search">
        <Icon name="search" size={14} />
        <input id="mem-search" type="text" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Im Gedächtnis suchen" aria-label="Im Gedächtnis suchen" />
      </div>
      {adding ? <div className="mem-new"><ItemForm projects={projects} folder={folder} onDone={() => { setAdding(false); onChanged(); }} onCancel={() => setAdding(false)} /></div> : null}
      <ul id="mem-list" className="mem-list">
        {shown.map((i) => (
          <li key={i.id} className={editing === i.id ? 'editing' : ''}>
            {editing === i.id ? <ItemForm item={i} projects={projects} folder={folder} onDone={() => { setEditing(null); onChanged(); }} onCancel={() => setEditing(null)} /> : (
              <>
                <span className="dot" style={{ background: scopeColor(i, projects) }} title={scopeLabel(i, projects)} />
                <div className="mem-item">
                  <div className="mem-item-text">{i.text}</div>
                  <div className="mem-item-meta">
                    <span>{scopeLabel(i, projects)}</span>
                    {i.tags.map((t) => <span key={t} className="mem-tag">{t}</span>)}
                    <span className="muted">{formatTime(i.updated)}</span>
                  </div>
                </div>
                <div className="mem-item-btns">
                  <button type="button" className="icon-btn" aria-label="Bearbeiten" title="Bearbeiten" onClick={() => setEditing(i.id)}><Icon name="pen" size={14} /></button>
                  <button type="button" className="icon-btn" aria-label="Vergessen" title="Vergessen" onClick={async () => { await api().memoryRemove(i.id); onChanged(); }}><Icon name="trash" size={14} /></button>
                </div>
              </>
            )}
          </li>
        ))}
      </ul>
      {!shown.length && items.length ? <p className="muted mem-empty-small">Nichts gefunden.</p> : null}
    </div>
  );
}

export function MemoryDialog({ state, onClose }) {
  const [data, setData] = useState(null);
  const [tab, setTab] = useState('netz');
  const [selected, setSelected] = useState(null);
  const [adding, setAdding] = useState(false);
  const load = () => api().memoryList().then(setData);
  useEffect(() => {
    load();
    const off = api().onMemoryChanged?.(() => load());
    return () => { if (typeof off === 'function') off(); };
  }, []);
  const items = data?.items || [];
  const projects = data?.projects || [];
  const enabled = data ? data.enabled : state.memory !== false;
  const folder = state.free ? '' : state.folder; // ein Chat ohne Projekt ist kein Projekt, für das etwas gelten könnte
  const changed = (item) => { load(); if (item === null) setSelected(null); };
  const head = (
    <div className="segmented" role="tablist" aria-label="Ansicht">
      {[['netz', 'Netz'], ['liste', 'Liste']].map(([v, label]) => (
        <button key={v} type="button" role="tab" id={`mem-tab-${v}`} aria-selected={tab === v} className={tab === v ? 'on' : ''} onClick={() => setTab(v)}>{label}</button>
      ))}
    </div>
  );
  return (
    <Shell id="dlg-memory" title="Gedächtnis" className="mem-dialog" head={head} onClose={onClose} actions={(
      <>
        <span className="muted small mem-count">{items.length === 1 ? '1 Eintrag' : `${items.length} Einträge`} · nur auf diesem Laptop</span>
        <span className="spacer" />
        {items.length ? <button type="button" id="mem-clear" className="btn" onClick={async () => { if (await api().memoryClear()) { setSelected(null); load(); } }}>Alles vergessen</button> : null}
        <button type="button" id="mem-new" className="btn" onClick={() => { setTab('liste'); setAdding(true); }}><Icon name="plus" size={14} /> Neu</button>
        <button type="button" className="btn primary" onClick={onClose}>Fertig</button>
      </>
    )}>
      {!enabled ? (
        <div className="mem-off" role="status">
          <span>Das Gedächtnis ist ausgeschaltet. ASK merkt sich nichts Neues und schickt nichts davon an das Modell.</span>
          <button type="button" className="btn small" onClick={async () => { await api().setMemoryEnabled(true); load(); }}>Einschalten</button>
        </div>
      ) : null}
      {!data ? <p className="muted">Lädt …</p> : !items.length && !adding ? (
        <div className="mem-empty">
          <div className="mem-empty-mark"><span /><span /><span /></div>
          <p><strong>Noch nichts gemerkt</strong></p>
          <p className="muted">Nach einem Auftrag merkt sich ASK, was auch später nützt: wie du Antworten magst, was du kannst, was in deinen Projekten gilt. Sag «merk dir …» oder trag selbst etwas ein.</p>
          <button type="button" className="btn" onClick={() => { setTab('liste'); setAdding(true); }}><Icon name="plus" size={14} /> Eintrag hinzufügen</button>
        </div>
      ) : tab === 'netz' ? (
        <MemoryNet items={items} projects={projects} folder={folder} selected={selected} onSelect={setSelected} onChanged={changed} />
      ) : (
        <MemoryList items={items} projects={projects} folder={folder} adding={adding} setAdding={setAdding} onChanged={() => load()} />
      )}
    </Shell>
  );
}

// ---------- Skills ----------

const slugOf = (name) => String(name || '').toLowerCase().replace(/ä/g, 'ae').replace(/ö/g, 'oe').replace(/ü/g, 'ue').replace(/ß/g, 'ss')
  .normalize('NFKD').replace(/[\u0300-\u036f]/g, '').replace(/[^a-z0-9]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40).replace(/-+$/, '');

const EMPTY = { name: '', description: '', body: '' };

// Eigene Skills bearbeiten. initial: Name zum Öffnen ('' = neu); copy: Vorlage für einen neuen (mitgelieferten kopieren)
export function SkillsDialog({ onClose, initial, copy }) {
  const [list, setList] = useState(null);
  const [current, setCurrent] = useState(null); // slug, '' für neu, null für keinen
  const [draft, setDraft] = useState(EMPTY);
  const [error, setError] = useState('');
  const [info, setInfo] = useState('');
  const load = () => api().skillsList().then((l) => { const own = (l || []).filter((x) => x.source !== 'standard'); setList(own); return own; });
  useEffect(() => {
    load();
    if (copy) { setCurrent(''); setDraft({ name: copy.name, description: copy.description, body: copy.body }); }
    else if (typeof initial === 'string') open(initial);
  }, []);
  const open = async (slug) => {
    setError(''); setInfo('');
    if (slug === '') { setCurrent(''); setDraft(EMPTY); return; }
    const s = await api().skillsGet(slug);
    if (!s) { load(); return; }
    setCurrent(s.slug);
    setDraft({ name: s.slug, description: s.description, body: s.body });
  };
  const save = async () => {
    const res = await api().skillsSave(draft, current || '');
    if (!res?.ok) { setError(res?.reason || 'Nicht gespeichert.'); return; }
    setError(''); setInfo('Gespeichert.');
    await load();
    setCurrent(res.skill.slug);
    setDraft({ name: res.skill.slug, description: res.skill.description, body: res.skill.body });
  };
  const remove = async () => {
    if (!(await api().skillsRemove(current))) return;
    setCurrent(null); setDraft(EMPTY);
    load();
  };
  const exportIt = async () => {
    const res = await api().skillsExport(current);
    if (res?.ok === false) setError(res.reason);
    else if (res?.path) setInfo(`Exportiert: ${baseName(res.path)}`);
  };
  const importIt = async () => {
    const res = await api().skillsImport();
    if (res?.ok === false) { setError(res.reason); setInfo(''); return; }
    if (res?.skill) { await load(); await open(res.skill.slug); setInfo('Importiert.'); }
  };
  const slug = slugOf(draft.name);
  return (
    <Shell id="dlg-skills" title="Skills" className="skills-dialog" onClose={onClose} actions={(
      <>
        <button type="button" id="skills-import" className="btn" onClick={importIt}>Importieren …</button>
        <span className="spacer" />
        <button type="button" className="btn primary" onClick={onClose}>Fertig</button>
      </>
    )}>
      <div className="skills">
        <nav className="skills-nav" aria-label="Skills">
          <ul id="skills-list">
            {(list || []).map((s) => (
              <li key={s.slug}>
                <button type="button" className={current === s.slug ? 'on' : ''} onClick={() => open(s.slug)}>
                  <span className="skill-cmd">/{s.slug}</span>
                  <span className="skill-desc">{s.description}</span>
                </button>
              </li>
            ))}
          </ul>
          <button type="button" id="skills-new" className={`skills-add${current === '' ? ' on' : ''}`} onClick={() => open('')}><Icon name="plus" size={14} /> Neuer Skill</button>
        </nav>
        <div className="skills-main">
          {current === null ? (
            <div className="skills-intro">
              <p><strong>Eigene Anleitungen für wiederkehrende Aufträge</strong></p>
              <p className="muted">Ein Skill beschreibt, wie ASK etwas tun soll, zum Beispiel einen Wochenbericht nach deiner Vorlage. Du rufst ihn mit <code>/name</code> im Eingabefeld auf, oder ASK liest ihn selbst, wenn er zum Auftrag passt.</p>
              <p className="muted">Skills liegen nur auf diesem Laptop. Zum Teilen exportierst du sie als Datei, andere importieren sie.</p>
            </div>
          ) : (
            <form className="skill-form" onSubmit={(e) => { e.preventDefault(); save(); }}>
              <div className="mem-row">
                <label className="mem-field"><span>Name</span>
                  <input id="skill-name" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} placeholder="z. B. wochenbericht" maxLength={60} autoFocus={current === ''} />
                </label>
                <div className="skill-call muted small">{slug ? <>Aufruf: <code>/{slug}</code></> : ' '}</div>
              </div>
              <label className="mem-field"><span>Beschreibung</span>
                <input id="skill-desc" value={draft.description} onChange={(e) => setDraft({ ...draft, description: e.target.value })} placeholder="Wofür und wann ASK diesen Skill benutzt" maxLength={300} />
              </label>
              <label className="mem-field grow"><span>Anleitung</span>
                <textarea id="skill-body" value={draft.body} onChange={(e) => setDraft({ ...draft, body: e.target.value })} spellCheck={false}
                  placeholder={'Schritte, Vorgaben und Beispiele in Markdown, z. B.\n\n1. Lies die Notizen der Woche.\n2. Schreib den Bericht nach der Vorlage in berichte/.'} />
              </label>
              {error ? <p className="error" id="skill-error">{error}</p> : info ? <p className="muted small" role="status">{info}</p> : null}
              <div className="skill-btns">
                {current ? <button type="button" className="btn small danger" onClick={remove}><Icon name="trash" size={13} /> Löschen</button> : null}
                {current ? <button type="button" className="btn small" onClick={exportIt}><Icon name="down" size={13} /> Exportieren …</button> : null}
                <span className="spacer" />
                {current === '' ? <button type="button" className="btn small" onClick={() => setCurrent(null)}>Abbrechen</button> : null}
                <button type="submit" id="skill-save" className="btn small primary" disabled={!slug || !draft.description.trim() || !draft.body.trim()}>Speichern</button>
              </div>
            </form>
          )}
        </div>
      </div>
    </Shell>
  );
}
