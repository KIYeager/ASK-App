// Download-Seite für die Lernenden (ENTSCHEIDE E63): eine einzelne index.html mit Version, Datum, Grösse
// der Dateien, den Neuerungen und der Anleitung für SmartScreen. Sie liegt auf GitHub Pages von ASK-Releases
// (docs/index.html) und wird nach jeder normalen Version neu geschrieben (.github/workflows/download-page.yml).
// Die Seite fragt nie selbst bei GitHub nach: In der Schule teilt sich eine Klasse eine IP-Adresse, und die
// GitHub-API erlaubt ohne Anmeldung nur 60 Anfragen pro Stunde.
// Aufruf: node scripts/download-page.js <release.json> <Ausgabeordner>
//   release.json: Ausgabe von «gh release view --json tagName,body,publishedAt,assets»
const fs = require('node:fs');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const { updates } = require('../resources/app-config.json');

const esc = (s) => String(s).replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

const mb = (bytes) => `${Math.max(1, Math.round(bytes / 1e6))} MB`;

function dateText(iso) {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  return new Intl.DateTimeFormat('de-CH', { day: 'numeric', month: 'long', year: 'numeric', timeZone: 'Europe/Zurich' }).format(d);
}

// Angaben aus «gh release view --json …»
function fromRelease(release) {
  const version = String(release.tagName || '').replace(/^v/, '');
  if (!/^\d+\.\d+\.\d+/.test(version)) throw new Error(`Keine Version im Release: ${release.tagName}`);
  const size = (name) => (release.assets || []).find((a) => a.name === name)?.size || 0;
  return { version, notes: String(release.body || '').trim(), date: release.publishedAt, setupSize: size('ASK-Setup.exe'), portableSize: size('ASK.exe') };
}

const logo = () => `data:image/png;base64,${fs.readFileSync(path.join(__dirname, '..', 'src', 'renderer', 'public', 'tf-mark.png')).toString('base64')}`;

async function render({ version, notes, date, setupSize, portableSize, owner = updates.owner, repo = updates.repo }) {
  const { renderMarkdown } = await import(pathToFileURL(path.join(__dirname, '..', 'src', 'renderer', 'src', 'lib', 'md.mjs')).href);
  const base = `https://github.com/${owner}/${repo}/releases`;
  const file = (name) => `${base}/download/v${encodeURIComponent(version)}/${name}`;
  const v = esc(version);
  const meta = [`Version ${v}`, esc(dateText(date)), 'für Windows'].filter(Boolean).join(' · ');
  const notesHtml = notes ? renderMarkdown(notes) : '<p>Verbesserungen und Fehlerbehebungen.</p>';
  const mark = logo();

  return `<!doctype html>
<html lang="de-CH">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>ASK herunterladen</title>
<meta name="description" content="ASK Desktop der TF Bern für Windows herunterladen, Version ${v}.">
<link rel="icon" type="image/png" href="${mark}">
<style>
:root {
  --bg: #f5f7f8; --card: #ffffff; --soft: #f7f9fa; --line: #e1e7ea; --text: #141a1d; --muted: #5b6871;
  --acc: #0082b4; --acc-hover: #006b94; --acc-text: #006b94; --acc-ink: #ffffff; --acc-soft: #e6f3f8; --acc-line: #9ccce0;
  --warn: #8a5d00; --warn-soft: #fff7e8; --warn-line: #efd398; --code: #f1f5f7;
  --ss-bg: #1a5fb4; --ss-text: #ffffff; --ss-dim: rgba(255, 255, 255, .55);
  --shadow: 0 1px 2px rgba(16, 24, 32, .05), 0 8px 24px rgba(16, 24, 32, .06);
  --sans: "Segoe UI Variable Text", "Segoe UI", system-ui, -apple-system, "Helvetica Neue", Arial, sans-serif;
  --mono: "Cascadia Mono", Consolas, "DejaVu Sans Mono", monospace;
  color-scheme: light;
}
@media (prefers-color-scheme: dark) {
  :root {
    --bg: #101314; --card: #16191b; --soft: #1c2023; --line: #2a2f33; --text: #eceff1; --muted: #9ba6ae;
    --acc: #55aacf; --acc-hover: #7cc0dd; --acc-text: #7cc0dd; --acc-ink: #06222e; --acc-soft: #173040; --acc-line: #2c5b72;
    --warn: #e7bb5a; --warn-soft: #2a2415; --warn-line: #5a4a22; --code: #1f2427;
    --ss-bg: #174f96; --ss-dim: rgba(255, 255, 255, .45);
    --shadow: 0 1px 2px rgba(0, 0, 0, .3), 0 8px 24px rgba(0, 0, 0, .25);
    color-scheme: dark;
  }
  .mark { filter: brightness(0) invert(1); }
}
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; }
body { margin: 0; background: var(--bg); color: var(--text); font: 16px/1.55 var(--sans); -webkit-font-smoothing: antialiased; }
a { color: var(--acc-text); }
.wrap { max-width: 760px; margin: 0 auto; padding: 24px 16px 48px; }
header { display: flex; align-items: center; gap: 12px; margin-bottom: 28px; }
.mark { width: 36px; height: 36px; }
header b { font-size: 17px; letter-spacing: .01em; }
header span { color: var(--muted); font-size: 14px; }
.card { background: var(--card); border: 1px solid var(--line); border-radius: 16px; box-shadow: var(--shadow); padding: 32px; }
.card + .card, .card + section { margin-top: 20px; }
h1 { font-size: clamp(28px, 6vw, 38px); line-height: 1.15; margin: 0 0 10px; letter-spacing: -.01em; }
h2 { font-size: 20px; margin: 0 0 14px; }
.lead { color: var(--muted); font-size: 17px; margin: 0 0 24px; max-width: 34em; }
.get { display: flex; flex-wrap: wrap; align-items: center; gap: 12px 18px; }
.btn { display: inline-flex; align-items: center; gap: 12px; background: var(--acc); color: var(--acc-ink); text-decoration: none;
  border-radius: 12px; padding: 14px 24px 14px 20px; font-weight: 600; font-size: 18px; transition: background .15s; }
.btn:hover { background: var(--acc-hover); }
.btn:focus-visible, .alt a:focus-visible { outline: 3px solid var(--acc-line); outline-offset: 2px; }
.btn svg { flex: none; }
.btn small { display: block; font-weight: 400; font-size: 13px; opacity: .85; }
.meta { color: var(--muted); font-size: 14px; }
.alt { margin: 22px 0 0; padding-top: 18px; border-top: 1px solid var(--line); color: var(--muted); font-size: 15px; }
.alt a { font-weight: 600; }
.note { display: none; margin: 0 0 20px; padding: 10px 14px; border-radius: 10px; background: var(--warn-soft); border: 1px solid var(--warn-line); color: var(--warn); font-size: 15px; }
.not-windows .note { display: block; }
ol.steps { list-style: none; counter-reset: s; margin: 0; padding: 0; display: grid; gap: 18px; }
ol.steps > li { counter-increment: s; display: grid; grid-template-columns: 34px minmax(0, 1fr); gap: 14px; align-items: start; }
ol.steps > li::before { content: counter(s); width: 34px; height: 34px; border-radius: 50%; background: var(--acc-soft); color: var(--acc-text);
  display: grid; place-items: center; font-weight: 700; border: 1px solid var(--acc-line); }
.steps b { display: block; }
.steps p { margin: 2px 0 0; color: var(--muted); }
.done .steps > li:first-child::before { content: "✓"; background: var(--acc); color: var(--acc-ink); border-color: var(--acc); }
.ss { display: grid; grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); gap: 12px; margin-top: 14px; }
.ss figure { margin: 0; }
.ss figcaption { font-size: 13px; color: var(--muted); margin-top: 6px; }
.win { background: var(--ss-bg); color: var(--ss-text); border-radius: 8px; padding: 14px 14px 12px; min-height: 150px; display: flex; flex-direction: column; font-size: 13px; line-height: 1.35; }
.win .t { font-weight: 600; font-size: 14px; margin-bottom: 8px; }
.win .bar { height: 6px; border-radius: 3px; background: var(--ss-dim); margin: 3px 0; }
.win .bar.s { width: 60%; }
.win .link { margin-top: 8px; align-self: flex-start; text-decoration: underline; }
.win .row { margin-top: auto; display: flex; flex-wrap: wrap; gap: 8px; justify-content: flex-end; padding-top: 10px; }
.win .b { border: 1px solid rgba(255, 255, 255, .7); border-radius: 4px; padding: 4px 10px; white-space: nowrap; }
.win .hl { outline: 3px solid #ffd166; outline-offset: 3px; border-radius: 4px; }
.notes { font-size: 15px; }
.notes > :first-child { margin-top: 0; }
.notes > :last-child { margin-bottom: 0; }
.notes ul, .notes ol { padding-left: 1.3em; }
.notes li + li { margin-top: 4px; }
.notes code { font-family: var(--mono); font-size: .9em; background: var(--code); padding: 1px 5px; border-radius: 4px; }
.notes .codeblock { background: var(--code); border-radius: 8px; padding: 10px 12px; overflow-x: auto; margin: 10px 0; }
.notes .codehead { display: none; }
.notes pre { margin: 0; }
.notes table { border-collapse: collapse; display: block; overflow-x: auto; }
.notes th, .notes td { border: 1px solid var(--line); padding: 4px 8px; text-align: left; }
.notes blockquote { margin: 10px 0; padding-left: 12px; border-left: 3px solid var(--line); color: var(--muted); }
footer { margin-top: 28px; color: var(--muted); font-size: 14px; display: flex; flex-wrap: wrap; gap: 8px 20px; }
@media (max-width: 560px) {
  .card { padding: 22px 18px; border-radius: 14px; }
  .btn { width: 100%; justify-content: center; }
  .ss { grid-template-columns: minmax(0, 1fr); }
}
@media (prefers-reduced-motion: reduce) { .btn { transition: none; } }
</style>
</head>
<body>
<div class="wrap">
  <header>
    <img class="mark" src="${mark}" alt="" width="36" height="36">
    <div><b>ASK Desktop</b><br><span>Technische Fachschule Bern</span></div>
  </header>

  <main>
    <div class="card">
      <h1>ASK für Windows</h1>
      <p class="lead">Man beschreibt einen Auftrag, und das Sprachmodell arbeitet selbständig in einem Ordner auf dem Laptop.</p>
      <p class="note" id="note">ASK läuft nur auf Windows. Diese Seite auf dem Windows-Laptop öffnen und dort herunterladen.</p>
      <div class="get">
        <a class="btn" id="setup" href="${file('ASK-Setup.exe')}" download>
          <svg width="26" height="26" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M12 4v11"/><path d="m7 10 5 5 5-5"/><path d="M5 20h14"/></svg>
          <span>Herunterladen<small>ASK-Setup.exe${setupSize ? ` · ${mb(setupSize)}` : ''}</small></span>
        </a>
        <span class="meta">${meta}</span>
      </div>
      <p class="alt">Ohne Installation, zum Beispiel wenn man auf dem Laptop nichts installieren darf:
        <a href="${file('ASK.exe')}" download>ASK.exe herunterladen</a>${portableSize ? ` (${mb(portableSize)})` : ''}.
        Diese Variante aktualisiert sich nicht selbst; sie meldet eine neue Version, und man lädt sie hier wieder herunter.</p>
    </div>

    <section class="card" id="anleitung" aria-labelledby="h-steps">
      <h2 id="h-steps">So geht’s</h2>
      <ol class="steps">
        <li><div><b>Herunterladen</b><p>Auf den blauen Knopf klicken. Die Datei landet im Ordner «Downloads».</p></div></li>
        <li><div><b>Doppelklicken</b><p>ASK-Setup.exe öffnen. Die Installation braucht keine Admin-Rechte und stellt keine Fragen.</p></div></li>
        <li><div><b>Warnt Windows? Trotzdem ausführen.</b>
          <p>Beim ersten Mal erscheint oft «Der Computer wurde durch Windows geschützt». Das ist normal, weil ASK nicht bei Microsoft registriert ist.</p>
          <div class="ss" aria-hidden="true">
            <figure><div class="win"><div class="t">Der Computer wurde durch Windows geschützt</div><div class="bar"></div><div class="bar s"></div><span class="link hl">Weitere Informationen</span></div>
              <figcaption>1. «Weitere Informationen» anklicken</figcaption></figure>
            <figure><div class="win"><div class="t">Der Computer wurde durch Windows geschützt</div><div class="bar"></div><div class="bar s"></div><div class="row"><span class="b hl">Trotzdem ausführen</span><span class="b">Nicht ausführen</span></div></div>
              <figcaption>2. «Trotzdem ausführen» anklicken</figcaption></figure>
          </div>
        </div></li>
        <li><div><b>Fertig</b><p>ASK startet und steht im Startmenü und auf dem Desktop. Gibt es eine neue Version, fragt ASK selbst, ob es sie installieren soll.</p></div></li>
      </ol>
    </section>

    <section class="card" aria-labelledby="h-notes">
      <h2 id="h-notes">Neu in Version ${v}</h2>
      <div class="notes">${notesHtml}</div>
    </section>
  </main>

  <footer>
    <a href="${base}">Alle Versionen und Beta-Versionen</a>
    <span>ASK Desktop · TF Bern</span>
  </footer>
</div>
<script>
(function () {
  var ua = navigator.userAgent || '';
  if (!/Windows/i.test(ua)) document.body.classList.add('not-windows');
  document.getElementById('setup').addEventListener('click', function () {
    document.body.classList.add('done');
    var steps = document.getElementById('anleitung');
    if (steps && steps.scrollIntoView) setTimeout(function () { steps.scrollIntoView({ behavior: matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth', block: 'start' }); }, 300);
  });
})();
</script>
</body>
</html>
`;
}

async function build(releaseFile, out) {
  const info = fromRelease(JSON.parse(fs.readFileSync(releaseFile, 'utf8')));
  fs.mkdirSync(out, { recursive: true });
  fs.writeFileSync(path.join(out, 'index.html'), await render(info));
  return info;
}

if (require.main === module) {
  const [releaseFile, out] = process.argv.slice(2);
  if (!releaseFile || !out) {
    console.error('Aufruf: node scripts/download-page.js <release.json> <Ausgabeordner>');
    process.exit(2);
  }
  build(releaseFile, out).then(
    (info) => console.log(`Download-Seite für ASK ${info.version} geschrieben: ${path.join(out, 'index.html')}`),
    (err) => { console.error(err.message); process.exit(1); },
  );
}

module.exports = { render, fromRelease, build, dateText, mb };
