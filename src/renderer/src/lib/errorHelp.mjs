// Was bei einem Fehler zu tun ist (ENTSCHEIDE E69): aus dem Text der Meldung eine kurze Erklärung in einfachen
// Worten und zwei, drei Schritte. null, wenn nichts Passendes bekannt ist (dann bleibt nur die Meldung).
const CASES = [
  {
    test: /context.?length|maximum context|zu lang|too many tokens|token limit|context window/i,
    title: 'Der Chat ist zu lang für dieses Modell.',
    steps: ['Einen neuen Chat beginnen (Ctrl+N) und kurz sagen, worum es geht.', 'Oder unten rechts ein Modell mit mehr Platz wählen.'],
  },
  {
    test: /\b401\b|\b403\b|nicht angemeldet|anmeldung|token (abgelaufen|ungültig)|invalid token|abgelaufen|unauthori[sz]ed|forbidden/i,
    title: 'Die Anmeldung bei ASK ist abgelaufen.',
    steps: ['Unten links auf «Nicht angemeldet» oder «ASK: Fehler» klicken.', 'Neu anmelden, dann den Auftrag nochmals senden.'],
  },
  {
    test: /\b429\b|rate.?limit|too many requests|zu viele anfragen/i,
    title: 'ASK bekommt gerade zu viele Anfragen.',
    steps: ['Eine Minute warten.', 'Dann den Auftrag nochmals senden.'],
  },
  {
    test: /\b50[0-4]\b|overloaded|überlastet|bad gateway|service unavailable|internal server error|gateway/i,
    title: 'ASK hat gerade ein Problem.',
    steps: ['Ein paar Minuten warten und nochmals senden.', 'Hält es an: der Lehrperson melden (Knopf «Details kopieren»).'],
  },
  {
    test: /timeout|zeitüberschreitung|timed out|verbindung|connect|dns|resolve|nicht erreichbar|network|netzwerk|offline|ENOTFOUND|ECONN|certificate|zertifikat/i,
    title: 'Keine Verbindung zu ASK.',
    steps: ['WLAN prüfen: Bist du im Schulnetz oder mit dem Internet verbunden?', 'ask.ict-tfbern.ch im Browser öffnen: Lädt die Seite?', 'Dann hier nochmals senden oder unten links «Erneut prüfen».'],
  },
  {
    test: /terminal|python|befehl.*(startet|läuft) nicht|low integrity/i,
    title: 'Das Terminal von ASK läuft nicht.',
    steps: ['Unten links auf «Terminal: Fehler» klicken und «Neu starten».', 'Prüfen, ob der Projektordner noch da ist.', 'Hilft das nicht: ASK schliessen und neu öffnen.'],
  },
];

export function errorHelp(text) {
  const t = String(text || '');
  if (!t) return null;
  const hit = CASES.find((c) => c.test.test(t));
  return hit ? { title: hit.title, steps: hit.steps } : null;
}

// Text für «Details kopieren»: was die Lehrperson oder der Support braucht
export function errorDetails(text, state) {
  const lines = [
    `ASK ${state?.version || ''}`.trim(),
    `Zeit: ${new Date().toLocaleString('de-CH')}`,
    state?.askUrl ? `ASK-Adresse: ${state.askUrl}` : '',
    state?.ask?.state ? `Anmeldung: ${state.ask.state}${state.ask.message ? ` (${state.ask.message})` : ''}` : '',
    state?.terminal?.state ? `Terminal: ${state.terminal.state}${state.terminal.error ? ` (${state.terminal.error})` : ''}` : '',
    state?.model ? `Modell: ${state.model}` : '',
    `Windows: ${typeof navigator !== 'undefined' ? navigator.userAgent : ''}`,
    '',
    `Meldung: ${text}`,
  ];
  return lines.filter((l, i) => l || i === lines.length - 2).join('\n');
}
