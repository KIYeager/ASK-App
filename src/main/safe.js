// Pfade aus Dateien, die das Modell ändern kann (.ask im Arbeitsordner) oder die von aussen kommen
// (GitHub), sind nicht vertrauenswürdig: Der Hauptprozess läuft mit normalen Rechten, das Terminal
// mit niedrigen. Jeder solche Pfad wird hier geprüft, bevor der Hauptprozess damit schreibt.
const fs = require('node:fs');
const path = require('node:path');

const SHA1 = /^[0-9a-f]{40}$/;

// Relativer Pfad mit / als Trenner, der sicher im Ordner bleibt: keine leeren Teile, kein «..»,
// kein Laufwerk oder Datenstrom (:), kein Rückstrich, keine Steuerzeichen. Teile, die auf Punkt
// oder Leerzeichen enden, lehnt Windows nicht ab, sondern kürzt sie («.ask.» wird «.ask»).
function safeRel(rel) {
  if (typeof rel !== 'string' || !rel || rel.length > 1000) return false;
  if (/[\\:\u0000-\u001f<>"|?*]/.test(rel) || rel.startsWith('/')) return false;
  return rel.split('/').every((part) => part && part !== '.' && part !== '..' && !/[. ]$/.test(part));
}

// Absoluter Pfad zu rel im Ordner root. Fehler, wenn rel nicht sicher ist oder ein vorhandener
// Teil des Wegs (Verknüpfung, Junction) aus dem Ordner hinausführt.
function safeJoin(root, rel) {
  if (!safeRel(rel)) throw new Error(`Unsicherer Pfad: ${String(rel).slice(0, 120)}`);
  const full = path.join(root, ...rel.split('/'));
  const base = fs.realpathSync.native(root).toLowerCase();
  let existing = full;
  for (;;) {
    try {
      const real = fs.realpathSync.native(existing).toLowerCase();
      if (real !== base && !real.startsWith(base + path.sep)) throw new Error(`Der Pfad führt aus dem Ordner hinaus: ${rel.slice(0, 120)}`);
      return full;
    } catch (err) {
      if (err.code !== 'ENOENT' && err.code !== 'ENOTDIR') throw err;
      const parent = path.dirname(existing);
      if (parent === existing) throw err;
      existing = parent; // noch nicht vorhanden: den nächsten vorhandenen Ordner prüfen
    }
  }
}

module.exports = { safeRel, safeJoin, SHA1 };
