// Neue Version veröffentlichen (ENTSCHEIDE E33):  npm run release -- 0.2.0 "Was ist neu"
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

function main() {
  const [version, ...noteParts] = process.argv.slice(2);
  const notes = noteParts.join(' ').trim();
  if (!version || !VERSION.test(version)) {
    fail('Aufruf: npm run release -- <Version> "Was ist neu"\nVersion wie 0.2.0 oder für eine Beta 0.3.0-beta.1');
  }
  if (!notes) fail('Bitte kurz angeben, was neu ist. Der Text erscheint im Update-Dialog der App.');
  const current = JSON.parse(fs.readFileSync(path.join(repo, 'package.json'), 'utf8')).version;
  if (compare(version, current) <= 0) fail(`Die neue Version muss grösser sein als die aktuelle (${current}).`);
  if (git('status', '--porcelain')) fail('Es gibt noch nicht committete Änderungen. Zuerst committen oder verwerfen.');
  if (git('tag', '-l', `v${version}`)) fail(`Den Tag v${version} gibt es schon.`);
  const branch = git('rev-parse', '--abbrev-ref', 'HEAD');

  setVersion('package.json', version);
  setVersion('package-lock.json', version);
  git('commit', '-am', `Version ${version}`);
  git('tag', '-a', `v${version}`, '-m', notes);
  console.log(`Version ${version} auf ${branch} committet, Tag v${version} erstellt. Lade hoch …`);
  execFileSync('git', ['push', 'origin', branch, `v${version}`], { cwd: repo, stdio: 'inherit' });
  console.log(`\nFertig. GitHub baut jetzt ASK ${version} (Reiter «Actions» im Repository, rund 15 bis 20 Minuten).`);
  console.log(version.includes('-') ? 'Beta: Nur wer «Beta-Versionen erhalten» eingeschaltet hat, bekommt sie angeboten.'
    : 'Danach bieten alle installierten Apps das Update an.');
}

if (require.main === module) main();

module.exports = { compare, VERSION };
