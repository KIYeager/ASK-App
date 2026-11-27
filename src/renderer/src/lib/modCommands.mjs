// Befehle von Mods (ENTSCHEIDE E72): Vorlage aus mod.json füllen und die Eingabe einem Befehl zuordnen.
// Ohne Abhängigkeiten, damit die Tests sie in Node laden können.

/** Vorlage eines Befehls aus mod.json: {{text}} ist der Rest der Eingabe, sonst kommt er darunter */
export function fillTemplate(prompt, rest) {
  const t = String(rest || '').trim();
  return prompt.includes('{{text}}') ? prompt.replaceAll('{{text}}', t) : (t ? `${prompt}\n\n${t}` : prompt);
}

/** Eingabe «/name Rest» zu einem Befehl eines Mods: { cmd, rest } oder null */
export function modCommandFor(text, commands) {
  const m = /^\/([a-z0-9][a-z0-9-]*)(?:\s+([\s\S]*))?$/i.exec(String(text || '').trim());
  if (!m) return null;
  const cmd = (commands || []).find((c) => c.cmd === `/${m[1].toLowerCase()}` && (c.js || c.prompt));
  return cmd ? { cmd, rest: m[2] || '' } : null;
}
