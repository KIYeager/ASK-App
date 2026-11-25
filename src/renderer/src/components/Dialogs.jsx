import { useEffect, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { baseName } from '../lib/format.js';
import { PreviewBody } from './RightPanel.jsx';

function Dialog({ id, title, wide, full, children, actions, onClose }) {
  return (
    <div id={id} className={`dialog${wide ? ' wide' : ''}${full ? ' full' : ''}`} role="dialog" aria-modal="true" aria-labelledby={`${id}-title`}>
      <div className="dialog-head">
        <h1 id={`${id}-title`}>{title}</h1>
        {onClose ? <button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={16} /></button> : null}
      </div>
      <div className="dialog-body">{children}</div>
      {actions ? <div className="actions">{actions}</div> : null}
    </div>
  );
}

function WarningDialog() {
  const [ok, setOk] = useState(false);
  return (
    <Dialog id="dlg-warning" title="Bevor es losgeht"
      actions={<button id="warning-ok" type="button" className="btn primary" disabled={!ok} onClick={() => api().acceptWarning()}>Weiter</button>}>
      <p>Mit ASK arbeitet das Sprachmodell selbständig in einem Ordner auf deinem Laptop. Es kann dort Dateien lesen, neue Dateien schreiben, Programme ausführen und Dateien <strong>ändern oder löschen</strong>. Vor dem Löschen und Überschreiben fragt es dich.</p>
      <p>Arbeite deshalb nur mit <strong>Kopien</strong>. Lege für jedes Projekt einen eigenen Ordner an und kopiere nur die Dateien hinein, die das Modell braucht. Wichtige Originale bleiben an ihrem Ort.</p>
      <p>Ordner in OneDrive oder SharePoint lassen sich nicht wählen, weil Änderungen dort sofort in die Cloud übertragen würden. Ausserhalb des gewählten Ordners kann das Modell nichts schreiben, lesen kann es aber auch andere Dateien auf dem Laptop.</p>
      <p>Was das Modell aus deinen Dateien liest, wird an ASK geschickt. Lege keine vertraulichen oder persönlichen Daten in den Ordner.</p>
      <p>Prüfe die Ergebnisse, bevor du sie abgibst oder weitergibst.</p>
      <label className="check"><input type="checkbox" id="warning-check" checked={ok} onChange={(e) => setOk(e.target.checked)} /> Ich habe das verstanden.</label>
    </Dialog>
  );
}

function UrlDialog({ state }) {
  const [url, setUrl] = useState(state.askUrl || '');
  const [error, setError] = useState('');
  const save = async () => {
    const res = await api().setAskUrl(url);
    setError(res.ok ? '' : res.reason || '');
  };
  return (
    <Dialog id="dlg-url" title="Adresse von ASK" actions={<button id="url-ok" type="button" className="btn primary" onClick={save}>Speichern</button>}>
      <p>Gib die Adresse ein, unter der du ASK im Browser öffnest.</p>
      <input id="url-input" type="url" placeholder="https://…" spellCheck={false} value={url} onChange={(e) => setUrl(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') save(); }} />
      {error ? <p id="url-error" className="error">{error}</p> : null}
    </Dialog>
  );
}

// Neues Projekt: Name, daraus der Ordner unter ASK-Projekte. Oder einen vorhandenen Ordner als Projekt öffnen.
function ProjectDialog({ state, first, error: startError, onResult, onClose }) {
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const run = async (action) => {
    setError('');
    const res = await action();
    if (res && !res.ok && !res.canceled) setError(res.reason || 'Das ging nicht.');
    else if (res?.ok) onResult?.();
  };
  const err = error || startError || (first && state.terminal?.state === 'fehler' ? state.terminal.error : '');
  const recent = (state.recent || []).filter((f) => f !== state.folder);
  return (
    <Dialog id={first ? 'dlg-start' : 'dlg-project'} title={first ? 'Dein erstes Projekt' : 'Neues Projekt'} onClose={first ? null : onClose}>
      <p>{first ? 'ASK arbeitet in einem Projektordner auf deinem Laptop. Gib dem Projekt einen Namen; der Ordner entsteht unter ' : 'Der Ordner entsteht unter '}<code>ASK-Projekte</code> in deinem Benutzerordner.</p>
      <form className="row-form" onSubmit={(e) => { e.preventDefault(); run(() => api().newFolder(name)); }}>
        <input id="project-name" value={name} onChange={(e) => setName(e.target.value)} placeholder="z. B. Modul 293 Webauftritt" autoFocus aria-label="Name des Projekts" maxLength={80} />
        <button id="start-new" type="submit" className="btn primary">Projekt anlegen</button>
      </form>
      <div className="or"><span>oder</span></div>
      <button id="start-choose" type="button" className="btn" onClick={() => run(() => api().chooseFolder())}><Icon name="folderOpen" size={15} /> Vorhandenen Ordner als Projekt öffnen …</button>
      {err ? <p id="start-error" className="error">{err}</p> : null}
      {first && recent.length ? (
        <>
          <h2 id="recent-title">Zuletzt verwendet</h2>
          <ul id="recent-list" className="recent">
            {recent.map((f) => (
              <li key={f}>
                <button type="button" className="link" title={f} onClick={() => run(() => api().openFolder(f))}>{f}</button>
                <button type="button" className="btn small" title="Aus der Liste entfernen" onClick={() => api().forgetFolder(f)}>Entfernen</button>
              </li>
            ))}
          </ul>
        </>
      ) : null}
    </Dialog>
  );
}

function AskDialog({ state, onClose }) {
  const a = state.ask || {};
  return (
    <Dialog id="dlg-ask" title="ASK" onClose={onClose} actions={(
      <>
        {a.state === 'angemeldet' ? <button id="ask-logout" type="button" className="btn" onClick={async () => { onClose(); await api().logout(); }}><Icon name="logout" size={15} /> Abmelden</button> : null}
        <button id="ask-retry" type="button" className="btn" onClick={async () => { await api().connectAsk(); await api().models(); }}>Erneut prüfen</button>
        <button id="ask-close" type="button" className="btn primary" onClick={onClose}>Schliessen</button>
      </>
    )}>
      <p>Adresse: {state.askUrl}</p>
      {a.state === 'angemeldet' ? (
        <>
          <p>Angemeldet als {a.user || '(unbekannt)'}. {state.models.length} Modell(e) verfügbar.</p>
          <p>Die App schickt deine Aufträge direkt an das gewählte Modell. Befehle und Dateizugriffe führt sie selbst auf diesem Laptop aus, nur im Arbeitsordner.</p>
        </>
      ) : a.state === 'fehler' ? <p>ASK meldet einen Fehler: {a.message || 'unbekannt'}</p> : <p>Nicht angemeldet. Die Anmeldeseite von ASK erscheint im Hauptfenster.</p>}
    </Dialog>
  );
}

function TerminalDialog({ state, text, onClose }) {
  const [logs, setLogs] = useState('');
  const t = state.terminal || {};
  useEffect(() => { api().terminalLogs().then((l) => setLogs(l || '(keine Meldungen)')); }, [t.state]);
  return (
    <Dialog id="dlg-terminal" title={text ? 'Hinweis' : 'Terminal'} wide onClose={onClose} actions={(
      <>
        {!text ? <button id="terminal-restart" type="button" className="btn" onClick={() => { api().restartTerminal(); onClose(); }}>Neu starten</button> : null}
        <button id="terminal-close" type="button" className="btn primary" onClick={onClose}>Schliessen</button>
      </>
    )}>
      <p id="terminal-detail">{text || (t.state === 'läuft'
        ? `Das Terminal läuft im Ordner ${t.folder}. ${t.lowIntegrity ? 'Ausserhalb des Ordners kann es nichts schreiben oder löschen.' : ''}`
        : (t.error || `Status: ${t.state}`))}</p>
      {!text ? <pre id="terminal-logs" className="logs">{logs}</pre> : null}
    </Dialog>
  );
}

function GithubDialog({ onClose }) {
  const [s, setS] = useState(null);
  const [token, setToken] = useState('');
  const [error, setError] = useState('');
  const load = () => api().githubStatus().then(setS);
  useEffect(() => { load(); }, []);
  const connected = !!(s?.connected && s.login);
  const connect = async () => {
    const res = await api().githubConnect(token);
    if (!res.ok) { setError(res.message); return; }
    setToken(''); setError(''); load();
  };
  return (
    <Dialog id="dlg-github" title="GitHub" onClose={onClose} actions={(
      <>
        {connected ? <button id="github-disconnect" type="button" className="btn" onClick={async () => { await api().githubDisconnect(); load(); }}>Trennen</button> : null}
        {!connected ? <button id="github-ok" type="button" className="btn primary" onClick={connect}>Verbinden</button> : null}
        <button id="github-close" type="button" className="btn" onClick={onClose}>Schliessen</button>
      </>
    )}>
      <div id="github-body">
        {!s ? <p className="muted">Lädt …</p> : connected ? (
          <>
            <p>Verbunden als <strong>{s.login}</strong>.</p>
            {s.linked ? (
              <p>Dieses Projekt gehört zu <a href={s.url} onClick={(e) => { e.preventDefault(); api().openLink(s.url); }}>{s.repo}</a>.
                {' '}Hier geändert: {s.changed.length + s.added.length + s.deleted.length} Dateien{s.remoteNewer ? '; auf GitHub gibt es Neueres' : ''}.</p>
            ) : <p>Dieses Projekt ist mit keinem Repository verbunden. Sag ASK zum Beispiel «Hol das Repository besitzer/name» oder «Leg das Projekt auf GitHub an».</p>}
            <p className="muted small">Im Chat: «lade die Änderungen auf GitHub hoch», «hol die neuesten Änderungen von GitHub».</p>
            {s.trusted?.length ? (
              <>
                <p>Hochladen ohne Rückfrage:</p>
                <ul className="trusted">{s.trusted.map((repo) => <li key={repo}>{repo} <button type="button" className="link" onClick={async () => { await api().githubUntrust(repo); load(); }}>wieder fragen</button></li>)}</ul>
              </>
            ) : null}
          </>
        ) : s.message ? <p>GitHub meldet: {s.message}</p> : null}
      </div>
      {s && !connected ? (
        <div id="github-connect">
          <p>Damit ASK Projekte von GitHub holen und hochladen kann, braucht die App einmal ein persönliches Token mit dem Recht «repo».{' '}
            <button id="github-token-link" type="button" className="link" onClick={() => api().openLink('https://github.com/settings/tokens/new?scopes=repo&description=ASK%20Desktop')}>Token auf github.com erstellen</button></p>
          <input id="github-token" type="password" placeholder="ghp_… oder github_pat_…" spellCheck={false} autoComplete="off" value={token}
            onChange={(e) => setToken(e.target.value)} onKeyDown={(e) => { if (e.key === 'Enter') connect(); }} />
          <p className="muted small">Das Token wird mit deinem Windows-Konto verschlüsselt gespeichert. Das Modell und das Terminal sehen es nie. Anlegen und Hochladen bestätigst du jedes Mal.</p>
          {error ? <p id="github-error" className="error">{error}</p> : null}
        </div>
      ) : null}
    </Dialog>
  );
}

const UPDATE_TEXT = {
  aus: 'Automatische Updates sind in dieser Version nicht eingerichtet.',
  'bereit-zu-pruefen': 'Wird beim Start geprüft.',
  pruefen: 'Sucht nach Updates …',
  aktuell: 'Du hast die neueste Version.',
  laedt: 'Lädt das Update …',
  bereit: 'Das Update ist bereit.',
  verfuegbar: 'Eine neue Version ist verfügbar.',
  fehler: 'Prüfen nicht möglich.',
};

function Segmented({ value, options, onChange, name }) {
  return (
    <div className="segmented" role="radiogroup" aria-label={name}>
      {options.map(([v, label]) => (
        <button key={v} type="button" role="radio" aria-checked={value === v} className={value === v ? 'on' : ''} onClick={() => onChange(v)}>{label}</button>
      ))}
    </div>
  );
}

function SettingsDialog({ state, onClose, onGithub }) {
  const u = state.update || {};
  return (
    <Dialog id="dlg-settings" title="Einstellungen" onClose={onClose} actions={<button type="button" className="btn primary" onClick={onClose}>Fertig</button>}>
      <div className="setting">
        <div><div className="s-title">Darstellung</div><div className="s-text">Hell, Dunkel oder wie in Windows eingestellt.</div></div>
        <Segmented name="Darstellung" value={state.theme || 'system'} onChange={(theme) => api().setSettings({ theme })}
          options={[['system', 'System'], ['hell', 'Hell'], ['dunkel', 'Dunkel']]} />
      </div>
      <div className="setting">
        <div><div className="s-title">Rückfrage vor heiklen Schritten</div>
          <div className="s-text">{{ aus: 'ASK fragt nie. Rückgängig bleibt möglich.', heikel: 'Vor dem Löschen, vor dem Überschreiben vorhandener Dateien und vor Befehlen mit Pfaden ausserhalb des Projekts.', alle: 'Zusätzlich vor jedem Befehl und jeder Änderung an vorhandenen Dateien.' }[state.approvals || 'heikel']}</div></div>
        <Segmented name="Rückfrage" value={state.approvals || 'heikel'} onChange={(approvals) => api().setSettings({ approvals })}
          options={[['aus', 'Nie'], ['heikel', 'Heikle Schritte'], ['alle', 'Immer']]} />
      </div>
      <div className="setting">
        <div><div className="s-title">Frühere Chats</div><div className="s-text">ASK darf frühere Chats im selben Projekt durchsuchen und lesen (/chats).</div></div>
        <Segmented name="Frühere Chats" value={state.chatMemory ? 'an' : 'aus'} onChange={(v) => api().setChatMemory(v === 'an')}
          options={[['aus', 'Aus'], ['an', 'An']]} />
      </div>
      <div className="setting">
        <div><div className="s-title">ASK-Konto</div><div className="s-text">{state.ask?.state === 'angemeldet' ? `Angemeldet als ${state.ask.user || '(unbekannt)'} · ${state.askUrl}` : 'Nicht angemeldet'}</div></div>
        {state.ask?.state === 'angemeldet' ? <button type="button" className="btn" onClick={async () => { onClose(); await api().logout(); }}><Icon name="logout" size={15} /> Abmelden</button> : null}
      </div>
      <div className="setting">
        <div><div className="s-title">GitHub</div><div className="s-text">Projekte von GitHub holen und hochladen.</div></div>
        <button type="button" className="btn" onClick={onGithub}><Icon name="github" size={15} /> GitHub …</button>
      </div>
      <div className="setting">
        <div><div className="s-title">Version {state.version}</div>
          <div className="s-text">{UPDATE_TEXT[u.state] || ''}{u.state === 'fehler' && u.message ? ` ${u.message}` : ''}{u.available && ['laedt', 'bereit', 'verfuegbar'].includes(u.state) ? ` Neue Version: ${u.available}${u.state === 'laedt' ? ` (${u.progress || 0} %)` : ''}.` : ''}</div></div>
        {u.state === 'bereit' ? <button type="button" className="btn primary" onClick={() => api().installUpdate()}>Neu starten und installieren</button>
          : u.state === 'verfuegbar' ? <button type="button" className="btn primary" onClick={() => api().downloadUpdate()}>Herunterladen</button>
          : u.state !== 'aus' ? <button type="button" className="btn" disabled={u.state === 'pruefen' || u.state === 'laedt'} onClick={() => api().checkUpdate()}>Jetzt prüfen</button> : null}
      </div>
    </Dialog>
  );
}

function BigPreview({ entry, onClose }) {
  return (
    <Dialog id="dlg-preview" title={entry ? baseName(entry.path) : ''} full onClose={onClose}>
      <div id="big-body" className="preview-body big"><PreviewBody entry={entry} big /></div>
    </Dialog>
  );
}

// Welcher Dialog offen ist: erst die Pflichtschritte (Hinweis, Adresse, erstes Projekt), sonst der gewählte
export function activeDialog(state, dialog) {
  if (!state) return null;
  if (!state.warningAccepted) return 'warning';
  if (!state.askUrl) return 'url';
  if (!state.folder) return 'start';
  return dialog?.name || null;
}

export default function Dialogs({ state, dialog, setDialog, selected, startError }) {
  const name = activeDialog(state, dialog);
  if (!name) return null;
  const close = () => setDialog(null);
  return (
    <div id="overlay" className="overlay" onMouseDown={(e) => { if (e.target === e.currentTarget && dialog && name === dialog.name) close(); }}>
      {name === 'warning' ? <WarningDialog /> : null}
      {name === 'url' ? <UrlDialog state={state} /> : null}
      {name === 'start' ? <ProjectDialog state={state} first error={startError} /> : null}
      {name === 'project' ? <ProjectDialog state={state} onResult={close} onClose={close} /> : null}
      {name === 'ask' ? <AskDialog state={state} onClose={close} /> : null}
      {name === 'terminal' ? <TerminalDialog state={state} text={dialog.text} onClose={close} /> : null}
      {name === 'github' ? <GithubDialog onClose={close} /> : null}
      {name === 'settings' ? <SettingsDialog state={state} onClose={close} onGithub={() => setDialog({ name: 'github' })} /> : null}
      {name === 'preview' ? <BigPreview entry={selected} onClose={close} /> : null}
    </div>
  );
}
