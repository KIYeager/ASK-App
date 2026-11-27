import { useEffect, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import { HELP_TARGETS, HELP_TOPICS, searchHelp } from '../lib/helpTopics.mjs';

// Fenster «Hilfe» (ENTSCHEIDE E79): links Suche und Themen, rechts Fragen mit kurzen Antworten.
// Die Übersicht führt zur Einführung, zu den Tastenkombinationen und zu einer Frage an ASK im Chat.
function HelpItem({ item, open, onOpen }) {
  return (
    <details className="help-item" open={open || undefined}>
      <summary><span>{item.q}</span><Icon name="chevDown" size={14} className="help-chev" /></summary>
      <div className="help-answer">
        <p>{item.a}</p>
        {item.open && HELP_TARGETS[item.open] ? (
          <button type="button" className="btn small" onClick={() => onOpen(item.open)}>
            {HELP_TARGETS[item.open]} <Icon name="arrowRight" size={13} />
          </button>
        ) : null}
      </div>
    </details>
  );
}

export default function HelpDialog({ pane = 'start', releasesUrl, onPane, onOpen, onAsk, onLink, onClose }) {
  const [query, setQuery] = useState('');
  const search = useRef(null);
  useEffect(() => { search.current?.focus(); }, []);
  const q = query.trim();
  const topic = HELP_TOPICS.find((t) => t.id === pane);
  const current = topic ? pane : 'start';
  const hits = q ? searchHelp(q) : [];
  const show = (id) => { setQuery(''); onPane(id); };
  const cards = [
    { icon: 'map', title: 'Einführung', text: 'In vier Karten zeigen, wo was ist.', run: () => onOpen('tour') },
    { icon: 'keyboard', title: 'Tastenkombinationen', text: 'Alle Kürzel auf einen Blick.', run: () => onOpen('shortcuts') },
    { icon: 'chat', title: 'ASK fragen', text: 'Deine Frage zur App im Chat stellen.', run: () => onAsk('') },
    releasesUrl ? { icon: 'gift', title: 'Neu in ASK', text: 'Was sich in den letzten Versionen geändert hat.', run: () => onLink(releasesUrl) } : null,
  ].filter(Boolean);
  return (
    <div id="dlg-help" className="dialog settings-dialog help-dialog" role="dialog" aria-modal="true" aria-labelledby="dlg-help-title">
      <div className="dialog-head">
        <h1 id="dlg-help-title">Hilfe</h1>
        <button type="button" className="icon-btn" aria-label="Schliessen" onClick={onClose}><Icon name="close" size={16} /></button>
      </div>
      <div className="settings-body">
        <nav className="settings-nav" aria-label="Themen">
          <label className="settings-search">
            <Icon name="search" size={15} />
            <input id="help-search" ref={search} type="search" placeholder="In der Hilfe suchen" value={query} aria-label="Hilfe durchsuchen"
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => { if (e.key === 'Escape' && query) { e.stopPropagation(); setQuery(''); } }} />
          </label>
          <div className="settings-group">
            <button type="button" className={!q && current === 'start' ? 'on' : ''} aria-current={!q && current === 'start' ? 'page' : undefined}
              onClick={() => show('start')}><Icon name="help" size={16} /> Übersicht</button>
          </div>
          <div className="settings-group">
            {HELP_TOPICS.map((t) => (
              <button key={t.id} type="button" className={!q && current === t.id ? 'on' : ''} aria-current={!q && current === t.id ? 'page' : undefined}
                onClick={() => show(t.id)}><Icon name={t.icon} size={16} /> {t.title}</button>
            ))}
          </div>
          <span className="spacer" />
          <button type="button" className="settings-foot" onClick={() => onOpen('shortcuts')}>Tastenkombinationen <Icon name="arrowRight" size={13} /></button>
        </nav>
        <section className="settings-pane help-pane" aria-live="polite">
          {q ? (
            <>
              <h2>Suche</h2>
              {hits.map((t) => (
                <div key={t.id} className="help-group">
                  <h3>{t.title}</h3>
                  {t.items.map((it) => <HelpItem key={it.q} item={it} open={hits.length === 1 && t.items.length <= 2} onOpen={onOpen} />)}
                </div>
              ))}
              {!hits.length ? <p className="muted">Dazu steht hier nichts.</p> : null}
              <div className="help-ask">
                <span>{hits.length ? 'Nicht das Richtige dabei?' : 'ASK kennt die App und kann dir weiterhelfen.'}</span>
                <button id="help-ask" type="button" className="btn small" onClick={() => onAsk(q)}>
                  <Icon name="chat" size={14} /> ASK fragen: «{q.length > 40 ? `${q.slice(0, 40)}…` : q}»
                </button>
              </div>
            </>
          ) : current === 'start' ? (
            <>
              <h2>Hilfe zu ASK</h2>
              <p className="muted help-intro">Such nach einem Stichwort oder wähle ein Thema. Findest du nichts, frag ASK direkt im Chat oder deine Lehrperson.</p>
              <div className="help-cards">
                {cards.map((c) => (
                  <button key={c.title} type="button" className="help-card" onClick={c.run}>
                    <Icon name={c.icon} size={18} />
                    <span className="s-title">{c.title}</span>
                    <span className="muted">{c.text}</span>
                  </button>
                ))}
              </div>
              <h3>Themen</h3>
              <div className="help-topics">
                {HELP_TOPICS.map((t) => (
                  <button key={t.id} type="button" className="help-topic" onClick={() => show(t.id)}>
                    <Icon name={t.icon} size={16} />
                    <span className="help-topic-text">
                      <span className="s-title">{t.title}</span>
                      <span className="muted">{t.items.slice(0, 2).map((it) => it.q).join(' · ')}</span>
                    </span>
                    <Icon name="arrowRight" size={13} />
                  </button>
                ))}
              </div>
            </>
          ) : (
            <>
              <h2>{topic.title}</h2>
              {topic.items.map((it) => <HelpItem key={it.q} item={it} onOpen={onOpen} />)}
            </>
          )}
        </section>
      </div>
    </div>
  );
}
