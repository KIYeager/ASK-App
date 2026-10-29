// Das Modell sieht, was es erstellt hat (ENTSCHEIDE E25): view_file zeichnet Bilder, PDF, Word, Excel
// und PowerPoint als PNG, browser_test öffnet eine HTML-Datei in einem unsichtbaren, abgeschotteten
// Fenster, führt Klicks und Tasten aus und macht ein Bildschirmfoto. Die Bilder landen in
// .ask/arbeit/<Chat>/ansicht/; die Arbeitsschleife (agent.js) schickt sie dem Modell als Nachricht.
const fs = require('node:fs');
const path = require('node:path');

const OUT_DIR = (chat) => `.ask/arbeit/${chat}/ansicht`;
const IMAGE_MAX = 1600; // Kantenlänge der Bilder für das Modell
const PAGE_WIDTH = 1240; // Seitenbreite in Pixel (A4 etwa 150 dpi)
const MAX_PAGES = 6;
const DEFAULT_PAGES = 3;
const MAX_PDF = 40 * 1024 * 1024;
const BROWSER_SIZE = { width: 1280, height: 800 };
const LOAD_TIMEOUT = 15000;
const MAX_ACTIONS = 25;
const LIVE_FPS = 12; // Bilder pro Sekunde, wenn unsichtbar getestet wird
const OFFICE = new Set(['.docx', '.xlsx', '.pptx']);
const RASTER = new Set(['.png', '.jpg', '.jpeg']);
const OTHER_IMAGES = { '.gif': 'image/gif', '.webp': 'image/webp', '.bmp': 'image/bmp', '.svg': 'image/svg+xml' };
const HTML = new Set(['.html', '.htm']);

const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const withTimeout = (promise, ms, text) => Promise.race([promise, wait(ms).then(() => { throw new Error(text); })]);

// «1-3», «2, 5», «4» → [1, 2, 3]; leer → die ersten DEFAULT_PAGES
function parsePages(text) {
  const out = [];
  for (const part of String(text ?? '').split(/[,;\s]+/).filter(Boolean)) {
    const m = /^(\d+)(?:\s*[-–]\s*(\d+))?$/.exec(part);
    if (!m) continue;
    const from = Number(m[1]);
    const to = Math.min(Number(m[2] || m[1]), from + MAX_PAGES);
    for (let p = from; p <= to; p++) if (p >= 1 && !out.includes(p)) out.push(p);
  }
  return (out.length ? out : Array.from({ length: DEFAULT_PAGES }, (_, i) => i + 1)).slice(0, MAX_PAGES);
}

// Tastennamen für sendInputEvent: «Pfeil hoch» usw. auf Electron-Namen abbilden
const KEYS = { pfeilhoch: 'Up', pfeilrunter: 'Down', pfeillinks: 'Left', pfeilrechts: 'Right', arrowup: 'Up', arrowdown: 'Down',
  arrowleft: 'Left', arrowright: 'Right', leertaste: 'Space', ' ': 'Space', space: 'Space', enter: 'Enter', return: 'Enter',
  escape: 'Escape', esc: 'Escape', tab: 'Tab', backspace: 'Backspace', rücktaste: 'Backspace', shift: 'Shift', umschalt: 'Shift' };
const keyCode = (name) => KEYS[String(name).toLowerCase().replace(/[\s_-]/g, '')] || String(name);

class Visual {
  // electron: { BrowserWindow, nativeImage }; convertToPdf(root, file) aus office.js; renderPage: Pfad zu render/pdf.html;
  // getPane(): Promise auf { wc, mode, allow(url), close() } des Browser-Bereichs oder null
  constructor({ electron, convertToPdf, renderPage, getPane }) {
    this.electron = electron;
    this.getPane = getPane || null; // sichtbarer Browser-Bereich der App (main.js), sonst null
    this.convertToPdf = convertToPdf;
    this.renderPage = renderPage;
    this.window = null;
    this.idle = null;
  }

  // Datei im Arbeitsordner, sonst Fehler
  resolve(root, rel) {
    const full = path.resolve(root, String(rel || ''));
    let real;
    try {
      real = fs.realpathSync.native(full);
    } catch {
      throw new Error(`Datei nicht gefunden: ${rel}`);
    }
    const base = fs.realpathSync.native(root).toLowerCase();
    if (real.toLowerCase() !== base && !real.toLowerCase().startsWith(base + path.sep)) throw new Error('Nur Dateien im Arbeitsordner.');
    if (!fs.statSync(real).isFile()) throw new Error(`${rel} ist keine Datei.`);
    return real;
  }

  save(root, chat, name, png) {
    const rel = `${OUT_DIR(chat)}/${name}`;
    const full = path.join(root, ...rel.split('/'));
    fs.mkdirSync(path.dirname(full), { recursive: true });
    fs.writeFileSync(full, png);
    return rel;
  }

  // Unsichtbares Fenster mit pdf.js, wird nach einer Minute ohne Auftrag geschlossen
  async renderer() {
    clearTimeout(this.idle);
    this.idle = setTimeout(() => this.closeRenderer(), 60000);
    if (this.window && !this.window.isDestroyed()) return this.window.webContents;
    const { BrowserWindow } = this.electron;
    this.window = new BrowserWindow({ show: false, width: 800, height: 600,
      webPreferences: { sandbox: true, contextIsolation: true, nodeIntegration: false, partition: 'ask-render', spellcheck: false } });
    this.window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
    this.window.webContents.on('will-navigate', (e) => e.preventDefault());
    await this.window.loadFile(this.renderPage);
    return this.window.webContents;
  }

  closeRenderer() {
    clearTimeout(this.idle);
    if (this.window && !this.window.isDestroyed()) this.window.destroy();
    this.window = null;
  }

  png(dataUrl) {
    return Buffer.from(String(dataUrl).replace(/^data:image\/png;base64,/, ''), 'base64');
  }

  // view_file: { ok, output, summary, images: [Pfade] }
  async view(root, rel, { seiten, chat }) {
    const full = this.resolve(root, rel);
    const ext = path.extname(full).toLowerCase();
    const stem = `${path.basename(full, ext).replace(/[^\w.-]+/g, '_').slice(0, 40)}-${Date.now().toString(36)}`;
    if (HTML.has(ext)) return this.browse(root, rel, { chat });
    if (RASTER.has(ext)) {
      const { nativeImage } = this.electron;
      let img = nativeImage.createFromPath(full);
      if (img.isEmpty()) throw new Error('Das Bild lässt sich nicht öffnen.');
      const { width, height } = img.getSize();
      if (Math.max(width, height) > IMAGE_MAX) img = width >= height ? img.resize({ width: IMAGE_MAX, quality: 'good' }) : img.resize({ height: IMAGE_MAX, quality: 'good' });
      const saved = this.save(root, chat, `${stem}.png`, img.toPNG());
      return { ok: true, output: `Bild ${width} × ${height} Pixel. Es folgt als Bild in der nächsten Nachricht.`, summary: `${width} × ${height} Pixel`, images: [saved] };
    }
    if (OTHER_IMAGES[ext]) {
      const wc = await this.renderer();
      const dataUrl = `data:${OTHER_IMAGES[ext]};base64,${fs.readFileSync(full).toString('base64')}`;
      const r = await withTimeout(wc.executeJavaScript(`renderImage(${JSON.stringify(dataUrl)}, ${IMAGE_MAX})`), 30000, 'Das Zeichnen dauert zu lange.');
      const saved = this.save(root, chat, `${stem}.png`, this.png(r.url));
      return { ok: true, output: `Bild ${r.width} × ${r.height} Pixel. Es folgt als Bild in der nächsten Nachricht.`, summary: `${r.width} × ${r.height} Pixel`, images: [saved] };
    }
    let pdf = full;
    let note = '';
    if (OFFICE.has(ext)) {
      const res = await this.convertToPdf(root, full);
      if (!res.ok) throw new Error(`Umwandlung mit Office nicht möglich: ${res.fehler}`);
      pdf = res.target;
      note = 'Mit Office in PDF umgewandelt. ';
    } else if (ext !== '.pdf') {
      throw new Error('view_file zeigt Bilder (PNG, JPG, GIF, WebP, SVG), PDF, Word, Excel und PowerPoint (.docx, .xlsx, .pptx) und HTML.');
    }
    if (fs.statSync(pdf).size > MAX_PDF) throw new Error('Die PDF-Datei ist grösser als 40 MB.');
    const pages = parsePages(seiten);
    const wc = await this.renderer();
    const b64 = fs.readFileSync(pdf).toString('base64');
    const r = await withTimeout(wc.executeJavaScript(`renderPdf(${JSON.stringify(b64)}, ${JSON.stringify(pages)}, ${PAGE_WIDTH})`), 60000, 'Das Zeichnen der Seiten dauert zu lange.');
    const images = r.images.map((img) => this.save(root, chat, `${stem}-s${img.page}.png`, this.png(img.url)));
    const shown = r.images.map((i) => i.page);
    const unit = ext === '.pptx' ? 'Folien' : 'Seiten';
    if (!shown.length) return { ok: false, output: `${note}Das Dokument hat ${r.count} ${unit}; die gewünschten gibt es nicht.`, summary: `${r.count} ${unit}` };
    const more = r.count > shown[shown.length - 1] ? ` Weitere ${unit} mit seiten (z. B. «${shown[shown.length - 1] + 1}-${Math.min(r.count, shown[shown.length - 1] + MAX_PAGES)}»).` : '';
    return { ok: true, images, summary: `${unit} ${shown.join(', ')} von ${r.count}`,
      output: `${note}${r.count} ${unit}; angesehen: ${shown.join(', ')}. Die Bilder folgen in der nächsten Nachricht.${more}` };
  }

  // Unsichtbares Fenster mit fester Grösse, wenn der Browser-Bereich der App nicht sichtbar ist (App
  // minimiert, im Test ohne Fenster). Liefert laufend Bilder an onLive.
  offscreen(live) {
    const { BrowserWindow } = this.electron;
    const win = new BrowserWindow({ show: false, ...BROWSER_SIZE, useContentSize: true,
      webPreferences: { offscreen: true, sandbox: true, contextIsolation: true, nodeIntegration: false, backgroundThrottling: false,
        partition: `ask-browser-${Date.now()}`, spellcheck: false } });
    const wc = win.webContents;
    let lastPaint = null;
    let dirty = false;
    wc.setFrameRate(30);
    wc.on('paint', (_e, _dirty, image) => { lastPaint = image; dirty = true; });
    wc.setWindowOpenHandler(() => ({ action: 'deny' }));
    wc.session.setPermissionRequestHandler((_wc, _perm, cb) => cb(false));
    wc.session.on('will-download', (e) => e.preventDefault());
    const stream = setInterval(() => {
      if (!dirty || !lastPaint || lastPaint.isEmpty()) return;
      dirty = false;
      live({ frame: `data:image/jpeg;base64,${lastPaint.resize({ width: 960, quality: 'good' }).toJPEG(80).toString('base64')}` });
    }, Math.round(1000 / LIVE_FPS));
    return {
      wc, mode: 'unsichtbar', lastPaint: () => lastPaint,
      allow: (url) => wc.on('will-navigate', (e, to) => { if (to.split('#')[0] !== url) e.preventDefault(); }),
      close: () => { clearInterval(stream); if (!win.isDestroyed()) win.destroy(); },
    };
  }

  // browser_test: HTML öffnen, Aktionen ausführen, Bildschirmfoto und Konsole zurück (ENTSCHEIDE E25, E27).
  // Sichtbar im Browser-Bereich der App (getPane), sonst unsichtbar. onLive meldet Start, Schritte,
  // Konsole und Ende (unsichtbar auch Bilder) an die Oberfläche.
  async browse(root, rel, { aktionen, warten, chat, onLive }) {
    const live = typeof onLive === 'function' ? onLive : () => {};
    const full = this.resolve(root, rel);
    if (!HTML.has(path.extname(full).toLowerCase())) throw new Error('browser_test öffnet nur .html-Dateien.');
    const fileUrl = require('node:url').pathToFileURL(full).href;
    live({ phase: 'start', path: rel });
    const pane = this.getPane ? await this.getPane().catch(() => null) : null;
    const target = pane || this.offscreen(live);
    live({ mode: target.mode });
    const { wc } = target;
    const logs = [];
    const notes = [];
    let synthetic = 0; // Zeitpunkt der letzten eigenen Eingabe; Eingaben danach stammen von der Person
    let userInputs = 0;
    const onConsole = (e, level, message, line, source) => {
      const lvl = e?.level ?? ['verbose', 'info', 'warning', 'error'][level] ?? 'info';
      if (lvl !== 'warning' && lvl !== 'error') return;
      const msg = e?.message ?? message;
      if (/Electron Security Warning/.test(msg)) return; // Hinweis von Electron selbst, nicht von der Seite
      const ln = e?.lineNumber ?? line;
      const src = path.basename(String(e?.sourceId ?? source ?? '')) || '';
      const entry = `${lvl === 'error' ? 'Fehler' : 'Warnung'}: ${msg}${src ? ` (${src}:${ln})` : ''}`;
      if (logs.length < 30) logs.push(entry);
      live({ log: entry, level: lvl });
    };
    const onGone = () => notes.push('Die Seite ist abgestürzt.');
    const onInput = (_e, input) => {
      if (['mouseDown', 'keyDown', 'rawKeyDown'].includes(input?.type) && Date.now() - synthetic > 150) userInputs++;
    };
    const send = (event) => { synthetic = Date.now(); wc.sendInputEvent(event); };
    wc.on('console-message', onConsole);
    wc.on('render-process-gone', onGone);
    wc.on('input-event', onInput);
    target.allow(fileUrl);
    try {
      await withTimeout(wc.loadFile(full), LOAD_TIMEOUT, 'Die Seite lädt nicht innerhalb von 15 Sekunden.');
      live({ step: 'geladen' });
      wc.focus();
      await wait(Math.min(Math.max(Number(warten) || 1000, 200), 10000));

      const done = [];
      for (const a of (Array.isArray(aktionen) ? aktionen : []).slice(0, MAX_ACTIONS)) {
        if (a.klick !== undefined || (a.x !== undefined && a.y !== undefined)) {
          let pos = a.x !== undefined ? { x: Number(a.x), y: Number(a.y) } : null;
          if (!pos) {
            pos = await wc.executeJavaScript(`(() => { const e = document.querySelector(${JSON.stringify(String(a.klick))});
              if (!e) return null; e.scrollIntoView({ block: 'center' }); const r = e.getBoundingClientRect();
              return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`).catch(() => null);
          }
          if (!pos) { done.push(`Klick ${a.klick}: Element nicht gefunden`); live({ step: done.at(-1) }); continue; }
          const at = { x: Math.round(pos.x), y: Math.round(pos.y) };
          send({ type: 'mouseMove', ...at });
          send({ type: 'mouseDown', ...at, button: 'left', clickCount: 1 });
          send({ type: 'mouseUp', ...at, button: 'left', clickCount: 1 });
          done.push(`Klick ${a.klick ?? `${at.x},${at.y}`}`);
        } else if (a.taste !== undefined) {
          const code = keyCode(a.taste);
          wc.focus();
          send({ type: 'keyDown', keyCode: code });
          if (code.length === 1) send({ type: 'char', keyCode: code });
          await wait(Math.min(Math.max(Number(a.dauer) || 60, 20), 3000));
          send({ type: 'keyUp', keyCode: code });
          done.push(`Taste ${a.taste}${a.dauer ? ` ${a.dauer} ms` : ''}`);
        } else if (a.text !== undefined) {
          synthetic = Date.now();
          await wc.insertText(String(a.text));
          done.push(`Text «${String(a.text).slice(0, 30)}»`);
        } else if (a.warten !== undefined) {
          await wait(Math.min(Math.max(Number(a.warten) || 0, 0), 5000));
          done.push(`${Math.min(Number(a.warten) || 0, 5000)} ms gewartet`);
          live({ step: done.at(-1) });
          continue;
        }
        live({ step: done.at(-1) });
        await wait(120);
      }
      await wait(300);
      const info = await wc.executeJavaScript(`({ title: document.title, text: (document.body ? document.body.innerText : '').replace(/\\s+/g, ' ').trim().slice(0, 600),
        vw: innerWidth, vh: innerHeight, h: document.documentElement.scrollHeight })`).catch(() => ({ title: '', text: '', vw: 0, vh: 0, h: 0 }));
      let shot = await wc.capturePage().catch(() => null);
      if ((!shot || shot.isEmpty()) && target.lastPaint) shot = target.lastPaint();
      if (shot && !shot.isEmpty()) {
        const s = shot.getSize();
        if (target.mode === 'unsichtbar' && (s.width > BROWSER_SIZE.width || s.height > BROWSER_SIZE.height)) { // Offscreen-Fenster sind teils ein paar Pixel grösser
          shot = shot.crop({ x: 0, y: 0, width: Math.min(s.width, BROWSER_SIZE.width), height: Math.min(s.height, BROWSER_SIZE.height) });
        } else if (s.width > IMAGE_MAX) {
          shot = shot.resize({ width: IMAGE_MAX, quality: 'good' });
        }
      }
      const images = shot && !shot.isEmpty()
        ? [this.save(root, chat, `${path.basename(full, path.extname(full)).replace(/[^\w.-]+/g, '_').slice(0, 40)}-${Date.now().toString(36)}-browser.png`, shot.toPNG())]
        : [];
      const errors = logs.filter((l) => l.startsWith('Fehler')).length;
      const size = info.vw ? `${info.vw} × ${info.vh}` : `${BROWSER_SIZE.width} × ${BROWSER_SIZE.height}`;
      const output = [
        `Geöffnet: ${rel}${info.title ? ` («${info.title}»)` : ''}, Fenster ${size}${info.h > (info.vh || BROWSER_SIZE.height) + 20 ? `, Seite ${info.h} Pixel hoch (nur der obere Teil ist im Bild)` : ''}.`,
        done.length ? `Aktionen: ${done.join('; ')}.` : '',
        userInputs ? `Die Person hat während des Tests selbst ${userInputs}-mal geklickt oder eine Taste gedrückt; das Bild zeigt auch ihre Eingaben.` : '',
        logs.length ? `Konsole:\n${logs.join('\n')}` : 'Konsole: keine Fehler oder Warnungen.',
        ...notes,
        info.text ? `Sichtbarer Text (Anfang): ${info.text}` : 'Kein sichtbarer Text.',
        images.length ? 'Das Bildschirmfoto folgt in der nächsten Nachricht.' : 'Kein Bildschirmfoto möglich.',
      ].filter(Boolean).join('\n');
      const summary = `${errors ? `${errors} Fehler in der Konsole` : 'keine Fehler'}${images.length ? ' · Bildschirmfoto' : ''}`;
      if (target.mode === 'unsichtbar' && shot && !shot.isEmpty()) live({ frame: `data:image/jpeg;base64,${shot.toJPEG(85).toString('base64')}` });
      live({ phase: 'end', summary, userInputs });
      return { ok: true, output, images, summary };
    } catch (err) {
      live({ phase: 'end', summary: err.message, failed: true });
      throw err;
    } finally {
      wc.removeListener('console-message', onConsole);
      wc.removeListener('render-process-gone', onGone);
      wc.removeListener('input-event', onInput);
      target.close();
    }
  }
}

module.exports = { Visual, parsePages, keyCode, OUT_DIR };
