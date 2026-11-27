import { useCallback, useEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import PopMenu from './PopMenu.jsx';
import { api } from '../api.js';
import { formatTime, shortTime } from '../lib/format.js';

// Linke Leiste: Projekte (ein Projekt = ein Arbeitsordner) mit ihren Chats
export default function Sidebar({ groups, chatId, running, runningChats = [], query, setQuery, renaming, onRenameDone, onNewChat, onOpenChat, onDeleteChat,
  onNewProject, onProjectMenu, onNewInProject, state, onSettings, onStatus, onOpen, update, onUpdate, onRefresh }) {
  const [collapsed, setCollapsed] = useState(() => new Set());
  const [profile, setProfile] = useState(null); // Menü am Profil: { x, y }
  const [menu, setMenu] = useState(null); // { folder, chat, x, y }
  const closeMenu = useCallback(() => setMenu(null), []);
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
  // Menü am Profil wie in Claude: Konto, Einstellungen, Abkürzungen, Hilfe, Version
  const profileItems = () => {
    const u = state.update || {};
    let host = state.askUrl || '';
    try { host = new URL(state.askUrl).host; } catch { /* keine Adresse */ }
    return [
      { heading: true, label: ask.user || 'ASK', sub: askOk ? host : 'Nicht angemeldet' },
      { icon: 'settings', label: 'Einstellungen', hint: 'Ctrl+,', run: () => onSettings() },
      { icon: 'chart', label: 'Nutzung', run: () => onOpen({ name: 'settings', pane: 'nutzung' }) },
      { separator: true },
      { icon: 'bolt', label: 'Skills', run: () => onOpen({ name: 'settings', pane: 'skills' }) },
      { icon: 'puzzle', label: 'Mods', hint: state.modsCode ? 'Code an' : '', run: () => onOpen({ name: 'settings', pane: 'mods' }) },
      { icon: 'pin', label: 'Gedächtnis', run: () => onOpen({ name: 'memory' }) },
      { icon: 'github', label: 'GitHub', run: () => onOpen({ name: 'github' }) },
      { separator: true },
      { icon: 'help', label: 'Hilfe und Einführung', run: () => onOpen({ name: 'tour' }) },
      { icon: 'keyboard', label: 'Tastenkombinationen', run: () => onOpen({ name: 'shortcuts' }) },
      state.releasesUrl ? { icon: 'gift', label: 'Neu in ASK', run: () => api().openLink(state.releasesUrl) } : null,
      { separator: true },
      u.enabled ? { icon: 'reload', label: 'Nach Updates suchen', hint: `ASK ${state.version || ''}`, run: () => api().updateAction() }
        : { icon: 'info', label: `ASK ${state.version || ''}`, enabled: false },
      u.enabled ? { icon: 'spark', label: 'Beta-Versionen erhalten', hint: u.beta ? 'An' : 'Aus', run: () => api().setBetaUpdates(!u.beta, true) } : null,
      { separator: true },
      askOk ? { icon: 'logout', label: 'Abmelden', run: () => api().logout() } : { icon: 'logout', label: 'Anmelden …', run: () => onStatus('ask') },
    ];
  };
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
      {menu ? <PopMenu x={menu.x} y={menu.y} label="Chat" onClose={closeMenu} items={[
        // Chat, der im Hintergrund arbeitet (parallele Chats): von hier aus anhalten
        runningChats.some((r) => r.id === menu.chat.id) ? { label: 'Stoppen', run: () => api().chatStopChat(menu.folder, menu.chat.id) } : null,
        { label: 'Umbenennen', run: () => setRenamingChat({ folder: menu.folder, id: menu.chat.id }) },
        { label: menu.chat.pinned ? 'Nicht mehr anheften' : 'Anheften', run: async () => { await api().chatPin(menu.folder, menu.chat.id, !menu.chat.pinned); onRefresh?.(); } },
        { label: 'Löschen', danger: true, run: () => onDeleteChat(menu.folder, menu.chat.id, menu.chat.title) },
      ]} /> : null}
      <div className="side-foot">
        {/* Nur solange das Update noch nicht geladen wird; danach zeigt die Update-Karte den Stand */}
        {update && update.state === 'verfügbar' ? (
          <button type="button" id="update-pill" className="update-chip" onClick={onUpdate}
            title={update.portable ? `ASK ${update.version} ist da (du hast ${update.current}). Klicken für den Download.` : `ASK ${update.version} ist da (du hast ${update.current}).`}>
            <Icon name="down" size={14} />
            <span id="update-text">{`Update ${update.version}${/-/.test(update.version) ? ' (Beta)' : ''}`}</span>
          </button>
        ) : null}
        {/* Terminal und Python nur, wenn etwas nicht stimmt; sonst ruhig */}
        {!termOk || state.python ? (
          <div className="side-status">
            {!termOk ? (
              <button type="button" id="terminal-pill" className={`dot-btn ${term.state === 'fehler' ? 'bad' : 'warn'}`}
                onClick={() => onStatus('terminal')} title={term.error || 'Terminal'}>
                {term.state === 'startet' ? (term.detail || 'Terminal startet') : term.state === 'fehler' ? 'Terminal: Fehler' : 'Terminal gestoppt'}
              </button>
            ) : null}
            {/* Python wird nach einer neuen Fassung einmal entpackt (E68); nur Befehle mit Python warten darauf */}
            {state.python ? (
              <span id="python-pill" className="dot-btn warn" title="Einmalig nach einem Update. Befehle mit Python warten, bis es fertig ist.">
                {`Python wird eingerichtet … ${state.python.percent || 0} %`}
              </span>
            ) : null}
          </div>
        ) : null}
        {/* Das ganze Profil ist ein Knopf und öffnet ein Menü (ENTSCHEIDE E73) */}
        <button type="button" id="account-btn" className={`account${profile ? ' open' : ''}`} aria-haspopup="menu" aria-expanded={!!profile}
          title={ask.message || (askOk ? `Angemeldet bei ${state.askUrl}` : 'Konto und Einstellungen')}
          onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setProfile(profile ? null : { x: r.left, y: r.top - 6, w: r.width }); }}>
          <span className="avatar" aria-hidden="true">{initials(ask.user)}</span>
          <span className="who">
            <span className="ell name">{ask.user || 'ASK'}</span>
            <span id="ask-pill" className={`status-text ${askOk ? 'ok' : ask.state === 'fehler' ? 'bad' : 'warn'}`}>
              {askOk ? 'Verbunden' : ask.state === 'fehler' ? 'ASK: Fehler' : 'Nicht angemeldet'}
            </span>
          </span>
          {state.modsCode ? <span className="mods-dot" title="Ein Mod mit Code ist eingeschaltet"><Icon name="puzzle" size={13} /></span> : null}
          <Icon name="chevUpDown" size={14} className="account-chev" />
        </button>
        {profile ? <PopMenu x={profile.x} y={profile.y} above label="Konto" className="profile-menu" onClose={() => setProfile(null)} items={profileItems()} /> : null}
      </div>
    </aside>
  );
}

function initials(name) {
  const parts = String(name || '').trim().split(/\s+/).filter(Boolean);
  if (!parts.length) return 'A';
  return (parts[0][0] + (parts[1]?.[0] || '')).toUpperCase();
}

export function RenameField({ name, onDone: done, className = 'rename' }) {
  const [value, setValue] = useState(name);
  const finished = useRef(false);
  const onDone = (v) => { if (finished.current) return; finished.current = true; done(v); };
  return (
    <input className={className} value={value} autoFocus maxLength={200} onChange={(e) => setValue(e.target.value)} aria-label="Neuer Name"
      onFocus={(e) => e.target.select()} onBlur={() => onDone(value)}
      onKeyDown={(e) => { e.stopPropagation(); if (e.key === 'Enter') onDone(value); if (e.key === 'Escape') onDone(name); }} />
  );
}
