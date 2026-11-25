// Nachbau der GitHub-REST-API, die src/main/github.js benutzt (Repositories, Git-Daten, Contents,
// Branches, Pull Requests).
// Alles im Speicher. Für Unit-Tests (tests/github.test.js) und den Mock von ASK (mock-ask.js).
const crypto = require('node:crypto');

const TOKEN = 'gh-test-token';
const LOGIN = 'lea-test';

function createMockGitHub() {
  const repos = new Map();
  const sha1 = (text) => crypto.createHash('sha1').update(text).digest('hex');
  const blobSha = (buf) => crypto.createHash('sha1').update(`blob ${buf.length}\0`).update(buf).digest('hex');

  function commitFiles(repo, files, message, parents) {
    const entries = Object.entries(files).map(([p, sha]) => ({ path: p, mode: '100644', type: 'blob', sha }));
    const tree = sha1(JSON.stringify(entries));
    repo.trees.set(tree, entries);
    const sha = sha1(`${tree}${message}${parents.join()}${Math.random()}`);
    repo.commits.set(sha, { tree, parents, message });
    return sha;
  }

  function addBlob(repo, buf) {
    const sha = blobSha(buf);
    repo.blobs.set(sha, buf);
    return sha;
  }

  // Liefert { status, body } oder null, wenn der Pfad nicht zur API gehört.
  function handle(method, url, headers, body) {
    if (headers.authorization !== `Bearer ${TOKEN}`) return { status: 401, body: { message: 'Bad credentials' } };
    const u = new URL(url, 'http://x');
    const p = u.pathname;
    let m;
    if (method === 'GET' && p === '/user') return { status: 200, body: { login: LOGIN } };
    if (method === 'POST' && p === '/user/repos') {
      const name = `${LOGIN}/${body.name}`;
      if (repos.has(name)) return { status: 422, body: { message: 'name already exists on this account' } };
      const repo = { name, private: body.private !== false, branch: 'main', blobs: new Map(), trees: new Map(), commits: new Map(), refs: {}, pulls: [] };
      repos.set(name, repo);
      if (body.auto_init) repo.refs.main = commitFiles(repo, { 'README.md': addBlob(repo, Buffer.from(`# ${body.name}\n`)) }, 'Initial commit', []);
      return { status: 201, body: { full_name: name, default_branch: 'main', private: repo.private } };
    }
    if ((m = /^\/repos\/([^/]+\/[^/]+)(\/.*)?$/.exec(p))) {
      const repo = repos.get(decodeURIComponent(m[1]));
      if (!repo) return { status: 404, body: { message: 'Not Found' } };
      const rest = m[2] || '';
      if (method === 'GET' && rest === '') return { status: 200, body: { full_name: repo.name, default_branch: repo.branch, private: repo.private } };
      if ((m = /^\/git\/ref\/heads\/(.+)$/.exec(rest)) && method === 'GET') {
        const sha = repo.refs[decodeURIComponent(m[1])];
        if (!sha) return Object.keys(repo.refs).length ? { status: 404, body: { message: 'Not Found' } } : { status: 409, body: { message: 'Git Repository is empty.' } };
        return { status: 200, body: { object: { sha } } };
      }
      if (method === 'GET' && rest === '/branches') {
        return { status: 200, body: Object.entries(repo.refs).map(([name, sha]) => ({ name, commit: { sha }, protected: false })) };
      }
      if (rest === '/git/refs' && method === 'POST') {
        const ref = /^refs\/heads\/(.+)$/.exec(body.ref || '');
        if (!ref) return { status: 422, body: { message: 'Reference name is invalid' } };
        if (repo.refs[ref[1]]) return { status: 422, body: { message: 'Reference already exists' } };
        if (!repo.commits.has(body.sha)) return { status: 422, body: { message: 'Object does not exist' } };
        repo.refs[ref[1]] = body.sha;
        return { status: 201, body: { ref: body.ref, object: { sha: body.sha } } };
      }
      if (rest === '/pulls' && method === 'GET') {
        const state = u.searchParams.get('state') || 'open';
        return { status: 200, body: repo.pulls.filter((pr) => state === 'all' || pr.state === state) };
      }
      if (rest === '/pulls' && method === 'POST') {
        const invalid = (message) => ({ status: 422, body: { message: 'Validation Failed', errors: [{ resource: 'PullRequest', code: 'custom', message }] } });
        if (!repo.refs[body.head] || !repo.refs[body.base]) return invalid(`${repo.refs[body.head] ? 'base' : 'head'} invalid`);
        if (repo.refs[body.head] === repo.refs[body.base]) return invalid(`No commits between ${body.base} and ${body.head}`);
        if (repo.pulls.some((pr) => pr.state === 'open' && pr.head.ref === body.head && pr.base.ref === body.base)) return invalid(`A pull request already exists for ${LOGIN}:${body.head}.`);
        const number = repo.pulls.length + 1;
        const pr = { number, state: 'open', title: body.title, body: body.body, draft: false, html_url: `https://github.com/${repo.name}/pull/${number}`,
          head: { ref: body.head }, base: { ref: body.base }, user: { login: LOGIN } };
        repo.pulls.push(pr);
        return { status: 201, body: pr };
      }
      if ((m = /^\/git\/refs\/heads\/(.+)$/.exec(rest)) && method === 'PATCH') {
        if (!repo.commits.has(body.sha)) return { status: 422, body: { message: 'Object does not exist' } };
        repo.refs[decodeURIComponent(m[1])] = body.sha;
        return { status: 200, body: { object: { sha: body.sha } } };
      }
      if ((m = /^\/git\/trees\/([0-9a-f]+)$/.exec(rest)) && method === 'GET') {
        const commit = repo.commits.get(m[1]);
        const entries = repo.trees.get(commit ? commit.tree : m[1]);
        if (!entries) return { status: 404, body: { message: 'Not Found' } };
        return { status: 200, body: { sha: commit ? commit.tree : m[1], tree: entries, truncated: false } };
      }
      if (rest === '/git/blobs' && method === 'POST') return { status: 201, body: { sha: addBlob(repo, Buffer.from(body.content, 'base64')) } };
      if ((m = /^\/git\/blobs\/([0-9a-f]+)$/.exec(rest)) && method === 'GET') {
        const blob = repo.blobs.get(m[1]);
        return blob ? { status: 200, body: { content: blob.toString('base64'), encoding: 'base64' } } : { status: 404, body: { message: 'Not Found' } };
      }
      if (rest === '/git/trees' && method === 'POST') {
        const missing = body.tree.find((e) => !repo.blobs.has(e.sha));
        if (missing) return { status: 422, body: { message: `tree.sha ${missing.sha} is not a valid blob` } };
        const sha = sha1(JSON.stringify(body.tree));
        repo.trees.set(sha, body.tree);
        return { status: 201, body: { sha } };
      }
      if (rest === '/git/commits' && method === 'POST') {
        const sha = sha1(`${body.tree}${body.message}${body.parents.join()}${Math.random()}`);
        repo.commits.set(sha, { tree: body.tree, parents: body.parents, message: body.message });
        return { status: 201, body: { sha } };
      }
      if ((m = /^\/contents\/(.+)$/.exec(rest)) && method === 'PUT') {
        const branch = body.branch || repo.branch;
        const head = repo.refs[branch];
        const files = {};
        if (head) for (const e of repo.trees.get(repo.commits.get(head).tree)) files[e.path] = e.sha;
        files[decodeURIComponent(m[1])] = addBlob(repo, Buffer.from(body.content, 'base64'));
        repo.refs[branch] = commitFiles(repo, files, body.message, head ? [head] : []);
        return { status: 201, body: { commit: { sha: repo.refs[branch] } } };
      }
    }
    return { status: 404, body: { message: 'Not Found' } };
  }

  // Für Tests: aktueller Inhalt einer Datei auf GitHub (Standard-Branch oder branch) und Anzahl Commits
  function fileOnGitHub(name, file, branch) {
    const repo = repos.get(name);
    const head = repo?.refs[branch || repo.branch];
    const entry = head && repo.trees.get(repo.commits.get(head).tree).find((e) => e.path === file);
    return entry ? repo.blobs.get(entry.sha).toString('utf8') : null;
  }

  function commitCount(name) {
    const repo = repos.get(name);
    let n = 0;
    for (let sha = repo?.refs[repo.branch]; sha; sha = repo.commits.get(sha).parents[0]) n++;
    return n;
  }

  // Eine Änderung direkt auf GitHub (wie aus einer anderen Sitzung), auf dem Standard-Branch oder branch
  function commitOnGitHub(name, file, text, branch) {
    const repo = repos.get(name);
    const b = branch || repo.branch;
    const head = repo.refs[b];
    const files = {};
    for (const e of repo.trees.get(repo.commits.get(head).tree)) files[e.path] = e.sha;
    files[file] = addBlob(repo, Buffer.from(text));
    repo.refs[b] = commitFiles(repo, files, `Änderung an ${file}`, [head]);
  }

  return { handle, fileOnGitHub, commitCount, commitOnGitHub, repos };
}

module.exports = { createMockGitHub, TOKEN, LOGIN };
