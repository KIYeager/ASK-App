# ASK Desktop

ASK Desktop ist eine Windows-App für die Lernenden der TF Bern. Sie ist eine eigene Chat-Oberfläche für ASK (die Open-WebUI-Instanz der Schule), im Stil eines Terminal-Agenten: Man beschreibt einen Auftrag, und das Sprachmodell arbeitet selbständig in einem Ordner auf dem Laptop. Es liest und schreibt Dateien, führt Python aus, führt eine Aufgabenliste, prüft die Ergebnisse und arbeitet dabei direkt in den Dateien des Ordners, wie Claude Code in einem Projekt. Jeder Auftrag lässt sich rückgängig machen. Jeder Schritt erscheint im Chat als Zeile wie `● Befehl(python auswertung.py)` mit der Ausgabe darunter.

Technisch meldet man sich einmal auf der ASK-Seite an (auch über das Microsoft-Konto); danach liest die App das Login-Token und spricht das Modell direkt über die Schnittstelle von ASK an (`/api/chat/completions`, gestreamt, mit Werkzeugliste). Die Werkzeuge führt die App selbst aus, über [open-terminal](https://github.com/open-webui/open-terminal), das lokal auf `127.0.0.1` mit niedriger Integritätsstufe läuft. ASK sieht nur Text; der ASK-Server erreicht den Laptop nie, und ASK braucht keine Freigabe für Tool-Server (ENTSCHEIDE E19).

Neue Version veröffentlichen: siehe [unten](#neue-version-veröffentlichen). Weitere Dokumente: [Machbarkeit](docs/MACHBARKEIT.md), [Entscheide](docs/ENTSCHEIDE.md), [Testprotokoll](docs/TESTPROTOKOLL.md).

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

`npm start` startet die App aus den Quellen. `npm run dist` baut den Installer `ASK-Setup-<Version>.exe` im Unterordner `dist` des Build-Ordners, `npm run dist:portable` die einzelne EXE `ASK-<Version>.exe` ohne Installation. Nur das Python neu bauen: `npm run build:python`; `npm run dist` packt es danach mit `scripts/pack-python.js` in `python-pack\python.zip` (ENTSCHEIDE E39). Icon und Startbild entstehen mit `scripts/make-icon.py`.

Tests: siehe [Testprotokoll](docs/TESTPROTOKOLL.md), Abschnitt «Tests wiederholen». Bei jedem Pull Request laufen auf GitHub (`.github/workflows/ci.yml`) ESLint und die Unit-Tests auf Linux, Ruff und die Python-Tests ohne Windows, und auf Windows die Tests des Terminals (in einem Ordner mit Umlaut) und der Skills. Lokal: `npm run lint`, `npm test`.

Lokal bauen braucht es nur noch zum Ausprobieren. Veröffentlichte Versionen baut GitHub (nächster Abschnitt).

## Neue Version veröffentlichen

Neue Versionen baut GitHub, nicht der eigene Rechner. Normalerweise entsteht eine neue Version beim Merge eines Pull Requests; dazu wählt man mit einem Label, ob es eine Beta, eine Version für alle oder keine neue Version wird. GitHub baut auf einem Windows-Rechner Installer und EXE und stellt sie in das öffentliche Repository [KIYeager/ASK-Releases](https://github.com/KIYeager/ASK-Releases). Alle installierten Apps bieten das Update danach von selbst an (ENTSCHEIDE E37, E38). Der Code in ASK-App bleibt privat; ASK-Releases enthält nur die fertigen Dateien, und die App holt sie ohne Token.

### Einmal einrichten

Nur vor dem allerersten Release nötig.

1. **Release-Repository anlegen.** Auf GitHub «New repository», Owner `KIYeager`, Name `ASK-Releases`, **Public**, Häkchen bei «Add a README file» (ein leeres Repository nimmt keine Releases an). Danach den Inhalt von [docs/ASK-Releases-README.md](docs/ASK-Releases-README.md) als README einfügen: Er erklärt den Lernenden, was sie herunterladen sollen.
2. **Token anlegen.** GitHub, Profilbild, Settings, Developer settings, Personal access tokens, Fine-grained tokens, «Generate new token»:
   - Resource owner: `KIYeager`
   - Repository access: «Only select repositories», nur `ASK-Releases`
   - Permissions, Repository permissions: «Contents: Read and write»
   - Expiration: nach Wunsch. Läuft das Token ab, scheitert das nächste Release beim Veröffentlichen, dann ein neues anlegen und Schritt 3 wiederholen.
3. **Token in ASK-App eintragen.** Im Repository ASK-App: Settings, Secrets and variables, Actions, «New repository secret». Name `RELEASES_TOKEN`, Wert das Token aus Schritt 2.
4. **Workflows dürfen schreiben.** Im Repository ASK-App: Settings, Actions, General, ganz unten «Workflow permissions»: «Read and write permissions» wählen und speichern. Beim Merge macht der Workflow den Commit «Version …» und den Tag auf `main`. Ist `main` geschützt (Settings, Branches oder Rules), dort «GitHub Actions» als Ausnahme erlauben, sonst scheitert der Schritt «Version setzen».
5. **Optional: Merge erst mit gewählter Release-Art.** In einer Regel für `main` (Settings, Rules oder Branches) die Prüfung «Release-Art gewählt» als erforderlich eintragen. Ohne das ist sie nur rot, ein Merge geht trotzdem.

### Vom Pull Request zur neuen Version

1. **PR öffnen** nach `main`, wie gewohnt. Die Vorlage erinnert an die Wahl.
2. **Release-Art wählen:** rechts beim PR unter «Labels» genau eines setzen:

   | Label | Beim Merge |
   |---|---|
   | `beta` | Beta-Version, nur für «Beta-Versionen erhalten» |
   | `release` | neue Version für alle |
   | `kein-release` | keine neue Version (z. B. nur Doku oder Tests) |

   Die Prüfung «Release-Art gewählt» bleibt rot, bis genau eines gesetzt ist. Ist sie grün, steht in ihren Details, welche Version beim Merge entsteht («Beim Merge wird daraus ASK 0.2.1-beta.1»).
3. **Versionsnummer:** Ohne Angabe zählt sie automatisch hoch:

   | Aktuell | `beta` | `release` |
   |---|---|---|
   | `0.2.0` | `0.2.1-beta.1` | `0.2.1` |
   | `0.2.1-beta.1` | `0.2.1-beta.2` | `0.2.1` |

   Eigene Nummer, zum Beispiel für neue Funktionen: im PR-Text eine eigene Zeile `Version: 0.3.0`. Mit `release` wird daraus `0.3.0`, mit `beta` `0.3.0-beta.1` (bzw. die nächste Beta von `0.3.0`). Eine volle Beta-Nummer wie `Version: 0.3.0-beta.4` gilt so, wie sie ist. Die Nummer muss grösser sein als die aktuelle, sonst ist die Prüfung rot.
4. **Text im Update-Dialog:** der Titel des PRs. Eigener Text für die Lernenden: im PR-Text eine Zeile `Release-Text: Die Seitenleiste lässt sich jetzt breiter ziehen`.
5. **Mergen.** Der Workflow «Release» setzt die Version in `package.json`, macht den Commit «Version 0.2.1» und den Tag `v0.2.1` auf `main` und baut dann rund 15 bis 20 Minuten (Reiter «Actions»): npm-Pakete, Lint und Tests, Python, Installer, einzelne EXE, Veröffentlichen.
6. **Prüfen.** In [ASK-Releases, Releases](https://github.com/KIYeager/ASK-Releases/releases) steht «ASK 0.2.1» mit sechs Dateien: `ASK-Setup-0.2.1.exe`, `ASK-Setup-0.2.1.exe.blockmap`, `latest.yml`, `ASK-0.2.1.exe` und mit festem Namen `ASK-Setup.exe` und `ASK.exe`; eine Vollversion mit «Latest», eine Beta mit «Pre-release». Installierte Apps fragen spätestens 6 Stunden später oder beim nächsten Start; sofort geht es mit «Nach Updates suchen» in den Einstellungen.

Erstellt ein KI-Agent den PR (Claude Code, Codex, Copilot), fragt er nach der Release-Art und setzt das Label selbst; die Regel dafür steht in [AGENTS.md](AGENTS.md).

Nach dem Merge liegt auf `main` ein Commit «Version …» mehr. Vor der nächsten Arbeit darum `git pull`.

Jedes Release braucht rund 30 bis 40 der 2000 Gratis-Minuten von GitHub Actions pro Monat (Windows-Rechner zählen bei privaten Repositories doppelt).

### Ohne Pull Request, von Hand

Für den Fall, dass man direkt auf `main` arbeitet oder eine Version neu bauen will. Im Projektordner, auf `main`, alles committet:

```powershell
npm run release -- 0.3.0 "Was ist neu"
```

Für eine Beta `0.3.0-beta.1`. Das Skript (`scripts/release.js`) prüft, dass alles committet und die Version grösser ist, setzt sie in `package.json` und `package-lock.json`, macht Commit und Tag mit dem Text und lädt beides hoch; dann baut der Workflow wie oben. Dafür braucht es nur Node und Git, kein `npm install`.

### Beta-Versionen

Eine Beta erscheint in ASK-Releases als «Pre-release». Angeboten bekommt sie nur, wer «Beta-Versionen erhalten» eingeschaltet hat (Einstellungen, Ordner-Menü oder `/beta`). Alle anderen sehen sie nicht, und der Link für die Lernenden zeigt weiter auf die letzte Vollversion. Weitere PRs mit `beta` zählen hoch (`0.2.1-beta.2`); der nächste PR mit `release` macht daraus `0.2.1` für alle, auch für die Beta-Tester.

### Wenn etwas schiefgeht

Im Reiter «Actions» den roten Lauf öffnen; der Schritt mit dem roten Kreuz nennt den Grund.

| Meldung oder Lage | Abhilfe |
|---|---|
| «Secret RELEASES_TOKEN fehlt» | Einmal einrichten, Schritt 3. Danach im Lauf oben rechts «Re-run jobs». |
| Fehler beim Veröffentlichen mit `401` oder `Bad credentials` | Token abgelaufen oder ohne Recht auf ASK-Releases: neues Token (Schritt 2), Secret ersetzen (Schritt 3), «Re-run jobs». |
| «Tag … passt nicht zu package.json» | Der Tag wurde von Hand gesetzt. Immer per PR-Label oder `npm run release`. |
| Prüfung «Release-Art gewählt» rot | Genau eines der Labels `beta`, `release`, `kein-release` setzen; bei einer Zeile `Version:` im PR die Meldung in den Details lesen. |
| Schritt «Version setzen» scheitert mit `403` oder «protected branch» | Einmal einrichten, Schritt 4. Danach den Lauf mit «Re-run jobs» wiederholen. |
| PR gemergt, aber kein Lauf «Release» | Kein Label `beta` oder `release` gesetzt. Nachholen von Hand: `npm run release` auf `main`. |
| Fehler beim Herunterladen (npm, Python, NuGet) | Meist vorübergehend: einmal «Re-run jobs». |
| Tests oder Build scheitern wegen des Codes | Fehler beheben und als neuer PR mit Label mergen; daraus entsteht die nächste Nummer. Die gescheiterte Version wird nie veröffentlicht; ihr Tag kann bleiben. |
| Release in ASK-Releases ist halb da (Dateien fehlen) | Das Release in ASK-Releases löschen («Delete»), dann «Re-run jobs». |
| Eine veröffentlichte Version hat einen Fehler | Nicht löschen, sondern schnell eine korrigierte Version nachschieben (`0.2.1`). Apps gehen nie auf eine ältere Version zurück. |

`npm run release` meldet selbst, wenn noch Änderungen nicht committet sind, die Version nicht grösser ist oder es den Tag schon gibt; dann wurde nichts verändert.

### Ort der Releases ändern

`updates` in `resources/app-config.json` anpassen (gilt für App und Build) und im Workflow `.github/workflows/release.yml` das `--repo` ändern. Bereits installierte Apps suchen am alten Ort weiter, bis sie eine Version mit der neuen Angabe haben; die letzte Version am alten Ort muss also schon die neue Angabe enthalten.

## Installation

Für die Lernenden gibt es einen festen Link, der immer auf die neueste Version zeigt:

https://github.com/KIYeager/ASK-Releases/releases/latest/download/ASK-Setup.exe

`ASK-Setup.exe` ausführen. Die Installation läuft pro Nutzer nach `%LOCALAPPDATA%\Programs\ask-desktop`, ohne Admin-Rechte und ohne weitere Fragen, und legt Verknüpfungen im Startmenü und auf dem Desktop an. Python, Bibliotheken und Anleitungen sind dabei; die Lernenden müssen nichts installieren. Python liegt in der App als ein einziges Archiv; beim ersten Start entpackt ASK es nach `%LOCALAPPDATA%\ASK-Desktop\python\<Fassung>` (unten in der Statuszeile «Python wird eingerichtet (einmalig) … 40 %», je nach Laptop und Virenscanner einige Minuten). Danach startet ASK direkt, und ein Update entpackt Python nur neu, wenn sich Python selbst geändert hat (ENTSCHEIDE E39). Fehlt die Verknüpfung im Startmenü, legt ASK sie beim Start wieder an. Deinstallieren über Einstellungen, Apps; das entpackte Python wird dabei entfernt. Einstellungen bleiben in `%APPDATA%\ASK`, Laufzeitdaten in `%LOCALAPPDATA%\ASK-Desktop`.

Ohne Installation: `ASK-<Version>.exe` irgendwo ablegen und doppelklicken. Beim ersten Start entpackt die EXE die App einmal nach `%LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build>` (Startbild «ASK wird gestartet …»), danach startet sie die App direkt von dort. Eine neuere EXE entpackt sich beim ersten Start neu, die App löscht dann den älteren Stand. Es gibt keine Verknüpfungen und keinen Eintrag unter «Apps». Einstellungen und Laufzeitdaten liegen am gleichen Ort wie bei der installierten App. Installierte App und EXE laufen nicht gleichzeitig.

**Updates.** Die installierte App sucht 20 Sekunden nach dem Start und danach alle 6 Stunden in ASK-Releases nach einer neueren Version. Gibt es eine, fragt sie einmal «ASK 0.2.0 ist da. Jetzt aktualisieren?» und zeigt den Text des Releases. Nach «Jetzt aktualisieren» lädt sie das Update im Hintergrund; ein Balken oben in der App und die Taskleiste zeigen den Fortschritt, man kann weiterarbeiten. Ist es geladen, zählt der Balken 10 Sekunden herunter, dann schliesst sich ASK, ein kleines Fenster zeigt das Installieren (ohne Admin-Rechte), und ASK startet von selbst wieder; «Jetzt neu starten» wartet nicht, «Später» installiert erst beim Schliessen von ASK. Während das Modell arbeitet, fragt und startet sie nie, sondern erst danach (ENTSCHEIDE E40). Chats, Ordner und Einstellungen bleiben. Bei «Später» in der Frage fragt sie bis zum nächsten Start nicht mehr von selbst, unten steht aber «Update 0.2.0» zum Anklicken. Von Hand: «Nach Updates suchen» in den Einstellungen oder im Ordner-Menü, `/update` oder der Eintrag unten. Ohne Internet bleibt die Suche still.

Die einzelne EXE kann sich nicht selbst ersetzen. Sie sucht genauso, meldet eine neue Version aber nur mit «Download öffnen» (Seite des Releases); die neue `ASK.exe` entpackt sich beim ersten Start und räumt den alten Stand weg.

Installer und EXE sind nicht signiert. Windows SmartScreen kann deshalb beim ersten Start warnen («Weitere Informationen», «Trotzdem ausführen»).

## Bedienung

Beim ersten Start erscheint ein Hinweis in einfachem Deutsch (das Modell kann im Ordner ändern und löschen, darum nur mit Kopien arbeiten, Inhalte gehen an ASK). Danach die Wahl des Arbeitsordners (die ASK-Adresse ist eingebaut): «Neuen Projektordner anlegen» erstellt `%USERPROFILE%\ASK-Projekte\Projekt <Datum>`, oder ein vorhandener Ordner wird gewählt. Ist man nicht bei ASK angemeldet, zeigt die App die Anmeldeseite von ASK. Solange die App prüft, ob man angemeldet ist (beim Start und direkt nach dem Login), zeigt sie stattdessen «Verbinde mit ASK …» bzw. «Anmeldung wird geprüft …»; die Oberfläche von Open WebUI ist dabei nie zu sehen.

Links liegen die Chats, gruppiert nach Arbeitsordner (der aktuelle zuerst, darunter die zuletzt verwendeten), mit «+ Neuer Chat» und einer Suche über die Titel. Ein Klick öffnet einen Chat; liegt er in einem anderen Ordner, wechselt die App dorthin (das Terminal startet neu). Das «+» neben einem Ordnernamen beginnt dort einen neuen Chat. «…» an einem Chat (oder ein Rechtsklick) öffnet ein Menü: Umbenennen, Anheften (angeheftete Chats stehen zuoberst) und Löschen nach Rückfrage (die Dateien im Ordner bleiben). Die Suche filtert die Titel sofort und zeigt ab zwei Zeichen unter «Im Inhalt gefunden» auch Chats, in denen der Begriff in Eingaben, Antworten oder Arbeitsschritten vorkommt. ☰ blendet die Leiste aus. Oben stehen der Arbeitsordner (Klick öffnet die letzten Ordner und weitere Aktionen), der Schalter «🌐 Web» und die Modellwahl. Mit «Web» sucht ASK vor der ersten Antwort eines Auftrags im Web (mit der Suchmaschine, die in ASK eingerichtet ist) und gibt die Treffer dem Modell mit; die Quellen erscheinen als Links unter der Antwort. Der Schalter ist nur aktiv, wenn ASK die Websuche für das Konto freigibt. Unten das Eingabefeld: Enter sendet, Shift+Enter macht eine neue Zeile, Pfeil nach oben und unten blättern durch die früheren Eingaben des Chats (der angefangene Text bleibt erhalten), Esc stoppt das Modell. Arbeitet das Modell gerade, reiht Enter die Nachricht ein («Wartet: …» über der Eingabe); sie geht hinaus, sobald der Auftrag fertig ist. Nach einem Stopp oder Fehler steht sie wieder im Eingabefeld. Mit `/` öffnen sich Befehle: `/neu`, `/goal`, `/suchen`, `/ordner`, `/explorer`, `/modell`, `/denken`, `/web`, `/explorer`, `/neustart` (Terminal), `/einstellungen`, `/abmelden`, `/update`, `/beta`. Darunter die Statuszeile mit Terminal und ASK; ein Klick zeigt Details.

Im Chat erscheinen Antworten als Markdown, mit Kopierknopf unter jeder Antwort und Codeblöcken mit eigenem «Kopieren». Werkzeugaufrufe sind kompakt wie in Claude Code: eine Zeile für den Aufruf (`● Befehl(…)`), eine für das Kurzresultat (`⎿ 84 Zeilen gelesen`, `⎿ +10 −4 Zeilen`, erste Fehlerzeile in Rot); ein Klick zeigt ganzen Befehl, Ausgabe oder Diff. Mehrere Lese- und Suchschritte hintereinander stehen in einer Zeile («2 Dateien gelesen, 1 Ordner angesehen»). Die Gedanken des Modells sind ausgeblendet; Ctrl+O (oder «Ctrl+O Details» unten) zeigt Gedanken und alle Details. In Antworten sind nur Dateien des Ordners farbig hervorgehoben (Klick öffnet die Vorschau), anderer Inline-Code ist dezent. Unter jedem Auftrag steht eine Zeile wie «✻ 7 Schritte · 9’840 Token · 140 Token/s · Modell 70.4 s»; der Tooltip zeigt die Werte pro Schritt. Die Tokenzahlen kommen von ASK; liefert ASK keine, schätzt die App sie aus der Textlänge und schreibt «≈» davor. Daneben steht, wie voll der Verlauf für das Modell ist («Kontext 41k / 128k (32 %)»). Die Grösse des Fensters nimmt die App aus den Angaben von ASK zum Modell oder aus einer Fehlermeldung von ASK; kennt sie sie nicht, zeigt sie nur die Tokenzahl. Ab 80 % rät sie zu einem neuen Chat. Ein Dateiname in einer Antwort öffnet per Klick die Vorschau. Die Chats eines Ordners liegen in `.ask/chats`; beim Ordnerwechsel öffnet die App den letzten Chat des neuen Ordners.

Oben neben «Web» öffnet «✻ Standard» den Regler für den Denkaufwand, wie «Reasoning Effort» in ASK: Standard (das Modell entscheidet), Tief, Mittel, Hoch, Maximal. Die App schickt `reasoning_effort` (low, medium, high), ab Hoch zusätzlich den Qwen-Schalter `/think` und `chat_template_kwargs`/`think`; lehnt ASK einen Parameter ab, lässt sie ihn weg.

Mit `/goal <Ziel>` setzt man ein Ziel für den Chat (Leiste «◎ Ziel» über der Eingabe). Das Modell arbeitet dann, bis es das Ziel mit dem Werkzeug `goal_done` als erreicht meldet; hört es vorher auf (etwa nach «Los geht's:»), stösst die App es an («↻ Weiter am Ziel»). Antwortet es zweimal hintereinander ohne Arbeitsschritt, wartet es offenbar auf die Person, und die App hält an. `/goal` zeigt das Ziel, `/goal aus` hebt es auf.

**Eingabe ändern oder neu senden.** An der letzten Eingabe eines Chats stehen «✎ Bearbeiten» und «↻ Neu senden». Beides nimmt die Eingabe und alles danach aus dem Chat; hat der Auftrag Dateien geändert, fragt die App, ob der Ordner auf den Stand davor zurückgehen soll (bei Nein bleibt alles, wie es ist). Der Text steht danach wieder im Eingabefeld, die Anhänge bleiben dabei.

**Einstellungen.** Das Zahnrad oben (oder `/einstellungen`) öffnet die Einstellungen: Adresse von ASK, Details immer zeigen, Aufgabenliste eingeklappt lassen, GitHub, Terminal, Anmeldungen im Browser löschen, Abmelden und die Version der App.

**Tastatur.** Alles Anklickbare (Chats, Dateien, Arbeitsschritte, Befehle) ist mit Tab erreichbar und mit Enter oder Leertaste auslösbar. Dialoge halten den Fokus, Esc schliesst sie, und danach steht der Fokus wieder dort, wo er war.

**Anhänge.** Bilder, Bildschirmfotos und Dokumente fügt man mit Ctrl+V ein, zieht sie ins Fenster oder wählt sie mit «+» links im Eingabefeld (höchstens 10, je bis 50 MB). Vor dem Senden stehen sie als Kacheln über der Eingabe und lassen sich mit × entfernen. Die App legt sie versteckt unter `.ask/anhaenge/<Chat>/` ab (eingefügte Bildschirmfotos als `bild-<Datum>-<Zeit>.png`, gleiche Namen als «name (2).ext») und nennt dem Modell die Pfade; sie verschwinden mit dem Chat oder nach 14 Tagen. Was das Modell daraus macht (ein bearbeitetes Dokument, ein Bild im Bericht), speichert es im Arbeitsordner. Bilder schickt sie zusätzlich direkt mit, wie die ASK-Webseite, aus den letzten zwei Eingaben mit Bildern, grosse Bilder verkleinert auf 2048 Pixel. Meldet ASK, dass das Modell keine Bilder versteht, oder lehnt es sie ab, gehen nur die Pfade mit (ENTSCHEIDE E23).

**GitHub.** Mit `/github` (oder im Ordner-Menü «GitHub …») trägt man einmal ein persönliches Token mit dem Recht «repo» ein; die App speichert es verschlüsselt, das Modell sieht es nie. Danach kann man sagen «leg das Projekt auf GitHub an», «hol das Projekt lea/website von GitHub» oder «lad die Änderungen hoch». Das Modell holt mit `github_clone`/`github_pull`, lädt mit `github_push` hoch und legt mit `github_create` ein neues, privates Repository an. Hochladen und Anlegen bestätigt man in einem Dialog; beim Hochladen schaltet das Häkchen «Für dieses Repository nicht mehr fragen» die Rückfrage für dieses Repository ab (im `/github`-Dialog «wieder fragen»). `.ask` wird nie hochgeladen, ebenso wenig, was in `.gitignore` steht (zum Beispiel `.env` mit Passwörtern). Wurde eine Datei hier und auf GitHub geändert, überschreibt die App nichts und meldet einen Konflikt; das Hochladen bleibt gesperrt, bis die Person entschieden hat, welche Fassung gilt, und das Überschreiben auf GitHub bestätigt sie immer in einem Dialog (ENTSCHEIDE E22, E33).

Ist ein Auftrag fertig, klappen seine Arbeitsschritte zu einer Zeile zusammen («▸ 23 Arbeitsschritte · 15 Min. · 2 fehlgeschlagen»); sichtbar bleiben die Schlussantwort, Hinweise, Quellen und die Token-Zeile. Ein Klick klappt die Schritte auf.

Ein Auftrag läuft, bis das Modell fertig ist. Ein Schleifenschutz hält es an, wenn es dreimal hintereinander genau denselben Schritt macht oder acht Werkzeugaufrufe hintereinander scheitern (Notbremse nach 500 Schritten). Befehle, die Programme mit Fenster starten (Word, Excel, PowerPoint, Explorer, Editor, Browser, `start`, auch hinter `cmd /c` oder `call`), lehnt die App ab: sie sperren Dateien und warten auf Eingaben. Wird der Verlauf für das Modell zu lang, schickt die App ältere Arbeitsschritte gekürzt mit (der gespeicherte Chat bleibt vollständig) und sagt es im Chat. Hinweise, Fehler und eine gestoppte Antwort bleiben im Chat stehen, auch nach dem Wiederöffnen.

Die Seitenleiste zeigt die zuletzt geänderten Dateien im ganzen Ordner (neueste zuerst) und den Inhalt des Arbeitsordners, alles live. Ein Klick zeigt eine Vorschau für Text, Bilder und PDF; «Gross» vergrössert sie. Office-Dateien lassen sich öffnen, im Explorer zeigen oder mit dem installierten Office als PDF anzeigen. Doppelklick öffnet eine Datei im zugehörigen Programm. Bei Programmen und Skripten (`.exe`, `.bat`, `.cmd`, `.lnk`, `.vbs`, `.ps1`, `.py` und allem Unbekannten) fragt die App zuerst nach und bietet «Im Explorer zeigen» an: Solche Dateien laufen beim Öffnen mit den Rechten der Person, nicht im geschützten Terminal (ENTSCHEIDE E33).

**Internet.** Das Modell sucht selbst im Internet, so oft es nötig ist (ENTSCHEIDE E26): `web_search` über die Websuche von ASK (dieselbe Suchmaschine wie auf der Webseite; ist sie nicht freigegeben oder scheitert sie, über DuckDuckGo) mit bis zu 8 Treffern, `web_fetch` holt eine Seite als lesbaren Text ohne HTML (12’000 Zeichen pro Aufruf, PDF wird gespeichert und mit `read_file` gelesen). Adressen im lokalen Netz und auf dem Laptop sind gesperrt. Die gelesenen Seiten stehen als Quellen unter der Antwort. Der Schalter «Web» lässt ASK zusätzlich vor der ersten Antwort suchen.

**Frühere Chats.** Der Schalter «◷ Chats» oben (oder `/chats`) erlaubt dem Modell, frühere Chats im selben Ordner zu nutzen (ENTSCHEIDE E28): Es bekommt eine Liste der letzten 15 Chats (Datum, Titel) und die Werkzeuge `chat_search` (Suche über Eingaben, Antworten und Werkzeugaufrufe) und `chat_read` (einen Chat als Text lesen). Ausgeschaltet beginnt jeder Chat ohne Wissen über andere. Chats anderer Ordner sieht es nie.

**Aufgabenliste.** Wie in Claude Code steht die Aufgabenliste des Modells über dem Eingabefeld, nicht mehr in der Seitenleiste: «Aufgaben ▬▬ 3/7» mit Fortschrittsbalken, Erledigtes abgehakt und durchgestrichen, die aktuelle Aufgabe hervorgehoben, bei langen Listen nur der Ausschnitt um die aktuelle. Während eines Auftrags ist sie offen und die Zeile «✻ …» nennt die aktuelle Aufgabe; danach schrumpft sie auf eine Zeile. Ctrl+T (oder ein Klick) klappt sie auf und zu. Im Chat steht nur «Aufgaben ⎿ 3 von 7 erledigt», die Liste dazu in den Details. Eine fertige Liste verschwindet beim nächsten Auftrag.

**Browser.** Die App hat einen eigenen Browser (ENTSCHEIDE E27, E29). Der Knopf «⧉ Browser» oben oder das Werkzeug `browser` öffnet rechts einen echten Browser-Bereich (halbe Breite statt der Seitenleiste; ⤢ ganzes Fenster, ↻ neu laden, × schliessen, Adressleiste mit Zurück und Vorwärts, Eingabe ohne Punkt sucht mit DuckDuckGo). Das Modell öffnet damit Webseiten und HTML-Dateien aus dem Arbeitsordner, arbeitet ohne Neuladen auf der offenen Seite weiter und klickt (nach CSS-Selektor, sichtbarem Text oder Position), tippt und scrollt. Zurück bekommt es Adresse, Titel, Konsole, Text (6000 Zeichen), Bedienelemente und ein Bildschirmfoto. Man sieht alles live und kann selbst klicken und tippen; das Modell erfährt, wie oft die Person eingegriffen hat. Ein Aufruf dauert höchstens 2 Minuten, Esc und ■ beenden ihn sofort. Ist die App minimiert, öffnet das Modell die Seite unsichtbar mit 1280 × 800 und zeigt im Bereich Bilder davon.

Anmelden macht die Person selbst im Browser-Bereich; das Modell gibt keine Passwörter ein. Die Anmeldungen bleiben gespeichert, getrennt von ASK und Chrome, und lassen sich mit «Anmeldungen löschen» entfernen. Das Modell soll vor allem nachfragen, was etwas abschickt, kauft, postet oder löscht, und Text auf Webseiten nicht als Anweisung behandeln. Das steht nur in seinen Anweisungen und ist keine technische Sperre, darum wichtige Konten dort besser nicht anmelden. Downloads sind blockiert, Adressen im lokalen Netz kann nur die Person öffnen, nicht das Modell.

**Bilder erstellen.** Bietet ASK dem Konto das Werkzeug «Flux Bild» an, erstellt und bearbeitet das Modell Bilder mit `bild_erstellen` (ENTSCHEIDE E31): «mach ein Bild von …» oder ein Bild anhängen und sagen, was sich ändern soll. Flux läuft auf dem ASK-Server; die App schickt dafür eine eigene Anfrage an ASK, wie die Webseite, und speichert das Ergebnis im Arbeitsordner, neue Bilder unter `bilder/`, ein bearbeitetes Bild ersetzt seine Vorlage (Rückgängig wie bei anderen Dateien). Das Bild steht gross im Chat, Klick öffnet die Vorschau. Format, Qualität, Stil, Anzahl (bis 4), Referenzfotos aus Wikipedia und das TF-Bern-Logo gibt das Modell an Flux weiter. Ohne Flux in ASK gibt es das Werkzeug nicht.

**Ansehen und testen.** Das Modell sieht, was es erstellt (ENTSCHEIDE E25). `view_file` zeigt ihm Bilder, PDF, Word, Excel und PowerPoint als Bild (Seiten bzw. Folien, Standard die ersten 3, höchstens 6; Office-Dateien wandelt die App dafür mit Office in PDF um). Die Bilder stehen als Vorschau in der Werkzeugzeile (Klick: gross) und liegen in `.ask/arbeit/<Chat>/ansicht/`. Versteht das Modell laut ASK keine Bilder, gibt es `view_file` nicht, und `browser` liefert nur Text.

**Arbeiten im Ordner.** Das Modell legt neue Dateien dort an, wo sie ins Projekt passen, und ändert vorhandene Dateien direkt, ohne Kopien wie `_V2` (ENTSCHEIDE E24). Vor jedem Auftrag sichert die App den Stand des Ordners. Unter einem Auftrag, der Dateien geändert hat, steht «⎿ Dateien: 1 geändert · 2 neu (…)» mit dem Knopf «↶ Rückgängig» (beim letzten solchen Auftrag); er setzt den Ordner nach einer Rückfrage auf den Stand vor dem Auftrag zurück, löscht neu angelegte Dateien und sagt es dem Modell. Gesichert wird jede Dateifassung nur einmal; Dateien über 100 MB und `.git`, `node_modules` werden nicht gesichert. Die Sicherung liegt nicht im Arbeitsordner, sondern unter `%LOCALAPPDATA%\ASK-Desktop\projekte\<Kennung>\sicherung`, wo das Modell sie nicht ändern kann (ENTSCHEIDE E33).

**Projektnotizen.** `.ask/projekt.md` ist wie `CLAUDE.md` in Claude Code: Die App gibt die Datei jedem Chat im Ordner mit, und das Modell hält darin kurz fest, was spätere Chats wissen müssen (Aufbau, wichtige Dateien, Vorlagen, Wünsche). Im Ordner-Menü öffnet «Projektnotizen bearbeiten» sie im Editor.

Im Arbeitsordner legt die App nur `.ask/` an. Dort liegen die Anleitungen (`skills/`), die Aufgabenliste, die Chats (`chats/`), die Projektnotizen, die Hilfsskripte des Modells (`skripte/<Chat>/`), seine Zwischendateien (`arbeit/<Chat>/`) und die Anhänge (`anhaenge/<Chat>/`), damit der Arbeitsordner aufgeräumt bleibt; diese drei verschwinden mit dem Chat und werden nach 14 Tagen automatisch gelöscht. Die Sicherung für «Rückgängig» und die Verbindung mit GitHub liegen ausserhalb des Ordners (siehe oben); die Sicherung wird ebenfalls nach 14 Tagen gelöscht. `.ask` ist versteckt (Windows-Attribut wie bei `.git`) und erscheint im Explorer nur mit «Ausgeblendete Elemente», umgewandelte PDFs (`pdf/`) und Aufträge an die App.

Scrollt man im Chat nach oben, bleibt die Ansicht dort, auch während das Modell arbeitet; «↓ nach unten» oder Scrollen bis ans Ende lässt sie dem Text wieder folgen. Im Ordner-Menü sind Ordner, die es nicht mehr gibt, mit «(nicht gefunden)» markiert; ein Klick entfernt sie aus der Liste, und «Aus der Liste entfernen» listet alle Ordner.

Die Ränder zwischen den Chats links, dem Chat, der Seitenleiste rechts und dem Browser lassen sich mit der Maus ziehen; ein Doppelklick auf einen Rand setzt die Standardbreite zurück, mit Tab angewählt verschieben ihn die Pfeiltasten. Der Chat bleibt mindestens 360 Pixel breit. Die Breiten merkt sich die App auf diesem Rechner.

Tastenkürzel: Ctrl+Plus/Minus/0 zoomt die Oberfläche, Ctrl+R lädt die Anmeldeseite von ASK neu.

## Bekannte Grenzen

- **Lesen ausserhalb des Ordners.** Windows verhindert Schreiben und Löschen ausserhalb des Arbeitsordners (Low Integrity Level). Genauer: ausserhalb aller Ordner, die je Arbeitsordner waren. Die Kennzeichnung bleibt an einem Ordner, auch wenn er aus der Liste entfernt wird; das Modell kann also aus Projekt B über die Shell auch in Projekt A schreiben, und dort gibt es dafür keine Sicherung. Lesen kann das Modell über die Shell aber alle Dateien, die der Nutzer lesen darf, und es hat Internetzugang. Die Datei-API von open-terminal ist zusätzlich auf den Ordner gesperrt. Siehe MACHBARKEIT, Abschnitt 5.
- **Originale im Ordner.** Das Modell ändert Dateien direkt; Windows schützt sie nicht (ENTSCHEIDE E14). Schutz bietet die Sicherung mit «Rückgängig» (E24). Sie gilt 14 Tage, nur für den letzten Auftrag mit Änderungen und nicht für Dateien über 100 MB, `.git` und Ordner mit mehr als 5000 Dateien. Fürs Rückgängigmachen müssen betroffene Dateien in Word, Excel oder PowerPoint geschlossen sein.
- **Werkzeuge unter Low.** Windows PowerShell 5.1 und die Unix-Werkzeuge von Git for Windows starten im Terminal nicht. Das Modell arbeitet mit `cmd.exe` und Python. Office lässt sich nur über die App umwandeln, nicht direkt steuern.
- **Ordner auf FAT/exFAT-USB-Sticks** oder Netzlaufwerken werden abgelehnt, weil sie sich nicht schützen lassen.
- **Abhängigkeit von ASK.** Die App braucht nur, was auch die ASK-Webseite braucht: Anmeldung und Zugriff auf ein Modell. Ob ASK die Werkzeugliste (`tools`) an das Modell weitergibt, ist mit dem echten ASK noch nicht geprüft. Lehnt ASK sie ab oder schreibt das Modell Aufrufe als `<tool_call>` in den Text, wechselt die App von selbst in einen Textmodus und beschreibt die Werkzeuge im Systemprompt.
- **Schalter «Web» sucht einmal pro Auftrag.** Mit dem Schalter sucht ASK vor der ersten Antwort. Während der Arbeit sucht das Modell selbst mit `web_search` (siehe «Internet»).
- **Werkzeuge und Wissen aus ASK** (die Werkzeuge und Wissenssammlungen, die einem Modell in ASK zugeordnet sind) stehen in der App nicht zur Verfügung, ausser Flux für Bilder.
- **Flux nicht mit dem echten ASK geprüft.** Beim Bau war Flux nur im Beta-Arbeitsbereich verfügbar. Wie ASK das Bild über die Schnittstelle zurückgibt, ist deshalb nicht geprüft; die App sucht an allen üblichen Stellen (E31). Kommt kein Bild, sagt die App, was ASK stattdessen geantwortet hat, und legt die Antwort unter `.ask/arbeit/<Chat>/flux-….json` ab. Während der Anfrage steht kurz ein Chat «ASK Desktop: Bild» im ASK-Verlauf.
- **GitHub ohne Git.** Der Abgleich läuft über die Schnittstelle von GitHub, nicht mit Git: kein Zusammenführen innerhalb einer Datei, keine Branches ausser dem Standard-Branch, keine Git-Historie im Ordner. Ist ein Ordner schon ein Git-Repository, bleibt `.git` unberührt. Mit dem echten GitHub ist der Abgleich noch nicht getestet, nur mit einem Nachbau der Schnittstelle.
- **Kontextanzeige.** Nennt ASK weder Tokenzahlen noch die Grösse des Kontextfensters, ist die Anzeige eine Schätzung aus der Textlänge ohne Prozentangabe.
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
- Die Kennzeichnung «Low» von einem Ordner wieder entfernen, wenn er aus der Liste genommen wird.
- `pip install` in einer virtuellen Umgebung (`.venv`) scheitert im Terminal an `PIP_USER=1`; ohne `.venv` geht es.
- Vorschau für Office-Dateien direkt beim Anklicken, sobald klar ist, dass Office auf allen Laptops vorhanden ist.
