// Entwicklungsserver des sichtbaren Projekts (ENTSCHEIDE E48): einklappbarer Bereich oben im Browser-Tab.
// Öffnen lädt die Adresse im Browser darunter, Ausgabe zeigt die letzten Zeilen, Stoppen beendet den Prozessbaum.
import { useEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';

const STATE_TEXT = { startet: 'startet …', läuft: 'läuft', gestoppt: 'gestoppt', fehler: 'Fehler' };
const STATE_CLASS = { startet: 'starting', läuft: 'up', gestoppt: 'stopped', fehler: 'failed' };
const active = (s) => s.state === 'startet' || s.state === 'läuft';

function loadOpen() {
  try { return localStorage.getItem('ask.server-offen') !== '0'; } catch { return true; }
}

function Output({ id, last }) {
  const [lines, setLines] = useState(null);
  const box = useRef(null);
  useEffect(() => {
    let alive = true;
    api().serverOutput(id).then((l) => { if (alive) setLines(l || []); });
    return () => { alive = false; };
  }, [id, last]);
  useEffect(() => {
    const el = box.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 40) el.scrollTop = el.scrollHeight; // unten bleiben, wenn man unten ist
  }, [lines]);
  useEffect(() => { if (box.current) box.current.scrollTop = box.current.scrollHeight; }, []);
  return <pre ref={box} className="srv-out">{lines === null ? 'Lädt …' : lines.join('\n') || '(noch keine Ausgabe)'}</pre>;
}

function ServerRow({ s, onOpen, notice }) {
  const [showOut, setShowOut] = useState(false);
  const [busy, setBusy] = useState(false);
  const stop = async () => {
    setBusy(true);
    const res = await api().serverStop(s.id);
    setBusy(false);
    if (!res?.ok && res?.reason) notice(res.reason);
  };
  const exit = !active(s) && s.exitCode !== null && s.exitCode !== undefined ? ` · Exitcode ${s.exitCode}` : '';
  return (
    <li className={`srv ${STATE_CLASS[s.state] || ''}`}>
      <div className="srv-row">
        <span className={`srv-dot ${STATE_CLASS[s.state] || ''}`} title={STATE_TEXT[s.state]} aria-hidden="true" />
        <span className="srv-main">
          <span className="srv-name ell" title={s.command}>{s.name}</span>
          {s.url && active(s) ? (
            <button type="button" className="link srv-url ell" title={`${s.url} im Browser öffnen`} onClick={() => onOpen(s.url)}>{s.url.replace(/^https?:\/\//, '').replace(/\/$/, '')}</button>
          ) : (
            <span className={`srv-state ell${s.state === 'fehler' ? ' bad' : ''}`} title={s.error || ''}>{s.state === 'fehler' && s.error ? s.error : `${STATE_TEXT[s.state]}${exit}`}</span>
          )}
        </span>
        <span className="srv-acts">
        {active(s) ? (
          <>
            <button type="button" className="btn small" disabled={!s.url} title={s.url ? '' : 'Noch keine Adresse erkannt'} onClick={() => onOpen(s.url)}>Öffnen</button>
            <button type="button" className={`btn small${showOut ? ' on' : ''}`} aria-expanded={showOut} onClick={() => setShowOut(!showOut)}>Ausgabe</button>
            <button type="button" className="btn small" disabled={busy} onClick={stop}>Stoppen</button>
          </>
        ) : (
          <>
            <button type="button" className={`btn small${showOut ? ' on' : ''}`} aria-expanded={showOut} onClick={() => setShowOut(!showOut)}>Ausgabe</button>
            <button type="button" className="icon-btn" title="Aus der Liste entfernen" aria-label={`${s.name} aus der Liste entfernen`} onClick={() => api().serverRemove(s.id)}><Icon name="close" size={14} /></button>
          </>
        )}
        </span>
      </div>
      {showOut ? <Output id={s.id} last={s.last} /> : null}
    </li>
  );
}

export default function ServerBar({ servers, onOpen, notice }) {
  const [open, setOpen] = useState(loadOpen);
  const [command, setCommand] = useState('');
  const [starting, setStarting] = useState(false);
  const running = servers.filter(active).length;
  // Startet das Modell einen Server, klappt der Bereich auf
  const seen = useRef(running);
  useEffect(() => {
    if (running > seen.current && !open) setOpen(true);
    seen.current = running;
  }, [running, open]);
  const toggle = () => {
    setOpen(!open);
    try { localStorage.setItem('ask.server-offen', open ? '0' : '1'); } catch { /* egal */ }
  };
  const start = async (e) => {
    e.preventDefault();
    const c = command.trim();
    if (!c || starting) return;
    setStarting(true);
    const res = await api().serverStart(c);
    setStarting(false);
    if (res?.ok) setCommand('');
    else notice(res?.reason || 'Der Server liess sich nicht starten.');
  };
  return (
    <section id="servers" className={`servers${open ? ' open' : ''}`} aria-label="Server">
      <button type="button" className="srv-head" aria-expanded={open} onClick={toggle}>
        <Icon name="server" size={14} />
        <span className="srv-title">Server</span>
        <span className="muted small">{running ? `${running} ${running === 1 ? 'läuft' : 'laufen'}` : servers.length ? 'keiner läuft' : ''}</span>
        <span className="spacer" />
        <Icon name={open ? 'chevUp' : 'chevDown'} size={13} />
      </button>
      {open ? (
        <div className="srv-body">
          {servers.length ? <ul className="srv-list">{servers.map((s) => <ServerRow key={s.id} s={s} onOpen={onOpen} notice={notice} />)}</ul> : null}
          <form className="srv-start" onSubmit={start}>
            <input id="srv-command" className="srv-input" spellCheck={false} autoComplete="off" placeholder="Befehl, z. B. npm run dev" aria-label="Befehl für den Server"
              value={command} onChange={(e) => setCommand(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setCommand(''); e.target.blur(); } }} />
            <button type="submit" className="btn small primary" disabled={!command.trim() || starting}>{starting ? 'Startet …' : 'Server starten'}</button>
          </form>
        </div>
      ) : null}
    </section>
  );
}
