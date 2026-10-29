// GitHub-Projekte holen und hochladen: Repository klonen, Änderungen holen (pull) und als Commit
// hochladen (push), dazu ein neues Repository anlegen. Ohne git auf dem Laptop (unter Low Integrity
// laufen die Git-Werkzeuge nicht zuverlässig), direkt über die REST-API von GitHub mit einem
// persönlichen Token. Das Token kennt nur der Hauptprozess, nie das Modell oder das Terminal.
//
// Der letzte abgeglichene Stand liegt in .ask/github.json (Repository, Branch, Commit und die
// Prüfsumme jeder Datei). Damit erkennt der Abgleich, was hier, was auf GitHub und was an beiden
// Orten geändert wurde; Dateien mit Änderungen an beiden Orten werden nie überschrieben.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');

const MAX_FILE = 50 * 1024 * 1024; // GitHub lehnt grössere Dateien ab bzw. warnt
const MAX_FILES = 3000;
// Nicht hochladen: Arbeitsdaten der App (Chats, Skripte, Anleitungen) und typische Ballast-Ordner
const SKIP = /^(\.ask|\.git|node_modules|__pycache__|\.venv)(\/|$)/i;
const LINK = path.join('.ask', 'github.json');

// Git-Prüfsumme einer Datei, wie git hash-object
function blobSha(buffer) {
  return crypto.createHash('sha1').update(`blob ${buffer.length}\0`).update(buffer).digest('hex');
}

function listLocal(root) {
  const out = [];
  const walk = (dir) => {
    for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      const rel = path.relative(root, full).split(path.sep).join('/');
      if (SKIP.test(rel) || entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) walk(full);
      else if (entry.isFile()) {
        if (out.length >= MAX_FILES) throw new Error(`Mehr als ${MAX_FILES} Dateien im Ordner. Das ist für den Abgleich zu viel.`);
        const size = fs.statSync(full).size;
        if (size > MAX_FILE) throw new Error(`${rel} ist grösser als 50 MB und kann nicht auf GitHub.`);
        out.push({ path: rel, full });
      }
    }
  };
  walk(root);
  return out;
}

function parseRepo(text) {
  const m = /(?:github\.com[/:])?([\w.-]+)\/([\w.-]+?)(?:\.git)?\/?$/i.exec(String(text || '').trim());
  if (!m) throw new Error('Repository als «besitzer/name» oder als GitHub-Adresse angeben.');
  return `${m[1]}/${m[2]}`;
}

class GitHub {
  // store: { get(), set(token) } für das verschlüsselt gespeicherte Token
  constructor({ api = 'https://api.github.com', web = 'https://github.com', store, fetchImpl = fetch }) {
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
      return JSON.parse(fs.readFileSync(path.join(root, LINK), 'utf8'));
    } catch {
      return null;
    }
  }

  writeLink(root, link) {
    fs.mkdirSync(path.join(root, '.ask'), { recursive: true });
    fs.writeFileSync(path.join(root, LINK), JSON.stringify(link, null, 2));
  }

  url(link) {
    return `${this.web}/${link.repo}`;
  }

  // Stand auf GitHub: Commit und Dateien (Pfad → Prüfsumme). Leeres Repository: commit null.
  async remote(link) {
    let head;
    try {
      head = (await this.request('GET', `/repos/${link.repo}/git/ref/heads/${encodeURIComponent(link.branch)}`)).object.sha;
    } catch (err) {
      if (err.status === 404 || err.status === 409) return { commit: null, files: {} };
      throw err;
    }
    const tree = await this.request('GET', `/repos/${link.repo}/git/trees/${head}?recursive=1`);
    if (tree.truncated) throw new Error('Das Repository ist für den Abgleich zu gross.');
    const files = {};
    for (const e of tree.tree) if (e.type === 'blob') files[e.path] = e.sha;
    return { commit: head, files };
  }

  // Was hat sich seit dem letzten Abgleich hier geändert?
  localChanges(root, link) {
    const local = new Map(listLocal(root).map((f) => [f.path, blobSha(fs.readFileSync(f.full))]));
    const known = link?.files || {};
    const changed = [...local].filter(([p, sha]) => known[p] && known[p] !== sha).map(([p]) => p);
    const added = [...local.keys()].filter((p) => !known[p]);
    const deleted = Object.keys(known).filter((p) => !local.has(p) && !SKIP.test(p));
    return { local, changed, added, deleted };
  }

  async status(root) {
    const link = this.readLink(root);
    if (!link) return { linked: false, connected: this.connected(), login: this.connected() ? await this.whoami() : '' };
    const { changed, added, deleted } = this.localChanges(root, link);
    const remote = await this.remote(link);
    return { linked: true, connected: true, login: await this.whoami(), repo: link.repo, branch: link.branch, url: this.url(link),
      changed, added, deleted, remoteNewer: !!remote.commit && remote.commit !== link.commit };
  }

  async create(root, name, { privat = true, beschreibung = '' } = {}) {
    if (this.readLink(root)) throw new Error(`Der Ordner ist schon mit ${this.readLink(root).repo} verbunden.`);
    const repo = await this.request('POST', '/user/repos', { name, private: privat, description: beschreibung, auto_init: true });
    const link = { repo: repo.full_name, branch: repo.default_branch || 'main', commit: null, files: {} };
    // auto_init legt eine README an: zuerst holen, dann den Ordner hochladen
    this.writeLink(root, link);
    await this.pull(root);
    const pushed = await this.push(root, 'Projekt aus ASK hochgeladen');
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
      const mine = local.get(p);
      if (mine === sha) { files[p] = sha; continue; }
      if (mine === undefined || mine === known[p]) {
        const blob = await this.request('GET', `/repos/${link.repo}/git/blobs/${sha}`);
        const full = path.join(root, ...p.split('/'));
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
        if (mine !== undefined) fs.rmSync(path.join(root, ...p.split('/')), { force: true });
        delete files[p];
        if (mine !== undefined) removed.push(p);
      } else {
        conflicts.push(p);
      }
    }
    this.writeLink(root, { ...link, commit: remote.commit, files });
    return { written, removed, conflicts, commit: remote.commit };
  }

  // Ordner als Commit hochladen. Gibt es auf GitHub Neueres, zuerst pull.
  async push(root, message) {
    const link = this.readLink(root);
    if (!link) throw new Error('Der Ordner ist mit keinem GitHub-Repository verbunden (github_create oder github_clone).');
    const remote = await this.remote(link);
    if (remote.commit && remote.commit !== link.commit) {
      throw new Error('Auf GitHub gibt es neuere Änderungen. Zuerst github_pull, dann nochmals hochladen.');
    }
    const { local } = this.localChanges(root, link);
    if (!remote.commit) await this.initEmpty(link, root, local);
    const base = remote.commit ? remote : await this.remote(link);
    const tree = [];
    let uploaded = 0;
    for (const [p, sha] of local) {
      if (base.files[p] !== sha) {
        const content = fs.readFileSync(path.join(root, ...p.split('/'))).toString('base64');
        await this.request('POST', `/repos/${link.repo}/git/blobs`, { content, encoding: 'base64' });
        uploaded++;
      }
      tree.push({ path: p, mode: '100644', type: 'blob', sha });
    }
    // Auf GitHub liegende Dateien in ausgeschlossenen Ordnern (z. B. .github) bleiben erhalten
    for (const [p, sha] of Object.entries(base.files)) if (SKIP.test(p)) tree.push({ path: p, mode: '100644', type: 'blob', sha });
    const deleted = Object.keys(base.files).filter((p) => !SKIP.test(p) && !local.has(p));
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

module.exports = { GitHub, blobSha, listLocal, parseRepo, SKIP };
