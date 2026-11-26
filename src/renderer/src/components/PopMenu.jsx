import { useEffect, useRef } from 'react';

// Kleines Menü an der Mausposition (Rechtsklick) oder unter einem Knopf: Pfeiltasten wählen, Esc und Klick daneben schliessen.
// items: [{ label, run, danger?, separator? }]; null-Einträge werden übersprungen.
export default function PopMenu({ x, y, label, items, onClose }) {
  const ref = useRef(null);
  useEffect(() => {
    ref.current?.querySelector('button')?.focus();
    const away = (e) => { if (!ref.current?.contains(e.target)) onClose(); };
    const key = (e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } };
    document.addEventListener('mousedown', away);
    document.addEventListener('keydown', key, true);
    return () => { document.removeEventListener('mousedown', away); document.removeEventListener('keydown', key, true); };
  }, [onClose]);
  const onKey = (e) => {
    const list = [...ref.current.querySelectorAll('button')];
    const i = list.indexOf(document.activeElement);
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); list[(i + (e.key === 'ArrowDown' ? 1 : -1) + list.length) % list.length].focus(); }
    if (e.key === 'Tab') onClose();
  };
  // Trennlinien nur zwischen Einträgen, nie am Anfang, am Ende oder doppelt
  const shown = items.filter(Boolean).filter((it, i, all) => !it.separator || (i > 0 && !all[i - 1].separator && all.slice(i + 1).some((x) => !x.separator)));
  const left = Math.max(4, Math.min(x, window.innerWidth - 210));
  const top = Math.max(4, Math.min(y, window.innerHeight - 12 - shown.length * 32));
  return (
    <div ref={ref} className="chat-menu popover" role="menu" aria-label={label} style={{ left, top }} onKeyDown={onKey}>
      {shown.map((it, i) => (it.separator ? <hr key={`sep${i}`} className="menu-sep" /> : (
        <button key={it.label} type="button" role="menuitem" className={it.danger ? 'danger' : ''} onClick={() => { onClose(); it.run(); }}>{it.label}</button>
      )))}
    </div>
  );
}
