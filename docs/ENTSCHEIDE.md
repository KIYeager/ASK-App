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

## E36 Eingabe ändern, Chat-Menü, Kontextanzeige, Einstellungen, Tastatur

- **Eingabe ändern oder neu senden** gibt es nur für die letzte Eingabe. Sie nimmt die Eingabe und alles danach aus dem Chat und setzt den Ordner nach Rückfrage mit derselben Sicherung zurück wie «Rückgängig» (E24); lehnt die Person das Zurücksetzen ab, bleibt auch der Chat, wie er ist. So kann der Chat nie etwas anderes behaupten, als im Ordner liegt.
- **Chat-Menü**: Umbenennen und Anheften stehen in der Chat-Datei selbst (`title`, `pinned`), damit sie mit dem Ordner mitgehen. Die Suche im Inhalt läuft über alle Ordner der Liste, mit derselben Suche, die das Modell mit `chat_search` benutzt (E28).
- **Kontextanzeige**: Die Grösse des Kontextfensters kennt die App nur, wenn ASK sie zum Modell angibt oder sie einmal in einer Fehlermeldung genannt hat (dann merkt sich die App die Zahl pro Modell). Sonst zeigt sie nur die Tokenzahl, ohne Prozent.
- **Aufgabenliste**: Sie gehörte in der Oberfläche schon zum Chat (aus dessen Arbeitsschritten). Die Datei `.ask/tasks.json` für den ganzen Ordner las niemand mehr; das Werkzeug schreibt sie nicht mehr.
- **Tastatur**: Was sich anklicken lässt, ist mit Tab erreichbar; Dialoge halten den Fokus und geben ihn beim Schliessen zurück. Die gedämpfte Schrift ist heller (Kontrast etwa 4.7:1 statt 3:1).

## E37 Updates über ein öffentliches Release-Repository

Neue Versionen kamen bisher nur als Datei, die jemand auf dem eigenen Windows-Rechner bauen (`setup.ps1`, `npm run dist`) und verteilen musste. Neu baut der Workflow `.github/workflows/release.yml` bei jedem Tag `v<Version>` auf `windows-latest` und veröffentlicht in einem eigenen, öffentlichen Repository `KIYeager/ASK-Releases`; die App aktualisiert sich mit electron-updater.

Warum ein zweites Repository: ASK-App ist privat. electron-updater könnte private Releases nur mit einem Token in der App lesen, und ein Token in jeder Kopie der App wäre öffentlich. Im öffentlichen Repository liegen nur die fertigen Dateien. Gefunden wird die neueste Version über `github.com/<repo>/releases.atom` und `releases/latest`, nicht über die API von GitHub; die API erlaubt ohne Token nur 60 Anfragen pro Stunde und Adresse, und im Schulnetz teilen sich viele Laptops eine Adresse.

Die installierte App fragt vor dem Herunterladen und vor dem Neustart (Wunsch: die Lernenden entscheiden), nie während eines Auftrags. «Beim Beenden» installiert beim nächsten Schliessen. Vor dem Neustart beendet die App das Terminal, damit keine Python-Prozesse Dateien im Programmordner sperren. Die einzelne EXE (E18) prüft mit derselben Quelle, zeigt aber nur einen Link: Sie müsste ihren eigenen Starter ersetzen, und dafür gibt es in electron-updater nichts.

Beta: Tags wie `v0.3.0-beta.1` werden Vorabversionen. Der Schalter «Beta-Versionen erhalten» setzt `allowPrerelease`. Jedes Release hat nur `latest.yml`; electron-updater sucht bei einer Vorabversion zuerst `beta.yml` und nimmt dann `latest.yml` desselben Releases. Wer den Schalter wieder ausschaltet, behält die Beta, bis eine grössere normale Version kommt (kein Zurückstufen).

Die Angabe des Repositorys steht einmal in `resources/app-config.json` (`updates`); electron-builder schreibt daraus `app-update.yml` in die installierte App, die einzelne EXE setzt sie zur Laufzeit. electron-builder lädt selbst nichts hoch (`--publish never`), das macht `gh release create` im Workflow mit dem Secret `RELEASES_TOKEN` (Fine-grained Token nur für ASK-Releases). Installer und EXE bleiben unsigniert; electron-updater prüft die Signatur nur, wenn `publisherName` gesetzt ist, und die Prüfsumme aus `latest.yml` (SHA-512) schützt vor beschädigten Downloads, nicht vor einem fremden Release im Repository.

## E38 Neue Version beim Merge, Beta oder Vollversion per Label

Eine neue Version soll ohne eigenen Befehl entstehen, und bei jeder soll man bewusst wählen, ob sie Beta oder für alle ist. Darum hat jeder PR nach `main` genau eines der Labels `beta`, `release` oder `kein-release`; die Prüfung «Release-Art gewählt» (`release-label.yml`) ist rot, bis eines gesetzt ist, und zeigt die Version, die beim Merge entsteht. Ein Label statt eines Ankreuzfelds, weil es sich zuverlässig auslesen lässt und auch nach dem Merge am PR sichtbar bleibt; `kein-release`, damit Doku- und Test-PRs nicht jedes Mal eine Version an alle schicken.

Beim Merge setzt der Job «version» in `release.yml` die nächste Version (`scripts/release.js --auto`), macht Commit und Tag auf `main` (`git push --atomic`, mit dem `GITHUB_TOKEN`) und ruft danach im selben Lauf den Bau auf. Im selben Lauf, weil ein Tag, den der `GITHUB_TOKEN` setzt, keine weiteren Workflows auslöst. Merges kurz nacheinander warten aufeinander (`concurrency`), damit jede Version auf dem Commit der vorherigen aufbaut.

Nummern: Eine Beta zählt auf die nächste Vollversion hin (0.2.0 → 0.2.1-beta.1 → 0.2.1-beta.2 → mit `release` 0.2.1). Eine eigene Nummer steht als Zeile `Version: 0.3.0` im PR-Text, damit man dafür nicht pro Version ein Label anlegen muss. Den Text für den Update-Dialog nimmt die App aus dem Tag: PR-Titel oder die Zeile `Release-Text:`. `npm run release` von Hand bleibt für Arbeit direkt auf `main`.

Voraussetzung: Workflows dürfen auf `main` schreiben (Einstellung «Read and write permissions», bei geschütztem `main` eine Ausnahme für GitHub Actions).

Dabei behoben: v0.2.0 scheiterte beim Veröffentlichen mit `no matches found for '-'` von `gh release create`. Die Argumente gingen als `@flags` an gh, und `$flags` war nach `if` nur ein Text statt einer Liste. Vermutlich hat PowerShell den Text beim Splatting nicht als Argument übergeben; nachgestellt ist das nicht. Jetzt bekommt gh eine ausdrückliche Liste `[string[]]`, und der Workflow schreibt den Aufruf ins Log.

## E39 Python als ein Archiv, einmal entpackt

Das erste echte Update (0.2.1 auf 0.2.2) dauerte über 3 Minuten, ohne dass man etwas sah: Der Installer lief still, entfernte zuerst die alte Version und schrieb dann alles neu. Wer in dieser Zeit auf die Verknüpfung klickte, bekam «geändert oder verschoben», und Windows bot an, sie zu löschen. Der Installer legt bei Updates und bei einer Installation über eine bestehende keine fehlende Verknüpfung neu an; sie blieb weg.

Von den 11'990 Dateien des Installers gehörten 11'951 zum mitgelieferten Python, der Rest von ASK sind 39 Dateien. Jede einzelne Datei kostet beim Löschen und Schreiben Zeit, dazu prüft der Virenscanner jede. Darum liegt Python jetzt als ein Archiv in der App (`resources\python-pack\python.zip`, gepackt von `scripts/pack-python.js` mit `tar.exe`), und die App entpackt es vor dem ersten Terminal-Start nach `%LOCALAPPDATA%\ASK-Desktop\python\<id>` (`src/main/python.js`): mit `tar.exe` von Windows (bsdtar, ab Windows 10 1803 vorhanden), zuerst nach `<id>.neu`, erst danach umbenannt, mit Fortschritt in der Statuszeile. Die `id` hängt nur von `build-python.ps1` und `requirements.txt` ab; ändert sich Python nicht, entpackt ein Update nichts. Ältere Fassungen räumt die App weg, die Deinstallation (nicht ein Update) entfernt den Ordner (`scripts/installer.nsh`).

Damit die vorkompilierten `.pyc` nach dem Entpacken gültig bleiben, entstehen sie mit `--invalidation-mode unchecked-hash`: Python vergleicht sie nicht mit den Zeitstempeln der Quellen (ein ZIP speichert diese nicht immer sekundengenau), und ohne Schreibrecht (Low Integrity) würde sonst jeder Import neu übersetzen.

Sicherheit: Der Starter des Terminals läuft mit normalen Rechten aus diesem Ordner. Er liegt neben `laufzeit`, nicht darin, und bekommt keine Low-Kennzeichnung; Prozesse mit niedriger Integritätsstufe können ihn lesen, aber nicht ändern (wie den Stand der einzelnen EXE unter `ASK-Desktop\app`, E18).

Weiter: Beim Neustart für ein Update läuft der Installer nicht mehr still, sondern zeigt sein Fortschrittsfenster und startet ASK danach (`quitAndInstall(false, true)`); der Dialog sagt vorher, dass ASK sich schliesst und von selbst wiederkommt. Fehlt die Verknüpfung im Startmenü, legt die installierte App sie beim Start mit `shell.writeShortcutLink` wieder an.

Der Wechsel selbst ist noch einmal langsam: Das Update auf die erste Version mit Archiv entfernt die alten 12'000 Dateien, und der erste Start entpackt Python. Ab dann schreibt ein Update rund 40 Dateien.

## E40 Ein Klick, dann Balken und Countdown

Beim Test von 0.2.3-beta.2 war nach «Jetzt aktualisieren» nicht zu sehen, dass etwas geschieht: Den Download (rund 210 MB) zeigte nur ein kleiner Eintrag unten in der Statuszeile, der bei schmalem Fenster abgeschnitten wird, und erst nach dem Download kam die zweite Frage «Jetzt neu starten?». Wer nicht so lange wartet, schliesst ASK oder klickt woanders.

Jetzt gibt es nur noch die eine Frage vor dem Download. Danach zeigt ein Balken oben in der App den Download in Prozent (dazu die Taskleiste), mit dem Hinweis, dass man weiterarbeiten kann. Ist das Update geladen, zählt der Balken 10 Sekunden herunter und startet dann neu (Installer mit Fortschrittsfenster, E39); «Jetzt neu starten» und «Später» stehen im Balken. «Später» installiert beim Schliessen von ASK und startet nicht mehr von selbst. Startet während des Countdowns ein Auftrag, hält die App an und zählt nach dem Auftrag neu. Ein Neustart mitten in einem Auftrag ist damit weiter ausgeschlossen, und es braucht keinen zweiten Dialog.

Dass beim Test nach «Jetzt neu starten» kein Fortschrittsfenster kam, lag an der installierten 0.2.2: Das Update führt immer die alte Version aus, und die installierte noch still. Das Fenster von E39 zeigt erst ein Update aus 0.2.3-beta.2 heraus.


## E41 Rückfrage vor heiklen Schritten

Bisher stand nur im Systemprompt, dass das Modell vor dem Löschen fragen soll. Jetzt prüft die App jeden Werkzeugaufruf, bevor sie ihn ausführt (`src/main/approvals.js`). Als heikel gelten Befehle, die löschen (`del`, `erase`, `rd`, `rmdir`, `rm`, `Remove-Item`, `format`, in Einzeilern `os.remove`, `shutil.rmtree`, `.unlink()` usw.), Befehle mit Pfaden ausserhalb des Ordners (Laufwerk, `..\`, UNC, `%USERPROFILE%` und ähnliche) und `write_file` auf eine vorhandene Datei, die das Modell nicht im selben Auftrag selbst geschrieben hat. Die Arbeitsschleife hält an, die Oberfläche zeigt die Rückfrage im Chat (nicht als Fenster, damit die Person sieht, worum es geht), die Antwort kommt über `chat:approve` zurück. «Immer in diesem Chat» gilt für diese Art Schritt bis zum nächsten Chat. Abgelehnt bekommt das Modell als Ergebnis, dass die Person den Schritt abgelehnt hat. Stoppen gilt als Nein.

Stufen in den Einstellungen: «Nie», «Heikle Schritte» (Standard) und «Immer» (zusätzlich vor jedem Befehl und jeder Änderung an vorhandenen Dateien). Bewusst nicht erkannt wird, was ein Skript intern tut: Ein `python aufraeumen.py`, das Dateien löscht, läuft ohne Rückfrage. Das liesse sich nur mit einer Sperre im Terminal lösen; die Sicherung mit «Rückgängig» (E24) deckt den Fall ab.

## E42 Neue Oberfläche mit React und Vite, Hülle bleibt Electron

Die Oberfläche war eine einzelne `app.js` mit 1’664 Zeilen, die das DOM von Hand baute. Sie ist jetzt mit React gebaut (Vite, Ausgabe nach `src/renderer/dist`, nur das kommt in die App). Der Chat ist ein reines Datenmodell (`src/renderer/src/lib/chatModel.mjs`): gespeicherte Chats und Live-Ereignisse ergeben dieselben Einträge, gruppiert in Aufträge; das Modell hat eigene Tests. Text wird während des Streamens alle 80 ms gesammelt angewendet. Die Schnittstelle zum Hauptprozess (`window.ask`, `src/preload/shell.js`) ist gleich geblieben und nur erweitert. Was parallel in der alten Oberfläche dazukam (E34 Warteschlange, E36 Bearbeiten, Chat-Menü, Kontext und Einstellungen, E40 Update-Balken), ist übernommen. Für die Entwicklung im Browser gibt es eine Attrappe mit Beispieldaten.

Gestaltung nach Variante E aus dem Redesign: freundlicher Chat mit Sprechblasen und einklappbarer Karte für die Arbeitsschritte, rechts ein Arbeitsbereich mit Dateien, Vorschau und Browser, Akzent TF Blau, Hell und Dunkel. Modell, Denkaufwand und Websuche stehen im Eingabefeld. Zusatzinfos (Kopieren, Token-Zeile, Gedanken) erscheinen erst beim Darüberfahren bzw. mit Ctrl+O, damit der Chat ruhig bleibt.

Geprüft wurde auch ein Wechsel zu Tauri 2, Electrobun oder Wails. Dagegen sprach: Der Hauptprozess (rund 5’100 Zeilen Node.js) müsste in Rust oder Go neu geschrieben werden (bei Electrobun mindestens alle Electron-Schnittstellen ersetzt), mehrere Webansichten in einem Fenster brauchen bei Tauri auch in 2.11 das Feature `unstable`, und Bildschirmfotos und Fernsteuerung des Browser-Bereichs gingen nur über eigene WebView2-Anbindungen. Der Platzgewinn (rund 85 MB) ist klein neben den 290 MB Python. Siehe auch E9.

## E43 Projekte

Die linke Leiste gruppierte Chats nach den zuletzt verwendeten Ordnern (höchstens 8). Jetzt gibt es Projekte: Ein Projekt ist genau ein Arbeitsordner mit Namen und Farbe, gespeichert in den Einstellungen (`projects`). Ein neues Projekt bekommt einen Ordner `ASK-Projekte\<Name>` (Zeichen, die Windows nicht erlaubt, werden ersetzt; Gerätenamen wie `CON` bekommen einen Zusatz). Ein vorhandener Ordner wird beim Öffnen zum Projekt. Aus der Liste entfernen löscht nichts. Bestehende Einstellungen werden beim ersten Start übernommen (jeder Ordner aus `recent` wird ein Projekt). Chats zwischen Projekten verschieben geht bewusst nicht: Anhänge, Skripte und Sicherung eines Chats liegen im Ordner des Projekts.

## E44 Das Modell kann fragen

Fehlte dem Modell eine Angabe, schrieb es eine Frage als Antwort und der Auftrag war zu Ende; im Ziel-Modus stiess die App es dann wieder an. Jetzt hat es das Werkzeug `frage_stellen` (Frage, 2 bis 6 Antworten zum Anklicken, optional mehrere wählbar), wie AskUserQuestion in Claude Code. Die Arbeitsschleife wartet auf die Antwort, die als Werkzeugergebnis zurückgeht; der Auftrag läuft danach weiter, ohne neue Eingabe und ohne neue Sicherung. Im Chat steht die Frage als Karte, beantwortet wandert sie in die Arbeitsschritte. Eine Eingabe im normalen Eingabefeld gilt als Antwort. Der Systemprompt sagt, wann fragen (wirklich fehlende Angaben, Konflikte, gesperrte Dateien) und wann nicht (was sich mit den Werkzeugen selbst herausfinden lässt). In automatischen Tests beantwortet die App Fragen mit der ersten Antwort und erlaubt Rückfragen.

## E45 Weitere Korrekturen aus einer zweiten Durchsicht

Eine zweite Durchsicht des Hauptprozesses fand vieles, das E33 und E34 schon behoben hatten (Ordnerwechsel während eines Auftrags, Stopp beim Start eines Befehls, gleichzeitige Terminal-Starts, verpasste Proben, atomares Speichern, 403, Pfade von GitHub, Doppelklick auf Programme). Dazu kamen: «Rückgängig» ging nach einem Auftrag mit über 5000 Dateien nicht mehr (Vergleichen und Zurücksetzen haben jetzt keine Obergrenze, nur das Sichern) und lief nicht gegen einen gleichzeitig gestarteten Auftrag gesperrt. Neue Chats setzen den Textmodus und «keine Bilder» zurück, ebenso ein Modellwechsel. `localhost.` mit Punkt am Ende galt nicht als lokale Adresse, und bei Flux-Bildern wurde das Ziel einer Weiterleitung nicht geprüft. Eine hier gelöschte Datei, die auf GitHub unverändert ist, kommt beim Holen nicht zurück. Angehängte Dateien mit Gerätenamen wie `nul.txt` bekommen einen anderen Namen. Die Vorschau liest von grossen Textdateien nur den Anfang. `$` in Ordnernamen verfälschte den Systemprompt. Tests: `tests/haertung.test.js`, `tests/rueckfrage.test.js`.

## E46 Parallele Chats, auch über Projekte

Bisher lief immer nur ein Auftrag, und während er lief, liess sich kein anderer Chat und kein anderes Projekt öffnen. Jetzt hat jedes Projekt eine eigene Laufzeit (Terminal und Dateibeobachtung, `runtimes` in `main.js`), und jeder Chat seinen eigenen Agenten. Wechselt man das Projekt, zeigt die App das Terminal und die Dateien dieses Projekts; die Aufträge anderer Chats laufen im Hintergrund weiter. Ereignisse tragen Chat und Ordner mit, die Oberfläche zeigt nur die des sichtbaren Chats; die Liste links zeigt mit einem Kreisel, welche Chats arbeiten, und ihr Menü hat «Stoppen». Rückfragen und Fragen gehen an den Agenten, der sie gestellt hat. Eine Laufzeit ohne sichtbaren und ohne laufenden Chat wird freigegeben (Terminal beendet). Die Sicherung für «Rückgängig» gehört zum Ordner, nicht zum Chat: Ändern zwei Chats im selben Projekt gleichzeitig Dateien, setzt «Rückgängig» auch die Änderungen des anderen zurück. Die Rückfrage vor dem Zurücksetzen listet die betroffenen Dateien.

## E47 GitHub-Bereich, Branches und Pull Requests

GitHub ging bisher nur über das Modell und den `/github`-Dialog, und nur auf dem Standard-Branch. Rechts gibt es jetzt den Reiter «GitHub» (`src/renderer/src/components/GithubPane.jsx`) für das sichtbare Projekt: verbundenes Repository, Branch, was hier geändert, neu oder gelöscht ist, Hinweis auf Neueres auf GitHub, «Holen» und «Hochladen» mit einer Beschreibung, ohne Verbindung «Repository verbinden» und «Neues Repository» (privat voreingestellt), dazu Branches und Pull Requests. Das Modell bekommt die Werkzeuge `github_branch` (liste, wechseln, neu) und `github_pr` (liste, neu).

- **Gleiche Regeln wie beim Modell.** Hochladen und Anlegen laufen im Hauptprozess über dieselben Werkzeuge (`runTool` mit `github_push` bzw. `github_create`), damit Rückfrage, «Für dieses Repository nicht mehr fragen» und die eigene Rückfrage vor dem Ersetzen einer auf GitHub geänderten Datei genau gleich gelten. Holen, Branch und Pull Request aus dem Reiter brauchen keine Rückfrage, weil die Person selbst klickt; erstellt das Modell einen Pull Request, fragt die App vorher (`ctx.confirm`).
- **Nicht gleichzeitig.** Während in einem Projekt ein Auftrag läuft, lehnt der Hauptprozess Abgleiche aus dem Reiter ab, und während der Reiter abgleicht, bekommt das Modell bei GitHub-Werkzeugen «kurz warten» zurück (`GitHub.exclusive`, `locked`). Sonst schrieben beide dieselbe gespeicherte Verbindung.
- **Branches über die API.** Die Liste kommt von `GET /repos/…/branches` (höchstens 100). Ein neuer Branch ist ein neuer Ref (`POST /git/refs`) auf dem Commit des letzten Abgleichs, nicht auf dem neuesten Stand von GitHub: Darauf beruhen die Prüfsummen in der Verbindung, und Änderungen hier gehen so ohne Konflikt auf den neuen Branch. Wechseln setzt den Branch in der Verbindung (sie liegt weiterhin ausserhalb des Ordners, E33) und holt dessen Stand mit dem normalen Abgleich: Was hier unverändert ist, bekommt die Fassung des anderen Branches, was es dort nicht gibt, wird entfernt. Darum geht Wechseln nur, wenn hier nichts mehr hochzuladen ist; sonst würden Änderungen mit dem anderen Stand vermischt oder später auf den falschen Branch geladen. Die App erklärt das und schlägt vor, zuerst hochzuladen oder einen neuen Branch anzulegen. Den Ziel-Branch prüft sie vorher, damit ein Tippfehler nicht wie ein leeres Repository aussieht und alles löscht. Branch-Namen prüft `validBranch` nach den Regeln von git (enger: nur Buchstaben, Ziffern, `- _ . /`), weil sie in Adressen der API mit dem Token gehen; Schrägstriche bleiben in der Adresse stehen (`heads/feature/x`).
- **Pull Requests.** Vom Branch des Ordners in den Standard-Branch (`POST /pulls`), mit Titel und Beschreibung; offene Pull Requests (die 30 neuesten) mit Link, geöffnet im Browser von Windows (`link:open`). Ein Pull Request enthält nur, was auf GitHub liegt: Mit Änderungen, die noch nicht hochgeladen sind, lehnen Reiter und Werkzeug ab. Zusammenführen zweier Branches macht man auf GitHub über den Pull Request, nicht in der App.
- **Neu laden.** Der Reiter lädt beim Öffnen, nach jeder Aktion, nach Änderungen an Dateien, nach dem Ende eines Auftrags und nach dem Schliessen eines Dialogs (gesammelt, 0.9 s), solange er sichtbar ist.

Nicht gewählt (bewusst ohne echtes Git): ein lokales git, das im Ordner ein `.git` anlegt und die Arbeit macht. Git liefe mit den normalen Rechten der Person (die Git-Werkzeuge laufen unter Low nicht zuverlässig, E22), in einem Ordner, in den das Modell schreiben kann. Über `.git/config` (Filter wie `filter.x.clean`, `core.fsmonitor`, `core.sshCommand`, Aliase) und `.git/hooks` führt git dabei Befehle aus, die dort jemand hinterlegt hat; das Modell könnte so nach einer manipulierten Webseite oder Datei Befehle ausserhalb der Sperre starten (E33). Ein mitgeliefertes git ohne diese Wege wäre ein eigener Nachbau mit allen Ausnahmen; die REST-API kommt ohne Programm im Ordner aus. Ebenfalls nicht gewählt: Zusammenführen von Branches in der App (Konflikte innerhalb einer Datei kann die App nicht lösen, E22) und Wechseln mit mitgenommenen Änderungen wie `git switch` (unklar, was bei einer Datei passiert, die auf beiden Branches verschieden ist).

Tests: `tests/github.test.js` (Branch-Namen, Branches auflisten, anlegen und wechseln, Pull Requests, Werkzeuge `github_branch` und `github_pr`, Sperre während des Abgleichs), Nachbau der API in `tests/e2e/mock-github.js` um Branches, Refs und Pull Requests erweitert. Die Oberfläche hat in der Attrappe (`npm run dev:ui`) Beispieldaten; `?github=aus` zeigt «nicht verbunden», `?github=leer` «kein Repository». Mit dem echten GitHub ist auch das noch nicht getestet.

## E48 Entwicklungsserver

Für eine Webseite mit Vite, React oder Express startete das Modell `npm run dev` mit `run_command`. Der Befehl endet nie: Der Auftrag wartete bis zum Zeitlimit, danach lief der Server unbemerkt weiter, und öffnen konnte ihn das Modell nicht, weil `browser` keine lokalen Adressen öffnet (E29).

Jetzt gibt es Server als eigene Sache (`src/main/servers.js`, eine Instanz pro Projekt-Terminal). Gestartet wird über dieselbe Terminal-API wie jeder Befehl (Low Integrity, Job-Objekt, E6), aber mit `wait=0`, ohne auf das Ende zu warten. Die App fragt jede Sekunde neue Ausgabe ab, behält die letzten 200 Zeilen, erkennt darin die Adresse und prüft mit einer Verbindung auf den Port, ob der Server antwortet. Zustände: startet, läuft, gestoppt, Fehler (mit Exitcode). Stoppen ist `DELETE /execute/{id}` und beendet den Prozessbaum. Werkzeuge für das Modell: `server_starten` (wartet bis 8 s auf die Adresse und liefert id, Adresse und Ausgabe), `server_liste`, `server_stoppen`. `server_starten` läuft durch dieselbe Rückfrage (E41) und dieselbe Sperre für Programme mit Fenster wie `run_command`. Höchstens 5 Server pro Projekt; derselbe Befehl startet nicht zweimal.

Browser: Das Modell darf `http://localhost:<port>`, `127.0.0.1` und `[::1]` öffnen, aber nur für Ports, die ein noch laufender Server der App in diesem Projekt genannt hat; die Ports der Terminals bleiben gesperrt, alles andere im lokalen Netz auch. Das gilt beim Öffnen und nach Weiterleitungen. Die Person kann wie bisher jede Adresse öffnen. Eine stärkere Prüfung (lauscht wirklich ein Prozess dieses Terminals auf dem Port?) gibt es nicht: `/ports` von open-terminal findet unter Windows keine Unterprozesse. Das Restrisiko ist klein, weil das Modell lokale Adressen aus dem Terminal ohnehin erreicht (Python, curl); gesperrt bleibt vor allem, dass es mit der Anmeldung der Person im Browser beliebige lokale Dienste bedient.

Oberfläche: ein einklappbarer Bereich «Server» oben im Reiter «Browser», kein eigener Reiter. Server sind dazu da, im Browser angesehen zu werden; «Öffnen» lädt die Seite direkt darunter, und ein vierter Reiter wäre im schmalen Bereich rechts zu eng. Ein grüner Punkt am Reiter zeigt, dass ein Server läuft. Startet das Modell einen Server, klappt der Bereich auf. Die Person startet Server ohne Rückfrage, weil sie den Befehl selbst tippt.

Lebensdauer: Server gehören zum Terminal ihres Projekts. Sie laufen beim Wechsel des Chats weiter und werden gestoppt, wenn die App das Projekt schliesst (anderes Projekt sichtbar und dort kein Auftrag mehr, Projekt aus der Liste entfernt) oder sich beendet; zuerst einzeln, danach beendet das Job-Objekt des Terminals, was übrig ist. Endet oder startet das Terminal neu, gelten seine Server als gestoppt. Tests: `tests/servers.test.js`, Freigabe der Ports in `tests/visual.test.js`.

## E49 Nutzungsübersicht und Einführung

**Nutzung.** Die App zeigt, wie viel mit ASK gearbeitet wurde. Der Dialog «Nutzung» (`/nutzung`, Einstellungen) zählt aus den gespeicherten Chats aller Projekte (`src/main/usage.js`, IPC `usage:get`): Chats, Aufträge (Eingaben der Person ohne automatische Nachrichten), Arbeitsschritte (Werkzeugergebnisse), erzeugte Token (`stats.tokens`, geschätzte zählen mit und werden mit «≈» markiert), Zeit des Modells (`stats.totalMs`, wie «Modell 70.4 s» unter einer Antwort) und Dateien aus `changes` der Eingabe (neu und geändert; ein rückgängig gemachter Auftrag zählt nicht). Dazu die letzten 30 Tage mit Aufträgen und Token pro Tag. Gerechnet wird nur lokal und ohne Netz; die Zusammenfassung pro Chat-Datei bleibt zwischengespeichert, solange sich Änderungszeit und Grösse nicht ändern, damit auch Hunderte Chats schnell gehen. Die Prompt-Token sind bewusst nicht dabei: Jeder Arbeitsschritt schickt den ganzen Verlauf wieder, ihre Summe wäre ein Vielfaches dessen, was die Person gelesen oder geschrieben hat. Chats in gelöschten Ordnern oder auf anderen Laptops fehlen; die Zahlen sind eine Übersicht, keine Abrechnung.

Gestaltung wie die anderen Dialoge: vier Zahlen oben, ein Balkendiagramm aus CSS (eine Reihe in TF Blau, keine Legende, Wert beim Darüberfahren), eine Tabelle pro Projekt. Keine Diagramm-Bibliothek.

**ASK.** Ob ASK einem normalen Konto Nutzungszahlen zeigt, ist nicht bekannt. Die App versucht darum nach dem Öffnen des Dialogs, ohne ihn aufzuhalten (zuerst kommen die lokalen Zahlen), mit dem Token der Person und höchstens 5 Sekunden lang: die eigene Chatliste von Open WebUI (`/api/v1/chats/?page=…`, Zahl der Chats auf der Webseite, davon in den letzten 30 Tagen) und einige mögliche Schnittstellen für Nutzungszahlen (`/api/v1/chats/stats/usage`, `/api/v1/users/user/usage`, `/api/v1/analytics/summary`), von denen nur Zahlen mit bekannten Namen übernommen werden (Token, Nachrichten). Die Anfragen laufen ohne die Abmelde-Logik (ein 401 oder 403 heisst hier «nicht erlaubt», nicht «abgemeldet»). Kommt nichts, fehlt der Abschnitt. **Nicht mit dem echten ASK geprüft**; welche Angaben wirklich kommen, zeigt erst ein Test mit einem Konto der Schule.

**Einführung.** Nach dem Hinweis, dem ersten Projekt und der Anmeldung zeigt die App einmal vier Karten, die auf Teile der Oberfläche zeigen (Projekte, Eingabefeld, Arbeitsschritte mit Fragen und Rückfragen, Dateien mit «Rückgängig»). Pro Karte ein Satz oder zwei, «Weiter» und «Überspringen», Esc überspringt. Ein Rahmen hebt den Teil hervor, der Rest ist abgedunkelt; fehlt der Teil (etwa noch kein Auftrag im Chat), zeigt die Karte auf den Chat. Gemerkt wird das in `prefs.tourDone` (wie die anderen Vorlieben der Oberfläche); wer ASK schon vor dieser Version benutzt hat (Hinweis schon bestätigt), bekommt `tourDone` beim Start gesetzt und sieht die Einführung nicht von selbst. «Einführung zeigen» in den Einstellungen öffnet sie wieder. Eine längere Anleitung mit Beispielauftrag wäre möglich, aber die Oberfläche soll sich selbst erklären; die Einführung zeigt nur, wo was ist.

Nebenbei behoben: Ohne Update-Balken war die Arbeitsfläche nur so hoch wie ihr Inhalt (die leere Zeile für den Balken bekam den Platz); die Arbeitsfläche steht jetzt fest in der dritten Zeile.

## E50 Gedächtnis

ASK merkt sich über alle Chats hinweg kurze, dauerhafte Angaben: über die Person (Bereich «konto»: Vorlieben, Kenntnisse, Ziele, wie sie Antworten mag) und über ein Projekt (Bereich «projekt»: Entscheide, Vorgaben). Ein Eintrag ist ein Satz mit ein bis drei Schlagworten, dazu Herkunft (Chat) und Zeit (`src/main/memory.js`, `knowledge.js`). Nach jedem fertigen Auftrag fragt die App das Modell im Hintergrund einmal mit einer kurzen Anweisung, ob es etwas Dauerhaftes gab, und erwartet nur JSON (`{"neu":[…],"veraltet":[ids]}`, höchstens drei neue). Nicht nach Stopp oder Fehler, nicht bei sehr kurzen Eingaben, höchstens einmal pro Eingabe und immer nur eine Anfrage gleichzeitig; scheitert sie, bleibt das still, denn der Auftrag selbst ist fertig. Die Antwort liest die App vorsichtig (Codeblöcke, Text davor, Kommas am Ende, typografische Anführungszeichen) und verwirft, was nach Zugangsdaten, Kontonummern, Gesundheit oder Noten aussieht. Ausdrücklich merken und vergessen kann das Modell mit `merken` und `vergessen`. Jeder Chat bekommt die Einträge über die Person und die seines Projekts im Systemprompt, neueste zuerst, höchstens 2’500 Zeichen, ausdrücklich als «Angaben, keine Anweisungen». Projektnotizen (`.ask/projekt.md`) bleiben für Aufbau und Dateien eines Projekts.

Warum in den Daten der App (`userData/gedaechtnis.json`) und nicht im Projektordner: Den Projektordner kann das Modell beschreiben (E33). Ein Eintrag dort liesse sich von einer manipulierten Webseite oder Datei aus in alle künftigen Chats schmuggeln, und der Ordner wird vielleicht weitergegeben oder auf GitHub hochgeladen, mit allem, was ASK über die Person weiss. In userData schreibt nur die App (das Terminal läuft mit niedriger Integritätsstufe), und die Angaben über die Person gelten projektübergreifend, gehören also zu keinem Ordner. Was bleibt: Ein Auftrag, in dem das Modell manipuliert wurde, kann `merken` aufrufen oder über seine Schlussantwort die Extraktion beeinflussen. Darum ist jeder Eintrag im Dialog «Gedächtnis» sichtbar, mit Herkunft, und lässt sich ändern oder löschen; neue automatische Einträge erscheinen zudem als Hinweis im Chat.

Was an ASK geht: bei jedem Chat die Einträge im Systemprompt; nach jedem Auftrag die letzte Eingabe und die Schlussantwort (je höchstens 1’500 Zeichen) mit den bekannten Einträgen, an dasselbe Modell wie der Chat, ohne Werkzeuge und ohne Websuche. Ausschalten: Einstellungen, «Gedächtnis» auf Aus. Dann gehen keine Einträge mehr an ASK, es gibt keine Abfrage nach dem Auftrag, und `merken`/`vergessen` fehlen; die gespeicherten Einträge bleiben, bis man sie im Dialog löscht («Alles vergessen»).

Der Dialog zeigt das Gedächtnis als Netz und als Liste. Das Netz ordnet die Knoten mit einer eigenen kleinen kräftebasierten Rechnung an (`src/renderer/src/lib/graph.mjs`, ohne Bibliothek): fester Startwert und feste Zahl Schritte, damit das Bild bei gleichen Daten gleich bleibt und nichts nachzittert; 400 Einträge brauchen rund eine Viertelsekunde.

## E51 Eigene Skills

Lernende wiederholen Aufträge (Wochenbericht, Code erklären, Quellen prüfen). Wie in Claude gibt es dafür eigene Skills: `userData/skills/<name>/SKILL.md` mit Kopf (`name`, `description`) und der Anleitung (`src/main/skills.js`). Der Systemprompt nennt nur Name und Beschreibung (höchstens 1’800 Zeichen); die Anleitung liest das Modell bei Bedarf mit `skill_lesen`. Ruft die Person `/name …` auf, geht die Anleitung für diesen Auftrag vor ihrem Text an das Modell; im Chat steht nur, was sie getippt hat. Das Modell kann mit `skill_erstellen` einen Skill anlegen, aber nur nach Rückfrage. Gründe für userData statt `.ask/skills` (dort liegen die mitgelieferten Anleitungen, E3): Eigene Skills gelten in allen Projekten, und eine Anleitung, die das Modell selbst im Ordner ändern könnte, wäre ein Weg, künftige Aufträge zu steuern (E33).

Teilen ohne Server: Export als `name.md`, Import einer solchen Datei. Der Import prüft Grösse (höchstens 100 KB), Text (UTF-8, keine Nullbytes), den Kopf und den Namen; der Name wird zu einem sicheren Ordnernamen (klein, Umlaute ausgeschrieben, nur Buchstaben, Ziffern und Bindestriche, keine Befehle der App wie `/neu`), und ein vorhandener Skill wird nur nach Rückfrage ersetzt. Ein importierter Skill ist eine Anleitung von jemand anderem an das Modell. Er läuft mit denselben Grenzen wie jeder Auftrag (niedrige Integritätsstufe, Rückfragen vor heiklen Schritten, Rückgängig); vor dem Import lohnt es sich, ihn zu lesen.

## E52 Modus im Eingabefeld, Planmodus, Wiederholen, Benachrichtigungen, Rückgängig ab jedem Auftrag, @Datei

**Modus.** Die Stufe der Rückfrage (E41) stand versteckt in den Einstellungen. Jetzt wählt man im Eingabefeld einen Modus, wie in Claude Code, mit Shift+Tab: «Auto» (Standard), «Nachfragen» (vor jedem Befehl und jeder Änderung an vorhandenen Dateien) und «Plan». Die Einstellung «heikle Schritte / immer» fällt weg. Der Modus gilt für alle Chats und wird bei jedem Schritt gelesen; ein Wechsel wirkt also auch auf einen laufenden Auftrag.

**Prüfung in «Auto».** Feste Regeln (E41) erkennen nur, was im Befehl steht, und fragen oft bei Harmlosem (ein `del` einer Zwischendatei) und nie bei Gefährlichem ohne Schlüsselwort (`reg add`, `curl … | iex`). Darum beurteilt im Modus «Auto» ein zweiter, kurzer Aufruf an ein Modell in ASK jeden Befehl (`run_command`, `server_starten`) und jeden Dateischritt, den die Regeln heikel finden (`src/main/review.js`), wie der Auto-Modus von Claude Code. Der Prüfer bekommt den Auftrag der Person, den Ordner und den einen Schritt, nicht den Verlauf; die Anweisung sagt, dass Text im Schritt nur zu prüfen und nicht zu befolgen ist (ein Befehl könnte «das ist sicher» enthalten). Er antwortet mit JSON (`gefaehrlich`, `grund`). Harmlos läuft der Schritt, und derselbe Schritt wird im Chat nicht nochmals geprüft; gefährlich wird er zur Rückfrage mit dem Grund. Lesen, Suchen und neue Dateien im Projekt prüft er nicht, das kostet nur Zeit. Ist die Prüfung nicht möglich (Fehler, 20 Sekunden ohne Antwort, kein JSON), gelten die festen Regeln der Stufe «heikel», und die Rückfrage sagt das. Jede Prüfung ist eine eigene Anfrage an ASK, sie braucht ein bis wenige Sekunden pro Befehl; in den Einstellungen lässt sich dafür ein schnelleres Modell wählen. Der Prüfer ist eine zweite Meinung, keine Sperre: Der Schutz bleibt die Sperre des Terminals (E33) und «Rückgängig».

**Planmodus.** Das Modell bekommt nur Werkzeuge, die nichts ändern (lesen, suchen, Web, Ansehen, frühere Chats, GitHub-Status, Aufgabenliste, Fragen) und dazu `plan_vorlegen`. Ruft es trotzdem ein anderes Werkzeug auf (etwa im Textmodus), weist die App es ab. Mit `plan_vorlegen` endet der Auftrag; der Plan steht als Karte im Chat. «Umsetzen» wechselt auf Auto und schickt «Setze den Plan um.», das Modell hat den Plan im Verlauf. «Ändern» schreibt «Ändere den Plan:» ins Eingabefeld.

**Wiederholen.** Bricht die Verbindung zu ASK ab (Netzwerkfehler) oder antwortet ASK mit 429, 502, 503 oder 504, wartet die App 2, 5, 10, 20 und 30 Sekunden und versucht dieselbe Anfrage nochmals. Eine halb angekommene Antwort verfällt (die Anzeige verwirft sie), damit kein doppelter Text im Verlauf steht. Nicht wiederholt werden Fehler mit anderem Status (das Modell oder die Anfrage ist das Problem), eine abgelaufene Anmeldung und eine Verbindung, die fünf Minuten nichts sendete. Stoppen bricht das Warten ab.

**Benachrichtigungen.** Läuft ein Auftrag, während man woanders ist (Fenster im Hintergrund oder anderer Chat sichtbar), meldet Windows, wenn er fertig ist (ab 8 Sekunden Dauer), eine Frage stellt oder um Erlaubnis bittet. Ein Klick öffnet den Chat. Abschaltbar in den Einstellungen.

**Rückgängig ab jedem Auftrag.** «Rückgängig» gab es nur beim letzten Auftrag mit Änderungen. Die Sicherung jedes Auftrags hält aber den ganzen Stand davor fest, also lässt sich auch zu einem früheren Auftrag zurück. Dabei gehen die Änderungen aller Aufträge danach verloren; die Rückfrage sagt das, und die späteren Aufträge sind danach als rückgängig gemacht markiert. Bei früheren Aufträgen erscheint der Knopf erst beim Darüberfahren, damit der Chat ruhig bleibt.

**@Datei.** `@` im Eingabefeld schlägt Dateien des Projekts vor (dieselbe Suche wie im Dateibereich). Gewählt steht `@pfad` im Text, mit Leerzeichen `@"pfad"`. Beim Senden prüft die App, welche erwähnten Pfade es gibt, und hängt für das Modell einen Hinweis an («Mit @ erwähnt, liegt im Arbeitsordner: …»). Den Inhalt schickt sie nicht mit; das Modell liest die Datei selbst, wenn es sie braucht. So bleibt der Verlauf klein, und eine grosse Datei sprengt den Kontext nicht.

Tests: `tests/modi.test.js`.

## E53 Anmeldung, wenn die ASK-Seite nicht lädt

Lernende mit neu installierter App sahen statt der Anmeldeseite den gesperrten Chat («ASK: Fehler», in 0.3.2 ein endloses «Verbinde mit ASK …»). Ursache: Lädt die ASK-Seite nicht (offline, Proxy, Zertifikat), meldet Electron `did-fail-load` und danach für die Fehlerseite von Chromium trotzdem `did-finish-load`, und `getURL()` gibt weiter die Adresse von ASK zurück. Der Hinweis «ASK ist nicht erreichbar» kam als einmaliges Ereignis an die Oberfläche, das `did-finish-load` gleich wieder löschte (oder das die Oberfläche beim Start verpasste). Danach las die App das Token aus der Fehlerseite, was scheitert und bis 0.3.1 zum Status «fehler» führte.

Jetzt:
- Der Ladefehler steht im Status von ASK (`loadError`), den die Oberfläche beim Start abholt. Er bleibt bis zu einem erfolgreich geladenen Dokument; `did-finish-load` nach einem Ladefehler zählt nicht. Die Ereignisse `ask:loaded` und `ask:load-failed` gibt es nicht mehr.
- Nach einem Ladefehler liest die App kein Token und lädt die Seite alle 15 Sekunden von selbst neu. Ein Token, das sich nicht lesen lässt, ist kein Fehler von ASK; der Grund steht als `detail` im Status.
- Ist die Seite fertig geladen, aber nicht auf ASK (gleich zur Microsoft-Anmeldung weitergeleitet, Seite des Netzes), gilt «nicht angemeldet»: Die Seite wird gezeigt, statt hinter dem Kreisel zu warten.
- Dauert «Verbinde mit ASK …» länger als 15 Sekunden, zeigt die Oberfläche einen Hinweis mit «Seite neu laden» und dem Grund, falls bekannt.
- Meldet ASK nach der Anmeldung einen Fehler (z. B. kein Modell für das Konto), steht er über dem Eingabefeld, mit «Erneut prüfen» und «Abmelden und neu anmelden».

Tests: `tests/askapi.test.js`. In der Oberfläche mit Beispieldaten: `?offline=1`, `?askfehler=1`.
