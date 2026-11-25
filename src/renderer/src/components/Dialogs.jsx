import { useEffect, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { baseName, formatCount, formatDuration } from '../lib/format.js';
import { PreviewBody } from './RightPanel.jsx';
import Tour from './Tour.jsx';
import { MemoryDialog, SkillsDialog } from './Knowledge.jsx';

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
        {a.state === 'angemeldet' || a.state === 'fehler' ? <button id="ask-logout" type="button" className="btn" onClick={async () => { onClose(); await api().logout(); }}><Icon name="logout" size={15} /> {a.state === 'fehler' ? 'Neu anmelden' : 'Abmelden'}</button> : null}
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
            ) : <p>Dieses Projekt ist mit keinem Repository verbunden. Verbinde es rechts im Bereich «GitHub» oder sag ASK zum Beispiel «Hol das Repository besitzer/name» oder «Leg das Projekt auf GitHub an».</p>}
            <p className="muted small">Holen, Hochladen, Branches und Pull Requests: rechts im Bereich «GitHub» oder im Chat, z. B. «lade die Änderungen auf GitHub hoch».</p>
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
  sucht: 'Sucht nach Updates …',
  aktuell: 'Du hast die neueste Version.',
  verfügbar: 'Eine neue Version ist da.',
  lädt: 'Das Update wird geladen …',
  geladen: 'Das Update ist geladen.',
  neustart: 'ASK startet gleich neu.',
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

function SettingsDialog({ state, onClose, onGithub, onVerbose, onTerminal, onUsage, onTour, onMemory, onSkills }) {
  const u = state.update || {};
  return (
    <Dialog id="dlg-settings" title="Einstellungen" onClose={onClose} actions={<button type="button" className="btn primary" onClick={onClose}>Fertig</button>}>
      <div className="setting">
        <div><div className="s-title">Darstellung</div><div className="s-text">Hell, Dunkel oder wie in Windows eingestellt.</div></div>
        <Segmented name="Darstellung" value={state.theme || 'system'} onChange={(theme) => api().setSettings({ theme })}
          options={[['system', 'System'], ['hell', 'Hell'], ['dunkel', 'Dunkel']]} />
      </div>
      <div className="setting">
        <div><div className="s-title">Prüfung im Modus «Auto»</div>
          <div className="s-text">Ein Modell in ASK prüft jeden Befehl und fragt nur bei Gefährlichem. Ein schnelles Modell genügt.</div></div>
        <label className="s-select">
          <select id="review-model" value={state.reviewModel || ''} onChange={(e) => api().setSettings({ reviewModel: e.target.value })} aria-label="Modell für die Prüfung">
            <option value="">Wie im Chat</option>
            {(state.models || []).map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
          </select>
          <Icon name="chevDown" size={13} />
        </label>
      </div>
      <div className="setting">
        <div><div className="s-title">Frühere Chats</div><div className="s-text">ASK darf frühere Chats im selben Projekt durchsuchen und lesen (/chats).</div></div>
        <Segmented name="Frühere Chats" value={state.chatMemory ? 'an' : 'aus'} onChange={(v) => api().setChatMemory(v === 'an')}
          options={[['aus', 'Aus'], ['an', 'An']]} />
      </div>
      <div className="setting">
        <div><div className="s-title">Gedächtnis</div><div className="s-text">ASK merkt sich Vorlieben, Kenntnisse und Projektwissen für alle Chats, nur auf diesem Laptop (/gedaechtnis).</div></div>
        <div className="setting-ctl">
          <Segmented name="Gedächtnis" value={state.memory !== false ? 'an' : 'aus'} onChange={(v) => api().setMemoryEnabled(v === 'an')} options={[['aus', 'Aus'], ['an', 'An']]} />
          <button type="button" id="settings-memory" className="btn small" onClick={onMemory}>Ansehen …</button>
        </div>
      </div>
      <div className="setting">
        <div><div className="s-title">Skills</div><div className="s-text">Eigene Anleitungen für wiederkehrende Aufträge, aufrufen mit /name (/skills).</div></div>
        <button type="button" id="settings-skills" className="btn" onClick={onSkills}>Skills …</button>
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
        <div><div className="s-title">Benachrichtigungen</div><div className="s-text">Meldung von Windows, wenn ASK fertig ist oder etwas fragt und du woanders bist.</div></div>
        <Segmented name="Benachrichtigungen" value={state.prefs?.notify === false ? 'aus' : 'an'} onChange={(v) => api().setPrefs({ notify: v === 'an' })}
          options={[['aus', 'Aus'], ['an', 'An']]} />
      </div>
      <div className="setting">
        <div><div className="s-title">Nutzung</div><div className="s-text">Aufträge, Token und Dateien pro Projekt (/nutzung).</div></div>
        <button type="button" id="settings-usage" className="btn" onClick={onUsage}><Icon name="chart" size={15} /> Nutzung …</button>
      </div>
      <div className="setting">
        <div><div className="s-title">Ansicht</div><div className="s-text">Gedanken und alle Details zeigen (Ctrl+O), Aufgabenliste nach dem Auftrag einklappen.</div></div>
        <div className="checks">
          <label className="check"><input type="checkbox" id="pref-verbose" checked={!!state.prefs?.verbose} onChange={(e) => onVerbose(e.target.checked)} /> Details</label>
          <label className="check"><input type="checkbox" id="pref-todo" checked={state.prefs?.todoCollapsed ?? true} onChange={(e) => api().setPrefs({ todoCollapsed: e.target.checked })} /> Einklappen</label>
        </div>
      </div>
      <div className="setting">
        <div><div className="s-title">Version {state.version}</div>
          <div className="s-text">{UPDATE_TEXT[u.state] || (u.enabled ? '' : 'Updates gibt es nur in der installierten App.')}{u.state === 'fehler' && u.error ? ` ${u.error}` : ''}</div>
          <label className="check small"><input type="checkbox" id="pref-beta" checked={!!u.beta} onChange={(e) => api().setBetaUpdates(e.target.checked, false)} /> Beta-Versionen erhalten</label></div>
        {u.enabled ? <button type="button" id="settings-update" className="btn" onClick={() => api().updateAction()}>Nach Updates suchen</button> : null}
      </div>
      <div className="setting">
        <div><div className="s-title">Mehr</div><div className="s-text">Kurze Einführung, Meldungen des Terminals, Anmeldungen im Browser der App.</div></div>
        <div className="checks">
          <button type="button" id="settings-tour" className="btn small" onClick={onTour}>Einführung zeigen</button>
          <button type="button" className="btn small" onClick={onTerminal}>Terminal …</button>
          <button type="button" className="btn small" onClick={() => api().browserClear()}>Browser-Anmeldungen löschen</button>
        </div>
      </div>
    </Dialog>
  );
}

// Nutzung (ENTSCHEIDE E49): Zahlen aus den Chats aller Projekte, Balken der letzten 30 Tage, Tabelle pro Projekt.
// Der Abschnitt ASK erscheint nur, wenn ASK etwas liefert (nicht mit dem echten ASK geprüft).
const dayLabel = (date, long) => new Date(`${date}T12:00:00`).toLocaleDateString('de-CH', long ? { weekday: 'short', day: '2-digit', month: '2-digit' } : { day: '2-digit', month: '2-digit' });

function UsageChart({ days, measure }) {
  const [hover, setHover] = useState(-1);
  const values = days.map((d) => d[measure] || 0);
  const max = Math.max(...values, 1);
  const unit = measure === 'tasks' ? (v) => `${formatCount(v)} ${v === 1 ? 'Auftrag' : 'Aufträge'}` : (v) => `${formatCount(v)} Token`;
  const sum = values.reduce((a, b) => a + b, 0);
  const h = days[hover];
  return (
    <div className="u-chart">
      <div className="u-plot" role="img" aria-label={`Letzte 30 Tage: ${unit(sum)}, höchstens ${unit(max)} an einem Tag`} onMouseLeave={() => setHover(-1)}>
        <span className="u-max">{formatCount(max)}</span>
        {days.map((d, i) => (
          <div key={d.date} className={`u-col${i === hover ? ' on' : ''}`} onMouseEnter={() => setHover(i)}>
            {values[i] ? <span className="u-bar" style={{ height: `${Math.max(2, (values[i] / max) * 100)}%` }} /> : null}
          </div>
        ))}
        {h ? (
          <div className="u-tip" style={{ left: `${((hover + 0.5) / days.length) * 100}%` }}>
            <strong>{dayLabel(h.date, true)}</strong> {unit(h[measure])}
            {measure === 'tasks' && h.tokens ? <span className="muted"> · {formatCount(h.tokens)} Token</span> : null}
          </div>
        ) : null}
      </div>
      <div className="u-axis"><span>{dayLabel(days[0].date)}</span><span>{dayLabel(days[Math.floor(days.length / 2)].date)}</span><span>Heute</span></div>
    </div>
  );
}

function UsageDialog({ onClose }) {
  const [u, setU] = useState(null);
  const [measure, setMeasure] = useState('tasks');
  useEffect(() => {
    let alive = true;
    // zuerst nur lokal (schnell), dann mit der Anfrage an ASK
    api().usageGet(false).then((r) => { if (alive) setU((old) => old || r); }).catch(() => {});
    api().usageGet(true).then((r) => { if (alive && r) setU(r); }).catch(() => {});
    return () => { alive = false; };
  }, []);
  const t = u?.total;
  const approx = (p) => (p.estimatedTokens > 0 ? '≈ ' : '');
  const files = (p) => p.files.added + p.files.changed;
  const a = u?.ask;
  return (
    <Dialog id="dlg-usage" title="Nutzung" wide onClose={onClose} actions={<button type="button" className="btn primary" onClick={onClose}>Schliessen</button>}>
      {!u ? <p className="muted">Lädt …</p> : (
        <>
          <div className="u-tiles">
            <div className="u-tile"><div className="u-label">Aufträge</div><div className="u-num">{formatCount(t.tasks)}</div><div className="u-sub">in {formatCount(t.chats)} {t.chats === 1 ? 'Chat' : 'Chats'}</div></div>
            <div className="u-tile"><div className="u-label">Token</div><div className="u-num">{approx(t)}{formatCount(t.tokens)}</div><div className="u-sub">{formatCount(t.steps)} Arbeitsschritte</div></div>
            <div className="u-tile"><div className="u-label">Dateien</div><div className="u-num">{formatCount(files(t))}</div><div className="u-sub">{formatCount(t.files.added)} neu, {formatCount(t.files.changed)} geändert</div></div>
            <div className="u-tile"><div className="u-label">Zeit</div><div className="u-num">{formatDuration(t.modelMs)}</div><div className="u-sub">Modell</div></div>
          </div>
          <div className="u-head">
            <h2>Letzte 30 Tage</h2>
            <Segmented name="Wert" value={measure} onChange={setMeasure} options={[['tasks', 'Aufträge'], ['tokens', 'Token']]} />
          </div>
          {u.days.some((d) => d.tasks || d.tokens) ? <UsageChart days={u.days} measure={measure} /> : <p className="muted small">In den letzten 30 Tagen noch kein Auftrag.</p>}
          <h2>Projekte</h2>
          {u.projects.length ? (
            <table className="u-table">
              <thead><tr><th>Projekt</th><th>Chats</th><th>Aufträge</th><th>Schritte</th><th>Token</th><th>Dateien</th><th>Zeit</th></tr></thead>
              <tbody>
                {u.projects.map((p) => (
                  <tr key={p.folder} className={p.missing ? 'missing' : ''}>
                    <td title={p.folder}><span className="u-proj"><span className="swatch" style={{ background: p.color || 'var(--dim)' }} /><span className="ell">{p.name}</span>{p.missing ? <span className="small">Ordner fehlt</span> : null}</span></td>
                    <td>{formatCount(p.chats)}</td><td>{formatCount(p.tasks)}</td><td>{formatCount(p.steps)}</td>
                    <td>{approx(p)}{formatCount(p.tokens)}</td><td>{formatCount(files(p))}</td><td>{formatDuration(p.modelMs)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : <p className="muted small">Noch keine Projekte.</p>}
          {a ? (
            <>
              <h2>ASK-Webseite</h2>
              <div className="u-ask">
                {a.webChats !== undefined ? <span><strong>{formatCount(a.webChats)}{a.more ? '+' : ''}</strong> Chats{a.webChats30 ? `, ${formatCount(a.webChats30)} in den letzten 30 Tagen` : ''}</span> : null}
                {a.messages !== undefined ? <span><strong>{formatCount(a.messages)}</strong> Nachrichten</span> : null}
                {a.tokens !== undefined ? <span><strong>{formatCount(a.tokens)}</strong> Token</span> : null}
                {a.tokens === undefined && (a.inputTokens !== undefined || a.outputTokens !== undefined)
                  ? <span><strong>{formatCount((a.inputTokens || 0) + (a.outputTokens || 0))}</strong> Token</span> : null}
              </div>
            </>
          ) : null}
          <p className="muted small">Gezählt aus den Chats in deinen Projektordnern, nur auf diesem Laptop.{t.estimatedTokens > 0 ? ' ≈ heisst teilweise geschätzt.' : ''}{a ? ' ASK-Webseite: Angaben von ASK.' : ''}</p>
        </>
      )}
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

// Welcher Dialog offen ist: erst die Pflichtschritte (Hinweis, Adresse, erstes Projekt), sonst der gewählte.
// Danach einmal die Einführung (E49), sobald die Anmeldung bei ASK durch ist.
const LOGIN_PENDING = ['nicht-angemeldet', 'unbekannt', 'pruefen'];
export function activeDialog(state, dialog) {
  if (!state) return null;
  if (!state.warningAccepted) return 'warning';
  if (!state.askUrl) return 'url';
  if (!state.folder) return 'start';
  if (dialog?.name) return dialog.name;
  if (state.prefs && state.prefs.tourDone === false && !LOGIN_PENDING.includes(state.ask?.state)) return 'tour';
  return null;
}

export default function Dialogs({ state, dialog, setDialog, selected, startError, onVerbose }) {
  const name = activeDialog(state, dialog);
  if (!name) return null;
  const close = () => setDialog(null);
  if (name === 'tour') return <Tour state={state} onDone={() => { api().setPrefs({ tourDone: true }); close(); }} />;
  return (
    <div id="overlay" className="overlay" onMouseDown={(e) => { if (e.target === e.currentTarget && dialog && name === dialog.name) close(); }}>
      {name === 'warning' ? <WarningDialog /> : null}
      {name === 'url' ? <UrlDialog state={state} /> : null}
      {name === 'start' ? <ProjectDialog state={state} first error={startError} /> : null}
      {name === 'project' ? <ProjectDialog state={state} onResult={close} onClose={close} /> : null}
      {name === 'ask' ? <AskDialog state={state} onClose={close} /> : null}
      {name === 'terminal' ? <TerminalDialog state={state} text={dialog.text} onClose={close} /> : null}
      {name === 'github' ? <GithubDialog onClose={close} /> : null}
      {name === 'settings' ? <SettingsDialog state={state} onClose={close} onGithub={() => setDialog({ name: 'github' })} onVerbose={onVerbose}
        onTerminal={() => setDialog({ name: 'terminal' })} onUsage={() => setDialog({ name: 'usage' })} onTour={() => setDialog({ name: 'tour' })}
        onMemory={() => setDialog({ name: 'memory' })} onSkills={() => setDialog({ name: 'skills' })} /> : null}
      {name === 'usage' ? <UsageDialog onClose={close} /> : null}
      {name === 'memory' ? <MemoryDialog state={state} onClose={close} /> : null}
      {name === 'skills' ? <SkillsDialog onClose={close} /> : null}
      {name === 'preview' ? <BigPreview entry={selected} onClose={close} /> : null}
    </div>
  );
}
