// Tests für das Aufräumen alter Stände der einzelnen EXE (node --test tests/).
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { appCacheRoot, staleAppDirs, MIN_AGE_MS } = require('../src/main/portable');

function rootWith(dirs, files = []) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'ask-portable-'));
  for (const name of dirs) fs.mkdirSync(path.join(root, name, 'resources'), { recursive: true });
  for (const name of files) fs.writeFileSync(path.join(root, name), '');
  return root;
}

function found(root, own, now) {
  return staleAppDirs(root, own, now).map((d) => path.basename(d)).sort();
}

const later = () => Date.now() + MIN_AGE_MS + 60000;

test('Ordner unter %LOCALAPPDATA%\\ASK-Desktop\\app', () => {
  assert.strictEqual(appCacheRoot({ LOCALAPPDATA: 'C:\\Users\\lea\\AppData\\Local' }),
    path.join('C:\\Users\\lea\\AppData\\Local', 'ASK-Desktop', 'app'));
  assert.strictEqual(appCacheRoot({}), '');
});

test('findet ältere Stände, lässt den eigenen stehen', () => {
  const root = rootWith(['0.1.0-aaa', '0.1.0-bbb', '0.2.0-ccc']);
  try {
    assert.deepStrictEqual(found(root, '0.2.0-ccc'), ['0.1.0-aaa', '0.1.0-bbb']);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('eigener Stand auch bei anderer Schreibweise', () => {
  const root = rootWith(['0.2.0-CCC']);
  try {
    assert.deepStrictEqual(found(root, '0.2.0-ccc'), []);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('abgebrochenes Entpacken (.neu) erst nach einer Stunde', () => {
  const root = rootWith(['0.2.0-ccc', '0.2.0-ddd.neu']);
  try {
    assert.deepStrictEqual(found(root, '0.2.0-ccc'), []);
    assert.deepStrictEqual(found(root, '0.2.0-ccc', later()), ['0.2.0-ddd.neu']);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('Dateien bleiben stehen', () => {
  const root = rootWith(['0.2.0-ccc'], ['notiz.txt']);
  try {
    assert.deepStrictEqual(found(root, '0.2.0-ccc', later()), []);
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test('fehlender Ordner ergibt keine Treffer', () => {
  assert.deepStrictEqual(staleAppDirs(path.join(os.tmpdir(), 'gibt-es-nicht-ask'), 'x', later()), []);
});
