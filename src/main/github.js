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
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { safeRel, safeJoin, SHA1 } = require('./safe');

const MAX_FILE = 50 * 1024 * 1024; // GitHub lehnt grössere Dateien ab bzw. warnt
const MAX_FILES = 3000;
// Nicht hochladen: Arbeitsdaten der App (Chats, Skripte, Anleitungen) und typische Ballast-Ordner
const SKIP = /^(\.ask|\.git|node_modules|__pycache__|\.venv)(\/|$)/i;
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
  let re = '';
  for (let i = 0; i < p.length; i++) {
    const c = p[i];
    if (c === '*' && p[i + 1] === '*') {
      if (p[i + 2] === '/') { re += '(?:.*/)?'; i += 2; } else { re += '.*'; i += 1; }
    } else if (c === '*') re += '[^/]*';
    else if (c === '?') re += '[^/]';
    else if (c === '[') {
      const end = p.indexOf(']', i + 2);
      if (end < 0) re += '\\[';
      else { re += `[${p.slice(i + 1, end).replace(/^!/, '^').replace(/\\/g, '\\\\')}]`; i = end; }
    } else if (c === '\\' && i + 1 < p.length) { re += p[i + 1].replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); i += 1; }
    else re += c.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  }
  try {
    return { base, negate, dirOnly, re: new RegExp(`^${anchored ? '' : '(?:.*/)?'}${re}$`) };
  } catch {
    return null;
  }
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
    if (r.re.test(r.base ? rel.slice(r.base.length + 1) : rel)) hit = !r.negate;
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

// Gespeicherte Verbindung prüfen: Repository und Branch gehen in Adressen der GitHub-API, die Pfade in Dateinamen.
function validLink(link) {
  if (!link || typeof link !== 'object' || typeof link.repo !== 'string' || !REPO.test(link.repo) || link.repo.split('/').some((part) => /^\.+$/.test(part))) return false;
  if (typeof link.branch !== 'string' || !/^[\w./-]{1,200}$/.test(link.branch) || link.branch.includes('..')) return false;
  if (link.commit !== null && !SHA1.test(String(link.commit))) return false;
  const files = link.files;
  if (!files || typeof files !== 'object' || Array.isArray(files)) return false;
  return Object.entries(files).every(([p, sha]) => safeRel(p) && SHA1.test(String(sha)));
}

class GitHub {
  // store: { get(), set(token) } für das verschlüsselt gespeicherte Token
  // linkFile(root): wo die Verbindung eines Ordners gespeichert ist (in der App ausserhalb des Ordners)
  constructor({ api = 'https://api.github.com', web = 'https://github.com', store, fetchImpl = fetch, linkFile = (root) => path.join(root, LINK) }) {
    this.linkFile = linkFile;
    this.api = api.replace(/\/$/, '');
    this.web = web.replace(/\/$/, '');
    this.store = store;
    this.fetch = fetchImpl;
    this.login = '';
  }

  connected() {
    return !!this.store.get();
  }

  async request(method, url, body) {
    const token = this.store.get();
    if (!token) throw new Error('Nicht mit GitHub verbunden. Die Person kann in der App mit /github ein Token eintragen.');
    const res = await this.fetch(`${this.api}${url}`, {
      method,
      headers: { Authorization: `Bearer ${token}`, Accept: 'application/vnd.github+json', 'X-GitHub-Api-Version': '2022-11-28',
        ...(body ? { 'Content-Type': 'application/json' } : {}) },
      body: body ? JSON.stringify(body) : undefined,
    });
    const text = await res.text();
    let data = null;
    try { data = text ? JSON.parse(text) : null; } catch { /* kein JSON */ }
    if (!res.ok) {
      const message = data?.message || text.slice(0, 200) || `HTTP ${res.status}`;
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
      head = (await this.request('GET', `/repos/${link.repo}/git/ref/heads/${encodeURIComponent(link.branch)}`)).object.sha;
    } catch (err) {
      if (err.status === 404 || err.status === 409) return { commit: null, files: {}, modes: {}, others: [] };
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
      if (mine === undefined || mine === known[p]) {
        const blob = await this.request('GET', `/repos/${link.repo}/git/blobs/${sha}`);
        fs.mkdirSync(path.dirname(full), { recursive: true });
        fs.writeFileSync(full, Buffer.from(blob.content, 'base64'));
        files[p] = sha;
        written.push(p);
      } else if (known[p] !== sha) {
        conflicts.push(p); // hier und auf GitHub verschieden geändert
      }
    }
    for (const p of Object.keys(known)) {
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
        await this.request('POST', `/repos/${link.repo}/git/blobs`, { content, encoding: 'base64' });
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
    await this.request('PATCH', `/repos/${link.repo}/git/refs/heads/${encodeURIComponent(link.branch)}`, { sha: commit.sha });
    this.writeLink(root, { ...link, commit: commit.sha, files: Object.fromEntries(local) });
    return { commit: commit.sha, uploaded, deleted, url: `${this.url(link)}/commit/${commit.sha}` };
  }

  // In ein leeres Repository lässt sich über die Git-API nicht schreiben: erste Datei über die Contents-API
  async initEmpty(link, root, local) {
    const [first] = [...local.keys()];
    const content = first ? fs.readFileSync(path.join(root, ...first.split('/'))).toString('base64') : Buffer.from('# Projekt\n').toString('base64');
    await this.request('PUT', `/repos/${link.repo}/contents/${encodeURI(first || 'README.md')}`, { message: 'Erste Datei', content, branch: link.branch });
  }
}

module.exports = { GitHub, blobSha, listLocal, parseRepo, validLink, ignoreRule, ignored, SKIP };
