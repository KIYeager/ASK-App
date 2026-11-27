// Gedächtnis (ENTSCHEIDE E50) und eigene Skills (E51): Dialoge «Gedächtnis» mit Netz und Liste und «Skills»,
// dazu die Skills als Befehle im Eingabefeld (/name).
import { useEffect, useMemo, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { baseName, formatTime } from '../lib/format.js';
import { MAP, fit, mindmap, related, shorten } from '../lib/graph.mjs';
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
            {item.source === 'merken' ? 'Gemerkt auf Wunsch' : item.source === 'hand' ? 'Eingetragen' : item.source === 'import' ? 'Importiert' : 'Automatisch gemerkt'}
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

const TAGS_SHOWN = 10; // Schlagworte über der Karte, bis man «alle» wählt

// Gedankenkarte: «Du» in der Mitte, ein Ast für «Über dich» und einer pro Projekt, jeder Eintrag als lesbare Zeile
// (lib/graph.mjs). Ziehen verschiebt, das Mausrad zoomt; ein Schlagwort oben hebt seine Einträge hervor, beim
// Darüberfahren leuchten die Einträge mit gemeinsamem Schlagwort.
function MemoryNet({ items, projects, folder, selected, onSelect, onChanged }) {
  const map = useMemo(() => mindmap(items, projects), [items, projects]);
  const [view, setView] = useState(null);
  const [hover, setHover] = useState(null); // Kennung eines Eintrags
  const [tag, setTag] = useState(null); // gewähltes Schlagwort
  const [allTags, setAllTags] = useState(false);
  const box = useRef(null);
  const drag = useRef(null);
  const viewRef = useRef(view);
  viewRef.current = view;
  const byId = useMemo(() => new Map(map.nodes.map((n) => [n.id, n])), [map]);

  const fitView = () => {
    const r = box.current?.getBoundingClientRect();
    if (r) setView(fit(map.extent, r.width, r.height, 30, 0.82)); // nie so klein, dass man die Einträge nicht mehr lesen kann
  };
  // Erster Ausschnitt, sobald die Fläche ihre Grösse hat; neue Einträge verschieben ihn nicht
  useEffect(() => { if (!view) fitView(); }, [map]);
  // Ein Schlagwort, das es nicht mehr gibt, nicht weiter als Filter halten
  useEffect(() => { if (tag && !map.tags.some(([t]) => t === tag)) setTag(null); }, [map, tag]);

  // Gewählter Eintrag bleibt ganz sichtbar: Das Seitenfeld nimmt rechts 300 px weg, und die Zeile kann am Rand liegen
  useEffect(() => {
    const n = selected ? byId.get(`i:${selected}`) : null;
    const r = box.current?.getBoundingClientRect();
    const v = viewRef.current;
    if (!n || !r || !v) return;
    const left = (n.side > 0 ? n.x - 10 : n.x - n.width - 6) * v.k + v.x;
    const right = (n.side > 0 ? n.x + n.width + 6 : n.x + 10) * v.k + v.x;
    const y = n.y * v.k + v.y;
    const dx = left < 12 ? 12 - left : right > r.width - 12 ? Math.max(12 - left, r.width - 12 - right) : 0;
    const dy = y < 70 ? 70 - y : y > r.height - 40 ? r.height - 40 - y : 0;
    if (dx || dy) setView({ ...v, x: v.x + dx, y: v.y + dy });
  }, [selected, byId]);

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
      const k = Math.max(0.3, Math.min(2.5, v.k * Math.exp(-e.deltaY * 0.0015)));
      setView({ k, x: mx - ((mx - v.x) / v.k) * k, y: my - ((my - v.y) / v.k) * k });
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, []);

  const zoom = (f) => {
    const r = box.current.getBoundingClientRect();
    const v = view;
    if (!v) return;
    const k = Math.max(0.3, Math.min(2.5, v.k * f));
    setView({ k, x: r.width / 2 - ((r.width / 2 - v.x) / v.k) * k, y: r.height / 2 - ((r.height / 2 - v.y) / v.k) * k });
  };

  // Ziehen verschiebt die Karte; ein Klick ohne Ziehen wählt den Eintrag unter dem Zeiger oder hebt die Auswahl auf
  const down = (e, id = null) => {
    if (e.button !== 0) return;
    e.stopPropagation();
    box.current.setPointerCapture?.(e.pointerId);
    drag.current = { id, sx: e.clientX, sy: e.clientY, moved: false, view };
  };
  const move = (e) => {
    const d = drag.current;
    if (!d || !d.view) return;
    const dx = e.clientX - d.sx;
    const dy = e.clientY - d.sy;
    if (!d.moved && Math.hypot(dx, dy) < 4) return;
    d.moved = true;
    setView({ ...d.view, x: d.view.x + dx, y: d.view.y + dy });
  };
  const up = () => {
    const d = drag.current;
    drag.current = null;
    if (!d || d.moved) return;
    onSelect(d.id);
  };

  const lit = hover ? related(items, hover) : tag ? new Set(items.filter((i) => (i.tags || []).includes(tag)).map((i) => i.id)) : null;
  const selItem = selected ? items.find((i) => i.id === selected) : null;
  const hoverNode = hover ? byId.get(`i:${hover}`) : null;
  const tip = hoverNode && view && !drag.current
    ? { x: (hoverNode.x + hoverNode.side * 60) * view.k + view.x, y: (hoverNode.y - 12) * view.k + view.y, text: hoverNode.item.text, tags: hoverNode.item.tags || [] } : null;
  const curve = (a, b) => { const mx = (a.x + b.x) / 2; return `M${a.x} ${a.y} C${mx} ${a.y} ${mx} ${b.y} ${b.x} ${b.y}`; };
  const shownTags = allTags ? map.tags : map.tags.slice(0, TAGS_SHOWN);

  return (
    <div className="mem-net">
      <div ref={box} id="mem-graph" className="mem-canvas" onPointerDown={(e) => down(e)} onPointerMove={move} onPointerUp={up} onPointerCancel={() => { drag.current = null; }}
        onPointerLeave={() => setHover(null)}>
        {view ? (
          <svg width="100%" height="100%" role="group" aria-label={`Karte mit ${items.length} Einträgen`}>
            <g transform={`translate(${view.x} ${view.y}) scale(${view.k})`}>
              {map.links.map((l) => {
                const a = byId.get(l.from);
                const b = byId.get(l.to);
                const on = !lit || l.kind === 'ast' || lit.has(b.item?.id);
                return <path key={`${l.from}-${l.to}`} d={curve(a, b)} className={`mem-link ${l.kind}${on ? '' : ' dim'}`} style={l.color ? { stroke: l.color } : undefined} />;
              })}
              {map.nodes.map((n) => {
                if (n.kind === 'du') {
                  return <g key={n.id} className="mem-node du"><circle r={24} /><text className="mem-label du" dy="0.35em" textAnchor="middle">Du</text></g>;
                }
                if (n.kind === 'gruppe') {
                  const name = shorten(n.label, MAP.hubChars);
                  const w = Math.max(64, name.length * 7.4 + 44);
                  return (
                    <g key={n.id} transform={`translate(${n.x} ${n.y})`} className={`mem-node gruppe${n.about ? ' about' : ''}`}>
                      <title>{n.topic ? `Über dich, Thema «${n.label}»` : n.about ? 'Was ASK über dich weiss, für alle Projekte' : `Gilt nur im Projekt ${n.label}`}</title>
                      <rect x={-w / 2} y={-14} width={w} height={28} rx={14} style={n.color ? { fill: n.color, stroke: n.color } : undefined} />
                      <text className="mem-label gruppe" dy="0.35em" textAnchor="middle" x={-9}>{name}</text>
                      <text className="mem-label count" dy="0.35em" textAnchor="end" x={w / 2 - 10}>{n.count}</text>
                    </g>
                  );
                }
                const dim = lit && !lit.has(n.item.id);
                const isSel = n.item.id === selected;
                const right = n.side > 0;
                return (
                  <g key={n.id} transform={`translate(${n.x} ${n.y})`} className={`mem-node item${dim ? ' dim' : ''}${isSel ? ' sel' : ''}${hover === n.item.id ? ' hover' : ''}`}
                    role="button" tabIndex={0} aria-label={n.item.text} aria-pressed={isSel}
                    onPointerDown={(e) => down(e, n.item.id)} onPointerEnter={() => setHover(n.item.id)} onPointerLeave={() => setHover((h) => (h === n.item.id ? null : h))}
                    onFocus={() => setHover(n.item.id)} onBlur={() => setHover((h) => (h === n.item.id ? null : h))}
                    onKeyDown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); onSelect(n.item.id); } }}>
                    <rect className="hit" x={right ? -8 : -n.width - 4} y={-13} width={n.width + 12} height={26} rx={8} />
                    <circle r={4.5} style={n.color ? { fill: n.color } : undefined} />
                    <text className="mem-label item" dy="0.35em" x={right ? 12 : -12} textAnchor={right ? 'start' : 'end'}>
                      {n.label}
                    </text>
                  </g>
                );
              })}
            </g>
          </svg>
        ) : null}
        {tip ? <div className="mem-tip" style={{ left: tip.x, top: tip.y }}>{tip.text}{tip.tags.length ? <span className="mem-tip-tags">{tip.tags.map((t) => `#${t}`).join(' ')}</span> : null}</div> : null}
        {map.tags.length ? (
          <div className="mem-tagbar" onPointerDown={(e) => e.stopPropagation()}>
            <span className="mem-tagbar-label">Schlagworte</span>
            {shownTags.map(([t, n]) => (
              <button key={t} type="button" className={`mem-chip${tag === t ? ' on' : ''}`} aria-pressed={tag === t} title={`${n} ${n === 1 ? 'Eintrag' : 'Einträge'} mit «${t}» hervorheben`}
                onClick={() => setTag(tag === t ? null : t)}>{t}<span>{n}</span></button>
            ))}
            {map.tags.length > TAGS_SHOWN ? (
              <button type="button" className="mem-chip more" onClick={() => setAllTags(!allTags)}>{allTags ? 'weniger' : `+${map.tags.length - TAGS_SHOWN}`}</button>
            ) : null}
          </div>
        ) : null}
        <div className="mem-zoom" onPointerDown={(e) => e.stopPropagation()}>
          <button type="button" className="icon-btn" title="Vergrössern" aria-label="Vergrössern" onClick={() => zoom(1.25)}><Icon name="plus" size={15} /></button>
          <button type="button" className="icon-btn" title="Verkleinern" aria-label="Verkleinern" onClick={() => zoom(0.8)}><svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" aria-hidden="true"><path d="M5 12h14" /></svg></button>
          <button type="button" className="icon-btn" title="Alles zeigen" aria-label="Alles zeigen" onClick={fitView}><Icon name="expand" size={14} /></button>
        </div>
        <div className="mem-legend" aria-hidden="true">Klick: Eintrag öffnen · Ziehen: verschieben · Mausrad: zoomen</div>
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

// Import aus einer anderen KI (ENTSCHEIDE E78): Text einfügen, ASK macht Vorschläge, die Person wählt aus
function MemoryImport({ onDone, onCancel }) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [found, setFound] = useState(null); // { items: [{ text, tags, known, on }], byModel }
  const [error, setError] = useState('');
  const read = async () => {
    setBusy(true); setError('');
    let res;
    try { res = await api().memoryImportRead(text); } catch (err) { res = { ok: false, reason: err?.message || String(err) }; }
    setBusy(false);
    if (!res?.ok) { setError(res?.reason || 'Das hat nicht geklappt.'); return; }
    if (!res.items.length) { setError('Im Text hat ASK nichts gefunden, was sich als Eintrag eignet.'); return; }
    setFound({ byModel: res.byModel, items: res.items.map((i) => ({ ...i, on: !i.known })) });
  };
  const save = async () => {
    const chosen = found.items.filter((i) => i.on).map(({ text: t, tags }) => ({ text: t, tags }));
    setBusy(true); setError('');
    let res;
    try { res = await api().memoryImportSave(chosen); } catch (err) { res = { ok: false, reason: err?.message || String(err) }; }
    setBusy(false);
    if (!res?.ok) { setError(res?.reason || 'Nicht gespeichert.'); return; }
    onDone(res);
  };
  const toggle = (i) => setFound({ ...found, items: found.items.map((x, j) => (j === i ? { ...x, on: !x.on } : x)) });
  const count = found ? found.items.filter((i) => i.on).length : 0;
  if (!found) {
    return (
      <div className="mem-import">
        <p><strong>Gedächtnis aus einer anderen KI übernehmen</strong></p>
        <p className="muted">Frag Claude, ChatGPT oder eine andere KI: «Liste alles auf, was du über mich gespeichert hast, ein Punkt pro Zeile.» Kopiere die Antwort und füge sie hier ein. ASK macht daraus Einträge; du wählst danach aus, was übernommen wird.</p>
        <textarea id="mem-import-text" value={text} onChange={(e) => setText(e.target.value)} spellCheck={false} autoFocus placeholder={'- Lernt Informatik im zweiten Lehrjahr\n- Mag kurze Antworten mit einem Beispiel\n- Arbeitet mit Windows 11 und VS Code'} aria-label="Exportierter Text" />
        {error ? <p className="error" role="alert">{error}</p> : <p className="muted small">Der Text geht zur Auswertung an das Modell in ASK. Füge nichts Vertrauliches ein (Passwörter, Gesundheit, Noten).</p>}
        <div className="mem-form-btns">
          <button type="button" className="btn small" onClick={onCancel}>Abbrechen</button>
          <button type="button" id="mem-import-read" className="btn small primary" disabled={busy || text.trim().length < 10} onClick={read}>{busy ? 'ASK liest …' : 'Auswerten'}</button>
        </div>
      </div>
    );
  }
  return (
    <div className="mem-import">
      <p><strong>{found.items.length === 1 ? '1 Vorschlag' : `${found.items.length} Vorschläge`}</strong> <span className="muted">{found.byModel ? 'Wähle aus, was ASK sich merken soll.' : 'ASK war nicht erreichbar: Jede Zeile ist ein Vorschlag, ohne Schlagworte.'}</span></p>
      <ul id="mem-import-list" className="mem-import-list">
        {found.items.map((i, n) => (
          <li key={n} className={i.on ? 'on' : ''}>
            <label>
              <input type="checkbox" checked={i.on} onChange={() => toggle(n)} />
              <span className="mem-item">
                <span className="mem-item-text">{i.text}</span>
                <span className="mem-item-meta">{i.tags.map((t) => <span key={t} className="mem-tag">{t}</span>)}{i.known ? <span className="muted">schon vorhanden</span> : null}</span>
              </span>
            </label>
          </li>
        ))}
      </ul>
      {error ? <p className="error" role="alert">{error}</p> : null}
      <div className="mem-form-btns">
        <button type="button" className="btn small" onClick={() => setFound({ ...found, items: found.items.map((x) => ({ ...x, on: count === 0 })) })}>{count === 0 ? 'Alle wählen' : 'Keine wählen'}</button>
        <span className="spacer" />
        <button type="button" className="btn small" onClick={() => { setFound(null); setError(''); }}>Zurück</button>
        <button type="button" id="mem-import-save" className="btn small primary" disabled={busy || !count} onClick={save}>{busy ? 'Speichert …' : count === 1 ? '1 Eintrag übernehmen' : `${count} Einträge übernehmen`}</button>
      </div>
    </div>
  );
}

export function MemoryDialog({ state, onClose }) {
  const [importing, setImporting] = useState(false);
  const [note, setNote] = useState('');
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
      {[['netz', 'Karte'], ['liste', 'Liste']].map(([v, label]) => (
        <button key={v} type="button" role="tab" id={`mem-tab-${v}`} aria-selected={tab === v} className={tab === v ? 'on' : ''} onClick={() => setTab(v)}>{label}</button>
      ))}
    </div>
  );
  return (
    <Shell id="dlg-memory" title="Gedächtnis" className="mem-dialog" head={head} onClose={onClose} actions={(
      <>
        <span className="muted small mem-count" role="status">{note || `${items.length === 1 ? '1 Eintrag' : `${items.length} Einträge`} · nur auf diesem Laptop`}</span>
        <span className="spacer" />
        {items.length ? <button type="button" id="mem-clear" className="btn" onClick={async () => { if (await api().memoryClear()) { setSelected(null); load(); } }}>Alles vergessen</button> : null}
        <button type="button" id="mem-import" className="btn" title="Gedächtnis aus Claude, ChatGPT oder einer anderen KI übernehmen" onClick={() => { setNote(''); setImporting(true); }}><Icon name="down" size={14} /> Importieren …</button>
        <button type="button" id="mem-new" className="btn" onClick={() => { setImporting(false); setTab('liste'); setAdding(true); }}><Icon name="plus" size={14} /> Neu</button>
        <button type="button" className="btn primary" onClick={onClose}>Fertig</button>
      </>
    )}>
      {!enabled ? (
        <div className="mem-off" role="status">
          <span>Das Gedächtnis ist ausgeschaltet. ASK merkt sich nichts Neues und schickt nichts davon an das Modell.</span>
          <button type="button" className="btn small" onClick={async () => { await api().setMemoryEnabled(true); load(); }}>Einschalten</button>
        </div>
      ) : null}
      {importing ? (
        <MemoryImport onCancel={() => setImporting(false)} onDone={(res) => {
          setImporting(false);
          setNote(`${res.added === 1 ? '1 Eintrag' : `${res.added} Einträge`} übernommen${res.failed ? `, ${res.failed} nicht (zu kurz oder heikel)` : ''}`);
          load();
        }} />
      ) : !data ? <p className="muted">Lädt …</p> : !items.length && !adding ? (
        <div className="mem-empty">
          <div className="mem-empty-mark"><span /><span /><span /></div>
          <p><strong>Noch nichts gemerkt</strong></p>
          <p className="muted">Nach einem Auftrag merkt sich ASK, was auch später nützt: wie du Antworten magst, was du kannst, was in deinen Projekten gilt. Sag «merk dir …» oder trag selbst etwas ein.</p>
          <div className="mem-empty-btns">
            <button type="button" className="btn" onClick={() => { setTab('liste'); setAdding(true); }}><Icon name="plus" size={14} /> Eintrag hinzufügen</button>
            <button type="button" className="btn" onClick={() => setImporting(true)}><Icon name="down" size={14} /> Aus einer anderen KI importieren …</button>
          </div>
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
