// Liest src/preload/shell.js und erzeugt daraus die Tabelle Methode -> Kanal und den Shim für Tauri.
// Die Oberfläche bleibt unverändert: der Shim stellt dasselbe window.ask bereit wie der Electron-Preload.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const src = fs.readFileSync(path.join(repo, 'src/preload/shell.js'), 'utf8');
const table = {};
for (const m of src.matchAll(/^\s*(\w+):\s*(?:on\('([^']+)'\)|\([^)]*\)\s*=>\s*ipcRenderer\.(invoke|send)\('([^']+)')/gm)) {
  const [, name, onChannel, kind, channel] = m;
  table[name] = onChannel ? ['on', onChannel] : [kind, channel];
}
const out = path.join(here, '../app/shim');
fs.mkdirSync(out, { recursive: true });
fs.writeFileSync(path.join(out, 'api-table.json'), JSON.stringify(table, null, 1) + '\n');
const shim = `// Erzeugt von spike/tauri/tools/gen-shim.mjs aus src/preload/shell.js. Nicht von Hand ändern.
// Läuft als initialization_script nur in der Webview "ui" (vor den Skripten der Oberfläche).
(() => {
  if (window.ask) return;
  const TABLE = ${JSON.stringify(table)};
  // Electron überträgt Uint8Array (Anhänge, hineingezogene Dateien) direkt; Tauri nur JSON. Darum als Base64.
  const b64 = (u8) => { let s = ''; for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000)); return btoa(s); };
  const pack = (v) => (v instanceof Uint8Array ? { __bytes: b64(v) } : Array.isArray(v) ? v.map(pack)
    : v && typeof v === 'object' && v.constructor === Object ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, pack(x)])) : v);
  const call = (channel, args) => window.__TAURI__.core.invoke('ask_invoke', { channel, args: pack(args) });
  const api = {};
  for (const [name, [kind, channel]] of Object.entries(TABLE)) {
    if (kind === 'on') {
      api[name] = (cb) => {
        // listen() aus @tauri-apps/api/event (app.withGlobalTauri); gibt wie im Preload eine Abmeldefunktion zurück
        const pending = window.__TAURI__.event.listen(channel, (e) => cb(e.payload));
        return () => { pending.then((unlisten) => unlisten()); };
      };
    } else if (kind === 'send') {
      api[name] = (...args) => { call(channel, args).catch(() => {}); };
    } else {
      api[name] = (...args) => call(channel, args);
    }
  }
  Object.defineProperty(window, 'ask', { value: Object.freeze(api), writable: false });
})();
`;
fs.writeFileSync(path.join(out, 'shim.js'), shim);
console.log(`${Object.keys(table).length} Methoden (${Object.values(table).filter((t) => t[0] === 'on').length} Ereignisse)`);
