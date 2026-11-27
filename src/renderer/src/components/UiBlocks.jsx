import { useEffect, useMemo, useRef, useState } from 'react';
import Icon from './Icon.jsx';
import Markdown from './Markdown.jsx';
import { api } from '../api.js';
import { asCode, checkHtml, evalFormula, formatNumber, parseUi, splitUi } from '../lib/uiBlocks.mjs';

// Interaktive Antworten (ENTSCHEIDE E80): Antworttext mit Bausteinen ```ask-ui (feste Bausteine) und ```ask-html
// (eigenes HTML im abgeschotteten iframe). Ausgeschaltet oder fehlerhaft erscheint der Block als gewöhnlicher Code.

export default function AnswerText({ text, streaming, ctx }) {
  const md = (t, key) => (t.trim() ? <Markdown key={key} text={t} isFile={ctx.isFile} onFile={ctx.openFileByName} onLink={ctx.openLink} onCopy={ctx.copy} /> : null);
  const parts = useMemo(() => splitUi(text), [text]);
  if (!ctx.state?.interactiveUi || !parts.some((p) => p.k !== 'md')) return md(String(text || ''), 'all');
  return (
    <>
      {parts.map((p, i) => {
        if (p.k === 'md') return md(p.text, i);
        if (!p.done) return streaming ? <div key={i} className="ui-pending"><span className="spin small" aria-hidden="true" /> Baut eine interaktive Ansicht …</div> : md(asCode(p), i);
        return p.k === 'ui' ? <UiBlock key={i} part={p} ctx={ctx} /> : <HtmlBlock key={i} part={p} ctx={ctx} />;
      })}
    </>
  );
}

function Broken({ part, error, ctx }) {
  return (
    <div className="ui-broken">
      <div className="ui-broken-note"><Icon name="alert" size={14} /> Baustein konnte nicht angezeigt werden: {error}</div>
      <Markdown text={asCode(part)} onLink={ctx.openLink} onCopy={ctx.copy} />
    </div>
  );
}

function UiBlock({ part, ctx }) {
  const parsed = useMemo(() => parseUi(part.src), [part.src]);
  if (!parsed.ok) return <Broken part={part} error={parsed.error} ctx={ctx} />;
  return (
    <div className="ui-blocks">
      {parsed.blocks.map((b, i) => {
        const C = COMPONENTS[b.typ];
        return (
          <section key={i} className={`ui-block ui-${b.typ}`} aria-label={b.titel || undefined}>
            {b.titel && b.typ !== 'hinweis' ? <h4 className="ui-title">{b.titel}</h4> : null}
            <C b={b} ctx={ctx} />
          </section>
        );
      })}
    </div>
  );
}

// ---------- Einfache Bausteine ----------

const NOTE_ICONS = { info: 'info', tipp: 'spark', warnung: 'alert', erfolg: 'check' };

function Hinweis({ b }) {
  return (
    <div className={`ui-note ${b.art}`}>
      <Icon name={NOTE_ICONS[b.art]} size={16} />
      <div>{b.titel ? <strong>{b.titel}</strong> : null}<p>{b.text}</p></div>
    </div>
  );
}

function Schritte({ b }) {
  const [done, setDone] = useState(() => new Set());
  const toggle = (i) => setDone((d) => { const n = new Set(d); if (n.has(i)) n.delete(i); else n.add(i); return n; });
  return (
    <>
      <div className="ui-progress" aria-hidden="true"><span style={{ width: `${(done.size / b.schritte.length) * 100}%` }} /></div>
      <ol className="ui-steps">
        {b.schritte.map((s, i) => (
          <li key={i} className={done.has(i) ? 'done' : ''}>
            <label>
              <input type="checkbox" checked={done.has(i)} onChange={() => toggle(i)} />
              <span className="ui-step-num">{i + 1}</span>
              <span><strong>{s.titel}</strong>{s.text ? <span className="ui-step-text">{s.text}</span> : null}</span>
            </label>
          </li>
        ))}
      </ol>
      <div className="ui-foot muted">{done.size} von {b.schritte.length} erledigt</div>
    </>
  );
}

function Vergleich({ b }) {
  return (
    <div className="ui-compare" style={{ '--cols': b.spalten.length }}>
      {b.spalten.map((c, i) => (
        <div key={i} className={`ui-col${c.hervorheben ? ' hi' : ''}`}>
          <div className="ui-col-head"><strong>{c.titel}</strong>{c.untertitel ? <span className="muted">{c.untertitel}</span> : null}</div>
          <ul>{c.punkte.map((p, j) => <li key={j}>{p}</li>)}</ul>
        </div>
      ))}
    </div>
  );
}

// ---------- Diagramm (SVG, Farben aus der App) ----------

const COLORS = 6;
const W = 600;
const H = 260;
const PAD = { l: 48, r: 12, t: 14, b: 42 };

// Schöne Achsenteilung: 1, 2 oder 5 mal Zehnerpotenz
function ticks(lo, hi) {
  const span = hi - lo || 1;
  const raw = span / 4;
  const mag = 10 ** Math.floor(Math.log10(raw));
  const step = [1, 2, 5, 10].map((m) => m * mag).find((s) => s >= raw) || raw;
  const start = Math.floor(lo / step) * step;
  const end = Math.ceil(hi / step) * step;
  const out = [];
  for (let v = start; v <= end + step / 2; v += step) out.push(Number(v.toFixed(10)));
  return out;
}
const short = (s, n = 12) => (s.length > n ? `${s.slice(0, n - 1)}…` : s);

function Diagramm({ b }) {
  const [hidden, setHidden] = useState(() => new Set());
  const unit = b.einheit ? ` ${b.einheit}` : '';
  if (b.art === 'kreis') return <Kreis b={b} unit={unit} />;
  const series = b.reihen.map((r, i) => ({ ...r, i })).filter((r) => !hidden.has(r.i));
  const all = series.flatMap((r) => r.werte);
  const t = ticks(Math.min(0, ...all, 0), Math.max(0, ...all, 1));
  const lo = t[0];
  const hi = t[t.length - 1];
  const y = (v) => PAD.t + (1 - (v - lo) / (hi - lo || 1)) * (H - PAD.t - PAD.b);
  const n = b.beschriftungen.length;
  const slot = (W - PAD.l - PAD.r) / n;
  const x = (i) => PAD.l + slot * (i + 0.5);
  const fmt = (v) => `${formatNumber(v, 2)}${unit}`;
  const showValues = b.art === 'balken' && n * Math.max(series.length, 1) <= 12;
  return (
    <>
      <svg className="ui-chart" viewBox={`0 0 ${W} ${H}`} role="img" aria-label={b.titel || 'Diagramm'}>
        {t.map((v) => (
          <g key={v}>
            <line x1={PAD.l} x2={W - PAD.r} y1={y(v)} y2={y(v)} className={v === 0 ? 'axis' : 'grid'} />
            <text x={PAD.l - 6} y={y(v) + 4} className="tick" textAnchor="end">{formatNumber(v, 2)}</text>
          </g>
        ))}
        {b.beschriftungen.map((l, i) => (
          <text key={i} x={x(i)} y={H - PAD.b + 16} className="tick" textAnchor="middle">
            <title>{l}</title>{short(l, Math.max(4, Math.floor(slot / 7)))}
          </text>
        ))}
        {b.art === 'balken' ? series.map((r, si) => {
          const bw = Math.min(42, (slot * 0.75) / series.length);
          return r.werte.map((v, i) => {
            const bx = x(i) - (bw * series.length) / 2 + si * bw;
            const top = Math.min(y(v), y(0));
            const h = Math.max(1, Math.abs(y(v) - y(0)));
            return (
              <g key={`${r.i}-${i}`}>
                <rect x={bx + 1} y={top} width={Math.max(2, bw - 2)} height={h} rx="3" className={`c${r.i % COLORS}`}>
                  <title>{`${b.beschriftungen[i]}${r.name ? ` · ${r.name}` : ''}: ${fmt(v)}`}</title>
                </rect>
                {showValues ? <text x={bx + bw / 2} y={v >= 0 ? top - 4 : top + h + 12} className="val" textAnchor="middle">{formatNumber(v, 2)}</text> : null}
              </g>
            );
          });
        }) : series.map((r) => (
          <g key={r.i} className={`line c${r.i % COLORS}`}>
            <polyline points={r.werte.map((v, i) => `${x(i)},${y(v)}`).join(' ')} />
            {r.werte.map((v, i) => <circle key={i} cx={x(i)} cy={y(v)} r="3.5"><title>{`${b.beschriftungen[i]}${r.name ? ` · ${r.name}` : ''}: ${fmt(v)}`}</title></circle>)}
          </g>
        ))}
      </svg>
      {b.reihen.length > 1 || b.reihen[0].name ? (
        <div className="ui-legend">
          {b.reihen.map((r, i) => (
            <button key={i} type="button" className={hidden.has(i) ? 'off' : ''} aria-pressed={!hidden.has(i)} title="Ein- oder ausblenden"
              onClick={() => setHidden((h) => { const s = new Set(h); if (s.has(i)) s.delete(i); else if (s.size < b.reihen.length - 1) s.add(i); return s; })}>
              <span className={`dot c${i % COLORS}`} />{r.name || `Reihe ${i + 1}`}
            </button>
          ))}
        </div>
      ) : null}
    </>
  );
}

function Kreis({ b, unit }) {
  const values = b.reihen[0].werte;
  const total = values.reduce((a, v) => a + v, 0) || 1;
  const R = 90;
  const r = 52;
  const C = 110;
  let angle = -Math.PI / 2;
  const arcs = values.map((v, i) => {
    const a0 = angle;
    const a1 = angle + (v / total) * Math.PI * 2;
    angle = a1;
    const large = a1 - a0 > Math.PI ? 1 : 0;
    const p = (rad, a) => `${C + rad * Math.cos(a)},${C + rad * Math.sin(a)}`;
    // Ein voller Kreis (ein einziger Wert) braucht zwei Bögen
    const d = v / total >= 0.9999
      ? `M${p(R, 0)} A${R},${R} 0 1 1 ${p(R, Math.PI)} A${R},${R} 0 1 1 ${p(R, 0)} M${p(r, 0)} A${r},${r} 0 1 0 ${p(r, Math.PI)} A${r},${r} 0 1 0 ${p(r, 0)}Z`
      : `M${p(R, a0)} A${R},${R} 0 ${large} 1 ${p(R, a1)} L${p(r, a1)} A${r},${r} 0 ${large} 0 ${p(r, a0)}Z`;
    return { d, v, i };
  });
  return (
    <div className="ui-pie">
      <svg viewBox="0 0 220 220" role="img" aria-label={b.titel || 'Kreisdiagramm'}>
        {arcs.map((a) => a.v > 0 ? <path key={a.i} d={a.d} className={`c${a.i % COLORS}`} fillRule="evenodd"><title>{`${b.beschriftungen[a.i]}: ${formatNumber(a.v, 2)}${unit} (${formatNumber((a.v / total) * 100, 1)} %)`}</title></path> : null)}
        <text x={C} y={C + 5} textAnchor="middle" className="pie-total">{formatNumber(total, 2)}{unit}</text>
      </svg>
      <ul className="ui-pie-legend">
        {values.map((v, i) => (
          <li key={i}><span className={`dot c${i % COLORS}`} /><span>{b.beschriftungen[i]}</span><span className="muted">{formatNumber((v / total) * 100, 1)} %</span></li>
        ))}
      </ul>
    </div>
  );
}

// ---------- Üben: Quiz und Karteikarten ----------

function Quiz({ b }) {
  const [picked, setPicked] = useState(() => b.fragen.map(() => -1));
  const answered = picked.filter((p) => p >= 0).length;
  const right = picked.filter((p, i) => p === b.fragen[i].richtig).length;
  return (
    <>
      {b.fragen.map((f, qi) => {
        const p = picked[qi];
        return (
          <div key={qi} className="ui-q">
            <p className="ui-q-text"><span className="muted">{qi + 1}.</span> {f.frage}</p>
            <div className="ui-q-answers" role="group" aria-label={`Frage ${qi + 1}`}>
              {f.antworten.map((a, ai) => {
                const state = p < 0 ? '' : ai === f.richtig ? ' right' : ai === p ? ' wrong' : ' dim';
                return (
                  <button key={ai} type="button" className={`ui-q-answer${state}`} disabled={p >= 0}
                    onClick={() => setPicked((all) => all.map((x, i) => (i === qi ? ai : x)))}>
                    {p >= 0 && ai === f.richtig ? <Icon name="check" size={14} /> : null}{a}
                  </button>
                );
              })}
            </div>
            {p >= 0 ? (
              <p className={`ui-q-feedback ${p === f.richtig ? 'right' : 'wrong'}`}>
                <strong>{p === f.richtig ? 'Richtig.' : 'Nicht ganz.'}</strong> {f.erklaerung}
              </p>
            ) : null}
          </div>
        );
      })}
      <div className="ui-foot">
        <span className="muted">{answered < b.fragen.length ? `${answered} von ${b.fragen.length} beantwortet` : `Ergebnis: ${right} von ${b.fragen.length} richtig`}</span>
        {answered ? <button type="button" className="btn small" onClick={() => setPicked(b.fragen.map(() => -1))}><Icon name="reload" size={13} /> Nochmals</button> : null}
      </div>
    </>
  );
}

function Karteikarten({ b }) {
  const [i, setI] = useState(0);
  const [back, setBack] = useState(false);
  const k = b.karten[i];
  const go = (d) => { setI((i + d + b.karten.length) % b.karten.length); setBack(false); };
  return (
    <>
      <button type="button" className={`ui-card${back ? ' back' : ''}`} onClick={() => setBack(!back)} aria-label={back ? 'Vorderseite zeigen' : 'Rückseite zeigen'}>
        <span className="ui-card-side muted">{back ? 'Rückseite' : 'Vorderseite'}</span>
        <span className="ui-card-text">{back ? k.hinten : k.vorne}</span>
        <span className="ui-card-hint muted">{back ? '' : 'Klicken zum Umdrehen'}</span>
      </button>
      <div className="ui-foot">
        <button type="button" className="btn small" onClick={() => go(-1)} disabled={b.karten.length < 2}><Icon name="arrowLeft" size={13} /> Zurück</button>
        <span className="muted">Karte {i + 1} von {b.karten.length}</span>
        <button type="button" className="btn small" onClick={() => go(1)} disabled={b.karten.length < 2}>Weiter <Icon name="arrowRight" size={13} /></button>
      </div>
    </>
  );
}

// ---------- Rechner: Regler und Zahlen, Ergebnisse aus Formeln ----------

function Rechner({ b }) {
  const [vals, setVals] = useState(() => Object.fromEntries(b.eingaben.map((e) => [e.name, e.wert])));
  const results = b.ergebnisse.map((r) => {
    try { return evalFormula(r.formel, vals); } catch { return NaN; }
  });
  return (
    <div className="ui-calc">
      <div className="ui-calc-in">
        {b.eingaben.map((e) => (
          <label key={e.name} className="ui-field">
            <span className="ui-field-label">{e.label || e.name}</span>
            <span className="ui-calc-row">
              <input type="range" min={e.min} max={e.max} step={e.schritt} value={vals[e.name]} aria-label={e.label || e.name}
                onChange={(ev) => setVals({ ...vals, [e.name]: Number(ev.target.value) })} />
              <input type="number" className="input" step={e.schritt} value={vals[e.name]} aria-label={`${e.label || e.name} als Zahl`}
                onChange={(ev) => setVals({ ...vals, [e.name]: ev.target.value === '' ? 0 : Number(ev.target.value) })} />
              <span className="muted ui-unit">{e.einheit}</span>
            </span>
          </label>
        ))}
      </div>
      <div className="ui-calc-out" aria-live="polite">
        {b.ergebnisse.map((r, i) => (
          <div key={i} className="ui-result">
            <span className="muted">{r.label}</span>
            <strong>{formatNumber(results[i], r.stellen)}{r.einheit ? ` ${r.einheit}` : ''}</strong>
          </div>
        ))}
      </div>
    </div>
  );
}

// ---------- Zurück in den Chat: Auswahl und Formular ----------

function Auswahl({ b, ctx }) {
  const [sent, setSent] = useState(null);
  const [checked, setChecked] = useState(() => new Set());
  const send = (text) => { setSent(text); ctx.uiSend?.(text); };
  return (
    <>
      {b.frage ? <p className="ui-q-text">{b.frage}</p> : null}
      {b.mehrfach ? (
        <>
          <div className="ui-checks">
            {b.optionen.map((o) => (
              <label key={o} className="ui-check">
                <input type="checkbox" disabled={!!sent} checked={checked.has(o)}
                  onChange={() => setChecked((c) => { const n = new Set(c); if (n.has(o)) n.delete(o); else n.add(o); return n; })} /> {o}
              </label>
            ))}
          </div>
          <div className="ui-foot">
            <button type="button" className="btn primary small" disabled={!!sent || !checked.size}
              onClick={() => send(b.optionen.filter((o) => checked.has(o)).join(', '))}>Auswahl senden</button>
          </div>
        </>
      ) : (
        <div className="ui-options">
          {b.optionen.map((o) => (
            <button key={o} type="button" className={`ui-option${sent === o ? ' on' : ''}`} disabled={!!sent} onClick={() => send(o)}>{o}</button>
          ))}
        </div>
      )}
      {sent ? <div className="ui-foot muted"><Icon name="check" size={13} /> Gesendet</div> : null}
    </>
  );
}

function Formular({ b, ctx }) {
  const [vals, setVals] = useState(() => Object.fromEntries(b.felder.map((f) => [f.name, ''])));
  const [sent, setSent] = useState(false);
  const missing = b.felder.some((f) => f.pflicht && !String(vals[f.name]).trim());
  const submit = (e) => {
    e.preventDefault();
    if (missing || sent) return;
    const lines = b.felder.filter((f) => String(vals[f.name]).trim()).map((f) => `- ${f.label}: ${String(vals[f.name]).trim()}`);
    setSent(true);
    ctx.uiSend?.(`${b.titel || 'Formular'}\n${lines.join('\n')}`);
  };
  const set = (name, v) => setVals({ ...vals, [name]: v });
  return (
    <form className="ui-form" onSubmit={submit}>
      {b.felder.map((f) => (
        <label key={f.name} className="ui-field">
          <span className="ui-field-label">{f.label}{f.pflicht ? <span className="muted"> (nötig)</span> : null}</span>
          {f.art === 'auswahl' ? (
            <select className="input" value={vals[f.name]} disabled={sent} onChange={(e) => set(f.name, e.target.value)}>
              <option value="">Bitte wählen</option>
              {f.optionen.map((o) => <option key={o} value={o}>{o}</option>)}
            </select>
          ) : f.art === 'mehrzeilig' ? (
            <textarea className="input" rows={3} value={vals[f.name]} placeholder={f.platzhalter} disabled={sent} onChange={(e) => set(f.name, e.target.value)} />
          ) : (
            <input className="input" type={f.art === 'zahl' ? 'number' : 'text'} value={vals[f.name]} placeholder={f.platzhalter} disabled={sent}
              onChange={(e) => set(f.name, e.target.value)} />
          )}
        </label>
      ))}
      <div className="ui-foot">
        <button type="submit" className="btn primary small" disabled={sent || missing}>{b.senden}</button>
        {sent ? <span className="muted"><Icon name="check" size={13} /> Gesendet</span> : null}
      </div>
    </form>
  );
}

const COMPONENTS = { hinweis: Hinweis, schritte: Schritte, vergleich: Vergleich, diagramm: Diagramm, quiz: Quiz,
  karteikarten: Karteikarten, rechner: Rechner, auswahl: Auswahl, formular: Formular };

// ---------- Eigenes HTML: abgeschottetes iframe über das Protokoll askui ----------

const MIN_H = 60;
const MAX_H = 900;

function HtmlBlock({ part, ctx }) {
  const checked = useMemo(() => checkHtml(part.src), [part.src]);
  const [url, setUrl] = useState(null);
  const [failed, setFailed] = useState(null);
  const [height, setHeight] = useState(160);
  const [big, setBig] = useState(false);
  const frame = useRef(null);
  const lastSend = useRef(0);
  useEffect(() => {
    if (!checked.ok) return undefined;
    let alive = true;
    (async () => {
      let u = null;
      try { u = await api().uiPut(checked.html); } catch { u = null; }
      if (!alive) return;
      if (u) setUrl(u); else setFailed('Die App konnte das HTML nicht bereitstellen.');
    })();
    return () => { alive = false; };
  }, [checked]);
  useEffect(() => {
    const onMsg = (e) => {
      if (!frame.current || e.source !== frame.current.contentWindow) return;
      const d = e.data;
      if (!d || d.askUi !== 1 || typeof d.art !== 'string') return;
      if (d.art === 'hoehe' && Number.isFinite(d.wert)) setHeight(Math.min(MAX_H, Math.max(MIN_H, Math.ceil(d.wert) + 4)));
      const text = typeof d.wert === 'string' ? d.wert.trim() : '';
      if (!text) return;
      if (d.art === 'eingeben') ctx.uiCompose?.(text);
      if (d.art === 'senden') {
        // Senden nur nach einer Bedienung im Baustein (er hat den Fokus) und höchstens alle 3 s; sonst ins Eingabefeld
        const now = Date.now();
        const fresh = document.activeElement === frame.current && now - lastSend.current > 3000;
        if (fresh) { lastSend.current = now; ctx.uiSend?.(text); } else ctx.uiCompose?.(text);
      }
    };
    window.addEventListener('message', onMsg);
    return () => window.removeEventListener('message', onMsg);
  }, [ctx]);
  if (!checked.ok) return <Broken part={part} error={checked.error} ctx={ctx} />;
  if (failed) return <Broken part={part} error={failed} ctx={ctx} />;
  return (
    <div className={`ui-html${big ? ' big' : ''}`}>
      <div className="ui-html-head">
        <span className="muted"><Icon name="shield" size={13} /> Interaktive Ansicht von ASK · ohne Internet</span>
        <span className="spacer" />
        <button type="button" className="icon-btn" title="Neu laden" aria-label="Neu laden"
          onClick={() => { if (frame.current && url) frame.current.src = url; }}><Icon name="reload" size={14} /></button>
        <button type="button" className="icon-btn" title={big ? 'Verkleinern' : 'Vergrössern'} aria-label={big ? 'Verkleinern' : 'Vergrössern'}
          onClick={() => setBig(!big)}><Icon name="expand" size={14} /></button>
        <button type="button" className="icon-btn" title="HTML kopieren" aria-label="HTML kopieren" onClick={() => ctx.copy?.(part.src)}><Icon name="copy" size={14} /></button>
      </div>
      {url ? (
        <iframe ref={frame} src={url} title="Interaktive Ansicht" sandbox="allow-scripts allow-forms" referrerPolicy="no-referrer"
          style={big ? undefined : { height }} loading="lazy" />
      ) : <div className="ui-pending"><span className="spin small" aria-hidden="true" /> Lädt …</div>}
    </div>
  );
}
