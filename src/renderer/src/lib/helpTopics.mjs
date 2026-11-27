// Inhalt des Fensters «Hilfe» (ENTSCHEIDE E79): kurze Fragen und Antworten für Lernende, nach Themen.
// Die ausführliche Anleitung für das Modell steht im Skill ask-hilfe; wer die Oberfläche ändert, passt beides an.
// `open` nennt einen Ort in der App, den die Antwort mit einem Knopf öffnet (siehe HELP_TARGETS).

export const HELP_TARGETS = {
  tour: 'Einführung starten',
  shortcuts: 'Tastenkombinationen',
  project: 'Projekt anlegen',
  memory: 'Gedächtnis öffnen',
  usage: 'Nutzung öffnen',
  github: 'GitHub öffnen',
  terminal: 'Terminal ansehen',
  'settings:konto': 'Konto öffnen',
  'settings:darstellung': 'Darstellung öffnen',
  'settings:chat': 'Chat-Einstellungen öffnen',
  'settings:modelle': 'Modelle öffnen',
  'settings:benachrichtigungen': 'Benachrichtigungen öffnen',
  'settings:skills': 'Skills öffnen',
  'settings:mods': 'Mods öffnen',
  'settings:ueber': 'Über ASK öffnen',
};

export const HELP_TOPICS = [
  {
    id: 'erste-schritte', title: 'Erste Schritte', icon: 'spark',
    items: [
      { q: 'Was kann ASK?',
        a: 'ASK ist der KI-Assistent der TF Bern. Du kannst Fragen stellen, Texte und Code schreiben lassen und Dateien bearbeiten lassen. Was du eingibst, geht an den ASK-Server der Schule.' },
      { q: 'Chat oder Projekt: was brauche ich?',
        a: 'Für Fragen reicht ein Chat ohne Projekt (Gruppe «Chats» oben links). Soll ASK Dateien anlegen oder ändern, brauchst du ein Projekt, also einen Ordner auf deinem Laptop: «+» bei «Projekte» oder /projekt.',
        open: 'project', keys: 'ordner arbeitsordner neu' },
      { q: 'Wie gebe ich einen Auftrag?',
        a: 'Schreib ins Eingabefeld, was ASK tun soll, und drück Enter. Shift+Enter macht eine neue Zeile. Mit @ nennst du eine Datei, mit / öffnest du die Befehle.',
        keys: 'eingabe senden schreiben befehl' },
      { q: 'Diagramme, Quiz und Rechner im Chat',
        a: 'ASK kann Antworten interaktiv machen: Diagramme, Quiz, Karteikarten, Rechner mit Reglern, Knöpfe zum Auswählen oder kleine Programme ohne Internet. Bitte einfach darum («mach ein Quiz dazu»). Ausschalten: Einstellungen → Chat → Interaktive Antworten.',
        open: 'settings:chat', keys: 'interaktiv intelligent ui diagramm quiz rechner karteikarten widget' },
      { q: 'Wo sehe ich, was ASK gerade macht?',
        a: 'Unter deiner Eingabe zeigt ASK jeden Arbeitsschritt. Ein Klick auf einen Schritt zeigt Befehl, Ausgabe oder Änderung. Ctrl+O zeigt alle Details.',
        keys: 'arbeitsschritte details' },
      { q: 'Die kurze Einführung nochmals ansehen',
        a: 'Die Einführung zeigt in vier Karten, wo was ist.', open: 'tour', keys: 'tour' },
    ],
  },
  {
    id: 'modus', title: 'Modus und Rückfragen', icon: 'shield',
    items: [
      { q: 'Was heissen Auto, Nachfragen und Plan?',
        a: 'Auto: ASK arbeitet selbst und fragt nur bei Heiklem nach. Nachfragen: ASK fragt vor jedem Befehl und jeder Änderung. Plan: ASK ändert nichts, sondern legt zuerst einen Plan vor; «Umsetzen» führt ihn aus. Wechseln: im Eingabefeld oder mit Shift+Tab.',
        keys: 'auto nachfragen plan modus' },
      { q: 'ASK fragt mich etwas. Wie antworte ich?',
        a: 'Klick eine der Antworten auf der Karte an oder schreib eine eigene ins Feld darunter. Eine Eingabe im normalen Eingabefeld gilt auch als Antwort.',
        keys: 'frage rückfrage antworten' },
      { q: 'Was bedeuten «Erlauben», «Immer in diesem Chat» und «Ablehnen»?',
        a: 'Vor heiklen Schritten (zum Beispiel Löschen) fragt ASK um Erlaubnis. «Immer in diesem Chat» erlaubt solche Schritte bis zum Ende des Chats. Bei «Ablehnen» macht ASK ohne den Schritt weiter.',
        keys: 'erlaubnis löschen sicherheit' },
      { q: 'Wie halte ich ASK an?',
        a: 'Esc oder der Stopp-Knopf ■ neben dem Eingabefeld. Eine wartende Nachricht kommt dann zurück ins Eingabefeld.',
        keys: 'stoppen abbrechen anhalten esc' },
      { q: 'Modell, Denkaufwand und Web',
        a: 'Im Eingabefeld wählst du rechts das Modell. Die Glühbirne stellt ein, wie gründlich ASK überlegt. «Web» lässt ASK vor der Antwort im Internet suchen, sofern die Schule es freigibt.',
        keys: 'modell denken glühbirne websuche internet' },
    ],
  },
  {
    id: 'dateien', title: 'Dateien und Rückgängig', icon: 'undo',
    items: [
      { q: 'Wo sehe ich, was ASK geändert hat?',
        a: 'Rechts im Reiter «Dateien». Was der letzte Auftrag angelegt oder geändert hat, ist mit «neu» oder «geändert» markiert. Unter dem Auftrag steht zum Beispiel «Dateien: 2 neu, 1 geändert».',
        keys: 'dateiliste änderungen neu geändert' },
      { q: 'ASK hat etwas Falsches gemacht. Wie mache ich es rückgängig?',
        a: 'Klick unter dem Auftrag auf «Rückgängig». Der Ordner geht auf den Stand vor dem Auftrag zurück, neue Dateien werden gelöscht. Offene Word-, Excel- oder PowerPoint-Dateien vorher schliessen. Die Sicherung gilt 14 Tage.',
        keys: 'zurück undo wiederherstellen fehler' },
      { q: 'Eingabe ändern oder nochmals senden',
        a: 'Fahr mit der Maus über deine letzte Eingabe: «Bearbeiten» oder «Neu senden». ASK fragt, ob die Dateien auf den Stand davor zurück sollen.',
        keys: 'bearbeiten neu senden korrigieren' },
      { q: 'Bilder und Dokumente anhängen',
        a: 'Mit Ctrl+V einfügen, ins Fenster ziehen oder die Büroklammer im Eingabefeld benutzen (höchstens 10 Dateien, je bis 50 MB).',
        keys: 'anhang anhänge bild bildschirmfoto büroklammer hochladen' },
      { q: 'Was sind Docs?',
        a: 'Ein Doc ist ein Dokument, das ASK für dich schreibt, zum Beispiel ein Bericht, ein Lernzettel oder ein Brief. Es steht rechts im Reiter «Docs». Sag im Chat «Schreib ein Doc über …» und schau zu, wie ASK es füllt. Danach bearbeitest du es dort selbst wie in Word.',
        keys: 'doc docs dokument bericht lernzettel brief claude docs schreiben' },
      { q: 'Ein Doc kommentieren oder als Word speichern',
        a: 'Markiere im Doc eine Stelle, klick auf «Kommentar» und schreib, was ASK tun soll («kürzer», «Quelle?»). «An ASK senden» schickt den Kommentar an ASK; ASK ändert die Stelle und antwortet im Kommentar. Mit dem Pfeil oben rechts speicherst du das Doc als Word, PDF oder Markdown.',
        keys: 'doc kommentar kommentieren export word docx pdf markdown abgeben speichern' },
      { q: 'Im Doc etwas anstreichen oder erklären lassen',
        a: 'Markiere Text im Doc und klick mit der rechten Maustaste darauf: «Erklären», «Grafik dazu», «Einfacher», «Beispiel» oder «Bitte an ASK …» gehen an ASK, «Anstreichen» markiert die Stelle farbig. Oder oben im Doc auf «Anstreichen» (der Pfeil daneben wählt die Farbe) bzw. auf «ASK». Der Stern ganz oben gibt dir Feedback von ASK als Kommentare.',
        keys: 'anstreichen markieren leuchtstift farbe erklären grafik diagramm bild feedback vorschläge' },
      { q: 'Ein Doc mit anderen zusammen schreiben',
        a: 'Öffne das Doc und klick oben auf den Personen-Knopf. Wähle einen Ordner, den alle haben (OneDrive, Teams oder ein Laufwerk der Schule), und gib ihn frei. Die anderen klicken in der Liste der Docs auf den Personen-Knopf und wählen den Ordner «….askdoc». Danach schreibt ihr gleichzeitig; bei OneDrive kommen Änderungen nach ein paar Sekunden an.',
        keys: 'teilen geteilt zusammen gruppe gruppenarbeit live mitschreiben onedrive teams laufwerk' },
      { q: 'Eine Datei ansehen',
        a: 'Klick rechts auf die Datei und dann «Vorschau» oder «Öffnen». Word, Excel und PowerPoint zeigt die Vorschau nur, wenn Office installiert ist.',
        keys: 'vorschau öffnen pdf word excel powerpoint' },
    ],
  },
  {
    id: 'chats', title: 'Chats, Projekte und Tabs', icon: 'chat',
    items: [
      { q: 'Neuer Chat, während ein anderer arbeitet',
        a: '«Neuer Chat» oben links oder Ctrl+N. Der andere Chat arbeitet im Hintergrund weiter; ein Kreisel in der Liste zeigt das.',
        keys: 'neu parallel gleichzeitig hintergrund' },
      { q: 'Chat umbenennen, anheften, verschieben oder exportieren',
        a: 'Rechtsklick auf den Chat in der Liste oder «…» daneben. Dort gibt es auch «Löschen».',
        keys: 'umbenennen anheften verschieben exportieren löschen menü' },
      { q: 'Etwas in den Chats suchen',
        a: 'Das Suchfeld oben links findet Titel und Inhalte aller Chats. Im offenen Chat sucht Ctrl+F.',
        keys: 'suchen finden' },
      { q: 'Der Chat ist sehr lang geworden',
        a: 'Der Ring im Eingabefeld zeigt, wie voll der Chat für das Modell ist. Wird es knapp, fasst ASK ältere Aufträge selbst zusammen. Sofort geht es mit /zusammenfassen.',
        keys: 'kontext lang ring zusammenfassen voll' },
      { q: 'Ein Ziel für den Chat setzen',
        a: '/goal und dein Ziel: ASK arbeitet, bis es das Ziel erreicht hat. /goal aus hebt es auf.',
        keys: 'goal ziel' },
    ],
  },
  {
    id: 'erweitern', title: 'Gedächtnis, Skills und Mods', icon: 'bolt',
    items: [
      { q: 'Was merkt sich ASK über mich?',
        a: 'ASK merkt sich Vorlieben und Wissen, das auch später nützt («Gemerkt: …» im Chat). «Merk dir …» legt etwas ab, «vergiss …» löscht es. Alles liegt nur auf diesem Laptop.',
        open: 'memory', keys: 'gedächtnis merken vergessen' },
      { q: 'Was sind Skills?',
        a: 'Anleitungen für Aufträge, die oft vorkommen, zum Beispiel /code-erklaeren oder /lernjournal. ASK benutzt passende Skills von selbst; mit /name rufst du einen direkt auf.',
        open: 'settings:skills', keys: 'skill anleitung katalog' },
      { q: 'Was sind Mods?',
        a: 'Mods ändern ASK selbst: Farben, Figuren, eigene Befehle. Neue Mods sind aus, bis du sie nach einer Vorschau einschaltest. Ctrl+Shift+M schaltet alle Mods aus.',
        open: 'settings:mods', keys: 'mod aussehen design erweiterung' },
      { q: 'GitHub verbinden',
        a: 'Mit /github oder Profil → GitHub ein Token eintragen. Danach kann ASK Projekte holen und hochladen; Hochladen bestätigst du jedes Mal.',
        open: 'github', keys: 'github repository hochladen push' },
    ],
  },
  {
    id: 'einstellungen', title: 'Einstellungen und Updates', icon: 'settings',
    items: [
      { q: 'Hell oder dunkel einstellen',
        a: 'Einstellungen → Darstellung → Farbschema: System, Hell oder Dunkel.',
        open: 'settings:darstellung', keys: 'dunkel hell farbe design dark mode' },
      { q: 'Schrift grösser oder kleiner',
        a: 'Ctrl+Plus macht alles grösser, Ctrl+Minus kleiner, Ctrl+0 setzt zurück.',
        keys: 'zoom grösse schrift' },
      { q: 'Meldung, wenn ASK fertig ist',
        a: 'Einstellungen → Benachrichtigungen: Windows meldet sich, wenn ASK fertig ist oder etwas fragt und du in einem anderen Fenster bist.',
        open: 'settings:benachrichtigungen', keys: 'benachrichtigung meldung fertig' },
      { q: 'Updates und Beta-Versionen',
        a: 'ASK sucht selbst nach neuen Versionen und fragt dann. Von Hand: Profil → Nach Updates suchen oder /update. Beta-Versionen bringen Neues früher, können aber Fehler haben.',
        open: 'settings:ueber', keys: 'update version beta aktualisieren' },
      { q: 'Wie viel habe ich mit ASK gearbeitet?',
        a: 'Die Nutzung zeigt Aufträge, Token und Dateien der letzten 30 Tage. Gezählt wird nur auf diesem Laptop.',
        open: 'usage', keys: 'nutzung statistik token' },
    ],
  },
  {
    id: 'probleme', title: 'Probleme lösen', icon: 'alert',
    items: [
      { q: '«Nicht angemeldet» oder die Anmeldeseite erscheint',
        a: 'Einstellungen → Konto → «Anmelden». Lädt die Seite nicht, hilft «Im Browser …» oder Ctrl+R.',
        open: 'settings:konto', keys: 'anmelden login konto passwort' },
      { q: '«Verbindung zu ASK unterbrochen»',
        a: 'ASK versucht es selbst bis zu fünfmal. Hilft das nicht, prüf das Internet und sende später nochmals.',
        keys: 'verbindung internet offline fehler' },
      { q: 'Das Terminal läuft nicht',
        a: 'Klick auf den Hinweis unten links für Details. /neustart startet das Terminal neu.',
        open: 'terminal', keys: 'terminal befehle neustart' },
      { q: '«Python wird eingerichtet» beim ersten Start',
        a: 'Das ist normal und passiert nur einmal. Je nach Laptop dauert es einige Minuten.',
        keys: 'python einrichten langsam start' },
      { q: 'Ein Chat reagiert nicht mehr',
        a: 'Drück Esc oder ■. ASK hält auch selbst an, wenn es mehrmals denselben Schritt macht.',
        keys: 'hängt reagiert nicht stoppen' },
      { q: 'Windows fragt plötzlich nach einer Bestätigung',
        a: 'Wahrscheinlich läuft ein Mod mit Code (im Profilmenü steht «Code an»). Dann kommen heikle Bestätigungen aus Sicherheitsgründen von Windows. Soll das aufhören: Mod ausschalten.',
        open: 'settings:mods', keys: 'windows bestätigung mod code' },
      { q: 'Ein Fehler steht im Chat',
        a: 'Die Meldung sagt, was zu tun ist. «Details kopieren» kopiert die Angaben für deine Lehrperson.',
        keys: 'fehler meldung lehrperson' },
    ],
  },
];

// Für die Suche: klein, ohne Akzente, ß als ss
export function fold(s) {
  return String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '').replace(/ß/g, 'ss');
}

// Themen mit den Fragen, in denen alle Wörter der Suche vorkommen (in Frage, Antwort, Stichwörtern oder Titel des Themas)
export function searchHelp(query, topics = HELP_TOPICS) {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (!words.length) return topics;
  return topics
    .map((t) => ({ ...t, items: t.items.filter((it) => {
      const hay = fold(`${t.title} ${it.q} ${it.a} ${it.keys || ''}`);
      return words.every((w) => hay.includes(w));
    }) }))
    .filter((t) => t.items.length);
}
