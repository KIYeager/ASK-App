# Entscheide

Entscheide, die ohne Rückfrage getroffen wurden, mit Begründung. Neueste unten.

## E1 Build-Ordner ausserhalb von OneDrive

Das Repo liegt in OneDrive. `node_modules`, das gebaute Python und der Installer umfassen Zehntausende Dateien und mehrere hundert MB, die sonst auf SharePoint synchronisiert würden. Alles Gebaute liegt deshalb in einem lokalen Build-Ordner (`ASK_BUILD_DIR`, Standard `%LOCALAPPDATA%\ASK-Desktop-Build`): Python unter `python`, der Installer unter `dist`. `scripts/setup.ps1` installiert die npm-Pakete mit `npm ci` in `deps` und legt im Projekt nur eine Junction `node_modules` an. OneDrive folgt Junctions nicht.

Korrektur während der Arbeit: Zuerst lag die Junction im Projekt und `npm install` lief im Projektordner. npm hat die Junction dabei durch einen echten Ordner ersetzt, sodass rund 450 MB `node_modules` etwa anderthalb Stunden im OneDrive-Ordner lagen, bis es auffiel. Der Ordner ist entfernt; darum gilt jetzt: `npm install` nie direkt im Projektordner ausführen, sondern `scripts/setup.ps1`. Besser wäre es, das Repo ganz ausserhalb von OneDrive abzulegen (siehe README, offene Punkte).

## E2 Anweisungen über den Systemprompt von open-terminal

*Abgelöst durch E19: Die App schickt den Systemprompt selbst mit jeder Anfrage.*

Siehe MACHBARKEIT, Abschnitt 2. `/system` wirkt ab Open WebUI 0.8.11 und steht in der Systemnachricht; `AGENTS.md` wirkt für persönliche Terminals unter Windows erst ab 0.12.0. Die App schreibt deshalb keine `AGENTS.md` in den Arbeitsordner. Eine vorhandene `AGENTS.md` der Lernenden bleibt unangetastet und wirkt zusätzlich.

## E3 Skills als Kopie in `.ask/skills/`

Die Anleitungen werden beim Öffnen eines Ordners nach `<ordner>/.ask/skills/` kopiert (nur wenn sich die Version geändert hat). Das Modell findet sie so mit kurzen relativen Pfaden, und neuere ASK-Versionen sehen sie über `/skills` als echte Skills. `.ask/` ist ohnehin der Ordner der App (`tasks.json`).

## E4 Echter API-Key nur im Hauptprozess, Platzhalter in ASK

*Teilweise abgelöst durch E19: Der Key bleibt im Hauptprozess, in ASK steht gar nichts mehr.*

In den ASK-Einstellungen steht als Key nur `ask-desktop`. Der echte Key wird bei jedem Start neu erzeugt, über eine Pipe an das Startskript gegeben und von Electron in die Anfragen der ASK-Ansicht eingesetzt. Er liegt damit weder auf der Festplatte noch in ASK, in der Prozessliste oder in der Umgebung der Befehle. Ein Browser ausserhalb der App kennt den Key nicht und kommt nicht an das Terminal.

## E5 Fester Port pro Nutzer

«Freier Port» wird so umgesetzt: Beim ersten Start wählt die App zufällig einen freien Port zwischen 20000 und 39999 und merkt ihn sich. Ist er später belegt, nimmt sie einen neuen und aktualisiert die Verbindung in ASK. So bleibt die Verbindung in ASK meist unverändert, was vor allem beim manuellen Einrichten wichtig ist.

## E6 Low Integrity Level ist Pflicht

Kann der gewählte Ordner nicht mit Low gekennzeichnet werden (z. B. FAT-USB-Stick, fremder Besitzer), lehnt die App ihn ab, statt ohne Schreibschutz zu laufen. Für Lernende ist ein Ordner unter dem eigenen Benutzerprofil der Normalfall; dort klappt es.

## E7 Pipe-Runner statt ConPTY für Modellbefehle

Die ConPTY-Ausgabe enthält Steuersequenzen und hat im Test Ausgaben verschluckt. Für `POST /execute` nutzt die App den Pipe-Runner von open-terminal mit UTF-8. Das interaktive Terminal in ASK bleibt bei ConPTY, weil es dort um Bedienung durch Menschen geht.

## E8 Keine Umgehung fehlender ASK-Freigaben

*Gegenstandslos seit E19: Die App trägt nichts mehr in ASK ein. Siehe dort zur Absprache mit der ASK-Administration.*

Das Backend von Open WebUI würde `terminalServers` auch ohne Berechtigung speichern. Die App schreibt die Verbindung nur, wenn ASK die Berechtigung «Direkte Tool-Server» und «Direkte Integrationen» meldet (Admins haben immer Zugriff). Sonst erklärt sie, was fehlt.

## E9 Electron statt Tauri

Tauri nutzt unter Windows WebView2. Damit lassen sich Header von WebSocket-Verbindungen nicht verändern, Berechtigungsabfragen wie Local Network Access nicht pro Origin beantworten und mehrere Webansichten nur umständlich in einem Fenster anordnen. Electron bringt mit `session.webRequest`, `WebContentsView` und eigenen Berechtigungs-Handlern alles mit, was für die Key-Einsetzung und die Seitenleiste nötig ist, und liefert überall dieselbe Chromium-Version. Der Nachteil ist die Grösse (rund 100 MB mehr), was bei einer Schul-App vertretbar ist. Version: Electron 44.6.0 (stabil, zum Zeitpunkt der Wahl gut zwei Wochen alt).

## E10 Python aus dem offiziellen NuGet-Paket

Mitgeliefert wird Python 3.14.8 aus dem NuGet-Paket «python» der Python Software Foundation (SHA512 gegen den NuGet-Katalog geprüft). Gründe: normale Ordnerstruktur ohne `._pth`-Datei, darum findet `python skript.py` Module im selben Ordner wie gewohnt; pip ist dabei; die Programmdateien sind von der PSF signiert, was bei verwalteten Schulgeräten (Defender, AppLocker) eher durchgeht als unsignierte Builds. Verworfen: das «embeddable»-Paket (isolierter Suchpfad, Skriptordner fehlt in `sys.path`, kein pip), python-build-standalone (unsigniert), PyInstaller (liefert keinen Interpreter, mit dem das Modell eigene Skripte ausführen kann). 3.14 statt 3.15, weil 3.15 erst seit Oktober 2026 draussen ist und noch nicht alle Pakete fertige Wheels haben könnten.

## E11 pandas und matplotlib zusätzlich

Zusätzlich zu python-docx, openpyxl, python-pptx, pypdf und reportlab sind pandas und matplotlib dabei. Modelle greifen für CSV-Auswertungen fast immer zu pandas und für Diagramme zu matplotlib; fehlen sie, scheitert der erste Versuch. Entfernt werden dafür Testsuiten von pandas, numpy und matplotlib, die Typ-Stubs von jedi (Ursache des Pfadlängenproblems) und debugpy. Grösse des Python-Ordners: rund 290 MB.

## E12 Notebooks abgeschaltet

Die Notebook-Funktion von open-terminal öffnet Dateien direkt über `os.path.abspath` und umgeht damit die Ordnersperre. Sie ist für den Arbeitsablauf nicht nötig und bleibt aus (`OPEN_TERMINAL_ENABLE_NOTEBOOKS=false`).

## E13 Weitere Sperren: AppData und alle Cloud-Sync-Ordner

Neben den im Auftrag genannten Ordnern sind auch `AppData` (Einstellungen anderer Programme) und Netzwerkpfade gesperrt. OneDrive- und SharePoint-Ordner werden zusätzlich zu den Umgebungsvariablen und dem Namen über die Registry erkannt (`HKCU\Software\SyncEngines\Providers\OneDrive\*\MountPoint` und die `Tenants`-Einträge). So fallen auch synchronisierte SharePoint-Bibliotheken unter `%USERPROFILE%\<Organisation>` darunter, deren Pfad «OneDrive» nicht enthält. Gesperrt ist auch ein Ordner, der einen solchen Ordner enthält.

## E14 Schutz der Originale nur über die Anweisung

Die Low-Kennzeichnung vererbt sich auf bereits vorhandene Dateien im Arbeitsordner (getestet). Das Modell kann sie also ändern. Ein zusätzlicher Schutz einzelner Dateien wäre möglich, würde aber nur für Dateien gelten, die vor dem Öffnen im Ordner lagen, und wirkt dadurch unberechenbar. Darum gilt: Windows schützt alles ausserhalb des Ordners, innerhalb gilt die Anweisung «nicht überschreiben, nichts löschen ohne Nachfrage», und der Ersthinweis verlangt das Arbeiten mit Kopien.

## E15 Office-Umwandlung über die App

Word, Excel und PowerPoint lassen sich aus einem Low-Prozess nicht per COM steuern (getestet, ebenso startet Windows PowerShell 5.1 unter Low nicht). Damit das Modell Dokumente trotzdem «nach PDF wandeln und kontrollieren» kann, legt das Prüfskript einen Auftrag in `.ask/` ab; die App selbst (normale Stufe) wandelt die Datei mit dem installierten Office in PDF um. Sie akzeptiert nur `.docx`, `.xlsx` und `.pptx` aus dem Arbeitsordner, öffnet sie schreibgeschützt mit deaktivierten Makros und schreibt nur nach `.ask/pdf/`.

## E16 ASK erst laden, wenn das Terminal läuft

*Gegenstandslos seit E19: Die ASK-Seite dient nur noch der Anmeldung.*

Open WebUI holt Werkzeuge (`/openapi.json`) und Systemprompt (`/system`) eines persönlichen Terminals nur beim Laden der Seite. Lädt ASK schneller als das Terminal startet, fehlen dem Modell die Werkzeuge bis zum nächsten Neuladen. Die App lädt ASK deshalb erst, wenn das Terminal läuft (höchstens 20 s Wartezeit, bei einem Fehler sofort). Nach einem Ordnerwechsel lädt sie ASK neu, weil der Ordnerpfad im Systemprompt steht. Ein Neustart des Terminals im selben Ordner braucht kein Neuladen: Port und Platzhalter-Key bleiben gleich.

## E17 Installer pro Nutzer mit einem Klick

NSIS mit `oneClick`, `perMachine: false` und `allowElevation: false`: Installation nach `%LOCALAPPDATA%\Programs\ask-desktop` ohne Admin-Abfrage, Verknüpfungen in Startmenü und auf dem Desktop, Deinstallation über «Apps» in den Windows-Einstellungen. Einstellungen in `%APPDATA%\ASK` bleiben bei der Deinstallation erhalten. Mitgeliefert werden nur die Chromium-Sprachdateien Deutsch und Englisch.

## E18 Einzelne EXE mit Zwischenspeicher zusätzlich

Neben dem Installer baut `npm run dist:portable` eine einzelne `ASK-<Version>.exe`. Das Target `portable` von electron-builder wurde verworfen: Es entpackt bei jedem Start rund 650 MB in den Temp-Ordner, erst nach `7z-out`, dann als Kopie (Start im Test 124 s), und `useZip` scheitert an der festen 4-Minuten-Grenze für makensis. Stattdessen packt `scripts/build-portable.js` den Ordner `win-unpacked` mit einem eigenen Starter (`scripts/starter.nsi`, zlib, ohne Prüfsumme) und ruft makensis ohne Zeitgrenze auf. Der Starter entpackt beim ersten Start mit Startbild nach `%LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build>`, über einen Ordner `.neu`, der erst nach vollständigem Entpacken umbenannt wird. Spätere Starts rufen `ASK.exe` direkt auf. Ein Mutex verhindert, dass zwei Doppelklicke gleichzeitig entpacken. Die App löscht beim Start mit der Einzelinstanz-Sperre ältere Stände und `.neu`-Ordner, die älter als eine Stunde sind (`src/main/portable.js`, mit `original-fs`, weil das fs von Electron `app.asar` als Ordner behandelt).

## E19 Eigene Chat-Oberfläche statt eingebetteter ASK-Seite

Die App zeigt ASK nicht mehr als Webseite mit eingetragenem Terminal, sondern hat einen eigenen Chat im Stil eines Terminal-Agenten. Die ASK-Seite dient nur noch der Anmeldung (auch über das Microsoft-Konto); danach liest die App `localStorage.token` aus der Seite und schickt die Aufträge mit diesem Token an `/api/chat/completions` (gestreamt, mit `tools`). Die Werkzeuge (`run_command`, `command_status`, `read_file`, `write_file`, `edit_file`, `list_files`, `find_files`, `search_text`, `update_tasks`) führt der Hauptprozess über open-terminal aus, mit dem echten Key; Low Integrity, Ordnersperre und Job-Objekt bleiben wie bisher (E6, E13). Lange Befehle liefern ihre Ausgabe laufend, Stoppen beendet den Prozessbaum.

Gründe: eine Oberfläche, die man selbst gestalten kann; Werkzeugaufrufe sichtbar und verständlich; keine Abhängigkeit von den ASK-Freigaben «Direkte Integrationen» und «Direkte Tool-Server», vom Laden der Werkzeuge beim Seitenaufbau (E16) und von der Open-WebUI-Version (ab 0.8.11). Folgen: Chats liegen im Arbeitsordner (`.ask/chats`), nicht in ASK; Funktionen der ASK-Webseite wie Datei-Upload oder Wissensdatenbanken gibt es in der App nicht.

Weil ASK die Werkzeugliste nicht unbedingt an das Modell weitergibt, hat die App einen Textmodus: Lehnt ASK `tools` mit HTTP 400 ab oder schreibt das Modell `<tool_call>{…}</tool_call>` in den Text (Format von Qwen), beschreibt die App die Werkzeuge im Systemprompt, wertet die Aufrufe aus dem Text aus und schickt Ergebnisse als `<tool_response>`. Ältere Werkzeugergebnisse gehen gekürzt an das Modell, damit lange Aufträge in das Kontextfenster passen.

Offen: Die ASK-Administration sollte wissen, dass die App das Modell über die Schnittstelle anspricht und Befehle auf den Laptops ausführt. Das Recht dazu hat jedes Konto, das ASK im Browser benutzen kann; ob die Schule diese Nutzung will, ist eine Absprache, keine technische Sperre.

## E20 Office wandelt nur Kopien um

Rückmeldung aus einem echten Auftrag: Eine Datei in `ausgabe/` liess sich nach einer PDF-Prüfung nicht mehr überschreiben, und die Umwandlung brach mit `RPC_E_DISCONNECTED` ab. Office öffnete bisher das Original; hängt oder stürzt Office dabei ab, kann die Datei gesperrt bleiben. Die App kopiert die Datei deshalb vor jeder Umwandlung in den Temp-Ordner, entfernt aus der Kopie einen Verweis auf eine externe Word-Vorlage (`attachedTemplate`, sonst sucht Word sie im Netz oder schickt Anmeldedaten an einen fremden Rechner), aktualisiert Inhaltsverzeichnisse und versucht es bei einem Verbindungsabbruch einmal neu. Zusätzlich lehnt `run_command` Programme mit Fenster ab; das Modell hatte Word selbst gestartet. Andere Verweise nach aussen (verknüpfte Felder, externe Bilder) werden weiterhin abgelehnt.

## E21 Weniger Schritte, Ziel-Modus

Ein Modell, das «jede Datei prüfen und jeden Fehler beheben» soll, schreibt sich bei Simulationen eigene Prüf-, Diagnose- und Kalibrierskripte und braucht Dutzende Schritte. Der Systemprompt verlangt deshalb gezieltes Prüfen (Office mit `pruefen.py`, Python durch Ausführen, HTML/JavaScript prüft die Person im Browser) und Dateien in einem Zug. In der Anzeige klappt ein fertiger Auftrag seine Schritte zu einer Zeile zusammen. Statt einer festen Schrittgrenze gibt es einen Schleifenschutz.

`/goal` setzt ein Ziel, das im Chat gespeichert wird. Das Modell bekommt es im Systemprompt und ein Werkzeug `goal_done`; hört es auf, ohne `goal_done` aufzurufen, schickt die App eine Aufforderung weiterzuarbeiten, höchstens zweimal ohne Arbeitsschritt dazwischen. So endet ein Auftrag nicht mehr nach einer blossen Ankündigung, und ein Modell, das auf eine Antwort wartet, läuft nicht endlos weiter.

## E22 GitHub über die Schnittstelle, ohne Git

Ein Projekt soll sich auf GitHub ablegen lassen, damit man es in einer anderen Sitzung oder mit einem anderen Programm weiterbearbeiten kann. Git selbst ist auf den Schullaptops nicht sicher vorhanden und läuft unter Low Integrity nur eingeschränkt (Unix-Werkzeuge von Git for Windows starten nicht). Die App gleicht deshalb über die REST-Schnittstelle von GitHub ab (Git Data API: Blobs, Tree, Commit, Ref), im Hauptprozess, nicht im Terminal. Das Modell bekommt fünf Werkzeuge: `github_status`, `github_clone`, `github_pull`, `github_push`, `github_create`.

- **Token.** Ein persönliches Token (Recht «repo»), das die Person einmal mit `/github` einträgt. Es wird mit Windows-Verschlüsselung (Electron `safeStorage`, DPAPI) in den Einstellungen gespeichert und verlässt den Hauptprozess nie: weder das Modell noch das Terminal sehen es. Sonst könnte das Modell über Befehle mit dem Token auf alle Repositories der Person zugreifen.
- **Bestätigung.** Hochladen und Anlegen fragt die Person in einem Dialog mit Repository, Nachricht und Dateiliste. Beim Hochladen kann sie mit einem Häkchen die Rückfrage für dieses Repository abschalten (gespeichert in den Einstellungen, im `/github`-Dialog zurücksetzbar); ein Hochladen lässt sich auf GitHub über die Historie zurücknehmen. Anlegen fragt immer, weil dabei privat oder öffentlich entschieden wird. Neue Repositories sind privat, ausser das Modell verlangt ausdrücklich öffentlich; dann steht «ÖFFENTLICHES» im Dialog. Holen braucht keine Bestätigung, weil es nie lokale Änderungen überschreibt.
- **Abgleich.** `.ask/github.json` merkt sich Repository, Branch, letzten Commit und den Blob-SHA jeder Datei beim letzten Abgleich. Damit lässt sich für jede Datei sagen, ob sie hier, auf GitHub oder an beiden Orten geändert wurde. Beidseitig geänderte Dateien bleiben unangetastet und werden als Konflikt gemeldet; die Person entscheidet. Ist GitHub neuer als der letzte Abgleich, lehnt `github_push` ab, bis geholt wurde. Es gibt kein Zusammenführen innerhalb einer Datei.
- **Ausgeschlossen.** `.ask`, `.git`, `node_modules`, `__pycache__` und `.venv` werden nie hochgeladen. Dateien über 50 MB und Ordner mit mehr als 3000 Dateien lehnt die App ab.

Nicht gewählt (GitHub): ein mitgeliefertes Git (rund 50 MB mehr, Anmeldung über den Git Credential Manager mit Fenster, Token wäre im Terminal) und das Token als Umgebungsvariable im Terminal (das Modell könnte es auslesen).

## E23 Anhänge als Dateien, Bilder zusätzlich direkt

Eingefügte oder hineingezogene Dateien speichert die App unter `.ask/anhaenge/<Chat>/`, bevor sie die Nachricht schickt. So arbeitet das Modell mit denselben Werkzeugen wie bei jeder anderen Datei (lesen, in ein Dokument übernehmen). Zuerst lagen sie sichtbar in `eingang/`; bei jedem eingefügten Bildschirmfoto wäre dieser Ordner gewachsen, und ein Anhang ist Material für einen Auftrag, kein Ergebnis. Darum liegen sie jetzt versteckt, verschwinden mit dem Chat und nach 14 Tagen (wie die Hilfsskripte); was das Modell daraus macht, speichert es im Arbeitsordner. Die Nachricht an das Modell nennt die Pfade. Vorhandene Dateien werden nie überschrieben, ein gleicher Name bekommt « (2)».

Qwen3.8 Flash Next versteht Bilder, und die ASK-Webseite schickt sie mit. Die App schickt Bilder deshalb zusätzlich als `image_url` (data-URL) in der Nachricht, wie Open WebUI. Damit der Verlauf nicht mit jedem Schritt wächst, gehen nur die Bilder der letzten zwei Eingaben mit Bildern mit; ältere stehen nur noch als Pfad im Text. Bilder über 2048 Pixel werden für das Modell verkleinert, die gespeicherte Datei bleibt im Original. Gibt ASK beim Modell `capabilities.vision: false` an oder lehnt es eine Anfrage mit Bild ab, wiederholt die App sie ohne Bilder und schickt künftig nur Pfade.

Nicht gewählt: Dateien über die Datei-Schnittstelle von Open WebUI hochladen. Die Inhalte gingen dann zusätzlich in die Wissensverarbeitung von ASK, und das Modell könnte die Datei im Ordner nicht bearbeiten.

## E24 Direkt im Ordner arbeiten, mit Sicherung und «Rückgängig»

Bisher legte das Modell Ergebnisse in `ausgabe/` ab und überschrieb Dateien der Person nie; Änderungen an einem Dokument ergaben eine Kopie in `ausgabe/`. Das schützte Originale, passte aber schlecht zu Projekten (eine Website von GitHub, ein Programm), in denen Dateien an ihrem Platz geändert werden sollen. Claude Code arbeitet direkt im Ordner und verlässt sich auf Rückfragen, Git und Checkpoints. Die ASK-App fragt nicht vor jeder Änderung, und Lernende haben kein Git; ihre Checkpoints erfassten zudem nur Änderungen über die eigenen Werkzeuge, Word- und Excel-Dateien ändert das Modell aber über Python-Skripte.

Deshalb arbeitet das Modell jetzt direkt im Ordner (neue Dateien dort, wo sie hingehören; vorhandene Dateien unter demselben Namen ändern), und die App sichert vor jedem Auftrag den ganzen Ordner (`snapshot.js`):

- Jede Dateifassung liegt einmal in `.ask/sicherung/objekte/` (Name = SHA-1 des Inhalts), ein Stand ist eine Liste Pfad → Prüfsumme. Dateien mit gleicher Grösse und Änderungszeit wie im letzten Stand werden nicht neu gelesen; ein Auftrag kostet so Platz nur für die Dateien, die sich seither geändert haben.
- Nach dem Auftrag vergleicht die App den Ordner mit dem Stand davor und zeigt «Dateien: 1 geändert · 2 neu» mit «Rückgängig». Rückgängig setzt geänderte und gelöschte Dateien zurück und löscht neu angelegte, nach einer Rückfrage mit der Liste der Dateien. Vorher sichert die App den jetzigen Stand, damit nichts unwiderruflich verloren geht. Das Modell bekommt einen Hinweis im Verlauf.
- Den Knopf gibt es nur beim letzten Auftrag mit Änderungen, weil das Zurücksetzen eines älteren Auftrags auch alle späteren Änderungen zurücksetzen würde.
- Nicht gesichert: `.ask`, `.git`, `node_modules`, `__pycache__`, `.venv`, Office-Sperrdateien (`~$…`), Dateien über 100 MB und Dateien, die gerade exklusiv geöffnet sind (Hinweis im Chat). Ordner mit mehr als 5000 Dateien sichert die App nicht. Stände älter als 14 Tage und nicht mehr gebrauchte Fassungen werden beim Öffnen des Ordners gelöscht.

Dazu kommen Projektnotizen in `.ask/projekt.md` (wie `CLAUDE.md`): Die App hängt sie an den Systemprompt jedes Chats im Ordner (höchstens 6000 Zeichen), das Modell pflegt sie, die Person kann sie über das Ordner-Menü bearbeiten. So weiss ein neuer Chat, wie das Projekt aufgebaut ist, ohne alles neu zu lesen.

Nicht gewählt: Sicherung nur der Dateien, die `write_file` und `edit_file` ändern (erfasst keine Skripte), und eine Kopie des ganzen Ordners pro Auftrag (zu viel Platz).

## E25 Das Modell sieht, was es erstellt

`pruefen.py` prüft Office-Dateien über ihren Inhalt, nicht über ihr Aussehen, und HTML konnte das Modell gar nicht ausführen. Qwen3.8 Flash Next versteht Bilder; die App gibt ihm deshalb zwei Werkzeuge (`visual.js`), die im Hauptprozess laufen, nicht im Terminal:

- `view_file`: PNG und JPEG verkleinert die App mit `nativeImage`, GIF, WebP, BMP und SVG zeichnet sie in einem unsichtbaren Fenster. Word, Excel und PowerPoint wandelt sie mit Office in PDF um (dieselbe abgesicherte Umwandlung wie für `pruefen.py`, E20), PDF-Seiten zeichnet pdf.js (3.11.174, mitgeliefert in `src/vendor/pdfjs`, ohne eigenen Worker) 1240 Pixel breit. Höchstens 6 Seiten pro Aufruf.
- `browser_test`: eine HTML-Datei im Arbeitsordner in einem unsichtbaren Offscreen-Fenster mit 1280 × 800 Pixeln, Sandbox, eigener flüchtiger Sitzung, ohne Popups, Downloads, Berechtigungen und Navigation weg von der Datei; Laden höchstens 15 s. Klicks (CSS-Selektor oder Position), Tasten und Text gehen als echte Eingabeereignisse an die Seite. Zurück kommen Fehler und Warnungen der Konsole, Titel, sichtbarer Text und das Bildschirmfoto. Netzwerk bleibt erlaubt, damit Seiten mit Bibliotheken von einem CDN laufen; ins Netz kommt das Modell ohnehin über Python.

Werkzeugergebnisse dürfen bei `/api/chat/completions` nur Text sein. Die Bilder gehen deshalb nach den Werkzeugergebnissen als eigene Nachricht («[Ansicht] Bilder zu …», im Chat nicht als Eingabe angezeigt) an das Modell, nur aus den letzten zwei Ansichten, damit der Verlauf nicht mit jedem Schritt wächst. Lehnt ASK eine Anfrage mit Bild ab, gilt wie bei Anhängen (E23): künftig nur Text. Der Systemprompt verlangt, Aussehen einmal zu prüfen und höchstens einmal nachzuprüfen, damit das Modell nicht endlos Bilder anfordert.

Nicht gewählt: Bildschirmfotos mit einem mitgelieferten Browser wie Playwright (mehrere hundert MB) und das Ansehen über das Terminal (Low Integrity, kein Office, kein Fenster).

## E26 Websuche als Werkzeug, Aufgabenliste über der Eingabe, Browser live

**Websuche.** Der Schalter «Web» lässt ASK nur einmal vor der ersten Antwort suchen; während der Arbeit behalf sich das Modell mit curl auf DuckDuckGo und bekam rohes HTML (viele Tokens, Fehler beim Herauslesen, manchmal blockiert). Jetzt gibt es zwei Werkzeuge im Hauptprozess (`web.js`): `web_search` ruft die Websuche von ASK auf (`/api/v1/retrieval/process/web/search`, mit `query` und `queries` für ältere und neuere Open-WebUI-Versionen; ein 403 meldet nicht ab, sondern schaltet auf DuckDuckGo um), `web_fetch` holt eine Seite über das Netzwerk von Chromium (Proxy von Windows) und macht daraus Text (ohne Skripte, Navigation, Kopf und Fuss; Überschriften, Listen und Tabellen bleiben erkennbar), höchstens 3 MB, 12’000 Zeichen pro Aufruf. Gesperrt sind andere Protokolle als http(s) und Adressen im lokalen Netz und auf dem Rechner, auch nach einer Weiterleitung, damit das Modell nicht über die App an Geräte im Schulnetz oder an das Terminal kommt. Die Seiten, die das Modell gelesen hat, erscheinen als Quellen unter der Antwort.

**Aufgabenliste.** Claude Code zeigt die Aufgaben über dem Eingabefeld, unter der Statuszeile des laufenden Auftrags, und wiederholt sie nicht im Verlauf. Die App macht es ebenso: Die Liste stammt aus dem letzten `update_tasks` des Chats (nicht mehr aus der Datei `.ask/tasks.json`), ist während des Auftrags offen und danach eine Zeile; Ctrl+T klappt sie auf. Die Seitenleiste hat keinen Aufgabenbereich mehr.

**Browser live.** `browser_test` lief unsichtbar, die Person sah nur das Bildschirmfoto am Ende. Das Offscreen-Fenster liefert ohnehin laufend Bilder; die App schickt davon etwa 8 pro Sekunde (JPEG, 640 Pixel breit) mit Schritten und Konsolenmeldungen an die Seitenleiste. Gezeigt wird genau das Fenster, das das Modell bedient (1280 × 800), nicht eine verkleinerte Kopie mit anderem Layout. Eingreifen kann die Person dort nicht, damit sie dem Modell nicht dazwischenklickt.

## E27 Echter Browser-Bereich statt Live-Bildern

Die Live-Bilder aus dem unsichtbaren Fenster (E26) ruckelten (8 pro Sekunde) und waren in der grossen Ansicht unscharf, und eingreifen konnte man nicht. Claude zeigt im Browser-Fenster die echte Seite und schickt nur dem Modell Bildschirmfotos. Die App macht es jetzt ebenso: `browser_test` lädt die Seite in eine WebContentsView im App-Fenster (eigene flüchtige Sitzung `ask-browser`, keine Popups, Downloads und Berechtigungen, Navigation nur innerhalb der Datei) über dem Bereich `#browser-host` rechts. Das Modell steuert sie mit denselben Eingabeereignissen, das Bildschirmfoto kommt mit `capturePage` aus dieser Ansicht (höchstens 1600 Pixel breit). Die Person kann direkt klicken und tippen; Eingaben, die nicht kurz nach einer eigenen Eingabe der App kommen, zählt die App als Eingriff der Person und meldet sie dem Modell. Die Seite bleibt nach dem Test offen.

Folgen: Die Seite ist so gross wie der Bereich (halbes Fenster oder ganzes Fenster) statt fest 1280 × 800; das Modell bekommt die Grösse im Ergebnis. Während eines Dialogs oder des Denk-Reglers wird die Ansicht ausgeblendet, weil sie über der Oberfläche liegt. Ist das Fenster minimiert oder unsichtbar, rendert Windows die Seite nicht; dann testet die App wie bisher im unsichtbaren Fenster mit 1280 × 800 und zeigt Bilder davon im Bereich (12 pro Sekunde, 960 Pixel breit).

## E28 Frühere Chats nur mit Schalter

Ein neuer Chat wusste nichts von früheren Chats im selben Ordner. Ein Schalter «Chats» (Einstellung `chatMemory`, Standard aus) gibt dem Modell eine Liste der letzten 15 Chats des Ordners im Systemprompt und die Werkzeuge `chat_search` und `chat_read` (`history.js`). Gelesen wird der Verlauf als Text: Eingaben, Antworten und Werkzeugaufrufe als Kurzzeile, ohne Werkzeugausgaben, Bilder und automatische Nachrichten, damit er kurz bleibt. Standard aus, weil alte Chats Dinge enthalten können, die nicht in den neuen Auftrag gehören, und weil jede Liste den Systemprompt verlängert; ausgeschaltet fehlen auch die Werkzeuge. Nur der aktuelle Ordner: Chats anderer Projekte bleiben getrennt. Dauerhaftes Wissen über ein Projekt gehört weiterhin in die Projektnotizen (E24).

## E29 Der Browser wird ein allgemeiner Browser

`browser_test` öffnete nur lokale HTML-Dateien. Das Modell schrieb deshalb für eine Webseite eine Hilfsdatei mit Weiterleitung; die Sperre gegen Navigation blockierte sie, die Seite blieb schwarz, und die App meldete «keine Fehler». Danach hing der Aufruf 24 Minuten, weil das Warten auf die Seite kein Zeitlimit hatte und der Stopp den Browser nicht erreichte.

Das Werkzeug heisst jetzt `browser` und öffnet Webseiten und HTML-Dateien (`ziel`); ohne `ziel` arbeitet es auf der offenen Seite weiter, sonst liesse sich nichts über mehrere Schritte bedienen. Dazu kommen Klick nach sichtbarem Text, Eingabe in ein bestimmtes Feld, Scrollen, die Liste der Bedienelemente, ein Knopf «Browser» und eine Adressleiste. Jeder Zugriff auf die Seite hat ein Zeitlimit von 10 s, der ganze Aufruf von 2 Minuten, und der Stopp bricht ihn sofort ab. Leere Seiten werden gemeldet.

Navigation: Webseiten sind frei, Dateien nur im Arbeitsordner. Das Modell kann keine Adressen im lokalen Netz öffnen; landet es über einen Link oder eine Weiterleitung dort, bekommt es weder Text noch Bild. Die Person kann solche Adressen selbst öffnen.

Anmeldung: Die Sitzung liegt in `persist:ask-browser`, getrennt von ASK und Chrome, damit das Modell mit einer Anmeldung weiterarbeiten kann. Das Modell gibt keine Passwörter ein. Popups sind als kleine Fenster erlaubt, weil Anmeldungen über Microsoft und Google sie brauchen; neue Tabs öffnen in derselben Ansicht. Downloads und Berechtigungen (Kamera, Standort usw.) bleiben gesperrt, ausser Vollbild.

Risiko: Das Modell handelt auf Webseiten mit der Anmeldung der Person, und Text auf einer Seite kann versuchen, ihm Anweisungen zu geben (Prompt Injection). Dass es vor dem Abschicken nachfragt und solchen Text nicht befolgt, steht nur im Systemprompt; das ist keine technische Sperre. Darum der Hinweis, wichtige Konten dort nicht anzumelden, und der Knopf «Anmeldungen löschen».

## E30 Alten Terminal-Eintrag aus ASK entfernen

Versionen vor E19 schrieben eine Verbindung «ASK Desktop» mit dem Platzhalter-Key `ask-desktop` in die ASK-Einstellungen (`ui.terminalServers`). Den echten Key setzte nur das Fenster der App ein. Seit E19 entfernte niemand den Eintrag, und auf der ASK-Webseite meldete Open Terminal `Invalid API key`, sobald die App auf dem gemerkten Port lief.

Die App entfernt den Eintrag deshalb einmal pro Anmeldung. Das ist die einzige Stelle, an der sie noch in die ASK-Einstellungen schreibt, und sie gilt nur für Einträge mit diesem Namen, diesem Key und einer Adresse auf `127.0.0.1`. Die Terminals der Person bleiben unberührt; auch die, welche die alten Versionen deaktiviert haben, schaltet die App nicht wieder ein, weil sich nicht mehr erkennen lässt, welche das waren. Ein Fehler beim Aufräumen wird nicht gemeldet und meldet niemanden ab; der nächste Start versucht es wieder. Die Auswahl im normalen Browser (`localStorage.selectedTerminalId`) erreicht die App nicht.

## E31 Bilder mit Flux aus ASK

ASK hat neu das Server-Werkzeug «Flux Bild» (`flux_bild`, Funktion `bild_erstellen`): Bilder erstellen und bearbeiten mit Format, Qualität, Stil, Anzahl, Wikipedia-Referenzfotos und TF-Bern-Logo. Die App führt ihre Werkzeuge selbst aus (E19); ein Server-Werkzeug kann sie nicht aufrufen, nur ASK bitten, es zu verwenden (`tool_ids`). Server- und App-Werkzeuge in derselben Anfrage zu mischen ist heikel: ASK würde auch die Aufrufe der App-Werkzeuge selbst ausführen wollen. Darum bekommt das Modell der App ein eigenes Werkzeug `bild_erstellen` mit denselben Angaben (dazu `vorlage` und `datei`), und die App schickt dafür eine eigene, kurze Anfrage an ASK: dasselbe Modell, nur mit Flux, Systemprompt «rufe bild_erstellen mit genau diesen Argumenten auf». Zum Bearbeiten geht die Vorlage als Bild in der Nachricht mit.

Wie das Bild zurückkommt, liess sich beim Bau nicht prüfen; Flux lief nur im Beta-Arbeitsbereich, und dort gab es Probleme. Soweit von älteren Open-WebUI-Versionen bekannt (ASK läuft auf 0.13), führt Open WebUI Werkzeuge mit nativen Werkzeugaufrufen nur aus, wenn die Anfrage zu einem Chat gehört (Chat, Nachricht, Sitzung), und Werkzeuge melden Bilder meist als Ereignis «files», das ASK in die Nachricht im Chat schreibt. Die App macht es darum wie die Webseite: Sie legt einen vorübergehenden Chat «ASK Desktop: Bild» an, schickt die Anfrage mit `chat_id`, `id` und `session_id`, liest die Antwort aus dem Chat (bis ASK keine laufende Aufgabe mehr meldet oder der Text steht) und löscht den Chat danach. Kommt die Antwort doch gestreamt, wertet sie den Strom aus; lässt sich kein Chat anlegen, geht es ohne. Gesucht wird an allen üblichen Stellen: Dateien vom Typ Bild, Links auf `/api/v1/files/…` und `/cache/image/…`, eingebettete Bilder (`data:image/…`), auch in HTML-kodierten Werkzeugergebnissen; Bild-Links auf andere Server nur, wenn sonst nichts da ist, Wikipedia-Bilder (Referenzfotos) nie. Das Token geht nur an ASK, nie an andere Server.

Gespeichert wird im Arbeitsordner wie bei anderen Dateien (E24): neue Bilder unter `bilder/` mit einem Namen aus der Beschreibung, nie über ein bestehendes; ein bearbeitetes Bild ersetzt seine Vorlage, ausser sie ist ein Anhang unter `.ask` (dann nach `bilder/`) oder das Dateiformat ändert sich (dann daneben). Das Modell sieht das Ergebnis nicht von selbst, nur mit `view_file`; die Person sieht es gross im Chat. Angeboten wird `bild_erstellen` nur, wenn ASK das Werkzeug in der Liste `/api/v1/tools/` für das Konto führt (geprüft bei jeder Anmeldung). Ein Aufruf dauert höchstens 5 Minuten; Stopp hält auch die Aufgabe in ASK an.

Kommt kein Bild, sagt die App, was ASK geantwortet hat, und legt die ganze Antwort (eingebettete Bilder gekürzt) unter `.ask/arbeit/<Chat>/flux-….json` ab. Damit lässt sich der Weg anpassen, sobald Flux mit dem echten ASK läuft. Getestet ist der Ablauf mit einem Nachbau von ASK, der beide Wege spielt (Hintergrund mit Datei im Chat, gestreamter Link).

## E32 Tastaturfokus bleibt im Chat

Schreibt die Person im Chat, während das Modell im Browser-Bereich arbeitet, sprang die Eingabe in den Browser. Zuerst rief die App beim Bedienen `focus()` auf der Seite auf; das ist seit dem 30.10.2026 weg. Es reichte aber nicht: In Electron holen schon das Laden einer Seite und jeder Mausklick, den die App an die Seite schickt, den Tastaturfokus in den Bereich, wenn das App-Fenster im Vordergrund ist. Der Test lief bis dahin mit dem Fenster im Hintergrund und bemerkte es nicht.

Die App gibt den Fokus jetzt nach dem Laden, nach jedem Klick (nach 30 ms) und nach jeder Aktion an die Oberfläche zurück, sofern der Bereich ihn hat. Ausnahme: Die Person klickt während des Aufrufs selbst in den Browser; dann bleibt er dort. Ob der Bereich schon vor dem Aufruf den Fokus hatte, zählt nicht, denn auch Einblenden und frühere Aufrufe holen ihn dorthin, ohne dass die Person dort ist. Tasten (`sendInputEvent`) und Text (`insertText`) erreichen die Seite auch ohne Fokus, auch in ein zuvor angeklicktes Feld.

## E33 Was das Modell schreiben kann, ist für die App nicht vertrauenswürdig

Das Terminal läuft mit niedriger Integritätsstufe, die App selbst mit den normalen Rechten der Person. Alles, was die App aus dem Arbeitsordner liest und danach mit ihren Rechten ausführt, ist darum ein Weg aus dem Schutz hinaus, sobald das Modell (zum Beispiel nach einer manipulierten Webseite oder Datei) es darauf anlegt. Eine Durchsicht am 25.11.2026 fand drei solche Wege; alle sind geschlossen:

- **Rückgängig.** Die Liste der gesicherten Dateien lag in `.ask/sicherung`. Ein Eintrag wie `..\..\AppData\…\Startup\x.bat` liess die App beim Zurücksetzen ausserhalb des Ordners schreiben. Sicherung und GitHub-Verbindung liegen jetzt unter `%LOCALAPPDATA%\ASK-Desktop\projekte\<Kennung>` (nicht im Laufzeitordner des Terminals, der ebenfalls beschreibbar ist); vorhandene Daten aus `.ask` übernimmt die App einmal. Zusätzlich prüft sie jeden Stand beim Lesen: nur relative Pfade ohne `..`, Laufwerk oder Datenstrom, nur echte Prüfsummen, und kein Schreiben über eine Verknüpfung aus dem Ordner hinaus (`src/main/safe.js`).
- **Doppelklick.** «Öffnen» startete jede Datei im zugehörigen Programm, auch eine vom Modell angelegte `Bericht.bat`. Ohne Rückfrage öffnet die App jetzt nur noch Dokumente, Bilder, Medien und Text; bei allem anderen fragt sie nach.
- **Office-Umwandlung.** Die Prüfung auf externe Verweise suchte mit Mustern im XML und liess sich mit einfachen Anführungszeichen, einem Ziel auf `/hyperlink` oder klein geschriebenen Feldnamen umgehen; Word hätte dann Inhalte von fremden Rechnern geholt und dabei Anmeldedaten mitgeschickt. Die Datei wird jetzt in der App gelesen (`src/main/ooxml.js`): Verweise nach ihrem Typ, Felder nach dem ersten Wort der Anweisung, auch wenn Word sie auf mehrere Stücke verteilt, in allen Teilen des Dokuments. Was sich nicht lesen lässt, gilt als abgelehnt. Die alte Prüfung im PowerShell-Skript bleibt als zweite Stufe.

Dazu gehört der Abgleich mit GitHub: Repository und Branch aus der gespeicherten Verbindung gehen in Adressen der GitHub-API mit dem Token der Person und werden darum geprüft; Pfade aus einem Repository, die aus dem Ordner hinausführen, legt die App nicht an. `.gitignore` gilt beim Hochladen (was schon auf GitHub liegt, bleibt dabei, wie bei git), und eine Datei, die hier und auf GitHub verschieden geändert wurde, ersetzt das Hochladen nur nach einer eigenen Rückfrage.

Nicht geschlossen: Die Kennzeichnung «Low» bleibt an jedem Ordner, der einmal Arbeitsordner war (README, Bekannte Grenzen).

## E34 Enter während eines Auftrags reiht ein

Bisher stoppte Enter einen laufenden Auftrag; wer eine Folgefrage tippte und Enter drückte, brach die Arbeit ab und schickte nichts. Jetzt reiht Enter eine Nachricht ein, die nach dem Auftrag von selbst hinausgeht. Gestoppt wird nur mit Esc oder ■; dann, nach einem Fehler und beim Wechsel des Chats steht die wartende Nachricht wieder im Eingabefeld, damit nichts ungewollt abgeschickt wird.

Ordnerwechsel während eines Auftrags lehnt die App ab: Der laufende Auftrag schrieb seinen Chat sonst in den neuen Ordner. Mehrere Starts des Terminals kurz hintereinander laufen nacheinander, und nur der letzte startet wirklich; eine einzelne verpasste Probe gilt nicht mehr als Ausfall.

## E35 Langer Verlauf

Die App weiss nicht, wie gross das Kontextfenster des Modells in ASK ist. Sie schickt darum ältere Arbeitsschritte von Anfang an gekürzt mit (Ergebnisse und lange Argumente, etwa den Inhalt einer geschriebenen Datei; die Datei liegt ja im Ordner) und kürzt stärker, sobald ASK meldet, dass der Verlauf zu lang ist. Reicht auch das nicht, sagt sie, dass ein neuer Chat nötig ist. Der gespeicherte Chat bleibt immer vollständig. Eine Zusammenfassung des Verlaufs durch das Modell gibt es nicht.
