// Einstellungen als grosses Fenster mit Bereichen links (ENTSCHEIDE E73), dazu Skills, Kataloge und Mods (E70–E72).
// Suchen filtert alle Zeilen der Bereiche. Untere Dialoge (Skill bearbeiten, Katalog, Vorschau eines Mods) kehren
// beim Schliessen in denselben Bereich zurück (dialog.back).
import { useEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { api } from '../api.js';
import { Segmented, UPDATE_TEXT, UsageView } from './Dialogs.jsx';
import { formatSize } from '../lib/format.js';
import { keepDialogOverReload } from '../lib/mods.js';

export const PANES = [
  [
    { id: 'konto', label: 'Konto', icon: 'user' },
    { id: 'nutzung', label: 'Nutzung', icon: 'chart' },
  ],
  [
    { id: 'darstellung', label: 'Darstellung', icon: 'palette' },
    { id: 'chat', label: 'Chat', icon: 'chat' },
    { id: 'modelle', label: 'Modelle', icon: 'spark' },
    { id: 'gedaechtnis', label: 'Gedächtnis', icon: 'pin' },
    { id: 'benachrichtigungen', label: 'Benachrichtigungen', icon: 'bell' },
  ],
  [
    { id: 'skills', label: 'Skills', icon: 'bolt' },
    { id: 'mods', label: 'Mods', icon: 'puzzle' },
    { id: 'github', label: 'GitHub', icon: 'github' },
  ],
  [
    { id: 'ueber', label: 'Über ASK', icon: 'info' },
  ],
];
const ALL = PANES.flat();

export function Toggle({ checked, onChange, label, id, disabled }) {
  return (
    <button type="button" id={id} role="switch" aria-checked={!!checked} aria-label={label} disabled={disabled}
      className={`toggle${checked ? ' on' : ''}`} onClick={() => onChange(!checked)}><span className="knob" /></button>
  );
}

function Row({ title, text, badge, children, id }) {
  return (
    <div className="s-row" id={id}>
      <div className="s-row-text">
        <div className="s-title">{title}{badge ? <span className="s-badge">{badge}</span> : null}</div>
        {text ? <div className="s-text">{text}</div> : null}
      </div>
      {children ? <div className="s-row-ctl">{children}</div> : null}
    </div>
  );
}

function ModelSelect({ id, value, models, onChange, label }) {
  return (
    <label className="s-select">
      <select id={id} value={value || ''} onChange={(e) => onChange(e.target.value)} aria-label={label}>
        <option value="">Wie im Chat</option>
        {(models || []).map((m) => <option key={m.id} value={m.id}>{m.name}</option>)}
      </select>
      <Icon name="chevDown" size={13} />
    </label>
  );
}

// Zahl mit Einheit (Token): übernimmt beim Verlassen des Felds oder mit Enter; der Hauptprozess holt sie in den erlaubten Bereich
function NumberField({ id, label, value, min, max, step = 1000, unit = 'Token', onChange }) {
  const [text, setText] = useState(String(value ?? ''));
  useEffect(() => { setText(String(value ?? '')); }, [value]);
  const commit = () => {
    const n = Math.round(Number(String(text).replace(/[’'\s]/g, '')));
    if (!Number.isFinite(n) || n <= 0) { setText(String(value ?? '')); return; }
    const v = Math.min(max, Math.max(min, n));
    setText(String(v));
    if (v !== value) onChange(v);
  };
  return (
    <label className="s-number">
      <input id={id} type="number" inputMode="numeric" min={min} max={max} step={step} value={text} aria-label={label}
        onChange={(e) => setText(e.target.value)} onBlur={commit} onKeyDown={(e) => { if (e.key === 'Enter') { e.preventDefault(); e.target.blur(); } }} />
      <span>{unit}</span>
    </label>
  );
}

// Zeilen der einfachen Bereiche als Daten, damit die Suche sie findet
function rowsOf(state, go, onVerbose, close) {
  const u = state.update || {};
  const loggedIn = state.ask?.state === 'angemeldet';
  return [
    { pane: 'konto', title: 'ASK-Konto', text: loggedIn ? `Angemeldet als ${state.ask.user || '(unbekannt)'}` : 'Nicht angemeldet. «Anmelden» öffnet die Anmeldung von ASK in einem eigenen Fenster.',
      control: loggedIn
        ? <button type="button" className="btn" onClick={async () => { close(); await api().logout(); }}><Icon name="logout" size={15} /> Abmelden</button>
        : <><button type="button" id="settings-login" className="btn" onClick={() => api().askLoginWindow()}>Anmelden</button>
          <button type="button" id="settings-login-browser" className="btn small" onClick={() => go('ask-token')}>Im Browser …</button></> },
    { pane: 'konto', title: 'Adresse von ASK', text: state.askUrl || '–', keywords: 'url server schule' },
    { pane: 'darstellung', title: 'Farbschema', text: 'Hell, Dunkel oder wie in Windows eingestellt.', keywords: 'theme hell dunkel',
      control: <Segmented name="Farbschema" value={state.theme || 'system'} onChange={(theme) => api().setSettings({ theme })} options={[['system', 'System'], ['hell', 'Hell'], ['dunkel', 'Dunkel']]} /> },
    { pane: 'darstellung', title: 'Grösse', text: 'Ctrl + und Ctrl − vergrössern und verkleinern, Ctrl 0 setzt zurück.', keywords: 'zoom schrift' },
    { pane: 'darstellung', title: 'Eigenes Aussehen', text: 'Farben, Schriften und Figuren über Mods ändern.',
      control: <button type="button" className="btn" onClick={() => go('settings', { pane: 'mods' })}>Mods …</button> },
    { pane: 'chat', title: 'Details zeigen', text: 'Gedanken und alle Arbeitsschritte zeigen (Ctrl+O).', keywords: 'ansicht ausführlich verbose',
      control: <Toggle id="pref-verbose" label="Details zeigen" checked={!!state.prefs?.verbose} onChange={onVerbose} /> },
    { pane: 'chat', title: 'Aufgabenliste einklappen', text: 'Die Liste über dem Eingabefeld nach dem Auftrag einklappen.',
      control: <Toggle id="pref-todo" label="Aufgabenliste einklappen" checked={state.prefs?.todoCollapsed ?? true} onChange={(v) => api().setPrefs({ todoCollapsed: v })} /> },
    { pane: 'chat', title: 'Frühere Chats', text: 'ASK darf frühere Chats im selben Projekt durchsuchen und lesen (/chats).', keywords: 'verlauf',
      control: <Toggle label="Frühere Chats" checked={!!state.chatMemory} onChange={(v) => api().setChatMemory(v)} /> },
    { pane: 'modelle', title: 'Prüfung im Modus «Auto»', text: 'Ein Modell in ASK prüft jeden Befehl und fragt nur bei Gefährlichem. Ein schnelles Modell genügt.',
      control: <ModelSelect id="review-model" label="Modell für die Prüfung" value={state.reviewModel} models={state.models} onChange={(v) => api().setSettings({ reviewModel: v })} /> },
    { pane: 'modelle', title: 'Hilfsagenten', text: 'ASK kann Teilfragen an Hilfsagenten abgeben, die nur lesen und recherchieren (bis 3 gleichzeitig). Ein schnelles Modell spart Zeit.',
      control: <ModelSelect id="helper-model" label="Modell für Hilfsagenten" value={state.helperModel} models={state.models} onChange={(v) => api().setSettings({ helperModel: v })} /> },
    { pane: 'modelle', title: 'Kontextgrösse', text: 'So viel Verlauf geht höchstens an das Modell. Meldet ASK für ein Modell ein kleineres Fenster, gilt das kleinere. Der Ring im Eingabefeld zeigt, wie voll es ist.', keywords: 'kontext token fenster context verlauf',
      control: <NumberField id="context-size" label="Kontextgrösse in Token" value={state.contextSize ?? 250000} min={8000} max={2000000} onChange={(v) => api().setSettings({ contextSize: v })} /> },
    { pane: 'modelle', title: 'Reserve zum Zusammenfassen', text: 'Ist der Verlauf bis auf diese Reserve voll, fasst ASK die älteren Aufträge zusammen und arbeitet weiter; du siehst weiterhin den ganzen Chat. /zusammenfassen tut es sofort.', keywords: 'kontext komprimieren puffer buffer zusammenfassen kompakt',
      control: <NumberField id="context-reserve" label="Reserve in Token" value={state.contextReserve ?? 30000} min={2000} max={500000} onChange={(v) => api().setSettings({ contextReserve: v })} /> },
    { pane: 'gedaechtnis', title: 'Gedächtnis', text: 'ASK merkt sich Vorlieben, Kenntnisse und Projektwissen für alle Chats (/gedaechtnis).', badge: 'Nur lokal',
      control: <Toggle label="Gedächtnis" checked={state.memory !== false} onChange={(v) => api().setMemoryEnabled(v)} /> },
    { pane: 'gedaechtnis', title: 'Gespeicherte Einträge', text: 'Ansehen, ändern, löschen oder selbst etwas eintragen.',
      control: <button type="button" id="settings-memory" className="btn" onClick={() => go('memory')}>Ansehen …</button> },
    { pane: 'benachrichtigungen', title: 'Meldungen von Windows', text: 'Wenn ASK fertig ist oder etwas fragt und du in einem anderen Fenster bist.', keywords: 'benachrichtigung',
      control: <Toggle label="Benachrichtigungen" checked={state.prefs?.notify !== false} onChange={(v) => api().setPrefs({ notify: v })} /> },
    { pane: 'github', title: 'GitHub', text: 'Projekte von GitHub holen und hochladen, Branches und Pull Requests.',
      control: <button type="button" className="btn" onClick={() => go('github')}><Icon name="github" size={15} /> GitHub …</button> },
    { pane: 'ueber', title: `Version ${state.version}`, text: UPDATE_TEXT[u.state] || (u.enabled ? 'ASK sucht regelmässig nach neuen Versionen.' : 'Updates gibt es nur in der installierten App.'), keywords: 'update',
      control: u.enabled ? <button type="button" id="settings-update" className="btn" onClick={() => api().updateAction()}>Nach Updates suchen</button> : null },
    { pane: 'ueber', title: 'Beta-Versionen erhalten', text: 'Neue Funktionen früher ausprobieren. Beta-Versionen können Fehler haben.',
      control: <Toggle id="pref-beta" label="Beta-Versionen erhalten" checked={!!u.beta} onChange={(v) => api().setBetaUpdates(v, false)} /> },
    { pane: 'ueber', title: 'Einführung', text: 'Die kurze Tour durch ASK nochmals ansehen.', keywords: 'tour hilfe',
      control: <button type="button" id="settings-tour" className="btn" onClick={() => go('tour')}>Zeigen</button> },
    { pane: 'ueber', title: 'Tastenkombinationen', text: 'Alle Tastenkürzel auf einen Blick.', keywords: 'shortcuts tastatur',
      control: <button type="button" className="btn" onClick={() => go('shortcuts')}>Anzeigen</button> },
    { pane: 'ueber', title: 'Terminal', text: 'Meldungen des Terminals, in dem ASK Befehle ausführt.',
      control: <button type="button" className="btn" onClick={() => go('terminal')}>Terminal …</button> },
    { pane: 'ueber', title: 'Anmeldungen im Browser der App', text: 'Cookies und Anmeldungen im eingebauten Browser löschen.',
      control: <button type="button" className="btn" onClick={() => api().browserClear()}>Löschen …</button> },
  ];
}

export default function SettingsDialog({ state, pane: initialPane, go, onClose, onVerbose, onCompose, onPane }) {
  const [pane, setPaneState] = useState(ALL.some((p) => p.id === initialPane) ? initialPane : 'konto');
  const [query, setQuery] = useState('');
  // Bereich merken, damit untere Dialoge (Vorschau, Katalog …) dorthin zurückkehren
  const setPane = (id) => { setPaneState(id); onPane?.(id); };
  // Ein Knopf im Fenster selbst wechselt den Bereich über den Dialog («Mods …» unter Darstellung): übernehmen.
  // Ohne das blieb der Bereich stehen, weil er nur beim Öffnen gelesen wurde, und der Knopf tat nichts.
  useEffect(() => {
    if (ALL.some((p) => p.id === initialPane)) { setPaneState(initialPane); setQuery(''); }
  }, [initialPane]);
  const search = useRef(null);
  useEffect(() => { search.current?.focus(); }, []);
  const rows = rowsOf(state, go, onVerbose, onClose);
  const q = query.trim().toLowerCase();
  const hits = q ? rows.filter((r) => `${r.title} ${r.text || ''} ${r.keywords || ''} ${ALL.find((p) => p.id === r.pane)?.label}`.toLowerCase().includes(q)) : [];
  const paneHits = q ? ALL.filter((p) => p.label.toLowerCase().includes(q) || (p.id === 'skills' && /skill|katalog|anleitung/.test(q)) || (p.id === 'mods' && /mod|plugin|theme|design|figur/.test(q))) : [];
  const current = ALL.find((p) => p.id === pane);
  const showRow = (r, i) => <Row key={`${r.pane}${i}`} title={r.title} text={r.text} badge={r.badge}>{r.control}</Row>;
  return (
    <div id="dlg-settings" className="dialog settings-dialog" role="dialog" aria-modal="true" aria-labelledby="dlg-settings-title">
      <div className="dialog-head">
        <h1 id="dlg-settings-title">Einstellungen</h1>
        <button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={16} /></button>
      </div>
      <div className="settings-body">
        <nav className="settings-nav" aria-label="Bereiche">
          <label className="settings-search">
            <Icon name="search" size={15} />
            <input ref={search} type="search" placeholder="Suchen" value={query} onChange={(e) => setQuery(e.target.value)} aria-label="Einstellungen durchsuchen"
              onKeyDown={(e) => { if (e.key === 'Escape' && query) { e.stopPropagation(); setQuery(''); } }} />
          </label>
          {PANES.map((group, gi) => (
            <div key={gi} className="settings-group">
              {group.map((p) => (
                <button key={p.id} type="button" className={!q && pane === p.id ? 'on' : ''} aria-current={!q && pane === p.id ? 'page' : undefined}
                  onClick={() => { setQuery(''); setPane(p.id); }}>
                  <Icon name={p.icon} size={16} /> {p.label}
                </button>
              ))}
            </div>
          ))}
          <span className="spacer" />
          <button type="button" className="settings-foot" onClick={() => go('shortcuts')}>Tastenkombinationen <Icon name="arrowRight" size={13} /></button>
        </nav>
        <section className="settings-pane" aria-live="polite">
          {q ? (
            <>
              <h2>Suche</h2>
              {paneHits.map((p) => (
                <Row key={p.id} title={p.label} text="Bereich öffnen"><button type="button" className="btn small" onClick={() => { setQuery(''); setPane(p.id); }}>Öffnen</button></Row>
              ))}
              {hits.map(showRow)}
              {!hits.length && !paneHits.length ? <p className="muted">Nichts gefunden.</p> : null}
            </>
          ) : (
            <>
              <h2>{current.label}</h2>
              {pane === 'nutzung' ? <UsageView /> : null}
              {pane === 'skills' ? <SkillsPane state={state} go={go} onCompose={onCompose} /> : null}
              {pane === 'mods' ? <ModsPane state={state} go={go} onCompose={onCompose} /> : null}
              {rows.filter((r) => r.pane === pane).map(showRow)}
            </>
          )}
        </section>
      </div>
    </div>
  );
}

// ---------- Skills ----------

function useSkills() {
  const [list, setList] = useState(null);
  useEffect(() => {
    const load = () => api().skillsList().then((l) => setList(Array.isArray(l) ? l : [])).catch(() => setList([]));
    load();
    const off = api().onMemoryChanged?.(load);
    return () => { if (typeof off === 'function') off(); };
  }, []);
  return list;
}

function Message({ msg }) {
  if (!msg) return null;
  return <p className={`s-msg${msg.bad ? ' bad' : ''}`} role={msg.bad ? 'alert' : 'status'}>{msg.text}</p>;
}

function installedText(res) {
  if (res?.kind === 'mod') return `Mod «${res.mod?.name || ''}» hinzugefügt. Er ist aus, bis du ihn unter Mods einschaltest.`;
  if (res?.kind === 'skill') return `Skill «/${res.skill?.slug || ''}» hinzugefügt.`;
  return 'Hinzugefügt.';
}

function CatalogSection({ state, go, kind }) {
  const [repo, setRepo] = useState('');
  const [link, setLink] = useState('');
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState(null);
  const [choose, setChoose] = useState(null);
  const catalogs = state.catalogs || [];
  const addCatalog = async () => {
    const r = await api().catalogSources([...catalogs.filter((c) => c.eigen).map((c) => c.repo), repo.trim()]);
    if (r?.ok === false) setMsg({ bad: true, text: r.reason });
    else { setRepo(''); setMsg(null); }
  };
  const removeCatalog = (name) => api().catalogSources(catalogs.filter((c) => c.eigen && c.repo !== name).map((c) => c.repo));
  const fromLink = async () => {
    setBusy(true); setMsg(null); setChoose(null);
    const r = await api().catalogLink(link.trim()).catch((e) => ({ ok: false, reason: String(e?.message || e) }));
    setBusy(false);
    if (!r?.ok) { if (!r?.canceled) setMsg({ bad: true, text: r?.reason || 'Nicht hinzugefügt.' }); return; }
    if (r.choose) { setChoose({ repo: r.repo, items: r.choose }); return; }
    setLink(''); setMsg({ text: installedText(r) });
  };
  const pick = async (it) => {
    const r = await api().catalogInstall(choose.repo, it.kind, it.path);
    setMsg(r?.ok ? { text: installedText(r) } : { bad: !r?.canceled, text: r?.reason || 'Abgebrochen.' });
  };
  return (
    <>
      <h3>Kataloge</h3>
      <p className="s-text">{kind === 'mod' ? 'Mods aus öffentlichen GitHub-Repositories.' : 'Skills aus öffentlichen GitHub-Repositories, z. B. von Anthropic (Englisch; ASK antwortet trotzdem auf Deutsch).'}</p>
      {catalogs.map((c) => (
        <Row key={c.repo} title={c.repo} text={c.eigen ? 'Von dir hinzugefügt' : 'Mitgeliefert'}>
          <button type="button" className="btn small" onClick={() => go('catalog', { repo: c.repo, kind })}>Durchsuchen …</button>
          {c.eigen ? <button type="button" className="icon-btn" title="Katalog entfernen" aria-label={`Katalog ${c.repo} entfernen`} onClick={() => removeCatalog(c.repo)}><Icon name="close" size={14} /></button> : null}
        </Row>
      ))}
      <div className="s-inline">
        <input className="input" placeholder="Weiterer Katalog: besitzer/repository" value={repo} onChange={(e) => setRepo(e.target.value)} aria-label="Katalog hinzufügen"
          onKeyDown={(e) => { if (e.key === 'Enter' && repo.trim()) addCatalog(); }} />
        <button type="button" className="btn" disabled={!repo.trim()} onClick={addCatalog}>Katalog hinzufügen</button>
      </div>
      <h3>Von Link hinzufügen</h3>
      <p className="s-text">Link auf einen Ordner mit SKILL.md (Skill) oder mod.json (Mod) auf GitHub.</p>
      <div className="s-inline">
        <input className="input" placeholder="https://github.com/anthropics/skills/tree/main/skills/frontend-design" value={link} onChange={(e) => setLink(e.target.value)} aria-label="Link"
          onKeyDown={(e) => { if (e.key === 'Enter' && link.trim() && !busy) fromLink(); }} />
        <button type="button" className="btn" disabled={!link.trim() || busy} onClick={fromLink}>{busy ? 'Lädt …' : 'Hinzufügen'}</button>
      </div>
      {choose ? (
        <div className="s-list">
          <p className="s-text">Unter dem Link liegen mehrere Einträge. Wähle aus:</p>
          {choose.items.map((it) => (
            <Row key={it.path} title={it.path || choose.repo} badge={it.kind === 'mod' ? 'Mod' : 'Skill'}>
              <button type="button" className="btn small" onClick={() => pick(it)}>Hinzufügen</button>
            </Row>
          ))}
        </div>
      ) : null}
      <Message msg={msg} />
    </>
  );
}

function SkillsPane({ state, go, onCompose }) {
  const list = useSkills();
  const own = (list || []).filter((s) => s.source !== 'standard');
  const std = (list || []).filter((s) => s.source === 'standard');
  const [msg, setMsg] = useState(null);
  const importIt = async () => {
    const r = await api().skillsImport();
    if (r?.ok === false) setMsg({ bad: true, text: r.reason });
    else if (r?.skill) setMsg({ text: `Skill «/${r.skill.slug}» importiert.` });
  };
  return (
    <div className="s-section">
      <p className="s-text lead">Skills sind Anleitungen für wiederkehrende Aufträge. ASK liest einen passenden Skill selbst; mit /name rufst du ihn direkt auf.</p>
      <div className="s-actions">
        <button type="button" className="btn primary" id="skills-new" onClick={() => go('skills', { slug: '' })}><Icon name="plus" size={14} /> Neuer Skill</button>
        <button type="button" className="btn" onClick={() => onCompose('/skill-erstellen ')}><Icon name="spark" size={14} /> Mit ASK erstellen</button>
        <button type="button" className="btn" id="skills-import" onClick={importIt}>Importieren …</button>
      </div>
      <Message msg={msg} />
      <h3>Eigene Skills</h3>
      {list === null ? <p className="muted">Lädt …</p> : !own.length ? <p className="muted small">Noch keine eigenen Skills.</p> : own.map((s) => (
        <Row key={`e-${s.slug}`} title={`/${s.slug}`} text={s.description}>
          <button type="button" className="btn small" onClick={() => go('skills', { slug: s.slug })}>Bearbeiten</button>
        </Row>
      ))}
      <h3>Mitgeliefert</h3>
      {std.map((s) => (
        <Row key={`s-${s.slug}`} title={`/${s.slug}`} text={s.description} badge={s.replaced ? 'durch eigenen ersetzt' : null}>
          <button type="button" className="btn small" onClick={() => go('skill-view', { slug: s.slug })}>Ansehen</button>
          <Toggle label={`Skill ${s.slug}`} checked={s.enabled} disabled={s.replaced} onChange={(v) => api().skillsEnable(s.slug, v)} />
        </Row>
      ))}
      <CatalogSection state={state} go={go} kind="skill" />
    </div>
  );
}

// ---------- Mods ----------

function modParts(m) {
  return [m.style ? 'Aussehen' : '', m.script ? 'Code' : '', m.befehle?.length ? 'Befehle' : '', m.anweisungen ? 'Anweisungen' : '', m.nachfragen?.length ? 'Regeln' : ''].filter(Boolean);
}

function useModsList() {
  const [data, setData] = useState(null);
  useEffect(() => {
    const load = () => api().modsList?.().then(setData).catch(() => setData({ mods: [] }));
    load();
    const off = api().onModsChanged?.(load);
    return () => { if (typeof off === 'function') off(); };
  }, []);
  return data;
}

function ModsPane({ state, go, onCompose }) {
  const data = useModsList();
  const [msg, setMsg] = useState(null);
  // Ein-/Ausschalten und Löschen laden die Oberfläche neu; danach geht es hier weiter
  const run = async (p, reload = false) => {
    if (reload) keepDialogOverReload({ name: 'settings', pane: 'mods' });
    const r = await (typeof p === 'function' ? p() : p).catch((e) => ({ ok: false, reason: String(e?.message || e) }));
    if (r?.ok === false && !r.canceled) setMsg({ bad: true, text: r.reason });
    else if (r?.kind) setMsg({ text: installedText(r) });
    else setMsg(null);
  };
  const mods = data?.mods || [];
  return (
    <div className="s-section">
      <p className="s-text lead">Mods ändern ASK nach deinem Geschmack: Aussehen, eigene /Befehle, Bereiche in der Seitenleiste, Figuren oder Regeln, bei denen ASK zuerst fragt.</p>
      {state.modsCode ? (
        <div className="s-note warn" role="status">
          <Icon name="shield" size={16} />
          <div><strong>Ein Mod mit Code ist eingeschaltet.</strong> Heikle Schritte (Befehle erlauben, Modus «Auto», Skills speichern …) bestätigst du jetzt in einem Fenster von Windows. Ctrl+Shift+M schaltet alle Mods sofort aus.</div>
          <button type="button" className="btn small" onClick={() => run(() => api().modsOffAll(), true)}>Alle ausschalten</button>
        </div>
      ) : (
        <div className="s-note"><Icon name="shield" size={16} /><div>Code in einem Mod sieht und bedient alles in ASK. Schalte nur Mods ein, deren Code du angesehen hast oder deren Quelle du vertraust. Neue und geänderte Mods sind aus.</div></div>
      )}
      <div className="s-actions">
        <button type="button" className="btn primary" onClick={() => onCompose('/mod-erstellen ')}><Icon name="spark" size={14} /> Mit ASK erstellen</button>
        <button type="button" className="btn" onClick={() => run(() => api().modsImport(), true)}>Aus Ordner importieren …</button>
        <button type="button" className="btn" onClick={() => api().modsFolder()}>Ordner öffnen</button>
      </div>
      <Message msg={msg} />
      <Row title="Anleitung für andere KI" text="Mit Claude, ChatGPT, Codex oder Copilot einen Mod bauen: Anleitung kopieren und in den Chat einfügen, oder als SKILL.md speichern (in Claude als Skill «ask-mods»). Den fertigen Ordner holst du mit «Aus Ordner importieren …».">
        <button type="button" className="btn small" onClick={async () => { const r = await api().modsGuide('kopieren'); setMsg(r?.ok ? { text: 'Anleitung kopiert. Füge sie im Chat der anderen KI ein und beschreibe deinen Mod.' } : { bad: true, text: r?.reason || 'Nicht kopiert.' }); }}>Kopieren</button>
        <button type="button" className="btn small" onClick={async () => { const r = await api().modsGuide('speichern'); if (r?.path) setMsg({ text: `Gespeichert: ${r.path}` }); else if (r?.ok === false && !r.canceled) setMsg({ bad: true, text: r.reason }); }}>Speichern …</button>
      </Row>
      <h3>Deine Mods</h3>
      {data === null ? <p className="muted">Lädt …</p> : !mods.length ? <p className="muted small">Noch keine Mods. Lass ASK einen erstellen oder hol einen aus einem Katalog (unten).</p> : mods.map((m) => (
        <Row key={m.id} title={<>{m.name} <span className="s-ver">{m.version}</span></>}
          text={<>{m.beschreibung}{m.geaendert ? <span className="s-warn"> Seit dem Einschalten geändert: erst nach erneuter Vorschau wieder aktiv.</span> : null}{m.fehler ? <span className="s-warn"> {m.fehler}</span> : null}
            <span className="s-tags">{modParts(m).map((t) => <span key={t} className={`s-tag${t === 'Code' ? ' code' : ''}`}>{t}</span>)}</span></>}>
          <button type="button" className="btn small" onClick={() => go('mod-preview', { id: m.id })}>Vorschau</button>
          <button type="button" className="icon-btn" title="Löschen" aria-label={`Mod ${m.name} löschen`} onClick={() => run(() => api().modsRemove(m.id), m.aktiv)}><Icon name="trash" size={14} /></button>
          <Toggle label={`Mod ${m.name}`} checked={m.aktiv} disabled={!!m.fehler}
            onChange={(v) => (v ? go('mod-preview', { id: m.id }) : run(() => api().modsEnable(m.id, false), true))} />
        </Row>
      ))}
      <CatalogSection state={state} go={go} kind="mod" />
    </div>
  );
}

// ---------- Untere Dialoge ----------

function SubDialog({ id, title, onClose, children, actions, wide = true }) {
  return (
    <div id={id} className={`dialog${wide ? ' wide' : ''}`} role="dialog" aria-modal="true" aria-labelledby={`${id}-title`}>
      <div className="dialog-head">
        <h1 id={`${id}-title`}>{title}</h1>
        <button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={16} /></button>
      </div>
      <div className="dialog-body">{children}</div>
      {actions ? <div className="actions">{actions}</div> : null}
    </div>
  );
}

// Mods und Skills deutlich getrennt: eigene Farbe und eigenes Symbol, Umschalter oben, Abschnitte mit Erklärung
const KINDS = {
  mod: { label: 'Mods', one: 'Mod', icon: 'puzzle', text: 'Verändern ASK selbst: Aussehen, Bereiche, Figuren, Knöpfe oder Befehle. Nach dem Hinzufügen aus, bis du sie unter Mods einschaltest.' },
  skill: { label: 'Skills', one: 'Skill', icon: 'bolt', text: 'Anleitungen, die ASK bei passenden Aufträgen liest. Direkt aufrufen mit /name im Eingabefeld.' },
};

function CatalogEntry({ e, repo, state, onAdd }) {
  const k = KINDS[e.kind] || KINDS.skill;
  return (
    <div className={`s-row cat-row kind-${e.kind}`}>
      <span className="cat-icon" aria-hidden="true"><Icon name={k.icon} size={16} /></span>
      <div className="s-row-text">
        <div className="s-title">{e.kind === 'skill' ? `/${e.name}` : e.name}<span className={`s-badge kind-${e.kind}`}>{k.one}</span></div>
        {e.description ? <div className="s-text">{e.description}</div> : null}
      </div>
      <div className="s-row-ctl">
        <button type="button" className="btn small ghost" onClick={() => api().openLink(`https://github.com/${repo}/tree/HEAD/${e.path}`)}>Ansehen</button>
        {e.installiert || state === 'ok' ? <span className="muted small cat-done"><Icon name="check" size={13} /> Hinzugefügt</span>
          : <button type="button" className="btn small" disabled={state === 'lädt'} onClick={() => onAdd(e)}>{state === 'lädt' ? 'Lädt …' : 'Hinzufügen'}</button>}
      </div>
    </div>
  );
}

export function CatalogDialog({ repo, kind, onClose }) {
  const [res, setRes] = useState(null);
  const [done, setDone] = useState({});
  const [msg, setMsg] = useState(null);
  const [filter, setFilter] = useState('');
  const [show, setShow] = useState(null); // 'skill' | 'mod' | 'alle'
  useEffect(() => { api().catalogList(repo).then(setRes).catch((e) => setRes({ ok: false, reason: String(e?.message || e) })); }, [repo]);
  const add = async (e) => {
    setDone((d) => ({ ...d, [e.path]: 'lädt' }));
    const r = await api().catalogInstall(repo, e.kind, e.path).catch((err) => ({ ok: false, reason: String(err?.message || err) }));
    setDone((d) => ({ ...d, [e.path]: r?.ok ? 'ok' : '' }));
    setMsg(r?.ok ? { text: installedText(r) } : r?.canceled ? null : { bad: true, text: r?.reason || 'Nicht hinzugefügt.' });
  };
  const all = res?.entries || [];
  const count = (k) => all.filter((e) => e.kind === k).length;
  // Zuerst die Art, von der aus der Katalog geöffnet wurde; gibt es davon keine, alle
  const current = show || (kind && count(kind) ? kind : 'alle');
  const q = filter.trim().toLowerCase();
  const matching = all.filter((e) => `${e.name} ${e.description}`.toLowerCase().includes(q));
  const kinds = current === 'alle' ? ['mod', 'skill'] : [current];
  return (
    <SubDialog id="dlg-catalog" title={`Katalog ${repo}`} onClose={onClose} actions={<button type="button" className="btn primary" onClick={onClose}>Fertig</button>}>
      <p className="s-text">Einträge aus <button type="button" className="linkish" onClick={() => api().openLink(`https://github.com/${repo}`)}>github.com/{repo}</button>. Sieh dir einen Eintrag an, bevor du ihn hinzufügst.</p>
      {!res ? <p className="muted">Lädt …</p> : res.ok === false ? <p className="s-msg bad" role="alert">{res.reason}</p> : (
        <>
          <div className="cat-tools">
            <Segmented name="Art" value={current} onChange={setShow}
              options={[['skill', `Skills (${count('skill')})`], ['mod', `Mods (${count('mod')})`], ['alle', `Alle (${all.length})`]]} />
            <input className="input" type="search" placeholder="Filtern" value={filter} onChange={(e) => setFilter(e.target.value)} aria-label="Einträge filtern" />
          </div>
          {kinds.map((k) => {
            const list = matching.filter((e) => e.kind === k);
            if (current === 'alle' && !list.length) return null;
            return (
              <section key={k} className={`cat-section kind-${k}`}>
                <h3><Icon name={KINDS[k].icon} size={14} /> {KINDS[k].label} <span className="cat-count">{list.length}</span></h3>
                <p className="s-text cat-lead">{KINDS[k].text}</p>
                <div className="s-list">
                  {!list.length ? <p className="muted small">{q ? 'Nichts gefunden.' : `Keine ${KINDS[k].label} in diesem Katalog.`}</p>
                    : list.map((e) => <CatalogEntry key={e.path} e={e} repo={repo} state={done[e.path]} onAdd={add} />)}
                </div>
              </section>
            );
          })}
          {current === 'alle' && !matching.length ? <p className="muted small">Nichts gefunden.</p> : null}
        </>
      )}
      <Message msg={msg} />
    </SubDialog>
  );
}

const ASK_KIND_TEXT = { loeschen: 'vor dem Löschen von Dateien', schreiben: 'vor dem Schreiben von Dateien', befehle: 'vor jedem Befehl', web: 'vor Zugriffen aufs Internet' };

export function ModPreviewDialog({ id, onClose }) {
  const [m, setM] = useState(null);
  const [file, setFile] = useState('');
  const [trust, setTrust] = useState(false);
  const [msg, setMsg] = useState(null);
  useEffect(() => {
    api().modsGet(id).then((x) => {
      setM(x || false);
      if (x?.files?.length) setFile((x.files.find((f) => f.path === x.script) || x.files.find((f) => f.text) || x.files[0]).path);
    });
  }, [id]);
  if (m === null) return <SubDialog id="dlg-mod" title="Mod" onClose={onClose}><p className="muted">Lädt …</p></SubDialog>;
  if (!m) return <SubDialog id="dlg-mod" title="Mod" onClose={onClose}><p className="muted">Den Mod gibt es nicht mehr.</p></SubDialog>;
  const shown = m.files.find((f) => f.path === file);
  const enable = async () => {
    keepDialogOverReload({ name: 'settings', pane: 'mods' });
    const r = await api().modsEnable(m.id, true).catch((e) => ({ ok: false, reason: String(e?.message || e) }));
    if (r?.ok) onClose();
    else setMsg({ bad: true, text: r?.reason || 'Nicht eingeschaltet.' });
  };
  return (
    <SubDialog id="dlg-mod" title={`${m.name}${m.version ? ` ${m.version}` : ''}`} onClose={onClose} actions={(
      <>
        {m.script && !m.aktiv ? <label className="check"><input type="checkbox" checked={trust} onChange={(e) => setTrust(e.target.checked)} /> Ich habe den Code angesehen und vertraue der Quelle</label> : null}
        <span className="spacer" />
        <button type="button" className="btn" onClick={onClose}>{m.aktiv ? 'Schliessen' : 'Abbrechen'}</button>
        {m.aktiv ? <button type="button" className="btn" onClick={async () => { keepDialogOverReload({ name: 'settings', pane: 'mods' }); await api().modsEnable(m.id, false); onClose(); }}>Ausschalten</button>
          : <button type="button" className="btn primary" disabled={(!!m.script && !trust) || !!m.fehler} onClick={enable}>Einschalten</button>}
      </>
    )}>
      <p className="s-text">{m.beschreibung}{m.autor ? ` · von ${m.autor}` : ''}</p>
      {m.fehler ? <p className="s-msg bad">{m.fehler}</p> : null}
      <ul className="mod-rights">
        {m.style ? <li><Icon name="palette" size={15} /> Ändert das Aussehen von ASK ({m.style}).</li> : null}
        {m.script ? <li className="code"><Icon name="alert" size={15} /> Führt Code in ASK aus ({m.script}): sieht und bedient alles in der Oberfläche, auch deine Chats. Heikle Schritte bestätigst du dann in einem Fenster von Windows.</li> : null}
        {m.befehle?.length ? <li><Icon name="term" size={15} /> Befehle: {m.befehle.map((b) => `/${b.name}`).join(', ')}</li> : null}
        {m.anweisungen ? <li><Icon name="chat" size={15} /> Zusätzliche Anweisung an ASK: «{m.anweisungen}»</li> : null}
        {m.nachfragen?.length ? <li><Icon name="hand" size={15} /> ASK fragt {m.nachfragen.map((k) => ASK_KIND_TEXT[k] || k).join(', ')}.</li> : null}
      </ul>
      <div className="mod-files">
        <div className="mod-file-tabs" role="tablist" aria-label="Dateien">
          {m.files.map((f) => (
            <button key={f.path} type="button" role="tab" aria-selected={f.path === file} className={f.path === file ? 'on' : ''} onClick={() => setFile(f.path)}>
              {f.path} <span className="muted">{formatSize(f.size)}</span>
            </button>
          ))}
        </div>
        {shown ? (shown.text ? <pre className="mod-code"><code>{shown.text}</code></pre> : <p className="muted small">Keine Textdatei ({formatSize(shown.size)}).</p>) : null}
      </div>
      <Message msg={msg} />
    </SubDialog>
  );
}

export function SkillViewDialog({ slug, onClose, onCopy }) {
  const [s, setS] = useState(null);
  useEffect(() => { api().skillsGet(slug, 'standard').then((x) => setS(x || false)); }, [slug]);
  return (
    <SubDialog id="dlg-skill-view" title={`/${slug}`} onClose={onClose} actions={(
      <>
        <span className="muted small">Mitgeliefert. Für eine eigene Fassung: kopieren und als neuen Skill speichern.</span>
        <span className="spacer" />
        {s ? <button type="button" className="btn" onClick={() => onCopy(s)}>Als eigenen Skill kopieren</button> : null}
        <button type="button" className="btn primary" onClick={onClose}>Schliessen</button>
      </>
    )}>
      {s === null ? <p className="muted">Lädt …</p> : !s ? <p className="muted">Nicht gefunden.</p> : (
        <>
          <p className="s-text">{s.description}</p>
          <pre className="mod-code skill-body">{s.body}</pre>
        </>
      )}
    </SubDialog>
  );
}

export const SHORTCUTS = [
  ['Chats', [['Ctrl+N', 'Neuer Chat'], ['Ctrl+W', 'Tab schliessen'], ['Ctrl+Tab', 'Nächster Tab'], ['Ctrl+Shift+Tab', 'Vorheriger Tab'], ['Ctrl+1 … 9', 'Zu Tab springen'], ['Ctrl+Shift+T', 'Geschlossenen Tab wieder öffnen']]],
  ['Eingabe', [['Enter', 'Senden (während eines Auftrags: einreihen)'], ['Shift+Enter', 'Neue Zeile'], ['/', 'Befehle und Skills'], ['@', 'Datei erwähnen'], ['Esc', 'Auftrag stoppen']]],
  ['Ansicht', [['Ctrl+,', 'Einstellungen'], ['Ctrl+O', 'Details zeigen'], ['Ctrl+T', 'Aufgabenliste ein- und ausklappen'], ['Ctrl +', 'Grösser'], ['Ctrl −', 'Kleiner'], ['Ctrl+0', 'Normale Grösse']]],
  ['Mods und Fehler', [['Ctrl+Shift+M', 'Alle Mods ausschalten und neu laden'], ['Ctrl+R', 'ASK-Seite neu laden'], ['Ctrl+Shift+J', 'Entwicklertools der Oberfläche']]],
];

export function ShortcutsDialog({ onClose }) {
  return (
    <SubDialog id="dlg-shortcuts" title="Tastenkombinationen" onClose={onClose} actions={<button type="button" className="btn primary" onClick={onClose}>Fertig</button>}>
      <div className="shortcuts">
        {SHORTCUTS.map(([group, list]) => (
          <section key={group}>
            <h3>{group}</h3>
            <dl>{list.map(([k, t]) => <div key={k} className="sc-row"><dt>{k.split(/\+(?=.)/).map((p, i) => <kbd key={i}>{p.trim()}</kbd>)}</dt><dd>{t}</dd></div>)}</dl>
          </section>
        ))}
      </div>
    </SubDialog>
  );
}
