// Die CSP in index.html und die von Tauri gelten beide (Tauri hängt ein zweites meta an). Jede Quelle, die
// tauri.conf.json erlaubt, muss darum auch in index.html stehen (ENTSCHEIDE E74).
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');

test('CSP in index.html erlaubt alles, was tauri.conf.json erlaubt', () => {
  const conf = JSON.parse(fs.readFileSync('src-tauri/tauri.conf.json', 'utf8')).app.security.csp;
  const html = fs.readFileSync('src/renderer/index.html', 'utf8');
  const meta = /http-equiv="Content-Security-Policy"\s+content="([^"]+)"/.exec(html)[1];
  const dirs = Object.fromEntries(meta.split(';').map((d) => d.trim().split(/\s+/)).filter((d) => d[0]).map(([k, ...v]) => [k, v]));
  for (const [dir, sources] of Object.entries(conf)) {
    const have = dirs[dir] || dirs['default-src'];
    for (const src of sources.split(/\s+/)) assert.ok(have.includes(src), `${dir}: ${src} fehlt in index.html`);
  }
});
