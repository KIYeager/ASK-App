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

test('server_starten wird wie ein Befehl geprüft', () => {
  assert.strictEqual(assessRisk('server_starten', { befehl: 'npm run dev' }, { mode: 'heikel' }), null);
  assert.strictEqual(assessRisk('server_starten', { befehl: 'del /q dist & npm run dev' }, { mode: 'heikel' })?.kind, 'loeschen');
  assert.strictEqual(assessRisk('server_starten', { befehl: 'python -m http.server -d C:\\Users' }, { mode: 'heikel' })?.kind, 'ausserhalb');
  assert.strictEqual(assessRisk('server_starten', { befehl: 'npm run dev' }, { mode: 'alle' })?.title, 'Server starten?');
});

test('Löschen in Schaltern, Wrappern, Bedingungen, Schleifen und git wird erkannt', () => {
  for (const c of ['del/q f', 'rd/s/q src', 'rmdir/s x', 'cmd /c del x', 'cmd /c "del x"', 'powershell -c "ri x"', 'pwsh -Command Remove-Item x',
    'if exist a del a', 'if not exist b (rd /s /q c)', 'for %f in (*.tmp) do del %f', 'start "" cmd /c del x', 'call del x', 'find . -delete',
    'git clean -fdx', 'git reset --hard', 'git reset --hard HEAD~1', 'git checkout -- .', 'git checkout .', 'git restore a.py', 'git rm a.py',
    'robocopy a b /mir', 'robocopy a b /move', 'python -c "import os; os.system(\'rm -r x\')"',
    'python -c "import subprocess; subprocess.run([\'cmd\', \'/c\', \'del x\'])"']) {
    assert.strictEqual(cmd(c)?.kind, 'loeschen', c);
  }
  for (const c of ['git status', 'git checkout main', 'git commit -m "rd notes"', 'echo "del x"', 'dir /s /b', 'findstr /s foo *.py',
    'python -c "import os; print(os.getcwd())"', 'node -e "console.log(1)"']) {
    assert.strictEqual(cmd(c), null, c);
  }
});

test('Verschieben, Überschreiben und unlesbare Befehle', () => {
  for (const c of ['move a b', 'mv a b', 'copy /y a b', 'xcopy /y a b', 'robocopy a b', 'Move-Item a b',
    'python -c "open(\'a.txt\', \'w\').write(\'x\')"', 'python -c "import shutil; shutil.move(\'a\', \'b\')"']) {
    assert.strictEqual(cmd(c)?.kind, 'verschieben', c);
  }
  assert.strictEqual(cmd('pwsh -enc SQBFAFgA')?.kind, 'unklar');
  // Umleitung: nur, wenn die Datei schon da ist und das Modell sie nicht selbst geschrieben hat
  const ctx = (exists, made = false) => ({ mode: 'heikel', exists: (p) => exists && p === 'main.py', madeThisTurn: () => made });
  assert.strictEqual(assessRisk('run_command', { command: 'echo x > main.py' }, ctx(true))?.kind, 'verschieben');
  assert.strictEqual(assessRisk('run_command', { command: 'echo x > main.py' }, ctx(false)), null);
  assert.strictEqual(assessRisk('run_command', { command: 'echo x > main.py' }, ctx(true, true)), null);
  for (const c of ['echo x >> main.py', 'python x.py 2>&1', 'python x.py > nul', 'copy a.txt b.txt']) {
    assert.strictEqual(assessRisk('run_command', { command: c }, ctx(true)), null, c);
  }
});

test('Weitere Pfade ausserhalb des Ordners', () => {
  for (const c of ['type sub\\..\\..\\x', 'echo hi >C:\\Users\\a\\x', 'cd ..', 'cd .. && dir', 'type \\Windows\\win.ini', 'ls /c/Users/x',
    'cat $HOME/x', 'dir %ProgramData%', 'dir %HOMEDRIVE%', 'dir %OneDrive%', 'type $env:APPDATA\\x', 'cd ~', 'dir C:',
    'python -c "import os; print(os.path.expanduser(\'~\'))"']) {
    assert.strictEqual(cmd(c)?.kind, 'ausserhalb', c);
  }
  for (const c of ['rd /s/q build', 'python skript.py --out ergebnis.xlsx', 'echo %cd%', 'dir /s/b']) {
    assert.notStrictEqual(cmd(c)?.kind, 'ausserhalb', c);
  }
  const w = (p) => assessRisk('write_file', { path: p }, { mode: 'heikel' })?.kind ?? null;
  assert.strictEqual(w('sub/../../x'), 'ausserhalb');
  assert.strictEqual(w('/etc/x'), 'ausserhalb');
  assert.strictEqual(w('\\Windows\\x'), 'ausserhalb');
  assert.strictEqual(w('ordner/a.txt'), null);
});

test('Argumente null oder kein Objekt werfen nicht', () => {
  assert.strictEqual(assessRisk('run_command', null, { mode: 'heikel' }), null);
  assert.strictEqual(assessRisk('write_file', 'x', { mode: 'heikel' }), null);
});
