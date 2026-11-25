// Testseiten für den Spike, nur auf 127.0.0.1. Ersetzt die echte ASK-Anmeldeseite und eine fremde Webseite,
// damit der Spike keine echten Konten oder Server braucht.
import http from 'node:http';

const port = Number(process.env.SPIKE_PORT || 18765);
const page = (title, body) => `<!doctype html><meta charset="utf-8"><title>${title}</title>
<style>body{font:16px system-ui;margin:24px;background:#fff;color:#123}button,input{font-size:16px;padding:6px 10px;margin:4px 0}</style>${body}`;

const routes = {
  // Attrappe der ASK-Anmeldung: setzt localStorage.token wie Open WebUI nach dem Login
  '/auth': page('ASK Anmeldung (Attrappe)', `<h1>ASK (Attrappe)</h1><p>Anmelden mit Microsoft …</p>
<button id="login" onclick="localStorage.token='spike-token-4711';document.getElementById('s').textContent='angemeldet'">Anmelden</button>
<p id="s">nicht angemeldet</p>
<script>setTimeout(() => document.getElementById('login').click(), 1500)</script>`),
  // Seite für die Fernsteuerung
  '/seite': page('Testseite Browser', `<h1>Testseite für den Browser-Bereich</h1>
<button id="knopf">Klick mich</button> <span id="klick">noch nicht geklickt</span><br>
<input id="feld" placeholder="Name"> <span id="eingabe"></span><br>
<a id="weg" href="http://127.0.0.1:${port}/gesperrt">Link auf gesperrte Adresse</a> ·
<a id="dl" href="/datei.bin">Download</a> ·
<button id="popup" onclick="window.open('https://example.org/','_blank','width=400,height=300')">Popup</button>
<script>
  document.getElementById('knopf').addEventListener('click', (e) => { document.getElementById('klick').textContent = 'geklickt, isTrusted=' + e.isTrusted; });
  document.getElementById('feld').addEventListener('input', (e) => { document.getElementById('eingabe').textContent = 'Eingabe: ' + e.target.value + ', isTrusted=' + e.isTrusted; });
  console.error('Testfehler aus der Seite');
</script>`),
  '/gesperrt': page('Gesperrt', '<h1>Diese Seite hätte nicht laden dürfen</h1>'),
};

http.createServer((req, res) => {
  const url = new URL(req.url, `http://127.0.0.1:${port}`);
  if (url.pathname === '/datei.bin') {
    res.writeHead(200, { 'Content-Type': 'application/octet-stream', 'Content-Disposition': 'attachment; filename="datei.bin"' });
    return res.end(Buffer.alloc(1024, 1));
  }
  const html = routes[url.pathname];
  if (!html) { res.writeHead(404); return res.end('nicht gefunden'); }
  res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
  res.end(html);
}).listen(port, '127.0.0.1', () => console.log(`Testseiten auf http://127.0.0.1:${port}`));
