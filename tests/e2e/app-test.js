// End-to-End-Test im Entwicklungsmodus. Wird über ASK_TEST_SCRIPT in den Hauptprozess geladen.
// Umgebung: ASK_TEST_REPORT (Pfad für das Ergebnis), ASK_TEST_SCENARIO (chat|anmelden|erststart|coworker),
// ASK_TEST_SHOTS (Ordner für Bildschirmfotos), ASK_TEST_ASK_URL (Mock-ASK).
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { runTool } = require('../../src/main/tools');

const wait = (ms) => new Promise((r) => setTimeout(r, ms));

async function until(label, fn, timeout = 60000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const v = await fn();
    if (v) return v;
    await wait(300);
  }
  throw new Error(`Zeitüberschreitung: ${label}`);
}

function getJson(url) {
  return new Promise((resolve, reject) => {
    http.get(url, (res) => {
      let b = '';
      res.on('data', (c) => { b += c; });
      res.on('end', () => { try { resolve(JSON.parse(b)); } catch (e) { reject(e); } });
    }).on('error', reject);
  });
}

// Schickt einen Auftrag über die Oberfläche und wartet, bis das Modell fertig ist.
async function order(shell, agent, text, timeout = 90000) {
  const before = agent.chat.messages.length;
  await shell(`(() => { const i = document.getElementById('input'); i.value = ${JSON.stringify(text)};
    i.dispatchEvent(new Event('input')); document.getElementById('send-btn').click(); })()`);
  // Kurze Abläufe können fertig sein, bevor die erste Abfrage kommt: darum auch neue Nachrichten zählen.
  await until('Auftrag gestartet', () => agent.running || agent.chat.messages.length > before, 10000);
  await until('Auftrag fertig', () => !agent.running, timeout);
  await wait(400);
}

// Skills mit echtem Python und Office, über dieselben Werkzeuge, die das Modell benutzt.
async function coworker(ctx, check, shell, shoot, folder) {
  const { terminal } = ctx;
  const repo = path.join(__dirname, '..', '..');
  const skeleton = (skill) => fs.readFileSync(path.join(repo, 'resources', 'skills', skill, 'SKILL.md'), 'utf8')
    .split('## Gerüst')[1].match(/```python\n([\s\S]*?)```/)[1];
  const run = (command) => runTool(terminal, 'run_command', { command, wait: 180 });

  fs.writeFileSync(path.join(folder, 'daten.csv'),
    Buffer.from('Datum;Kategorie;Betrag\n01.09.2026;Bücher;12.50\n02.09.2026;Material;4.75\n', 'latin1'));
  for (const [skill, file] of [['word', 'Anleitung.docx'], ['excel', 'Auswertung.xlsx'], ['powerpoint', 'Praesentation.pptx']]) {
    const w = await runTool(terminal, 'write_file', { path: `.ask/skripte/${skill}.py`, content: skeleton(skill) });
    check(`Skript .ask/skripte/${skill}.py mit write_file`, w.ok, w.output);
    const r = await run(`python .ask/skripte/${skill}.py`);
    check(`${file} erstellt`, r.ok && fs.existsSync(path.join(folder, file)), r.output);
    const p = await run(`python .ask/skills/pruefen.py ${file} --pdf`);
    check(`pruefen.py --pdf ${file}`, p.ok && /PDF erstellt/.test(p.output) && /Seiten: \d/.test(p.output), p.output);
  }
  const visible = fs.readdirSync(folder).filter((n) => n.endsWith('.py'));
  check('Keine Skripte im sichtbaren Arbeitsordner', visible.length === 0, visible.join(', '));
  const doc = await runTool(terminal, 'read_file', { path: 'Anleitung.docx' });
  check('read_file liest Word als Text', doc.ok && doc.output.length > 50, doc.output.slice(0, 200));
  const pdfDir = path.join(folder, '.ask', 'pdf');
  check('PDFs in .ask/pdf', fs.existsSync(pdfDir) && fs.readdirSync(pdfDir).length === 3, fs.existsSync(pdfDir) ? fs.readdirSync(pdfDir).join(', ') : '-');

  const img = await run('python -c "import matplotlib.pyplot as p; p.bar([1,2],[3,4]); p.savefig(\'diagramm.png\')"');
  check('Diagramm mit matplotlib', img.ok, img.output);
  const { nativeImage } = require('electron');
  const vctx = { visual: ctx.agent.visual, folder, chat: 'test', canSee: true };
  const inked = (rel) => { // Grösse und Anteil nicht weisser Pixel
    const im = nativeImage.createFromPath(path.join(folder, rel)); const s = im.getSize(); const bm = im.toBitmap(); let n = 0;
    for (let i = 0; i < bm.length; i += 16) if (bm[i] < 200 || bm[i + 1] < 200 || bm[i + 2] < 200) n++;
    return { width: s.width, height: s.height, ink: n / (bm.length / 16) }; };
  const vdoc = await runTool(terminal, 'view_file', { path: 'Anleitung.docx' }, vctx);
  const d0 = vdoc.images?.[0] ? inked(vdoc.images[0]) : {};
  check('view_file: Word-Seite als Bild (Office → PDF → pdf.js)', vdoc.ok && vdoc.images.length >= 1 && d0.width === 1240 && d0.height > 1700 && d0.ink > 0.002 && d0.ink < 0.5,
    `${vdoc.output} ${JSON.stringify(d0)}`);
  const vppt = await runTool(terminal, 'view_file', { path: 'Praesentation.pptx', seiten: '2' }, vctx);
  const p0 = vppt.images?.[0] ? inked(vppt.images[0]) : {};
  check('view_file: Folie 2 als Bild im Querformat', vppt.ok && vppt.images.length === 1 && /Folien 2 von 2/.test(vppt.summary) && p0.width === 1240 && p0.height < 800 && p0.ink > 0.002,
    `${vppt.output} ${JSON.stringify(p0)}`);
  const vpng = await runTool(terminal, 'view_file', { path: 'diagramm.png' }, vctx);
  check('view_file: Diagramm', vpng.ok && vpng.images.length === 1 && inked(vpng.images[0]).ink > 0.01, vpng.output);
  const vbad = await runTool(terminal, 'view_file', { path: '../ausserhalb.png' }, vctx);
  check('view_file: nur Dateien im Arbeitsordner', !vbad.ok, vbad.output);
  fs.copyFileSync(path.join(pdfDir, fs.readdirSync(pdfDir).find((n) => n.endsWith('.pdf') && !n.includes('-'))),
    path.join(folder, 'Anleitung.pdf'));
  await until('Bild unter «Zuletzt geändert»', () => shell('[...document.querySelectorAll("#output-list li")].some((li) => li.textContent.includes("diagramm.png"))'), 10000);
  await shell('[...document.querySelectorAll("#output-list li")].find((li) => li.textContent.includes("diagramm.png")).click()');
  const imgOk = await until('Bildvorschau', () => shell('(() => { const i = document.querySelector("#preview-body img"); return i && i.complete && i.naturalWidth > 0; })()'), 10000).catch(() => false);
  check('Bildvorschau', imgOk);
  await shoot('vorschau-bild');
  await until('PDF unter «Zuletzt geändert»', () => shell('[...document.querySelectorAll("#output-list li")].some((li) => li.textContent.includes("Anleitung.pdf"))'), 10000);
  await shell('[...document.querySelectorAll("#output-list li")].find((li) => li.textContent.includes("Anleitung.pdf")).click()');
  check('PDF-Vorschau eingebettet', await until('PDF-iframe', () => shell('!!document.querySelector("#preview-body iframe")'), 5000).catch(() => false));
  await shell('[...document.querySelectorAll("#output-list li")].find((li) => li.textContent.includes("Auswertung.xlsx")).click()');
  await until('Office-Knöpfe', () => shell('!!document.querySelector("#preview-body .office")'), 5000);
  await shell('[...document.querySelectorAll("#preview-body .office button")].find((b) => b.textContent === "Als PDF anzeigen").click()');
  check('«Als PDF anzeigen» für Excel', await until('PDF aus Excel', () => shell('!!document.querySelector("#preview-body iframe")'), 120000).catch(() => false));
  await wait(2500);
  await shoot('vorschau-excel-pdf');
}

async function chat(ctx, check, shell, shoot) {
  const { appWindow, terminal, ask, agent, settings } = ctx;
  const mockUrl = process.env.ASK_TEST_ASK_URL;
  const folder = settings.get('folder');

  await until('angemeldet', () => ask.status.state === 'angemeldet', 30000);
  check('Bei ASK angemeldet (Token aus der Seite)', ask.status.user === 'Lea Test', JSON.stringify(ask.status));
  check('Modelle geladen, versteckte ausgeblendet', ask.models.length === 1 && ask.models[0].id === 'qwen-test', JSON.stringify(ask.models));
  await until('Anmeldeseite ausgeblendet', () => !appWindow.view.getVisible(), 5000).catch(() => {});
  check('ASK-Seite nach der Anmeldung verborgen', !appWindow.view.getVisible());
  check('Statuszeile «ASK: Lea Test»', (await shell('document.getElementById("ask-text").textContent')) === 'ASK: Lea Test');
  await shoot('1-leer');

  await order(shell, agent, 'Bitte teste die Werkzeuge');
  const log = await getJson(`${mockUrl}/__mock/chatlog`);
  check('Anfrage an ASK mit Modell, Streaming und Werkzeugliste', log[0]?.model === 'qwen-test' && log[0].stream === true
    && ['run_command', 'read_file', 'write_file', 'edit_file', 'update_tasks'].every((t) => log[0].tools.includes(t)), JSON.stringify(log[0]));
  check('Systemprompt nennt den Arbeitsordner', log[0]?.system.includes(folder), log[0]?.system.slice(0, 200));
  check('Werkzeugergebnisse gehen mit tool_call_id zurück', log[2]?.roles.filter((r) => r === 'tool').length === 3
    && log[2].toolIds.every(Boolean), JSON.stringify(log[2]));
  const rows = await shell(`[...document.querySelectorAll('.msg.tool')].map((m) => ({ call: m.querySelector('.call').textContent,
    result: m.querySelector('.result').textContent, bad: m.classList.contains('bad'), running: m.classList.contains('running') }))`);
  check('Drei Werkzeugzeilen im Chat', rows.length === 3, JSON.stringify(rows));
  check('Befehl mit Ausgabe «hallo aus ASK»', /Befehl\(echo hallo aus ASK\)/.test(rows[0]?.call) && rows[0].result.includes('hallo aus ASK'), JSON.stringify(rows[0]));
  check('Aufgaben im Chat nur als Zeile «1 von 3 erledigt», Liste in den Details', /Aufgaben/.test(rows[1]?.call) && rows[1].result.includes('1 von 3 erledigt')
    && !rows[1].result.includes('Befehl testen') && await shell('[...document.querySelectorAll(".msg.tool")][1].querySelector(".details").textContent.includes("☒ Befehl testen")'), JSON.stringify(rows[1]));
  check('Datei geschrieben', fs.readFileSync(path.join(folder, 'ausgabe', 'bericht.txt'), 'utf8') === 'Inhalt des Berichts');
  const final = await shell(`(() => { const a = [...document.querySelectorAll('.msg.assistant')].pop();
    return { md: a?.querySelector('.md')?.innerHTML || '', think: a?.querySelector('details.thinking pre')?.textContent || '' }; })()`);
  check('Schlusstext als Markdown (Code, Liste, fett)', /Fertig/.test(final.md) && /<code class="file">ausgabe\/bericht.txt<\/code>/.test(final.md)
    && /<li>/.test(final.md) && /<strong>Bericht<\/strong>/.test(final.md), final.md);
  check('Denktext getrennt (reasoning und <think>)', final.think.includes('Ich prüfe') && final.think.includes('Alles erledigt') && !final.md.includes('think'), final.think);
  await until('Aufgaben über der Eingabe', () => shell('!document.getElementById("todo").hidden'), 10000).catch(() => {});
  const todoLine = await shell(`({ count: document.getElementById('todo-count').textContent, current: document.getElementById('todo-current').textContent,
    open: document.getElementById('todo').classList.contains('open'), sidebar: !!document.getElementById('tasks-panel') })`);
  check('Aufgaben über der Eingabe: nach dem Auftrag eine Zeile «1/3 · Bericht schreiben»', todoLine.count === '1/3' && todoLine.current === 'Bericht schreiben'
    && !todoLine.open && !todoLine.sidebar, JSON.stringify(todoLine));
  await shell(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 't', ctrlKey: true, bubbles: true }))`);
  const todoOpen = await shell(`({ open: document.getElementById('todo').classList.contains('open'),
    items: [...document.querySelectorAll('#todo-list li')].map((li) => li.className + ':' + li.textContent) })`);
  check('Ctrl+T klappt die Liste auf (erledigt, aktuell, offen)', todoOpen.open && todoOpen.items.join('|') === 'erledigt:✓Befehl testen|in_arbeit:▸Bericht schreiben|offen:○Prüfen',
    JSON.stringify(todoOpen));
  await shoot('2a-aufgaben');
  await shell(`document.dispatchEvent(new KeyboardEvent('keydown', { key: 't', ctrlKey: true, bubbles: true }))`);
  await until('Datei unter «Zuletzt geändert»', () => shell('[...document.querySelectorAll("#output-list li")].some((li) => li.textContent.includes("bericht.txt"))'), 10000);
  await shell('[...document.querySelectorAll(".md code")].find((c) => c.textContent === "ausgabe/bericht.txt").click()');
  await until('Vorschau', () => shell('(document.querySelector("#preview-body pre")||{}).textContent === "Inhalt des Berichts"'), 5000)
    .then(() => check('Klick auf Dateinamen im Chat zeigt die Vorschau', true))
    .catch(() => check('Klick auf Dateinamen im Chat zeigt die Vorschau', false));
  const saved = fs.readdirSync(path.join(folder, '.ask', 'chats'));
  check('Chat in .ask/chats gespeichert', saved.length === 1, saved.join(','));
  check('Tokenzahlen bei ASK angefordert', log.every((l) => l.usage), JSON.stringify(log.map((l) => l.usage)));
  const stats = await shell(`(() => { const s = [...document.querySelectorAll('.msg.stats')].pop(); return s ? { text: s.textContent, title: s.title } : null; })()`);
  check('Zeile mit Token/s nach dem Auftrag (gemessen, nicht geschätzt)', stats && /3 Schritte · [\d’']+ Token · [\d’']+ Token\/s · Modell [\d.]+ s/.test(stats.text)
    && !stats.text.includes('≈') && /Schritt 3: .*Eingabe/.test(stats.title), JSON.stringify(stats));
  const savedStats = JSON.parse(fs.readFileSync(path.join(folder, '.ask', 'chats', saved[0]), 'utf8')).messages.filter((m) => m.role === 'assistant').map((m) => m.stats);
  check('Werte pro Schritt im Chat gespeichert', savedStats.length === 3 && savedStats.every((s) => s && s.tokens > 0 && !s.estimated && s.genMs > 0), JSON.stringify(savedStats));
  check('Unter dem Auftrag: neue Datei mit «Rückgängig»', await shell(`(() => { const c = [...document.querySelectorAll('.msg.changes')];
    return c.length === 1 && c[0].textContent.includes('1 neu') && c[0].textContent.includes('bericht.txt') && !c[0].querySelector('.undo-btn').hidden; })()`));
  check('Ohne Schalter keine Websuche angefordert', log.every((l) => !l.web), JSON.stringify(log.map((l) => l.web)));
  await shoot('2-chat');

  // Websuche: Schalter, nur bei der ersten Anfrage des Auftrags, Quellen unter der Antwort
  check('Websuche in ASK freigegeben, Schalter aktiv', ask.webSearch === true && await shell('!document.getElementById("web-btn").disabled'));
  await shell('document.getElementById("web-btn").click()');
  await until('Websuche an', () => shell('document.getElementById("web-btn").classList.contains("on")'), 5000).catch(() => {});
  check('Schalter «Web» schaltet ein und bleibt gespeichert', settings.get('webSearch') === true);
  const before = (await getJson(`${mockUrl}/__mock/chatlog`)).length;
  await order(shell, agent, 'Mit Websuche: was ist aktuell bei Proxmox?');
  const webLog = (await getJson(`${mockUrl}/__mock/chatlog`)).slice(before);
  check('Websuche nur bei der ersten Anfrage des Auftrags', webLog.length === 2 && webLog[0].web && !webLog[1].web, JSON.stringify(webLog.map((l) => l.web)));
  const sources = await shell(`(() => { const s = [...document.querySelectorAll('.msg.sources')].pop(); return s ? { text: s.textContent, links: [...s.querySelectorAll('a')].map((a) => a.getAttribute('href')) } : null; })()`);
  check('Quellen unter der Antwort (Titel, Link, Domain)', sources && sources.text.includes('2 Quellen') && sources.text.includes('Proxmox News')
    && sources.text.includes('proxmox.com') && sources.links.includes('https://pve.proxmox.com/wiki/Roadmap'), JSON.stringify(sources));
  const savedSources = JSON.parse(fs.readFileSync(path.join(folder, '.ask', 'chats', saved[0]), 'utf8')).messages.filter((m) => m.sources).map((m) => m.sources.length);
  check('Quellen im Chat gespeichert', savedSources.length === 1 && savedSources[0] === 2, JSON.stringify(savedSources));
  await shoot('2b-websuche');
  await shell('document.getElementById("web-btn").click()');
  await until('Websuche aus', () => settings.get('webSearch') === false, 5000).catch(() => {});
  check('Schalter «Web» schaltet wieder aus', settings.get('webSearch') === false);
await compact(ctx, check, shell, shoot);

  await githubFlow(ctx, check, shell, shoot);
  await attachFlow(ctx, check, shell, shoot);
  await undoFlow(ctx, check, shell, shoot);
  await visualFlow(ctx, check, shell, shoot);
  await webFlow(ctx, check, shell, shoot);
  await memoryFlow(ctx, check, shell, shoot);

  // Sperren gelten auch für die Werkzeuge der App
  const outside = path.join(path.dirname(folder), 'ausserhalb-chat.txt');
  const shellOut = await runTool(terminal, 'run_command', { command: `echo x > "${outside}"` });
  check('Schreiben ausserhalb per Befehl verweigert', !fs.existsSync(outside), shellOut.output);
  const apiOut = await runTool(terminal, 'write_file', { path: outside, content: 'x' });
  check('Schreiben ausserhalb per write_file verweigert', !apiOut.ok && !fs.existsSync(outside), apiOut.output);
  const edit = await runTool(terminal, 'edit_file', { path: 'ausgabe/bericht.txt', old_text: 'Inhalt', new_text: 'Neuer Inhalt' });
  check('edit_file ändert die Datei', edit.ok && fs.readFileSync(path.join(folder, 'ausgabe', 'bericht.txt'), 'utf8') === 'Neuer Inhalt des Berichts', edit.output);
  const found = await runTool(terminal, 'search_text', { text: 'neuer inhalt' });
  check('search_text findet den Text', found.ok && found.output.includes('bericht.txt'), found.output);
  const glob = await runTool(terminal, 'find_files', { pattern: '*.txt', path: 'ausgabe' });
  check('find_files findet die Datei', glob.ok && glob.output.includes('bericht.txt'), glob.output);
  const list = await runTool(terminal, 'list_files', {});
  check('list_files zeigt ausgabe/', list.ok && list.output.includes('ausgabe/'), list.output);

  // Stoppen während eines langen Befehls
  const stopStarted = Date.now();
  await shell(`(() => { const i = document.getElementById('input'); i.value = 'langsam bitte'; document.getElementById('send-btn').click(); })()`);
  await until('Befehl läuft', () => shell('!!document.querySelector(".msg.tool.running")'), 15000);
  await wait(1500);
  await shoot('3-laeuft');
  await shell('document.getElementById("send-btn").click()');
  await until('gestoppt', () => !agent.running, 15000);
  check('Stoppen beendet den Befehl rasch', Date.now() - stopStarted < 20000, `${Date.now() - stopStarted} ms`);
  check('Hinweis «Gestoppt.»', await shell('[...document.querySelectorAll(".msg.notice")].some((n) => n.textContent.includes("Gestoppt."))'));

  // Textmodus, wenn ASK die Werkzeugliste ablehnt; dazu ohne Tokenzahlen (stream_options abgelehnt)
  await getJson(`${mockUrl}/__mock/tools/aus`);
  await getJson(`${mockUrl}/__mock/usage/aus`);
  await order(shell, agent, 'textmodus bitte');
  const estimated = await shell(`[...document.querySelectorAll('.msg.stats')].pop()?.textContent || ''`);
  check('Ohne Tokenzahlen von ASK: geschätzt mit «≈»', /≈ [\d’']+ Token\/s/.test(estimated), estimated);
  const textRows = await shell(`[...document.querySelectorAll('.msg.tool')].map((m) => m.textContent)`);
  check('Textmodus: Werkzeug aus <tool_call> ausgeführt', agent.textMode && textRows.some((t) => t.includes('textmodus ok')), JSON.stringify(textRows.slice(-1)));
  check('Textmodus: Schlusstext', await shell('[...document.querySelectorAll(".msg.assistant .md")].pop().textContent === "Textmodus fertig."'));
  const tlog = await getJson(`${mockUrl}/__mock/chatlog`);
  check('Textmodus: Werkzeuge im Systemprompt, keine tool-Rollen', tlog.at(-1).tools.length === 0 && tlog.at(-1).system.includes('<tool_call>')
    && !tlog.at(-1).roles.includes('tool'), JSON.stringify(tlog.at(-1).roles));
  await getJson(`${mockUrl}/__mock/tools/an`);
  await getJson(`${mockUrl}/__mock/usage/an`);
  agent.textMode = false;
  ask.noUsage = false;

  // Linke Leiste: Chats nach Ordner, neuer Chat, Suche, gespeicherten Chat öffnen
  const items = () => shell(`[...document.querySelectorAll('.chat-group')].map((g) => ({ folder: g.dataset.folder, current: g.classList.contains('current'),
    chats: [...g.querySelectorAll('.chat-item')].map((i) => ({ title: i.querySelector('.ctitle').textContent, active: i.classList.contains('active') })) }))`);
  let bar = await until('Chat in der linken Leiste', async () => {
    const g = await items();
    return g[0]?.chats.some((c) => c.title === 'Bitte teste die Werkzeuge' && c.active) ? g : null;
  }, 5000).catch(() => null);
  check('Linke Leiste: aktueller Ordner zuerst, Chat aktiv markiert', bar && bar[0].current && bar[0].folder === folder, JSON.stringify(bar));
  await shell('document.getElementById("new-btn").click()');
  await until('leerer Chat', () => shell('document.querySelectorAll("#messages .msg").length === 0 && !document.getElementById("welcome").hidden'), 5000);
  check('Neuer Chat ist leer', true);
  bar = await items();
  check('Linke Leiste: «Neuer Chat» oben und aktiv, alter Chat darunter', bar[0].chats[0]?.title === 'Neuer Chat' && bar[0].chats[0].active
    && bar[0].chats[1]?.title === 'Bitte teste die Werkzeuge', JSON.stringify(bar[0]));
  await shell(`(() => { const i = document.getElementById('chat-search'); i.value = 'gibt es nicht'; i.dispatchEvent(new Event('input')); })()`);
  check('Suche ohne Treffer', await shell('document.querySelectorAll(".chat-item").length === 0 && document.getElementById("chat-groups").textContent.includes("Keine Treffer")'));
  await shell(`(() => { const i = document.getElementById('chat-search'); i.value = 'TESTE'; i.dispatchEvent(new Event('input')); })()`);
  check('Suche findet den Chat (ohne Gross-/Kleinschreibung)', await shell('[...document.querySelectorAll(".chat-item")].map((i) => i.textContent).join("|").includes("Bitte teste")'));
  await shell(`(() => { const i = document.getElementById('chat-search'); i.value = ''; i.dispatchEvent(new Event('input')); })()`);
  await shoot('4-chats');
  await shell('[...document.querySelectorAll(".chat-item")].find((i) => i.textContent.includes("Bitte teste")).click()');
  await until('Chat geladen', () => shell('document.querySelectorAll(".msg.tool").length >= 3'), 5000)
    .then(() => check('Gespeicherten Chat aus der linken Leiste öffnen', true))
    .catch(() => check('Gespeicherten Chat aus der linken Leiste öffnen', false));
  const firstChat = agent.chat.id;

  // Ordnerwechsel: neuer Ordner, neuer Chat, Systemprompt mit neuem Pfad
  const other = `${folder}-zwei`;
  fs.mkdirSync(other, { recursive: true });
  const opened = await ctx.openFolder(other);
  check('Ordnerwechsel akzeptiert', opened.ok, JSON.stringify(opened));
  await until('Terminal im neuen Ordner', () => terminal.state === 'läuft' && terminal.folder === opened.path, 60000);
  await until('leerer Chat im neuen Ordner', () => shell('document.querySelectorAll("#messages .msg").length === 0'), 5000)
    .then(() => check('Neuer Ordner beginnt mit leerem Chat', true))
    .catch(() => check('Neuer Ordner beginnt mit leerem Chat', false));
  await order(shell, agent, 'Bitte teste die Werkzeuge');
  const last = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Nach Ordnerwechsel nennt der Systemprompt den neuen Ordner', last.system.includes(opened.path));
  check('Datei im neuen Ordner geschrieben', fs.existsSync(path.join(opened.path, 'ausgabe', 'bericht.txt')));
  bar = await until('zwei Ordner in der Leiste', async () => { const g = await items(); return g.length >= 2 ? g : null; }, 5000).catch(() => []);
  check('Linke Leiste: neuer Ordner oben, alter Ordner mit seinem Chat darunter', bar[0]?.folder === opened.path && bar[0].current
    && bar[1]?.folder === folder && !bar[1].current && bar[1].chats.some((c) => c.title === 'Bitte teste die Werkzeuge'), JSON.stringify(bar));
  await shoot('5-zwei-ordner');

  // Chat aus dem anderen Ordner öffnen: wechselt den Ordner und öffnet genau diesen Chat
  await shell(`[...document.querySelectorAll('.chat-group')][1].querySelector('.chat-item').click()`);
  await until('zurück im ersten Ordner', () => terminal.state === 'läuft' && terminal.folder === folder && agent.chat.id === firstChat, 60000)
    .then(() => check('Chat aus anderem Ordner öffnen wechselt den Ordner und öffnet den Chat', true))
    .catch(() => check('Chat aus anderem Ordner öffnen wechselt den Ordner und öffnet den Chat', false, `${terminal.folder} ${agent.chat.id}`));
  await until('Chat angezeigt', () => shell('document.querySelectorAll(".msg.tool").length >= 3'), 5000).catch(() => {});

  // Chat im anderen Ordner löschen (die Rückfrage gibt es nur über die Oberfläche)
  const otherChats = agent.list(opened.path);
  const chatScripts = path.join(opened.path, '.ask', 'skripte', otherChats[0]?.id || 'x');
  fs.mkdirSync(chatScripts, { recursive: true });
  fs.writeFileSync(path.join(chatScripts, 'hilfe.py'), 'print(1)');
  check('Chat löschen', otherChats.length === 1 && agent.remove(opened.path, otherChats[0].id) && agent.list(opened.path).length === 0, JSON.stringify(otherChats));
  check('Skripte des gelöschten Chats sind weg', !fs.existsSync(chatScripts));
  check('Dateien im Ordner bleiben beim Löschen', fs.existsSync(path.join(opened.path, 'ausgabe', 'bericht.txt')));
}

// Anhänge: Bildschirmfoto einfügen, Datei hineinziehen, Bild geht ans Modell, Datei liegt in .ask/anhaenge/<Chat>/
async function attachFlow(ctx, check, shell, shoot) {
  const { agent, settings } = ctx;
  const mockUrl = process.env.ASK_TEST_ASK_URL;
  const folder = settings.get('folder');
  const chips = () => shell('document.querySelectorAll("#attach-row .attach-chip").length');
  const pasteImage = `(async () => {
    const c = document.createElement('canvas'); c.width = 40; c.height = 30;
    const g = c.getContext('2d'); g.fillStyle = '#c33'; g.fillRect(0, 0, 40, 30);
    const blob = await new Promise((r) => c.toBlob(r, 'image/png'));
    const dt = new DataTransfer(); dt.items.add(new File([blob], 'image.png', { type: 'image/png' }));
    document.getElementById('input').dispatchEvent(new ClipboardEvent('paste', { clipboardData: dt, bubbles: true, cancelable: true }));
    return true; })()`;
  const drop = (name, text) => `(() => {
    const dt = new DataTransfer(); dt.items.add(new File([${JSON.stringify(text)}], ${JSON.stringify(name)}, { type: 'text/plain' }));
    document.dispatchEvent(new DragEvent('drop', { dataTransfer: dt, bubbles: true, cancelable: true }));
    return true; })()`;

  await shell(pasteImage);
  await shell(drop('notiz.txt', 'Notiz für den Test'));
  await shell(drop('weg.txt', 'kommt wieder weg'));
  await until('drei Anhänge', async () => (await chips()) === 3, 5000).catch(() => {});
  check('Einfügen und Hineinziehen: drei Anhänge über der Eingabe', (await chips()) === 3 && await shell('!!document.querySelector("#attach-row img")'));
  await shell('[...document.querySelectorAll("#attach-row .attach-chip")].pop().querySelector(".remove").click()');
  check('Anhang lässt sich vor dem Senden entfernen', (await chips()) === 2);
  await shoot('12-anhaenge');

  await order(shell, agent, 'Schau dir das Bild an');
  const inboxDir = path.join(folder, '.ask', 'anhaenge', agent.chat.id);
  const inbox = fs.readdirSync(inboxDir);
  check('Anhänge in .ask/anhaenge/<Chat>/ gespeichert, Bildschirmfoto mit Datum benannt', inbox.includes('notiz.txt') && inbox.some((n) => /^bild-\d{8}-\d{6}\.png$/.test(n))
    && !inbox.includes('weg.txt') && fs.readFileSync(path.join(inboxDir, 'notiz.txt'), 'utf8') === 'Notiz für den Test'
    && !fs.existsSync(path.join(folder, 'eingang')), inbox.join(', '));
  let log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Bild geht ans Modell, Pfade stehen in der Nachricht', log.images === 1 && log.lastUser.includes(`.ask/anhaenge/${agent.chat.id}/notiz.txt`), JSON.stringify({ images: log.images, lastUser: log.lastUser }));
  check('Eingabe im Chat zeigt Bild und Datei, Eingabezeile wieder leer', await shell(`(() => { const u = [...document.querySelectorAll('.msg.user')].pop();
    return !!u.querySelector('img.attach-thumb') && u.textContent.includes('notiz.txt') && document.getElementById('attach-row').hidden; })()`));
  await until('Vorschaubild geladen', () => shell('[...document.querySelectorAll(".msg.user img.attach-thumb")].pop().naturalWidth > 0'), 5000).catch(() => {});
  check('Vorschaubild im Chat aus .ask/anhaenge geladen', await shell('[...document.querySelectorAll(".msg.user img.attach-thumb")].pop().naturalWidth === 40'));
  check('Anhänge im Chat gespeichert', agent.chat.messages.filter((m) => m.role === 'user').pop().attachments?.length === 2);

  await order(shell, agent, 'Werte den Anhang aus');
  const read = agent.chat.messages.filter((m) => m.role === 'tool').pop();
  check('Modell liest den Anhang', read.ok && read.content.includes('Notiz für den Test'), read.content);

  // ASK lehnt Bilder ab: einmal ohne Bild wiederholen, Hinweis, künftig nur Pfade
  await getJson(`${mockUrl}/__mock/vision/aus`);
  await shell(pasteImage);
  await until('Anhang', async () => (await chips()) === 1, 5000).catch(() => {});
  await order(shell, agent, 'Schau dir das Bild an');
  log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Bild abgelehnt: Hinweis und Antwort ohne Bild', log.images === 0 && agent.chat.messages.at(-1).content === 'Ich sehe das Bild.'
    && await shell('[...document.querySelectorAll(".msg.notice")].some((n) => n.textContent.includes("nimmt keine Bilder an"))'), JSON.stringify(log.images));
  await getJson(`${mockUrl}/__mock/vision/an`);
}

// Frühere Chats: Schalter «Chats», Liste im Systemprompt, chat_search und chat_read
async function memoryFlow(ctx, check, shell, shoot) {
  const { agent, settings } = ctx;
  const mockUrl = process.env.ASK_TEST_ASK_URL;
  const folder = settings.get('folder');
  const old = path.join(folder, '.ask', 'chats', 'alt-test.json');
  fs.writeFileSync(old, JSON.stringify({ id: 'alt-test', title: 'Alter Bericht über Proxmox', created: 1, updated: Date.now() - 86400000, messages: [
    { role: 'user', content: 'Schreib den Bericht über Proxmox Backups', time: 1 },
    { role: 'assistant', content: 'Der Bericht liegt in bericht.docx. Backups laufen jeden Tag um 2 Uhr.', time: 2 }] }));
  await order(shell, agent, 'Was haben wir früher gemacht?');
  let log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Schalter «Chats» aus: keine Liste, keine Chat-Werkzeuge', !log.system.includes('Frühere Chats') && !log.tools.includes('chat_search')
    && !(await shell('document.getElementById("memory-btn").classList.contains("on")')));
  await shell('document.getElementById("memory-btn").click()');
  await until('Chats an', () => settings.get('chatMemory') === true, 3000).catch(() => {});
  await order(shell, agent, 'Was haben wir früher gemacht?');
  const first = (await getJson(`${mockUrl}/__mock/chatlog`)).slice(-3)[0];
  check('Schalter «Chats» an: Liste früherer Chats im Systemprompt, Werkzeuge dabei', settings.get('chatMemory') === true && first.system.includes('# Frühere Chats in diesem Ordner')
    && first.system.includes('«Alter Bericht über Proxmox» (id alt-test)') && first.tools.includes('chat_search') && first.tools.includes('chat_read'), first.system.slice(-400));
  const tools = agent.chat.messages.filter((m) => m.role === 'tool').slice(-2);
  check('chat_search findet, chat_read liest den alten Chat', tools[0].ok && tools[0].content.includes('(id alt-test') && tools[1].ok
    && tools[1].content.includes('Backups laufen jeden Tag um 2 Uhr'), JSON.stringify(tools.map((t) => t.content.slice(0, 160))));
  await shell('document.getElementById("memory-btn").click()');
  await until('Chats aus', () => settings.get('chatMemory') === false, 3000).catch(() => {});
  fs.rmSync(old, { force: true });
}

// Internet: web_search über die Websuche von ASK, web_fetch liefert Text statt HTML, Quellen unter der Antwort
async function webFlow(ctx, check, shell, shoot) {
  const { agent } = ctx;
  const mockUrl = process.env.ASK_TEST_ASK_URL;
  await order(shell, agent, 'Bitte recherchiere Proxmox 9');
  const tools = agent.chat.messages.filter((m) => m.role === 'tool').slice(-2);
  const searches = await getJson(`${mockUrl}/__mock/searches`);
  check('web_search über die Websuche von ASK', tools[0].ok && searches.at(-1)?.query === 'proxmox 9 neuerungen' && searches.at(-1)?.queries?.[0] === 'proxmox 9 neuerungen'
    && tools[0].content.includes('Treffer (ASK)') && tools[0].content.includes('Proxmox VE 9 Testseite'), tools[0].content);
  check('web_fetch liefert lesbaren Text ohne HTML, Skript und Navigation', tools[1].ok && tools[1].content.includes('Proxmox VE 9 bringt einen neuen Kernel & bessere Backups.')
    && tools[1].content.includes('- Kernel 6.14') && !/<p>|geheim|Impressum|Menü/.test(tools[1].content), tools[1].content.slice(0, 400));
  const rows = await shell(`[...document.querySelectorAll('.msg.tool')].slice(-2).map((m) => m.querySelector('.call').textContent + ' ⎿ ' + m.querySelector('.result').textContent)`);
  check('Zeilen «Websuche(…)» und «Webseite(…)» im Chat', /^Websuche\(proxmox 9 neuerungen\) ⎿ ⎿2 Treffer$/.test(rows[0]) && /^Webseite\(http.*__mock\/seite\) ⎿ ⎿Proxmox VE 9 Testseite · [\d.]+k Zeichen$/.test(rows[1]), JSON.stringify(rows));
  const src = await shell(`(() => { const s = [...document.querySelectorAll('.msg.sources')].pop(); return s ? s.textContent : ''; })()`);
  check('Gelesene Seite als Quelle unter der Antwort', src.includes('1 Quelle') && src.includes('Proxmox VE 9 Testseite') && !src.includes('Roadmap'), src);
  const bad = await require('../../src/main/tools').runTool(null, 'web_fetch', { url: 'file:///C:/Windows/win.ini' }, { web: agent.web });
  check('web_fetch: nur http und https', !bad.ok && /http/.test(bad.output), bad.output);
}

// Ansehen: das Modell sieht ein Bild und testet eine HTML-Seite im unsichtbaren Browser
async function visualFlow(ctx, check, shell, shoot) {
  const { agent, settings, appWindow } = ctx;
  const { nativeImage } = require('electron');
  const mockUrl = process.env.ASK_TEST_ASK_URL;
  const folder = settings.get('folder');
  agent.noImages = false; // attachFlow liess ASK Bilder ablehnen; wie nach einem Neustart der App
  const red = nativeImage.createFromBitmap(Buffer.alloc(60 * 40 * 4, Buffer.from([0, 0, 200, 255])), { width: 60, height: 40 });
  fs.writeFileSync(path.join(folder, 'grafik.png'), red.toPNG());
  fs.writeFileSync(path.join(folder, 'spiel.html'), `<!doctype html><html><head><meta charset="utf-8"><title>Testspiel</title>
    <style>body{margin:0;background:#c00;color:#fff;font:32px sans-serif;height:100vh}</style></head>
    <body><button id="start">Start</button><p id="s">Bereit</p><script>
    document.getElementById('start').onclick = () => { document.getElementById('s').textContent = 'Gestartet'; };
    addEventListener('keydown', (e) => { if (e.key === 'ArrowRight') document.getElementById('s').textContent += ' Rechts'; });
    console.error('absichtlicher Testfehler');
    </script></body></html>`);

  await order(shell, agent, 'Bitte ansehen test');
  const viewed = agent.chat.messages.filter((m) => m.role === 'tool').pop();
  check('view_file: Bild für das Modell gespeichert', viewed.ok && viewed.images?.length === 1 && fs.existsSync(path.join(folder, viewed.images[0]))
    && viewed.images[0].startsWith(`.ask/arbeit/${agent.chat.id}/ansicht/`), JSON.stringify(viewed));
  let log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Bild geht als eigene Nachricht ans Modell', log.lastImages === 1 && log.lastUser.startsWith('[Ansicht] Bilder zu view_file(grafik.png)'), JSON.stringify({ images: log.images, lastUser: log.lastUser }));
  await until('Vorschaubild in der Werkzeugzeile', () => shell('(() => { const i = [...document.querySelectorAll(".msg.tool .shots img")].pop(); return !!i && i.naturalWidth === 60; })()'), 5000).catch(() => {});
  check('Werkzeugzeile «Ansehen» zeigt das Bild', await shell(`(() => { const t = [...document.querySelectorAll('.msg.tool')].pop();
    return t.querySelector('.call').textContent.startsWith('Ansehen(grafik.png') && t.querySelector('.shots img').naturalWidth === 60; })()`));
  check('Ansicht nicht als Eingabe im Chat', agent.transcript().every((i) => !String(i.content || '').startsWith('[Ansicht]')));

  await order(shell, agent, 'Bitte spiel testen');
  const tested = agent.chat.messages.filter((m) => m.role === 'tool').pop();
  check('browser_test: Konsole, Klick und Taste', tested.ok && tested.content.includes('Fehler: absichtlicher Testfehler') && tested.content.includes('Gestartet Rechts')
    && tested.content.includes('«Testspiel»'), tested.content);
  const shot = tested.images?.[0] ? nativeImage.createFromPath(path.join(folder, tested.images[0])) : null;
  const size = shot?.getSize() || {};
  const px = shot ? shot.toBitmap() : Buffer.alloc(0);
  const at = (x, y) => { const i = (y * size.width + x) * 4; return [px[i + 2], px[i + 1], px[i]]; }; // BGRA
  const [r, g, b] = shot ? at(Math.round(size.width * 0.7), Math.round(size.height * 0.7)) : [0, 0, 0];
  const pane = appWindow.browserBounds || {};
  check('Bildschirmfoto aus dem Browser-Bereich zeigt die Seite', size.width > 300 && size.height > 200 && r > 150 && g < 60 && b < 60
    && (([, w, h]) => Math.abs(w - pane.width) <= 2 && Math.abs(h - pane.height) <= 2)(/Fenster (\d+) × (\d+)/.exec(tested.content)?.map(Number) || []),
    JSON.stringify({ size, rgb: [r, g, b], pane, text: /Fenster \d+ × \d+/.exec(tested.content)?.[0] }));
  log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Bildschirmfoto geht ans Modell', log.lastImages === 1 && log.lastUser.startsWith('[Ansicht] Bilder zu browser_test(spiel.html'), JSON.stringify({ images: log.images, lastUser: log.lastUser }));
  const live = await shell(`({ visible: !document.getElementById('browser-pane').hidden, mode: document.getElementById('browser-pane').dataset.mode,
    status: document.getElementById('bp-status').textContent, log: document.getElementById('bp-log').textContent,
    path: document.getElementById('bp-path').textContent, running: document.getElementById('browser-pane').classList.contains('running'),
    sidebar: getComputedStyle(document.getElementById('sidebar')).display })`);
  check('Browser-Bereich: echte Seite sichtbar, Schritte, Konsole, Seitenleiste weg', live.visible && live.mode === 'sichtbar' && appWindow.browser?.getVisible()
    && live.status.startsWith('✓ 1 Fehler') && live.log.includes('absichtlicher Testfehler') && live.path === 'spiel.html' && !live.running && live.sidebar === 'none', JSON.stringify(live));
  check('Seite bleibt nach dem Test offen und bedienbar', appWindow.browser.webContents.getURL().endsWith('spiel.html')
    && await appWindow.browser.webContents.executeJavaScript('document.getElementById("s").textContent') === 'Gestartet Rechts');
  // Eigene Eingabe der Person: Klick in die Seite
  appWindow.browser.webContents.sendInputEvent({ type: 'keyDown', keyCode: 'Right' });
  appWindow.browser.webContents.sendInputEvent({ type: 'keyUp', keyCode: 'Right' });
  await wait(300);
  check('Eigene Tasten wirken in der Seite', (await appWindow.browser.webContents.executeJavaScript('document.getElementById("s").textContent')) === 'Gestartet Rechts Rechts');
  await shoot('14-ansehen');
  const small = appWindow.browserBounds.width;
  await shell('document.getElementById("bp-big").click()');
  await until('gross', () => appWindow.browserBounds.width > small + 200, 3000).catch(() => {});
  check('«Gross» vergrössert den Browser aufs ganze Fenster', appWindow.browserBounds.width > small + 200, `${small} → ${appWindow.browserBounds.width}`);
  await shoot('14b-browser-gross');
  await shell('document.getElementById("bp-big").click()');
  await shell('document.getElementById("bp-close").click()');
  await until('zu', () => !appWindow.browser.getVisible(), 3000).catch(() => {});
  check('Schliessen versteckt den Browser und leert die Seite', !appWindow.browser.getVisible() && await shell('getComputedStyle(document.getElementById("sidebar")).display !== "none"'));

  // App nicht sichtbar: unsichtbar testen, Bilder an die Oberfläche
  const { Visual } = require('../../src/main/visual');
  const { BrowserWindow } = require('electron');
  const events = [];
  const hidden = await new Visual({ electron: { BrowserWindow, nativeImage } }).browse(folder, 'spiel.html', { chat: agent.chat.id, onLive: (e) => events.push(e),
    aktionen: [{ klick: '#start' }] });
  const hiddenShot = nativeImage.createFromPath(path.join(folder, hidden.images[0])).getSize();
  check('Ausweichlösung unsichtbar: Bilder fürs Live-Bild, 1280 × 800', events.some((e) => e.mode === 'unsichtbar') && events.some((e) => String(e.frame).startsWith('data:image/jpeg'))
    && hiddenShot.width === 1280 && hiddenShot.height === 800 && hidden.output.includes('Gestartet'), JSON.stringify({ hiddenShot, modes: events.filter((e) => e.mode) }));
}

// Rückgängig: Auftrag ändert und legt an, Knopf setzt den Ordner zurück, das Modell erfährt es
async function undoFlow(ctx, check, shell, shoot) {
  const { agent, settings } = ctx;
  const folder = settings.get('folder');
  const bericht = path.join(folder, 'ausgabe', 'bericht.txt');
  const vorher = fs.readFileSync(bericht, 'utf8');
  await order(shell, agent, 'Bitte ändere den Bericht');
  check('Modell ändert direkt und legt neu an', fs.readFileSync(bericht, 'utf8') === 'Vom Modell geändert' && fs.existsSync(path.join(folder, 'projekt', 'neu.txt')));
  const row = await shell(`(() => { const c = [...document.querySelectorAll('.msg.changes')]; const last = c.pop();
    return { text: last.textContent, button: !last.querySelector('.undo-btn').hidden, others: c.every((r) => r.querySelector('.undo-btn').hidden) }; })()`);
  check('Änderungen «1 geändert · 1 neu», «Rückgängig» nur beim letzten Auftrag', /1 geändert · 1 neu/.test(row.text) && row.button && row.others, JSON.stringify(row));
  await shoot('13-aenderungen');
  await shell(`[...document.querySelectorAll('.msg.changes')].pop().querySelector('.undo-btn').click()`);
  await until('zurückgesetzt', () => agent.chat.messages.some((m) => m.note === 'undo'), 10000).catch(() => {});
  check('Rückgängig setzt die Datei zurück und löscht die neue', fs.readFileSync(bericht, 'utf8') === vorher
    && !fs.existsSync(path.join(folder, 'projekt', 'neu.txt')) && !fs.existsSync(path.join(folder, 'projekt')));
  check('Hinweis im Chat, Auftrag als rückgängig markiert', await shell(`(() => { const c = [...document.querySelectorAll('.msg.changes')].pop();
    return c.classList.contains('undone') && c.querySelector('.undo-btn').hidden
      && [...document.querySelectorAll('.msg.notice')].some((n) => n.textContent.includes('Rückgängig gemacht: 2 Dateien')); })()`));
  const note = agent.chat.messages.find((m) => m.note === 'undo');
  check('Das Modell erfährt es im Verlauf', note?.role === 'user' && note.content.includes('rückgängig gemacht'), JSON.stringify(note));
  const stands = fs.readdirSync(path.join(folder, '.ask', 'sicherung', 'staende')).length;
  check('Sicherung in .ask/sicherung (ein Stand pro Auftrag, einer vor Rückgängig)', stands >= 3, String(stands));
}

// GitHub: verbinden über /github, Projekt anlegen, hochladen, holen (gegen den Nachbau in mock-github.js)
async function githubFlow(ctx, check, shell, shoot) {
  const { agent, settings } = ctx;
  const mockUrl = process.env.ASK_TEST_ASK_URL;
  const folder = settings.get('folder');
  const onGitHub = (p) => getJson(`${mockUrl}/__mock/github/datei?repo=lea-test/testprojekt&path=${encodeURIComponent(p)}`);

  await order(shell, agent, 'Bitte auf github anlegen');
  const notConnected = await shell(`[...document.querySelectorAll('.msg.tool')].pop().querySelector('.result').textContent`);
  check('Ohne Verbindung: Hinweis auf /github', /Nicht mit GitHub verbunden.*\/github/.test(notConnected), notConnected);

  await shell(`(() => { const i = document.getElementById('input'); i.value = '/github'; i.dispatchEvent(new Event('input')); document.getElementById('send-btn').click(); })()`);
  await until('GitHub-Dialog', () => shell('!document.getElementById("dlg-github").hidden'), 5000);
  await shell(`(() => { document.getElementById('github-token').value = 'falsch'; document.getElementById('github-ok').click(); })()`).catch(() => {});
  await until('Fehler', () => shell('!document.getElementById("github-error").hidden'), 5000).catch(() => {});
  check('Falsches Token wird abgelehnt', await shell('document.getElementById("github-error").textContent.includes("Token")'));
  await shell(`(() => { document.getElementById('github-token').value = 'gh-test-token'; document.getElementById('github-ok').click(); })()`);
  await until('verbunden', () => shell('document.getElementById("github-body").textContent.includes("lea-test")'), 5000).catch(() => {});
  check('Verbunden als lea-test', await shell('document.getElementById("github-body").textContent.includes("Verbunden als lea-test")'));
  const stored = JSON.parse(fs.readFileSync(path.join(process.env.ASK_TEST_USER_DATA, 'einstellungen.json'), 'utf8')).githubToken;
  check('Token verschlüsselt gespeichert', stored && !stored.includes('gh-test-token') && !Buffer.from(stored, 'base64').toString('latin1').includes('gh-test-token'));
  await shoot('11-github');
  await shell('document.getElementById("github-close").click()');

  await order(shell, agent, 'Bitte auf github anlegen');
  const readme = await onGitHub('ausgabe/bericht.txt');
  check('Projekt auf GitHub angelegt und hochgeladen', readme.text === fs.readFileSync(path.join(folder, 'ausgabe', 'bericht.txt'), 'utf8'), JSON.stringify(readme));
  check('.ask nicht hochgeladen', (await onGitHub('.ask/tasks.json')).text === null);
  check('Ordner mit dem Repository verbunden', JSON.parse(fs.readFileSync(path.join(folder, '.ask', 'github.json'), 'utf8')).repo === 'lea-test/testprojekt');

  fs.writeFileSync(path.join(folder, 'ausgabe', 'bericht.txt'), 'Inhalt des Berichts, ergänzt');
  process.env.ASK_TEST_REMEMBER = '1'; // Häkchen «Für dieses Repository nicht mehr fragen»
  await order(shell, agent, 'Bitte auf github hochladen');
  delete process.env.ASK_TEST_REMEMBER;
  check('Häkchen: Repository ohne Rückfrage gemerkt', JSON.stringify(settings.get('githubTrusted')) === '["lea-test/testprojekt"]', JSON.stringify(settings.get('githubTrusted')));
  await shell(`(() => { const i = document.getElementById('input'); i.value = '/github'; i.dispatchEvent(new Event('input')); document.getElementById('send-btn').click(); })()`);
  await until('Liste ohne Rückfrage', () => shell('document.getElementById("github-body").textContent.includes("lea-test/testprojekt wieder fragen")'), 5000).catch(() => {});
  await shell('[...document.querySelectorAll("#github-body .trusted button")][0].click()').catch(() => {});
  await until('wieder fragen', () => (settings.get('githubTrusted') || []).length === 0, 5000).catch(() => {});
  check('«wieder fragen» im GitHub-Dialog', (settings.get('githubTrusted') || []).length === 0);
  await shell('document.getElementById("github-close").click()');
  check('Änderung als Commit hochgeladen', (await onGitHub('ausgabe/bericht.txt')).text === 'Inhalt des Berichts, ergänzt');

  await getJson(`${mockUrl}/__mock/github/aendern?repo=lea-test/testprojekt&path=notizen.txt&text=${encodeURIComponent('aus einer anderen Sitzung')}`);
  await order(shell, agent, 'Bitte von github holen');
  check('Änderung aus anderer Sitzung geholt', fs.readFileSync(path.join(folder, 'notizen.txt'), 'utf8') === 'aus einer anderen Sitzung');
  const rows = await shell(`[...document.querySelectorAll('.msg.tool')].slice(-3).map((r) => r.querySelector('.call').textContent + ' ⎿ ' + r.querySelector('.result').textContent)`);
  check('GitHub-Zeilen im Chat', rows.some((r) => /GitHub\(neu testprojekt \(privat\)\)/.test(r)) && rows.some((r) => /GitHub\(holen\)/.test(r)), JSON.stringify(rows));
  fs.writeFileSync(path.join(folder, 'ausgabe', 'bericht.txt'), 'Inhalt des Berichts'); // für die folgenden Prüfungen
}

// Anzeige wie Claude Code, Lese-Gruppe, Denkstufen, Schleifenschutz, versteckter Ordner .ask
async function compact(ctx, check, shell, shoot) {
  const { agent, settings } = ctx;
  const mockUrl = process.env.ASK_TEST_ASK_URL;
  const folder = settings.get('folder');
  const visible = (sel) => shell(`[...document.querySelectorAll(${JSON.stringify(sel)})].some((n) => n.offsetParent !== null)`);

  const attrib = require('node:child_process').execFileSync('attrib', [path.join(folder, '.ask')], { encoding: 'latin1' });
  check('Ordner .ask ist versteckt', /^\s*\S*\s*H\s/.test(attrib.split(folder)[0]) || / H /.test(attrib.split(folder)[0]), attrib.trim());

  check('Details der Werkzeuge zugeklappt', !(await visible('.msg.tool .details')));
  check('Gedanken ausgeblendet', !(await visible('details.thinking')));
  const first = await shell('(() => { const r = document.querySelector(".msg.tool"); return { call: r.querySelector(".call").textContent, result: r.querySelector(".result").textContent }; })()');
  check('Werkzeugzeile: Aufruf und Kurzresultat', /Befehl\(echo hallo aus ASK\)/.test(first.call) && first.result.includes('hallo aus ASK'), JSON.stringify(first));
  await shell('document.querySelector(".msg.tool .call").click()');
  check('Klick klappt Details auf (ganzer Befehl)', await shell('(() => { const d = document.querySelector(".msg.tool .details"); return d.offsetParent !== null && d.textContent.includes("echo hallo aus ASK"); })()'));
  await shell('document.querySelector(".msg.tool .call").click()');
  check('Nochmals klicken klappt zu', !(await shell('document.querySelector(".msg.tool .details").offsetParent !== null')));
  await shell('document.dispatchEvent(new KeyboardEvent("keydown", { key: "o", ctrlKey: true, bubbles: true }))');
  check('Ctrl+O zeigt Gedanken und alle Details', await visible('details.thinking') && await shell('[...document.querySelectorAll(".msg.tool .details")].every((d) => d.offsetParent !== null)'));
  await shoot('6-ausfuehrlich');
  await shell('document.dispatchEvent(new KeyboardEvent("keydown", { key: "o", ctrlKey: true, bubbles: true }))');
  check('Ctrl+O nochmals: wieder kompakt', !(await visible('details.thinking')));

  // Kopieren: Codeblock und ganze Antwort (Antwort mit Websuche enthält einen Codeblock)
  const { clipboard } = require('electron');
  // Die Zwischenablage teilt sich die App mit dem ganzen Rechner: mehrmals lesen, bis der Text da ist.
  const paste = async () => until('Zwischenablage', async () => String(await clipboard.readText()), 3000).catch(() => '');
  clipboard.clear();
  const block = await shell(`(() => { const b = [...document.querySelectorAll('.codeblock')].pop(); if (!b) return null;
    b.querySelector('button.copy').click(); return b.querySelector('.codehead span').textContent; })()`);
  await wait(300);
  const copiedCode = await paste();
  check('Codeblock mit Kopfzeile und Kopieren', block === 'bash' && copiedCode === 'pvesh get /version', `${block} / ${copiedCode}`);
  await shell(`[...document.querySelectorAll('.msg.assistant .msg-actions button.copy')].pop().click()`);
  await wait(300);
  const copiedAnswer = await paste();
  check('Ganze Antwort kopieren (Markdown)', copiedAnswer.startsWith('Laut der Proxmox-Webseite ist **Proxmox VE 8.3** aktuell.')
    && copiedAnswer.includes('```bash'), copiedAnswer.slice(0, 80));
  check('Inline-Code dezent, Datei hervorgehoben', await shell(`(() => { const f = document.querySelector('.md code.file'); const n = [...document.querySelectorAll('.md code')].find((c) => !c.classList.contains('file') && !c.closest('pre'));
    return !!f && getComputedStyle(f).color !== (n ? getComputedStyle(n).color : ''); })()`));

  // Mehrere Lese-Schritte als eine Zeile
  await order(shell, agent, 'Bitte lies die Dateien');
  const group = await shell(`(() => { const g = [...document.querySelectorAll('.msg.tool.group')].pop(); return g ? { call: g.querySelector('.call').textContent, subs: g.querySelectorAll('.details .sub').length } : null; })()`);
  check('Lese-Schritte als eine Zeile', group && group.call === '2 Dateien gelesen, 1 Ordner angesehen' && group.subs === 3, JSON.stringify(group));
  await shoot('7-kompakt');

  // Denkaufwand: Regler im Aufklappfenster (Standard, Tief bis Maximal)
  const slide = (value) => shell(`(() => { document.getElementById('think-btn').click(); const r = document.getElementById('think-range');
    r.value = '${value}'; r.dispatchEvent(new Event('input')); })()`);
  check('Denkaufwand: Standard voreingestellt', await shell('document.getElementById("think-label").textContent === "Standard"'));
  await slide(3);
  await until('Denken hoch', () => settings.get('thinking') === 'hoch', 5000).catch(() => {});
  await shoot('8-denkaufwand');
  await shell('document.body.click()');
  check('Regler auf «Hoch», Fenster schliesst beim Klick daneben', await shell('document.getElementById("think-label").textContent === "Hoch" && document.getElementById("think-pop").hidden'));
  await order(shell, agent, 'Bitte lies gründlich');
  let log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Denken «Hoch»: reasoning_effort high, Qwen-Signale und /think', log.effort === 'high' && log.think === true && log.enableThinking === true && log.system.trimEnd().endsWith('/think'), JSON.stringify({ ...log, system: log.system.slice(-20) }));
  await slide(1);
  await until('Denken tief', () => settings.get('thinking') === 'tief', 5000).catch(() => {});
  await order(shell, agent, 'Bitte lies kurz');
  log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Denken «Tief»: reasoning_effort low, kein /think', log.effort === 'low' && log.think === undefined && !log.system.trimEnd().endsWith('/think'), JSON.stringify({ ...log, system: log.system.slice(-20) }));
  await getJson(`${mockUrl}/__mock/think/aus`);
  await order(shell, agent, 'Bitte lies nochmals');
  log = (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1);
  check('Lehnt ASK den Denkaufwand ab, läuft der Auftrag ohne ihn', log.effort === undefined
    && await shell('[...document.querySelectorAll(".msg.assistant .md")].pop().textContent === "Gelesen."'), JSON.stringify({ ...log, system: log.system.slice(-20) }));
  await getJson(`${mockUrl}/__mock/think/an`);
  ctx.ask.noThinkParams = false;
  await shell(`(() => { document.getElementById('think-btn').click(); document.getElementById('think-standard').click(); document.body.click(); })()`);
  await until('Denken Standard', () => settings.get('thinking') === 'standard', 5000).catch(() => {});
  check('Knopf «Standard» setzt zurück', settings.get('thinking') === 'standard');

  // Fertiger Auftrag mit vielen Schritten: zu einer Zeile zusammengeklappt
  await order(shell, agent, 'viele schritte bitte');
  const fold = await shell(`(() => { const f = [...document.querySelectorAll('.msg.fold')].pop(); if (!f) return null;
    const hidden = [...document.querySelectorAll('.msg.tool.folded')].every((n) => n.offsetParent === null);
    const answer = [...document.querySelectorAll('.msg.assistant .md')].pop().textContent; f.click();
    return { text: f.textContent, hidden, answer, open: [...document.querySelectorAll('.msg.tool.folded')].length === 0 }; })()`);
  check('Viele Schritte eingeklappt, Antwort sichtbar, Klick klappt auf', fold && /5 Arbeitsschritte/.test(fold.text) && fold.hidden
    && fold.answer === 'Viele erledigt.' && fold.open, JSON.stringify(fold));
  await shoot('9-eingeklappt');

  // /goal: Ziel setzen; das Modell hört zu früh auf, die App stösst an, goal_done beendet
  await order(shell, agent, '/goal Erstelle den Zielbericht');
  const goalLog = (await getJson(`${mockUrl}/__mock/chatlog`)).slice(-3);
  check('/goal: Ziel im Systemprompt, Werkzeug goal_done', goalLog[0].system.includes('Ziel dieses Chats') && goalLog[0].tools.includes('goal_done'), JSON.stringify(goalLog.map((l) => l.tools.slice(-1))));
  check('/goal: nach «Ich fange an:» automatisch weiter', await shell('[...document.querySelectorAll(".msg.notice.auto")].some((n) => n.textContent.includes("Weiter am Ziel"))'));
  check('/goal: Ziel erreicht und gespeichert', agent.chat.goal?.done === true && agent.chat.goal.summary === 'Bericht steht.', JSON.stringify(agent.chat.goal));
  check('/goal: Ziel-Leiste zeigt «✓ erreicht»', await shell(`(() => { const b = document.getElementById('goal-bar');
    return !b.hidden && document.getElementById('goal-text').textContent === 'Erstelle den Zielbericht' && document.getElementById('goal-state').textContent === '✓ erreicht'; })()`));
  await shoot('10-ziel');
  await shell(`(() => { const i = document.getElementById('input'); i.value = '/goal aus'; i.dispatchEvent(new Event('input')); document.getElementById('send-btn').click(); })()`);
  await until('Ziel weg', () => !agent.chat.goal, 5000).catch(() => {});
  check('/goal aus hebt das Ziel auf', !agent.chat.goal && await shell('document.getElementById("goal-bar").hidden'));

  // Schleifenschutz
  const before = (await getJson(`${mockUrl}/__mock/chatlog`)).length;
  await order(shell, agent, 'schleife bitte');
  const loops = (await getJson(`${mockUrl}/__mock/chatlog`)).length - before;
  check('Schleifenschutz hält nach 3 gleichen Schritten an', loops === 3
    && await shell('[...document.querySelectorAll(".msg.notice")].some((n) => n.textContent.includes("3-mal hintereinander genau denselben Schritt"))'), `${loops} Anfragen`);
  check('Skriptordner pro Chat im Systemprompt', (await getJson(`${mockUrl}/__mock/chatlog`)).at(-1).system.includes(`.ask/skripte/${agent.chat.id}/`));
}

module.exports = async function run(ctx) {
  const { app, appWindow, terminal, ask, settings } = ctx;
  const report = { scenario: process.env.ASK_TEST_SCENARIO, checks: [] };
  const shots = process.env.ASK_TEST_SHOTS;
  const check = (name, ok, detail = '') => report.checks.push({ name, ok: !!ok, detail });
  const shell = (js) => appWindow.win.webContents.executeJavaScript(js);
  const shoot = async (name) => {
    if (!shots) return;
    fs.mkdirSync(shots, { recursive: true });
    await wait(500);
    fs.writeFileSync(path.join(shots, `${name}-shell.png`), (await appWindow.win.webContents.capturePage()).toPNG());
    if (appWindow.view.getVisible()) {
      fs.writeFileSync(path.join(shots, `${name}-ask.png`), (await appWindow.view.webContents.capturePage()).toPNG());
    }
  };

  try {
    const scenario = process.env.ASK_TEST_SCENARIO;
    if (scenario === 'erststart') {
      await until('Oberfläche', () => shell('!!document.querySelector("#dlg-warning:not([hidden])")'));
      check('Ersthinweis wird angezeigt', true);
      check('ASK-Seite ist während Dialog verborgen', !appWindow.view.getVisible());
      await shoot('1-ersthinweis');
      await shell('document.getElementById("warning-check").click(); document.getElementById("warning-ok").click()');
      // Die Adresse von ASK ist in resources/app-config.json eingebaut: kein Dialog «Adresse von ASK».
      const builtIn = settings.appConfig().askUrl;
      await until('Startdialog', () => shell('!!document.querySelector("#dlg-start:not([hidden])")'), 10000).catch(() => {});
      check('Eingebaute ASK-Adresse, kein Adress-Dialog', /^https:\/\//.test(builtIn) && settings.get('askUrl') === builtIn
        && await shell('document.getElementById("dlg-url").hidden'), builtIn);
      const http = await shell('window.ask.setAskUrl("http://unsicher.example")');
      check('http-Adresse wird abgelehnt', http.ok === false, JSON.stringify(http));
      // https-Adressen gibt es lokal nicht; die Test-Adresse wird direkt gesetzt.
      settings.set({ askUrl: process.env.ASK_TEST_ASK_URL });
      ask.setBase(process.env.ASK_TEST_ASK_URL);
      appWindow.loadAsk(process.env.ASK_TEST_ASK_URL);
      ctx.pushState();
      await until('Startdialog', () => shell('!!document.querySelector("#dlg-start:not([hidden])")'));
      check('Startdialog ohne Ordner', true);
      await shoot('3-start');
      const blocked = await ctx.openFolder(process.env.OneDrive || process.env.USERPROFILE);
      check('OneDrive bzw. Profil wird abgelehnt', !blocked.ok, blocked.reason);
      return;
    }

    await until('Terminal läuft', () => terminal.state === 'läuft' || (terminal.state === 'fehler' && terminal.error), 90000);
    check('Terminal läuft', terminal.state === 'läuft', terminal.error);
    check('Terminal mit Low Integrity', terminal.info?.low_integrity === true, JSON.stringify(terminal.info));

    if (scenario === 'anmelden') {
      await until('nicht angemeldet', () => ask.status.state === 'nicht-angemeldet', 30000);
      check('Status «nicht angemeldet»', true);
      await until('Anmeldeseite sichtbar', () => appWindow.view.getVisible(), 5000).catch(() => {});
      check('ASK-Anmeldeseite wird angezeigt', appWindow.view.getVisible());
      check('Eingabe gesperrt mit Hinweis', await shell('document.getElementById("input").placeholder.includes("anmelden")'));
      await shoot('anmelden');
      // Zwischen Anmeldung und fertiger Prüfung (Mock antwortet 1,5 s langsam) darf die Seite nicht sichtbar sein.
      const seen = [];
      const clicked = Date.now();
      await appWindow.view.webContents.executeJavaScript('document.getElementById("login").click()');
      await until('Prüfung beginnt', () => ask.status.state === 'pruefen', 5000)
        .then(() => check('Nach der Anmeldung sofort «Anmeldung wird geprüft»', Date.now() - clicked < 1500, `${Date.now() - clicked} ms`))
        .catch(() => check('Nach der Anmeldung sofort «Anmeldung wird geprüft»', false, ask.status.state));
      await wait(150);
      await shoot('anmelden-pruefen');
      check('Ladebildschirm statt ASK-Seite', await shell('document.getElementById("login-area").classList.contains("loading") && document.getElementById("ask-placeholder").textContent.includes("Anmeldung wird geprüft")'));
      while (ask.status.state !== 'angemeldet' && Date.now() - clicked < 20000) {
        seen.push(appWindow.view.getVisible());
        await wait(50);
      }
      check('Nach der Anmeldung: angemeldet', ask.status.state === 'angemeldet');
      check('Während der Prüfung war die ASK-Seite nie sichtbar', seen.length > 10 && !seen.includes(true), `${seen.length} Proben, sichtbar: ${seen.filter(Boolean).length}`);
      await until('Seite verborgen', () => !appWindow.view.getVisible(), 5000).catch(() => {});
      check('Nach der Anmeldung: ASK-Seite verborgen', !appWindow.view.getVisible());
      check('Nach der Anmeldung: Ladebildschirm weg', await until('Chat sichtbar', () => shell('document.getElementById("login-area").hidden'), 5000).catch(() => false));
      return;
    }

    if (scenario === 'coworker') {
      await coworker(ctx, check, shell, shoot, settings.get('folder'));
      return;
    }

    await chat(ctx, check, shell, shoot);
  } catch (err) {
    check('Ablauf', false, err.stack);
    await shoot('fehler');
  } finally {
    fs.writeFileSync(process.env.ASK_TEST_REPORT, JSON.stringify(report, null, 2));
    if (process.env.ASK_TEST_KEEP !== '1') app.quit();
  }
};
