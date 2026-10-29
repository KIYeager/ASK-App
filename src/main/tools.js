// Werkzeuge für das Modell (ENTSCHEIDE E19). Ausgeführt werden sie über das lokale Terminal
// (open-terminal mit Low Integrity und Ordnersperre) mit dem echten Key, den nur der Hauptprozess
// kennt. Jedes Werkzeug liefert den Text für das Modell und eine kurze Anzeige für die Oberfläche.
const http = require('node:http');

const MAX_OUTPUT = 12000; // Zeichen pro Ergebnis an das Modell
const POLL_SECONDS = 2;

function request(terminal, method, path, body, timeoutMs = 330000) {
  return new Promise((resolve, reject) => {
    if (terminal.state !== 'läuft' || !terminal.port) {
      reject(new Error('Das Terminal läuft nicht. Bitte in der Statusleiste neu starten.'));
      return;
    }
    const data = body === undefined ? null : Buffer.from(JSON.stringify(body));
    const req = http.request({ host: '127.0.0.1', port: terminal.port, method, path, timeout: timeoutMs,
      headers: { Authorization: `Bearer ${terminal.key}`, ...(data ? { 'Content-Type': 'application/json', 'Content-Length': data.length } : {}) },
    }, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        const text = Buffer.concat(chunks).toString('utf8');
        let json = null;
        try { json = JSON.parse(text); } catch { /* kein JSON */ }
        resolve({ status: res.statusCode, json, text, type: res.headers['content-type'] || '' });
      });
    });
    req.on('timeout', () => req.destroy(new Error('Zeitüberschreitung beim Terminal')));
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

function fail(res) {
  const detail = res.json?.detail;
  const text = typeof detail === 'string' ? detail : detail ? JSON.stringify(detail) : res.text.slice(0, 500);
  return new Error(text || `HTTP ${res.status}`);
}

function clip(text, max = MAX_OUTPUT) {
  text = String(text ?? '');
  if (text.length <= max) return text;
  const head = Math.floor(max * 0.3);
  return `${text.slice(0, head)}\n… (${text.length - max} Zeichen ausgelassen) …\n${text.slice(text.length - (max - head))}`;
}

const q = (params) => Object.entries(params).filter(([, v]) => v !== undefined && v !== null && v !== '')
  .map(([k, v]) => `${encodeURIComponent(k)}=${encodeURIComponent(v)}`).join('&');

function lines(text) {
  return String(text).split(/\r?\n/).length;
}

const zeilen = (n) => `${n} ${n === 1 ? 'Zeile' : 'Zeilen'}`;

const outputText = (json) => (json.output || []).map((o) => o.data ?? '').join('').replace(/\r\n/g, '\n');

// Programme mit Fenster sperren Dateien und warten auf Eingaben, die niemand macht (Word hielt so
// Dokumente in ausgabe/ fest). Erkannt werden sie als eigener Befehl, auch nach &, | oder «start».
const WINDOWED = new Set(['start', 'winword', 'excel', 'powerpnt', 'outlook', 'msaccess', 'notepad', 'mspaint', 'explorer',
  'msedge', 'chrome', 'firefox']);

function blockedCommand(command) {
  // Erstes Wort jedes Befehlsteils (getrennt durch &, |, Klammern), in Anführungszeichen auch mit Leerzeichen
  const programs = String(command).split(/[&|()]/).map((part) => {
    const s = part.trim();
    const word = s.startsWith('"') ? s.slice(1).split('"')[0] : s.split(/\s/)[0];
    return word.split(/[\\/]/).pop().toLowerCase().replace(/\.exe$/, '');
  });
  if (!programs.some((p) => WINDOWED.has(p))) return '';
  return 'Abgelehnt: Dieser Befehl würde ein Programm mit Fenster starten (z. B. Word, Excel, Explorer oder «start»). '
    + 'Das sperrt Dateien und wartet auf Eingaben. Für eine PDF-Prüfung python .ask/skills/pruefen.py <datei> --pdf verwenden; '
    + 'sonst der Person sagen, dass sie die Datei selbst öffnen soll.';
}

// Führt einen Befehl aus und liefert die Ausgabe laufend (onProgress). Stoppt über signal.
async function runCommand(terminal, args, { signal, onProgress }) {
  const command = String(args.command || '').trim();
  if (!command) throw new Error('command fehlt');
  const blocked = blockedCommand(command);
  if (blocked) return { ok: false, output: blocked, summary: 'abgelehnt' };
  const limit = Math.min(Math.max(Number(args.wait) || 120, 5), 600);
  const started = await request(terminal, 'POST', `/execute?${q({ wait: POLL_SECONDS })}`, { command });
  if (started.status !== 200 || !started.json) throw fail(started);
  let data = started.json;
  const id = data.id;
  let output = outputText(data);
  let offset = data.next_offset ?? (data.output || []).length;
  const end = Date.now() + limit * 1000;
  const kill = () => request(terminal, 'DELETE', `/execute/${encodeURIComponent(id)}`).catch(() => {});
  signal?.addEventListener('abort', kill, { once: true });
  try {
    while (data.status === 'running' && Date.now() < end) {
      if (signal?.aborted) throw new Error('abgebrochen');
      onProgress?.(output);
      const res = await request(terminal, 'GET', `/execute/${encodeURIComponent(id)}/status?${q({ wait: POLL_SECONDS, offset })}`);
      if (res.status !== 200 || !res.json) throw fail(res);
      data = res.json;
      output += outputText(data);
      offset = data.next_offset ?? offset + (data.output || []).length;
    }
  } finally {
    signal?.removeEventListener('abort', kill);
  }
  const running = data.status === 'running';
  const head = running
    ? `Läuft nach ${limit} s noch (id ${id}). Mit command_status nachfragen oder warten.`
    : `Exitcode ${data.exit_code}`;
  return {
    ok: running || data.exit_code === 0,
    output: `${head}\n${clip(output) || '(keine Ausgabe)'}`,
    display: output.trimEnd() || '(keine Ausgabe)',
    summary: running ? 'läuft noch' : (data.exit_code === 0 ? '' : `Exitcode ${data.exit_code}`),
  };
}

const TOOLS = [
  {
    name: 'run_command',
    label: 'Befehl',
    describe: (a) => a.command,
    description: 'Führt einen Befehl in cmd.exe im Arbeitsordner aus und liefert Ausgabe und Exitcode. ',
    parameters: {
      type: 'object',
      properties: {
        command: { type: 'string', description: 'Befehl für cmd.exe, z. B. python auswertung.py' },
        wait: { type: 'number', description: 'Höchstens so viele Sekunden warten (Standard 120).' },
      },
      required: ['command'],
    },
    run: (terminal, args, ctx) => runCommand(terminal, args, ctx),
  },
  {
    name: 'command_status',
    label: 'Befehl',
    describe: (a) => `Status ${a.id}`,
    description: 'Fragt nach Ausgabe und Exitcode eines Befehls, der beim Aufruf noch lief.',
    parameters: {
      type: 'object',
      properties: { id: { type: 'string', description: 'id aus run_command' } },
      required: ['id'],
    },
    async run(terminal, args) {
      const res = await request(terminal, 'GET', `/execute/${encodeURIComponent(args.id)}/status?${q({ wait: 30 })}`);
      if (res.status !== 200 || !res.json) throw fail(res);
      const out = outputText(res.json);
      const head = res.json.status === 'running' ? 'Läuft noch.' : `Exitcode ${res.json.exit_code}`;
      return { ok: true, output: `${head}\n${clip(out)}`, display: out.trimEnd() || head, summary: head };
    },
  },
  {
    name: 'read_file',
    label: 'Lesen',
    describe: (a) => a.path + (a.start_line ? ` (ab Zeile ${a.start_line})` : ''),
    description: 'Liest eine Datei im Arbeitsordner. Word, PDF, Excel und PowerPoint werden als Text geliefert. Bei langen Dateien start_line und end_line angeben.',
    parameters: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Pfad relativ zum Arbeitsordner' },
        start_line: { type: 'integer', description: 'Erste Zeile (ab 1)' },
        end_line: { type: 'integer', description: 'Letzte Zeile' },
      },
      required: ['path'],
    },
    async run(terminal, args) {
      const res = await request(terminal, 'GET', `/files/read?${q({ path: args.path, start_line: args.start_line, end_line: args.end_line })}`);
      if (res.status !== 200) throw fail(res);
      if (!res.json || typeof res.json.content !== 'string') {
        return { ok: true, output: `Binärdatei (${res.type}), kann nicht als Text gelesen werden.`, summary: 'Binärdatei' };
      }
      const total = res.json.total_lines;
      return { ok: true, output: `${clip(res.json.content, 30000)}\n\n(${zeilen(total)} insgesamt)`, summary: zeilen(lines(res.json.content)) };
    },
  },
  {
    name: 'write_file',
    label: 'Schreiben',
    describe: (a) => a.path,
    description: 'Schreibt eine Textdatei im Arbeitsordner (legt Ordner an, überschreibt vorhandene Dateien). Fertige Ergebnisse gehören nach ausgabe/.',
    parameters: {
      type: 'object',
      properties: {
        path: { type: 'string', description: 'Pfad relativ zum Arbeitsordner' },
        content: { type: 'string', description: 'Vollständiger Inhalt' },
      },
      required: ['path', 'content'],
    },
    async run(terminal, args) {
      const res = await request(terminal, 'POST', '/files/write', { path: args.path, content: String(args.content ?? '') });
      if (res.status !== 200) throw fail(res);
      const n = lines(args.content ?? '');
      return { ok: true, output: `Geschrieben: ${args.path} (${zeilen(n)})`, display: String(args.content ?? ''), summary: zeilen(n) };
    },
  },
  {
    name: 'edit_file',
    label: 'Bearbeiten',
    describe: (a) => a.path,
    description: 'Ersetzt in einer Datei einen Textabschnitt durch einen neuen. old_text muss genau einmal vorkommen (ausser replace_all ist true).',
    parameters: {
      type: 'object',
      properties: {
        path: { type: 'string' },
        old_text: { type: 'string', description: 'Bisheriger Text, genau wie in der Datei' },
        new_text: { type: 'string', description: 'Neuer Text' },
        replace_all: { type: 'boolean', description: 'Alle Vorkommen ersetzen' },
      },
      required: ['path', 'old_text', 'new_text'],
    },
    async run(terminal, args) {
      const res = await request(terminal, 'POST', '/files/replace', { path: args.path,
        replacements: [{ target: String(args.old_text ?? ''), replacement: String(args.new_text ?? ''), allow_multiple: !!args.replace_all }] });
      if (res.status !== 200) throw fail(res);
      const display = [...String(args.old_text).split('\n').map((l) => `- ${l}`), ...String(args.new_text).split('\n').map((l) => `+ ${l}`)].join('\n');
      return { ok: true, output: `Geändert: ${args.path}`, display, summary: `${zeilen(lines(args.new_text ?? ''))} neu` };
    },
  },
  {
    name: 'list_files',
    label: 'Ordner',
    describe: (a) => a.path || '.',
    description: 'Listet den Inhalt eines Ordners im Arbeitsordner auf.',
    parameters: { type: 'object', properties: { path: { type: 'string', description: 'Ordner, Standard .' } } },
    async run(terminal, args) {
      const res = await request(terminal, 'GET', `/files/list?${q({ directory: args.path || '.' })}`);
      if (res.status !== 200 || !res.json) throw fail(res);
      const entries = (res.json.entries || []).map((e) => {
        const dir = e.type === 'directory' || e.is_dir || e.isDirectory;
        return dir ? `${e.name}/` : `${e.name}${e.size !== undefined ? ` (${e.size} B)` : ''}`;
      });
      return { ok: true, output: entries.join('\n') || '(leer)', summary: `${entries.length} Einträge` };
    },
  },
  {
    name: 'find_files',
    label: 'Suchen',
    describe: (a) => a.pattern,
    description: 'Sucht Dateien nach Namen mit einem Muster wie *.docx oder **/*.py.',
    parameters: {
      type: 'object',
      properties: { pattern: { type: 'string' }, path: { type: 'string', description: 'Ordner, Standard .' } },
      required: ['pattern'],
    },
    async run(terminal, args) {
      const res = await request(terminal, 'GET', `/files/glob?${q({ pattern: args.pattern, path: args.path || '.', max_results: 200 })}`);
      if (res.status !== 200 || !res.json) throw fail(res);
      const list = res.json.matches || res.json.results || res.json.entries || [];
      const names = list.map((m) => (typeof m === 'string' ? m : m.path || m.name));
      return { ok: true, output: names.join('\n') || 'Keine Treffer.', summary: `${names.length} Treffer` };
    },
  },
  {
    name: 'search_text',
    label: 'Suchen',
    describe: (a) => `«${a.text}»${a.path ? ` in ${a.path}` : ''}`,
    description: 'Sucht einen Text in den Dateien des Arbeitsordners und liefert Datei, Zeile und Fundstelle.',
    parameters: {
      type: 'object',
      properties: { text: { type: 'string' }, path: { type: 'string', description: 'Ordner oder Datei, Standard .' } },
      required: ['text'],
    },
    async run(terminal, args) {
      const res = await request(terminal, 'GET', `/files/grep?${q({ query: args.text, path: args.path || '.', regex: false, case_insensitive: true })}`);
      if (res.status !== 200 || !res.json) throw fail(res);
      const list = res.json.matches || res.json.results || [];
      const found = list.map((m) => (typeof m === 'string' ? m : `${m.file || m.path}:${m.line ?? m.line_number}: ${String(m.content ?? m.text ?? '').trim()}`));
      return { ok: true, output: clip(found.join('\n')) || 'Keine Treffer.', summary: `${found.length} Treffer` };
    },
  },
  {
    name: 'update_tasks',
    label: 'Aufgaben',
    describe: (a) => a.auftrag || '',
    description: 'Legt die Aufgabenliste an oder aktualisiert sie. Die Person sieht sie in der Seitenleiste. Bei Aufträgen mit mehr als zwei Schritten zuerst anlegen und nach jedem Schritt aktualisieren.',
    parameters: {
      type: 'object',
      properties: {
        auftrag: { type: 'string', description: 'Kurzbeschreibung des Auftrags' },
        aufgaben: {
          type: 'array',
          items: {
            type: 'object',
            properties: { titel: { type: 'string' }, status: { type: 'string', enum: ['offen', 'in_arbeit', 'erledigt'] } },
            required: ['titel', 'status'],
          },
        },
      },
      required: ['aufgaben'],
    },
    async run(terminal, args) {
      const tasks = { auftrag: String(args.auftrag || ''), aufgaben: Array.isArray(args.aufgaben) ? args.aufgaben : [] };
      const res = await request(terminal, 'POST', '/files/write', { path: '.ask/tasks.json', content: JSON.stringify(tasks, null, 2) });
      if (res.status !== 200) throw fail(res);
      const done = tasks.aufgaben.filter((t) => t.status === 'erledigt').length;
      return { ok: true, output: 'Aufgabenliste gespeichert.', tasks: tasks.aufgaben, summary: `${done} von ${tasks.aufgaben.length} erledigt` };
    },
  },
];

// GitHub: Projekt holen und hochladen (src/main/github.js). Das Token kennt nur der Hauptprozess;
// Anlegen und Hochladen bestätigt die Person in einem Dialog (ctx.confirm).
const NOT_CONNECTED = 'Nicht mit GitHub verbunden. Sag der Person: in der App /github eingeben und ein Token eintragen.';

function githubTool(fn) {
  return async (terminal, args, ctx) => {
    if (!ctx.github || !ctx.folder) throw new Error('GitHub ist hier nicht verfügbar.');
    if (!ctx.github.connected()) return { ok: false, output: NOT_CONNECTED, summary: 'nicht verbunden' };
    return fn(ctx.github, ctx.folder, args, ctx);
  };
}

const list = (items, max = 15) => (items.length ? `${items.slice(0, max).join(', ')}${items.length > max ? ` und ${items.length - max} weitere` : ''}` : '–');

TOOLS.push(
  {
    name: 'github_status',
    label: 'GitHub',
    describe: () => 'Status',
    description: 'Zeigt, ob der Arbeitsordner mit einem GitHub-Repository verbunden ist, was hier geändert ist und ob es auf GitHub Neueres gibt.',
    parameters: { type: 'object', properties: {} },
    run: githubTool(async (gh, folder) => {
      const s = await gh.status(folder);
      if (!s.linked) return { ok: true, output: `Mit GitHub verbunden als ${s.login}. Der Ordner ist mit keinem Repository verbunden (github_clone oder github_create).`, summary: 'kein Repository' };
      const output = [`Repository: ${s.repo} (${s.url}), Branch ${s.branch}`, `Geändert: ${list(s.changed)}`, `Neu: ${list(s.added)}`,
        `Gelöscht: ${list(s.deleted)}`, s.remoteNewer ? 'Auf GitHub gibt es neuere Änderungen: zuerst github_pull.' : 'GitHub ist auf dem Stand des letzten Abgleichs.'].join('\n');
      return { ok: true, output, summary: `${s.repo} · ${s.changed.length + s.added.length + s.deleted.length} Änderungen hier${s.remoteNewer ? ' · auf GitHub Neueres' : ''}` };
    }),
  },
  {
    name: 'github_clone',
    label: 'GitHub',
    describe: (a) => `holen ${a.repo}`,
    description: 'Holt ein GitHub-Repository in den Arbeitsordner und verbindet den Ordner damit. Vorhandene Dateien mit gleichem Namen werden nicht überschrieben.',
    parameters: { type: 'object', properties: { repo: { type: 'string', description: 'besitzer/name oder GitHub-Adresse' } }, required: ['repo'] },
    run: githubTool(async (gh, folder, a) => {
      const r = await gh.clone(folder, a.repo);
      const output = `Geholt von ${r.repo}: ${r.written.length} Dateien.${r.conflicts.length ? ` Nicht überschrieben, weil hier schon anders vorhanden: ${list(r.conflicts)}.` : ''}`;
      return { ok: true, output, summary: `${r.written.length} Dateien von ${r.repo}${r.conflicts.length ? ` · ${r.conflicts.length} Konflikte` : ''}` };
    }),
  },
  {
    name: 'github_pull',
    label: 'GitHub',
    describe: () => 'holen',
    description: 'Holt Änderungen vom verbundenen GitHub-Repository. Hier geänderte Dateien werden nie überschrieben, sondern als Konflikt gemeldet.',
    parameters: { type: 'object', properties: {} },
    run: githubTool(async (gh, folder) => {
      const r = await gh.pull(folder);
      const output = [`Geholt: ${list(r.written)}`, `Gelöscht (auf GitHub entfernt): ${list(r.removed)}`,
        r.conflicts.length ? `Konflikte (hier und auf GitHub geändert, nichts überschrieben): ${list(r.conflicts)}. Die Person entscheiden lassen.` : 'Keine Konflikte.'].join('\n');
      return { ok: true, output, summary: `${r.written.length} geholt${r.removed.length ? `, ${r.removed.length} gelöscht` : ''}${r.conflicts.length ? ` · ${r.conflicts.length} Konflikte` : ''}` };
    }),
  },
  {
    name: 'github_push',
    label: 'GitHub',
    describe: (a) => `hochladen «${a.nachricht || ''}»`,
    description: 'Lädt die Änderungen im Arbeitsordner als Commit auf das verbundene GitHub-Repository hoch (ohne .ask). Die Person bestätigt vorher.',
    parameters: { type: 'object', properties: { nachricht: { type: 'string', description: 'Kurze Commit-Nachricht, was sich geändert hat' } }, required: ['nachricht'] },
    run: githubTool(async (gh, folder, a, ctx) => {
      const s = await gh.status(folder);
      if (!s.linked) return { ok: false, output: 'Der Ordner ist mit keinem Repository verbunden (github_create oder github_clone).', summary: 'kein Repository' };
      const changes = [...s.changed, ...s.added, ...s.deleted];
      if (changes.length === 0) return { ok: true, output: 'Nichts hochzuladen: keine Änderungen seit dem letzten Abgleich.', summary: 'nichts zu tun' };
      if (!(await ctx.confirm(`${changes.length} Änderungen auf GitHub hochladen?`, `Repository: ${s.repo}\nNachricht: ${a.nachricht}\n\n${list(changes, 20)}`))) {
        return { ok: false, output: 'Die Person hat das Hochladen abgelehnt.', summary: 'abgelehnt' };
      }
      const r = await gh.push(folder, a.nachricht);
      return { ok: true, output: `Hochgeladen: ${r.uploaded} Dateien, ${r.deleted.length} gelöscht. ${r.url}`, summary: `${r.uploaded} hochgeladen${r.deleted.length ? `, ${r.deleted.length} gelöscht` : ''} · ${s.repo}` };
    }),
  },
  {
    name: 'github_create',
    label: 'GitHub',
    describe: (a) => `neu ${a.name}${a.privat === false ? ' (öffentlich)' : ' (privat)'}`,
    description: 'Legt ein neues GitHub-Repository an (standardmässig privat), verbindet den Arbeitsordner damit und lädt ihn hoch. Die Person bestätigt vorher.',
    parameters: {
      type: 'object',
      properties: {
        name: { type: 'string', description: 'Name des Repositorys, z. B. frogger-spiel' },
        privat: { type: 'boolean', description: 'Privat (Standard true). Öffentlich nur, wenn die Person es ausdrücklich will.' },
        beschreibung: { type: 'string' },
      },
      required: ['name'],
    },
    run: githubTool(async (gh, folder, a, ctx) => {
      const privat = a.privat !== false;
      if (!(await ctx.confirm(`Neues ${privat ? 'privates' : 'ÖFFENTLICHES'} GitHub-Repository «${a.name}» anlegen und den Arbeitsordner hochladen?`,
        `Konto: ${await gh.whoami()}\nOrdner: ${folder}\nHochgeladen wird alles ausser .ask (Chats, Skripte).`))) {
        return { ok: false, output: 'Die Person hat das Anlegen abgelehnt.', summary: 'abgelehnt' };
      }
      const r = await gh.create(folder, String(a.name).trim(), { privat, beschreibung: a.beschreibung || '' });
      return { ok: true, output: `Angelegt: ${r.repo} (${r.private ? 'privat' : 'öffentlich'}), ${r.uploaded} Dateien hochgeladen. ${r.url}`, summary: `${r.repo} · ${r.uploaded} Dateien` };
    }),
  },
);

const byName = new Map(TOOLS.map((t) => [t.name, t]));

// Werkzeugbeschreibungen im OpenAI-Format; die Befehlsbeschreibung kommt aus resources/anweisungen.
function toolSpecs(commandHint = '') {
  return TOOLS.map((t) => ({
    type: 'function',
    function: { name: t.name, description: t.name === 'run_command' ? t.description + commandHint : t.description, parameters: t.parameters },
  }));
}

function parseArgs(raw) {
  if (raw && typeof raw === 'object') return raw;
  const text = String(raw || '').trim();
  if (!text) return {};
  try {
    return JSON.parse(text);
  } catch {
    throw new Error(`Die Argumente sind kein gültiges JSON: ${text.slice(0, 200)}`);
  }
}

function describe(name, args) {
  if (name === 'goal_done') return { label: 'Ziel erreicht', detail: '' }; // Werkzeug der Arbeitsschleife (agent.js)
  const tool = byName.get(name);
  let detail = '';
  try { detail = tool?.describe(args) || ''; } catch { /* leer */ }
  return { label: tool?.label || name, detail: String(detail || '').replace(/\s+/g, ' ').slice(0, 200) };
}

// Führt einen Werkzeugaufruf aus. Fehler gehen als Text an das Modell zurück.
async function runTool(terminal, name, rawArgs, ctx = {}) {
  const tool = byName.get(name);
  if (!tool) return { ok: false, output: `Unbekanntes Werkzeug: ${name}. Verfügbar: ${TOOLS.map((t) => t.name).join(', ')}`, summary: 'unbekannt' };
  try {
    const args = parseArgs(rawArgs);
    return await tool.run(terminal, args, ctx);
  } catch (err) {
    if (ctx.signal?.aborted) return { ok: false, output: 'Von der Person abgebrochen.', summary: 'abgebrochen' };
    return { ok: false, output: `Fehler: ${err.message}`, summary: err.message.slice(0, 200) };
  }
}

module.exports = { TOOLS, toolSpecs, runTool, describe, parseArgs, clip, request, blockedCommand };
