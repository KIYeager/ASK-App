import { useState } from 'react';
import Icon from './Icon.jsx';
import { formatTime, shortTime } from '../lib/format.js';

// Linke Leiste: Projekte (ein Projekt = ein Arbeitsordner) mit ihren Chats
export default function Sidebar({ groups, chatId, running, query, setQuery, renaming, onRenameDone, onNewChat, onOpenChat, onDeleteChat,
  onNewProject, onProjectMenu, onNewInProject, state, onSettings, onStatus, update, onUpdate }) {
  const [collapsed, setCollapsed] = useState(() => new Set());
  const q = query.trim().toLowerCase();
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
                      return (
                        <li key={c.id} className={`chat-item${active ? ' active' : ''}${c.placeholder ? ' placeholder' : ''}`}
                          title={`${c.title}\n${c.count} ${c.count === 1 ? 'Auftrag' : 'Aufträge'} · ${formatTime(c.updated)} (${shortTime(c.updated)})\nRechtsklick: löschen`}
                          onClick={() => { if (!active) onOpenChat(g.folder, c.id); }}
                          onContextMenu={(e) => { e.preventDefault(); if (!c.placeholder) onDeleteChat(g.folder, c.id, c.title); }}>
                          {active && running ? <span className="spin small" aria-hidden="true" /> : null}
                          <span className="ctitle">{c.title}</span>
                        </li>
                      );
                    }) : <li className="chat-empty">Noch keine Chats</li>}
                </ul>
              ) : null}
            </section>
          );
        })}
        {!groups.length ? <div className="chat-empty">{q ? 'Keine Treffer' : 'Noch keine Projekte'}</div> : null}
      </div>
      <div className="side-foot">
        {update && ['bereit', 'verfuegbar', 'laedt'].includes(update.state) ? (
          <button type="button" className="update-chip" onClick={onUpdate} disabled={update.state === 'laedt'}>
            <Icon name="down" size={14} />
            {update.state === 'laedt' ? `Version ${update.available} lädt … ${update.progress || 0} %`
              : update.state === 'bereit' ? `Version ${update.available} bereit · Neu starten` : `Version ${update.available} herunterladen`}
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
                  {term.state === 'startet' ? 'Terminal startet' : term.state === 'fehler' ? 'Terminal: Fehler' : 'Terminal gestoppt'}
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

function RenameField({ name, onDone }) {
  const [value, setValue] = useState(name);
  return (
    <input className="rename" value={value} autoFocus onChange={(e) => setValue(e.target.value)} aria-label="Projektname"
      onFocus={(e) => e.target.select()} onBlur={() => onDone(value)}
      onKeyDown={(e) => { if (e.key === 'Enter') onDone(value); if (e.key === 'Escape') onDone(name); }} />
  );
}
