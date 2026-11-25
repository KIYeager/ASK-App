// GitHub-Projekte holen und hochladen: Repository klonen, Änderungen holen (pull) und als Commit
// hochladen (push), dazu ein neues Repository anlegen. Ohne git auf dem Laptop (unter Low Integrity
// laufen die Git-Werkzeuge nicht zuverlässig), direkt über die REST-API von GitHub mit einem
// persönlichen Token. Das Token kennt nur der Hauptprozess, nie das Modell oder das Terminal.
//
// Der letzte abgeglichene Stand (Repository, Branch, Commit und die Prüfsumme jeder Datei) liegt in
// einer Datei ausserhalb des Arbeitsordners (linkFile, in der App unter projectDataDir), damit das
// Modell ihn nicht ändern kann; er wird beim Lesen trotzdem geprüft. Damit erkennt der Abgleich,
// was hier, was auf GitHub und was an beiden Orten geändert wurde; Dateien mit Änderungen an
// beiden Orten werden nie stillschweigend überschrieben, weder hier noch auf GitHub.
// Was in .gitignore steht, wird nicht hochgeladen (ausser es liegt schon auf GitHub, wie bei git).
// Branches und Pull Requests ebenfalls über die API: Branch anlegen ist ein neuer Ref, wechseln ein
// Holen vom anderen Branch (nur ohne Änderungen hier, ENTSCHEIDE E47).
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { safeRel, safeJoin, SHA1 } = require('./safe');

const MAX_FILE = 50 * 1024 * 1024; // GitHub lehnt grössere Dateien ab bzw. warnt
const MAX_FILES = 3000;
// Nicht hochladen: Arbeitsdaten der App (Chats, Skripte, Anleitungen) und typische Ballast-Ordner
const SKIP = /(?:^|\/)(\.ask|\.git|node_modules|__pycache__|\.venv)(\/|$)/i;
const REQUEST_MS = 30 * 1000; // Zeitgrenze einer Anfrage an GitHub, sonst hängt der Abgleich ewig
const BLOB_MS = 120 * 1000; // Dateien hoch- und herunterladen darf länger dauern
const LINK = path.join('.ask', 'github.json');

// Git-Prüfsumme einer Datei, wie git hash-object
function blobSha(buffer) {
  return crypto.createHash('sha1').update(`blob ${buffer.length}\0`).update(buffer).digest('hex');
}

// Eine Zeile aus .gitignore als Regel. base: Ordner der .gitignore-Datei relativ zum Arbeitsordner ('' = oben).
function ignoreRule(line, base) {
  let p = line.replace(/\r$/, '').replace(/(?<!\\)\s+$/, '');
  if (!p || p.startsWith('#')) return null;
  const negate = p.startsWith('!');
  if (negate) p = p.slice(1);
  const dirOnly = p.endsWith('/');
  if (dirOnly) p = p.slice(0, -1);
  if (!p) return null;
  const anchored = p.includes('/');
  if (p.startsWith('/')) p = p.slice(1);
  // Muster als Liste von Bausteinen, geprüft ohne regulären Ausdruck mit verschachtelten Wiederholungen:
  // Das Modell schreibt .gitignore, und ein Muster wie «*a*a*a*a*b» würde den Hauptprozess sonst festfahren.
  const tokens = [];
  if (!anchored) tokens.push({ t: 'gslash' });
  for (let i = 0; i < p.length; i++) {
    const c = p[i];
    if (c === '*' && p[i + 1] === '*') {
      let end = i + 2;
      while (p[end] === '*') end++; // mehrere * hintereinander wie eines
      if (p[end] === '/') {
        i = end;
        if (tokens.at(-1)?.t !== 'gslash') tokens.push({ t: 'gslash' }); // mehrere **/ hintereinander wie eines
      } else {
        i = end - 1;
        if (tokens.at(-1)?.t !== 'gstar') tokens.push({ t: 'gstar' });
      }
    } else if (c === '*') {
      while (p[i + 1] === '*') i++;
      if (tokens.at(-1)?.t !== 'star') tokens.push({ t: 'star' });
    } else if (c === '?') tokens.push({ t: 'any' });
    else if (c === '[') {
      const end = p.indexOf(']', i + 2);
      if (end < 0) tokens.push({ t: 'lit', c: '[' });
      else {
        try {
          tokens.push({ t: 'cls', re: new RegExp(`^[${p.slice(i + 1, end).replace(/^!/, '^').replace(/\\/g, '\\\\')}]$`) });
        } catch { return null; }
        i = end;
      }
    } else if (c === '\\' && i + 1 < p.length) { tokens.push({ t: 'lit', c: p[i + 1] }); i += 1; }
    else tokens.push({ t: 'lit', c });
  }
  return { base, negate, dirOnly, match: (text) => globMatch(tokens, text) };
}

// Muster gegen einen Pfad, ein Zustandsdurchlauf (Zeit Zeichen × Bausteine, kein Zurückgehen)
function globMatch(tokens, text) {
  const n = tokens.length;
  let cur = new Uint8Array(n + 1);
  let next = new Uint8Array(n + 1);
  const close = (set) => {
    for (let i = 0; i < n; i++) if (set[i] && (tokens[i].t === 'star' || tokens[i].t === 'gstar' || tokens[i].t === 'gslash')) set[i + 1] = 1;
  };
  cur[0] = 1;
  close(cur);
  for (const ch of text) {
    next.fill(0);
    for (let i = 0; i < n; i++) {
      if (!cur[i]) continue;
      const tok = tokens[i];
      switch (tok.t) {
        case 'lit': if (ch === tok.c) next[i + 1] = 1; break;
        case 'any': if (ch !== '/') next[i + 1] = 1; break;
        case 'cls': if (ch !== '/' && tok.re.test(ch)) next[i + 1] = 1; break;
        case 'star': if (ch !== '/') next[i] = 1; break;
        case 'gstar': next[i] = 1; break;
        case 'gslash': next[i] = 1; if (ch === '/') next[i + 1] = 1; break;
        default: break;
      }
    }
    close(next);
    [cur, next] = [next, cur];
  }
  return cur[n] === 1;
}

function readIgnore(dir, base) {
  let text;
  try { text = fs.readFileSync(path.join(dir, '.gitignore'), 'utf8'); } catch { return []; }
  if (text.length > 200000) return [];
  return text.split('\n').map((line) => ignoreRule(line, base)).filter(Boolean);
}

// Die letzte passende Regel entscheidet, wie bei git
function ignored(rules, rel, isDir) {
  let hit = false;
  for (const r of rules) {
    if (r.dirOnly && !isDir) continue;
    if (r.base && !rel.startsWith(`${r.base}/`)) continue;
    if (r.match(r.base ? rel.slice(r.base.length + 1) : rel)) hit = !r.negate;
  }
  return hit;
}

// Dateien im Ordner ohne .ask usw. und ohne das, was .gitignore ausschliesst. tracked: Pfade, die
// schon abgeglichen sind; sie bleiben dabei, auch wenn .gitignore sie inzwischen nennt (wie bei git).
function listLocal(root, tracked = new Set()) {
  const out = [];
  const trackedDirs = new Set();
  for (const p of tracked) for (let i = p.indexOf('/'); i >= 0; i = p.indexOf('/', i + 1)) trackedDirs.add(p.slice(0, i));
  const walk = (dir, base, rules, parentIgnored) => {
    const mine = parentIgnored ? rules : [...rules, ...readIgnore(dir, base)];
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      const rel = base ? `${base}/${entry.name}` : entry.name;
      if (SKIP.test(rel) || entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        const ign = parentIgnored || ignored(mine, rel, true);
        if (!ign || trackedDirs.has(rel)) walk(full, rel, mine, ign);
      } else if (entry.isFile()) {
        if ((parentIgnored || ignored(mine, rel, false)) && !tracked.has(rel)) continue;
        if (out.length >= MAX_FILES) throw new Error(`Mehr als ${MAX_FILES} Dateien im Ordner. Das ist für den Abgleich zu viel.`);
        const size = fs.statSync(full).size;
        if (size > MAX_FILE) throw new Error(`${rel} ist grösser als 50 MB und kann nicht auf GitHub.`);
        out.push({ path: rel, full });
      }
    }
  };
  walk(root, '', [], false);
  return out;
}

// «besitzer/name», GitHub-Adresse (auch mit /tree/main oder /blob/… dahinter) oder git@github.com:besitzer/name.git
function parseRepo(text) {
  const t = String(text || '').trim();
  const url = /github\.com[/:]([\w.-]+)\/([\w.-]+?)(?:\.git)?(?:[/?#].*)?$/i.exec(t);
  const m = url || /^([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/.exec(t);
  if (!m || [m[1], m[2]].some((part) => /^\.+$/.test(part))) throw new Error('Repository als «besitzer/name» oder als GitHub-Adresse angeben.');
  return `${m[1]}/${m[2]}`;
}

const REPO = /^[\w.-]+\/[\w.-]+$/;

// Branch-Namen, die in Adressen der API und in der gespeicherten Verbindung gehen dürfen (Regeln von git, enger)
function validBranch(name) {
  return typeof name === 'string' && /^[\w./-]{1,200}$/.test(name) && !name.includes('..') && !name.includes('//')
    && !/^[-./]/.test(name) && !/[./]$/.test(name) && !/\.lock(\/|$)/i.test(name) && !name.includes('/.') && name !== 'HEAD';
}

// Branch in einer Adresse der API: Schrägstriche bleiben (heads/feature/x), der Rest wird kodiert
const refPath = (branch) => branch.split('/').map(encodeURIComponent).join('/');

// Gespeicherte Verbindung prüfen: Repository und Branch gehen in Adressen der GitHub-API, die Pfade in Dateinamen.
function validLink(link) {
  if (!link || typeof link !== 'object' || typeof link.repo !== 'string' || !REPO.test(link.repo) || link.repo.split('/').some((part) => /^\.+$/.test(part))) return false;
  if (!validBranch(link.branch)) return false;
  if (link.commit !== null && !SHA1.test(String(link.commit))) return false;
  const files = link.files;
  if (!files || typeof files !== 'object' || Array.isArray(files)) return false;
  return Object.entries(files).every(([p, sha]) => safeRel(p) && SHA1.test(String(sha)));
}

class GitHub {
  // store: { get(), set(token) } für das verschlüsselt gespeicherte Token
  // linkFile(root): wo die Verbindung eines Ordners gespeichert ist (in der App ausserhalb des Ordners)
  constructor({ api = 'https://api.github.com', web = 'https://github.com', store, fetchImpl = fetch, timeoutMs = REQUEST_MS, linkFile = (root) => path.join(root, LINK) }) {
    this.linkFile = linkFile;
    this.api = api.replace(/\/$/, '');
    this.web = web.replace(/\/$/, '');
    this.store = store;
    this.fetch = fetchImpl;
    this.timeoutMs = timeoutMs;
    this.login = '';
    this.busy = new Set(); // Ordner, in denen der Bereich «GitHub» gerade abgleicht (main.js)
  }

  // Ein Abgleich aus dem Bereich «GitHub» zur Zeit pro Ordner; das Modell wartet so lange (tools.js)
  static key(root) {
    return path.resolve(root).toLowerCase();
  }

  locked(root) {
    return this.busy.has(GitHub.key(root));
  }

  async exclusive(root, fn) {
    const key = GitHub.key(root);
    if (this.busy.has(key)) throw new Error('Ein Abgleich mit GitHub läuft schon.');
    this.busy.add(key);
    try {
      return await fn();
    } finally {
      this.busy.delete(key);
    }
  }

  connected() {
    return !!this.store.get();
  }

  async request(method, url, body, timeoutMs = this.timeoutMs) {
    const token = this.store.get();
    if (!token) throw new Error('Nicht mit GitHub verbunden. Die Person kann in der App mit /github ein Token eintragen.');
    let res;
    let text;
    const abort = new AbortController();
    const timer = setTimeout(() => abort.abort(Object.assign(new Error('Zeit abgelaufen'), { name: 'TimeoutError' })), timeoutMs);
    try {
      res = await this.fetch(`${this.api}${url}`, {
        method,
        headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28',
          ...(body ? { 'Content-Type': 'application/json' } : {}) },
        body: body ? JSON.stringify(body) : undefined,
        signal: abort.signal,
      });
      text = await res.text();
    } catch (err) {
      if (err?.name === 'TimeoutError' || err?.name === 'AbortError') throw new Error(`GitHub antwortet nicht (nach ${Math.round(timeoutMs / 1000)} s abgebrochen). Internetverbindung prüfen und nochmals versuchen.`);
      throw err;
    } finally {
      clearTimeout(timer);
    }
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch { /* kein JSON */ }
    if (!res.ok) {
      // Bei 422 steht der Grund oft erst in errors (z. B. «A pull request already exists»)
      const detail = Array.isArray(data?.errors) ? data.errors.map((e) => e?.message).filter(Boolean).join('; ') : '';
      const message = [data?.message, detail].filter(Boolean).join(': ') || text.slice(0, 200) || `HTTP ${res.status}`;
      if (res.status === 401) throw new Error('GitHub lehnt das Token ab (abgelaufen oder ungültig). Mit /github neu verbinden.');
      throw Object.assign(new Error(`GitHub: ${message}`), { status: res.status });
    }
    return data;
  }

  async connect(token) {
    const old = this.store.get();
    this.store.set(String(token || '').trim());
    try {
      const user = await this.request('GET', '/user');
      this.login = user.login;
      return user.login;
    } catch (err) {
      this.store.set(old || '');
      throw err;
    }
  }

  disconnect() {
    this.store.set('');
    this.login = '';
  }

  async whoami() {
    if (!this.login && this.connected()) this.login = (await this.request('GET', '/user')).login;
    return this.login;
  }

  readLink(root) {
    try {
      const link = JSON.parse(fs.readFileSync(this.linkFile(root), 'utf8'));
      return validLink(link) ? link : null;
    } catch {
      return null;
    }
  }

  writeLink(root, link) {
    const file = this.linkFile(root);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(link, null, 2));
  }

  url(link) {
    return `${this.web}/${link.repo}`;
  }

  // Stand auf GitHub: Commit, Dateien (Pfad → Prüfsumme), ihre Art (modes: ausführbar, Verknüpfung) und
  // Einträge, die keine Dateien sind (others: Submodule). Leeres Repository: commit null.
  async remote(link) {
    let head;
    try {
      head = (await this.request('GET', `/repos/${link.repo}/git/ref/heads/${refPath(link.branch)}`)).object.sha;
    } catch (err) {
      if (err.status === 404 || err.status === 409) return this.noBranch(link, err);
      throw err;
    }
    const tree = await this.request('GET', `/repos/${link.repo}/git/trees/${head}?recursive=1`);
    if (tree.truncated) throw new Error('Das Repository ist für den Abgleich zu gross.');
    const files = {};
    const modes = {};
    const others = [];
    for (const e of tree.tree) {
      if (e.type === 'blob') { files[e.path] = e.sha; modes[e.path] = e.mode; }
      else if (e.type === 'commit') others.push({ path: e.path, mode: e.mode || '160000', type: 'commit', sha: e.sha });
    }
    return { commit: head, files, modes, others };
  }

  // Der Branch liess sich nicht lesen. Nur ein Repository ohne jeden Branch ist wirklich leer; sonst wurde der
  // Branch gelöscht (z. B. nach einem Pull Request), das Repository umbenannt oder das Token verlor den Zugriff.
  // Ein leerer Stand würde beim Holen alle Dateien hier löschen.
  async noBranch(link, cause) {
    let list;
    try {
      await this.request('GET', `/repos/${link.repo}`);
      list = await this.request('GET', `/repos/${link.repo}/branches?per_page=1`);
    } catch (err) {
      if (err.status === 404) throw new Error(`Das Repository ${link.repo} gibt es auf GitHub nicht mehr (umbenannt oder gelöscht) oder das Token hat keinen Zugriff darauf.`);
      throw err;
    }
    if (Array.isArray(list) && list.length === 0 && (cause.status === 409 || cause.status === 404)) return { commit: null, files: {}, modes: {}, others: [] };
    throw new Error(`Den Branch ${link.branch} gibt es auf GitHub nicht mehr. Wähle im Reiter GitHub einen anderen Branch.`);
  }

  // Was hat sich seit dem letzten Abgleich hier geändert?
  localChanges(root, link) {
    const known = link?.files || {};
    const local = new Map(listLocal(root, new Set(Object.keys(known))).map((f) => [f.path, blobSha(fs.readFileSync(f.full))]));
    const changed = [...local].filter(([p, sha]) => known[p] && known[p] !== sha).map(([p]) => p);
    const added = [...local.keys()].filter((p) => !known[p]);
    const deleted = Object.keys(known).filter((p) => !local.has(p) && !SKIP.test(p));
    return { local, changed, added, deleted };
  }

  // Dateien, die seit dem letzten Abgleich hier und auf GitHub verschieden geändert wurden.
  // Hochladen würde die Fassung auf GitHub ersetzen.
  conflicts(link, local, remote) {
    const known = link?.files || {};
    return [...local].filter(([p, sha]) => remote.files[p] && remote.files[p] !== sha && remote.files[p] !== known[p] && known[p] !== sha).map(([p]) => p);
  }

  async status(root) {
    const link = this.readLink(root);
    if (!link) return { linked: false, connected: this.connected(), login: this.connected() ? await this.whoami() : '' };
    const { local, changed, added, deleted } = this.localChanges(root, link);
    const remote = await this.remote(link);
    return { linked: true, connected: true, login: await this.whoami(), repo: link.repo, branch: link.branch, url: this.url(link),
      changed, added, deleted, remoteNewer: !!remote.commit && remote.commit !== link.commit, conflicts: this.conflicts(link, local, remote) };
  }

  async create(root, name, { privat = true, beschreibung = '' } = {}) {
    if (this.readLink(root)) throw new Error(`Der Ordner ist schon mit ${this.readLink(root).repo} verbunden.`);
    const repo = await this.request('POST', '/user/repos', { name, private: privat, description: beschreibung, auto_init: true });
    const link = { repo: repo.full_name, branch: repo.default_branch || 'main', commit: null, files: {} };
    // auto_init legt eine README an: zuerst holen, dann den Ordner hochladen
    this.writeLink(root, link);
    await this.pull(root);
    // Die automatische README weicht einer eigenen: Das Repository ist eben erst angelegt worden
    const pushed = await this.push(root, 'Projekt aus ASK hochgeladen', { overwrite: true });
    return { ...pushed, repo: link.repo, url: this.url(link), private: repo.private };
  }

  async clone(root, repoText) {
    const repoName = parseRepo(repoText);
    const existing = this.readLink(root);
    if (existing && existing.repo.toLowerCase() !== repoName.toLowerCase()) {
      throw new Error(`Der Ordner ist schon mit ${existing.repo} verbunden. Für ein anderes Repository einen neuen Ordner nehmen.`);
    }
    const info = await this.request('GET', `/repos/${repoName}`);
    this.writeLink(root, existing || { repo: info.full_name, branch: info.default_branch || 'main', commit: null, files: {} });
    return { ...(await this.pull(root)), repo: info.full_name, url: this.url({ repo: info.full_name }) };
  }

  // Änderungen von GitHub holen. Hier geänderte Dateien werden nie überschrieben (Konflikt).
  async pull(root) {
    const link = this.readLink(root);
    if (!link) throw new Error('Der Ordner ist mit keinem GitHub-Repository verbunden (github_clone oder github_create).');
    const remote = await this.remote(link);
    const { local } = this.localChanges(root, link);
    const known = link.files || {};
    const files = { ...known };
    const written = [];
    const removed = [];
    const conflicts = [];
    for (const [p, sha] of Object.entries(remote.files)) {
      if (SKIP.test(p)) continue;
      // Pfade, die aus dem Ordner hinausführen oder die Windows anders schreibt («.ask./»): nie anlegen
      let full;
      try { full = safeJoin(root, p); } catch { conflicts.push(p); continue; }
      let mine = local.get(p);
      // Liegt hier eine Datei, die der Abgleich nicht kennt (von .gitignore ausgeschlossen): nicht überschreiben
      if (mine === undefined && !known[p]) {
        try { mine = blobSha(fs.readFileSync(full)); } catch { /* gibt es hier nicht */ }
      }
      if (mine === sha) { files[p] = sha; continue; }
      // Hier gelöscht, auf GitHub unverändert: gelöscht lassen (wird beim Hochladen auch dort gelöscht)
      if (mine === undefined && known[p] === sha) continue;
      if (mine === undefined || mine === known[p]) {
        const blob = await this.request('GET', `/repos/${link.repo}/git/blobs/${sha}`, undefined, Math.max(this.timeoutMs, BLOB_MS));
        fs.mkdirSync(path.dirname(full), { recursive: true });
        fs.writeFileSync(full, Buffer.from(blob.content, 'base64'));
        files[p] = sha;
        written.push(p);
      } else if (known[p] !== sha) {
        conflicts.push(p); // hier und auf GitHub verschieden geändert
      }
    }
    // Ohne Commit auf GitHub (leeres Repository) wird hier nie etwas gelöscht
    for (const p of remote.commit ? Object.keys(known) : []) {
      if (remote.files[p] || SKIP.test(p)) continue;
      const mine = local.get(p);
      if (mine === undefined || mine === known[p]) {
        if (mine !== undefined) fs.rmSync(safeJoin(root, p), { force: true });
        delete files[p];
        if (mine !== undefined) removed.push(p);
      } else {
        conflicts.push(p);
      }
    }
    this.writeLink(root, { ...link, commit: remote.commit, files });
    return { written, removed, conflicts, commit: remote.commit };
  }

  // Ordner als Commit hochladen. Gibt es auf GitHub Neueres, zuerst pull. Dateien, die hier und auf
  // GitHub verschieden geändert wurden, ersetzt das Hochladen nur mit overwrite (nach Rückfrage, tools.js).
  async push(root, message, { overwrite = false } = {}) {
    const link = this.readLink(root);
    if (!link) throw new Error('Der Ordner ist mit keinem GitHub-Repository verbunden (github_create oder github_clone).');
    const remote = await this.remote(link);
    if (remote.commit && remote.commit !== link.commit) {
      throw new Error('Auf GitHub gibt es neuere Änderungen. Zuerst github_pull, dann nochmals hochladen.');
    }
    const { local } = this.localChanges(root, link);
    if (!remote.commit) await this.initEmpty(link, root, local);
    const base = remote.commit ? remote : await this.remote(link);
    const clash = this.conflicts(link, local, base);
    if (clash.length && !overwrite) {
      throw Object.assign(new Error(`Hier und auf GitHub verschieden geändert: ${clash.slice(0, 15).join(', ')}. Hochladen würde die Fassung auf GitHub ersetzen. `
        + 'Die Person entscheiden lassen: Soll GitHub gewinnen, die Datei hier löschen und github_pull; soll die Fassung hier gewinnen, github_push mit ueberschreiben: true.'), { conflicts: clash });
    }
    const tree = [];
    let uploaded = 0;
    for (const [p, sha] of local) {
      if (base.files[p] !== sha) {
        const content = fs.readFileSync(path.join(root, ...p.split('/'))).toString('base64');
        await this.request('POST', `/repos/${link.repo}/git/blobs`, { content, encoding: 'base64' }, Math.max(this.timeoutMs, BLOB_MS));
        uploaded++;
      }
      tree.push({ path: p, mode: base.modes[p] || '100644', type: 'blob', sha }); // ausführbar bleibt ausführbar
    }
    // Auf GitHub liegende Dateien in ausgeschlossenen Ordnern (z. B. .github) und Submodule bleiben erhalten
    for (const [p, sha] of Object.entries(base.files)) if (SKIP.test(p)) tree.push({ path: p, mode: base.modes[p] || '100644', type: 'blob', sha });
    tree.push(...base.others);
    // Gelöscht wird auf GitHub nur, was abgeglichen war und hier fehlt. Was der Abgleich nie geholt hat
    // (Konflikt beim Holen, hier von .gitignore ausgeschlossen), bleibt auf GitHub, wie es ist.
    const known = link.files || {};
    const gone = (p) => !SKIP.test(p) && !local.has(p);
    for (const [p, sha] of Object.entries(base.files)) if (gone(p) && !known[p]) tree.push({ path: p, mode: base.modes[p] || '100644', type: 'blob', sha });
    const deleted = Object.keys(base.files).filter((p) => gone(p) && known[p]);
    if (uploaded === 0 && deleted.length === 0) {
      this.writeLink(root, { ...link, commit: base.commit, files: Object.fromEntries(local) });
      return { commit: base.commit, uploaded: 0, deleted: [], nothing: true, url: this.url(link) };
    }
    const newTree = await this.request('POST', `/repos/${link.repo}/git/trees`, { tree });
    const commit = await this.request('POST', `/repos/${link.repo}/git/commits`, { message: message || 'Änderungen aus ASK', tree: newTree.sha, parents: [base.commit] });
    await this.request('PATCH', `/repos/${link.repo}/git/refs/heads/${refPath(link.branch)}`, { sha: commit.sha });
    this.writeLink(root, { ...link, commit: commit.sha, files: Object.fromEntries(local) });
    return { commit: commit.sha, uploaded, deleted, url: `${this.url(link)}/commit/${commit.sha}` };
  }

  // ---------- Branches ----------

  linked(root) {
    const link = this.readLink(root);
    if (!link) throw new Error('Der Ordner ist mit keinem GitHub-Repository verbunden (github_clone oder github_create).');
    return link;
  }

  // Branches des Repositorys (bis 100), dazu der Standard-Branch und der des Ordners
  async branches(root) {
    const link = this.linked(root);
    const [info, list] = await Promise.all([
      this.request('GET', `/repos/${link.repo}`),
      this.request('GET', `/repos/${link.repo}/branches?per_page=100`),
    ]);
    const names = (Array.isArray(list) ? list : []).map((b) => String(b?.name || '')).filter(Boolean);
    if (!names.includes(link.branch)) names.push(link.branch); // noch nie hochgeladen (leeres Repository)
    const def = info?.default_branch || 'main';
    names.sort((a, b) => (a === def ? -1 : b === def ? 1 : a.localeCompare(b, 'de', { numeric: true })));
    return { current: link.branch, default: def, branches: names, url: this.url(link) };
  }

  // Neuer Branch auf dem Stand des letzten Abgleichs, danach ist der Ordner mit ihm verbunden. Änderungen
  // hier bleiben liegen und gehen beim nächsten Hochladen auf den neuen Branch.
  async createBranch(root, name) {
    const link = this.linked(root);
    const branch = String(name || '').trim();
    if (!validBranch(branch)) throw new Error(`«${branch}» geht nicht als Branch-Name. Erlaubt sind Buchstaben, Ziffern, - _ . und /, z. B. «neue-startseite».`);
    if (branch === link.branch) throw new Error(`Der Ordner ist schon mit dem Branch ${branch} verbunden.`);
    if (!link.commit) throw new Error('Auf GitHub gibt es noch keinen Stand, von dem ein Branch abzweigen kann. Zuerst hochladen.');
    try {
      await this.request('POST', `/repos/${link.repo}/git/refs`, { ref: `refs/heads/${branch}`, sha: link.commit });
    } catch (err) {
      if (err.status === 422) throw new Error(`Den Branch ${branch} gibt es auf GitHub schon. Mit «wechseln» zu ihm wechseln.`);
      throw err;
    }
    this.writeLink(root, { ...link, branch });
    return { branch, from: link.branch, commit: link.commit, url: `${this.url(link)}/tree/${refPath(branch)}` };
  }

  // Zu einem anderen Branch wechseln: Die Dateien im Ordner bekommen dessen Stand. Nur ohne Änderungen
  // hier, sonst gingen sie im Stand des anderen Branches unter (oder würden später dorthin hochgeladen).
  async switchBranch(root, name) {
    const link = this.linked(root);
    const branch = String(name || '').trim();
    if (!validBranch(branch)) throw new Error(`«${branch}» geht nicht als Branch-Name.`);
    if (branch === link.branch) return { branch, written: [], removed: [], conflicts: [], same: true };
    const { changed, added, deleted } = this.localChanges(root, link);
    const pending = [...changed, ...added, ...deleted];
    if (pending.length) {
      throw Object.assign(new Error(`Nicht gewechselt: Hier gibt es ${pending.length} ${pending.length === 1 ? 'Änderung' : 'Änderungen'}, die noch nicht hochgeladen ${pending.length === 1 ? 'ist' : 'sind'} (${pending.slice(0, 8).join(', ')}${pending.length > 8 ? ' …' : ''}). `
        + `Beim Wechseln bekäme der Ordner den Stand von ${branch}, und die Änderungen würden vermischt oder gingen später auf den falschen Branch. `
        + `Zuerst auf ${link.branch} hochladen oder dafür einen neuen Branch anlegen.`), { pending });
    }
    // Den Branch muss es geben, sonst sähe der Abgleich ein leeres Repository und würde alles löschen
    try {
      await this.request('GET', `/repos/${link.repo}/git/ref/heads/${refPath(branch)}`);
    } catch (err) {
      if (err.status === 404 || err.status === 409) throw new Error(`Den Branch ${branch} gibt es auf GitHub nicht.`);
      throw err;
    }
    this.writeLink(root, { ...link, branch });
    const r = await this.pull(root);
    return { branch, from: link.branch, ...r };
  }

  // ---------- Pull Requests ----------

  async pullRequests(root) {
    const link = this.linked(root);
    const list = await this.request('GET', `/repos/${link.repo}/pulls?state=open&per_page=30`);
    return (Array.isArray(list) ? list : []).map((p) => ({ number: p.number, title: String(p.title || ''), url: String(p.html_url || ''),
      head: String(p.head?.ref || ''), base: String(p.base?.ref || ''), user: String(p.user?.login || ''), draft: !!p.draft }));
  }

  // Pull Request vom Branch des Ordners in den Standard-Branch (oder base). Enthält nur, was auf GitHub liegt.
  async createPullRequest(root, { title, body = '', base = '' } = {}) {
    const link = this.linked(root);
    const t = String(title || '').trim();
    if (!t) throw new Error('Der Pull Request braucht einen Titel.');
    const target = String(base || '').trim() || (await this.request('GET', `/repos/${link.repo}`)).default_branch || 'main';
    if (!validBranch(target)) throw new Error(`«${target}» geht nicht als Branch-Name.`);
    if (target === link.branch) throw new Error(`Der Ordner ist mit ${target} verbunden, dem Ziel des Pull Requests. Zuerst einen eigenen Branch anlegen, dort hochladen und dann den Pull Request erstellen.`);
    const pr = await this.request('POST', `/repos/${link.repo}/pulls`, { title: t.slice(0, 250), body: String(body || '').slice(0, 20000), head: link.branch, base: target });
    return { number: pr.number, url: pr.html_url, head: link.branch, base: target };
  }

  // In ein leeres Repository lässt sich über die Git-API nicht schreiben: erste Datei über die Contents-API
  async initEmpty(link, root, local) {
    const [first] = [...local.keys()];
    const content = first ? fs.readFileSync(path.join(root, ...first.split('/'))).toString('base64') : Buffer.from('# Projekt\n').toString('base64');
    await this.request('PUT', `/repos/${link.repo}/contents/${encodeURI(first || 'README.md')}`, { message: 'Erste Datei', content, branch: link.branch }, Math.max(this.timeoutMs, BLOB_MS));
  }
}

module.exports = { GitHub, blobSha, listLocal, parseRepo, validLink, validBranch, ignoreRule, ignored, SKIP };
