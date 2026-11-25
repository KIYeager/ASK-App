import { useEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { formatTime, shortTime } from '../lib/format.js';

// Linke Leiste: Projekte (ein Projekt = ein Arbeitsordner) mit ihren Chats
export default function Sidebar({ groups, chatId, running, runningChats = [], query, setQuery, renaming, onRenameDone, onNewChat, onOpenChat, onDeleteChat,
  onNewProject, onProjectMenu, onNewInProject, state, onSettings, onStatus, update, onUpdate, onRefresh }) {
  const [collapsed, setCollapsed] = useState(() => new Set());
  const [menu, setMenu] = useState(null); // { folder, chat, x, y }
  const [renamingChat, setRenamingChat] = useState(null); // { folder, id }
  const [hits, setHits] = useState({ query: '', list: [] });
  const q = query.trim().toLowerCase();
  // Suche auch im Inhalt der Chats (über alle Projekte), kurz nach dem Tippen
  useEffect(() => {
    const raw = query.trim();
    if (raw.length < 2) { setHits({ query: '', list: [] }); return undefined; }
    const t = setTimeout(async () => {
      let list = [];
      try { list = await api().chatSearch(raw); } catch { list = []; }
      setHits({ query: raw, list: Array.isArray(list) ? list : [] });
    }, 250);
    return () => clearTimeout(t);
  }, [query]);
  const openMenu = (e, folder, chat) => {
    e.preventDefault();
    e.stopPropagation();
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ folder, chat, x: e.type === 'contextmenu' ? e.clientX : r.left, y: e.type === 'contextmenu' ? e.clientY : r.bottom + 4 });
  };
  const toggle = (folder) => {
    const next = new Set(collapsed);
    if (next.has(folder)) next.delete(folder); else next.add(folder);
    setCollapsed(next);
  };
  const ask = state.ask || {};
  const term = state.terminal || {};
  const termOk = term.state === 'läuft';
  const askOk = ask.state === 'angemeldet';
  return (
    <aside id="chatbar" className="chatbar">
      <button id="new-btn" type="button" className="new-chat" onClick={onNewChat} disabled={running || !state.folder} title="Neuer Chat im aktuellen Projekt (/neu)">
        <Icon name="pen" size={15} /> Neuer Chat
      </button>
      <label className="search">
        <Icon name="search" size={15} />
        <input id="chat-search" type="search" placeholder="Chats durchsuchen" spellCheck={false} value={query}
          onChange={(e) => setQuery(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') setQuery(''); }} />
      </label>
      <div className="side-label">
        <span>Projekte</span>
        <button type="button" className="icon-btn" title="Neues Projekt" aria-label="Neues Projekt" onClick={onNewProject}><Icon name="plus" size={15} /></button>
      </div>
      <div id="chat-groups" className="chat-groups">
        {groups.map((g) => {
          let chats = g.chats;
          if (g.current && chatId && !chats.some((c) => c.id === chatId)) chats = [{ id: chatId, title: 'Neuer Chat', updated: Date.now(), count: 0, placeholder: true }, ...chats];
          if (q) chats = chats.filter((c) => !c.placeholder && c.title.toLowerCase().includes(q));
          if (q && !chats.length) return null;
          const isCollapsed = !q && collapsed.has(g.folder);
          return (
            <section key={g.folder} className={`chat-group${g.current ? ' current' : ''}${isCollapsed ? ' collapsed' : ''}${g.missing ? ' missing' : ''}`} data-folder={g.folder}>
              <div className="group-head" title={g.missing ? `${g.folder} (nicht gefunden)` : g.folder}
                onContextMenu={(e) => { e.preventDefault(); onProjectMenu(g.folder); }}>
                <button type="button" className="group-toggle" onClick={() => toggle(g.folder)} aria-expanded={!isCollapsed}>
                  <span className="swatch" style={{ background: g.color }} />
                  {renaming === g.folder ? null : <span className="gname">{g.name}</span>}
                  {!g.current && runningChats.some((r) => r.folder && r.folder.toLowerCase() === g.folder.toLowerCase()) ? <span className="spin small" title="Arbeitet im Hintergrund" /> : null}
                  <Icon name={isCollapsed ? 'chevRight' : 'chevDown'} size={14} className="chev" />
                </button>
                {renaming === g.folder ? <RenameField name={g.name} onDone={(name) => onRenameDone(g.folder, name)} /> : null}
                <span className="group-actions">
                  {!g.missing ? (
                    <button type="button" className="icon-btn gnew" title={`Neuer Chat in ${g.name}`} aria-label={`Neuer Chat in ${g.name}`}
                      onClick={() => onNewInProject(g.folder)}><Icon name="plus" size={14} /></button>
                  ) : null}
                  <button type="button" className="icon-btn" title="Mehr" aria-label={`Menü für ${g.name}`} onClick={() => onProjectMenu(g.folder)}><Icon name="more" size={14} /></button>
                </span>
              </div>
              {!isCollapsed ? (
                <ul className="chat-items">
                  {g.missing ? <li className="chat-empty">Ordner nicht gefunden</li>
                    : chats.length ? chats.map((c) => {
                      const active = g.current && c.id === chatId;
                      if (renamingChat && renamingChat.folder === g.folder && renamingChat.id === c.id) {
                        return (
                          <li key={c.id} className="chat-item renaming">
                            <RenameField name={c.title} className="chat-rename" onDone={async (title) => {
                              setRenamingChat(null);
                              if (title.trim() && title.trim() !== c.title) await api().chatRename(g.folder, c.id, title.trim());
                              onRefresh?.();
                            }} />
                          </li>
                        );
                      }
                      return (
                        <li key={c.id} className={`chat-item${active ? ' active' : ''}${c.placeholder ? ' placeholder' : ''}`} aria-current={active ? 'true' : undefined}
                          title={`${c.title}\n${c.count} ${c.count === 1 ? 'Auftrag' : 'Aufträge'} · ${formatTime(c.updated)} (${shortTime(c.updated)})\nRechtsklick: Menü`}
                          onClick={() => { if (!active) onOpenChat(g.folder, c.id); }}
                          onContextMenu={(e) => { if (!c.placeholder) openMenu(e, g.folder, c); else e.preventDefault(); }}>
                          {(active && running) || runningChats.some((r) => r.id === c.id) ? <span className="spin small" title="Arbeitet" aria-hidden="true" /> : null}
                          {c.pinned ? <Icon name="pin" size={12} className="cpin" title="Angeheftet" /> : null}
                          <span className="ctitle">{c.title}</span>
                          {!c.placeholder ? (
                            <button type="button" className="icon-btn cmenu-btn" aria-label={`Menü für ${c.title}`} aria-haspopup="menu" title="Umbenennen, Anheften, Löschen"
                              onClick={(e) => openMenu(e, g.folder, c)}><Icon name="more" size={14} /></button>
                          ) : null}
                        </li>
                      );
                    }) : <li className="chat-empty">Noch keine Chats</li>}
                </ul>
              ) : null}
            </section>
          );
        })}
        {hits.query && hits.query === query.trim() && hits.list.length ? (
          <section className="chat-group content-hits">
            <div className="group-head"><span className="group-toggle"><span className="gname">Im Inhalt gefunden</span></span></div>
            <ul className="chat-items">
              {hits.list.map((h) => (
                <li key={`${h.folder}-${h.id}`} className="chat-item hit" title={`${h.title}\n${h.name}`} onClick={() => onOpenChat(h.folder, h.id)}>
                  <div className="hit-top"><span className="ctitle">{h.title}</span><span className="hit-folder">{h.name}</span></div>
                  {h.snippets?.[0] ? <div className="hit-snippet">{h.snippets[0]}</div> : null}
                </li>
              ))}
            </ul>
          </section>
        ) : null}
        {!groups.length ? <div className="chat-empty">{q ? 'Keine Treffer' : 'Noch keine Projekte'}</div> : null}
        {q && groups.length && !groups.some((g) => g.chats.some((c) => c.title.toLowerCase().includes(q))) && !(hits.query === query.trim() && hits.list.length)
          ? <div className="chat-empty">Keine Treffer</div> : null}
      </div>
      {menu ? <ChatMenu menu={menu} onClose={() => setMenu(null)}
        onRename={() => setRenamingChat({ folder: menu.folder, id: menu.chat.id })}
        onPin={async () => { await api().chatPin(menu.folder, menu.chat.id, !menu.chat.pinned); onRefresh?.(); }}
        onDelete={() => onDeleteChat(menu.folder, menu.chat.id, menu.chat.title)}
        // Chat, der im Hintergrund arbeitet (parallele Chats): von hier aus anhalten
        onStop={runningChats.some((r) => r.id === menu.chat.id) ? () => api().chatStopChat(menu.folder, menu.chat.id) : null} /> : null}
      <div className="side-foot">
        {update && ['verfügbar', 'lädt', 'geladen', 'neustart'].includes(update.state) ? (
          <button type="button" id="update-pill" className="update-chip" onClick={onUpdate} disabled={update.state === 'lädt'}
            title={update.portable ? `ASK ${update.version} ist da (du hast ${update.current}). Klicken für den Download.` : `ASK ${update.version} ist da (du hast ${update.current}).`}>
            <Icon name="down" size={14} />
            <span id="update-text">{update.state === 'lädt' ? `Update lädt … ${update.percent} %`
              : update.state === 'verfügbar' ? `Update ${update.version}${/-/.test(update.version) ? ' (Beta)' : ''}` : 'Update bereit: neu starten'}</span>
          </button>
        ) : null}
        <div className="account">
          <span className="avatar" aria-hidden="true">{initials(ask.user)}</span>
          <div className="who">
            <div className="ell">{ask.user || 'ASK'}</div>
            {/* Status nur, wenn etwas nicht stimmt; sonst ruhig */}
            <div className="status">
              <button type="button" id="ask-pill" className={`dot-btn ${askOk ? 'ok quiet' : ask.state === 'fehler' ? 'bad' : 'warn'}`} onClick={() => onStatus('ask')}
                title={ask.message || (askOk ? `Angemeldet bei ${state.askUrl}` : '')}>{askOk ? 'Verbunden' : ask.state === 'fehler' ? 'ASK: Fehler' : 'Nicht angemeldet'}</button>
              {!termOk ? (
                <button type="button" id="terminal-pill" className={`dot-btn ${term.state === 'fehler' ? 'bad' : 'warn'}`}
                  onClick={() => onStatus('terminal')} title={term.error || 'Terminal'}>
                  {term.state === 'startet' ? (term.detail || 'Terminal startet') : term.state === 'fehler' ? 'Terminal: Fehler' : 'Terminal gestoppt'}
                </button>
              ) : <button type="button" id="terminal-pill" className="dot-btn ok quiet" hidden onClick={() => onStatus('terminal')}>Terminal</button>}
            </div>
          </div>
          <button type="button" className="icon-btn" title="Einstellungen" aria-label="Einstellungen" onClick={onSettings}><Icon name="settings" size={16} /></button>
        </div>
      </div>
    </aside>
  );
}

function initials(name) {
  const parts = String(name || '').trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return 'A';
  return (parts[0][0] + (parts[1]?.[0] || '')).toUpperCase();
}

// Menü eines Chats: Umbenennen, Anheften, Löschen (Pfeiltasten, Esc schliesst)
function ChatMenu({ menu, onClose, onRename, onPin, onDelete, onStop }) {
  const ref = useRef(null);
  useEffect(() => {
    ref.current?.querySelector('button')?.focus();
    const away = (e) => { if (!ref.current?.contains(e.target)) onClose(); };
    const key = (e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } };
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('mousedown', away); document.removeEventListener('keydown', key, true); };
  }, [onClose]);
  const item = (label, fn, cls = '') => (
    <button type="button" role="menuitem" className={cls} onClick={() => { onClose(); fn(); }}>{label}</button>
  );
  const onKey = (e) => {
    const items = [...ref.current.querySelectorAll('button')];
    const i = items.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); items[(i + (e.key === 'ArrowDown' ? 1 : -1) + items.length) % items.length].focus(); }
    if (e.key === 'Tab') onClose();
  };
  const left = Math.max(4, Math.min(menu.x, window.innerWidth - 190));
  const top = Math.max(4, Math.min(menu.y, window.innerHeight - 130));
  return (
    <div ref={ref} className="chat-menu popover" role="menu" aria-label="Chat" style={{ left, top }} onKeyDown={onKey}>
      {onStop ? item('Stoppen', onStop) : null}
      {item('Umbenennen', onRename)}
      {item(menu.chat.pinned ? 'Nicht mehr anheften' : 'Anheften', onPin)}
      {item('Löschen', onDelete, 'danger')}
    </div>
  );
}

function RenameField({ name, onDone: done, className = 'rename' }) {
  const [value, setValue] = useState(name);
  const finished = useRef(false);
  const onDone = (v) => { if (finished.current) return; finished.current = true; done(v); };
  return (
    <input className={className} value={value} autoFocus maxLength={200} onChange={(e) => setValue(e.target.value)} aria-label="Neuer Name"
      onFocus={(e) => e.target.select()} onBlur={() => onDone(value)}
      onKeyDown={(e) => { e.stopPropagation(); if (e.key === 'Enter') onDone(value); if (e.key === 'Escape') onDone(name); }} />
  );
}
