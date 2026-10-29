// Nachbildung der Open-WebUI-Endpunkte, die ASK braucht. Nur für Tests.
// Aufruf: node mock-ask.js <port> [angemeldet|abgemeldet]
// Das «Modell» spielt feste Abläufe und streamt sie wie Open WebUI (SSE im OpenAI-Format):
//   Standard:  Befehl ausführen → Aufgaben und Datei schreiben → Schlusstext mit Denktext
//   «textmodus»: Werkzeugaufruf als <tool_call> im Text
//   «langsam»: langer Befehl, zum Stoppen
// /__mock/tools/aus lässt Anfragen mit «tools» mit HTTP 400 scheitern, /__mock/usage/aus solche mit
// «stream_options». Sonst kommen am Ende des Stroms Tokenzahlen (usage), wenn sie verlangt sind.
const http = require('node:http');
const { createMockGitHub } = require('./mock-github');

const github = createMockGitHub(); // GitHub-API unter /github-api

const port = Number(process.argv[2] || 18900);
const mode = process.argv[3] || 'angemeldet';
const TOKEN = 'test-token';
const log = [];
const chatLog = [];
const searches = []; // Anfragen an die Websuche von ASK
let toolsAccepted = true;
let usageAccepted = true;
let thinkAccepted = true;
let visionAccepted = true;
// Text einer Nachricht; mit Bildern ist content eine Liste von Teilen
const textOf = (content) => (Array.isArray(content) ? content.filter((p) => p.type === 'text').map((p) => p.text).join('\n') : String(content ?? ''));
const imagesIn = (messages) => messages.flatMap((m) => (Array.isArray(m.content) ? m.content.filter((p) => p.type === 'image_url') : [])).length;

const page = `<!doctype html><html><head><meta charset="utf-8"><title>Mock ASK</title></head>
<body style="font-family:Segoe UI;padding:20px"><h1>Mock ASK</h1>
<button id="login" style="font-size:18px;padding:8px 16px">Anmelden</button>
<script>
if (${JSON.stringify(mode)} === 'angemeldet') localStorage.token = localStorage.token || '${TOKEN}';
document.getElementById('login').onclick = () => { localStorage.token = '${TOKEN}'; location.href = '/'; };
</script></body></html>`;

function json(res, status, body) {
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(body));
}

const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const chunk = (delta, finish = null) => ({ id: 'mock', object: 'chat.completion.chunk', choices: [{ index: 0, delta, finish_reason: finish }] });

function toolCallChunks(calls) {
  const out = [];
  calls.forEach((c, index) => {
    const args = JSON.stringify(c.args);
    const half = Math.ceil(args.length / 2);
    out.push(chunk({ tool_calls: [{ index, id: `call_${index}_${Date.now()}`, type: 'function', function: { name: c.name, arguments: '' } }] }));
    out.push(chunk({ tool_calls: [{ index, function: { arguments: args.slice(0, half) } }] }));
    out.push(chunk({ tool_calls: [{ index, function: { arguments: args.slice(half) } }] }));
  });
  out.push(chunk({}, 'tool_calls'));
  return out;
}

function textChunks(text, reasoning = '') {
  const out = [];
  if (reasoning) out.push(chunk({ reasoning_content: reasoning }));
  for (let i = 0; i < text.length; i += 7) out.push(chunk({ content: text.slice(i, i + 7) }));
  out.push(chunk({}, 'stop'));
  return out;
}

// Antwort des Modells aus dem bisherigen Verlauf
function script(messages) {
  const lastUser = messages.map((m, i) => (m.role === 'user' && !/^(<tool_response>|\[Ansicht\])/.test(textOf(m.content)) ? i : -1))
    .filter((i) => i >= 0).pop();
  const text = textOf(messages[lastUser]?.content).toLowerCase();
  const after = messages.slice(lastUser + 1);
  const results = after.filter((m) => m.role === 'tool' || textOf(m.content).startsWith('<tool_response>')).length;
  if (text.includes('textmodus')) {
    if (results === 0) return textChunks('Ich führe den Befehl aus.\n<tool_call>\n{"name": "run_command", "arguments": {"command": "echo textmodus ok"}}\n</tool_call>');
    return textChunks('Textmodus fertig.');
  }
  if (text.includes('ziel ist noch nicht erreicht')) {
    if (results === 0) return toolCallChunks([{ name: 'goal_done', args: { zusammenfassung: 'Bericht steht.' } }]);
    return textChunks('Ziel erledigt.');
  }
  if (text.includes('zielbericht')) return textChunks('Ich fange an:');
  if (text.includes('viele schritte')) {
    if (results < 5) return toolCallChunks([{ name: 'run_command', args: { command: `echo schritt ${results + 1}` } }]);
    return textChunks('Viele erledigt.');
  }
  if (text.includes('auf github anlegen')) {
    if (results === 0) return toolCallChunks([{ name: 'github_create', args: { name: 'testprojekt', beschreibung: 'Aus dem Test' } }]);
    return textChunks('Das Projekt liegt jetzt auf GitHub.');
  }
  if (text.includes('auf github hochladen')) {
    if (results === 0) return toolCallChunks([{ name: 'github_push', args: { nachricht: 'Bericht ergänzt' } }]);
    return textChunks('Hochgeladen.');
  }
  if (text.includes('von github holen')) {
    if (results === 0) return toolCallChunks([{ name: 'github_pull', args: {} }]);
    return textChunks('Geholt.');
  }
  if (text.includes('bild an')) return textChunks('Ich sehe das Bild.');
  if (text.includes('recherchiere')) {
    if (results === 0) return toolCallChunks([{ name: 'web_search', args: { anfrage: 'proxmox 9 neuerungen' } }]);
    if (results === 1) return toolCallChunks([{ name: 'web_fetch', args: { url: `http://127.0.0.1:${port}/__mock/seite` } }]);
    return textChunks('Laut der Testseite bringt Proxmox VE 9 einen neuen Kernel.');
  }
  if (text.includes('ansehen test')) {
    if (results === 0) return toolCallChunks([{ name: 'view_file', args: { path: 'grafik.png' } }]);
    return textChunks('Angesehen.');
  }
  if (text.includes('spiel testen')) {
    if (results === 0) return toolCallChunks([{ name: 'browser_test', args: { path: 'spiel.html', aktionen: [{ klick: '#start' }, { taste: 'ArrowRight' }] } }]);
    return textChunks('Getestet.');
  }
  if (text.includes('ändere den bericht')) {
    if (results === 0) {
      return toolCallChunks([{ name: 'write_file', args: { path: 'ausgabe/bericht.txt', content: 'Vom Modell geändert' } },
        { name: 'write_file', args: { path: 'projekt/neu.txt', content: 'neu' } }]);
    }
    return textChunks('Geändert.');
  }
  if (text.includes('anhang')) {
    const file = messages.map((m) => textOf(m.content)).join('\n').match(/\.ask\/anhaenge\/[\w-]+\/notiz\.txt/)?.[0];
    if (results === 0) return toolCallChunks([{ name: 'read_file', args: { path: file || 'notiz.txt' } }]);
    return textChunks('Anhang gelesen.');
  }
  if (text.includes('schleife')) return toolCallChunks([{ name: 'run_command', args: { command: 'echo immer gleich' } }]);
  if (text.includes('lies')) {
    if (results === 0) {
      return toolCallChunks([{ name: 'read_file', args: { path: 'ausgabe/bericht.txt' } }, { name: 'read_file', args: { path: '.ask/skills/word/SKILL.md' } },
        { name: 'list_files', args: { path: '.' } }]);
    }
    return textChunks('Gelesen.');
  }
  if (text.includes('websuche')) {
    if (results === 0) return toolCallChunks([{ name: 'run_command', args: { command: 'echo nach der Suche' } }]);
    return textChunks('Laut der Proxmox-Webseite ist **Proxmox VE 8.3** aktuell.\n\n```bash\npvesh get /version\n```');
  }
  if (text.includes('langsam')) {
    if (results === 0) return toolCallChunks([{ name: 'run_command', args: { command: 'ping -n 40 127.0.0.1' } }]);
    return textChunks('Der Befehl ist fertig.');
  }
  if (results === 0) return toolCallChunks([{ name: 'run_command', args: { command: 'echo hallo aus ASK' } }]);
  if (results === 1) {
    return toolCallChunks([
      { name: 'update_tasks', args: { auftrag: 'Testauftrag', aufgaben: [{ titel: 'Befehl testen', status: 'erledigt' }, { titel: 'Bericht schreiben', status: 'in_arbeit' }, { titel: 'Prüfen', status: 'offen' }] } },
      { name: 'write_file', args: { path: 'ausgabe/bericht.txt', content: 'Inhalt des Berichts' } },
    ]);
  }
  return textChunks('<think>Alles erledigt.</think>Fertig. Die Datei liegt in `ausgabe/bericht.txt`.\n\n- Befehl getestet\n- **Bericht** geschrieben', 'Ich prüfe die Ergebnisse.');
}

http.createServer((req, res) => {
  log.push(`${req.method} ${req.url}`);
  if (req.url.startsWith('/github-api/')) {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      const r = github.handle(req.method, req.url.slice('/github-api'.length), req.headers, body ? JSON.parse(body) : null);
      json(res, r.status, r.body);
    });
    return undefined;
  }
  if (req.url.startsWith('/__mock/github/')) {
    // Für Tests: Datei auf GitHub lesen (/__mock/github/datei?repo=…&path=…) oder dort ändern (…/aendern)
    const u = new URL(req.url, 'http://x');
    if (u.pathname.endsWith('/aendern')) {
      github.commitOnGitHub(u.searchParams.get('repo'), u.searchParams.get('path'), u.searchParams.get('text'));
      return json(res, 200, {});
    }
    return json(res, 200, { text: github.fileOnGitHub(u.searchParams.get('repo'), u.searchParams.get('path')),
      commits: github.commitCount(u.searchParams.get('repo')) });
  }
  const authed = req.headers.authorization === `Bearer ${TOKEN}`;
  const url = req.url.split('?')[0];
  if (url === '/' || url === '/auth') {
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    return res.end(page);
  }
  if (url === '/api/config') return json(res, 200, { version: '0.12.3', features: authed ? { enable_web_search: true } : {} });
  if (url === '/__mock/seite') {
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    return res.end('<!doctype html><html><head><title>Proxmox VE 9 Testseite</title><script>var geheim = 1;</script></head><body>'
      + '<nav>Menü Start Produkte</nav><main><h1>Proxmox VE 9</h1><p>Proxmox VE 9 bringt einen neuen Kernel &amp; bessere Backups.</p>'
      + '<ul><li>Kernel 6.14</li><li>ZFS 2.3</li></ul>' + '<p>Mehr Text. </p>'.repeat(60) + '</main><footer>Impressum</footer></body></html>');
  }
  if (url === '/__mock/searches') return json(res, 200, searches);
  if (url === '/__mock/log') return json(res, 200, log);
  if (url === '/__mock/chatlog') return json(res, 200, chatLog);
  if (url.startsWith('/__mock/tools/')) {
    toolsAccepted = url.endsWith('/an');
    return json(res, 200, { toolsAccepted });
  }
  if (url.startsWith('/__mock/think/')) {
    thinkAccepted = url.endsWith('/an');
    return json(res, 200, { thinkAccepted });
  }
  if (url.startsWith('/__mock/vision/')) {
    visionAccepted = url.endsWith('/an');
    return json(res, 200, { visionAccepted });
  }
  if (url.startsWith('/__mock/usage/')) {
    usageAccepted = url.endsWith('/an');
    return json(res, 200, { usageAccepted });
  }
  if (!authed) return json(res, 401, { detail: 'Not authenticated' });
  if (url === '/api/v1/auths/') {
    return json(res, 200, { name: 'Lea Test', email: 'lea@example.ch', role: 'user', permissions: { features: { web_search: true } } });
  }
  if (url === '/api/v1/retrieval/process/web/search' && req.method === 'POST') {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      const q = JSON.parse(body || '{}');
      searches.push(q);
      json(res, 200, { status: true, collection_names: ['web-x'], filenames: [], items: [
        { link: `http://127.0.0.1:${port}/__mock/seite`, title: 'Proxmox VE 9 Testseite', snippet: 'Neuer Kernel und bessere Backups' },
        { link: 'https://pve.proxmox.com/wiki/Roadmap', title: 'Roadmap – Proxmox VE', snippet: 'Geplante Funktionen' }] });
    });
    return undefined;
  }
  if (url === '/api/models') {
    // Nach der Anmeldung langsam antworten, damit der Test die Phase «Anmeldung wird geprüft» sieht
    if (mode === 'abgemeldet') {
      setTimeout(() => json(res, 200, { data: [{ id: 'qwen-test', name: 'Qwen Test' }] }), 1500);
      return undefined;
    }
    return json(res, 200, { data: [{ id: 'qwen-test', name: 'Qwen Test', info: { meta: { capabilities: { vision: true } } } }, { id: 'versteckt', name: 'Versteckt', info: { meta: { hidden: true } } }] });
  }
  if (url === '/api/chat/completions' && req.method === 'POST') {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', async () => {
      const data = JSON.parse(body);
      chatLog.push({ model: data.model, stream: data.stream, usage: !!data.stream_options?.include_usage, web: !!data.features?.web_search,
        effort: data.reasoning_effort, think: data.think, enableThinking: data.chat_template_kwargs?.enable_thinking, tools: (data.tools || []).map((t) => t.function.name),
        system: data.messages[0]?.content || '', roles: data.messages.map((m) => m.role),
        toolIds: data.messages.filter((m) => m.role === 'tool').map((m) => m.tool_call_id),
        images: imagesIn(data.messages), lastImages: imagesIn(data.messages.slice(-1)), lastUser: textOf(data.messages.filter((m) => m.role === 'user').pop()?.content) });
      if (data.stream_options && !usageAccepted) return json(res, 400, { detail: 'Unrecognized request argument: stream_options' });
      if (data.reasoning_effort !== undefined && !thinkAccepted) return json(res, 400, { detail: 'Unrecognized request argument: reasoning_effort' });
      if (imagesIn(data.messages) && !visionAccepted) return json(res, 400, { detail: 'image input is not supported by this model' });
      if (data.tools && !toolsAccepted) return json(res, 400, { detail: 'tools are not supported by this model' });
      res.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' });
      const chunks = script(data.messages);
      if (data.features?.web_search) {
        // Wie Open WebUI: Quellen der Websuche als eigenes Ereignis vor der Antwort
        res.write(`data: ${JSON.stringify({ sources: [{ source: { type: 'web_search', name: 'proxmox 8', urls: ['https://www.proxmox.com/de/news', 'https://pve.proxmox.com/wiki/Roadmap'] },
          document: ['Proxmox VE 8.3 ist erschienen.', 'Roadmap'], metadata: [{ source: 'https://www.proxmox.com/de/news', title: 'Proxmox News' },
            { source: 'https://pve.proxmox.com/wiki/Roadmap', title: 'Roadmap – Proxmox VE' }] }] })}\n\n`);
      }
      for (const c of chunks) {
        res.write(`data: ${JSON.stringify(c)}\n\n`);
        await wait(15);
      }
      if (data.stream_options?.include_usage) {
        const usage = { prompt_tokens: Math.round(body.length / 4), completion_tokens: chunks.length * 5, total_tokens: 0 };
        res.write(`data: ${JSON.stringify({ id: 'mock', choices: [], usage })}\n\n`);
      }
      res.end('data: [DONE]\n\n');
    });
    return undefined;
  }
  return json(res, 404, { detail: 'not found' });
}).listen(port, '127.0.0.1', () => console.log(`Mock-ASK auf http://127.0.0.1:${port} (${mode})`));
