// Rückfrage vor heiklen Schritten (src/main/approvals.js)
const test = require('node:test');
const assert = require('node:assert');
const { assessRisk, commandWords } = require('../src/main/approvals');

const cmd = (command, mode = 'heikel') => assessRisk('run_command', { command }, { mode });

test('Löschbefehle werden erkannt, auch nach & und in Python oder PowerShell', () => {
  for (const c of ['del alt.txt', 'echo x & del /q *.tmp', 'rmdir /s /q build', 'rd build', 'rm -rf out', 'Remove-Item a.txt',
    'powershell -c "Remove-Item a.txt"', 'python -c "import os; os.remove(\'a.txt\')"', 'python -c "import shutil; shutil.rmtree(\'x\')"',
    '"C:\\Windows\\System32\\del.exe" x']) {
    assert.strictEqual(cmd(c)?.kind, 'loeschen', c);
  }
  for (const c of ['python auswertung.py', 'dir', 'echo deleted', 'type readme.md', 'python -c "print(1)"', 'git status']) {
    assert.strictEqual(cmd(c), null, c);
  }
});

test('Pfade ausserhalb des Ordners', () => {
  for (const c of ['type C:\\Users\\x\\geheim.txt', 'copy a.txt ..\\b.txt', 'dir %USERPROFILE%', 'type \\\\server\\share\\a']) {
    assert.strictEqual(cmd(c)?.kind, 'ausserhalb', c);
  }
  assert.strictEqual(cmd('python skript.py --out ergebnis.xlsx'), null);
});

test('Stufen aus und alle', () => {
  assert.strictEqual(cmd('del a.txt', 'aus'), null);
  assert.strictEqual(cmd('python x.py', 'alle')?.kind, 'befehl');
  assert.strictEqual(cmd('python x.py', 'unbekannt'), null); // unbekannt gilt als «heikel»
});

test('Überschreiben vorhandener Dateien, ausser das Modell hat sie in diesem Auftrag erstellt', () => {
  const ctx = (exists, made = false, mode = 'heikel') => ({ mode, exists: () => exists, madeThisTurn: () => made });
  assert.strictEqual(assessRisk('write_file', { path: 'bericht.docx' }, ctx(true))?.kind, 'ueberschreiben');
  assert.strictEqual(assessRisk('write_file', { path: 'neu.txt' }, ctx(false)), null);
  assert.strictEqual(assessRisk('write_file', { path: 'neu.txt' }, ctx(true, true)), null);
  assert.strictEqual(assessRisk('edit_file', { path: 'a.py' }, ctx(true)), null);
  assert.strictEqual(assessRisk('edit_file', { path: 'a.py' }, ctx(true, false, 'alle'))?.kind, 'aendern');
  assert.strictEqual(assessRisk('write_file', { path: 'C:\\temp\\x.txt' }, ctx(false))?.kind, 'ausserhalb');
  assert.strictEqual(assessRisk('read_file', { path: 'a.txt' }, ctx(true, false, 'alle')), null);
});

test('commandWords', () => {
  assert.deepStrictEqual(commandWords('@echo off && del "a b.txt" | more'), ['echo', 'del', 'more']);
});
