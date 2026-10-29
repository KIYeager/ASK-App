// Tests für die Sperre von Programmen mit Fenster in run_command (node --test tests/).
const test = require('node:test');
const assert = require('node:assert');
const { blockedCommand } = require('../src/main/tools');

test('Programme mit Fenster werden abgelehnt', () => {
  for (const command of [
    'start ausgabe\\bericht.docx',
    'winword ausgabe\\bericht.docx',
    '"C:\\Program Files\\Microsoft Office\\root\\Office16\\WINWORD.EXE" /q bericht.docx',
    'dir & start .',
    'excel.exe a.xlsx',
    'python a.py && POWERPNT vortrag.pptx',
    'explorer ausgabe',
  ]) assert.ok(blockedCommand(command), command);
});

test('normale Befehle laufen', () => {
  for (const command of [
    'python .ask/skripte/abc/excel.py',
    'python .ask/skills/pruefen.py ausgabe/Auswertung.xlsx --pdf',
    'echo start',
    'tasklist | findstr WINWORD',
    'python restart.py',
    'dir C:\\Windows\\explorer.exe',
    'type ausgabe\\notepad.txt',
  ]) assert.strictEqual(blockedCommand(command), '', command);
});
