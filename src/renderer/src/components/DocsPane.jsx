// Reiter «Docs» rechts (ENTSCHEIDE E81): lebende Dokumente wie Claude Docs. Oben die Liste der Docs des sichtbaren
// Projekts (docs/….md), sonst ein Doc im Editor wie in Word: formatiert bearbeiten, gespeichert wird Markdown, nach
// einer kurzen Pause von selbst. Schreibt ASK am Doc, lädt der Editor neu; hat die Person gleichzeitig getippt,
// überschreibt nichts das andere (Hinweis mit Wahl). Kommentare markieren eine Stelle und gehen auf Wunsch an ASK.
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import PopMenu from './PopMenu.jsx';
import { api } from '../api.js';
import { htmlToMd, mdToHtml } from '../lib/docMd.mjs';
import { fileUrl } from '../lib/format.js';

const SAVE_MS = 800;
// Farben zum Anstreichen (wie COLORS in docs.rs); ohne Farbe gilt Gelb
const COLORS = [['gelb', 'Gelb'], ['gruen', 'Grün'], ['blau', 'Blau'], ['rot', 'Rot']];
// Bitten an ASK zum markierten Text (Rechtsklick und Menü «ASK» oben): Text für ASK
const ACTIONS = [
  ['erklaeren', 'Erklären', 'help', 'Erkläre mir diese Stelle verständlich. Antworte im Kommentar, ändere das Doc nicht.'],
  ['grafik', 'Grafik dazu', 'chart', 'Mach eine Grafik dazu (Diagramm oder Bild) und füge sie im Doc direkt nach dieser Stelle ein.'],
  ['einfacher', 'Einfacher', 'pen', 'Formuliere diese Stelle einfacher und ersetze sie im Doc.'],
  ['beispiel', 'Beispiel', 'list', 'Ergänze direkt nach dieser Stelle ein kurzes Beispiel im Doc.'],
];
const when = (ms) => {
  const d = new Date(ms);
  if (d.toDateString() === new Date().toDateString()) return d.toLocaleTimeString('de-CH', { hour: '2-digit', minute: '2-digit' });
  return d.toLocaleDateString('de-CH', { day: '2-digit', month: '2-digit', year: '2-digit' });
};

// ---------- Liste ----------

function DocList({ docs, free, onOpen, onCompose, notice }) {
  const [naming, setNaming] = useState(false);
  const [joining, setJoining] = useState(false);
  const [name, setName] = useState('');
  const [error, setError] = useState('');
  const create = async (e) => {
    e.preventDefault();
    const title = name.trim();
    if (!title) return;
    const r = await api().docsCreate(title);
    if (!r?.ok) { setError(r?.reason || 'Nicht angelegt.'); return; }
    setNaming(false); setName(''); setError('');
    onOpen(r.path);
  };
  return (
    <div className="docs-list">
      <div className="docs-head">
        <b>Docs</b><span className="muted small">{free ? 'Chats ohne Projekt' : 'in diesem Projekt'}</span>
        <span className="spacer" />
        <button type="button" className="icon-btn" title="Geteiltes Doc öffnen" aria-label="Geteiltes Doc öffnen"
          onClick={() => setJoining(!joining)}><Icon name="user" size={15} /></button>
        <button type="button" className="btn small" onClick={() => setNaming(true)}><Icon name="plus" size={13} /> Neues Doc</button>
      </div>
      {joining ? <JoinPanel onOpen={(p) => { setJoining(false); onOpen(p); }} onClose={() => setJoining(false)} notice={notice} /> : null}
      {naming ? (
        <form className="docs-new" onSubmit={create}>
          <input autoFocus value={name} placeholder="Titel, z. B. Lernzettel Chemie" maxLength={120} aria-label="Titel des neuen Docs"
            onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setNaming(false); } }} />
          <button type="submit" className="btn small primary" disabled={!name.trim()}>Anlegen</button>
          {error ? <p className="docs-error">{error}</p> : null}
        </form>
      ) : null}
      {error && !naming ? <p className="docs-error docs-pad">{error}</p> : null}
      {docs === null ? <p className="muted docs-pad">Lädt …</p> : docs.length ? (
        <ul className="docs-items">
          {docs.map((d) => (
            <li key={d.path}>
              <button type="button" className="docs-item" onClick={() => onOpen(d.path)} title={d.path}>
                <Icon name="doc" size={16} />
                <span className="docs-item-text"><b className="ell">{d.title}</b><span className="muted ell">{d.path} · {when(d.modified)}</span></span>
                {d.geteilt ? <span className="docs-shared" title="Geteilt: andere schreiben mit"><Icon name="user" size={13} /></span> : null}
                {d.comments ? <span className="docs-badge" title={`${d.comments} offene Kommentare`}>{d.comments}</span> : null}
              </button>
            </li>
          ))}
        </ul>
      ) : (
        <div className="gh-empty">
          <span className="gh-empty-icon"><Icon name="doc" size={30} /></span>
          <b>Noch keine Docs</b>
          <p className="muted">Ein Doc ist ein Dokument, das ASK für dich schreibt und du hier weiterbearbeitest, kommentierst und als Word oder PDF speicherst.</p>
          <button type="button" className="btn small" onClick={() => onCompose('Schreib ein Doc: ')}><Icon name="spark" size={13} /> ASK ein Doc schreiben lassen</button>
        </div>
      )}
    </div>
  );
}

// ---------- Teilen (E82) ----------

const ago = (ms) => {
  if (!ms) return 'noch nie';
  const min = Math.round((Date.now() - ms) / 60000);
  return min < 1 ? 'gerade eben' : min < 60 ? `vor ${min} Min.` : min < 1440 ? `vor ${Math.round(min / 60)} Std.` : when(ms);
};

// Anmeldung bei Microsoft für das Teilen per Link (E83): Zustand und Knopf
function useMicrosoft() {
  const [ms, setMs] = useState(null);
  const refresh = useCallback(async () => setMs(await api().docsMsStatus?.() || { eingerichtet: false }), []);
  useEffect(() => { refresh(); }, [refresh]);
  return [ms, refresh];
}

function MsLogin({ ms, onDone, notice }) {
  const [busy, setBusy] = useState(false);
  // Ohne App-Registrierung (E83) gibt es kein Teilen per Link: dann erscheint davon nichts
  if (!ms?.eingerichtet) return null;
  if (ms.angemeldet) {
    return (
      <p className="muted small docs-row">Bei Microsoft als <b>{ms.name || 'Schulkonto'}</b><span className="spacer" />
        <button type="button" className="link-btn" onClick={async () => { await api().docsMsLogout(); onDone(); }}>Abmelden</button></p>
    );
  }
  const login = async () => {
    setBusy(true);
    const r = await api().docsMsLogin();
    setBusy(false);
    if (r?.ok) onDone(); else notice?.(r?.reason || 'Anmeldung nicht geklappt.');
  };
  return (
    <div className="docs-row">
      <button type="button" className="btn small primary" disabled={busy} onClick={login}>{busy ? 'Warte auf die Anmeldung im Browser …' : 'Mit Microsoft anmelden'}</button>
      <span className="muted small">einmal, mit dem Schulkonto (wie bei Teams)</span>
    </div>
  );
}

const teamsShare = (link, title) => `https://teams.microsoft.com/share?href=${encodeURIComponent(link)}&msgText=${encodeURIComponent(`Schreib mit mir in ASK am Doc «${title}»: In ASK rechts bei Docs auf den Personen-Knopf und diesen Link einfügen.`)}`;

function LinkBox({ link, title, notice }) {
  return (
    <div className="docs-linkbox">
      <input readOnly value={link} aria-label="Link zum Doc" onFocus={(e) => e.target.select()} />
      <div className="docs-row">
        <button type="button" className="btn small primary" onClick={() => { api().copy(link); notice?.('Link kopiert. Füge ihn in Teams ein.'); }}><Icon name="copy" size={13} /> Link kopieren</button>
        <button type="button" className="btn small" onClick={() => api().openLink(teamsShare(link, title))}>In Teams teilen</button>
      </div>
    </div>
  );
}

function SharePanel({ path, title, share, onChanged, onClose, notice }) {
  const [busy, setBusy] = useState(false);
  const [folderWay, setFolderWay] = useState(false);
  const [ms, refreshMs] = useMicrosoft();
  const byFolder = async () => {
    setBusy(true);
    const r = await api().docsShare(path);
    setBusy(false);
    if (r?.ok) { onChanged(); notice?.('Geteilt. Gib den Ordner jetzt in OneDrive oder Teams frei.'); } else if (!r?.canceled) notice?.(r?.reason || 'Nicht geteilt.');
  };
  const byLink = async () => {
    setBusy(true);
    const r = await api().docsShareLink(path);
    setBusy(false);
    if (r?.ok) { onChanged(); if (r.link) { api().copy(r.link); notice?.('Link erstellt und kopiert. Füge ihn in Teams ein.'); } } else notice?.(r?.reason || 'Nicht geteilt.');
  };
  const stop = async () => { const r = await api().docsUnshare(path); if (r?.ok) { onChanged(); onClose(); } };
  if (!share?.geteilt) {
    const byLinkWay = !!ms?.eingerichtet;
    return (
      <div className="docs-share">
        <div className="docs-row"><b>Doc live teilen</b><span className="spacer" /><button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={13} /></button></div>
        {!ms ? <p className="muted small">Lädt …</p> : byLinkWay ? (
          <>
            <p>Andere schreiben auf ihrem Laptop mit, und ihr seht die Änderungen gegenseitig. Du bekommst einen <b>Link</b>, den du in <b>Teams</b> schickst; wer ihn in ASK einfügt, schreibt mit.</p>
            <MsLogin ms={ms} onDone={refreshMs} notice={notice} />
            {ms.angemeldet ? (
              <div className="docs-row"><button type="button" className="btn small primary" disabled={busy} onClick={byLink}>{busy ? 'Erstellt den Link …' : 'Link erstellen'}</button>
                <span className="muted small">legt das Doc in deinem OneDrive unter «ASK Docs» ab</span></div>
            ) : null}
            {folderWay ? (
              <>
                <p className="muted small">Über einen Ordner, den alle haben (OneDrive freigegeben, Teams-Kanal, Laufwerk der Schule): ASK legt dort «….askdoc» an, die anderen wählen ihn in der Liste der Docs mit dem Personen-Knopf.</p>
                <div className="docs-row"><button type="button" className="btn small" disabled={busy} onClick={byFolder}>Ordner wählen …</button></div>
              </>
            ) : <button type="button" className="link-btn" onClick={() => setFolderWay(true)}>Lieber über einen Ordner teilen …</button>}
          </>
        ) : (
          <>
            <p>Andere schreiben auf ihrem Laptop mit, und ihr seht die Änderungen gegenseitig. Dafür braucht ihr einen <b>Ordner, den alle haben</b>: in OneDrive freigegeben, ein Teams-Kanal oder ein Laufwerk der Schule.</p>
            <p className="muted small">ASK legt dort «….askdoc» an. Die anderen klicken in der Liste der Docs auf den Personen-Knopf und wählen diesen Ordner.</p>
            <div className="docs-row"><button type="button" className="btn small primary" disabled={busy} onClick={byFolder}>{busy ? 'Teilt …' : 'Ordner wählen …'}</button></div>
          </>
        )}
      </div>
    );
  }
  return (
    <div className="docs-share">
      <div className="docs-row"><b>Geteilt</b><span className="spacer" /><button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={13} /></button></div>
      {share.fehler ? <p className="docs-error">{share.fehler}</p> : null}
      {share.link ? <LinkBox link={share.link} title={title} notice={notice} /> : null}
      <ul className="docs-people">
        {(share.personen || []).map((p, i) => (
          <li key={i}><Icon name="user" size={13} /> <b>{p.name}</b>{p.ich ? ' (du)' : ''}<span className="spacer" /><span className="muted small">{ago(p.zuletzt)}</span></li>
        ))}
      </ul>
      {(share.anderen || []).length ? (
        <>
          <p className="muted small">Zuletzt von anderen geändert:</p>
          <ul className="docs-recent">{share.anderen.map((c, i) => <li key={i}><b>{c.name}</b> {ago(c.zeit)}: «{c.text}»</li>)}</ul>
        </>
      ) : null}
      {share.link ? null : <p className="muted small ell" title={share.ordner}>Ordner: {share.ordner}</p>}
      <div className="docs-row">
        <button type="button" className="btn small" onClick={() => api().docsShowFolder(path)}>{share.link ? 'In OneDrive öffnen' : 'Ordner öffnen'}</button>
        <button type="button" className="btn small" onClick={stop}>Nicht mehr teilen</button>
      </div>
    </div>
  );
}

// Geteiltes Doc öffnen: Link aus Teams einfügen, oder einen Ordner wählen
function JoinPanel({ onOpen, onClose, notice }) {
  const [link, setLink] = useState('');
  const [busy, setBusy] = useState(false);
  const [ms, refreshMs] = useMicrosoft();
  const open = async (e) => {
    e.preventDefault();
    setBusy(true);
    const r = await api().docsJoinLink(link.trim());
    setBusy(false);
    if (r?.ok) onOpen(r.path); else notice?.(r?.reason || 'Nicht geöffnet.');
  };
  const folder = async () => { const r = await api().docsJoin(); if (r?.ok) onOpen(r.path); else if (!r?.canceled) notice?.(r?.reason || 'Nicht geöffnet.'); };
  return (
    <div className="docs-share">
      <div className="docs-row"><b>Geteiltes Doc öffnen</b><span className="spacer" /><button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={13} /></button></div>
      {!ms ? null : ms.eingerichtet ? (
        <>
          <MsLogin ms={ms} onDone={refreshMs} notice={notice} />
          <form className="docs-link" onSubmit={open}>
            <input autoFocus value={link} placeholder="Link aus Teams einfügen (https://…)" aria-label="Link zum geteilten Doc" onChange={(e) => setLink(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); onClose(); } }} />
            <button type="submit" className="btn small primary" disabled={busy || !link.trim().startsWith('https://') || !ms?.angemeldet}>{busy ? 'Öffnet …' : 'Öffnen'}</button>
          </form>
        </>
      ) : null}
      {ms?.eingerichtet ? <button type="button" className="link-btn" onClick={folder}>Oder einen geteilten Ordner «….askdoc» wählen …</button> : ms ? (
        <>
          <p className="muted small">Hat jemand ein Doc mit dir geteilt, wähl den Ordner «….askdoc», den ihr gemeinsam habt (OneDrive, Teams-Kanal oder Laufwerk der Schule).</p>
          <div className="docs-row"><button type="button" className="btn small primary" onClick={folder}>Ordner wählen …</button></div>
        </>
      ) : null}
    </div>
  );
}

// ---------- Kommentare ----------

function Entry({ e }) {
  return <p className={`docs-entry ${e.von === 'ask' ? 'ask' : ''}`}><b>{e.von === 'ask' ? 'ASK' : 'Du'}</b> {e.text}</p>;
}

function Thread({ t, onReply, onDone, onDelete, onShow }) {
  const [reply, setReply] = useState('');
  const [open, setOpen] = useState(false);
  const send = async (toAsk) => { if (!reply.trim()) return; await onReply(t, reply, toAsk); setReply(''); setOpen(false); };
  return (
    <li className={`docs-thread${t.erledigt ? ' done' : ''}`}>
      {t.zitat ? <button type="button" className={`docs-quote ${t.farbe || 'gelb'}`} onClick={() => onShow(t)} title="Stelle im Doc zeigen">«{t.zitat}»</button> : <span className="docs-quote whole">Zum ganzen Doc</span>}
      {(t.eintraege || []).map((e, i) => <Entry key={i} e={e} />)}
      {!(t.eintraege || []).length ? <p className="docs-entry muted">Angestrichen</p> : null}
      {open ? (
        <div className="docs-reply">
          <textarea autoFocus rows={2} value={reply} placeholder="Antworten …" onChange={(e) => setReply(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) send(true); if (e.key === 'Escape') { e.stopPropagation(); setOpen(false); } }} />
          <div className="docs-row">
            <button type="button" className="btn small primary" disabled={!reply.trim()} onClick={() => send(true)}>An ASK senden</button>
            <button type="button" className="btn small" disabled={!reply.trim()} onClick={() => send(false)}>Nur notieren</button>
          </div>
        </div>
      ) : null}
      <div className="docs-row">
        {!open ? <button type="button" className="link-btn" onClick={() => setOpen(true)}>Antworten</button> : null}
        <button type="button" className="link-btn" onClick={() => onDone(t, !t.erledigt)}>{t.erledigt ? 'Wieder öffnen' : 'Erledigt'}</button>
        <span className="spacer" />
        <button type="button" className="icon-btn" title="Kommentar löschen" aria-label="Kommentar löschen" onClick={() => onDelete(t)}><Icon name="trash" size={13} /></button>
      </div>
    </li>
  );
}

// ---------- Editor ----------

// Textstellen finden, auch wenn die Markierung über mehrere Absätze ging (Leerraum zählt nicht)
function rangeFor(root, quote) {
  const want = String(quote || '').replace(/\s+/g, '');
  if (!want || !root) return null;
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  const map = [];
  let flat = '';
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    const t = n.textContent;
    for (let i = 0; i < t.length; i++) if (!/\s/.test(t[i])) { flat += t[i]; map.push([n, i]); }
  }
  const at = flat.indexOf(want);
  if (at < 0) return null;
  const r = document.createRange();
  const [sn, so] = map[at];
  const [en, eo] = map[at + want.length - 1];
  r.setStart(sn, so);
  r.setEnd(en, eo + 1);
  return r;
}

const BLOCKS = [['p', 'Text'], ['h1', 'Titel'], ['h2', 'Überschrift'], ['h3', 'Unterüberschrift'], ['blockquote', 'Zitat'], ['pre', 'Code']];

// Bildpfad im Doc (relativ zum Doc) → Adresse im Projekt; nie aus dem Projekt hinaus
export function imageAddress(docPath, src, v = '') {
  const parts = String(docPath).split('/').slice(0, -1);
  for (const p of String(src).split('/')) {
    if (p === '..') { if (!parts.length) return ''; parts.pop(); } else if (p && p !== '.') parts.push(p);
  }
  return parts.length ? fileUrl(parts.join('/'), v) : '';
}

function blockOf(node, root) {
  for (let n = node; n && n !== root; n = n.parentNode) {
    const name = n.nodeName?.toLowerCase();
    if (BLOCKS.some(([b]) => b === name)) return name;
    if (name === 'li') return 'p';
  }
  return 'p';
}

function DocEditor({ path, live, files, synced, running, onBack, onAsk, onDirty, notice }) {
  const ed = useRef(null);
  const scroller = useRef(null);
  const hash = useRef('');
  const saved = useRef('');
  const saving = useRef(null); // Markdown, das gerade gespeichert wird (eigene Änderung, kein Konflikt)
  const dirty = useRef(false);
  const seq = useRef(0);
  const timer = useRef(null);
  const [title, setTitle] = useState('');
  const [status, setStatus] = useState('');
  const [conflict, setConflict] = useState(null);
  const [comments, setComments] = useState([]);
  const [showDone, setShowDone] = useState(false);
  const [draft, setDraft] = useState(null); // { quote, text }
  const [block, setBlock] = useState('p');
  const [link, setLink] = useState(null); // { url, range }
  const [menu, setMenu] = useState(null);
  const [writing, setWriting] = useState(false);
  const [missing, setMissing] = useState(false);
  const [share, setShare] = useState(null);
  const [sharing, setSharing] = useState(false);
  const shared = useRef(false);
  const [ctxMenu, setCtxMenu] = useState(null); // Rechtsklick auf markierten Text: { x, y, quote, range }
  const [toolMenu, setToolMenu] = useState(null); // Menüs «Anstreichen» und «ASK» oben: { kind, x, y, quote }
  const [markColor, setMarkColor] = useState('gelb');

  const setDirty = (on) => { dirty.current = on; onDirty?.(on); };

  const show = useCallback((md) => {
    const box = scroller.current;
    const top = box?.scrollTop || 0;
    const v = Date.now(); // Bilder nach einer Änderung neu laden
    if (ed.current) ed.current.innerHTML = mdToHtml(md, { image: (src) => imageAddress(path, src, v) });
    if (box) box.scrollTop = top;
  }, [path]);

  const load = useCallback(async (force, retry = true) => {
    const r = await api().docsRead(path);
    if (!r?.ok) { setMissing(true); return; }
    setMissing(false);
    setComments(r.comments || []);
    setShare(r.share || null);
    shared.current = !!r.share?.geteilt;
    // Nach «nicht gefunden» ist der Editor noch nicht wieder da: nach dem Zeichnen nochmals laden, sonst
    // landete der Text nirgends und die Prüfsumme sagte trotzdem «geladen»
    if (!ed.current) { if (retry) setTimeout(() => load(true, false), 0); return; }
    if (!force && r.hash === hash.current) return;
    if (!force && saving.current !== null && r.content === saving.current) return; // eigenes Speichern
    // Geteiltes Doc: Andere haben geschrieben, während du tippst. Deine Änderung wird gegen deinen Stand zusammengeführt.
    if (!force && dirty.current && shared.current) { saveRef.current(); return; }
    if (!force && dirty.current) { setConflict({ content: r.content, hash: r.hash }); return; }
    show(r.content);
    hash.current = r.hash;
    saved.current = r.content;
    setTitle(r.title);
    setConflict(null);
    setDirty(false);
    setStatus('');
  }, [path, show]);  

  const save = useCallback(async () => {
    clearTimeout(timer.current);
    timer.current = null;
    if (!dirty.current || !ed.current || conflict) return;
    const at = seq.current;
    const md = htmlToMd(ed.current);
    if (md === saved.current) { setDirty(false); setStatus('Gespeichert'); return; }
    setStatus('Speichert …');
    saving.current = md;
    const r = await api().docsSave(path, md, hash.current).finally(() => { saving.current = null; });
    if (r?.ok) {
      hash.current = r.hash;
      saved.current = md;
      setTitle(r.title || title);
      if (seq.current === at) {
        setDirty(false);
        setStatus('Gespeichert');
        // Zusammengeführt mit Änderungen anderer: zeigen, was jetzt gilt
        if (r.merged && typeof r.content === 'string') { show(r.content); saved.current = r.content; }
      }
    } else if (r?.conflict) {
      setConflict({ content: r.content, hash: r.hash });
      setStatus('Nicht gespeichert');
    } else {
      setStatus(`Nicht gespeichert: ${r?.reason || 'Fehler'}`);
    }
  }, [path, conflict, title]);  

  const saveRef = useRef(save);
  saveRef.current = save;

  useEffect(() => { hash.current = ''; setDirty(false); load(true); }, [path]);  
  // Dateien im Ordner geändert (auch durch ASK über den Ordner): neu lesen, falls es nicht die eigene Änderung war
  useEffect(() => { if (hash.current) load(false); }, [files, synced]);  
  // ASK schreibt an diesem Doc
  useEffect(() => {
    if (live?.path !== path) return undefined;
    load(false);
    setWriting(true);
    const t = setTimeout(() => setWriting(false), 4000);
    return () => clearTimeout(t);
  }, [live]);  
  // Beim Verlassen: Ungespeichertes sofort speichern. Als Layout-Effekt, weil React den Editor (ed.current)
  // vor dem Aufräumen gewöhnlicher Effekte schon abgehängt hat; save liest den Text vor dem ersten await.
  useLayoutEffect(() => () => { if (dirty.current) saveRef.current(); onDirty?.(false); }, []);  

  const changed = () => {
    seq.current += 1;
    setDirty(true);
    setStatus('Nicht gespeichert');
    clearTimeout(timer.current);
    timer.current = setTimeout(() => saveRef.current(), SAVE_MS);
  };

  // Markierte Stellen der offenen Kommentare (CSS Custom Highlight API, ändert das DOM nicht)
  const paintMarks = useCallback(() => {
    if (!window.CSS?.highlights || typeof window.Highlight !== 'function') return;
    for (const [c] of COLORS) {
      const ranges = comments.filter((x) => !x.erledigt && x.zitat && (x.farbe || 'gelb') === c).map((x) => rangeFor(ed.current, x.zitat)).filter(Boolean);
      CSS.highlights.set(`doc-${c}`, new window.Highlight(...ranges));
    }
  }, [comments]);
  useLayoutEffect(() => { paintMarks(); }, [paintMarks, title, status]);
  useEffect(() => () => { for (const [c] of COLORS) window.CSS?.highlights?.delete(`doc-${c}`); }, []);

  // Markierter Text im Doc (für Anstreichen und die Bitten an ASK)
  const selected = () => {
    const sel = window.getSelection();
    if (!sel || sel.isCollapsed || !sel.rangeCount || !ed.current?.contains(sel.anchorNode)) return null;
    const quote = String(sel).trim().slice(0, 500);
    return quote ? { quote, range: sel.getRangeAt(0).cloneRange() } : null;
  };
  // Rechtsklick auf markierten Text: Menü von ASK. Ohne Markierung bleibt das Menü von Windows (Einfügen, Rechtschreibung).
  const onContextMenu = (e) => {
    const s = selected();
    if (!s) return;
    e.preventDefault();
    setCtxMenu({ x: e.clientX, y: e.clientY, ...s });
  };
  const restore = (range) => {
    if (!range) return;
    ed.current?.focus();
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(range);
  };
  const openTool = (kind) => (e) => {
    const s = selected();
    const r = e.currentTarget.getBoundingClientRect();
    setToolMenu({ kind, x: r.left, y: r.bottom + 4, quote: s?.quote || '' });
  };

  const exec = (cmd, value = null) => {
    ed.current?.focus();
    document.execCommand(cmd, false, value);
    changed();
  };
  const keep = (e) => e.preventDefault(); // Knöpfe nehmen dem Editor die Markierung nicht weg

  const updateBlock = () => {
    const sel = window.getSelection();
    if (sel?.anchorNode && ed.current?.contains(sel.anchorNode)) setBlock(blockOf(sel.anchorNode, ed.current));
  };
  useEffect(() => {
    document.addEventListener('selectionchange', updateBlock);
    return () => document.removeEventListener('selectionchange', updateBlock);
  });

  const onPaste = (e) => {
    e.preventDefault();
    const html = e.clipboardData.getData('text/html');
    const text = e.clipboardData.getData('text/plain');
    // Eingefügtes läuft durch Markdown: nur bekannte Formatierung, nichts Fremdes (Skripte, Stile, Bilder)
    const md = html ? htmlToMd(new DOMParser().parseFromString(html, 'text/html').body) : text;
    if (!md.trim()) return;
    const single = !md.trim().includes('\n') && !/^(#|>|[-*+] |\d+\. |\||```)/.test(md.trim());
    const out = mdToHtml(md, { image: (src) => imageAddress(path, src) });
    document.execCommand('insertHTML', false, single ? out.replace(/^<p>|<\/p>$/g, '') : out);
    changed();
  };

  const onKey = (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') { e.preventDefault(); save(); }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); openLink(); }
  };

  const openLink = () => {
    const sel = window.getSelection();
    const range = sel?.rangeCount && ed.current?.contains(sel.anchorNode) ? sel.getRangeAt(0).cloneRange() : null;
    setLink({ url: 'https://', range });
  };
  const applyLink = (e) => {
    e.preventDefault();
    const url = link.url.trim();
    const range = link.range;
    setLink(null);
    if (!/^(https?:\/\/|mailto:)\S+$/i.test(url)) { notice?.('Links gehen nur auf Webadressen (https://…) oder E-Mail (mailto:…).'); return; }
    ed.current?.focus();
    if (range) { const sel = window.getSelection(); sel.removeAllRanges(); sel.addRange(range); }
    if (range && !range.collapsed) document.execCommand('createLink', false, url);
    else document.execCommand('insertHTML', false, `<a href="${url.replace(/"/g, '&quot;')}">${url.replace(/</g, '&lt;')}</a>`);
    changed();
  };

  const startComment = (given) => {
    const quote = typeof given === 'string' ? given : (selected()?.quote || '');
    setDraft({ quote, text: '', farbe: quote ? 'gelb' : '' });
  };
  const comment = async ({ quote, text, farbe = '', toAsk }) => {
    await save();
    const r = await api().docsCommentAdd(path, quote, text, farbe);
    if (!r?.ok) { notice?.(r?.reason || 'Kommentar nicht gespeichert.'); return false; }
    setComments((c) => [...c, r.thread]);
    if (toAsk) onAsk(r.message);
    return true;
  };
  const addComment = async (toAsk) => {
    const text = draft.text.trim();
    if (!text) return;
    if (await comment({ quote: draft.quote, text, farbe: draft.farbe || '', toAsk })) setDraft(null);
  };
  const needText = () => notice?.('Markiere zuerst Text im Doc, dann geht es mit Rechtsklick oder den Knöpfen oben.');
  const quickAction = async (key, quote) => {
    const text = ACTIONS.find((a) => a[0] === key)?.[3];
    if (!quote) { needText(); return; }
    window.getSelection()?.removeAllRanges();
    if (text) await comment({ quote, text, farbe: 'gelb', toAsk: true });
  };
  const markOnly = async (farbe, quote) => {
    if (!quote) { needText(); return; }
    setMarkColor(farbe);
    window.getSelection()?.removeAllRanges();
    await comment({ quote, text: '', farbe });
  };
  // Einträge der Menüs: Bitten an ASK, Anstreichen
  const askItems = (quote) => [
    ...ACTIONS.map(([key, label, icon]) => ({ label, icon, run: () => quickAction(key, quote) })),
    { label: 'Bitte an ASK …', icon: 'chat', run: () => startComment(quote) },
  ];
  const colorItems = (quote) => COLORS.map(([c, label]) => ({ label, swatch: `var(--mark-${c})`, type: 'radio', checked: c === markColor, run: () => markOnly(c, quote) }));
  const feedback = async () => {
    await save();
    onAsk(`Gib mir Feedback und Vorschläge zum Doc ${path}: Was ist gut, was fehlt, was ist unklar oder falsch? Lies es mit doc_lesen und setz deine Vorschläge als Kommentare an die passenden Stellen (doc_kommentieren ohne id, mit zitat). Ändere das Doc selbst nicht.`);
  };
  const replyTo = async (t, text, toAsk) => {
    await save();
    const r = await api().docsCommentReply(path, t.id, text);
    if (!r?.ok) { notice?.(r?.reason || 'Antwort nicht gespeichert.'); return; }
    setComments((c) => c.map((x) => (x.id === t.id ? r.thread : x)));
    if (toAsk) onAsk(r.message);
  };
  const setDone = async (t, on) => {
    const r = await api().docsCommentDone(path, t.id, on);
    if (r?.ok) setComments((c) => c.map((x) => (x.id === t.id ? r.thread : x)));
  };
  const removeComment = async (t) => {
    const r = await api().docsCommentDelete(path, t.id);
    if (r?.ok) setComments((c) => c.filter((x) => x.id !== t.id));
  };
  const showQuote = (t) => {
    const r = rangeFor(ed.current, t.zitat);
    const el = r?.startContainer?.parentElement;
    if (!el) { notice?.('Die Stelle steht so nicht mehr im Doc.'); return; }
    el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    const sel = window.getSelection();
    sel.removeAllRanges();
    sel.addRange(r);
  };

  const exportAs = async (format) => {
    await save();
    const r = await api().docsExport(path, format);
    if (r?.ok) notice?.(`Gespeichert: ${r.path}`);
    else if (!r?.canceled) notice?.(r?.reason || 'Nicht gespeichert.');
  };
  const remove = async () => {
    clearTimeout(timer.current);
    const r = await api().docsDelete(path);
    if (r?.ok) { setDirty(false); onBack(); }
  };

  const keepMine = async () => {
    hash.current = conflict.hash;
    setConflict(null);
    setTimeout(() => saveRef.current(), 0);
  };
  const takeTheirs = () => {
    show(conflict.content);
    hash.current = conflict.hash;
    saved.current = conflict.content;
    setConflict(null);
    setDirty(false);
    setStatus('');
  };

  const open = comments.filter((c) => !c.erledigt);
  const shown = showDone ? comments : open;
  const tool = (icon, label, run, extra = {}) => (
    <button type="button" className="icon-btn" title={label} aria-label={label} onMouseDown={keep} onClick={run} {...extra}><Icon name={icon} size={15} /></button>
  );
  const txt = (label, short, run, cls = '') => (
    <button type="button" className={`icon-btn docs-txt ${cls}`} title={label} aria-label={label} onMouseDown={keep} onClick={run}>{short}</button>
  );

  if (missing) {
    return (
      <div className="gh-empty">
        <b>Das Doc gibt es nicht mehr</b>
        <p className="muted">{path} wurde verschoben oder gelöscht.</p>
        <button type="button" className="btn small" onClick={onBack}>Zur Liste</button>
      </div>
    );
  }

  return (
    <div className="docs-editor">
      <div className="docs-head">
        <button type="button" className="icon-btn" title="Alle Docs" aria-label="Alle Docs" onClick={async () => { await save(); onBack(); }}><Icon name="list" size={15} /></button>
        <b className="ell" title={path}>{title || path}</b>
        {share?.geteilt ? <button type="button" className="docs-shared-btn" title="Geteilt: wer mitschreibt" onClick={() => setSharing(!sharing)}>
          <Icon name="user" size={13} /> {(share.personen || []).length}</button> : null}
        <span className="spacer" />
        {writing && running ? <span className="docs-live" title="ASK schreibt an diesem Doc"><span className="live-dot" aria-hidden="true" /> ASK schreibt</span>
          : <span className="muted small docs-status" aria-live="polite">{status}</span>}
        {!share?.geteilt ? <button type="button" className="icon-btn" title="Live teilen: andere schreiben mit" aria-label="Teilen" onClick={() => setSharing(!sharing)}><Icon name="user" size={15} /></button> : null}
        <button type="button" className="icon-btn" title="Feedback von ASK: Vorschläge als Kommentare" aria-label="Feedback von ASK" onClick={feedback}><Icon name="spark" size={15} /></button>
        <button type="button" className="icon-btn" title="Speichern als Word, PDF oder Markdown" aria-label="Speichern als …"
          onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setMenu({ x: r.left - 120, y: r.bottom + 4 }); }}><Icon name="down" size={15} /></button>
        <button type="button" className="icon-btn" title="Doc löschen" aria-label="Doc löschen" onClick={remove}><Icon name="trash" size={14} /></button>
      </div>
      {menu ? <PopMenu x={menu.x} y={menu.y} label="Speichern als" onClose={() => setMenu(null)} items={[
        { label: 'Word (.docx)', icon: 'file', run: () => exportAs('docx') },
        { label: 'PDF (mit Word)', icon: 'file', run: () => exportAs('pdf') },
        { label: 'Markdown (.md)', icon: 'file', run: () => exportAs('md') },
      ]} /> : null}
      {sharing ? <SharePanel path={path} title={title} share={share} notice={notice} onClose={() => setSharing(false)} onChanged={() => load(true)} /> : null}
      <div className="docs-tools" role="toolbar" aria-label="Formatierung">
        <select value={block} aria-label="Absatzart" onChange={(e) => { exec('formatBlock', e.target.value); setBlock(e.target.value); }}>
          {BLOCKS.map(([v, l]) => <option key={v} value={v}>{l}</option>)}
        </select>
        {txt('Fett (Ctrl+B)', 'F', () => exec('bold'), 'b')}
        {txt('Kursiv (Ctrl+I)', 'K', () => exec('italic'), 'i')}
        {txt('Durchgestrichen', 'S', () => exec('strikeThrough'), 's')}
        {tool('list', 'Aufzählung', () => exec('insertUnorderedList'))}
        {txt('Nummerierte Liste', '1.', () => exec('insertOrderedList'))}
        {tool('globe', 'Link (Ctrl+K)', openLink)}
        {tool('chart', 'Tabelle einfügen', () => exec('insertHTML', '<table><thead><tr><th>Spalte 1</th><th>Spalte 2</th></tr></thead><tbody><tr><td><br></td><td><br></td></tr></tbody></table><p><br></p>'))}
        {txt('Trennlinie', '—', () => exec('insertHorizontalRule'))}
        <span className="spacer" />
        <span className="docs-split">
          <button type="button" className="btn small" title="Markierten Text anstreichen (auch mit Rechtsklick)" onMouseDown={keep}
            onClick={() => markOnly(markColor, selected()?.quote)}><Icon name="marker" size={13} /><span className={`docs-dot ${markColor}`} aria-hidden="true" /> Anstreichen</button>
          <button type="button" className="btn small" title="Farbe wählen" aria-label="Farbe zum Anstreichen wählen" onMouseDown={keep} onClick={openTool('farbe')}><Icon name="chevDown" size={12} /></button>
        </span>
        <button type="button" className="btn small" title="Zum markierten Text: erklären, Grafik, einfacher, Beispiel, eigene Bitte (auch mit Rechtsklick)" onMouseDown={keep}
          onClick={openTool('ask')}><Icon name="spark" size={13} /> ASK <Icon name="chevDown" size={12} /></button>
        <button type="button" className="icon-btn" title="Kommentar schreiben (zur Markierung oder zum ganzen Doc)" aria-label="Kommentar" onMouseDown={keep} onClick={() => startComment()}><Icon name="chat" size={14} /></button>
      </div>
      {toolMenu ? <PopMenu x={toolMenu.x} y={toolMenu.y} label={toolMenu.kind === 'ask' ? 'ASK' : 'Farbe'} onClose={() => setToolMenu(null)}
        items={toolMenu.kind === 'ask' ? askItems(toolMenu.quote) : colorItems(toolMenu.quote)} /> : null}
      {ctxMenu ? <PopMenu x={ctxMenu.x} y={ctxMenu.y} label="Markierter Text" onClose={() => setCtxMenu(null)} items={[
        { heading: true, label: 'ASK fragen' },
        ...askItems(ctxMenu.quote),
        { separator: true },
        { label: 'Anstreichen', icon: 'marker', submenu: colorItems(ctxMenu.quote) },
        { separator: true },
        { label: 'Ausschneiden', hint: 'Ctrl+X', run: () => { restore(ctxMenu.range); document.execCommand('cut'); changed(); } },
        { label: 'Kopieren', hint: 'Ctrl+C', run: () => { restore(ctxMenu.range); document.execCommand('copy'); } },
      ]} /> : null}
      {link ? (
        <form className="docs-link" onSubmit={applyLink}>
          <input autoFocus value={link.url} aria-label="Adresse des Links" onChange={(e) => setLink({ ...link, url: e.target.value })}
            onKeyDown={(e) => { if (e.key === 'Escape') { e.stopPropagation(); setLink(null); } }} />
          <button type="submit" className="btn small primary">Link setzen</button>
          <button type="button" className="btn small" onClick={() => setLink(null)}>Abbrechen</button>
        </form>
      ) : null}
      {conflict ? (
        <div className="docs-conflict" role="alert">
          <span>ASK hat das Doc geändert, während du getippt hast.</span>
          <div className="docs-row">
            <button type="button" className="btn small primary" onClick={takeTheirs}>Fassung von ASK zeigen</button>
            <button type="button" className="btn small" onClick={keepMine}>Meine behalten</button>
          </div>
        </div>
      ) : null}
      <div className="docs-scroll" ref={scroller}>
        <div ref={ed} className="docs-page md" contentEditable suppressContentEditableWarning spellCheck lang="de-CH" role="textbox" aria-multiline="true"
          aria-label={`Doc ${title}`} onInput={changed} onPaste={onPaste} onKeyDown={onKey} onContextMenu={onContextMenu} onBlur={() => { if (dirty.current) save(); }} />
      </div>
      <div className="docs-comments">
        <div className="docs-row docs-comments-head">
          <b>Kommentare</b>{open.length ? <span className="docs-badge">{open.length}</span> : null}
          <span className="spacer" />
          {comments.length > open.length ? <button type="button" className="link-btn" onClick={() => setShowDone(!showDone)}>{showDone ? 'Erledigte ausblenden' : `Erledigte zeigen (${comments.length - open.length})`}</button> : null}
        </div>
        {draft ? (
          <div className="docs-draft">
            <span className="docs-quote whole">{draft.quote ? `«${draft.quote}»` : 'Zum ganzen Doc (vorher Text markieren für eine Stelle)'}</span>
            <textarea autoFocus rows={3} value={draft.text} placeholder="Was soll ASK hier tun? Z. B. «kürzer», «Quelle?», «Beispiel dazu»"
              onChange={(e) => setDraft({ ...draft, text: e.target.value })}
              onKeyDown={(e) => { if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) addComment(true); if (e.key === 'Escape') { e.stopPropagation(); setDraft(null); } }} />
            <div className="docs-row">
              <button type="button" className="btn small primary" disabled={!draft.text.trim()} onClick={() => addComment(true)}>An ASK senden</button>
              <button type="button" className="btn small" disabled={!draft.text.trim()} onClick={() => addComment(false)}>Nur notieren</button>
              <button type="button" className="btn small" onClick={() => setDraft(null)}>Abbrechen</button>
            </div>
          </div>
        ) : null}
        {shown.length ? (
          <ul className="docs-threads">
            {shown.map((t) => <Thread key={t.id} t={t} onReply={replyTo} onDone={setDone} onDelete={removeComment} onShow={showQuote} />)}
          </ul>
        ) : !draft ? <p className="muted small docs-pad">Text markieren und «Kommentar» wählen. ASK antwortet im Kommentar und ändert die Stelle.</p> : null}
      </div>
    </div>
  );
}

// ---------- Reiter ----------

export default function DocsPane({ root, free, files, live, path, setPath, running, onAsk, onCompose, notice }) {
  const [docs, setDocs] = useState(null);
  const dirty = useRef(false);
  const loadList = useCallback(async () => {
    const r = await api().docsList();
    setDocs(r?.ok ? r.docs : []);
  }, []);
  const [synced, setSynced] = useState(0);
  useEffect(() => { if (!path) loadList(); }, [root, files, live, path, loadList, synced]);
  // Geteilte Docs: Änderungen der anderen sind angekommen
  useEffect(() => api().onDocsSync?.(() => setSynced(Date.now())), []);
  // Welches Doc offen ist, steht im Systemprompt («dieses Doc»)
  useEffect(() => { api().docsOpen(path || ''); }, [path, root]);
  useEffect(() => () => { api().docsOpen(''); }, []);
  // ASK hat ein anderes Doc angelegt oder geändert: dorthin wechseln, ausser die Person tippt gerade
  useEffect(() => { if (live?.path && live.path !== path && !dirty.current) setPath(live.path); }, [live]);  
  return (
    <div className="docs-pane">
      {path ? <DocEditor key={`${root}|${path}`} path={path} live={live} files={files} synced={synced} running={running} onBack={() => setPath('')}
        onAsk={onAsk} onDirty={(on) => { dirty.current = on; }} notice={notice} />
        : <DocList docs={docs} free={free} onOpen={setPath} onCompose={onCompose} notice={notice} />}
    </div>
  );
}
