// Bereich «GitHub» rechts: Repository des sichtbaren Projekts, Branch, Änderungen hier, Holen und
// Hochladen, Pull Requests. Alles läuft im Hauptprozess (github.js); Hochladen und Anlegen fragen wie beim Modell.
import { useCallback, useEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { baseName, extOf, fileBadge } from '../lib/format.js';

const MAX_ROWS = 60;

// Vorschlag für den Namen eines neuen Repositorys aus dem Projektordner
function repoName(root) {
  return baseName(root || '').toLowerCase().replace(/ä/g, 'ae').replace(/ö/g, 'oe').replace(/ü/g, 'ue')
    .replace(/[^\w.-]+/g, '-').replace(/^[-.]+|[-.]+$/g, '').slice(0, 80);
}

const plural = (n, one, many) => `${n} ${n === 1 ? one : many}`;

// Meldungen sind für das Modell geschrieben («Zuerst github_pull»): hier die Knöpfe nennen
const BUTTONS = { github_pull: '«Holen»', github_push: '«Hochladen»', github_clone: '«Repository verbinden»', github_create: '«Neues Repository»', github_branch: '«+ Branch»' };
const forPerson = (text) => String(text || '').replace(/github_(pull|push|clone|create|branch)\b/g, (m) => BUTTONS[m]);

function pullText(r) {
  const parts = [];
  if (r.written?.length) parts.push(`${plural(r.written.length, 'Datei', 'Dateien')} geholt`);
  if (r.removed?.length) parts.push(`${r.removed.length} gelöscht`);
  let text = parts.length ? `${parts.join(', ')}.` : 'Schon auf dem neuesten Stand.';
  if (r.conflicts?.length) text += ` Nicht überschrieben, weil hier anders: ${r.conflicts.slice(0, 5).join(', ')}${r.conflicts.length > 5 ? ' …' : ''}.`;
  return text;
}

function Change({ path, kind }) {
  const label = { neu: 'neu', geaendert: 'geändert', geloescht: 'gelöscht' }[kind];
  return (
    <div className={`gh-file${kind === 'geloescht' ? ' gone' : ''}`} title={path}>
      <span className={`fi ext-${extOf(path)}`}>{fileBadge(path).slice(0, 3)}</span>
      <span className="tn">{path}</span>
      <span className={`mark ${kind}`}>{label}</span>
    </div>
  );
}

export default function GithubPane({ root, files, running, dialogOpen, onConnect }) {
  const [s, setS] = useState(null);
  const [info, setInfo] = useState(null);
  const [prs, setPrs] = useState(null);
  const [busy, setBusy] = useState('');
  const [msg, setMsg] = useState(null);
  const [message, setMessage] = useState('');
  const [cloneRepo, setCloneRepo] = useState('');
  const [newName, setNewName] = useState(() => repoName(root));
  const [privat, setPrivat] = useState(true);
  const [branchForm, setBranchForm] = useState(null);
  const [prForm, setPrForm] = useState(null);
  const seq = useRef(0);

  const load = useCallback(async () => {
    const n = ++seq.current;
    const st = await api().githubStatus();
    if (n !== seq.current) return;
    setS(st);
    if (!st?.ok || !st.linked) { setInfo(null); setPrs(null); return; }
    const [b, p] = await Promise.all([api().githubBranches(), api().githubPulls()]);
    if (n !== seq.current) return;
    setInfo(b?.ok ? b : null);
    setPrs(p?.ok ? p.pulls : null);
  }, []);

  // Neues Projekt: sofort laden; Änderungen an Dateien, Ende eines Auftrags, geschlossener Dialog: gesammelt
  useEffect(() => {
    setS(null); setMsg(null); setBranchForm(null); setPrForm(null); setNewName(repoName(root));
    if (root) load();
  }, [root, load]);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return undefined; }
    if (!root || dialogOpen) return undefined;
    const t = setTimeout(load, 900);
    return () => clearTimeout(t);
  }, [files, running, dialogOpen, root, load]);

  // Eine Aktion zur Zeit; danach neu laden. done(res) liefert den Text bei Erfolg (oder null für keinen).
  const act = async (label, fn, done) => {
    if (busy) return null;
    setBusy(label);
    setMsg(null);
    let res;
    try { res = await fn(); } catch (err) { res = { ok: false, message: err.message }; }
    setBusy('');
    if (res?.summary === 'abgelehnt') setMsg(null);
    else if (/Konflikt/.test(res?.summary || '')) setMsg({ error: true, text: 'Nicht hochgeladen: Dateien hier und auf GitHub verschieden geändert. «Fassung von hier hochladen» ersetzt die auf GitHub.' });
    else if (!res?.ok) setMsg({ error: true, text: forPerson(res?.message || res?.output || 'Das ging nicht.') });
    else { const text = done(res); setMsg(text ? { text, link: res.link } : null); }
    await load();
    return res;
  };

  // Während ASK im Projekt arbeitet, keine Abgleiche von hier (der Hauptprozess lehnt sie ohnehin ab)
  const locked = !!busy || running;

  if (!root) return <div className="panel-empty">Kein Projekt geöffnet.</div>;
  if (!s) return <div className="panel-empty">Lädt …</div>;

  const note = msg ? (
    <div className={`gh-msg${msg.error ? ' error' : ''}`} role="status">
      <span>{msg.text}</span>
      {msg.link ? <button type="button" className="link" onClick={() => api().openLink(msg.link)}>Öffnen</button> : null}
    </div>
  ) : null;

  // ---------- nicht verbunden ----------
  if (!s.connected) {
    return (
      <div className="gh-pane">
        <div className="gh-empty">
          <Icon name="github" size={28} className="gh-empty-icon" />
          <p><b>Nicht mit GitHub verbunden</b></p>
          <p className="muted small">Mit einem persönlichen Token holt ASK Projekte von GitHub und lädt Änderungen hoch.</p>
          <button id="gh-connect" type="button" className="btn primary" onClick={onConnect}>Verbinden …</button>
        </div>
      </div>
    );
  }
  if (!s.ok) {
    return (
      <div className="gh-pane">
        <div className="gh-empty">
          <p>GitHub meldet: {s.message || 'unbekannter Fehler'}</p>
          <button type="button" className="btn" onClick={load}>Erneut versuchen</button>
        </div>
      </div>
    );
  }

  const work = running ? <p className="muted small gh-running">ASK arbeitet gerade in diesem Projekt. Abgleichen geht danach wieder.</p> : null;
  const account = (
    <div className="gh-account muted small">
      <span className="ell">Verbunden als <b>{s.login}</b></span>
      <button type="button" className="link" onClick={onConnect}>Konto …</button>
    </div>
  );

  // ---------- kein Repository ----------
  if (!s.linked) {
    return (
      <div className="gh-pane">
        {account}
        {work}
        {note}
        <section className="gh-sec">
          <h3>Repository verbinden</h3>
          <p className="muted small">Holt ein Repository von GitHub in dieses Projekt.</p>
          <form className="gh-row" onSubmit={(e) => { e.preventDefault(); if (cloneRepo.trim()) act('clone', () => api().githubClone(cloneRepo.trim()), (r) => `${r.repo} verbunden. ${pullText(r)}`); }}>
            <input id="gh-clone" className="gh-input" placeholder="besitzer/name" spellCheck={false} value={cloneRepo} onChange={(e) => setCloneRepo(e.target.value)} aria-label="Repository" />
            <button type="submit" className="btn small" disabled={locked || !cloneRepo.trim()}>{busy === 'clone' ? <span className="spin small" /> : null}Holen</button>
          </form>
        </section>
        <section className="gh-sec">
          <h3>Neues Repository</h3>
          <p className="muted small">Legt ein Repository an und lädt das Projekt hoch (ohne .ask).</p>
          <form className="gh-row" onSubmit={(e) => { e.preventDefault(); if (newName.trim()) act('create', () => api().githubCreate(newName.trim(), privat), (r) => `Angelegt: ${r.summary || ''}`); }}>
            <input id="gh-new" className="gh-input" placeholder="Name" spellCheck={false} value={newName} onChange={(e) => setNewName(e.target.value)} aria-label="Name des Repositorys" />
            <button type="submit" className="btn small" disabled={locked || !newName.trim()}>{busy === 'create' ? <span className="spin small" /> : null}Anlegen</button>
          </form>
          <label className="gh-check"><input type="checkbox" checked={privat} onChange={(e) => setPrivat(e.target.checked)} /> Privat</label>
        </section>
      </div>
    );
  }

  // ---------- verbunden ----------
  const changes = [...s.changed.map((p) => [p, 'geaendert']), ...s.added.map((p) => [p, 'neu']), ...s.deleted.map((p) => [p, 'geloescht'])]
    .sort((a, b) => a[0].localeCompare(b[0], 'de', { numeric: true }));
  const pending = changes.length;
  const branches = info?.branches || [s.branch];
  const onDefault = !info || info.current === info.default;

  const pull = () => act('pull', () => api().githubPull(), pullText);
  const push = (overwrite = false) => act('push', () => api().githubPush(message.trim(), overwrite), (r) => {
    if (r.summary === 'nichts zu tun') return 'Nichts hochzuladen.';
    setMessage('');
    return `Auf ${s.branch} hochgeladen.`;
  });
  const switchTo = (name) => act('switch', () => api().githubSwitch(name), (r) => (r.same ? null : `Gewechselt zu ${r.branch}. ${pullText(r)}`));
  const createBranch = () => act('branch', () => api().githubBranchCreate(branchForm.trim()), (r) => {
    setBranchForm(null);
    return `Branch ${r.branch} angelegt. Änderungen hier gehen beim Hochladen dorthin.`;
  });
  const createPr = () => act('pr', async () => {
    const r = await api().githubPrCreate(prForm.title.trim(), prForm.body.trim());
    return r?.ok ? { ...r, link: r.url } : r;
  }, (r) => { setPrForm(null); return `Pull Request #${r.number} erstellt.`; });

  return (
    <div className="gh-pane">
      <div className="gh-card">
        <div className="gh-repo">
          <Icon name="github" size={16} />
          <button type="button" className="gh-repo-name ell" title={s.url} onClick={() => api().openLink(s.url)}>{s.repo}</button>
          <span className="spacer" />
          <button type="button" className="icon-btn" title="Neu laden" aria-label="Neu laden" disabled={!!busy} onClick={() => { setMsg(null); load(); }}><Icon name="reload" size={14} /></button>
          <button type="button" className="icon-btn" title="Konto" aria-label="Konto" onClick={onConnect}><Icon name="settings" size={14} /></button>
        </div>
        <div className="gh-branch">
          <Icon name="branch" size={15} />
          <label className="gh-select" title={pending ? 'Wechseln geht erst, wenn die Änderungen hier hochgeladen sind.' : 'Branch wechseln'}>
            <select id="gh-branch" value={s.branch} disabled={locked} aria-label="Branch" onChange={(e) => switchTo(e.target.value)}>
              {branches.map((b) => <option key={b} value={b}>{b}{info && b === info.default ? ' (Standard)' : ''}</option>)}
            </select>
            <Icon name="chevDown" size={12} />
          </label>
          <span className="spacer" />
          {branchForm === null ? <button type="button" className="ghost-btn" disabled={locked} onClick={() => setBranchForm('')}><Icon name="plus" size={13} /> Branch</button> : null}
        </div>
        {branchForm !== null ? (
          <form className="gh-row" onSubmit={(e) => { e.preventDefault(); if (branchForm.trim()) createBranch(); }}>
            <input className="gh-input" autoFocus placeholder={`Neuer Branch von ${s.branch}`} spellCheck={false} value={branchForm} aria-label="Name des neuen Branches"
              onChange={(e) => setBranchForm(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setBranchForm(null); } }} />
            <button type="submit" className="btn small" disabled={locked || !branchForm.trim()}>Anlegen</button>
            <button type="button" className="icon-btn" title="Abbrechen" aria-label="Abbrechen" onClick={() => setBranchForm(null)}><Icon name="close" size={14} /></button>
          </form>
        ) : null}
      </div>

      {work}
      {note}
      {s.remoteNewer ? (
        <div className="gh-hint">
          <Icon name="down" size={14} />
          <span>Auf GitHub gibt es Neueres.</span>
          <button type="button" className="link" disabled={locked} onClick={pull}>Holen</button>
        </div>
      ) : null}
      {s.conflicts?.length ? (
        <div className="gh-hint warn">
          <Icon name="alert" size={14} />
          <span>{plural(s.conflicts.length, 'Datei', 'Dateien')} hier und auf GitHub verschieden geändert: {s.conflicts.slice(0, 3).join(', ')}{s.conflicts.length > 3 ? ' …' : ''}.</span>
          <button type="button" className="link" disabled={locked} onClick={() => push(true)}>Fassung von hier hochladen</button>
        </div>
      ) : null}

      <section className="gh-sec gh-changes">
        <h3>Änderungen hier <span className="muted">{pending || ''}</span></h3>
        {pending ? (
          <div className="gh-files">
            {changes.slice(0, MAX_ROWS).map(([p, kind]) => <Change key={p} path={p} kind={kind} />)}
            {pending > MAX_ROWS ? <div className="muted small gh-more">und {pending - MAX_ROWS} weitere</div> : null}
          </div>
        ) : <p className="muted small">Keine Änderungen seit dem letzten Abgleich.</p>}
        <form className="gh-push" onSubmit={(e) => { e.preventDefault(); if (pending) push(false); }}>
          <input id="gh-message" className="gh-input" placeholder="Was hast du geändert?" value={message} onChange={(e) => setMessage(e.target.value)}
            aria-label="Beschreibung der Änderungen" disabled={!pending} maxLength={300} />
          <div className="gh-acts">
            <button id="gh-pull" type="button" className="btn small" disabled={locked} onClick={pull}>{busy === 'pull' || busy === 'switch' ? <span className="spin small" /> : <Icon name="down" size={14} />}Holen</button>
            <button id="gh-push" type="submit" className="btn small primary" disabled={locked || !pending}>{busy === 'push' ? <span className="spin small" /> : null}Hochladen</button>
          </div>
        </form>
      </section>

      <section className="gh-sec">
        <h3>Pull Requests
          {!onDefault && prForm === null ? <button type="button" className="ghost-btn" disabled={locked} onClick={() => setPrForm({ title: '', body: '' })}><Icon name="plus" size={13} /> Erstellen</button> : null}
        </h3>
        {prForm ? (
          <form className="gh-pr-form" onSubmit={(e) => { e.preventDefault(); if (prForm.title.trim() && !pending) createPr(); }}>
            <p className="muted small">Von <b>{s.branch}</b> nach <b>{info?.default}</b>.{pending ? ' Zuerst hochladen: Der Pull Request enthält nur, was auf GitHub liegt.' : ''}</p>
            <input className="gh-input" autoFocus placeholder="Titel" value={prForm.title} onChange={(e) => setPrForm({ ...prForm, title: e.target.value })} aria-label="Titel" maxLength={250} />
            <textarea className="gh-input" rows={3} placeholder="Beschreibung (optional)" value={prForm.body} onChange={(e) => setPrForm({ ...prForm, body: e.target.value })} aria-label="Beschreibung" />
            <div className="gh-acts">
              <button type="button" className="btn small" onClick={() => setPrForm(null)}>Abbrechen</button>
              <button type="submit" className="btn small primary" disabled={locked || !prForm.title.trim() || !!pending}>{busy === 'pr' ? <span className="spin small" /> : null}Erstellen</button>
            </div>
          </form>
        ) : null}
        {prs === null ? <p className="muted small">Lädt …</p> : prs.length ? (
          <ul className="gh-prs">
            {prs.map((p) => (
              <li key={p.number}>
                <button type="button" className="gh-pr" title={p.url} onClick={() => api().openLink(p.url)}>
                  <Icon name="pr" size={14} />
                  <span className="gh-pr-text"><span className="ell">{p.title}</span><span className="muted small ell">#{p.number} · {p.head} → {p.base}{p.draft ? ' · Entwurf' : ''}</span></span>
                </button>
              </li>
            ))}
          </ul>
        ) : <p className="muted small">{onDefault ? 'Keine offenen. Für einen Pull Request zuerst einen eigenen Branch anlegen.' : 'Keine offenen.'}</p>}
      </section>
    </div>
  );
}
