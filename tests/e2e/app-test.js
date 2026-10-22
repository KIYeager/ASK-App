// End-to-End-Test im Entwicklungsmodus. Wird über ASK_TEST_SCRIPT in den Hauptprozess geladen.
// Umgebung: ASK_TEST_REPORT (Pfad für das Ergebnis), ASK_TEST_SCENARIO (verbinden|gesperrt|erststart),
// ASK_TEST_SHOTS (Ordner für Bildschirmfotos).
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');

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

module.exports = async function run(ctx) {
  const { app, appWindow, terminal, connection, settings } = ctx;
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
      check('ASK-Ansicht ist während Dialog verborgen', !appWindow.view.getVisible());
      await shoot('1-ersthinweis');
      await shell('document.getElementById("warning-check").click(); document.getElementById("warning-ok").click()');
      await until('URL-Dialog', () => shell('!!document.querySelector("#dlg-url:not([hidden])")'));
      await shoot('2-adresse');
      await shell('document.getElementById("url-input").value = "http://unsicher.example"; document.getElementById("url-ok").click()');
      await wait(500);
      check('http-Adresse wird abgelehnt', await shell('!document.getElementById("url-error").hidden'));
      // https-Adressen gibt es lokal nicht; die Test-Adresse wird direkt gesetzt.
      settings.set({ askUrl: process.env.ASK_TEST_ASK_URL });
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

    if (scenario === 'oeffentlich') {
      // Echte öffentliche HTTPS-Seite als ASK-Ersatz: prüft Mixed Content, CORS, Local Network Access
      // und das Einsetzen des Keys in der fertigen App.
      await until('Seite geladen', () => !appWindow.view.webContents.isLoading()
        && appWindow.view.webContents.getURL().startsWith('https://'), 30000);
      const url = terminal.url();
      const before = await connection.check();
      check('Status ohne Anmeldung «nicht angemeldet»', before.state === 'nicht-angemeldet', JSON.stringify(before));
      const r = await appWindow.view.webContents.executeJavaScript(`(async () => {
        const out = {};
        const h = { Authorization: 'Bearer ask-desktop' };
        out.config = (await fetch('${url}/api/config')).status;
        out.cwd = (await fetch('${url}/files/cwd', { headers: h })).status;
        out.ohneKey = (await fetch('${url}/files/cwd')).status;
        const ex = await fetch('${url}/execute?wait=10', { method: 'POST', headers: { ...h, 'Content-Type': 'application/json' },
          body: JSON.stringify({ command: 'echo öffentlich' }) });
        out.execute = ex.status + ' ' + JSON.stringify((await ex.json()).output);
        out.ws = await new Promise((resolve) => {
          fetch('${url}/api/terminals', { method: 'POST', headers: h }).then((res) => res.json()).then((t) => {
            const ws = new WebSocket('${url.replace('http', 'ws')}/api/terminals/' + t.id);
            let got = '';
            ws.onopen = () => { ws.send(JSON.stringify({ type: 'auth', token: 'ask-desktop' }));
              ws.send(new TextEncoder().encode('echo wsok\\r\\n')); };
            ws.onmessage = async (m) => { got += typeof m.data === 'string' ? m.data : await m.data.text();
              if (got.includes('wsok')) { ws.close(); resolve('ok'); } };
            ws.onclose = (e) => resolve(got.includes('wsok') ? 'ok' : 'geschlossen ' + e.code);
            setTimeout(() => resolve('zeit'), 8000);
          }).catch((e) => resolve('fehler ' + e));
        });
        return out;
      })()`);
      check('HTTPS-Seite erreicht /api/config', r.config === 200, JSON.stringify(r));
      check('HTTPS-Seite mit Platzhalter-Key: 200', r.cwd === 200, JSON.stringify(r));
      check('Ohne Authorization-Header: 401', r.ohneKey === 401, JSON.stringify(r));
      check('Befehl von HTTPS-Seite', String(r.execute).includes('öffentlich'), r.execute);
      check('Interaktives Terminal über WebSocket', r.ws === 'ok', r.ws);
      return;
    }

    if (scenario === 'gesperrt') {
      await until('Status nicht-freigegeben', () => connection.status.state === 'nicht-freigegeben', 40000);
      check('Status «nicht freigegeben»', true);
      const s = await getJson(`${process.env.ASK_TEST_ASK_URL}/__mock/settings`);
      check('Keine Einstellung geschrieben', !s.ui.terminalServers.some((t) => t.name === 'ASK Desktop'), JSON.stringify(s));
      await shell('document.getElementById("ask-pill").click()');
      await wait(400);
      await shoot('gesperrt-dialog');
      return;
    }

    await until('verbunden', () => connection.status.state === 'verbunden', 60000);
    check('Status «verbunden»', true);
    const s = await getJson(`${process.env.ASK_TEST_ASK_URL}/__mock/settings`);
    const entries = s.ui.terminalServers;
    const ours = entries.find((t) => t.name === 'ASK Desktop');
    check('Eintrag in ASK geschrieben', ours && ours.enabled && ours.url === terminal.url(), JSON.stringify(entries));
    check('Nur Platzhalter-Key in ASK', ours?.key === 'ask-desktop');
    check('Eigenes Terminal deaktiviert, nicht gelöscht', entries.some((t) => t.name === 'Eigenes' && !t.enabled));
    const results = await until('Ergebnisse der Seite', async () => {
      const r = await appWindow.view.webContents.executeJavaScript('window.__results');
      return r && r.execute ? r : null;
    });
    check('openapi.json mit eingesetztem Key', results['/openapi.json'] === 200, JSON.stringify(results));
    check('/files/cwd mit eingesetztem Key', results['/files/cwd'] === 200);
    check('Befehl aus der Seite ausgeführt', String(results.execute).includes('hallo aus ASK'), results.execute);

    // Seitenleiste: tasks.json und ausgabe/ live
    const folder = settings.get('folder');
    fs.writeFileSync(path.join(folder, '.ask', 'tasks.json'), JSON.stringify({ auftrag: 'Testauftrag', aufgaben: [
      { titel: 'Daten lesen', status: 'erledigt' }, { titel: 'Tabelle bauen', status: 'in_arbeit' },
      { titel: 'Prüfen', status: 'offen' }] }));
    await until('Aufgaben in der Seitenleiste', () => shell('document.querySelectorAll("#tasks-list li").length === 3'), 10000);
    check('Aufgabenliste live', true);
    check('Fortschritt «1 von 3»', (await shell('document.getElementById("tasks-count").textContent')) === '1 von 3');
    fs.writeFileSync(path.join(folder, 'ausgabe', 'bericht.txt'), 'Inhalt des Berichts');
    await until('Datei in Ausgabe', () => shell('[...document.querySelectorAll("#output-list li")].some((li) => li.textContent.includes("bericht.txt"))'), 10000);
    check('Neue Datei in «Ausgabe» live', true);
    await shell('[...document.querySelectorAll("#output-list li")].find((li) => li.textContent.includes("bericht.txt")).click()');
    await until('Textvorschau', () => shell('(document.querySelector("#preview-body pre")||{}).textContent === "Inhalt des Berichts"'), 5000);
    check('Textvorschau', true);
    await shoot('verbunden');
  } catch (err) {
    check('Ablauf', false, err.stack);
    await shoot('fehler');
  } finally {
    fs.writeFileSync(process.env.ASK_TEST_REPORT, JSON.stringify(report, null, 2));
    if (process.env.ASK_TEST_KEEP !== '1') app.quit();
  }
};
