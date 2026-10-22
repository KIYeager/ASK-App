// Nachbildung der Open-WebUI-Endpunkte, die ASK Desktop braucht. Nur für Tests.
// Aufruf: node mock-ask.js <port> [frei|gesperrt]
const http = require('node:http');

const port = Number(process.argv[2] || 18900);
let permitted = (process.argv[3] || 'frei') === 'frei';
const TOKEN = 'test-token';
const userSettings = { ui: { terminalServers: [{ url: 'http://127.0.0.1:9', key: 'alt', name: 'Eigenes', enabled: true }] } };
const log = [];
let pageResults = {};

// Die Seite verhält sich vereinfacht wie Open WebUI: Sie liest die eingerichteten Terminals und
// ruft beim ausgewählten Terminal openapi.json, /system und /files/cwd mit dem gespeicherten Key ab.
const page = `<!doctype html><html><head><meta charset="utf-8"><title>Mock ASK</title></head>
<body style="font-family:Segoe UI;padding:20px"><h1>Mock ASK</h1><pre id="out">…</pre><script>
localStorage.token = localStorage.token || '${TOKEN}';
window.__results = {};
// Wie Open WebUI: Werkzeuge des Terminals nur einmal beim Laden der Seite holen.
let firstLoad = 'kein Terminal eingerichtet';
const firstLoadDone = (async () => {
  const s = await (await fetch('/api/v1/users/user/settings', { headers: { Authorization: 'Bearer ' + localStorage.token } })).json();
  const t = (s.ui.terminalServers || []).find((x) => x.url === localStorage.selectedTerminalId && x.enabled);
  if (!t) return;
  try { firstLoad = (await fetch(t.url + '/openapi.json', { headers: { Authorization: 'Bearer ' + t.key } })).status; }
  catch (e) { firstLoad = 'FEHLER ' + e.message; }
})();
async function poll() {
  await firstLoadDone;
  try {
    const s = await (await fetch('/api/v1/users/user/settings', { headers: { Authorization: 'Bearer ' + localStorage.token } })).json();
    const sel = localStorage.selectedTerminalId;
    const t = (s.ui.terminalServers || []).find((x) => x.url === sel && x.enabled);
    const r = { selected: sel || null, configured: !!t, firstLoad };
    if (t) {
      const h = { Authorization: 'Bearer ' + t.key };
      for (const p of ['/openapi.json', '/system', '/files/cwd']) {
        try {
          const res = await fetch(t.url + p, { headers: h });
          r[p] = res.status;
          if (p === '/system' && res.ok) r.systemText = (await res.json()).prompt;
        } catch (e) { r[p] = 'FEHLER ' + e.message; }
      }
      try {
        const res = await fetch(t.url + '/execute?wait=10', { method: 'POST', headers: { ...h, 'Content-Type': 'application/json' },
          body: JSON.stringify({ command: 'echo hallo aus ASK' }) });
        r.execute = res.status + ' ' + JSON.stringify((await res.json()).output).slice(0, 120);
      } catch (e) { r.execute = 'FEHLER ' + e.message; }
    }
    window.__results = r;
    document.getElementById('out').textContent = JSON.stringify(r, null, 2);
    fetch('/__mock/results', { method: 'POST', body: JSON.stringify(r) });
  } catch (e) { document.getElementById('out').textContent = 'Fehler ' + e; }
  setTimeout(poll, 2000);
}
poll();
</script></body></html>`;

function json(res, status, body) {
  res.writeHead(status, { 'Content-Type': 'application/json' });
  res.end(JSON.stringify(body));
}

http.createServer((req, res) => {
  log.push(`${req.method} ${req.url}`);
  const authed = req.headers.authorization === `Bearer ${TOKEN}`;
  if (req.url === '/' || req.url.startsWith('/?')) {
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    return res.end(page);
  }
  if (req.url === '/api/config') {
    return json(res, 200, { version: '0.12.3', features: { enable_tool_servers: true, enable_direct_integrations: true } });
  }
  if (req.url === '/__mock/settings') return json(res, 200, userSettings);
  if (req.url === '/__mock/results' && req.method === 'POST') {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => { pageResults = JSON.parse(body || '{}'); json(res, 200, {}); });
    return undefined;
  }
  if (req.url === '/__mock/results') return json(res, 200, pageResults);
  if (req.url === '/__mock/reset') { pageResults = {}; return json(res, 200, {}); }
  if (req.url === '/__mock/log') return json(res, 200, log);
  if (req.url.startsWith('/__mock/permit/')) {
    permitted = req.url.endsWith('/ja');
    return json(res, 200, { permitted });
  }
  if (!authed) return json(res, 401, { detail: 'Not authenticated' });
  if (req.url === '/api/v1/auths/') {
    return json(res, 200, { role: 'user', permissions: { features: { direct_tool_servers: permitted } } });
  }
  if (req.url === '/api/v1/users/user/settings') return json(res, 200, userSettings);
  if (req.url === '/api/v1/users/user/settings/update' && req.method === 'POST') {
    let body = '';
    req.on('data', (c) => { body += c; });
    req.on('end', () => {
      const update = JSON.parse(body);
      Object.assign(userSettings.ui, update.ui || {});
      json(res, 200, userSettings);
    });
    return undefined;
  }
  return json(res, 404, { detail: 'not found' });
}).listen(port, '127.0.0.1', () => console.log(`Mock-ASK auf http://127.0.0.1:${port}`));
