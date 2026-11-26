import { useEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';

// Dialog des Hauptprozesses in der eigenen Oberfläche statt im Fenster von Windows (ENTSCHEIDE E56): Updates, Rückfragen, Meldungen.
// ask: Optionen wie bei dialog.showMessageBox (message, detail, buttons, defaultId, cancelId, checkboxLabel), dazu dangerId
// (roter Knopf) und für Updates kind 'update', version, current, beta, intro, notes.
// onAnswer({ response, checkboxChecked }); Esc und Klick daneben geben cancelId.
const TYPE_ICON = { warning: 'alert', error: 'alert', question: 'help', info: 'bell' };

export default function AppAsk({ ask, onAnswer }) {
  const ref = useRef(null);
  const [checked, setChecked] = useState(!!ask.checkboxChecked);
  const buttons = ask.buttons?.length ? ask.buttons : ['OK'];
  const cancel = Number.isInteger(ask.cancelId) ? ask.cancelId : buttons.length - 1;
  const primary = Number.isInteger(ask.defaultId) ? ask.defaultId : 0;
  const danger = Number.isInteger(ask.dangerId) ? ask.dangerId : -1;
  const update = ask.kind === 'update';
  const answer = (response) => onAnswer({ response, checkboxChecked: response !== cancel && checked });

  useEffect(() => {
    // Bei «Löschen» und Ähnlichem steht der Fokus auf Abbrechen, damit Enter nichts Unumkehrbares auslöst
    const focus = danger >= 0 && buttons.length > 1 ? cancel : primary;
    ref.current?.querySelector(`[data-i="${focus}"]`)?.focus();
    const key = (e) => {
      if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onAnswer({ response: cancel, checkboxChecked: false }); }
      if (e.key === 'Tab') { // Fokus bleibt im Dialog
        const list = [...ref.current.querySelectorAll('button, input')];
        const i = list.indexOf(document.activeElement);
        e.preventDefault();
        list[(i + (e.shiftKey ? -1 : 1) + list.length) % list.length]?.focus();
      }
    };
    document.addEventListener('keydown', key, true);
    return () => document.removeEventListener('keydown', key, true);
  }, [ask, cancel, primary, danger, buttons.length, onAnswer]);

  const notes = String(ask.notes || '').split('\n').map((l) => l.trim()).filter(Boolean);
  const title = update ? `ASK ${ask.version} ist da` : ask.message;
  const text = update ? ask.intro : ask.detail;
  // Reihenfolge wie in den übrigen Dialogen der App: Abbrechen links, Hauptaktion rechts, die anderen dazwischen
  const rank = (i) => (i === cancel && buttons.length > 1 ? 0 : i === primary ? 2 : 1);
  const order = buttons.map((label, i) => ({ label, i })).sort((a, b) => rank(a.i) - rank(b.i) || a.i - b.i);
  const icon = update ? 'down' : danger >= 0 ? 'trash' : TYPE_ICON[ask.type] || 'bell';

  return (
    <div className="overlay app-ask-overlay" onMouseDown={(e) => { if (e.target === e.currentTarget) onAnswer({ response: cancel, checkboxChecked: false }); }}>
      <div ref={ref} className={`dialog app-ask${update ? ' update' : ''}${danger >= 0 ? ' danger' : ''} t-${ask.type || 'none'}`} role="alertdialog" aria-modal="true"
        aria-labelledby="app-ask-title" aria-describedby={text ? 'app-ask-text' : undefined}>
        <div className="app-ask-head">
          <span className="app-ask-icon" aria-hidden="true"><Icon name={icon} size={20} strokeWidth={2} /></span>
          <div className="app-ask-titles">
            <h1 id="app-ask-title">{title}{update && ask.beta ? <span className="app-ask-badge">Beta</span> : null}</h1>
            {update ? <div className="app-ask-versions">{ask.current} <Icon name="arrowRight" size={12} /> <strong>{ask.version}</strong></div> : null}
          </div>
        </div>
        <div className="dialog-body app-ask-body">
          {text ? <div id="app-ask-text" className="app-ask-text">{String(text).split(/\n{2,}/).map((p, i) => <p key={i}>{p}</p>)}</div> : null}
          {notes.length ? (
            <section className="app-ask-notes">
              <h2>Neu in dieser Version</h2>
              <ul>{notes.map((n, i) => <li key={i}>{n.replace(/^[•\-*]\s*/, '')}</li>)}</ul>
            </section>
          ) : null}
          {ask.checkboxLabel ? (
            <label className="check app-ask-check"><input type="checkbox" checked={checked} onChange={(e) => setChecked(e.target.checked)} /> {ask.checkboxLabel}</label>
          ) : null}
        </div>
        <div className="actions">
          {order.map(({ label, i }) => (
            <button key={i} data-i={i} type="button" onClick={() => answer(i)}
              className={`btn${i === danger ? ' danger' : (i === primary || buttons.length === 1) && i !== cancel ? ' primary' : buttons.length === 1 ? ' primary' : ''}`}>{label}</button>
          ))}
        </div>
      </div>
    </div>
  );
}
