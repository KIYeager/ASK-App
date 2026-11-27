// Suche im sichtbaren Chat (Ctrl+F, ENTSCHEIDE E78): markiert Treffer und springt von einem zum nächsten.
// Markiert wird mit der CSS Custom Highlight API (::highlight in app.css): Sie färbt Textstellen, ohne das DOM zu
// ändern, das React verwaltet. Gesucht wird im sichtbaren Text; eingeklappte Arbeitsschritte zählen erst, wenn sie
// offen sind (Ctrl+O zeigt alle).
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';

const ALL = 'ask-find';
const CURRENT = 'ask-find-current';
const MAX_HITS = 2000; // genug für jeden Chat; schützt vor einem einzelnen Buchstaben in einem sehr langen Verlauf
const supported = () => typeof CSS !== 'undefined' && !!CSS.highlights && typeof Highlight !== 'undefined';

function clear() {
  if (!supported()) return;
  CSS.highlights.delete(ALL);
  CSS.highlights.delete(CURRENT);
}

// Alle Stellen im sichtbaren Text unter root, an denen needle vorkommt (ohne Gross- und Kleinschreibung)
function findRanges(root, needle) {
  const q = needle.toLowerCase();
  const out = [];
  if (!root || !q) return out;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node && out.length < MAX_HITS; node = walker.nextNode()) {
    const text = node.nodeValue.toLowerCase();
    let at = text.indexOf(q);
    if (at < 0) continue;
    // Nicht gezeichneter Text (eingeklappt, versteckt) zählt nicht: Dorthin liesse sich nicht springen
    const el = node.parentElement;
    if (!el || el.closest('[hidden]') || !el.getClientRects().length) continue;
    for (; at >= 0 && out.length < MAX_HITS; at = text.indexOf(q, at + q.length)) {
      const r = document.createRange();
      r.setStart(node, at);
      r.setEnd(node, at + q.length);
      out.push(r);
    }
  }
  return out;
}

// version und view ändern sich, wenn sich der Chat ändert (neue Antwort, anderer Chat, alle Schritte aufgeklappt)
export default function FindBar({ root, version, view, onClose, onJump }) {
  const [query, setQuery] = useState('');
  const [count, setCount] = useState(0);
  const [index, setIndex] = useState(0);
  const input = useRef(null);
  const ranges = useRef([]);
  const wanted = useRef(0); // Treffer, zu dem gesprungen werden soll; bleibt beim Nachsuchen möglichst derselbe

  useEffect(() => { input.current?.focus(); input.current?.select(); return clear; }, []);

  const show = (i, scroll) => {
    const list = ranges.current;
    if (!list.length) { setIndex(0); if (supported()) CSS.highlights.delete(CURRENT); return; }
    const at = ((i % list.length) + list.length) % list.length;
    wanted.current = at;
    setIndex(at);
    if (supported()) CSS.highlights.set(CURRENT, new Highlight(list[at]));
    if (scroll) {
      onJump?.();
      const box = root.current;
      const rect = list[at].getBoundingClientRect();
      if (box && rect) {
        const outer = box.getBoundingClientRect();
        // Treffer in die Mitte der Ansicht, wenn er nicht schon gut sichtbar ist
        if (rect.top < outer.top + 60 || rect.bottom > outer.bottom - 60) box.scrollTop += rect.top - outer.top - outer.height / 2;
      }
    }
  };

  // Suchen: kurz nach dem Tippen und wenn sich der Chat ändert
  useLayoutEffect(() => {
    const t = setTimeout(() => {
      const q = query.trim();
      const list = q ? findRanges(root.current, q) : [];
      ranges.current = list;
      setCount(list.length);
      if (supported()) {
        if (list.length) CSS.highlights.set(ALL, new Highlight(...list)); else CSS.highlights.delete(ALL);
      }
      show(Math.min(wanted.current, Math.max(0, list.length - 1)), false);
    }, 120);
    return () => clearTimeout(t);
  }, [query, version, view]);

  const step = (delta) => show(wanted.current + delta, true);
  const onKey = (e) => {
    if (e.key === 'Enter') { e.preventDefault(); if (ranges.current.length) step(e.shiftKey ? -1 : 1); }
    else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onClose(); }
  };
  const q = query.trim();
  return (
    <div id="find-bar" className="find-bar" role="search" aria-label="Im Chat suchen">
      <Icon name="search" size={14} />
      <input id="find-input" ref={input} type="text" value={query} spellCheck={false} autoComplete="off" placeholder="Im Chat suchen" aria-label="Im Chat suchen"
        onChange={(e) => { wanted.current = 0; setQuery(e.target.value); }} onKeyDown={onKey} />
      <span id="find-count" className={`find-count${q && !count ? ' none' : ''}`} aria-live="polite">
        {!q ? '' : count ? `${index + 1} von ${count}${count >= MAX_HITS ? '+' : ''}` : 'Kein Treffer'}
      </span>
      <button type="button" className="icon-btn" title="Vorheriger Treffer (Shift+Enter)" aria-label="Vorheriger Treffer" disabled={!count} onClick={() => step(-1)}><Icon name="chevUp" size={15} /></button>
      <button type="button" className="icon-btn" title="Nächster Treffer (Enter)" aria-label="Nächster Treffer" disabled={!count} onClick={() => step(1)}><Icon name="chevDown" size={15} /></button>
      <button type="button" className="icon-btn" title="Suche schliessen (Esc)" aria-label="Suche schliessen" onClick={onClose}><Icon name="close" size={14} /></button>
    </div>
  );
}
