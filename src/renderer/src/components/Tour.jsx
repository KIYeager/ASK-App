import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { api } from '../api.js';

// Einführung beim ersten Start (ENTSCHEIDE E49): vier Karten, die auf Teile der Oberfläche zeigen.
// Wenig Text, «Weiter» und «Überspringen»; gemerkt in prefs.tourDone, wieder zu öffnen in den Einstellungen.
const STEPS = [
  { target: ['#chatbar'], title: 'Projekte',
    text: 'Links stehen deine Projekte mit ihren Chats. Ein Projekt ist ein Ordner auf deinem Laptop.' },
  { target: ['#input-box'], title: 'Auftrag',
    text: 'Hier beschreibst du, was ASK tun soll. Im Feld wählst du Web, Denkaufwand und Modell; mit / gibt es Befehle.' },
  { target: ['.work', '#messages .msg', '#transcript'], title: 'Im Chat',
    text: 'ASK zeigt jeden Arbeitsschritt. Fragt es etwas oder will es löschen, antwortest du direkt im Chat.' },
  { target: ['#sidebar'], title: 'Dateien',
    text: 'Rechts siehst du, was im Projekt entsteht. «Rückgängig» unter einem Auftrag holt den Stand davor zurück.' },
];

const CARD_W = 300;
const GAP = 14;
const PAD = 6; // Abstand des Rahmens um das Ziel

function findTarget(selectors) {
  for (const sel of selectors) {
    const el = document.querySelector(sel);
    const r = el?.getBoundingClientRect();
    if (r && r.width > 0 && r.height > 0) return { x: r.left, y: r.top, w: r.width, h: r.height };
  }
  return null;
}

// Karte neben das Ziel: rechts oder links bei hohen Zielen, sonst darüber oder darunter, notfalls hinein
function place(r) {
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const clampX = (x) => Math.max(GAP, Math.min(vw - CARD_W - GAP, x));
  const top = Math.max(GAP, Math.min(vh - 220, r.y + 24));
  if (r.h > vh * 0.4 && vw - (r.x + r.w) >= CARD_W + 2 * GAP) return { left: r.x + r.w + GAP, top };
  if (r.h > vh * 0.4 && r.x >= CARD_W + 2 * GAP) return { left: r.x - CARD_W - GAP, top };
  if (r.y >= 200) return { left: clampX(r.x + r.w / 2 - CARD_W / 2), bottom: vh - r.y + GAP };
  if (vh - (r.y + r.h) >= 200) return { left: clampX(r.x + r.w / 2 - CARD_W / 2), top: r.y + r.h + GAP };
  return { left: clampX(r.x + r.w / 2 - CARD_W / 2), top: Math.max(GAP, r.y + r.h / 2 - 90) };
}

export default function Tour({ state, onDone }) {
  const [step, setStep] = useState(0);
  const [rect, setRect] = useState(null);
  const next = useRef(null);
  const s = STEPS[step];
  const last = step === STEPS.length - 1;

  // Beide Seiten zeigen, damit die Karten auf etwas zeigen können
  useEffect(() => {
    if (state.chatbarOpen === false) api().setChatbar(true);
    if (state.sidebarOpen === false) api().setSidebar(true);
  }, []);

  // Ziel messen; die Oberfläche kann sich während der Einführung verschieben (Seiten gehen auf, Fenstergrösse)
  useLayoutEffect(() => {
    const measure = () => setRect((old) => {
      const r = findTarget(s.target);
      return JSON.stringify(r) === JSON.stringify(old) ? old : r;
    });
    measure();
    const timer = setInterval(measure, 250);
    window.addEventListener('resize', measure);
    return () => { clearInterval(timer); window.removeEventListener('resize', measure); };
  }, [step]);

  useEffect(() => { next.current?.focus(); }, [step]);

  const forward = () => (last ? onDone() : setStep(step + 1));
  useEffect(() => {
    const onKey = (e) => {
      if (e.key === 'Escape') { e.preventDefault(); onDone(); }
      else if (e.key === 'ArrowRight') forward();
      else if (e.key === 'ArrowLeft' && step > 0) setStep(step - 1);
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  });

  const pos = rect ? place(rect) : null;
  return (
    <div id="tour" className={`tour${rect ? '' : ' dim'}`}>
      {rect ? <div className="tour-spot" style={{ left: rect.x - PAD, top: rect.y - PAD, width: rect.w + 2 * PAD, height: rect.h + 2 * PAD }} /> : null}
      <div className={`tour-card${pos ? '' : ' center'}`} style={pos || undefined} role="dialog" aria-modal="true"
        aria-labelledby="tour-title" aria-describedby="tour-text">
        <div className="tour-count">{step + 1} / {STEPS.length}</div>
        <h1 id="tour-title">{s.title}</h1>
        <p id="tour-text">{s.text}</p>
        <div className="tour-actions">
          <div className="tour-dots" aria-hidden="true">{STEPS.map((x, i) => <span key={x.title} className={i === step ? 'on' : ''} />)}</div>
          <span className="spacer" />
          {!last ? <button type="button" id="tour-skip" className="ghost-btn" onClick={onDone}>Überspringen</button> : null}
          <button type="button" id="tour-next" ref={next} className="btn small primary" onClick={forward}>{last ? 'Fertig' : 'Weiter'}</button>
        </div>
      </div>
    </div>
  );
}
