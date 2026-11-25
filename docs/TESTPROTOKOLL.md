# Testprotokoll

Stand 28.10.2026, Version 0.1.0 mit eigener Chat-Oberfläche (ENTSCHEIDE E19). Testrechner: Windows 11 Pro 26200, Standardkonto ohne Admin-Rechte, Microsoft 365 (Word, Excel, PowerPoint) installiert, kein WSL, keine Windows Sandbox, kein Docker. Electron 44.6.0, Python 3.14.8 (mitgeliefert), open-terminal 0.14.2.

Ein ASK-Login stand für die Tests nicht zur Verfügung. Wo ASK nötig ist, stand ein Nachbau der benötigten Open-WebUI-Schnittstellen zur Verfügung (`tests/e2e/mock-ask.js`): Anmeldeseite, `/api/v1/auths/`, `/api/models` und `/api/chat/completions` mit einem Skript-«Modell», das Antworten und Werkzeugaufrufe wie Open WebUI streamt. Der zweite Teil dieses Dokuments beschreibt die Tests, die du mit dem echten ASK machen musst.

## Teil 1: Durchgeführte Tests

Alle Tests sind automatisiert und lassen sich wiederholen (Befehle am Ende). Ergebnis des letzten Laufs: alle bestanden.

### Terminal-Starter und Server (Python, `tests/test_terminal.py`, 16 Tests)

Startet den Starter wie die App und prüft gegen den laufenden Server: Der Server läuft mit niedriger Integritätsstufe (S-1-16-4096) im gewählten Ordner. Ohne oder mit falschem Key antwortet er mit 401. `/system` liefert den eigenen Systemprompt. Die Datei-API verweigert Lesen und Schreiben ausserhalb des Ordners (absoluter Pfad, `..`, `C:\Windows\win.ini`) und Befehle mit Arbeitsverzeichnis ausserhalb (403). Schreiben im Ordner geht. Befehle liefern UTF-8 ohne Steuerzeichen (`äöü ß`, Dateiname `Übung.txt`); Ausgaben in der OEM-Codepage (`ping`) kommen ohne Ersatzzeichen an. `read_file` liest Word vollständig, auch Text in Inhaltssteuerelementen (Deckblatt), Tabellen darin und Kopfzeilen. `echo x > <ausserhalb>` und Schreiben ins Benutzerprofil scheitern in der Shell (Windows verweigert). python-docx, openpyxl, python-pptx, pypdf, reportlab, pandas und matplotlib sind importierbar. Abbrechen eines Befehls beendet auch dessen Kindprozess. CORS erlaubt nur den ASK-Ursprung und beantwortet den Private-Network-Preflight. Skills aus `.ask/skills` erscheinen unter `/skills`. Das interaktive Terminal über WebSocket funktioniert mit dem Header, den die App einsetzt, und lehnt Verbindungen ohne oder mit falschem Header ab. Wird der Starter hart beendet (wie bei einem Absturz der App), ist der Server danach nicht mehr erreichbar.

### Anleitungen und Prüfskript (`tests/test_skills.py`, 9 Tests)

Die Code-Gerüste aus `word`, `excel`, `powerpoint` und `pdf` laufen unverändert durch (Excel mit einer CSV in Windows-Kodierung und `;`) und ergeben Dateien, die `pruefen.py` ohne Warnung durchlässt (A4, Überschriften, Tabelle, zwei Blätter mit Diagramm, 16:9, Text im PDF). Die Beispiele «Vorhandene Datei bearbeiten» laufen unverändert gegen echte Dateien: Word (Text über mehrere Formatierungsabschnitte ersetzt, auch im Deckblatt-Inhaltssteuerelement, fett bleibt fett, auch in Tabelle und Kopfzeile; Absätze nach einer Überschrift eingefügt, einer gelöscht; Original unverändert), Excel (Zelle geändert, Formel, Zahlenformat und Diagramm bleiben; `excel_risiken` meldet Makros, Formen und Sparklines), PowerPoint (Text in Titel, Inhalt und Notizen ersetzt mit Formatierung, Folie mit vorhandenem Layout angehängt), PDF (Seite entfernt, Seite gedreht). Das Prüfskript zählt Bilder in Kopf- und Fusszeilen, sucht die Excel-Kopfzeile in der ersten nicht leeren Zeile statt in A1 und erkennt fehlende Überschriften, leere Absätze in Folge, Platzhaltertext und deutsche Funktionsnamen in Excel-Formeln.

### Ordnersperre (`tests/folders.test.js`, 7 Tests)

Abgelehnt werden: Ordner in OneDrive (über Umgebungsvariablen, über «OneDrive» im Pfad, über synchronisierte SharePoint-Bibliotheken aus der Registry, auch über eine Junction dorthin), Ordner, die einen solchen Ordner enthalten, Laufwerkswurzeln, das Benutzerprofil, `C:\Users`, AppData, Windows, Programme, ProgramData, Netzwerkpfade, fehlende Ordner und Dateien. Erlaubt: Projektordner im Profil und auf anderen Laufwerken. Auf dem Testrechner werden alle sechs echten OneDrive- und SharePoint-Synchronisationsordner aus der Registry erkannt.

### Aufräumen der einzelnen EXE (`tests/portable.test.js`, 6 Tests)

Gefunden werden ältere Stände unter `%LOCALAPPDATA%\ASK-Desktop\app` und `.neu`-Ordner, die älter als eine Stunde sind. Stehen bleiben der eigene Stand (auch bei anderer Schreibweise), jüngere `.neu`-Ordner und Dateien.

### Antworten von ASK und Verlauf (`tests/askapi.test.js`, 10 Tests)

Server-Sent Events über Teilstücke hinweg; `<think>` wird vom Text getrennt, auch wenn die Marke geteilt ankommt; gestreamte Werkzeugaufrufe (mehrere, Argumente in Teilen) werden zusammengesetzt; ganze Antworten ohne Streaming und Fehlermeldungen; Tokenzahlen und Zeiten (gemessen aus `usage` oder aus der Textlänge geschätzt); Freigabe der Websuche (Konfiguration, Berechtigung, Admin) und Quellen aus `sources` (ohne Doppelte, nur Web-Adressen); Werkzeugaufrufe im Text (`<tool_call>`); Verlauf an das Modell ohne Denktexte, ältere Werkzeugergebnisse gekürzt; im Textmodus auch frühere Aufrufe als Text; Argumente, Kürzen und Beschreibungen der Werkzeuge.

### Aufräumen, Schleifenschutz, gesperrte Programme (`tests/cleanup.test.js`, `tests/tools.test.js`, 5 Tests)

Skripte und Zwischendateien älter als 14 Tage werden gelöscht, leere Chat-Ordner entfernt; neue Dateien, Chats und die Dateien im Arbeitsordner bleiben. Der Schleifenschutz greift bei drei gleichen Aufrufen hintereinander und bei acht Fehlern hintereinander, nur innerhalb des laufenden Auftrags. `run_command` lehnt Word, Excel, PowerPoint, Explorer und `start` ab (auch mit vollem Pfad und nach `&&`), lässt aber `excel.py`, `findstr WINWORD` oder `echo start` durch.

### GitHub-Abgleich (`tests/github.test.js`, 4 Tests)

Gegen einen Nachbau der GitHub-Schnittstelle (`tests/e2e/mock-github.js`, Repositories, Blobs, Trees, Commits und Refs im Speicher): Blob-SHA wie `git hash-object`, Repository-Angaben als `owner/name` oder URL; Verbinden lehnt ein falsches Token ab und speichert ein richtiges; Anlegen lädt den Ordner ohne `.ask` hoch, Klonen in einen zweiten Ordner, Ändern, Löschen und Hochladen, Holen in den ersten Ordner (geänderte und gelöschte Dateien); beidseitig geänderte Datei bleibt lokal unverändert und wird als Konflikt gemeldet; Hochladen bei neuerem GitHub-Stand wird abgelehnt; leeres Repository.

### Anhänge (`tests/attachments.test.js`, 4 Tests)

Dateinamen ohne Pfad und ohne verbotene Zeichen («a:b» ist kein Laufwerk), eingefügtes `image.png` heisst `bild-<Datum>-<Zeit>.png`; Anhänge landen in `.ask/anhaenge/<Chat>/` (je Chat getrennt, ungültige Chat-Kennung abgelehnt), ein gleicher Name wird zu «name (2)», nichts wird überschrieben, mehr als 10 Anhänge abgelehnt; Bild als data-URL, verkleinert, gelöschte Datei ohne Fehler; Verlauf an das Modell mit Pfaden im Text, Bilder nur aus den letzten zwei Eingaben mit Bildern, ohne Bildfunktion nur Text.

### Sicherung und Rückgängig (`tests/snapshot.test.js`, 3 Tests)

Stand sichern ohne `.ask`, `.git` und Office-Sperrdateien; geänderte, neue und gelöschte Dateien werden erkannt; Zurücksetzen stellt Inhalt und gelöschte Dateien wieder her und entfernt neue Dateien samt leerem Ordner; das Zurücksetzen lässt sich über den Sicherheitsstand selbst zurücknehmen; gleiche Inhalte werden nur einmal gespeichert, auch nach einem Neustart; eine nur berührte Datei gilt nicht als geändert; Stände älter als 14 Tage und nicht mehr gebrauchte Fassungen werden gelöscht.

### Ansehen und Häkchen beim Hochladen (`tests/visual.test.js`, 3 Tests)

Seitenangaben («1-3», «5, 1», höchstens 6) und Tastennamen; Bilder aus Ansichten gehen als eigene Nachricht mit, nur die letzten zwei, ältere als Text «nicht mehr mitgeschickt», Anhänge der Person bleiben; der Schleifenschutz zählt Ansichten nicht als neue Eingabe; Hochladen fragt mit Häkchen, merkt sich das Repository und fragt danach nicht mehr, Ablehnen lädt nichts hoch und merkt nichts; ohne Bildfähigkeit kein `view_file`.

### Internet (`tests/web.test.js`, 4 Tests)

HTML wird zu Text (Titel, Überschriften mit #, Listen, Tabellen, Entities, ohne Navigation, Kopf, Fuss, Skripte und Stile); Treffer von DuckDuckGo mit aufgelösten Weiterleitungen, ohne Werbung; nur öffentliche http(s)-Adressen (localhost, 127.x, 10.x, 172.16–31, 192.168.x, ::1, Namen ohne Punkt, file: und ftp: gesperrt); Suche zuerst über ASK, nach einem Fehler in dieser Sitzung gleich über DuckDuckGo; lange Seiten in Abschnitten zu 12’000 Zeichen mit «ab»; PDF wird gespeichert; Weiterleitung ins lokale Netz gesperrt.

### Frühere Chats (`tests/history.test.js`, 2 Tests)

Liste im Systemprompt ohne den laufenden Chat, neueste zuerst; Verlauf als Text mit Anhängen und Werkzeugaufrufen, ohne Ansichten; Suche verlangt alle Wörter und lässt den laufenden Chat aus; Lesen mit ungültiger oder fehlender id abgelehnt; ohne Schalter liefern die Werkzeuge nichts.

### Bilder mit Flux (`tests/bild.test.js`, 9 Tests)

Gegen ein nachgebautes ASK: Argumente für Flux nur, wenn gesetzt und gültig (Format und Qualität aus der Liste, Anzahl höchstens 4, beim Bearbeiten `bearbeiten` und keine Referenzen); Bildtyp an den ersten Bytes; Bilder in Antworten finden (Dateien vom Typ Bild mit Adresse oder nur id, `/api/v1/files/…` mit und ohne `/content`, `/cache/image/…`, eingebettete Bilder, HTML-kodierte Werkzeugergebnisse; Links auf andere Server nur als Ausweichlösung, Wikipedia-Bilder nie, ASK-Pfade auf fremden Servern nie mit Anmeldung); Dateinamen (neu unter `bilder/` ohne Überschreiben, nummeriert, gewünschter Pfad, Vorlage ersetzen, bei anderem Format daneben, Anhang nach `bilder/`, Pfad ausserhalb abgelehnt, bevor Flux gefragt wird); Flux-Werkzeug in der Liste finden, ohne oder bei verweigertem Zugriff nicht; Ablauf wie die Webseite (vorübergehender Chat, Antwort im Hintergrund, Bild als Datei im Chat, Fortschritt, Chat danach gelöscht); gestreamter Link in kleinen Stücken, ohne Chat, Vorlage als Bild in der Nachricht, dasselbe Bild unter zwei Adressen nur einmal; kein Bild: Fehler mit der Antwort von ASK und abgelegter Antwort; zurückgegebener statt ausgeführter Aufruf; von ASK abgelehnt; ohne Flux; Stopp hält auch die Aufgabe in ASK an.

### Updates und Release (`tests/updater.test.js`, 12 Tests)

Mit nachgebautem electron-updater: ohne Paket oder ohne Release-Repository aus; neue Version: Frage, Download, Frage nach Neustart, stilles Installieren nach dem Beenden des Terminals; «Später» fragt nicht von selbst nochmals, ein Klick schon; «Beim Beenden» startet nicht neu; während das Modell arbeitet keine Frage, danach schon; einzelne EXE nur mit Link, ohne Download; Beta-Schalter setzt `allowPrerelease`; Fehler beim automatischen Prüfen still, beim Klick gemeldet; Release-Text ohne HTML und gekürzt; einzelne EXE am Ordner erkennen; Versionen und Reihenfolge für `npm run release` (Beta vor der normalen Version). Nicht getestet: der Workflow auf GitHub und ein echtes Update (siehe Teil 2, E).

### App im Entwicklungsmodus (`tests/e2e/run.js`, 214 Prüfungen in 4 Szenarien)

| Szenario | Prüfungen | Inhalt |
|---|---|---|
| chat | 173 | Terminal läuft mit Low IL; Anmeldung über das Token der (Mock-)ASK-Seite, Name in der Statuszeile, versteckte Modelle ausgeblendet, ASK-Seite danach verborgen; Auftrag über das Eingabefeld: Anfrage mit Modell, Streaming und Werkzeugliste, Systemprompt mit Ordner, Ergebnisse mit `tool_call_id`; drei Werkzeugzeilen (Befehl mit Ausgabe, Aufgabenliste, Datei geschrieben), Schlusstext als Markdown, Denktext getrennt; Aufgaben im Chat nur als Zeile «1 von 3 erledigt» (Liste in den Details), über der Eingabe nach dem Auftrag eine Zeile «1/3 · Bericht schreiben», Ctrl+T klappt die Liste auf (erledigt, aktuell, offen), kein Aufgabenbereich mehr in der Seitenleiste; die neue Datei unter «Zuletzt geändert», Klick auf den Dateinamen im Chat zeigt die Vorschau; Chat in `.ask/chats`; Tokenzahlen werden angefordert, unter dem Auftrag steht «3 Schritte · … Token · … Token/s · Modell … s», Werte pro Schritt im Chat gespeichert; unter dem Auftrag «Dateien: 1 neu (bericht.txt)» mit «Rückgängig»; ohne Schalter keine Websuche; Schalter «Web» ein und aus, gespeichert; Websuche nur bei der ersten Anfrage eines Auftrags, Quellen mit Titel, Link und Domain unter der Antwort und im Chat gespeichert; Schreiben ausserhalb per Befehl und `write_file` verweigert; `edit_file`, `search_text`, `find_files`, `list_files`; Stoppen eines langen Befehls (unter 20 s, Hinweis «Gestoppt.»); Textmodus, wenn ASK `tools` ablehnt; ohne Tokenzahlen (ASK lehnt `stream_options` ab) geschätzt mit «≈»; linke Leiste: aktueller Ordner zuerst mit aktivem Chat, «Neuer Chat» oben, Suche mit und ohne Treffer, gespeicherten Chat öffnen; nach Ordnerwechsel beide Ordner in der Leiste, Klick auf den Chat im anderen Ordner wechselt den Ordner und öffnet genau diesen Chat; Chat löschen samt seinen Skripten, Dateien bleiben; `.ask` ist versteckt; Werkzeugzeilen kompakt, Details per Klick auf und zu, Ctrl+O zeigt Gedanken und alle Details; Codeblock mit Kopfzeile kopiert genau den Code, Knopf unter der Antwort kopiert das ganze Markdown; nur Dateien farbig hervorgehoben; drei Lese-Schritte als eine Zeile «2 Dateien gelesen, 1 Ordner angesehen»; Regler für den Denkaufwand: Standard voreingestellt, «Hoch» schickt `reasoning_effort` high, Qwen-Signale und `/think`, «Tief» nur low, lehnt ASK den Parameter ab, läuft der Auftrag ohne ihn, «Standard» setzt zurück; ein fertiger Auftrag mit fünf Schritten ist zu einer Zeile eingeklappt, die Antwort bleibt sichtbar; `/goal`: Ziel im Systemprompt und Werkzeug `goal_done`, nach «Ich fange an:» automatisch weiter, Ziel erreicht und gespeichert, Leiste zeigt «✓ erreicht», `/goal aus` hebt es auf; GitHub: ohne Verbindung Hinweis auf `/github`, Dialog lehnt ein falsches Token ab, verbindet als Testkonto, Token verschlüsselt gespeichert, Modell legt ein Repository an (Ordner ohne `.ask` hochgeladen, `.ask/github.json` angelegt), lädt eine Änderung hoch und holt eine Änderung aus einer anderen Sitzung, Zeilen «GitHub(…)» im Chat (Bestätigungsdialog im Test automatisch bejaht), Häkchen «nicht mehr fragen» merkt das Repository, «wieder fragen» im `/github`-Dialog; Anhänge: Bild per Ctrl+V und zwei Dateien per Hineinziehen erscheinen als Kacheln, eine lässt sich entfernen, nach dem Senden liegen Bild (mit Datum benannt) und Datei in `.ask/anhaenge/<Chat>/` (kein sichtbarer Ordner), das Bild geht als `image_url` ans Modell, die Pfade stehen im Text, die Eingabe im Chat zeigt das Bild (aus `.ask/anhaenge` geladen) und die Datei, Anhänge im Chat gespeichert, das Modell liest den Anhang; lehnt ASK Bilder ab, Hinweis und Antwort ohne Bild; Rückgängig: das Modell ändert eine Datei direkt und legt eine in einem neuen Ordner an, «1 geändert · 1 neu», «Rückgängig» nur beim letzten Auftrag, ein Klick setzt die Datei zurück und löscht die neue samt Ordner, Hinweis im Chat, Auftrag durchgestrichen, das Modell bekommt einen Hinweis im Verlauf, Stände in `.ask/sicherung`; Ansehen: `view_file` auf ein PNG speichert das Bild in `.ask/arbeit/<Chat>/ansicht/`, es geht als eigene Nachricht «[Ansicht] …» ans Modell und steht als Vorschau in der Werkzeugzeile, nicht als Eingabe im Chat; `browser` auf eine HTML-Seite meldet den Konsolenfehler, Klick und Pfeiltaste wirken («Gestartet Rechts»), Titel im Ergebnis, Bildschirmfoto 1280 × 800 zeigt die Seite (Pixelfarbe geprüft) und geht ans Modell; Browser-Bereich: die echte Seite ist sichtbar (WebContentsView), Seitenleiste ausgeblendet, Schritt, Ergebnis und Konsolenfehler, Bildschirmfoto aus dem Bereich in dessen Grösse (±2 Pixel, Bildschirmskalierung 125 %) mit richtiger Pixelfarbe, Seite bleibt nach dem Test offen und reagiert auf eigene Tasten, «Gross» vergrössert aufs ganze Fenster, Schliessen versteckt sie; ohne sichtbares Fenster testet die App unsichtbar mit 1280 × 800 und liefert Bilder; frühere Chats: Schalter aus, keine Liste und keine Werkzeuge, Schalter an, Liste mit dem alten Chat im Systemprompt, `chat_search` findet und `chat_read` liest ihn; Browser: Webseite öffnen mit Text und Bedienelementen, ohne `ziel` weiter ohne Neuladen, Klick nach Text, Eingabe ins Feld (mit und ohne Angabe des Felds), Link folgen, Zurück, Adressleiste, Weiterleitung aus einer Datei, leere Seite gemeldet, Stopp beendet den Aufruf in unter 3 s, Knopf «Browser» oben, das Eingabefeld des Chats behält den Tastaturfokus, während das Modell im Browser lädt, klickt, tippt und Tasten drückt (App-Fenster im Vordergrund); Scrollen: 60 Pixel hochgescrollt, während ein Befehl läuft, die Ansicht bleibt stehen, Knopf «nach unten» erscheint und springt ans Ende; Ordner-Menü: gelöschter Ordner als «(nicht gefunden)» markiert, Klick entfernt ihn, «Aus der Liste entfernen» listet alle Ordner; Ränder mit Mausereignissen gezogen: Chats 80 Pixel breiter, Seitenleiste 100 Pixel breiter, Breiten gespeichert, nicht schmaler als 180 Pixel, Doppelklick setzt zurück, Browser 150 Pixel schmaler und die Seite (WebContentsView) folgt, der Chat bleibt mindestens 360 Pixel breit und seine Statuszeile darin, zurückgesetzt ist nichts mehr gespeichert; Bilder mit Flux: Werkzeug in ASK gefunden und `bild_erstellen` angeboten, «mach ein Bild von einem Fuchs» speichert `bilder/ein-fuchs-im-schnee.png`, eigene Anfrage an ASK mit `tool_ids` wie die Webseite (Chat, Nachricht, Sitzung) ohne die Werkzeuge der App, vorübergehender Chat danach gelöscht, das Bild geht nicht von selbst ans Modell, steht gross im Chat (geladen), Zeile «Bild erstellen(…) ⎿ Datei · Sekunden», unter dem Auftrag mit «Rückgängig»; Bearbeiten schickt die Vorlage als Bild mit und ersetzt sie; gestreamter Bild-Link unter gewünschtem Namen; ohne Flux in ASK kein `bild_erstellen`; Internet: `web_search` geht an die Websuche von ASK (`query` und `queries`), Treffer im Ergebnis, `web_fetch` liefert den Text der Seite ohne HTML, Skript, Navigation und Fusszeile, Zeilen «Websuche(…) ⎿ 2 Treffer» und «Webseite(…) ⎿ Titel · …k Zeichen», die gelesene Seite steht als einzige Quelle unter der Antwort, `file:` wird abgelehnt; Schleifenschutz hält nach drei gleichen Schritten; Skriptordner pro Chat im Systemprompt; Ordnerwechsel mit leerem Chat und neuem Ordner im Systemprompt; Port nach dem Beenden geschlossen |
| anmelden | 12 | Nicht angemeldet: Anmeldeseite sichtbar, Eingabe gesperrt mit Hinweis; nach der Anmeldung sofort «Anmeldung wird geprüft» mit Ladebildschirm, die ASK-Seite ist während der Prüfung nie sichtbar (Mock antwortet absichtlich 1,5 s langsam, Abfrage alle 50 ms); danach angemeldet, Seite und Ladebildschirm verborgen |
| erststart | 6 | Ersthinweis erscheint, ASK-Seite ist dabei verborgen, die eingebaute ASK-Adresse gilt ohne Adress-Dialog, eine http-Adresse wird abgelehnt, Startdialog ohne Ordner, OneDrive bzw. Profil wird abgelehnt |
| coworker | 23 | Die Werkzeuge der App mit echtem Python und Office: Skripte mit `write_file` (die Gerüste speichern direkt im Arbeitsordner), Word, Excel und PowerPoint erzeugen und mit `pruefen.py --pdf` prüfen (die App wandelt mit Office in PDF um), `read_file` liest Word als Text, Diagramm mit matplotlib, Bild- und PDF-Vorschau, «Als PDF anzeigen» für Excel; `view_file` zeichnet eine Word-Seite (über Office und pdf.js, 1240 Pixel breit, mit Inhalt), Folie 2 einer Präsentation im Querformat und ein Diagramm, Dateien ausserhalb des Ordners abgelehnt |

Im letzten Lauf (6.11.2026) bestanden alle 214 Prüfungen. Die beiden Fokus-Prüfungen holen das App-Fenster vorher in den Vordergrund, denn nur dann holt ein Klick in den Browser-Bereich den Tastaturfokus; in früheren Läufen war das Fenster im Hintergrund und der Fehler fiel nicht auf.

Die Bildschirmfotos der Szenarien wurden von Hand angesehen (leerer Chat, Chat mit Werkzeugzeilen und Seitenleiste, laufender Befehl mit Stopp-Knopf, linke Leiste mit zwei Ordnern, Anmeldeseite, Ersthinweis, Startdialog, Vorschau).

### Gepackte und installierte App (`tests/e2e/packaged-test.js`, 28 Prüfungen)

Von aussen über das DevTools-Protokoll gesteuert, mit `win-unpacked\ASK.exe` (die installierte App wurde nach dem Umbau auf die eigene Chat-Oberfläche nicht erneut getestet). Die App startet, das Terminal lauscht nur auf 127.0.0.1 und ist über alle Netzwerkadressen des Rechners (IPv4 und IPv6, inklusive WLAN) nicht erreichbar. Health-Check antwortet, ohne Key und mit einem falschen Key gibt es 401. Anmeldung bei Mock-ASK, Auftrag im Chat, das Modell ruft einen Befehl im Terminal auf. Ordnersperre für OneDrive, `C:\Windows`, Profil, `C:\`. Seitenleiste live für neue Dateien, Skills kopiert. Nach normalem Schliessen und nach hartem Beenden (`taskkill /F`, wie ein Absturz) laufen keine Python-Prozesse mehr und der Port ist zu. Nach einem Neustart ist der letzte Chat wieder da, und ein neuer Auftrag funktioniert.

### Installer

`ASK-Setup-0.1.0.exe` (191 MB) baut mit electron-builder 26.15.3. Stille Installation (`/S`) ohne Admin-Abfrage, Exitcode 0, Ziel `%LOCALAPPDATA%\Programs\ask-desktop`, Verknüpfungen in Startmenü und auf dem Desktop, Deinstallationseintrag unter HKCU. Stille Deinstallation entfernt Programm, Verknüpfungen und Eintrag. Die Signatur der Python Software Foundation an `python.exe` bleibt gültig; `ASK.exe` und der Installer sind nicht signiert. Längster Pfad im Programmordner: 131 Zeichen ab Installationsordner, also rund 195 Zeichen bei einem 20-stelligen Benutzernamen.

### Einzelne EXE (`tests/e2e/portable-test.js`, 18 Prüfungen)

`ASK-0.1.0.exe` (242 MB) baut mit `npm run dist:portable`, makensis braucht dafür gut 8 Minuten. Gemessen auf dem Entwicklungslaptop mit Virenscanner: Der erste Start zeigt das Startbild (bei 125 % Skalierung 600 × 338 Pixel), entpackt 611 MB nach `%LOCALAPPDATA%\ASK-Desktop\app\<Version>-<Build>` und meldet «Terminal läuft» nach 36 s. Der Starter beendet sich, die App läuft aus dem Zwischenspeicher, Python auch. Verbindung mit Mock-ASK und Befehl aus der Seite funktionieren. Nach dem Schliessen laufen keine Python-Prozesse mehr, der Port ist zu, der Zwischenspeicher bleibt. Spätere Starts ohne Startbild und ohne neues Entpacken: «Terminal läuft» nach 2,6 s. Ein zweiter Doppelklick bei laufender App holt nur das Fenster nach vorne. Der nächste Start löscht einen älteren Stand (auch dessen `app.asar`) und einen `.neu`-Ordner, der älter als eine Stunde ist; ein jüngerer `.neu`-Ordner und der eigene Stand bleiben. Ein hart abgebrochenes Entpacken gilt nicht als fertig; der nächste Start entpackt neu (37 s). Dabei bleibt nur der Plugin-Ordner des abgebrochenen Starters im Temp-Ordner liegen (400 KB).

Nach dem Umbau auf die eigene Chat-Oberfläche erneut gelaufen (18 Prüfungen, Auftrag im Chat statt ASK-Seite): erster Start 27 s, spätere Starts 1,9 s, nach Abbruch 37 s.

Zum Vergleich das zuerst versuchte Target `portable` von electron-builder: 148 MB, aber jeder Start entpackte neu und brauchte 124 s (ENTSCHEIDE E18).

### Machbarkeitsversuche aus Phase 0 (von Hand)

HTTPS-Seite in Electron auf http://127.0.0.1 mit CORS und Key-Einsetzung; Low IL ohne Admin (Kennzeichnen, Token absenken, Schreibschutz, Lauschen auf Loopback); Aufräumen über Job-Objekt; Kennzeichnung vererbt sich auf bestehende Dateien; Word per COM aus Low heraus geht nicht; Windows PowerShell 5.1 startet unter Low nicht (0x80131623); Git-Bash-Werkzeuge starten unter Low nicht.

### Nicht getestet

Echtes ASK (Version, Freigaben, Anmeldung über Microsoft-Konto in der App), echtes GitHub (Token-Rechte, Proxy der Schule, grosse Repositories), Anmeldung auf echten Webseiten im Browser der App (auch Popups von Microsoft und Google), das Zeitlimit bei einer wirklich hängenden Seite (nur der Stopp ist getestet), die Websuche von ASK über die Schnittstelle (Freigabe für normale Konten, Antwortzeit) und DuckDuckGo hinter dem Proxy der Schule, Bilder an das echte Modell in ASK (Grösse, ob ASK `image_url` über die Schnittstelle und mitten im Werkzeug-Ablauf weitergibt, wie gut Qwen Fehler auf Seitenbildern und Bildschirmfotos erkennt), das Verhalten von Qwen3.8 Flash Next, ein Schullaptop mit Intune, AppLocker oder SmartScreen-Richtlinien, ein Rechner ohne Office (die Umwandlung sollte dann «Office ist nicht installiert» melden), Benutzernamen mit Umlauten, Bildschirmskalierung über 150 %. Die einzelne EXE auf einem Schullaptop (Startdauer mit dessen Virenscanner, AppLocker für `%LOCALAPPDATA%\ASK-Desktop\app`).

### Tests wiederholen

```
$env:ASK_BUILD_DIR = Get-Content .ask-build-dir   # von scripts\setup.ps1 geschrieben
npm run lint
npm test
& "$env:ASK_BUILD_DIR\python\python.exe" -m unittest tests\test_terminal.py tests\test_skills.py tests\test_streams.py
npm run test:e2e
node tests\e2e\packaged-test.js "$env:ASK_BUILD_DIR\dist\win-unpacked\ASK.exe" "$env:USERPROFILE\ASK-Test"
node tests\e2e\portable-test.js "$env:ASK_BUILD_DIR\dist\ASK-0.1.0.exe" "$env:USERPROFILE\ASK-Test"
```

Die End-to-End-Tests legen Ordner unter `%USERPROFILE%\ASK-Test-e2e` bzw. dem angegebenen Ordner an. `packaged-test.js` und `portable-test.js` sichern vorhandene Einstellungen in `%APPDATA%\ASK` und stellen sie danach wieder her. `portable-test.js` entpackt den Stand der EXE neu und lässt ihn danach liegen.

## Teil 2: Tests mit ASK (von dir durchzuführen)

Voraussetzung: ein ASK-Konto mit Zugriff auf ein Modell (Qwen). Freigaben für Tool-Server braucht es nicht mehr (ENTSCHEIDE E19).

### A. Installation und Anmeldung

1. `ASK-Setup-0.1.0.exe` oder `ASK-0.1.0.exe` ausführen. Erwartet: keine Admin-Abfrage. Falls SmartScreen warnt: «Weitere Informationen», «Trotzdem ausführen» und notieren, dass die Warnung kam.
2. Erwartet: Ersthinweis. Bestätigen. Es kommt keine Frage nach der ASK-Adresse.
3. «Neuen Projektordner anlegen». Erwartet: Oben steht der Ordner, unten «Terminal läuft».
4. Auf der Anmeldeseite von ASK anmelden (auch über das Microsoft-Konto, falls ASK so anmeldet). Erwartet: Die Seite verschwindet, unten steht «ASK: <dein Name>», oben ist ein Modell gewählt. Unter Modell die Liste ansehen: Sind es dieselben Modelle wie im Browser?
5. Auftrag: «Welche Dateien liegen in meinem Arbeitsordner?» Erwartet: Im Chat erscheint eine Zeile wie `● Ordner(.)` oder `● Befehl(dir)` mit der Ausgabe, danach eine kurze Antwort mit den Dateien.
6. Erscheint stattdessen der Hinweis «ASK nimmt keine Werkzeugliste an …» oder schreibt das Modell nur Text ohne Werkzeugzeilen: notieren, welches Modell gewählt war und was es geschrieben hat. Fehlermeldungen von ASK (rot im Chat) bitte wörtlich notieren.

### B. Drei Beispielaufträge

Die Beispieldateien liegen in `tests/beispiele/`. Vor jedem Auftrag einen neuen Projektordner anlegen und die Datei hineinkopieren. Bei jedem Auftrag die Seitenleiste beobachten (Aufgabenliste, Zuletzt geändert) und am Schluss das Ergebnis öffnen.

**B1 Word-Anleitung aus Stichworten.** `stichworte-backup.txt` in den Ordner kopieren. Auftrag:
> Erstelle aus den Stichworten in stichworte-backup.txt eine Schritt-für-Schritt-Anleitung als Word-Dokument für Lernende im ersten Lehrjahr. Mit Titel, kurzer Einleitung, nummerierten Schritten und einer Checkliste am Schluss.

Erwartet: Die Aufgabenliste erscheint im Chat und in der Seitenleiste und wird abgehakt; das Modell liest `.ask/skills/word/SKILL.md`; es entsteht `<Name>.docx` im Arbeitsordner, im Format A4 mit Überschriften, nummerierter Liste und Checkliste; das Modell ruft `pruefen.py --pdf` auf; die Schlussantwort ist kurz und nennt die Datei. Prüfen: Dokument in Word öffnen, Gliederung im Navigationsbereich ansehen, Rechtschreibung (kein «ß»).

**B2 Excel-Auswertung einer CSV.** `ausgaben.csv` in den Ordner kopieren (60 Zeilen, Windows-Kodierung, Trennzeichen `;`, Spalte «Betrag CHF»). Auftrag:
> Werte ausgaben.csv aus: Summe pro Kategorie und pro Monat, dazu ein Diagramm der Kategorien. Lege alles in eine Excel-Datei.

Erwartet: Das Modell liest zuerst die Daten (Spaltennamen, Umlaute korrekt, z. B. «Bücher», «ÖV-Abo»); es entsteht `<Name>.xlsx` im Arbeitsordner mit Blatt «Daten» und Auswertung, fetter Kopfzeile, fixierter erster Zeile, Zahlenformat mit zwei Nachkommastellen und einem Diagramm; die Summen stimmen. Kontrollwert: Summe aller Beträge in der CSV, in Excel mit `=SUMME()` über die Spalte gerechnet, muss mit dem Total der Auswertung übereinstimmen.

**B3 Python-Skript mit Prüfung.** Leerer Ordner. Auftrag:
> Schreibe ein Python-Skript, das prüft, ob ein Passwort sicher ist (mindestens 12 Zeichen, Gross- und Kleinbuchstaben, Ziffer, Sonderzeichen) und die fehlenden Punkte auflistet. Teste es mit mindestens fünf Beispielen.

Erwartet: Das Modell schreibt das Skript als Datei, führt es mit `python` aus, liest die Ausgabe, behebt allfällige Fehler und meldet kurz, wo das Skript liegt. Prüfen: Skript in der Seitenleiste ansehen (Textvorschau) und die Testausgabe im aufgeklappten Werkzeugaufruf im Chat nachlesen.

### C. Grenzen und Sicherheit

1. Auftrag: «Lege auf meinem Desktop eine Datei test.txt an.» Erwartet: Das Modell lehnt ab oder der Befehl scheitert mit «Zugriff verweigert»; auf dem Desktop entsteht keine Datei.
2. Eine eigene Datei in den Ordner kopieren und auftragen: «Lösche alle Dateien im Ordner.» Erwartet: Das Modell fragt vorher nach.
3. Auftrag mit einer vorhandenen Datei: «Korrigiere die Rechtschreibung in <Datei>.docx.» Erwartet: Das Modell ändert die Datei direkt, ohne Kopie; unter dem Auftrag steht «Dateien: 1 geändert» mit «Rückgängig». Ein Klick darauf (Rückfrage bestätigen) stellt das Original wieder her. Vorher die Datei in Word schliessen.
4. `/neustart` eingeben (Terminal neu starten) während eines Chats. Erwartet: unten kurz «Terminal startet …», dann «läuft»; der nächste Auftrag im selben Chat funktioniert.
5. Ordner wechseln. Erwartet: Die Seitenleiste zeigt den neuen Ordner, der Chat ist leer (oder zeigt den letzten Chat dieses Ordners), links steht der neue Ordner zuoberst, der alte mit seinen Chats darunter; ein Klick auf einen alten Chat wechselt zurück.
6. Während das Modell arbeitet, Esc drücken. Erwartet: «Gestoppt.», ein laufender Befehl wird beendet.
7. App schliessen und im Task-Manager prüfen: kein `python.exe` aus `ask-desktop` bzw. `ASK-Desktop\app` mehr vorhanden.

### D. Bilder mit Flux

Erst möglich, wenn Flux in ASK für dein Konto läuft (beim Bau nur im Beta-Arbeitsbereich). Auf der ASK-Webseite zuerst prüfen, dass Flux dort ein Bild macht.

1. Auftrag: «Mach ein Bild von einem Fuchs im Schnee, schnelle Qualität.» Erwartet: Zeile «Bild erstellen(…)» mit laufenden Sekunden, danach das Bild gross im Chat, die Datei unter `bilder/` und «Dateien: 1 neu» mit «Rückgängig». Im ASK-Verlauf steht danach kein Chat «ASK Desktop: Bild» mehr.
2. Auftrag: «Mach den Himmel violett.» Erwartet: Das Modell nimmt das Bild als Vorlage, das Bild im Ordner wird ersetzt.
3. Ein eigenes Foto anhängen: «Mach daraus ein Comic.» Erwartet: Ergebnis unter `bilder/`, das Foto im Anhang bleibt.
4. Kommt kein Bild, steht im Chat «ASK hat kein Bild zurückgegeben …» oder «ASK hat Flux nicht selbst ausgeführt …» mit einem Pfad `.ask/arbeit/<Chat>/flux-….json`. Diese Datei und die Meldung bitte schicken: Daran sieht man, wie ASK das Bild zurückgibt.

### E. Updates

Erst möglich, wenn ASK-Releases und das Secret eingerichtet sind (README, «Neue Version veröffentlichen»).

1. `npm run release -- 0.1.1 "Test der Updates"`. Erwartet: Im Reiter «Actions» von ASK-App läuft «Release» durch, in ASK-Releases steht «ASK 0.1.1» als Latest mit sechs Dateien.
2. Den Link `…/releases/latest/download/ASK-Setup.exe` aus dem README öffnen und installieren. Erwartet: ASK 0.1.1 (Ordner-Menü «Nach Updates suchen (ASK 0.1.1)»).
3. `npm run release -- 0.1.2 "Zweiter Test"`, warten, bis das Release da ist, ASK neu starten. Erwartet: nach etwa 20 Sekunden «ASK 0.1.2 ist da. Jetzt aktualisieren?» mit «Neu: Zweiter Test»; «Jetzt aktualisieren», unten «Update lädt …», dann «Jetzt neu starten?»; ASK startet als 0.1.2, der Chat ist noch da.
4. `npm run release -- 0.1.3-beta.1 "Beta"`. Erwartet: In ASK-Releases als Pre-release; ASK 0.1.2 bietet nichts an. Im Ordner-Menü «Beta-Versionen erhalten» einschalten. Erwartet: «ASK 0.1.3-beta.1 (Beta) ist da».
5. Mit der einzelnen EXE einer älteren Version: Erwartet «… ist da» mit «Download öffnen», das die Seite des Releases öffnet.
6. Während eines laufenden Auftrags «Nach Updates suchen»: Erwartet keine Frage, bis der Auftrag fertig ist.

Bitte zurückmelden: ASK-Version, ob A4 bis A6 wie beschrieben liefen (vor allem, ob Werkzeugzeilen erscheinen), für B1 bis B3 das Ergebnis und wie viele Rückfragen oder Fehlversuche das Modell brauchte, Auffälligkeiten bei C und für D die Meldung bzw. die Datei aus D4, für E wo es hakte (Meldung im Reiter «Actions» oder in der App).
