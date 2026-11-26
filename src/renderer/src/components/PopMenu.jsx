import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';

// Kleines Menü an der Mausposition (Rechtsklick) oder unter einem Knopf: Pfeiltasten wählen, → öffnet ein Untermenü,
// ← schliesst es, Esc und Klick daneben schliessen. Für Menüs der App und solche aus dem Hauptprozess (ENTSCHEIDE E56).
// items: [{ label, run, danger?, separator?, type?: 'radio'|'checkbox', checked?, enabled?, swatch?, hint?, submenu? }]; null wird übersprungen.
export default function PopMenu({ x, y, label, items, onClose }) {
  const ref = useRef(null);
  useEffect(() => {
    ref.current?.querySelector('button:not(:disabled)')?.focus();
    const away = (e) => { if (!ref.current?.contains(e.target)) onClose(); };
    const key = (e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } };
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('mousedown', away); document.removeEventListener('keydown', key, true); };
  }, [onClose]);
  const shown = tidy(items);
  // Erst an der gewünschten Stelle zeichnen, dann mit der echten Grösse ins Fenster schieben; Untermenüs nach links, wenn rechts kein Platz ist
  const [pos, setPos] = useState({ left: x, top: y, flip: false });
  useLayoutEffect(() => {
    const r = ref.current?.getBoundingClientRect();
    if (!r) return;
    const left = Math.max(4, Math.min(x, window.innerWidth - r.width - 4));
    const top = Math.max(4, Math.min(y, window.innerHeight - r.height - 4));
    setPos({ left, top, flip: left + r.width + 200 > window.innerWidth });
  }, [x, y]);
  return (
    <div ref={ref} className={`chat-menu popover${pos.flip ? ' flip' : ''}`} role="menu" aria-label={label} style={{ left: pos.left, top: pos.top }}>
      <MenuList items={shown} onClose={onClose} />
    </div>
  );
}

// Trennlinien nur zwischen Einträgen, nie am Anfang, am Ende oder doppelt
function tidy(items) {
  return items.filter(Boolean).filter((it, i, all) => !it.separator || (i > 0 && !all[i - 1].separator && all.slice(i + 1).some((x) => !x.separator)));
}

function MenuList({ items, onClose, onBack }) {
  const list = useRef(null);
  const [sub, setSub] = useState(-1);
  const marks = items.some((it) => it.type === 'radio' || it.type === 'checkbox'); // dann haben alle Einträge die Spalte für das Häkchen
  const onKey = (e) => {
    const btns = [...list.current.querySelectorAll(':scope > .mi > button:not(:disabled)')];
    const i = btns.indexOf(document.activeElement);
    if (i < 0) return; // Taste gehört zum Untermenü
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); e.stopPropagation(); btns[(i + (e.key === 'ArrowDown' ? 1 : -1) + btns.length) % btns.length].focus(); }
    if (e.key === 'ArrowLeft' && onBack) { e.preventDefault(); e.stopPropagation(); onBack(); }
    if (e.key === 'Tab') onClose();
  };
  return (
    <div ref={list} className="menu-list" onKeyDown={onKey}>
      {items.map((it, i) => {
        if (it.separator) return <hr key={`sep${i}`} className="menu-sep" />;
        const marked = it.type === 'radio' || it.type === 'checkbox';
        const open = () => { setSub(i); setTimeout(() => list.current?.querySelector(`.mi[data-i="${i}"] .chat-menu button:not(:disabled)`)?.focus(), 0); };
        return (
          <div key={`${i}-${it.label}`} className="mi" data-i={i} onMouseEnter={() => setSub(it.submenu ? i : -1)}>
            <button type="button" role={marked ? (it.type === 'radio' ? 'menuitemradio' : 'menuitemcheckbox') : 'menuitem'} aria-checked={marked ? !!it.checked : undefined}
              aria-haspopup={it.submenu ? 'menu' : undefined} aria-expanded={it.submenu ? sub === i : undefined}
              className={it.danger ? 'danger' : ''} disabled={it.enabled === false} title={it.label.length > 40 ? it.label : undefined}
              onClick={() => { if (it.submenu) { open(); return; } it.run?.(); onClose(); }}
              onKeyDown={(e) => { if (it.submenu && (e.key === 'ArrowRight' || e.key === 'Enter' || e.key === ' ')) { e.preventDefault(); e.stopPropagation(); open(); } }}>
              {marks ? <span className="mi-check">{marked && it.checked ? <Icon name="check" size={13} strokeWidth={2.2} /> : null}</span> : null}
              {it.swatch ? <span className="swatch" style={{ background: it.swatch }} /> : null}
              <span className="mi-label">{it.label}</span>
              {it.hint ? <kbd className="mi-hint">{it.hint}</kbd> : null}
              {it.submenu ? <Icon name="chevRight" size={13} className="mi-more" /> : null}
            </button>
            {it.submenu && sub === i ? (
              <div className="chat-menu popover sub" role="menu" aria-label={it.label}>
                <MenuList items={tidy(it.submenu)} onClose={onClose} onBack={() => { setSub(-1); list.current?.querySelector(`.mi[data-i="${i}"] > button`)?.focus(); }} />
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
