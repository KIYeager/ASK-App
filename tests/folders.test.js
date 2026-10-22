// Tests der Ordnersperre (node --test tests/).
const test = require('node:test');
const assert = require('node:assert');
const { checkFolder, REASON } = require('../src/main/folders');

const env = {
  USERPROFILE: 'C:\\Users\\lea',
  APPDATA: 'C:\\Users\\lea\\AppData\\Roaming',
  LOCALAPPDATA: 'C:\\Users\\lea\\AppData\\Local',
  OneDrive: 'C:\\Users\\lea\\OneDrive - ICT TFBERN',
  OneDriveCommercial: 'C:\\Users\\lea\\OneDrive - ICT TFBERN',
  SystemRoot: 'C:\\WINDOWS',
  ProgramFiles: 'C:\\Program Files',
  'ProgramFiles(x86)': 'C:\\Program Files (x86)',
  ProgramData: 'C:\\ProgramData',
};

function ctx(extra = {}) {
  return {
    env,
    syncRoots: ['C:\\Users\\lea\\ICT TFBERN\\Klasse 1A - Dokumente'],
    realpath: (p) => extra.links?.[p] || p,
    stat: () => ({ isDirectory: () => true }),
    ...extra,
  };
}

const reason = (p, c = ctx()) => checkFolder(p, c).reason;

test('normaler Projektordner ist erlaubt', () => {
  const r = checkFolder('C:\\Users\\lea\\ASK-Projekte\\Bericht', ctx());
  assert.deepStrictEqual(r, { ok: true, path: 'C:\\Users\\lea\\ASK-Projekte\\Bericht' });
  assert.ok(checkFolder('D:\\Schule\\Projekt', ctx()).ok);
});

test('OneDrive über Umgebungsvariable, Namen und Registry', () => {
  assert.strictEqual(reason('C:\\Users\\lea\\OneDrive - ICT TFBERN\\Dokumente'), REASON.onedrive);
  assert.strictEqual(reason('c:\\users\\lea\\onedrive - ict tfbern'), REASON.onedrive);
  assert.strictEqual(reason('D:\\Backup\\OneDrive-alt\\x'), REASON.onedrive);
  assert.strictEqual(reason('C:\\Users\\lea\\ICT TFBERN\\Klasse 1A - Dokumente\\Abgaben'), REASON.onedrive);
});

test('Junction in OneDrive wird aufgelöst', () => {
  const c = ctx({ links: { 'C:\\Users\\lea\\Projekt': 'C:\\Users\\lea\\OneDrive - ICT TFBERN\\Projekt' } });
  assert.strictEqual(reason('C:\\Users\\lea\\Projekt', c), REASON.onedrive);
});

test('Ordner, der OneDrive enthält', () => {
  assert.strictEqual(reason('C:\\Users\\lea\\ICT TFBERN'), REASON.containsSync);
});

test('Laufwerkswurzeln, Profil und Benutzerordner', () => {
  assert.strictEqual(reason('C:\\'), REASON.root);
  assert.strictEqual(reason('D:'), REASON.root);
  assert.strictEqual(reason('C:\\Users\\lea'), REASON.profile);
  assert.strictEqual(reason('C:\\Users\\lea\\'), REASON.profile);
  assert.strictEqual(reason('C:\\Users'), REASON.usersRoot);
});

test('AppData, Windows und Programme', () => {
  assert.strictEqual(reason('C:\\Users\\lea\\AppData\\Local\\Temp\\x'), REASON.appdata);
  assert.strictEqual(reason('C:\\Windows\\System32'), REASON.system);
  assert.strictEqual(reason('C:\\Program Files\\ASK'), REASON.system);
  assert.strictEqual(reason('C:\\Program Files (x86)'), REASON.system);
  assert.strictEqual(reason('C:\\ProgramData\\x'), REASON.system);
});

test('Netzwerkpfade und fehlende Ordner', () => {
  assert.strictEqual(reason('\\\\server\\freigabe\\x'), REASON.network);
  const missing = ctx({ stat: () => { throw new Error('ENOENT'); } });
  assert.strictEqual(reason('C:\\gibtsnicht', missing), REASON.missing);
  const file = ctx({ stat: () => ({ isDirectory: () => false }) });
  assert.strictEqual(reason('C:\\Users\\lea\\a.txt', file), REASON.notDir);
});
