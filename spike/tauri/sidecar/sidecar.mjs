// Node als Sidecar hinter Tauri (Frage 5, Variante «Node bleibt»).
// Protokoll über stdin/stdout, eine JSON-Zeile pro Nachricht:
//   Tauri -> Sidecar: {"id":1,"channel":"chat:send","args":["Hallo"]}
//   Sidecar -> Tauri: {"id":1,"ok":true,"result":…}  oder  {"event":"chat:event","payload":{…}}
//
// Für den Spike beantwortet die Attrappe der Oberfläche (src/renderer/src/dev/mockApi.js) die meisten Kanäle.
// workspace:list, workspace:files und workspace:import kommen aus dem echten Hauptprozess-Code (src/main/workspace.js), unverändert,
// mit einem kleinen Ersatz für require('electron'). So zeigt der Spike, dass Module aus src/main ohne Electron laufen.
import fs from 'node:fs';
import path from 'node:path';
import readline from 'node:readline';
import Module, { createRequire } from 'node:module';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(here, '../../..');
const table = JSON.parse(fs.readFileSync(path.join(here, '../app/shim/api-table.json'), 'utf8'));
const byChannel = Object.fromEntries(Object.entries(table).map(([name, [kind, ch]]) => [ch, { name, kind }]));
const out = (msg) => process.stdout.write(JSON.stringify(msg) + '\n');
const log = (...a) => process.stderr.write(`[sidecar] ${a.join(' ')}\n`);

// Ersatz für require('electron'): nur was die geladenen Module beim Laden und für list/files brauchen.
const dataDir = process.env.SPIKE_DATA || path.join(here, '.data');
const electronStub = {
  app: { isPackaged: false, getAppPath: () => repo, getPath: (n) => path.join(dataDir, n), getVersion: () => '0.0.0-spike' },
};
const origLoad = Module._load;
Module._load = function load(request, ...rest) {
  if (request === 'electron') return electronStub;
  return origLoad.call(this, request, ...rest);
};

// Attrappe laden (erwartet Browser-Globale)
globalThis.window = globalThis;
globalThis.location = { search: '' };
const mock = await import(pathToFileURL(path.join(repo, 'src/renderer/src/dev/mockApi.js')).href);
mock.install();
const ask = globalThis.ask;

// Echter Code aus src/main
const require = createRequire(import.meta.url);
let workspace = null;
const folder = process.env.SPIKE_FOLDER;
if (folder) {
  try {
    const { Workspace } = require(path.join(repo, 'src/main/workspace.js'));
    workspace = new Workspace();
    workspace.on('files', (files) => out({ event: 'files', payload: files }));
    workspace.open(folder);
    log(`echter Workspace aus src/main/workspace.js offen: ${folder}`);
  } catch (err) {
    log(`Workspace nicht geladen: ${err.stack}`);
  }
}
// Gegenstück zu pack() im Shim: {__bytes: base64} wieder zu Buffer
const unpack = (v) => (v && typeof v === 'object' && typeof v.__bytes === 'string' ? Buffer.from(v.__bytes, 'base64')
  : Array.isArray(v) ? v.map(unpack) : v && typeof v === 'object' ? Object.fromEntries(Object.entries(v).map(([k, x]) => [k, unpack(x)])) : v);

const real = workspace ? {
  'workspace:import': (rel, files) => ({ ok: true, paths: workspace.import(String(rel || ''), files) }),
  'workspace:list': (rel) => workspace.list(String(rel || '')),
  'workspace:files': () => workspace.files(),
} : {};

// Ereignisse der Attrappe weiterreichen
for (const [name, [kind, channel]] of Object.entries(table)) {
  if (kind === 'on' && typeof ask[name] === 'function') ask[name]((payload) => out({ event: channel, payload }));
}

const rl = readline.createInterface({ input: process.stdin });
rl.on('line', async (line) => {
  let msg;
  try { msg = JSON.parse(line); } catch { return; }
  const { id, channel } = msg;
  const args = unpack(msg.args || []);
  try {
    let result;
    if (real[channel]) result = await real[channel](...args);
    else {
      const entry = byChannel[channel];
      const fn = entry && ask[entry.name];
      if (typeof fn !== 'function') throw new Error(`Kanal ${channel} fehlt in der Attrappe`);
      result = await fn(...args);
    }
    out({ id, ok: true, result: result === undefined ? null : result });
  } catch (err) {
    out({ id, ok: false, error: String(err?.message || err) });
  }
});
rl.on('close', () => process.exit(0));
log(`bereit (${Object.keys(table).length} Kanäle, echt: ${Object.keys(real).join(', ') || 'keine'})`);
