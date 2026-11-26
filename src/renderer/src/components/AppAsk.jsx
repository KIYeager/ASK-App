import { useEffect, useRef } from 'react';
import Icon from './Icon.jsx';

// Dialog des Hauptprozesses in der eigenen Oberfläche statt im Fenster von Windows (Updates und Meldungen).
// ask: Optionen wie bei dialog.showMessageBox, dazu für Updates kind 'update', version, current, beta, intro, notes.
// onAnswer(index): Index des Knopfs; Esc und Klick daneben geben cancelId (sonst den letzten Knopf).
const TYPE_ICON = { warning: 'alert', error: 'alert', question: 'help', info: 'bell' };

export default function AppAsk({ ask, onAnswer }) {
  const ref = useRef(null);
  const buttons = ask.buttons?.length ? ask.buttons : ['OK'];
  const cancel = Number.isInteger(ask.cancelId) ? ask.cancelId : buttons.length - 1;
  const primary = Number.isInteger(ask.defaultId) ? ask.defaultId : 0;
  const update = ask.kind === 'update';

  useEffect(() => {
    ref.current?.querySelector('.btn.primary, .btn')?.focus();
    const key = (e) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onAnswer(cancel); }
      if (e.key === 'Tab') { // Fokus bleibt im Dialog
        const list = [...ref.current.querySelectorAll('button')];
        const i = list.indexOf(document.activeElement);
        e.preventDefault();
        list[(i + (e.shiftKey ? -1 : 1) + list.length) % list.length]?.focus();
      }
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, [ask, cancel, onAnswer]);

  const notes = String(ask.notes || '').split('\n').map((l) => l.trim()).filter(Boolean);
  const title = update ? `ASK ${ask.version} ist da` : ask.message;
  const text = update ? ask.intro : ask.detail;
  // Reihenfolge der Knöpfe: Abbrechen links, Hauptaktion rechts (wie die übrigen Dialoge der App)
  const order = buttons.map((label, i) => ({ label, i })).sort((a, b) => (a.i === primary) - (b.i === primary));

  return (
    <div className="overlay app-ask-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onAnswer(cancel); }}>
      <div ref={ref} className={`dialog app-ask${update ? ' update' : ''} t-${ask.type || 'none'}`} role="alertdialog" aria-modal="true"
        aria-labelledby="app-ask-title" aria-describedby="app-ask-text">
        <div className="app-ask-head">
          <span className="app-ask-icon" aria-hidden="true"><Icon name={update ? 'down' : TYPE_ICON[ask.type] || 'bell'} size={20} strokeWidth={2} /></span>
          <div className="app-ask-titles">
            <h1 id="app-ask-title">{title}{update && ask.beta ? <span className="app-ask-badge">Beta</span> : null}</h1>
            {update ? <div className="app-ask-versions">{ask.current} <Icon name="arrowRight" size={12} /> <strong>{ask.version}</strong></div> : null}
          </div>
        </div>
        <div className="dialog-body app-ask-body">
          {text ? String(text).split(/\n{2,}/).map((p, i) => <p key={i} id={i === 0 ? 'app-ask-text' : undefined}>{p}</p>) : null}
          {notes.length ? (
            <section className="app-ask-notes">
              <h2>Neu in dieser Version</h2>
              <ul>{notes.map((n, i) => <li key={i}>{n.replace(/^[•\-*]\s*/, '')}</li>)}</ul>
            </section>
          ) : null}
        </div>
        <div className="actions">
          {order.map(({ label, i }) => (
            <button key={i} type="button" className={`btn${i === primary && buttons.length > 1 ? ' primary' : ''}${buttons.length === 1 ? ' primary' : ''}`}
              onClick={() => onAnswer(i)}>{label}</button>
          ))}
        </div>
      </div>
    </div>
  );
}
