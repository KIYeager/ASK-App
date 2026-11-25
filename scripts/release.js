// Neue Version veröffentlichen (ENTSCHEIDE E37, E38):  npm run release -- 0.2.0 "Was ist neu"
// Normalerweise nicht nötig: Beim Merge eines PRs mit Label «beta» oder «release» macht der Workflow das selbst.
// Beta:  npm run release -- 0.3.0-beta.1 "Zum Testen: …"
// Setzt die Version in package.json und package-lock.json, macht einen Commit und einen Tag v<Version>
// mit dem Text und lädt beides hoch. Den Rest (bauen, veröffentlichen in ASK-Releases) macht
// der Workflow .github/workflows/release.yml auf GitHub.
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');

const repo = path.join(__dirname, '..');
const VERSION = /^\d+\.\d+\.\d+(-beta\.\d+)?$/;

const git = (...args) => execFileSync('git', args, { cwd: repo, encoding: 'utf8' }).trim();

function fail(message) {
  console.error(message);
  process.exit(1);
}

// Vergleich nach SemVer für die Formen 1.2.3 und 1.2.3-beta.4
function compare(a, b) {
  const parse = (v) => {
    const [main, pre] = v.split('-beta.');
    return [...main.split('.').map(Number), pre === undefined ? Infinity : Number(pre)];
  };
  const pa = parse(a);
  const pb = parse(b);
  for (let i = 0; i < pa.length; i++) if (pa[i] !== pb[i]) return pa[i] < pb[i] ? -1 : 1;
  return 0;
}

function setVersion(file, version) {
  const full = path.join(repo, file);
  const data = JSON.parse(fs.readFileSync(full, 'utf8'));
  data.version = version;
  if (data.packages?.['']) data.packages[''].version = version; // package-lock.json
  fs.writeFileSync(full, `${JSON.stringify(data, null, 2)}\n`);
}

// Nächste Version beim Merge (Label «beta» oder «release», ENTSCHEIDE E38).
// Ohne Wunsch: 0.2.0 → beta 0.2.1-beta.1 / release 0.2.1; 0.2.1-beta.1 → beta 0.2.1-beta.2 / release 0.2.1.
// Mit Wunsch (Zeile «Version: 0.3.0» im PR): beta 0.3.0-beta.1 (bzw. die nächste Beta von 0.3.0) / release 0.3.0;
// eine volle Beta-Nummer wie 0.3.0-beta.4 gilt so, wie sie ist. Gibt { version } oder { error }.
function nextVersion(current, kind, wanted = '') {
  const m = /^(\d+)\.(\d+)\.(\d+)(?:-beta\.(\d+))?$/.exec(current);
  if (!m) return { error: `Die aktuelle Version ${current} hat nicht die Form 1.2.3 oder 1.2.3-beta.4.` };
  if (kind !== 'beta' && kind !== 'release') return { error: `Unbekannte Release-Art «${kind}».` };
  const base = `${m[1]}.${m[2]}.${m[3]}`;
  const currentBeta = m[4] === undefined ? null : Number(m[4]);
  let version;
  if (wanted) {
    if (!VERSION.test(wanted)) return { error: `«Version: ${wanted}» hat nicht die Form 0.3.0 oder 0.3.0-beta.1.` };
    if (wanted.includes('-')) {
      if (kind === 'release') return { error: `«Version: ${wanted}» ist eine Beta, das Label ist aber «release». Für eine Vollversion ohne «-beta»: Version: ${wanted.split('-')[0]}` };
      version = wanted;
    } else if (kind === 'release') {
      version = wanted;
    } else {
      version = wanted === base && currentBeta !== null ? `${base}-beta.${currentBeta + 1}` : `${wanted}-beta.1`;
    }
  } else if (currentBeta !== null) {
    version = kind === 'beta' ? `${base}-beta.${currentBeta + 1}` : base;
  } else {
    const patch = `${m[1]}.${m[2]}.${Number(m[3]) + 1}`;
    version = kind === 'beta' ? `${patch}-beta.1` : patch;
  }
  if (compare(version, current) <= 0) return { error: `Version ${version} ist nicht grösser als die aktuelle (${current}).` };
  return { version };
}

// Zeilen «Version: …» und «Release-Text: …» aus dem Text eines PRs
function fromPrBody(body = '') {
  const line = (name) => (new RegExp(`^[ \\t>*-]*${name}:[ \\t]*(.*?)[ \\t]*$`, 'im').exec(body)?.[1] || '').replace(/[`*]/g, '').trim();
  return { wanted: line('Version'), text: line('Release-Text') };
}

const currentVersion = () => JSON.parse(fs.readFileSync(path.join(repo, 'package.json'), 'utf8')).version;

// Version setzen, Commit und Tag mit dem Text; push: Zielbranch (z. B. «main») oder der aktuelle Branch.
function publish(version, notes, pushBranch) {
  if (git('status', '--porcelain')) fail('Es gibt noch nicht committete Änderungen. Zuerst committen oder verwerfen.');
  if (git('tag', '-l', `v${version}`)) fail(`Den Tag v${version} gibt es schon.`);
  const branch = pushBranch || git('rev-parse', '--abbrev-ref', 'HEAD');
  setVersion('package.json', version);
  setVersion('package-lock.json', version);
  git('commit', '-am', `Version ${version}`);
  git('tag', '-a', `v${version}`, '-m', notes);
  console.log(`Version ${version} committet, Tag v${version} erstellt. Lade hoch …`);
  // --atomic: Commit und Tag zusammen oder gar nicht
  execFileSync('git', ['push', '--atomic', 'origin', `HEAD:refs/heads/${branch}`, `v${version}`], { cwd: repo, stdio: 'inherit' });
}

// Im Workflow beim Merge: node scripts/release.js --auto <beta|release>, PR in PR_TITLE und PR_BODY.
// Mit --plan nur ausrechnen (Prüfung im PR), ohne Commit.
function auto(kind, planOnly) {
  const { wanted, text } = fromPrBody(process.env.PR_BODY || '');
  const res = nextVersion(currentVersion(), kind, wanted);
  if (res.error) fail(res.error);
  const notes = text || (process.env.PR_TITLE || '').trim() || `Neue Version ${res.version}.`;
  if (process.env.GITHUB_OUTPUT) fs.appendFileSync(process.env.GITHUB_OUTPUT, `version=${res.version}\ntag=v${res.version}\n`);
  if (planOnly) {
    console.log(`Beim Merge wird daraus ASK ${res.version}${kind === 'beta' ? ' (Beta)' : ''}. Text im Update-Dialog: ${notes}`);
    return;
  }
  publish(res.version, notes, 'main');
}

function main() {
  const args = process.argv.slice(2);
  if (args[0] === '--auto' || args[0] === '--plan') return auto(args[1], args[0] === '--plan');
  const [version, ...noteParts] = args;
  const notes = noteParts.join(' ').trim();
  if (!version || !VERSION.test(version)) {
    fail('Aufruf: npm run release -- <Version> "Was ist neu"\nVersion wie 0.2.0 oder für eine Beta 0.3.0-beta.1');
  }
  if (!notes) fail('Bitte kurz angeben, was neu ist. Der Text erscheint im Update-Dialog der App.');
  const current = currentVersion();
  if (compare(version, current) <= 0) fail(`Die neue Version muss grösser sein als die aktuelle (${current}).`);
  publish(version, notes);
  console.log(`\nFertig. GitHub baut jetzt ASK ${version} (Reiter «Actions» im Repository, rund 15 bis 20 Minuten).`);
  console.log(version.includes('-') ? 'Beta: Nur wer «Beta-Versionen erhalten» eingeschaltet hat, bekommt sie angeboten.'
    : 'Danach bieten alle installierten Apps das Update an.');
  return undefined;
}

if (require.main === module) main();

module.exports = { compare, VERSION, nextVersion, fromPrBody };
