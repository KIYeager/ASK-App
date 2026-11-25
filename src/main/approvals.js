// Rückfrage vor heiklen Schritten (ENTSCHEIDE E33). Das Modell hält an, die Person entscheidet im Chat:
// Erlauben, Immer in diesem Chat, Ablehnen. Stufen in den Einstellungen:
//   aus     nie fragen
//   heikel  vor Löschen, vor Überschreiben vorhandener Dateien und vor Befehlen mit Pfaden ausserhalb des Ordners
//   alle    zusätzlich vor jedem Befehl und jeder Änderung an vorhandenen Dateien
// Erkannt wird, was im Befehl selbst steht. Ein Python-Skript, das intern löscht, fällt nicht darunter;
// dafür bleibt «Rückgängig» (snapshot.js).

const MODES = ['aus', 'heikel', 'alle'];

// Befehle, die Dateien oder Ordner löschen (cmd, PowerShell, Unix-Werkzeuge in Git for Windows)
const DELETE_WORDS = new Set(['del', 'erase', 'rd', 'rmdir', 'rm', 'remove-item', 'ri', 'rmdir.exe', 'format', 'cipher', 'sdelete']);
// Löschen in Python- oder PowerShell-Einzeilern
const DELETE_CALLS = /\b(os\.(remove|unlink|rmdir|removedirs)|shutil\.rmtree|\.unlink\s*\(|\.rmdir\s*\(|send2trash|Remove-Item|\[System\.IO\.(File|Directory)\]::Delete)\b/i;

// Erstes Wort jedes Befehlsteils (getrennt durch &, |, ;, Klammern, Zeilenumbruch)
function commandWords(command) {
  return String(command).split(/[&|;()\n]/).map((part) => {
    const s = part.trim().replace(/^@/, '');
    const word = s.startsWith('"') ? s.slice(1).split('"')[0] : s.split(/\s/)[0];
    return word.split(/[\\/]/).pop().toLowerCase().replace(/\.(exe|cmd|bat)$/, '');
  }).filter(Boolean);
}

// Pfade, die aus dem Arbeitsordner hinausführen: Laufwerk (C:\…), UNC, ..\, %USERPROFILE% und ähnliche
const OUTSIDE = /(^|[\s"'=(])(?:[a-z]:[\\/]|\\\\[^\\\s]+|\.\.[\\/]|~[\\/]|%(?:userprofile|appdata|localappdata|homepath|windir|systemroot|programfiles|temp)%|\$env:(?:userprofile|appdata|localappdata|windir|temp))/i;

function shorten(text, max = 300) {
  const t = String(text || '').replace(/\s+/g, ' ').trim();
  return t.length > max ? `${t.slice(0, max)} …` : t;
}

// Braucht dieser Werkzeugaufruf eine Rückfrage? Liefert { kind, title, detail, reason } oder null.
// ctx: { mode, exists(rel) → bool, madeThisTurn(rel) → bool }
function assessRisk(name, args = {}, ctx = {}) {
  const mode = MODES.includes(ctx.mode) ? ctx.mode : 'heikel';
  if (mode === 'aus') return null;
  const exists = ctx.exists || (() => false);
  const made = ctx.madeThisTurn || (() => false);
  // server_starten (ENTSCHEIDE E48) startet einen Befehl wie run_command, er läuft nur weiter
  if (name === 'run_command' || name === 'server_starten') {
    const command = String((name === 'server_starten' ? args.befehl : args.command) || '');
    const words = commandWords(command);
    if (words.some((w) => DELETE_WORDS.has(w)) || DELETE_CALLS.test(command)) {
      return { kind: 'loeschen', title: 'Darf ASK Dateien löschen?', detail: shorten(command),
        reason: 'Sie kommen in die Sicherung. «Rückgängig» holt sie zurück.' };
    }
    if (OUTSIDE.test(command)) {
      return { kind: 'ausserhalb', title: 'Befehl mit Pfad ausserhalb des Ordners', detail: shorten(command),
        reason: 'Der Befehl nennt einen Ort ausserhalb des Projekts.' };
    }
    if (mode === 'alle') return { kind: 'befehl', title: name === 'server_starten' ? 'Server starten?' : 'Befehl ausführen?', detail: shorten(command), reason: '' };
    return null;
  }
  if (name === 'write_file' || name === 'edit_file') {
    const rel = String(args.path || '');
    if (!rel) return null;
    if (OUTSIDE.test(` ${rel}`)) {
      return { kind: 'ausserhalb', title: 'Datei ausserhalb des Ordners?', detail: rel, reason: 'Der Pfad liegt ausserhalb des Projekts.' };
    }
    if (!exists(rel) || made(rel)) return null;
    if (name === 'write_file') {
      return { kind: 'ueberschreiben', title: `Darf ASK «${rel}» überschreiben?`, detail: rel,
        reason: 'Der alte Stand kommt in die Sicherung.' };
    }
    if (mode === 'alle') return { kind: 'aendern', title: `Darf ASK «${rel}» ändern?`, detail: rel, reason: '' };
  }
  return null;
}

module.exports = { assessRisk, commandWords, MODES };
