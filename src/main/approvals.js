// Rückfrage vor heiklen Schritten (ENTSCHEIDE E33). Das Modell hält an, die Person entscheidet im Chat:
// Erlauben, Immer in diesem Chat, Ablehnen. Stufen in den Einstellungen:
//   aus     nie fragen
//   heikel  vor Löschen, vor Überschreiben vorhandener Dateien und vor Befehlen mit Pfaden ausserhalb des Ordners
//   alle    zusätzlich vor jedem Befehl und jeder Änderung an vorhandenen Dateien
// Erkannt wird, was im Befehl selbst steht. Ein Python-Skript, das intern löscht, fällt nicht darunter;
// dafür bleibt «Rückgängig» (snapshot.js).

const MODES = ['aus', 'heikel', 'alle'];

// Befehle, die Dateien oder Ordner löschen (cmd, PowerShell, Unix-Werkzeuge in Git for Windows)
const DELETE_WORDS = new Set(['del', 'erase', 'rd', 'rmdir', 'rm', 'remove-item', 'ri', 'format', 'cipher', 'sdelete', 'clear-content', 'clc', 'shred']);
// Befehle, die Dateien verschieben oder ohne Rückfrage überschreiben
const MOVE_WORDS = new Set(['move', 'move-item', 'mi', 'mv', 'robocopy', 'set-content', 'sc', 'out-file']);
// Löschen in Python- oder PowerShell-Einzeilern
const DELETE_CALLS = /\b(os\.(remove|unlink|rmdir|removedirs)|shutil\.rmtree|\.unlink\s*\(|\.rmdir\s*\(|send2trash|Remove-Item|\[System\.IO\.(File|Directory)\]::Delete)\b/i;
// Verschieben oder Überschreiben in Python-Einzeilern
const MOVE_CALLS = /\b(shutil\.(move|copy\w*)|os\.(replace|rename)|\.(replace|rename|write_text|write_bytes)\s*\()|\bopen\s*\([^)]*,\s*(?:mode\s*=\s*)?['"][wx]/i;
// Programme, die einen weiteren Befehl ausführen: dieser wird ebenfalls geprüft
const SHELLS = new Set(['cmd', 'powershell', 'pwsh']);
const PASS_THROUGH = new Set(['start', 'call', 'do', 'else', 'then']);
// git-Befehle, die Änderungen im Ordner verwerfen oder Dateien löschen
const GIT_DISCARD = /^\s*git\s+(?:-c\s+\S+\s+)*(?:reset\s+.*--hard|clean\b|checkout\s+(?:.*\s)?(?:--|\.)(?:\s|$)|restore\b|rm\b|stash\s+(?:drop|clear)\b)/i;

// Erstes Wort eines Befehlsteils: «del/q» und «rd/s/q» sind gültiges cmd, der Schalter gehört nicht zum Wort
function firstWord(part) {
  const s = part.trim().replace(/^@/, '');
  const raw = s.startsWith('"') ? s.slice(1).split('"')[0] : s.split(/\s/)[0];
  const word = /^[a-z][\w.-]*\//i.test(raw) ? raw.split('/')[0] : raw.split(/[\\/]/).pop();
  return word.toLowerCase().replace(/\.(exe|cmd|bat|com)$/, '');
}

// Rest nach dem ersten Wort
function afterFirst(part) {
  const s = part.trim().replace(/^@/, '');
  if (s.startsWith('"')) return s.slice(s.indexOf('"', 1) + 1 || s.length);
  const m = /^\S+?(?=\/|\s|$)/.exec(s);
  return s.slice(m ? m[0].length : 0);
}

const unquote = (s) => s.trim().replace(/^"(.*)"$/s, '$1').replace(/^'(.*)'$/s, '$1');

// «if exist a del a», «if not errorlevel 1 …», «if a==b …»: Bedingung überspringen, Befehl prüfen
function afterIf(rest) {
  let t = rest.trim().replace(/^\/i\s+/i, '').replace(/^not\s+/i, '');
  const m = /^(?:(?:exist|errorlevel|defined)\s+(?:"[^"]*"|\S+)|(?:"[^"]*"|\S+?)\s*==\s*(?:"[^"]*"|\S+)|(?:"[^"]*"|\S+)\s+(?:equ|neq|lss|leq|gtr|geq)\s+(?:"[^"]*"|\S+))\s*/i.exec(t);
  return m ? t.slice(m[0].length) : '';
}

// Befehl in Teile zerlegen und jeden Teil untersuchen. Liefert { del, move, unclear, writes: [Ziele von >] }.
function analyze(command, depth = 0) {
  const found = { del: false, move: false, unclear: false, writes: [] };
  const merge = (f) => {
    found.del ||= f.del; found.move ||= f.move; found.unclear ||= f.unclear; found.writes.push(...f.writes);
  };
  const text = String(command);
  if (DELETE_CALLS.test(text)) found.del = true;
  if (MOVE_CALLS.test(text)) found.move = true;
  if (depth > 3) { found.unclear = true; return found; }
  // Umleitung in eine Datei: «> datei» überschreibt sie (nicht «>>», nicht «2>&1», nicht «> nul»)
  for (const m of text.matchAll(/(?<![>\d&])\d?>(?![>&])\s*("[^"]+"|[^\s&|<>()]+)/g)) {
    const target = unquote(m[1]);
    if (!/^(nul|con|\$null|\/dev\/null)$/i.test(target)) found.writes.push(target);
  }
  for (const part of text.split(/[&|;()\n]/)) {
    const word = firstWord(part);
    if (!word) continue;
    let rest = afterFirst(part);
    if (word === 'if') { merge(analyze(afterIf(rest), depth + 1)); continue; }
    if (word === 'for') {
      const m = /\bdo\s+(.*)$/is.exec(rest);
      if (m) merge(analyze(m[1], depth + 1));
      continue;
    }
    if (PASS_THROUGH.has(word)) {
      if (word === 'start') rest = rest.replace(/^\s*(?:"[^"]*"\s*)?(?:\/\w+\s*)*/, ''); // Fenstertitel und Schalter
      merge(analyze(rest, depth + 1));
      continue;
    }
    if (DELETE_WORDS.has(word) || (word === 'find' && /\s-delete\b/.test(rest))) found.del = true;
    if (MOVE_WORDS.has(word)) found.move = true;
    if (word === 'robocopy' && /\/(mir|purge|mov|move)\b/i.test(rest)) found.del = true;
    if ((word === 'copy' || word === 'xcopy' || word === 'copy-item' || word === 'cp' || word === 'cpi')
      && /(^|\s)(\/y|-force|-f)\b/i.test(rest)) found.move = true;
    if (word === 'git' && GIT_DISCARD.test(part.trim().replace(/^@/, ''))) found.del = true;
    // Der Teil wurde auch an Zeichen in Anführungszeichen getrennt: für Shells und Einzeiler den ganzen Rest der Zeile nehmen
    const whole = text.slice(text.indexOf(part) + part.length - rest.length);
    if (SHELLS.has(word)) {
      if (/(^|\s)-(e|ec|enc|encodedcommand)\s/i.test(` ${rest} `)) { found.unclear = true; continue; }
      const m = word === 'cmd' ? /(?:^|\s)\/[ck]\s*(.*)$/is.exec(whole) : /(?:^|\s)-(?:c|command)\s+(.*)$/is.exec(whole);
      if (m) merge(analyze(unquote(m[1]), depth + 1));
      else if (word !== 'cmd' && /(^|\s)-(f|file)\s/i.test(` ${rest} `)) continue; // Skriptdatei: wie python skript.py
    }
    if ((word === 'python' || word === 'python3' || word === 'py' || word === 'node')
      && /(^|\s)-[ce]\s/.test(` ${rest} `) && /\b(os\.system|os\.popen|subprocess|execSync|spawnSync|exec\()/.test(whole)) {
      // Befehle, die der Einzeiler selbst startet: Zeichenketten im Code wie Befehle prüfen
      const code = unquote(whole.replace(/^.*?(?:^|\s)-[ce]\s+/s, ''));
      for (const m of code.matchAll(/(["'])((?:(?!\1).)+)\1/g)) merge(analyze(m[2], depth + 1));
    }
  }
  return found;
}

// Erstes Wort jedes Befehlsteils (getrennt durch &, |, ;, Klammern, Zeilenumbruch)
function commandWords(command) {
  return String(command).split(/[&|;()\n]/).map(firstWord).filter(Boolean);
}

// Pfade, die aus dem Arbeitsordner hinausführen: Laufwerk (C:\…), UNC, «..» als Teil eines Pfads, Pfade ab der
// Wurzel (\Windows, /c/Users), ~ und Umgebungsvariablen, die auf Orte ausserhalb zeigen
const ENV_OUTSIDE = 'userprofile|appdata|localappdata|homepath|homedrive|homeshare|windir|systemroot|systemdrive|programfiles(?:\\(x86\\))?|programw6432|programdata|commonprogramfiles(?:\\(x86\\))?|allusersprofile|public|temp|tmp|onedrive\\w*|userdomain_roamingprofile';
const OUTSIDE = new RegExp([
  String.raw`(^|[\s"'=(<>,;])(?:[a-z]:(?:[\\/]|$|[\s"'])|\\\\[^\\\s]+|\\(?![\\"])\S|\/[a-z]\/[\w.-]{2,}|~(?:[\\/]|$|[\s"']))`,
  String.raw`(^|[\s"'=(<>\\/,;])\.\.(?:[\\/]|$|[\s"';&|)])`,
  `%(?:${ENV_OUTSIDE})%`,
  `\\$env:(?:${ENV_OUTSIDE})\\b`,
  String.raw`\$(?:home|profile|pshome)\b|\$\{env:`,
  String.raw`expanduser\s*\(|Path\.home\s*\(|getenv\s*\(\s*['"](?:${ENV_OUTSIDE})['"]`,
].join('|'), 'i');

function shorten(text, max = 300) {
  const t = String(text || '').replace(/\s+/g, ' ').trim();
  return t.length > max ? `${t.slice(0, max)} …` : t;
}

// Braucht dieser Werkzeugaufruf eine Rückfrage? Liefert { kind, title, detail, reason } oder null.
// ctx: { mode, exists(rel) → bool, madeThisTurn(rel) → bool }. Im Zweifel fragt die App.
function assessRisk(name, args, ctx = {}) {
  if (!args || typeof args !== 'object') args = {};
  const mode = MODES.includes(ctx.mode) ? ctx.mode : 'heikel';
  if (mode === 'aus') return null;
  const exists = ctx.exists || (() => false);
  const made = ctx.madeThisTurn || (() => false);
  // server_starten (ENTSCHEIDE E48) startet einen Befehl wie run_command, er läuft nur weiter
  if (name === 'run_command' || name === 'server_starten') {
    const command = String((name === 'server_starten' ? args.befehl : args.command) || '');
    const found = analyze(command);
    if (found.del) {
      return { kind: 'loeschen', title: 'Darf ASK Dateien löschen?', detail: shorten(command),
        reason: 'Sie kommen in die Sicherung. «Rückgängig» holt sie zurück.' };
    }
    if (OUTSIDE.test(command)) {
      return { kind: 'ausserhalb', title: 'Befehl mit Pfad ausserhalb des Ordners', detail: shorten(command),
        reason: 'Der Befehl nennt einen Ort ausserhalb des Projekts.' };
    }
    if (found.unclear) {
      return { kind: 'unklar', title: 'Befehl, den ASK nicht lesen kann', detail: shorten(command),
        reason: 'Der Befehl ist verschlüsselt oder stark verschachtelt. Was er tut, lässt sich nicht prüfen.' };
    }
    const overwritten = found.writes.filter((t) => { try { return exists(t) && !made(t); } catch { return false; } });
    if (found.move || overwritten.length) {
      return { kind: 'verschieben', title: overwritten.length ? `Darf ASK «${overwritten[0]}» überschreiben?` : 'Darf ASK Dateien verschieben oder überschreiben?',
        detail: shorten(command), reason: 'Der alte Stand kommt in die Sicherung. «Rückgängig» holt ihn zurück.' };
    }
    if (mode === 'alle') return { kind: 'befehl', title: name === 'server_starten' ? 'Server starten?' : 'Befehl ausführen?', detail: shorten(command), reason: '' };
    return null;
  }
  if (name === 'write_file' || name === 'edit_file') {
    const rel = String(args.path || '');
    if (!rel) return null;
    if (OUTSIDE.test(` ${rel}`) || /^[\\/]/.test(rel)) {
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

module.exports = { assessRisk, commandWords, analyze, MODES };
