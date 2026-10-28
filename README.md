# ASK Desktop

ASK Desktop ist eine Windows-App für die Lernenden der TF Bern. Sie zeigt ASK (die Open-WebUI-Instanz der Schule) in einem eigenen Fenster und verbindet ASK mit einem Ordner auf dem Laptop. Das Sprachmodell kann in diesem Ordner selbständig arbeiten: Dateien lesen und schreiben, Python ausführen, eine Aufgabenliste führen, Ergebnisse prüfen und fertige Dateien in `ausgabe/` ablegen.

Technisch startet die App [open-terminal](https://github.com/open-webui/open-terminal) lokal auf `127.0.0.1` und trägt es in ASK als persönliche Terminal-Verbindung ein. Die Anfragen gehen direkt vom eingebetteten Browser an das lokale Terminal; der ASK-Server erreicht den Laptop nie.

Weitere Dokumente: [Machbarkeit](docs/MACHBARKEIT.md), [Entscheide](docs/ENTSCHEIDE.md), [Testprotokoll](docs/TESTPROTOKOLL.md).

## Aufbau

| Teil | Ort | Aufgabe |
|---|---|---|
| Hülle | `src/main/` | Electron-Hauptprozess: Fenster, Ordnerwahl und -sperre, Terminal-Verwaltung, Verbindung mit ASK, Office-Umwandlung |
| Oberfläche | `src/renderer/`, `src/preload/` | Statusleiste, Seitenleiste, Dialoge |
| Terminal-Starter | `python/ask_terminal/` | startet open-terminal mit niedriger Integritätsstufe in einem Job-Objekt, Ordnersperre in der Datei-API, CORS, WebSocket-Anmeldung |
| Anweisungen | `resources/anweisungen/` | Systemprompt für das Modell und Zusatz zur Befehlsbeschreibung |
| Skills | `resources/skills/` | Anleitungen für Word, Excel, PowerPoint, PDF, Prüfskript `pruefen.py`, Platz für Vorlagen in `vorlagen/` |
| Tests | `tests/` | Unit-, Python-, End-to-End- und Pakettests, Beispieldateien |

Die Isolationsvariante steckt in `src/main/backend/native-low.js`. Eine andere Variante (AppContainer, Sandbox, WSL) muss nur `start`, `stop` und `running` gleich anbieten.

## Build

Voraussetzungen auf dem Build-Rechner: Windows 11, Node.js 22 oder neuer, Git, Internetzugang. Ein installiertes Python braucht es nicht.

Wichtig, weil das Repo in OneDrive liegt: `npm install` nie direkt im Projektordner ausführen. npm würde `node_modules` (rund 450 MB) in den OneDrive-Ordner legen. Das Einrichtungsskript installiert alles in einen lokalen Build-Ordner und verlinkt nur `node_modules`.

```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup.ps1
```

Das Skript legt den Build-Ordner an (`%LOCALAPPDATA%\ASK-Desktop-Build`, änderbar mit `-BuildDir` oder `ASK_BUILD_DIR`), installiert die npm-Pakete mit `npm ci`, lädt Python 3.14.8 als offizielles NuGet-Paket (Prüfsumme wird kontrolliert), installiert die Pakete aus `python/requirements.txt` und entfernt Testsuiten. Den Build-Ordner merkt es sich in `.ask-build-dir` (nicht versioniert), damit die folgenden Befehle ihn ohne Umgebungsvariable finden. Dauer beim ersten Mal etwa 5 Minuten.

Vor dem Bauen die Adresse von ASK in `resources/app-config.json` eintragen, sonst fragt die App beim ersten Start danach:

```json
{ "askUrl": "https://<ASK-URL>" }
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

Beim ersten Start erscheint ein Hinweis in einfachem Deutsch (das Modell kann im Ordner ändern und löschen, darum nur mit Kopien arbeiten, Inhalte gehen an ASK). Danach die ASK-Adresse, falls sie nicht eingebaut ist, und die Wahl des Arbeitsordners: «Neuen Projektordner anlegen» erstellt `%USERPROFILE%\ASK-Projekte\Projekt <Datum>`, oder ein vorhandener Ordner wird gewählt.

Die Statusleiste zeigt den Ordner (Klick öffnet die letzten Ordner und weitere Aktionen), den Zustand des Terminals und der Verbindung mit ASK. Ein Klick auf einen Zustand öffnet Details: beim Terminal die Meldungen, bei ASK eine Erklärung und URL und Key zum Einrichten von Hand. Daneben «Terminal neu starten», «Ordner wechseln» und der Knopf für die Seitenleiste.

Nach der Anmeldung in ASK richtet die App die Terminal-Verbindung selbst ein und wählt sie im Chat vor (Terminal-Symbol im Eingabefeld). Das klappt nur, wenn ASK persönliche Terminal-Verbindungen erlaubt; sonst meldet die Statusleiste «Terminal nicht freigegeben».

Die Seitenleiste zeigt die Aufgabenliste aus `.ask/tasks.json`, die fertigen Dateien in `ausgabe/` (neueste zuerst) und den Inhalt des Arbeitsordners, alles live. Ein Klick zeigt eine Vorschau für Text, Bilder und PDF; «Gross» vergrössert sie. Office-Dateien lassen sich öffnen, im Explorer zeigen oder mit dem installierten Office als PDF anzeigen. Doppelklick öffnet eine Datei im zugehörigen Programm.

Im Arbeitsordner legt die App `ausgabe/` und `.ask/` an. In `.ask/` liegen die Anleitungen (`skills/`), die Aufgabenliste, umgewandelte PDFs (`pdf/`) und Aufträge an die App.

Tastenkürzel: Ctrl+R lädt ASK neu, Ctrl+Plus/Minus/0 zoomt ASK.

## Bekannte Grenzen

- **Lesen ausserhalb des Ordners.** Windows verhindert Schreiben und Löschen ausserhalb des Arbeitsordners (Low Integrity Level). Lesen kann das Modell über die Shell aber alle Dateien, die der Nutzer lesen darf, und es hat Internetzugang. Die Datei-API von open-terminal ist zusätzlich auf den Ordner gesperrt. Siehe MACHBARKEIT, Abschnitt 5.
- **Originale im Ordner** sind nur durch die Anweisung geschützt, nicht durch Windows (ENTSCHEIDE E14). Darum der Hinweis, mit Kopien zu arbeiten.
- **Werkzeuge unter Low.** Windows PowerShell 5.1 und die Unix-Werkzeuge von Git for Windows starten im Terminal nicht. Das Modell arbeitet mit `cmd.exe` und Python. Office lässt sich nur über die App umwandeln, nicht direkt steuern.
- **Ordner auf FAT/exFAT-USB-Sticks** oder Netzlaufwerken werden abgelehnt, weil sie sich nicht schützen lassen.
- **Abhängigkeit von ASK.** Persönliche Terminals brauchen in Open WebUI die Freigaben «Direkte Integrationen» und «Direkte Tool-Server». Den Systemprompt des Terminals übernimmt Open WebUI ab 0.8.11, eine `AGENTS.md` im Ordner für persönliche Terminals unter Windows ab 0.12.0.
- **Office-Umwandlung** braucht ein installiertes Microsoft Office. Ein Office-Fenster, das die Person gerade offen hat, wird nicht geschlossen; eine hängende unsichtbare Office-Instanz wird nach 2 Minuten beendet.
- **Grösse.** Installer rund 190 MB, einzelne EXE rund 240 MB, installiert bzw. entpackt rund 650 MB (davon Python mit pandas und matplotlib rund 290 MB).
- **Einzelne EXE beim ersten Start.** Der erste Start entpackt rund 610 MB, im Test 36 s bis zum laufenden Terminal, spätere Starts 3 s. Der entpackte Stand bleibt liegen wie eine Installation; entfernen lässt er sich nur durch Löschen von `%LOCALAPPDATA%\ASK-Desktop\app`.
- **Interaktive Notebooks** von open-terminal sind abgeschaltet (umgehen die Ordnersperre).

## Offene Punkte

Grundsatzfragen, die nur die Schule bzw. du entscheiden können:

1. **Freigabe in ASK.** Die ASK-Administration muss persönliche Terminal-Verbindungen erlauben (Einstellung «Direkte Integrationen» und Berechtigung «Direkte Tool-Server» für die Lernenden). Ohne diese Freigabe ist die App nur ein ASK-Fenster. Die App umgeht eine fehlende Freigabe bewusst nicht.
2. **ASK-Adresse** in `resources/app-config.json` eintragen.
3. **Code-Signatur und Geräteverwaltung.** Mit der Informatik klären, ob per Nutzer installierte Programme unter `AppData` auf den Schullaptops laufen dürfen (AppLocker, Intune) und ob ein Zertifikat für die Signatur verfügbar ist.
4. **Datenschutz.** Inhalte aus dem Arbeitsordner gehen an ASK und das Modell. Ob dafür Regeln für die Lernenden nötig sind, ist eine Schulfrage.
5. **TF-Bern-Vorlagen** für Word, PowerPoint und Excel als `.docx`, `.pptx`, `.xlsx` in `resources/skills/vorlagen/` ablegen.
6. **Ablage des Repos.** Das Repo liegt in OneDrive. Mit `scripts/setup.ps1` bleibt OneDrive frei von Build-Dateien, einfacher wäre ein Repo ausserhalb von OneDrive mit einem Git-Server als Sicherung.

Technische Weiterentwicklungen:

- Anweisungen und Skills nach den ersten Tests mit Qwen nachschärfen (`resources/anweisungen/systemprompt.md` ist bewusst kurz).
- Stärkere Isolation mit AppContainer, damit auch Lesen auf den Ordner beschränkt ist.
- Automatische Updates (electron-updater) und eine Versionsanzeige in der App.
- Vorschau für Office-Dateien direkt beim Anklicken, sobald klar ist, dass Office auf allen Laptops vorhanden ist.
