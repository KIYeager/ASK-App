# ASK Desktop

ASK Desktop ist eine Windows-App für die Lernenden der TF Bern. Sie ist eine eigene Chat-Oberfläche für ASK (die Open-WebUI-Instanz der Schule), im Stil eines Terminal-Agenten: Man beschreibt einen Auftrag, und das Sprachmodell arbeitet selbständig in einem Ordner auf dem Laptop. Es liest und schreibt Dateien, führt Python aus, führt eine Aufgabenliste, prüft die Ergebnisse und arbeitet dabei direkt in den Dateien des Ordners, wie Claude Code in einem Projekt. Jeder Auftrag lässt sich rückgängig machen. Jeder Schritt erscheint im Chat als Zeile wie `● Befehl(python auswertung.py)` mit der Ausgabe darunter.

Technisch meldet man sich einmal auf der ASK-Seite an (auch über das Microsoft-Konto); danach liest die App das Login-Token und spricht das Modell direkt über die Schnittstelle von ASK an (`/api/chat/completions`, gestreamt, mit Werkzeugliste). Die Werkzeuge führt die App selbst aus, über einen eigenen Terminal-Server in Rust (Nachbau der Schnittstelle von [open-terminal](https://github.com/open-webui/open-terminal) 0.14.2), der lokal auf `127.0.0.1` mit niedriger Integritätsstufe läuft. ASK sieht nur Text; der ASK-Server erreicht den Laptop nie, und ASK braucht keine Freigabe für Tool-Server (ENTSCHEIDE E19).

Neue Version veröffentlichen: siehe [unten](#neue-version-veröffentlichen). Weitere Dokumente: [Machbarkeit](docs/MACHBARKEIT.md), [Entscheide](docs/ENTSCHEIDE.md), [Testprotokoll](docs/TESTPROTOKOLL.md).

## Aufbau

| Teil | Ort | Aufgabe |
|---|---|---|
| Hülle | `src-tauri/src/` | Tauri 2 in Rust (ENTSCHEIDE E60): Hauptfenster mit den Webansichten Oberfläche, ASK-Anmeldung und Browser, DevTools-Protokoll von WebView2, Dialoge, Benachrichtigungen, Updates, Protokoll `askfile` |
| Logik | `src-tauri/core/` (Crate `ask-core`) | Steuerung der App (`app.rs`), Anmeldung und Modell-Anfragen an ASK (`askapi.rs`), Arbeitsschleife (`agent.rs`), Werkzeuge (`tools.rs`), Ordnerwahl und -sperre, Terminal-Verwaltung, Sicherung, GitHub, Office-Umwandlung; Chats in `.ask/chats` |
| Oberfläche | `src/renderer/` (React, Vite), Brücke `src/renderer/src/bridge.js` | Projekte und Chats, Chat mit Arbeitsschritten, Fragen und Rückfragen, Eingabe mit `/`-Befehlen, rechts Dateien, Vorschau, Browser und GitHub, Dialoge, Anmeldeansicht. Datenmodell des Chats in `src/renderer/src/lib/chatModel.mjs` |
| Terminal-Server | `src-tauri/terminal/` (Crate `ask-terminal`) | der Server läuft in derselben ASK.exe (`--terminal`), Low Integrity und Job-Objekt setzt `NativeLowBackend`; Ordnersperre in der Datei-API, CORS |
| Anweisungen | `resources/anweisungen/` | Systemprompt für das Modell und Zusatz zur Befehlsbeschreibung |
| Skills | `resources/skills/` | Anleitungen für Word, Excel, PowerPoint, PDF (erstellen und vorhandene Dateien bearbeiten), Hilfsfunktionen `bearbeiten.py`, Prüfskript `pruefen.py`, Platz für Vorlagen in `vorlagen/` |
| Tests | `tests/` | Unit-, Python-, End-to-End- und Pakettests, Beispieldateien |

Die Isolationsvariante steckt in `src-tauri/core/src/terminal.rs` (`NativeLowBackend`). Eine andere Variante (AppContainer, Sandbox, WSL) muss nur `start`, `stop` und `running` gleich anbieten.

## Build

Voraussetzungen auf dem Build-Rechner: Windows 11, Node.js 22 oder neuer, Rust (stabil, mit `rustup` und den «Desktop development with C++»-Werkzeugen von Visual Studio), Git, Internetzugang. Ein installiertes Python braucht es nicht. Die WebView2-Laufzeit ist auf Windows 10 und 11 vorhanden.

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

`npm start` (`tauri dev`) startet Vite und die App aus den Quellen, mit Neuladen bei jeder Änderung der Oberfläche; der Rust-Teil wird dabei neu kompiliert. `npm run dist` (`scripts/dist.js`, `tauri build`) baut den Installer `ASK-Setup-<Version>.exe` im Unterordner `dist` des Build-Ordners, `npm run dist:portable` die einzelne EXE `ASK-<Version>.exe` ohne Installation (aus dem App-Ordner, den `npm run dist` anlegt). Ohne Signaturschlüssel für Updates: `npm run dist -- --ohne-signatur`. Nur das Python neu bauen: `npm run build:python`; `npm run dist` packt es danach mit `scripts/pack-python.js` in `python-pack\python.zip` (ENTSCHEIDE E39). Icon und Startbild entstehen mit `scripts/make-icon.py`.

Oberfläche entwickeln: `npm run dev:ui` öffnet sie im Browser mit Beispieldaten (`src/renderer/src/dev/mockApi.js`, ohne App, mit einem vorgespielten Auftrag samt Frage und Rückfrage; `?tour=1` zeigt die Einführung, `?leer=1` einen leeren Chat).

Tests: siehe [Testprotokoll](docs/TESTPROTOKOLL.md), Abschnitt «Tests wiederholen». Bei jedem Pull Request laufen auf GitHub (`.github/workflows/ci.yml`) ESLint und die Unit-Tests auf Linux, die Rust-Tests mit Clippy, Ruff und die Python-Tests ohne Windows, und auf Windows die Tests des Terminals (in einem Ordner mit Umlaut), der Skills, die Rust-Tests und ein Build der App. Lokal: `npm run lint`, `npm test`, `npm run test:rust`.

Lokal bauen braucht es nur noch zum Ausprobieren. Veröffentlichte Versionen baut GitHub (nächster Abschnitt).

## Neue Version veröffentlichen

Neue Versionen baut GitHub, nicht der eigene Rechner. Normalerweise entsteht eine neue Version beim Merge eines Pull Requests; dazu wählt man mit einem Label, ob es eine Beta, eine Version für alle oder keine neue Version wird. GitHub baut auf einem Windows-Rechner Installer und EXE und stellt sie in das öffentliche Repository [KIYeager/ASK-Releases](https://github.com/KIYeager/ASK-Releases). Alle installierten Apps bieten das Update danach von selbst an (ENTSCHEIDE E37, E38). Der Code in ASK-App bleibt privat; ASK-Releases enthält nur die fertigen Dateien, und die App holt sie ohne Token.

### Einmal einrichten

Nur vor dem allerersten Release nötig.

1. **Release-Repository anlegen.** Auf GitHub «New repository», Owner `KIYeager`, Name `ASK-Releases`, **Public**, Häkchen bei «Add a README file» (ein leeres Repository nimmt keine Releases an). Danach den Inhalt von [docs/ASK-Releases-README.md](docs/ASK-Releases-README.md) als README einfügen: Er erklärt den Lernenden, was sie herunterladen sollen.
1a. **Download-Seite einschalten.** In ASK-Releases: Settings, Pages, bei «Build and deployment» Source «Deploy from a branch», Branch `main`, Ordner `/docs`, «Save». Danach in ASK-App unter Actions den Workflow «Download-Seite» mit «Run workflow» einmal von Hand starten; er legt `docs/index.html` in ASK-Releases ab. Nach ein bis zwei Minuten steht die Seite unter https://kiyeager.github.io/ASK-Releases/. Später schreibt der Release-Workflow sie bei jeder normalen Version selbst neu (ENTSCHEIDE E63).
2. **Token anlegen.** GitHub, Profilbild, Settings, Developer settings, Personal access tokens, Fine-grained tokens, «Generate new token»:
   - Resource owner: `KIYeager`
   - Repository access: «Only select repositories», nur `ASK-Releases`
   - Permissions, Repository permissions: «Contents: Read and write»
   - Expiration: nach Wunsch. Läuft das Token ab, scheitert das nächste Release beim Veröffentlichen, dann ein neues anlegen und Schritt 3 wiederholen.
3. **Token in ASK-App eintragen.** Im Repository ASK-App: Settings, Secrets and variables, Actions, «New repository secret». Name `RELEASES_TOKEN`, Wert das Token aus Schritt 2.
3a. **Updates signieren.** Die Tauri-Fassung installiert nur Updates mit gültiger Signatur (ENTSCHEIDE E60). Einmal auf dem eigenen Rechner `npx tauri signer generate -w %USERPROFILE%\.tauri\ask.key` ausführen und ein Passwort wählen. Den Inhalt von `ask.key` als Secret `TAURI_SIGNING_PRIVATE_KEY` eintragen, das Passwort als `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, und den Inhalt von `ask.key.pub` in `src-tauri/tauri.conf.json` bei `plugins.updater.pubkey` einsetzen (per Pull Request). Den Schlüssel gut aufbewahren: Ohne ihn können installierte Apps keine Updates mehr annehmen.
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
| «Secret TAURI_SIGNING_PRIVATE_KEY fehlt» | Einmal einrichten, Schritt 3a. Danach «Re-run jobs». |
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

Für die Lernenden gibt es eine Download-Seite mit der neuesten Version, ihren Neuerungen und einer Anleitung für die Warnung von Windows:

https://kiyeager.github.io/ASK-Releases/

Die Seite schreibt der Workflow «Download-Seite» nach jeder normalen Version neu, Beta-Versionen ändern sie nicht (ENTSCHEIDE E63). Wer die Datei ohne Seite will, nimmt den festen Link, der immer auf die neueste Version zeigt: https://github.com/KIYeager/ASK-Releases/releases/latest/download/ASK-Setup.exe

`ASK-Setup.exe` ausführen. Die Installation läuft pro Nutzer nach `%LOCALAPPDATA%\ASK`, ohne Admin-Rechte, und legt Verknüpfungen im Startmenü und auf dem Desktop an. Python, Bibliotheken und Anleitungen sind dabei; die Lernenden müssen nichts installieren. Python liegt in der App als ein einziges Archiv; beim ersten Start entpackt ASK es nach `%LOCALAPPDATA%\ASK-Desktop\python\<Fassung>` (unten in der Statuszeile «Python wird eingerichtet (einmalig) … 40 %», je nach Laptop und Virenscanner einige Minuten). Danach startet ASK direkt, und ein Update entpackt Python nur neu, wenn sich Python selbst geändert hat (ENTSCHEIDE E39). Eine frühere Electron-Fassung von ASK entfernt der Installer selbst; Einstellungen, Chats und die Verbindung mit GitHub bleiben, bei ASK meldet man sich einmal neu an (ENTSCHEIDE E60). Deinstallieren über Einstellungen, Apps; das entpackte Python wird dabei entfernt. Einstellungen bleiben in `%APPDATA%\ASK`, Laufzeitdaten in `%LOCALAPPDATA%\ASK-Desktop`.

Ohne Installation: `ASK-<Version>.exe` irgendwo ablegen und doppelklicken. Beim ersten Start entpackt die EXE die App einmal nach `%LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build>` (Startbild «ASK wird gestartet …»), danach startet sie die App direkt von dort. Eine neuere EXE entpackt sich beim ersten Start neu, die App löscht dann den älteren Stand. Es gibt keine Verknüpfungen und keinen Eintrag unter «Apps». Einstellungen und Laufzeitdaten liegen am gleichen Ort wie bei der installierten App. Installierte App und EXE laufen nicht gleichzeitig.

**Updates.** Die installierte App sucht 20 Sekunden nach dem Start und danach alle 6 Stunden in ASK-Releases nach einer neueren Version. Gibt es eine, fragt sie einmal «ASK 0.2.0 ist da. Jetzt aktualisieren?» und zeigt den Text des Releases. Nach «Jetzt aktualisieren» lädt sie das Update im Hintergrund; eine Karte unten links und die Taskleiste zeigen den Fortschritt, man kann weiterarbeiten. Updates sind signiert; ASK installiert nur geprüfte Pakete. Ist es geladen, zählt die Karte 10 Sekunden herunter, dann schliesst sich ASK, ein kleines Fenster zeigt das Installieren (ohne Admin-Rechte), und ASK startet von selbst wieder; «Jetzt neu starten» wartet nicht, «Später» installiert erst beim Schliessen von ASK. Während das Modell arbeitet, fragt und startet sie nie, sondern erst danach (ENTSCHEIDE E40). Chats, Ordner und Einstellungen bleiben. Bei «Später» in der Frage fragt sie bis zum nächsten Start nicht mehr von selbst, unten steht aber «Update 0.2.0» zum Anklicken. Von Hand: «Nach Updates suchen» in den Einstellungen oder im Ordner-Menü, `/update` oder der Eintrag unten. Ohne Internet bleibt die Suche still. Fragen und Meldungen zu Updates erscheinen als Dialog in der App selbst, mit Versionen und der Liste der Neuerungen, nicht als Fenster von Windows; dasselbe gilt für alle anderen Rückfragen und für die Menüs von Projekt und Ordner. Rechtsklick zeigt das Menü von WebView2 mit Ausschneiden, Kopieren, Einfügen und der Rechtschreibprüfung (ENTSCHEIDE E56, E60).

Die einzelne EXE kann sich nicht selbst ersetzen. Sie sucht genauso, meldet eine neue Version aber nur mit «Download öffnen» (Seite des Releases); die neue `ASK.exe` entpackt sich beim ersten Start und räumt den alten Stand weg.

Installer und EXE sind nicht signiert. Windows SmartScreen kann deshalb beim ersten Start warnen («Weitere Informationen», «Trotzdem ausführen»).

## Bedienung

Beim ersten Start erscheint ein Hinweis in einfachem Deutsch (das Modell kann im Ordner ändern und löschen, darum nur mit Kopien arbeiten, Inhalte gehen an ASK). Danach legt man das erste Projekt an: Name eingeben, die App erstellt `%USERPROFILE%\ASK-Projekte\<Name>`; oder man öffnet einen vorhandenen Ordner als Projekt. Nach der Anmeldung zeigt eine kurze Einführung in vier Karten, wo was ist (siehe «Einführung»). Ist man nicht bei ASK angemeldet, zeigt die App die Anmeldeseite von ASK. Solange sie prüft, ob man angemeldet ist, steht dort «Verbinde mit ASK …» bzw. «Anmeldung wird geprüft …»; die Oberfläche von Open WebUI ist nie zu sehen. Lädt die Anmeldeseite in der App nicht, kann man sich in einem eigenen Fenster anmelden oder im normalen Browser und das Token von dort in der App einfügen («Anmelden» und «Im Browser …» in den Einstellungen und in der Meldung). Findet die App den Namen von ASK nicht, nimmt sie für ihre eigenen Anfragen an ASK verschlüsseltes DNS von Quad9 zu Hilfe (ENTSCHEIDE E57, E60).

**Aufbau.** Links die Projekte mit ihren Chats, in der Mitte der Chat, rechts Dateien, Vorschau, Browser und GitHub (ENTSCHEIDE E42, E47). Oben das TF-Zeichen, Reiter für die zuletzt geöffneten Chats und die Knöpfe zum Ein- und Ausblenden der beiden Seiten. Ein leerer Chat zeigt das Eingabefeld in der Mitte mit kurzen Vorschlägen; sobald der Chat beginnt, steht es unten. Hell und Dunkel folgen Windows oder der Wahl in den Einstellungen.

**Projekte.** Ein Projekt ist ein Arbeitsordner mit Namen und Farbe (ENTSCHEIDE E43). Ein Klick auf einen Chat öffnet ihn; liegt er in einem anderen Projekt, wechselt die App dorthin (das Terminal startet neu). «+» neben dem Projekt beginnt dort einen neuen Chat, «…» oder Rechtsklick öffnet das Menü des Projekts (Umbenennen, Farbe, im Explorer öffnen, Projektnotizen, GitHub, aus der Liste entfernen; der Ordner bleibt). «…» an einem Chat (oder Rechtsklick) öffnet sein Menü: Umbenennen, Anheften (angeheftete Chats stehen oben), Löschen. Die Suche findet Titel und ab zwei Zeichen auch Inhalte der Chats aller Projekte («Im Inhalt gefunden»). Mehrere Chats arbeiten gleichzeitig, auch in verschiedenen Projekten (ENTSCHEIDE E46): Während einer arbeitet, kann man einen anderen öffnen oder einen neuen beginnen. Ein Kreisel in der Liste zeigt, welche Chats gerade arbeiten, «Stoppen» in ihrem Menü hält sie an. Ältere Einstellungen mit der Liste der zuletzt verwendeten Ordner werden beim ersten Start zu Projekten.

**Eingabe.** Enter sendet, Shift+Enter macht eine neue Zeile, Pfeil nach oben holt die letzte Eingabe, Ctrl+N beginnt einen neuen Chat. Während das Modell arbeitet, reiht Enter eine Folgenachricht ein, die nach dem Auftrag hinausgeht (ENTSCHEIDE E34); Esc oder ■ stoppt, die wartende Nachricht kommt dann zurück ins Eingabefeld. Rechts im Eingabefeld zeigt ein Ring, wie voll das Kontextfenster ist (ab 80 % gelb; E36). Im Eingabefeld: Büroklammer (Anhänge), «Web» (ASK sucht vor der ersten Antwort im Web, nur wenn ASK es für das Konto freigibt; die Quellen stehen unter der Antwort), der Denkaufwand, der Modus und rechts die Modellwahl. `@` schlägt Dateien des Projekts vor; der gewählte Pfad steht als `@pfad` im Text, und das Modell erfährt, dass es diese Datei meint. Mit `/` öffnen sich Befehle: `/neu`, `/projekt`, `/goal`, `/plan`, `/auto`, `/nachfragen`, `/suchen`, `/ordner`, `/explorer`, `/dateien`, `/github`, `/modell`, `/denken`, `/web`, `/chats`, `/einstellungen`, `/nutzung`, `/neustart` (Terminal), `/abmelden`. Unten links stehen Konto und Verbindung; fehlt etwas (nicht angemeldet, Terminal läuft nicht), steht es dort, ein Klick zeigt Details. Das Zahnrad öffnet die Einstellungen: ASK-Konto (Anmelden, Im Browser, Abmelden), Darstellung, Prüfmodell für «Auto», Benachrichtigungen, frühere Chats, ASK-Konto, GitHub, Nutzung, Version und Updates. Während ein Update lädt oder bereit ist, steht unten links eine Karte mit Fortschritt bzw. Countdown (ENTSCHEIDE E40, E56).

**Tabs.** Oben stehen die zuletzt geöffneten Chats dieser Sitzung als Tabs (höchstens sechs), mit einem Punkt in der Farbe des Projekts; er pulsiert, solange der Chat arbeitet. Ziehen sortiert um, Doppelklick benennt um, Mittelklick oder × schliesst, Rechtsklick öffnet ein Menü (Stoppen, Umbenennen, Schliessen, Andere Tabs schliessen, Tabs rechts davon schliessen, Neuer Chat). Tastatur: Ctrl+W schliesst den Tab, Ctrl+Tab und Ctrl+Shift+Tab wechseln, Ctrl+1 bis Ctrl+8 springen zum Tab, Ctrl+9 zum letzten, Ctrl+Shift+T öffnet den zuletzt geschlossenen wieder. Schliessen entfernt nur den Tab, nicht den Chat.

**Chat.** Die Eingabe steht als Sprechblase rechts. Die Arbeitsschritte eines Auftrags stehen in einer Karte («4 Arbeitsschritte»): pro Werkzeug eine Zeile mit Symbol, Aufruf und Kurzresultat («84 Zeilen gelesen», «+10 −4 Zeilen», erste Fehlerzeile in Rot); ein Klick zeigt ganzen Befehl, Ausgabe oder Diff. Mehrere Lese- und Suchschritte hintereinander stehen in einer Zeile («2 Dateien gelesen, 1 Ordner angesehen»). Während der Arbeit ist die Karte offen; ist der Auftrag fertig und hat er vier oder mehr Schritte, klappt sie zu, sichtbar bleiben Antwort, Hinweise, Quellen und Änderungen. Die Schlussantwort steht darunter als Markdown; Dateinamen des Projekts sind hervorgehoben (Klick öffnet die Vorschau). «Kopieren» und die Zeile «4 Schritte · 9’840 Token · 140 Token/s · Modell 70.4 s» erscheinen beim Darüberfahren; der Tooltip zeigt die Werte pro Schritt. Liefert ASK keine Tokenzahlen, schätzt die App sie und schreibt «≈» davor. Die Gedanken des Modells sind ausgeblendet; Ctrl+O zeigt Gedanken und alle Details. Die Chats eines Projekts liegen in `.ask/chats`.

**Fragen von ASK.** Fehlt dem Modell eine Entscheidung oder Angabe, fragt es mit dem Werkzeug `frage_stellen` (ENTSCHEIDE E44). Die Frage erscheint als Karte im Chat, mit Antworten zum Anklicken und einem Feld für eine eigene Antwort; auch eine Eingabe im normalen Eingabefeld gilt als Antwort. Das Modell wartet und arbeitet danach weiter. Stoppen beendet die Frage ohne Antwort.

**Modus.** Im Eingabefeld wählt man, wie selbständig ASK arbeitet (ENTSCHEIDE E52); Shift+Tab wechselt der Reihe nach. «Auto» (Standard) lässt jeden Befehl und jeden heiklen Dateischritt kurz von einem Modell in ASK prüfen: Harmloses läuft ohne Rückfrage, bei Gefährlichem (etwas ausserhalb des Projekts, Systemeinstellungen, Programme aus dem Internet, viel löschen) erscheint eine Rückfrage mit der Begründung und «Von ASK geprüft»; während der Prüfung steht «Prüft, ob der Schritt sicher ist …». Klappt die Prüfung nicht, gelten feste Regeln (unten). Welches Modell prüft, steht in den Einstellungen (Standard: wie im Chat). «Nachfragen» fragt vor jedem Befehl und jeder Änderung. «Plan» ändert nichts: Das Modell liest und sucht nur, fragt bei Unklarem und legt dann einen Plan als Karte vor. «Umsetzen» wechselt auf Auto und lässt den Plan ausführen, «Ändern» schreibt «Ändere den Plan:» ins Eingabefeld.

**Rückfrage vor heiklen Schritten.** Feste Regeln, wenn die Prüfung im Modus «Auto» nicht möglich ist: vor dem Löschen (`del`, `rd`, `rmdir`, `Remove-Item`, `os.remove` usw.), vor dem Überschreiben einer vorhandenen Datei, die das Modell nicht im selben Auftrag erstellt hat, und vor Befehlen mit Pfaden ausserhalb des Projekts hält das Modell an (ENTSCHEIDE E41). Im Chat steht, was es tun will: «Erlauben», «Immer in diesem Chat» (für diese Art Schritt) oder «Ablehnen». Abgelehnt erfährt das Modell es und macht ohne den Schritt weiter. Ist die App im Hintergrund, blinkt sie in der Taskleiste und meldet sich bei Windows (siehe unten). Was ein Python-Skript intern löscht, erkennt die App nicht; dafür bleibt «Rückgängig».

Im Eingabefeld öffnet der Knopf mit der Glühbirne den Regler für den Denkaufwand, wie «Reasoning Effort» in ASK: «Automatisch» oder Tief, Mittel, Maximal. Bei «Automatisch» beginnt jeder Auftrag bei Mittel, und das Modell stellt mit dem Werkzeug `denkaufwand` selbst um (Maximal für Schwieriges, Tief für Einfaches); das Eingabefeld zeigt dann zum Beispiel «Auto · Maximal». Die App schickt `reasoning_effort` low, medium bzw. xhigh, die drei Stufen, die ASK kennt; bei Maximal zusätzlich den Qwen-Schalter `/think` und `chat_template_kwargs`/`think` für Modelle über vLLM oder Ollama. Lehnt ein Modell eine Stufe ab und nennt die erlaubten, nimmt die App die nächste; sonst lässt sie den Parameter für dieses Modell weg (ENTSCHEIDE E62).

Mit `/goal <Ziel>` setzt man ein Ziel für den Chat (Leiste «◎ Ziel» über der Eingabe). Das Modell arbeitet dann, bis es das Ziel mit dem Werkzeug `goal_done` als erreicht meldet; hört es vorher auf (etwa nach «Los geht's:»), stösst die App es an («↻ Weiter am Ziel»). Antwortet es zweimal hintereinander ohne Arbeitsschritt, wartet es offenbar auf die Person, und die App hält an. `/goal` zeigt das Ziel, `/goal aus` hebt es auf.

**Verbindung und Benachrichtigungen.** Bricht die Verbindung zu ASK ab oder ist ASK kurz überlastet (502, 503, 504, 429), wartet die App und versucht es bis zu fünfmal nochmals (2, 5, 10, 20, 30 Sekunden); im Chat steht «Verbindung zu ASK unterbrochen. Neuer Versuch in 5 s (2/5)». Eine halb angekommene Antwort verfällt dabei. Fehler des Modells selbst werden nicht wiederholt. Ist die App im Hintergrund oder läuft der Chat nicht im Vordergrund, meldet Windows, wenn ASK fertig ist (Aufträge ab 8 Sekunden), etwas fragt oder um Erlaubnis bittet; ein Klick öffnet den Chat. Abschalten in den Einstellungen.

**Eingabe ändern oder neu senden.** An der letzten Eingabe eines Chats stehen beim Darüberfahren «Bearbeiten» und «Neu senden». Beides nimmt die Eingabe und alles danach aus dem Chat; hat der Auftrag Dateien geändert, fragt die App, ob der Ordner auf den Stand davor zurückgehen soll (bei Nein bleibt alles, wie es ist). Der Text steht danach wieder im Eingabefeld, die Anhänge bleiben dabei.

**Einstellungen.** Das Zahnrad unten links (oder `/einstellungen`) öffnet die Einstellungen: Darstellung, Prüfmodell für «Auto», Benachrichtigungen, frühere Chats, Gedächtnis, Skills, GitHub, Details immer zeigen, Aufgabenliste eingeklappt lassen, Version mit «Nach Updates suchen» und «Beta-Versionen erhalten», Nutzung, «Einführung zeigen», Terminal und Anmeldungen im Browser löschen.

**Nutzung.** `/nutzung` oder «Nutzung …» in den Einstellungen zeigt, wie viel man mit ASK gearbeitet hat (ENTSCHEIDE E49): oben Aufträge, erzeugte Token, angelegte und geänderte Dateien und die Zeit des Modells, darunter die letzten 30 Tage als Balken (Aufträge oder Token, Darüberfahren zeigt den Tag) und eine Tabelle pro Projekt mit Chats, Aufträgen, Arbeitsschritten, Token, Dateien und Zeit. Gezählt wird aus den Chats in `.ask/chats` aller Projekte, nur auf diesem Laptop (`src/main/usage.js`); nichts davon geht an ASK. Eingaben, die die App selbst schickt («Weiter am Ziel»), zählen nicht als Auftrag, ein rückgängig gemachter Auftrag zählt bei den Dateien nicht. «≈» heisst, dass ein Teil der Token geschätzt ist. Darunter steht ein Abschnitt «ASK-Webseite» (Zahl der eigenen Chats auf der Webseite, allenfalls Nutzungszahlen), aber nur, wenn ASK dem Konto solche Angaben gibt; ob das echte ASK das tut, ist nicht geprüft.

**Neu in ASK.** Nach jedem Update erscheint einmal eine Karte mit dem, was neu ist (ENTSCHEIDE E65).

**Vorlagen der TF Bern.** Gibt es `C:\Vorlagen` (Laptops der Schule), kennt das Modell die Vorlagen darin, liest sie und arbeitet in einer Kopie im Projekt; in `C:\Vorlagen` schreibt es nie. Mit `@` lässt sich eine Vorlage direkt wählen (E66).

**Hilfe bei Fehlern.** Fehler zeigen, was zu tun ist, und «Details kopieren» für die Lehrperson (E69).

**Einführung.** Beim ersten Start, nach dem Hinweis, dem ersten Projekt und der Anmeldung, zeigt die App vier kurze Karten, die auf die Teile der Oberfläche zeigen: Projekte links, Eingabefeld mit Web, Denkaufwand und Modell, Arbeitsschritte, Fragen und Rückfragen im Chat, Dateien rechts mit «Rückgängig». «Weiter» (oder Pfeil nach rechts), «Überspringen» (oder Esc). Die App merkt sich, dass die Einführung gesehen ist; wer ASK schon vor dieser Version benutzt hat, bekommt sie nicht von selbst. «Einführung zeigen» in den Einstellungen zeigt sie wieder.

**Tastatur.** Alles Anklickbare (Chats, Dateien, Arbeitsschritte, Befehle) ist mit Tab erreichbar und mit Enter oder Leertaste auslösbar. Dialoge halten den Fokus, Esc schliesst sie, und danach steht der Fokus wieder dort, wo er war.

**Anhänge.** Bilder, Bildschirmfotos und Dokumente fügt man mit Ctrl+V ein, zieht sie ins Fenster oder wählt sie mit der Büroklammer im Eingabefeld (höchstens 10, je bis 50 MB). Vor dem Senden stehen sie als Kacheln über der Eingabe und lassen sich mit × entfernen. Die App legt sie versteckt unter `.ask/anhaenge/<Chat>/` ab (eingefügte Bildschirmfotos als `bild-<Datum>-<Zeit>.png`, gleiche Namen als «name (2).ext») und nennt dem Modell die Pfade; sie verschwinden mit dem Chat oder nach 14 Tagen. Was das Modell daraus macht (ein bearbeitetes Dokument, ein Bild im Bericht), speichert es im Arbeitsordner. Bilder schickt sie zusätzlich direkt mit, wie die ASK-Webseite, aus den letzten zwei Eingaben mit Bildern, grosse Bilder verkleinert auf 2048 Pixel. Meldet ASK, dass das Modell keine Bilder versteht, oder lehnt es sie ab, gehen nur die Pfade mit (ENTSCHEIDE E23).

**GitHub.** Mit `/github` (oder in den Einstellungen bzw. im Menü des Projekts «GitHub …») trägt man einmal ein persönliches Token mit dem Recht «repo» ein; die App speichert es verschlüsselt, das Modell sieht es nie. Der Reiter «GitHub» rechts zeigt für das sichtbare Projekt das verbundene Repository (Klick öffnet es im Browser), den Branch, was hier seit dem letzten Abgleich geändert, neu oder gelöscht ist, und ob es auf GitHub Neueres gibt. «Holen» übernimmt die Änderungen von GitHub, «Hochladen» lädt die Änderungen mit einer kurzen Beschreibung als Commit hoch. Ist das Projekt mit keinem Repository verbunden, holt «Repository verbinden» eines (`besitzer/name`) in den Ordner, und «Neues Repository» legt eines an (standardmässig privat) und lädt das Projekt hoch. Mit der Auswahl des Branches wechselt man ihn: Der Ordner bekommt dessen Stand; das geht nur, wenn hier nichts mehr hochzuladen ist, sonst sagt die App warum. «+ Branch» legt einen neuen Branch auf dem Stand des letzten Abgleichs an; Änderungen hier gehen beim nächsten Hochladen dorthin. Auf einem eigenen Branch erstellt «Erstellen» unter «Pull Requests» einen Pull Request in den Standard-Branch (Titel, Beschreibung); darunter stehen die offenen Pull Requests, ein Klick öffnet sie im Browser. Der Reiter lädt nach Änderungen an Dateien und nach jedem Auftrag neu. Im Chat geht dasselbe: «leg das Projekt auf GitHub an», «hol das Projekt lea/website von GitHub», «lad die Änderungen hoch», «mach einen Branch neue-startseite», «erstell einen Pull Request». Das Modell holt mit `github_clone`/`github_pull`, lädt mit `github_push` hoch, legt mit `github_create` ein neues, privates Repository an, zeigt, wechselt und legt Branches mit `github_branch` an und zeigt und erstellt Pull Requests mit `github_pr`. Hochladen, Anlegen und Pull Requests des Modells bestätigt man in einem Dialog; Hochladen und Anlegen fragen auch im Reiter. Beim Hochladen schaltet das Häkchen «Für dieses Repository nicht mehr fragen» die Rückfrage für dieses Repository ab (im `/github`-Dialog «wieder fragen»). Während das Modell im Projekt arbeitet, sind die Knöpfe im Reiter gesperrt, und während man dort abgleicht, wartet das Modell. `.ask` wird nie hochgeladen. Wurde eine Datei hier und auf GitHub geändert, überschreibt die App nichts und meldet einen Konflikt; im Reiter lässt sich die Fassung von hier nach einer Rückfrage trotzdem hochladen (ENTSCHEIDE E22, E47).

Ein Auftrag läuft, bis das Modell fertig ist. Ein Schleifenschutz hält es an, wenn es dreimal hintereinander genau denselben Schritt macht oder acht Werkzeugaufrufe hintereinander scheitern (Notbremse nach 500 Schritten). Befehle, die Programme mit Fenster starten (Word, Excel, PowerPoint, Explorer, Editor, Browser, `start`, auch hinter `cmd /c` oder `call`), lehnt die App ab: sie sperren Dateien und warten auf Eingaben. Wird der Verlauf für das Modell zu lang, schickt die App ältere Arbeitsschritte gekürzt mit (der gespeicherte Chat bleibt vollständig) und sagt es im Chat. Hinweise, Fehler und eine gestoppte Antwort bleiben im Chat stehen, auch nach dem Wiederöffnen.

**Dateien und Vorschau.** Rechts zeigt der Reiter «Dateien» den Inhalt des Projekts wie der Explorer, live: Ein Klick auf einen Ordner öffnet ihn, über den Pfad oben oder ← geht es zurück; die Spalten Name, Geändert und Grösse sortieren. Was der letzte Auftrag angelegt oder geändert hat, ist mit «neu» bzw. «geändert» markiert, gelöschte Dateien stehen durchgestrichen dabei. Die Suche oben findet Dateien im ganzen Projekt. Ein Klick auf eine Datei zeigt sie unten mit «Öffnen», «Vorschau» und «Im Chat erwähnen» (setzt den Pfad ins Eingabefeld); Rechtsklick öffnet dasselbe Menü mit «Im Explorer zeigen». Dateien aus dem Explorer von Windows, die man in diesen Bereich zieht, kopiert die App in den offenen Ordner (nie über eine vorhandene Datei); in den Chat gezogen werden sie Anhänge. Der Reiter «Vorschau» zeigt Text, Bilder und PDF; Office-Dateien zeigt sie als PDF, wenn Office installiert ist (die App wandelt sie beim Anklicken um und nimmt ein noch aktuelles PDF aus `.ask/pdf` wieder); ohne Office lassen sie sich öffnen oder im Explorer zeigen, ⤢ vergrössert. Doppelklick öffnet Dokumente, Bilder und Text im zugehörigen Programm; Programme, Skripte und unbekannte Dateiarten (`.bat`, `.exe`, `.lnk` …) öffnet sie nur nach einer Rückfrage, denn sie liefen ausserhalb der Sperre (ENTSCHEIDE E33).

**Internet.** Das Modell sucht selbst im Internet, so oft es nötig ist (ENTSCHEIDE E26): `web_search` über die Websuche von ASK (dieselbe Suchmaschine wie auf der Webseite; ist sie nicht freigegeben oder scheitert sie, über DuckDuckGo) mit bis zu 8 Treffern, `web_fetch` holt eine Seite als lesbaren Text ohne HTML (12’000 Zeichen pro Aufruf, PDF wird gespeichert und mit `read_file` gelesen). Adressen im lokalen Netz und auf dem Laptop sind gesperrt. Die gelesenen Seiten stehen als Quellen unter der Antwort. Der Schalter «Web» lässt ASK zusätzlich vor der ersten Antwort suchen.

**Frühere Chats.** Der Schalter in den Einstellungen (oder `/chats`) erlaubt dem Modell, frühere Chats im selben Ordner zu nutzen (ENTSCHEIDE E28): Es bekommt eine Liste der letzten 15 Chats (Datum, Titel) und die Werkzeuge `chat_search` (Suche über Eingaben, Antworten und Werkzeugaufrufe) und `chat_read` (einen Chat als Text lesen). Ausgeschaltet beginnt jeder Chat ohne Wissen über andere. Chats anderer Ordner sieht es nie.

**Aufgabenliste.** Wie in Claude Code steht die Aufgabenliste des Modells über dem Eingabefeld, nicht mehr in der Seitenleiste: «Aufgaben ▬▬ 3/7» mit Fortschrittsbalken, Erledigtes abgehakt und durchgestrichen, die aktuelle Aufgabe hervorgehoben, bei langen Listen nur der Ausschnitt um die aktuelle. Während eines Auftrags ist sie offen und die Zeile «✻ …» nennt die aktuelle Aufgabe; danach schrumpft sie auf eine Zeile. Ctrl+T (oder ein Klick) klappt sie auf und zu. Im Chat steht nur «Aufgaben ⎿ 3 von 7 erledigt», die Liste dazu in den Details. Eine fertige Liste verschwindet beim nächsten Auftrag.

**Browser.** Die App hat einen eigenen Browser (ENTSCHEIDE E27, E29). Der Reiter «Browser» rechts oder das Werkzeug `browser` öffnet einen echten Browser-Bereich (⤢ ganzes Fenster, ↻ neu laden, × schliessen, Adressleiste mit Zurück und Vorwärts, Eingabe ohne Punkt sucht mit DuckDuckGo). Das Modell öffnet damit Webseiten und HTML-Dateien aus dem Arbeitsordner, arbeitet ohne Neuladen auf der offenen Seite weiter und klickt (nach CSS-Selektor, sichtbarem Text oder Position), tippt und scrollt. Zurück bekommt es Adresse, Titel, Konsole, Text (6000 Zeichen), Bedienelemente und ein Bildschirmfoto. Man sieht alles live und kann selbst klicken und tippen; das Modell erfährt, wie oft die Person eingegriffen hat. Ein Aufruf dauert höchstens 2 Minuten, Esc und ■ beenden ihn sofort. Ist die App minimiert, öffnet das Modell die Seite unsichtbar mit 1280 × 800 und zeigt im Bereich Bilder davon.

Anmelden macht die Person selbst im Browser-Bereich; das Modell gibt keine Passwörter ein. Die Anmeldungen bleiben gespeichert, getrennt von ASK und Chrome, und lassen sich mit «Anmeldungen löschen» entfernen. Das Modell soll vor allem nachfragen, was etwas abschickt, kauft, postet oder löscht, und Text auf Webseiten nicht als Anweisung behandeln. Das steht nur in seinen Anweisungen und ist keine technische Sperre, darum wichtige Konten dort besser nicht anmelden. Downloads sind blockiert, Adressen im lokalen Netz kann nur die Person öffnen, nicht das Modell (ausser den Servern, die die App gestartet hat, siehe «Server»).

**Server.** Entwicklungsserver wie `npm run dev`, `python -m http.server 8000` oder `node server.js` laufen weiter, ohne dass ein Auftrag auf ihr Ende wartet (ENTSCHEIDE E48). Das Modell startet sie mit `server_starten` (statt `run_command`), sieht sie mit `server_liste` und stoppt sie mit `server_stoppen`; es bekommt nach einigen Sekunden die id, die Adresse und die ersten Zeilen der Ausgabe und öffnet die Seite mit `browser`. Im Reiter «Browser» steht über der Seite der einklappbare Bereich «Server»: jeder Server mit Name, Adresse und Punkt (grün läuft, gelb startet, rot Fehler, grau gestoppt), «Öffnen» lädt die Adresse im Browser darunter, «Ausgabe» zeigt die letzten 200 Zeilen, «Stoppen» beendet den Server samt seinen Unterprozessen. Darunter startet die Person selbst einen Server mit einem Befehl. Die Adresse erkennt die App in der Ausgabe (`Local: http://localhost:5173/`, `Serving HTTP on 0.0.0.0 port 8000`, `listening on port 3000` und ähnliche); ein Server gilt als gestartet, sobald er auf dem Port antwortet. Gezeigt werden die Server des sichtbaren Projekts. Sie laufen beim Wechsel des Chats weiter und werden gestoppt, wenn das Projekt geschlossen wird (anderes Projekt öffnen, ohne dass dort noch ein Auftrag läuft) oder die App sich beendet. Höchstens 5 Server laufen pro Projekt.

**Hilfsagenten.** Braucht eine Teilfrage viel Suchen und Lesen, gibt das Modell sie mit `hilfsagent` an einen Hilfsagenten ab (ENTSCHEIDE E55). Er hat nur lesende Werkzeuge (Dateien auflisten, suchen, lesen, Websuche und Webseiten), sieht den Chat nicht und gibt nur sein Ergebnis zurück; so bleibt der Verlauf kurz. Mehrere Hilfsagenten im selben Schritt arbeiten gleichzeitig, höchstens drei. Im Chat steht jeder als Zeile mit seinem jeweils letzten Schritt, ein Klick zeigt alle Schritte und das Ergebnis. Das Modell dafür lässt sich in den Einstellungen wählen («Hilfsagenten», Standard: wie im Chat); ein schnelles Modell spart Zeit. Auch im Planmodus verfügbar.

**Bilder erstellen.** Bietet ASK dem Konto das Werkzeug «Flux Bild» an, erstellt und bearbeitet das Modell Bilder mit `bild_erstellen` (ENTSCHEIDE E31): «mach ein Bild von …» oder ein Bild anhängen und sagen, was sich ändern soll. Flux läuft auf dem ASK-Server; die App schickt dafür eine eigene Anfrage an ASK, wie die Webseite, und speichert das Ergebnis im Arbeitsordner, neue Bilder unter `bilder/`, ein bearbeitetes Bild ersetzt seine Vorlage (Rückgängig wie bei anderen Dateien). Das Bild steht gross im Chat, Klick öffnet die Vorschau. Format, Qualität, Stil, Anzahl (bis 4), Referenzfotos aus Wikipedia und das TF-Bern-Logo gibt das Modell an Flux weiter. Ohne Flux in ASK gibt es das Werkzeug nicht.

**Ansehen und testen.** Das Modell sieht, was es erstellt (ENTSCHEIDE E25). `view_file` zeigt ihm Bilder, PDF, Word, Excel und PowerPoint als Bild (Seiten bzw. Folien, Standard die ersten 3, höchstens 6; Office-Dateien wandelt die App dafür mit Office in PDF um). Die Bilder stehen als Vorschau in der Werkzeugzeile (Klick: gross) und liegen in `.ask/arbeit/<Chat>/ansicht/`. Versteht das Modell laut ASK keine Bilder, gibt es `view_file` nicht, und `browser` liefert nur Text.

**Arbeiten im Ordner.** Das Modell legt neue Dateien dort an, wo sie ins Projekt passen, und ändert vorhandene Dateien direkt, ohne Kopien wie `_V2` (ENTSCHEIDE E24). Vor jedem Auftrag sichert die App den Stand des Ordners. Unter einem Auftrag, der Dateien geändert hat, steht «Dateien: 2 neu, 1 geändert» mit den Namen und dem Knopf «Rückgängig» (beim letzten solchen Auftrag immer, bei früheren beim Darüberfahren); er setzt den Ordner nach einer Rückfrage auf den Stand vor dem Auftrag zurück, löscht neu angelegte Dateien und sagt es dem Modell. Bei einem früheren Auftrag gehen damit auch die Änderungen aller Aufträge danach zurück; die Rückfrage sagt das. Gesichert wird jede Dateifassung nur einmal; Dateien über 100 MB und `.git`, `node_modules` werden nicht gesichert. Die Sicherung liegt nicht im Arbeitsordner, sondern unter `%LOCALAPPDATA%\ASK-Desktop\projekte\<Kennung>\sicherung`, wo das Modell sie nicht ändern kann (ENTSCHEIDE E33).

**Projektnotizen.** `.ask/projekt.md` ist wie `CLAUDE.md` in Claude Code: Die App gibt die Datei jedem Chat im Ordner mit, und das Modell hält darin kurz fest, was spätere Chats wissen müssen (Aufbau, wichtige Dateien, Vorlagen, Wünsche). Im Menü des Projekts öffnet «Projektnotizen bearbeiten» sie im Editor.

**Gedächtnis.** ASK merkt sich über alle Chats hinweg, was auch später nützt (ENTSCHEIDE E50): über dich (wie du Antworten magst, was du schon kannst, deine Ziele) und über einzelne Projekte (Entscheide, Vorgaben). Nach jedem fertigen Auftrag fragt die App das Modell im Hintergrund einmal kurz, ob es etwas Dauerhaftes gab; gestoppte oder fehlgeschlagene Aufträge und sehr kurze Eingaben zählen nicht. Neues steht als Hinweis «Gemerkt: …» im Chat. Sagt man «merk dir …», merkt sich das Modell es mit dem Werkzeug `merken`; «vergiss …» löscht mit `vergessen`. Jeder Chat bekommt die Einträge über dich und die des eigenen Projekts im Systemprompt (neueste zuerst, höchstens 2’500 Zeichen). `/gedaechtnis` (oder Einstellungen, «Ansehen …») zeigt alles als **Netz** (in der Mitte «Du», darum die Projekte in ihrer Farbe, die Schlagworte und die Einträge; ziehen verschiebt, das Mausrad zoomt, ein Klick auf einen Eintrag zeigt Text, Herkunft, «Bearbeiten» und «Vergessen») oder als **Liste** mit Suche, «Neu» und «Alles vergessen». Das Gedächtnis liegt nur auf diesem Laptop in den Daten der App (`%APPDATA%\ASK\gedaechtnis.json`), nie im Projektordner. Ausschalten in den Einstellungen: Dann merkt sich ASK nichts mehr und schickt nichts davon an das Modell; die Einträge bleiben, bis man sie löscht.

**Eigene Skills.** Ein Skill ist eine eigene Anleitung für wiederkehrende Aufträge, wie die Skills in Claude (ENTSCHEIDE E51): ein Name, eine Beschreibung, wann er gilt, und die Anleitung in Markdown. `/skills` (oder Einstellungen, «Skills …») listet sie; dort legt man sie an, ändert und löscht sie. Im Eingabefeld erscheinen sie wie Befehle: `/wochenbericht KW 47` schickt «/wochenbericht KW 47» und gibt dem Modell für diesen Auftrag die Anleitung dazu (im Chat steht nur, was man getippt hat). Ohne Aufruf sieht das Modell Name und Beschreibung aller Skills und liest einen passenden mit `skill_lesen`. Auf Wunsch legt es selbst einen an (`skill_erstellen`, mit Rückfrage vor dem Speichern). Teilen geht über eine Datei: «Exportieren …» speichert `name.md`, «Importieren …» liest eine solche Datei (höchstens 100 KB, mit Kopf `name` und `description`). Skills liegen in `%APPDATA%\ASK\skills\<name>\SKILL.md`. Sie sind etwas anderes als die mitgelieferten Anleitungen für Word, Excel, PowerPoint und PDF in `.ask/skills/` (E3).

Im Arbeitsordner legt die App nur `.ask/` an. Dort liegen die Anleitungen (`skills/`), die Aufgabenliste, die Chats (`chats/`), die Projektnotizen, die Hilfsskripte des Modells (`skripte/<Chat>/`), seine Zwischendateien (`arbeit/<Chat>/`) und die Anhänge (`anhaenge/<Chat>/`), damit der Arbeitsordner aufgeräumt bleibt; diese drei verschwinden mit dem Chat und werden nach 14 Tagen automatisch gelöscht. Die Sicherung für «Rückgängig» und die Verbindung mit GitHub liegen ausserhalb des Ordners (siehe oben); die Sicherung wird ebenfalls nach 14 Tagen gelöscht. `.ask` ist versteckt (Windows-Attribut wie bei `.git`) und erscheint im Explorer nur mit «Ausgeblendete Elemente», umgewandelte PDFs (`pdf/`) und Aufträge an die App.

Scrollt man im Chat nach oben, bleibt die Ansicht dort, auch während das Modell arbeitet; «↓ nach unten» oder Scrollen bis ans Ende lässt sie dem Text wieder folgen. Projekte, deren Ordner es nicht mehr gibt, sind durchgestrichen mit «Ordner nicht gefunden»; im Menü des Projekts lassen sie sich aus der Liste entfernen.

Die Ränder links und rechts vom Chat lassen sich mit der Maus ziehen; ein Doppelklick setzt die Standardbreite zurück, mit Tab angewählt verschieben ihn die Pfeiltasten. Der Chat bleibt mindestens 380 Pixel breit. Die Breiten merkt sich die App auf diesem Rechner.

Tastenkürzel: Ctrl+Plus/Minus/0 zoomt die Oberfläche, Ctrl+R lädt die Anmeldeseite von ASK neu.

## Bekannte Grenzen

- **Lesen ausserhalb des Ordners.** Windows verhindert Schreiben und Löschen ausserhalb des Arbeitsordners (Low Integrity Level). Genauer: ausserhalb aller Ordner, die je Arbeitsordner waren. Das Modell kann also aus Projekt B über die Shell auch in Projekt A schreiben, und dort gibt es dafür keine Sicherung. Die Kennzeichnung wird entfernt, wenn ein Projekt aus der Liste genommen wird (mit `icacls … /T`, auch bei den Dateien darin); bei verschachtelten Projekten (ein Ordner liegt im anderen) bleibt sie, damit andere Projekte nicht verändert werden. Das Entfernen ist unter Windows noch nicht getestet. Lesen kann das Modell über die Shell aber alle Dateien, die der Nutzer lesen darf, und es hat Internetzugang. Die Datei-API des Terminal-Servers ist zusätzlich auf den Ordner gesperrt. Siehe MACHBARKEIT, Abschnitt 5.
- **Originale im Ordner.** Das Modell ändert Dateien direkt; Windows schützt sie nicht (ENTSCHEIDE E14). Schutz bieten die Rückfrage vor Löschen und Überschreiben (E41, erkennt nur, was im Befehl selbst steht) und die Sicherung mit «Rückgängig» (E24). Sie gilt 14 Tage, nur für den letzten Auftrag mit Änderungen und nicht für Dateien über 100 MB, `.git` und Ordner mit mehr als 5000 Dateien. Fürs Rückgängigmachen müssen betroffene Dateien in Word, Excel oder PowerPoint geschlossen sein.
- **Werkzeuge unter Low.** Windows PowerShell 5.1 und die Unix-Werkzeuge von Git for Windows starten im Terminal nicht. Das Modell arbeitet mit `cmd.exe` und Python. Office lässt sich nur über die App umwandeln, nicht direkt steuern.
- **Ordner auf FAT/exFAT-USB-Sticks** oder Netzlaufwerken werden abgelehnt, weil sie sich nicht schützen lassen.
- **Abhängigkeit von ASK.** Die App braucht nur, was auch die ASK-Webseite braucht: Anmeldung und Zugriff auf ein Modell. Ob ASK die Werkzeugliste (`tools`) an das Modell weitergibt, ist mit dem echten ASK noch nicht geprüft. Lehnt ASK sie ab oder schreibt das Modell Aufrufe als `<tool_call>` in den Text, wechselt die App von selbst in einen Textmodus und beschreibt die Werkzeuge im Systemprompt.
- **Schalter «Web» sucht einmal pro Auftrag.** Mit dem Schalter sucht ASK vor der ersten Antwort. Während der Arbeit sucht das Modell selbst mit `web_search` (siehe «Internet»).
- **Werkzeuge und Wissen aus ASK** (die Werkzeuge und Wissenssammlungen, die einem Modell in ASK zugeordnet sind) stehen in der App nicht zur Verfügung, ausser Flux für Bilder.
- **Flux nicht mit dem echten ASK geprüft.** Beim Bau war Flux nur im Beta-Arbeitsbereich verfügbar. Wie ASK das Bild über die Schnittstelle zurückgibt, ist deshalb nicht geprüft; die App sucht an allen üblichen Stellen (E31). Kommt kein Bild, sagt die App, was ASK stattdessen geantwortet hat, und legt die Antwort unter `.ask/arbeit/<Chat>/flux-….json` ab. Während der Anfrage steht kurz ein Chat «ASK Desktop: Bild» im ASK-Verlauf.
- **GitHub ohne Git.** Der Abgleich läuft über die Schnittstelle von GitHub, nicht mit Git (ENTSCHEIDE E47): kein Zusammenführen innerhalb einer Datei oder zweier Branches (das geht über einen Pull Request auf GitHub), keine Git-Historie im Ordner. Die Liste der Branches zeigt höchstens 100, die der Pull Requests die 30 neuesten offenen. Ist ein Ordner schon ein Git-Repository, bleibt `.git` unberührt. Mit dem echten GitHub ist der Abgleich noch nicht getestet, nur mit einem Nachbau der Schnittstelle.
- **Kontextanzeige.** Nennt ASK weder Tokenzahlen noch die Grösse des Kontextfensters, ist die Anzeige eine Schätzung aus der Textlänge ohne Prozentangabe.
- **Chats nicht in ASK.** Die Chats liegen nur im Arbeitsordner (`.ask/chats`), nicht im Verlauf von ASK.
- **Nutzung von ASK nicht geprüft.** Die Nutzungsübersicht zählt sicher nur, was in den Projektordnern liegt; gelöschte Chats und Chats auf anderen Laptops fehlen. Den Abschnitt «ASK-Webseite» füllt die App aus Schnittstellen von Open WebUI (eigene Chatliste, allenfalls Nutzungszahlen), die mit dem echten ASK nicht getestet sind; liefert ASK nichts, fehlt der Abschnitt.
- **Ausgabe von Windows-Programmen.** Programme wie `ping` schreiben in der OEM-Codepage; das Terminal wandelt solche Zeilen um. Gemischte Zeilen können einzelne falsche Zeichen enthalten.
- **Office-Umwandlung** braucht ein installiertes Microsoft Office. Office bekommt immer eine Kopie im Temp-Ordner, nie die Datei im Arbeitsordner; aus der Kopie wird ein Verweis auf eine externe Word-Vorlage entfernt, Inhaltsverzeichnisse werden vor dem Export aktualisiert, und bei einem Verbindungsabbruch versucht die App es einmal neu. Scheitert die Umwandlung, meldet `pruefen.py --pdf` Exitcode 2 und sagt ausdrücklich, dass Seitenzahl und Inhaltsverzeichnis nicht geprüft sind. Ein Office-Fenster, das die Person gerade offen hat, wird nicht geschlossen; eine hängende unsichtbare Office-Instanz wird nach 2 Minuten beendet.
- **Grösse.** Ohne Chromium rund 85 MB kleiner als die Electron-Fassung; den grössten Teil macht Python mit pandas und matplotlib aus (rund 290 MB entpackt). Genaue Zahlen nach dem ersten Build mit Tauri im Testprotokoll.
- **Tauri-Feature «unstable».** Mehrere Webansichten in einem Fenster gibt es in Tauri 2 nur mit diesem Feature (ENTSCHEIDE E60). Vor einem Update von Tauri die Ansichten (ASK-Anmeldung, Browser) neu prüfen.
- **End-to-End-Tests.** Die automatischen Tests in `tests/e2e` steuerten Electron und laufen mit der Tauri-Fassung nicht mehr; bis sie umgestellt sind, gilt die Prüfliste im Testprotokoll von Hand.
- **Einzelne EXE beim ersten Start.** Der erste Start entpackt rund 610 MB, im Test 36 s bis zum laufenden Terminal, spätere Starts 3 s. Der entpackte Stand bleibt liegen wie eine Installation; entfernen lässt er sich nur durch Löschen von `%LOCALAPPDATA%\ASK-Desktop\app`.
- **Interaktive Notebooks**, WebSocket-Terminals und die Ports-API von open-terminal gibt es im Rust-Server nicht (Notebooks umgingen die Ordnersperre).
- **Server.** Die Adresse eines Servers kennt die App nur aus seiner Ausgabe. Meldet ein Server seine Adresse nicht in einer der üblichen Formen, steht er ohne Adresse in der Liste, und das Modell kann ihn nicht im Browser öffnen (die Person über die Adressleiste schon). Welcher Prozess auf dem Port lauscht, prüft die App nicht; sie gibt nur Ports frei, die ein noch laufender, von ihr gestarteter Server genannt hat (ENTSCHEIDE E48). Server eines Projekts, das nicht sichtbar ist, laufen nicht weiter.

## Offene Punkte

Grundsatzfragen, die nur die Schule bzw. du entscheiden können:

1. **ASK-Administration informieren.** Die App braucht keine Freigabe für Tool-Server mehr, weil sie die Werkzeuge selbst ausführt (E19). Gerade deshalb sollte die ASK-Administration wissen, dass die App das Modell über die Schnittstelle anspricht und auf den Laptops Befehle ausführt. Wenn die Schule das nicht will, ist eine Absprache nötig, bevor die App verteilt wird.
2. **Code-Signatur und Geräteverwaltung.** Mit der Informatik klären, ob per Nutzer installierte Programme unter `AppData` auf den Schullaptops laufen dürfen (AppLocker, Intune) und ob ein Zertifikat für die Signatur verfügbar ist.
3. **Datenschutz.** Inhalte aus dem Arbeitsordner gehen an ASK und das Modell. Ob dafür Regeln für die Lernenden nötig sind, ist eine Schulfrage.
4. **TF-Bern-Vorlagen** für Word, PowerPoint und Excel als `.docx`, `.pptx`, `.xlsx` in `resources/skills/vorlagen/` ablegen.
5. **Ablage des Repos.** Das Repo liegt in OneDrive. Mit `scripts/setup.ps1` bleibt OneDrive frei von Build-Dateien, einfacher wäre ein Repo ausserhalb von OneDrive mit einem Git-Server als Sicherung.

Technische Weiterentwicklungen:

- Mit dem echten ASK und Qwen testen: Werkzeugaufrufe über die Schnittstelle, Qualität der Arbeitsschritte, Anweisungen und Skills nachschärfen (`resources/anweisungen/systemprompt.md` ist bewusst kurz).
- Stärkere Isolation mit AppContainer, damit auch Lesen auf den Ordner beschränkt ist.
- `pip install` in einer virtuellen Umgebung (`.venv`): `PIP_USER` ist entfernt; pip weicht bei nicht beschreibbarem `site-packages` selbst auf die Benutzerinstallation aus. Unter Windows (Low Integrity) noch nicht getestet.
- Die Bildmarke der Technischen Fachschule Bern steht ohne Wortmarke im Kopf der App. Das CD Manual (1.1.3) verbietet das; mit der Schule klären oder durch das ganze Logo ersetzen.
