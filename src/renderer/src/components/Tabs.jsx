import { useCallback, useState } from 'react';
import Icon from './Icon.jsx';
import PopMenu from './PopMenu.jsx';
import { RenameField } from './Sidebar.jsx';

// Tabs oben: zuletzt geöffnete Chats dieser Sitzung. Ziehen sortiert um, Mittelklick schliesst, Rechtsklick öffnet ein Menü,
// Doppelklick benennt um. Der aktive Tab liegt auf der Trennlinie und geht nahtlos in den Chat über.
export default function Tabs({ tabs, chatId, colorOf, isBusy, onOpen, onClose, onCloseMany, onMove, onRename, onNew, onStop }) {
  const [menu, setMenu] = useState(null); // { tab, index, x, y }
  const [renaming, setRenaming] = useState(null);
  const [drag, setDrag] = useState(null); // { id, over, after }
  const closeMenu = useCallback(() => setMenu(null), []);

  const dropAt = (e, t) => {
    if (!drag || drag.id === t.id) return;
    e.preventDefault();
    const r = e.currentTarget.getBoundingClientRect();
    const after = e.clientX > r.left + r.width / 2;
    if (drag.over !== t.id || drag.after !== after) setDrag({ ...drag, over: t.id, after });
  };

  return (
    <nav className="tabs" aria-label="Offene Chats" onDragEnd={() => setDrag(null)}>
      {tabs.map((t, i) => {
        const on = t.id === chatId;
        const closable = !on || tabs.length > 1;
        const cls = `tab${on ? ' on' : ''}${drag?.id === t.id ? ' dragging' : ''}${drag?.over === t.id ? (drag.after ? ' drop-after' : ' drop-before') : ''}`;
        return (
          <span key={t.id} className={cls} draggable={!t.placeholder && renaming !== t.id}
            onDragStart={(e) => { e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', t.title); setDrag({ id: t.id }); }}
            onDragOver={(e) => dropAt(e, t)}
            onDrop={(e) => { e.preventDefault(); if (drag && drag.over) onMove(drag.id, drag.over, drag.after); setDrag(null); }}
            onMouseDown={(e) => { if (e.button === 1) e.preventDefault(); }} // kein Rollen-Symbol beim Mittelklick
            onAuxClick={(e) => { if (e.button === 1 && closable) { e.preventDefault(); onClose(t, i); } }}
            onContextMenu={(e) => { e.preventDefault(); setMenu({ tab: t, index: i, x: e.clientX, y: e.clientY }); }}>
            {renaming === t.id ? (
              <span className="tab-open">
                <span className="swatch" style={{ background: colorOf(t) }} />
                <RenameField name={t.title} className="tab-rename" onDone={(title) => { setRenaming(null); if (title.trim() && title.trim() !== t.title) onRename(t, title.trim()); }} />
              </span>
            ) : (
              <button type="button" className="tab-open" title={`${t.title}\n${t.folder}\nDoppelklick: umbenennen · Rechtsklick: Menü`} aria-current={on ? 'page' : undefined}
                onClick={() => { if (!on) onOpen(t); }} onDoubleClick={() => { if (!t.placeholder) setRenaming(t.id); }}>
                <span className={`swatch${isBusy(t) ? ' busy' : ''}`} style={{ background: colorOf(t) }} />
                <span className="ell">{t.title}</span>
              </button>
            )}
            {closable && renaming !== t.id ? (
              <button type="button" className="tab-x" aria-label={`${t.title} schliessen`} title="Schliessen (Ctrl+W)" onClick={() => onClose(t, i)}><Icon name="close" size={12} /></button>
            ) : null}
          </span>
        );
      })}
      <button type="button" className="icon-btn tab-new" title="Neuer Chat (Ctrl+N)" aria-label="Neuer Chat" onClick={onNew}><Icon name="plus" size={15} /></button>
      {menu ? <PopMenu x={menu.x} y={menu.y} label="Tab" onClose={closeMenu} items={[
        isBusy(menu.tab) && onStop ? { label: 'Stoppen', run: () => onStop(menu.tab) } : null,
        !menu.tab.placeholder ? { label: 'Umbenennen', run: () => setRenaming(menu.tab.id) } : null,
        tabs.length > 1 ? { label: 'Schliessen', run: () => onClose(menu.tab, menu.index) } : null,
        tabs.length > 1 ? { label: 'Andere Tabs schliessen', run: () => onCloseMany(tabs.filter((x) => x.id !== menu.tab.id), menu.tab) } : null,
        menu.index < tabs.length - 1 ? { label: 'Tabs rechts davon schliessen', run: () => onCloseMany(tabs.slice(menu.index + 1), menu.tab) } : null,
        { separator: true },
        { label: 'Neuer Chat', run: onNew },
      ]} /> : null}
    </nav>
  );
}
