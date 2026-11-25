# ASK Desktop

ASK Desktop ist eine Windows-App für die Lernenden der TF Bern. Sie ist eine eigene Chat-Oberfläche für ASK (die Open-WebUI-Instanz der Schule), im Stil eines Terminal-Agenten: Man beschreibt einen Auftrag, und das Sprachmodell arbeitet selbständig in einem Ordner auf dem Laptop. Es liest und schreibt Dateien, führt Python aus, führt eine Aufgabenliste, prüft die Ergebnisse und arbeitet dabei direkt in den Dateien des Ordners, wie Claude Code in einem Projekt. Jeder Auftrag lässt sich rückgängig machen. Jeder Schritt erscheint im Chat als Zeile wie `● Befehl(python auswertung.py)` mit der Ausgabe darunter.

Technisch meldet man sich einmal auf der ASK-Seite an (auch über das Microsoft-Konto); danach liest die App das Login-Token und spricht das Modell direkt über die Schnittstelle von ASK an (`/api/chat/completions`, gestreamt, mit Werkzeugliste). Die Werkzeuge führt die App selbst aus, über [open-terminal](https://github.com/open-webui/open-terminal), das lokal auf `127.0.0.1` mit niedriger Integritätsstufe läuft. ASK sieht nur Text; der ASK-Server erreicht den Laptop nie, und ASK braucht keine Freigabe für Tool-Server (ENTSCHEIDE E19).

Weitere Dokumente: [Machbarkeit](docs/MACHBARKEIT.md), [Entscheide](docs/ENTSCHEIDE.md), [Testprotokoll](docs/TESTPROTOKOLL.md).

## Aufbau

| Teil | Ort | Aufgabe |
|---|---|---|
| Hülle | `src/main/` | Electron-Hauptprozess: Fenster, Ordnerwahl und -sperre, Terminal-Verwaltung, Office-Umwandlung |
| Agent | `src/main/askapi.js`, `agent.js`, `tools.js` | Anmeldung und Modell-Anfragen an ASK, Arbeitsschleife, Werkzeuge (Befehl, Lesen, Schreiben, Bearbeiten, Suchen, Aufgaben), Chats in `.ask/chats` |
| Oberfläche | `src/renderer/` (React, Vite), `src/preload/` | Projekte und Chats, Chat mit Arbeitsschritten, Fragen und Rückfragen, Eingabe mit `/`-Befehlen, rechts Dateien, Vorschau und Browser, Dialoge, Anmeldeansicht. Datenmodell des Chats in `src/renderer/src/lib/chatModel.mjs` |
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

`npm start` baut die Oberfläche mit Vite (`npm run build:ui`, nach `src/renderer/dist`) und startet die App aus den Quellen. `npm run dist` baut den Installer `ASK-Setup-<Version>.exe` im Unterordner `dist` des Build-Ordners, `npm run dist:portable` die einzelne EXE `ASK-<Version>.exe` ohne Installation. Nur das Python neu bauen: `npm run build:python`. Icon und Startbild entstehen mit `scripts/make-icon.py`.

Oberfläche entwickeln: `npm run dev:ui` öffnet sie im Browser mit Beispieldaten (`src/renderer/src/dev/mockApi.js`, ohne Electron, mit einem vorgespielten Auftrag samt Frage und Rückfrage). In der App mit Neuladen bei jeder Änderung: `npm run dev:ui` und in einem zweiten Fenster `$env:ASK_DEV_UI = "http://localhost:5173"; npx electron .`.

Updates: Mit `"updateUrl": "https://…"` in `resources/app-config.json` erzeugt `npm run dist` zusätzlich `latest.yml`. Diese Datei und `ASK-Setup-<Version>.exe` (bzw. `ASK-<Version>.exe`) auf diesen Server legen; die installierte App lädt neue Versionen im Hintergrund und bietet «Neu starten» an, die einzelne EXE zeigt einen Hinweis mit Link (ENTSCHEIDE E34). Ohne `updateUrl` ist die Prüfung aus.

Tests: siehe [Testprotokoll](docs/TESTPROTOKOLL.md), Abschnitt «Tests wiederholen».

## Installation

`ASK-Setup-<Version>.exe` ausführen. Die Installation läuft pro Nutzer nach `%LOCALAPPDATA%\Programs\ask-desktop`, ohne Admin-Rechte und ohne weitere Fragen, und legt Verknüpfungen im Startmenü und auf dem Desktop an. Python, Bibliotheken und Anleitungen sind dabei; die Lernenden müssen nichts installieren. Deinstallieren über Einstellungen, Apps. Einstellungen bleiben in `%APPDATA%\ASK`, Laufzeitdaten in `%LOCALAPPDATA%\ASK-Desktop`.

Ohne Installation: `ASK-<Version>.exe` irgendwo ablegen und doppelklicken. Beim ersten Start entpackt die EXE die App einmal nach `%LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build>` (Startbild «ASK wird gestartet …»), danach startet sie die App direkt von dort. Eine neuere EXE entpackt sich beim ersten Start neu, die App löscht dann den älteren Stand. Es gibt keine Verknüpfungen und keinen Eintrag unter «Apps». Einstellungen und Laufzeitdaten liegen am gleichen Ort wie bei der installierten App. Installierte App und EXE laufen nicht gleichzeitig.

Installer und EXE sind nicht signiert. Windows SmartScreen kann deshalb beim ersten Start warnen («Weitere Informationen», «Trotzdem ausführen»).

## Bedienung

Beim ersten Start erscheint ein Hinweis in einfachem Deutsch (das Modell kann im Ordner ändern und löschen, darum nur mit Kopien arbeiten, Inhalte gehen an ASK). Danach legt man das erste Projekt an: Name eingeben, die App erstellt `%USERPROFILE%\ASK-Projekte\<Name>`; oder man öffnet einen vorhandenen Ordner als Projekt. Ist man nicht bei ASK angemeldet, zeigt die App die Anmeldeseite von ASK. Solange sie prüft, ob man angemeldet ist, steht dort «Verbinde mit ASK …» bzw. «Anmeldung wird geprüft …»; die Oberfläche von Open WebUI ist nie zu sehen.

**Aufbau.** Links die Projekte mit ihren Chats, in der Mitte der Chat, rechts Dateien, Vorschau und Browser (ENTSCHEIDE E35). Oben das TF-Zeichen, Reiter für die zuletzt geöffneten Chats und die Knöpfe zum Ein- und Ausblenden der beiden Seiten. Ein leerer Chat zeigt das Eingabefeld in der Mitte mit kurzen Vorschlägen; sobald der Chat beginnt, steht es unten. Hell und Dunkel folgen Windows oder der Wahl in den Einstellungen.

**Projekte.** Ein Projekt ist ein Arbeitsordner mit Namen und Farbe (ENTSCHEIDE E36). Ein Klick auf einen Chat öffnet ihn; liegt er in einem anderen Projekt, wechselt die App dorthin (das Terminal startet neu). «+» neben dem Projekt beginnt dort einen neuen Chat, «…» oder Rechtsklick öffnet das Menü des Projekts (Umbenennen, Farbe, im Explorer öffnen, Projektnotizen, GitHub, aus der Liste entfernen; der Ordner bleibt). Rechtsklick auf einen Chat löscht ihn nach Rückfrage. Während das Modell arbeitet, lässt sich das Projekt nicht wechseln. Ältere Einstellungen mit der Liste der zuletzt verwendeten Ordner werden beim ersten Start zu Projekten.

**Eingabe.** Enter sendet, Shift+Enter macht eine neue Zeile, Pfeil nach oben holt die letzte Eingabe, Esc stoppt das Modell, Ctrl+N beginnt einen neuen Chat. Im Eingabefeld: Büroklammer (Anhänge), «Web» (ASK sucht vor der ersten Antwort im Web, nur wenn ASK es für das Konto freigibt; die Quellen stehen unter der Antwort), der Denkaufwand und rechts die Modellwahl. Mit `/` öffnen sich Befehle: `/neu`, `/projekt`, `/goal`, `/suchen`, `/ordner`, `/explorer`, `/dateien`, `/github`, `/modell`, `/denken`, `/web`, `/chats`, `/einstellungen`, `/neustart` (Terminal), `/abmelden`. Unten links stehen Konto und Verbindung; fehlt etwas (nicht angemeldet, Terminal läuft nicht), steht es dort, ein Klick zeigt Details. Das Zahnrad öffnet die Einstellungen: Darstellung, Rückfrage vor heiklen Schritten, frühere Chats, ASK-Konto, GitHub, Version und Updates.

**Chat.** Die Eingabe steht als Sprechblase rechts. Die Arbeitsschritte eines Auftrags stehen in einer Karte («4 Arbeitsschritte»): pro Werkzeug eine Zeile mit Symbol, Aufruf und Kurzresultat («84 Zeilen gelesen», «+10 −4 Zeilen», erste Fehlerzeile in Rot); ein Klick zeigt ganzen Befehl, Ausgabe oder Diff. Mehrere Lese- und Suchschritte hintereinander stehen in einer Zeile («2 Dateien gelesen, 1 Ordner angesehen»). Während der Arbeit ist die Karte offen; ist der Auftrag fertig und hat er vier oder mehr Schritte, klappt sie zu, sichtbar bleiben Antwort, Hinweise, Quellen und Änderungen. Die Schlussantwort steht darunter als Markdown; Dateinamen des Projekts sind hervorgehoben (Klick öffnet die Vorschau). «Kopieren» und die Zeile «4 Schritte · 9’840 Token · 140 Token/s · Modell 70.4 s» erscheinen beim Darüberfahren; der Tooltip zeigt die Werte pro Schritt. Liefert ASK keine Tokenzahlen, schätzt die App sie und schreibt «≈» davor. Die Gedanken des Modells sind ausgeblendet; Ctrl+O zeigt Gedanken und alle Details. Die Chats eines Projekts liegen in `.ask/chats`.

**Fragen von ASK.** Fehlt dem Modell eine Entscheidung oder Angabe, fragt es mit dem Werkzeug `frage_stellen` (ENTSCHEIDE E37). Die Frage erscheint als Karte im Chat, mit Antworten zum Anklicken und einem Feld für eine eigene Antwort; auch eine Eingabe im normalen Eingabefeld gilt als Antwort. Das Modell wartet und arbeitet danach weiter. Stoppen beendet die Frage ohne Antwort.

**Rückfrage vor heiklen Schritten.** Vor dem Löschen (`del`, `rd`, `rmdir`, `Remove-Item`, `os.remove` usw.), vor dem Überschreiben einer vorhandenen Datei, die das Modell nicht im selben Auftrag erstellt hat, und vor Befehlen mit Pfaden ausserhalb des Projekts hält das Modell an (ENTSCHEIDE E33). Im Chat steht, was es tun will: «Erlauben», «Immer in diesem Chat» (für diese Art Schritt) oder «Ablehnen». Abgelehnt erfährt das Modell es und macht ohne den Schritt weiter. In den Einstellungen: nie fragen, heikle Schritte (Standard) oder immer (vor jedem Befehl und jeder Änderung). Ist die App im Hintergrund, blinkt sie in der Taskleiste. Was ein Python-Skript intern löscht, erkennt die App nicht; dafür bleibt «Rückgängig».

Im Eingabefeld öffnet «✻ Standard» den Regler für den Denkaufwand, wie «Reasoning Effort» in ASK: Standard (das Modell entscheidet), Tief, Mittel, Hoch, Maximal. Die App schickt `reasoning_effort` (low, medium, high), ab Hoch zusätzlich den Qwen-Schalter `/think` und `chat_template_kwargs`/`think`; lehnt ASK einen Parameter ab, lässt sie ihn weg.

Mit `/goal <Ziel>` setzt man ein Ziel für den Chat (Leiste «◎ Ziel» über der Eingabe). Das Modell arbeitet dann, bis es das Ziel mit dem Werkzeug `goal_done` als erreicht meldet; hört es vorher auf (etwa nach «Los geht's:»), stösst die App es an («↻ Weiter am Ziel»). Antwortet es zweimal hintereinander ohne Arbeitsschritt, wartet es offenbar auf die Person, und die App hält an. `/goal` zeigt das Ziel, `/goal aus` hebt es auf.

**Anhänge.** Bilder, Bildschirmfotos und Dokumente fügt man mit Ctrl+V ein, zieht sie ins Fenster oder wählt sie mit der Büroklammer im Eingabefeld (höchstens 10, je bis 50 MB). Vor dem Senden stehen sie als Kacheln über der Eingabe und lassen sich mit × entfernen. Die App legt sie versteckt unter `.ask/anhaenge/<Chat>/` ab (eingefügte Bildschirmfotos als `bild-<Datum>-<Zeit>.png`, gleiche Namen als «name (2).ext») und nennt dem Modell die Pfade; sie verschwinden mit dem Chat oder nach 14 Tagen. Was das Modell daraus macht (ein bearbeitetes Dokument, ein Bild im Bericht), speichert es im Arbeitsordner. Bilder schickt sie zusätzlich direkt mit, wie die ASK-Webseite, aus den letzten zwei Eingaben mit Bildern, grosse Bilder verkleinert auf 2048 Pixel. Meldet ASK, dass das Modell keine Bilder versteht, oder lehnt es sie ab, gehen nur die Pfade mit (ENTSCHEIDE E23).

**GitHub.** Mit `/github` (oder in den Einstellungen bzw. im Menü des Projekts «GitHub …») trägt man einmal ein persönliches Token mit dem Recht «repo» ein; die App speichert es verschlüsselt, das Modell sieht es nie. Danach kann man sagen «leg das Projekt auf GitHub an», «hol das Projekt lea/website von GitHub» oder «lad die Änderungen hoch». Das Modell holt mit `github_clone`/`github_pull`, lädt mit `github_push` hoch und legt mit `github_create` ein neues, privates Repository an. Hochladen und Anlegen bestätigt man in einem Dialog; beim Hochladen schaltet das Häkchen «Für dieses Repository nicht mehr fragen» die Rückfrage für dieses Repository ab (im `/github`-Dialog «wieder fragen»). `.ask` wird nie hochgeladen. Wurde eine Datei hier und auf GitHub geändert, überschreibt die App nichts und meldet einen Konflikt (ENTSCHEIDE E22).

Ein Auftrag läuft, bis das Modell fertig ist. Ein Schleifenschutz hält es an, wenn es dreimal hintereinander genau denselben Schritt macht oder acht Werkzeugaufrufe hintereinander scheitern (Notbremse nach 500 Schritten). Befehle, die Programme mit Fenster starten (Word, Excel, PowerPoint, Explorer, Editor, Browser, `start`), lehnt die App ab: sie sperren Dateien und warten auf Eingaben.

**Dateien und Vorschau.** Rechts zeigt der Reiter «Dateien» den Ordner des Projekts als Baum, live. Was der letzte Auftrag angelegt oder geändert hat, ist mit «neu» bzw. «geändert» markiert, gelöschte Dateien stehen durchgestrichen darunter. Ein Klick zeigt die Datei im Reiter «Vorschau» (Text, Bilder, PDF; Office-Dateien öffnen, im Explorer zeigen oder mit dem installierten Office als PDF anzeigen), ⤢ vergrössert sie. Doppelklick öffnet Dokumente, Bilder und Text im zugehörigen Programm; Programme und Skripte (`.bat`, `.exe`, `.lnk` …) zeigt die App nur im Explorer, damit sie nicht ausserhalb der Sperre starten.

**Internet.** Das Modell sucht selbst im Internet, so oft es nötig ist (ENTSCHEIDE E26): `web_search` über die Websuche von ASK (dieselbe Suchmaschine wie auf der Webseite; ist sie nicht freigegeben oder scheitert sie, über DuckDuckGo) mit bis zu 8 Treffern, `web_fetch` holt eine Seite als lesbaren Text ohne HTML (12’000 Zeichen pro Aufruf, PDF wird gespeichert und mit `read_file` gelesen). Adressen im lokalen Netz und auf dem Laptop sind gesperrt. Die gelesenen Seiten stehen als Quellen unter der Antwort. Der Schalter «Web» lässt ASK zusätzlich vor der ersten Antwort suchen.

**Frühere Chats.** Der Schalter in den Einstellungen (oder `/chats`) erlaubt dem Modell, frühere Chats im selben Ordner zu nutzen (ENTSCHEIDE E28): Es bekommt eine Liste der letzten 15 Chats (Datum, Titel) und die Werkzeuge `chat_search` (Suche über Eingaben, Antworten und Werkzeugaufrufe) und `chat_read` (einen Chat als Text lesen). Ausgeschaltet beginnt jeder Chat ohne Wissen über andere. Chats anderer Ordner sieht es nie.

**Aufgabenliste.** Wie in Claude Code steht die Aufgabenliste des Modells über dem Eingabefeld, nicht mehr in der Seitenleiste: «Aufgaben ▬▬ 3/7» mit Fortschrittsbalken, Erledigtes abgehakt und durchgestrichen, die aktuelle Aufgabe hervorgehoben, bei langen Listen nur der Ausschnitt um die aktuelle. Während eines Auftrags ist sie offen und die Zeile «✻ …» nennt die aktuelle Aufgabe; danach schrumpft sie auf eine Zeile. Ctrl+T (oder ein Klick) klappt sie auf und zu. Im Chat steht nur «Aufgaben ⎿ 3 von 7 erledigt», die Liste dazu in den Details. Eine fertige Liste verschwindet beim nächsten Auftrag.

**Browser.** Die App hat einen eigenen Browser (ENTSCHEIDE E27, E29). Der Reiter «Browser» rechts oder das Werkzeug `browser` öffnet einen echten Browser-Bereich (⤢ ganzes Fenster, ↻ neu laden, × schliessen, Adressleiste mit Zurück und Vorwärts, Eingabe ohne Punkt sucht mit DuckDuckGo). Das Modell öffnet damit Webseiten und HTML-Dateien aus dem Arbeitsordner, arbeitet ohne Neuladen auf der offenen Seite weiter und klickt (nach CSS-Selektor, sichtbarem Text oder Position), tippt und scrollt. Zurück bekommt es Adresse, Titel, Konsole, Text (6000 Zeichen), Bedienelemente und ein Bildschirmfoto. Man sieht alles live und kann selbst klicken und tippen; das Modell erfährt, wie oft die Person eingegriffen hat. Ein Aufruf dauert höchstens 2 Minuten, Esc und ■ beenden ihn sofort. Ist die App minimiert, öffnet das Modell die Seite unsichtbar mit 1280 × 800 und zeigt im Bereich Bilder davon.

Anmelden macht die Person selbst im Browser-Bereich; das Modell gibt keine Passwörter ein. Die Anmeldungen bleiben gespeichert, getrennt von ASK und Chrome, und lassen sich mit «Anmeldungen löschen» entfernen. Das Modell soll vor allem nachfragen, was etwas abschickt, kauft, postet oder löscht, und Text auf Webseiten nicht als Anweisung behandeln. Das steht nur in seinen Anweisungen und ist keine technische Sperre, darum wichtige Konten dort besser nicht anmelden. Downloads sind blockiert, Adressen im lokalen Netz kann nur die Person öffnen, nicht das Modell.

**Bilder erstellen.** Bietet ASK dem Konto das Werkzeug «Flux Bild» an, erstellt und bearbeitet das Modell Bilder mit `bild_erstellen` (ENTSCHEIDE E31): «mach ein Bild von …» oder ein Bild anhängen und sagen, was sich ändern soll. Flux läuft auf dem ASK-Server; die App schickt dafür eine eigene Anfrage an ASK, wie die Webseite, und speichert das Ergebnis im Arbeitsordner, neue Bilder unter `bilder/`, ein bearbeitetes Bild ersetzt seine Vorlage (Rückgängig wie bei anderen Dateien). Das Bild steht gross im Chat, Klick öffnet die Vorschau. Format, Qualität, Stil, Anzahl (bis 4), Referenzfotos aus Wikipedia und das TF-Bern-Logo gibt das Modell an Flux weiter. Ohne Flux in ASK gibt es das Werkzeug nicht.

**Ansehen und testen.** Das Modell sieht, was es erstellt (ENTSCHEIDE E25). `view_file` zeigt ihm Bilder, PDF, Word, Excel und PowerPoint als Bild (Seiten bzw. Folien, Standard die ersten 3, höchstens 6; Office-Dateien wandelt die App dafür mit Office in PDF um). Die Bilder stehen als Vorschau in der Werkzeugzeile (Klick: gross) und liegen in `.ask/arbeit/<Chat>/ansicht/`. Versteht das Modell laut ASK keine Bilder, gibt es `view_file` nicht, und `browser` liefert nur Text.

**Arbeiten im Ordner.** Das Modell legt neue Dateien dort an, wo sie ins Projekt passen, und ändert vorhandene Dateien direkt, ohne Kopien wie `_V2` (ENTSCHEIDE E24). Vor jedem Auftrag sichert die App den Stand des Ordners. Unter einem Auftrag, der Dateien geändert hat, steht «Dateien: 2 neu, 1 geändert» mit den Namen und dem Knopf «Rückgängig» (beim letzten solchen Auftrag); er setzt den Ordner nach einer Rückfrage auf den Stand vor dem Auftrag zurück, löscht neu angelegte Dateien und sagt es dem Modell. Gesichert wird jede Dateifassung nur einmal; Dateien über 100 MB und `.git`, `node_modules` werden nicht gesichert.

**Projektnotizen.** `.ask/projekt.md` ist wie `CLAUDE.md` in Claude Code: Die App gibt die Datei jedem Chat im Ordner mit, und das Modell hält darin kurz fest, was spätere Chats wissen müssen (Aufbau, wichtige Dateien, Vorlagen, Wünsche). Im Menü des Projekts öffnet «Projektnotizen bearbeiten» sie im Editor.

Im Arbeitsordner legt die App nur `.ask/` an. Dort liegen die Anleitungen (`skills/`), die Aufgabenliste, die Chats (`chats/`), die Projektnotizen, die Hilfsskripte des Modells (`skripte/<Chat>/`), seine Zwischendateien (`arbeit/<Chat>/`) und die Anhänge (`anhaenge/<Chat>/`), damit der Arbeitsordner aufgeräumt bleibt; diese drei verschwinden mit dem Chat und werden nach 14 Tagen automatisch gelöscht. Die Sicherung für «Rückgängig» liegt in `sicherung/` und wird ebenfalls nach 14 Tagen gelöscht. `.ask` ist versteckt (Windows-Attribut wie bei `.git`) und erscheint im Explorer nur mit «Ausgeblendete Elemente», umgewandelte PDFs (`pdf/`) und Aufträge an die App.

Scrollt man im Chat nach oben, bleibt die Ansicht dort, auch während das Modell arbeitet; «↓ nach unten» oder Scrollen bis ans Ende lässt sie dem Text wieder folgen. Projekte, deren Ordner es nicht mehr gibt, sind durchgestrichen mit «Ordner nicht gefunden»; im Menü des Projekts lassen sie sich aus der Liste entfernen.

Die Ränder links und rechts vom Chat lassen sich mit der Maus ziehen; ein Doppelklick setzt die Standardbreite zurück, mit Tab angewählt verschieben ihn die Pfeiltasten. Der Chat bleibt mindestens 380 Pixel breit. Die Breiten merkt sich die App auf diesem Rechner.

Tastenkürzel: Ctrl+Plus/Minus/0 zoomt die Oberfläche, Ctrl+R lädt die Anmeldeseite von ASK neu.

## Bekannte Grenzen

- **Lesen ausserhalb des Ordners.** Windows verhindert Schreiben und Löschen ausserhalb des Arbeitsordners (Low Integrity Level). Lesen kann das Modell über die Shell aber alle Dateien, die der Nutzer lesen darf, und es hat Internetzugang. Die Datei-API von open-terminal ist zusätzlich auf den Ordner gesperrt. Siehe MACHBARKEIT, Abschnitt 5.
- **Originale im Ordner.** Das Modell ändert Dateien direkt; Windows schützt sie nicht (ENTSCHEIDE E14). Schutz bieten die Rückfrage vor Löschen und Überschreiben (E33, erkennt nur, was im Befehl selbst steht) und die Sicherung mit «Rückgängig» (E24). Sie gilt 14 Tage, nur für den letzten Auftrag mit Änderungen und nicht für Dateien über 100 MB, `.git` und Ordner mit mehr als 5000 Dateien. Fürs Rückgängigmachen müssen betroffene Dateien in Word, Excel oder PowerPoint geschlossen sein.
- **Werkzeuge unter Low.** Windows PowerShell 5.1 und die Unix-Werkzeuge von Git for Windows starten im Terminal nicht. Das Modell arbeitet mit `cmd.exe` und Python. Office lässt sich nur über die App umwandeln, nicht direkt steuern.
- **Ordner auf FAT/exFAT-USB-Sticks** oder Netzlaufwerken werden abgelehnt, weil sie sich nicht schützen lassen.
- **Abhängigkeit von ASK.** Die App braucht nur, was auch die ASK-Webseite braucht: Anmeldung und Zugriff auf ein Modell. Ob ASK die Werkzeugliste (`tools`) an das Modell weitergibt, ist mit dem echten ASK noch nicht geprüft. Lehnt ASK sie ab oder schreibt das Modell Aufrufe als `<tool_call>` in den Text, wechselt die App von selbst in einen Textmodus und beschreibt die Werkzeuge im Systemprompt.
- **Websuche einmal pro Auftrag.** ASK sucht vor der ersten Antwort; das Modell kann während der Arbeit nicht selbst nochmals suchen. Ob die Websuche über die Schnittstelle bei eurem ASK genauso läuft wie im Browser, ist noch nicht geprüft.
- **Werkzeuge und Wissen aus ASK** (die Werkzeuge und Wissenssammlungen, die einem Modell in ASK zugeordnet sind) stehen in der App nicht zur Verfügung, ausser Flux für Bilder.
- **Flux nicht mit dem echten ASK geprüft.** Beim Bau war Flux nur im Beta-Arbeitsbereich verfügbar. Wie ASK das Bild über die Schnittstelle zurückgibt, ist deshalb nicht geprüft; die App sucht an allen üblichen Stellen (E31). Kommt kein Bild, sagt die App, was ASK stattdessen geantwortet hat, und legt die Antwort unter `.ask/arbeit/<Chat>/flux-….json` ab. Während der Anfrage steht kurz ein Chat «ASK Desktop: Bild» im ASK-Verlauf.
- **GitHub ohne Git.** Der Abgleich läuft über die Schnittstelle von GitHub, nicht mit Git: kein Zusammenführen innerhalb einer Datei, keine Branches ausser dem Standard-Branch, keine Git-Historie im Ordner. Ist ein Ordner schon ein Git-Repository, bleibt `.git` unberührt. Mit dem echten GitHub ist der Abgleich noch nicht getestet, nur mit einem Nachbau der Schnittstelle.
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
- Update-Server einrichten (`updateUrl` in `resources/app-config.json`, ENTSCHEIDE E34) und die Installer dort ablegen; am besten mit Code-Signatur.
- Die Bildmarke der Technischen Fachschule Bern steht ohne Wortmarke im Kopf der App. Das CD Manual (1.1.3) verbietet das; mit der Schule klären oder durch das ganze Logo ersetzen.
- Vorschau für Office-Dateien direkt beim Anklicken, sobald klar ist, dass Office auf allen Laptops vorhanden ist.
