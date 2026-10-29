# ASK Desktop

ASK Desktop ist eine Windows-App für die Lernenden der TF Bern. Sie ist eine eigene Chat-Oberfläche für ASK (die Open-WebUI-Instanz der Schule), im Stil eines Terminal-Agenten: Man beschreibt einen Auftrag, und das Sprachmodell arbeitet selbständig in einem Ordner auf dem Laptop. Es liest und schreibt Dateien, führt Python aus, führt eine Aufgabenliste, prüft die Ergebnisse und legt fertige Dateien in `ausgabe/` ab. Jeder Schritt erscheint im Chat als Zeile wie `● Befehl(python auswertung.py)` mit der Ausgabe darunter.

Technisch meldet man sich einmal auf der ASK-Seite an (auch über das Microsoft-Konto); danach liest die App das Login-Token und spricht das Modell direkt über die Schnittstelle von ASK an (`/api/chat/completions`, gestreamt, mit Werkzeugliste). Die Werkzeuge führt die App selbst aus, über [open-terminal](https://github.com/open-webui/open-terminal), das lokal auf `127.0.0.1` mit niedriger Integritätsstufe läuft. ASK sieht nur Text; der ASK-Server erreicht den Laptop nie, und ASK braucht keine Freigabe für Tool-Server (ENTSCHEIDE E19).

Weitere Dokumente: [Machbarkeit](docs/MACHBARKEIT.md), [Entscheide](docs/ENTSCHEIDE.md), [Testprotokoll](docs/TESTPROTOKOLL.md).

## Aufbau

| Teil | Ort | Aufgabe |
|---|---|---|
| Hülle | `src/main/` | Electron-Hauptprozess: Fenster, Ordnerwahl und -sperre, Terminal-Verwaltung, Office-Umwandlung |
| Agent | `src/main/askapi.js`, `agent.js`, `tools.js` | Anmeldung und Modell-Anfragen an ASK, Arbeitsschleife, Werkzeuge (Befehl, Lesen, Schreiben, Bearbeiten, Suchen, Aufgaben), Chats in `.ask/chats` |
| Oberfläche | `src/renderer/`, `src/preload/` | Chat mit Eingabe und `/`-Befehlen, Seitenleiste, Dialoge, Anmeldeansicht |
| Terminal-Starter | `python/ask_terminal/` | startet open-terminal mit niedriger Integritätsstufe in einem Job-Objekt, Ordnersperre in der Datei-API, CORS, WebSocket-Anmeldung |
| Anweisungen | `resources/anweisungen/` | Systemprompt für das Modell und Zusatz zur Befehlsbeschreibung |
| Skills | `resources/skills/` | Anleitungen für Word, Excel, PowerPoint, PDF (erstellen und vorhandene Dateien bearbeiten), Hilfsfunktionen `bearbeiten.py`, Prüfskript `pruefen.py`, Platz für Vorlagen in `vorlagen/` |
| Tests | `tests/` | Unit-, Python-, End-to-End- und Pakettests, Beispieldateien |

Die Isolationsvariante steckt in `src/main/backend/native-low.js`. Eine andere Variante (AppContainer, Sandbox, WSL) muss nur `start`, `stop` und `running` gleich anbieten.

## Build

Voraussetzungen auf dem Build-Rechner: Windows 11, Node.js 22 oder neuer, Git, Internetzugang. Ein installiertes Python braucht es nicht.

Wichtig, weil das Repo in OneDrive liegt: `npm install` nie direkt im Projektordner ausführen. npm würde `node_modules` (rund 450 MB) in den OneDrive-Ordner legen. Das Einrichtungsskript installiert alles in einen lokalen Build-Ordner und verlinkt nur `node_modules`.

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
```

Das Skript legt den Build-Ordner an (`%LOCALAPPDATA%\ASK-Desktop-Build`, änderbar mit `-BuildDir` oder `ASK_BUILD_DIR`), installiert die npm-Pakete mit `npm ci`, lädt Python 3.14.8 als offizielles NuGet-Paket (Prüfsumme wird kontrolliert), installiert die Pakete aus `python/requirements.txt` und entfernt Testsuiten. Den Build-Ordner merkt es sich in `.ask-build-dir` (nicht versioniert), damit die folgenden Befehle ihn ohne Umgebungsvariable finden. Dauer beim ersten Mal etwa 5 Minuten.

Die Adresse von ASK ist in `resources/app-config.json` eingebaut, darum fragt die App beim ersten Start nicht danach. Eine in den Einstellungen gespeicherte andere Adresse hat Vorrang. Ändert sich die Adresse, hier anpassen und neu bauen:

```json
{ "askUrl": "https://ask.ict-tfbern.ch" }
```

Danach:

```powershell
npm start
```

```powershell
npm run dist
```

```powershell
npm run dist:portable
```

`npm start` startet die App aus den Quellen. `npm run dist` baut den Installer `ASK-Setup-<Version>.exe` im Unterordner `dist` des Build-Ordners, `npm run dist:portable` die einzelne EXE `ASK-<Version>.exe` ohne Installation. Nur das Python neu bauen: `npm run build:python`. Icon und Startbild entstehen mit `scripts/make-icon.py`.

Tests: siehe [Testprotokoll](docs/TESTPROTOKOLL.md), Abschnitt «Tests wiederholen».

## Installation

`ASK-Setup-<Version>.exe` ausführen. Die Installation läuft pro Nutzer nach `%LOCALAPPDATA%\Programs\ask-desktop`, ohne Admin-Rechte und ohne weitere Fragen, und legt Verknüpfungen im Startmenü und auf dem Desktop an. Python, Bibliotheken und Anleitungen sind dabei; die Lernenden müssen nichts installieren. Deinstallieren über Einstellungen, Apps. Einstellungen bleiben in `%APPDATA%\ASK`, Laufzeitdaten in `%LOCALAPPDATA%\ASK-Desktop`.

Ohne Installation: `ASK-<Version>.exe` irgendwo ablegen und doppelklicken. Beim ersten Start entpackt die EXE die App einmal nach `%LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build>` (Startbild «ASK wird gestartet …»), danach startet sie die App direkt von dort. Eine neuere EXE entpackt sich beim ersten Start neu, die App löscht dann den älteren Stand. Es gibt keine Verknüpfungen und keinen Eintrag unter «Apps». Einstellungen und Laufzeitdaten liegen am gleichen Ort wie bei der installierten App. Installierte App und EXE laufen nicht gleichzeitig.

Installer und EXE sind nicht signiert. Windows SmartScreen kann deshalb beim ersten Start warnen («Weitere Informationen», «Trotzdem ausführen»).

## Bedienung

Beim ersten Start erscheint ein Hinweis in einfachem Deutsch (das Modell kann im Ordner ändern und löschen, darum nur mit Kopien arbeiten, Inhalte gehen an ASK). Danach die Wahl des Arbeitsordners (die ASK-Adresse ist eingebaut): «Neuen Projektordner anlegen» erstellt `%USERPROFILE%\ASK-Projekte\Projekt <Datum>`, oder ein vorhandener Ordner wird gewählt. Ist man nicht bei ASK angemeldet, zeigt die App die Anmeldeseite von ASK. Solange die App prüft, ob man angemeldet ist (beim Start und direkt nach dem Login), zeigt sie stattdessen «Verbinde mit ASK …» bzw. «Anmeldung wird geprüft …»; die Oberfläche von Open WebUI ist dabei nie zu sehen.

Links liegen die Chats, gruppiert nach Arbeitsordner (der aktuelle zuerst, darunter die zuletzt verwendeten), mit «+ Neuer Chat» und einer Suche über die Titel. Ein Klick öffnet einen Chat; liegt er in einem anderen Ordner, wechselt die App dorthin (das Terminal startet neu). Das «+» neben einem Ordnernamen beginnt dort einen neuen Chat, ein Rechtsklick auf einen Chat löscht ihn nach Rückfrage (die Dateien im Ordner bleiben). ☰ blendet die Leiste aus. Oben stehen der Arbeitsordner (Klick öffnet die letzten Ordner und weitere Aktionen), der Schalter «🌐 Web» und die Modellwahl. Mit «Web» sucht ASK vor der ersten Antwort eines Auftrags im Web (mit der Suchmaschine, die in ASK eingerichtet ist) und gibt die Treffer dem Modell mit; die Quellen erscheinen als Links unter der Antwort. Der Schalter ist nur aktiv, wenn ASK die Websuche für das Konto freigibt. Unten das Eingabefeld: Enter sendet, Shift+Enter macht eine neue Zeile, Pfeil nach oben holt die letzte Eingabe, Esc stoppt das Modell. Mit `/` öffnen sich Befehle: `/neu`, `/goal`, `/suchen`, `/ordner`, `/explorer`, `/modell`, `/denken`, `/web`, `/explorer`, `/neustart` (Terminal), `/abmelden`. Darunter die Statuszeile mit Terminal und ASK; ein Klick zeigt Details.

Im Chat erscheinen Antworten als Markdown, mit Kopierknopf unter jeder Antwort und Codeblöcken mit eigenem «Kopieren». Werkzeugaufrufe sind kompakt wie in Claude Code: eine Zeile für den Aufruf (`● Befehl(…)`), eine für das Kurzresultat (`⎿ 84 Zeilen gelesen`, `⎿ +10 −4 Zeilen`, erste Fehlerzeile in Rot); ein Klick zeigt ganzen Befehl, Ausgabe oder Diff. Mehrere Lese- und Suchschritte hintereinander stehen in einer Zeile («2 Dateien gelesen, 1 Ordner angesehen»). Die Gedanken des Modells sind ausgeblendet; Ctrl+O (oder «Ctrl+O Details» unten) zeigt Gedanken und alle Details. In Antworten sind nur Dateien des Ordners farbig hervorgehoben (Klick öffnet die Vorschau), anderer Inline-Code ist dezent. Unter jedem Auftrag steht eine Zeile wie «✻ 7 Schritte · 9’840 Token · 140 Token/s · Modell 70.4 s»; der Tooltip zeigt die Werte pro Schritt. Die Tokenzahlen kommen von ASK; liefert ASK keine, schätzt die App sie aus der Textlänge und schreibt «≈» davor. Ein Dateiname in einer Antwort öffnet per Klick die Vorschau. Die Chats eines Ordners liegen in `.ask/chats`; beim Ordnerwechsel öffnet die App den letzten Chat des neuen Ordners.

Oben neben «Web» öffnet «✻ Standard» den Regler für den Denkaufwand, wie «Reasoning Effort» in ASK: Standard (das Modell entscheidet), Tief, Mittel, Hoch, Maximal. Die App schickt `reasoning_effort` (low, medium, high), ab Hoch zusätzlich den Qwen-Schalter `/think` und `chat_template_kwargs`/`think`; lehnt ASK einen Parameter ab, lässt sie ihn weg.

Mit `/goal <Ziel>` setzt man ein Ziel für den Chat (Leiste «◎ Ziel» über der Eingabe). Das Modell arbeitet dann, bis es das Ziel mit dem Werkzeug `goal_done` als erreicht meldet; hört es vorher auf (etwa nach «Los geht's:»), stösst die App es an («↻ Weiter am Ziel»). Antwortet es zweimal hintereinander ohne Arbeitsschritt, wartet es offenbar auf die Person, und die App hält an. `/goal` zeigt das Ziel, `/goal aus` hebt es auf.

Ist ein Auftrag fertig, klappen seine Arbeitsschritte zu einer Zeile zusammen («▸ 23 Arbeitsschritte · 15 Min. · 2 fehlgeschlagen»); sichtbar bleiben die Schlussantwort, Hinweise, Quellen und die Token-Zeile. Ein Klick klappt die Schritte auf.

Ein Auftrag läuft, bis das Modell fertig ist. Ein Schleifenschutz hält es an, wenn es dreimal hintereinander genau denselben Schritt macht oder acht Werkzeugaufrufe hintereinander scheitern (Notbremse nach 500 Schritten). Befehle, die Programme mit Fenster starten (Word, Excel, PowerPoint, Explorer, Editor, Browser, `start`), lehnt die App ab: sie sperren Dateien und warten auf Eingaben.

Die Seitenleiste zeigt die Aufgabenliste aus `.ask/tasks.json`, die fertigen Dateien in `ausgabe/` (neueste zuerst) und den Inhalt des Arbeitsordners, alles live. Ein Klick zeigt eine Vorschau für Text, Bilder und PDF; «Gross» vergrössert sie. Office-Dateien lassen sich öffnen, im Explorer zeigen oder mit dem installierten Office als PDF anzeigen. Doppelklick öffnet eine Datei im zugehörigen Programm.

Im Arbeitsordner legt die App `ausgabe/` und `.ask/` an. In `.ask/` liegen die Anleitungen (`skills/`), die Aufgabenliste, die Chats (`chats/`), die Hilfsskripte des Modells (`skripte/<Chat>/`) und seine Zwischendateien (`arbeit/<Chat>/`), damit der Arbeitsordner aufgeräumt bleibt; sie verschwinden mit dem Chat und werden nach 14 Tagen automatisch gelöscht. `.ask` ist versteckt (Windows-Attribut wie bei `.git`) und erscheint im Explorer nur mit «Ausgeblendete Elemente», umgewandelte PDFs (`pdf/`) und Aufträge an die App.

Tastenkürzel: Ctrl+Plus/Minus/0 zoomt die Oberfläche, Ctrl+R lädt die Anmeldeseite von ASK neu.

## Bekannte Grenzen

- **Lesen ausserhalb des Ordners.** Windows verhindert Schreiben und Löschen ausserhalb des Arbeitsordners (Low Integrity Level). Lesen kann das Modell über die Shell aber alle Dateien, die der Nutzer lesen darf, und es hat Internetzugang. Die Datei-API von open-terminal ist zusätzlich auf den Ordner gesperrt. Siehe MACHBARKEIT, Abschnitt 5.
- **Originale im Ordner** sind nur durch die Anweisung geschützt, nicht durch Windows (ENTSCHEIDE E14). Darum der Hinweis, mit Kopien zu arbeiten.
- **Werkzeuge unter Low.** Windows PowerShell 5.1 und die Unix-Werkzeuge von Git for Windows starten im Terminal nicht. Das Modell arbeitet mit `cmd.exe` und Python. Office lässt sich nur über die App umwandeln, nicht direkt steuern.
- **Ordner auf FAT/exFAT-USB-Sticks** oder Netzlaufwerken werden abgelehnt, weil sie sich nicht schützen lassen.
- **Abhängigkeit von ASK.** Die App braucht nur, was auch die ASK-Webseite braucht: Anmeldung und Zugriff auf ein Modell. Ob ASK die Werkzeugliste (`tools`) an das Modell weitergibt, ist mit dem echten ASK noch nicht geprüft. Lehnt ASK sie ab oder schreibt das Modell Aufrufe als `<tool_call>` in den Text, wechselt die App von selbst in einen Textmodus und beschreibt die Werkzeuge im Systemprompt.
- **Websuche einmal pro Auftrag.** ASK sucht vor der ersten Antwort; das Modell kann während der Arbeit nicht selbst nochmals suchen. Ob die Websuche über die Schnittstelle bei eurem ASK genauso läuft wie im Browser, ist noch nicht geprüft.
- **Werkzeuge und Wissen aus ASK** (die Werkzeuge und Wissenssammlungen, die einem Modell in ASK zugeordnet sind) stehen in der App nicht zur Verfügung.
- **Chats nicht in ASK.** Die Chats liegen nur im Arbeitsordner (`.ask/chats`), nicht im Verlauf von ASK.
- **Ausgabe von Windows-Programmen.** Programme wie `ping` schreiben in der OEM-Codepage; das Terminal wandelt solche Zeilen um. Gemischte Zeilen können einzelne falsche Zeichen enthalten.
- **Office-Umwandlung** braucht ein installiertes Microsoft Office. Office bekommt immer eine Kopie im Temp-Ordner, nie die Datei im Arbeitsordner; aus der Kopie wird ein Verweis auf eine externe Word-Vorlage entfernt, Inhaltsverzeichnisse werden vor dem Export aktualisiert, und bei einem Verbindungsabbruch versucht die App es einmal neu. Scheitert die Umwandlung, meldet `pruefen.py --pdf` Exitcode 2 und sagt ausdrücklich, dass Seitenzahl und Inhaltsverzeichnis nicht geprüft sind. Ein Office-Fenster, das die Person gerade offen hat, wird nicht geschlossen; eine hängende unsichtbare Office-Instanz wird nach 2 Minuten beendet.
- **Grösse.** Installer rund 190 MB, einzelne EXE rund 240 MB, installiert bzw. entpackt rund 650 MB (davon Python mit pandas und matplotlib rund 290 MB).
- **Einzelne EXE beim ersten Start.** Der erste Start entpackt rund 610 MB, im Test 36 s bis zum laufenden Terminal, spätere Starts 3 s. Der entpackte Stand bleibt liegen wie eine Installation; entfernen lässt er sich nur durch Löschen von `%LOCALAPPDATA%\ASK-Desktop\app`.
- **Interaktive Notebooks** von open-terminal sind abgeschaltet (umgehen die Ordnersperre).

## Offene Punkte

Grundsatzfragen, die nur die Schule bzw. du entscheiden können:

1. **ASK-Administration informieren.** Die App braucht keine Freigabe für Tool-Server mehr, weil sie die Werkzeuge selbst ausführt (E19). Gerade deshalb sollte die ASK-Administration wissen, dass die App das Modell über die Schnittstelle anspricht und auf den Laptops Befehle ausführt. Wenn die Schule das nicht will, ist eine Absprache nötig, bevor die App verteilt wird.
2. **Code-Signatur und Geräteverwaltung.** Mit der Informatik klären, ob per Nutzer installierte Programme unter `AppData` auf den Schullaptops laufen dürfen (AppLocker, Intune) und ob ein Zertifikat für die Signatur verfügbar ist.
3. **Datenschutz.** Inhalte aus dem Arbeitsordner gehen an ASK und das Modell. Ob dafür Regeln für die Lernenden nötig sind, ist eine Schulfrage.
4. **TF-Bern-Vorlagen** für Word, PowerPoint und Excel als `.docx`, `.pptx`, `.xlsx` in `resources/skills/vorlagen/` ablegen.
5. **Ablage des Repos.** Das Repo liegt in OneDrive. Mit `scripts/setup.ps1` bleibt OneDrive frei von Build-Dateien, einfacher wäre ein Repo ausserhalb von OneDrive mit einem Git-Server als Sicherung.

Technische Weiterentwicklungen:

- Mit dem echten ASK und Qwen testen: Werkzeugaufrufe über die Schnittstelle, Qualität der Arbeitsschritte, Anweisungen und Skills nachschärfen (`resources/anweisungen/systemprompt.md` ist bewusst kurz).
- Rückfrage vor heiklen Befehlen (Löschen), falls die Anweisung im Systemprompt nicht reicht.
- Stärkere Isolation mit AppContainer, damit auch Lesen auf den Ordner beschränkt ist.
- Automatische Updates (electron-updater) und eine Versionsanzeige in der App.
- Vorschau für Office-Dateien direkt beim Anklicken, sobald klar ist, dass Office auf allen Laptops vorhanden ist.
