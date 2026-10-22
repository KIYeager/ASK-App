# Entscheide

Entscheide, die ohne Rückfrage getroffen wurden, mit Begründung. Neueste unten.

## E1 Build-Ordner ausserhalb von OneDrive

Das Repo liegt in OneDrive. `node_modules`, das gebaute Python und der Installer umfassen Zehntausende Dateien und mehrere hundert MB, die sonst auf SharePoint synchronisiert würden. Alles Gebaute liegt deshalb in einem lokalen Build-Ordner (`ASK_BUILD_DIR`, Standard `%LOCALAPPDATA%\ASK-Desktop-Build`): Python unter `python`, der Installer unter `dist`. `scripts/setup.ps1` installiert die npm-Pakete mit `npm ci` in `deps` und legt im Projekt nur eine Junction `node_modules` an. OneDrive folgt Junctions nicht.

Korrektur während der Arbeit: Zuerst lag die Junction im Projekt und `npm install` lief im Projektordner. npm hat die Junction dabei durch einen echten Ordner ersetzt, sodass rund 450 MB `node_modules` etwa anderthalb Stunden im OneDrive-Ordner lagen, bis es auffiel. Der Ordner ist entfernt; darum gilt jetzt: `npm install` nie direkt im Projektordner ausführen, sondern `scripts/setup.ps1`. Besser wäre es, das Repo ganz ausserhalb von OneDrive abzulegen (siehe README, offene Punkte).

## E2 Anweisungen über den Systemprompt von open-terminal

Siehe MACHBARKEIT, Abschnitt 2. `/system` wirkt ab Open WebUI 0.8.11 und steht in der Systemnachricht; `AGENTS.md` wirkt für persönliche Terminals unter Windows erst ab 0.12.0. Die App schreibt deshalb keine `AGENTS.md` in den Arbeitsordner. Eine vorhandene `AGENTS.md` der Lernenden bleibt unangetastet und wirkt zusätzlich.

## E3 Skills als Kopie in `.ask/skills/`

Die Anleitungen werden beim Öffnen eines Ordners nach `<ordner>/.ask/skills/` kopiert (nur wenn sich die Version geändert hat). Das Modell findet sie so mit kurzen relativen Pfaden, und neuere ASK-Versionen sehen sie über `/skills` als echte Skills. `.ask/` ist ohnehin der Ordner der App (`tasks.json`).

## E4 Echter API-Key nur im Hauptprozess, Platzhalter in ASK

In den ASK-Einstellungen steht als Key nur `ask-desktop`. Der echte Key wird bei jedem Start neu erzeugt, über eine Pipe an das Startskript gegeben und von Electron in die Anfragen der ASK-Ansicht eingesetzt. Er liegt damit weder auf der Festplatte noch in ASK, in der Prozessliste oder in der Umgebung der Befehle. Ein Browser ausserhalb der App kennt den Key nicht und kommt nicht an das Terminal.

## E5 Fester Port pro Nutzer

«Freier Port» wird so umgesetzt: Beim ersten Start wählt die App zufällig einen freien Port zwischen 20000 und 39999 und merkt ihn sich. Ist er später belegt, nimmt sie einen neuen und aktualisiert die Verbindung in ASK. So bleibt die Verbindung in ASK meist unverändert, was vor allem beim manuellen Einrichten wichtig ist.

## E6 Low Integrity Level ist Pflicht

Kann der gewählte Ordner nicht mit Low gekennzeichnet werden (z. B. FAT-USB-Stick, fremder Besitzer), lehnt die App ihn ab, statt ohne Schreibschutz zu laufen. Für Lernende ist ein Ordner unter dem eigenen Benutzerprofil der Normalfall; dort klappt es.

## E7 Pipe-Runner statt ConPTY für Modellbefehle

Die ConPTY-Ausgabe enthält Steuersequenzen und hat im Test Ausgaben verschluckt. Für `POST /execute` nutzt die App den Pipe-Runner von open-terminal mit UTF-8. Das interaktive Terminal in ASK bleibt bei ConPTY, weil es dort um Bedienung durch Menschen geht.

## E8 Keine Umgehung fehlender ASK-Freigaben

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

Open WebUI holt Werkzeuge (`/openapi.json`) und Systemprompt (`/system`) eines persönlichen Terminals nur beim Laden der Seite. Lädt ASK schneller als das Terminal startet, fehlen dem Modell die Werkzeuge bis zum nächsten Neuladen. Die App lädt ASK deshalb erst, wenn das Terminal läuft (höchstens 20 s Wartezeit, bei einem Fehler sofort). Nach einem Ordnerwechsel lädt sie ASK neu, weil der Ordnerpfad im Systemprompt steht. Ein Neustart des Terminals im selben Ordner braucht kein Neuladen: Port und Platzhalter-Key bleiben gleich.

## E17 Installer pro Nutzer mit einem Klick

NSIS mit `oneClick`, `perMachine: false` und `allowElevation: false`: Installation nach `%LOCALAPPDATA%\Programs\ask-desktop` ohne Admin-Abfrage, Verknüpfungen in Startmenü und auf dem Desktop, Deinstallation über «Apps» in den Windows-Einstellungen. Einstellungen in `%APPDATA%\ASK` bleiben bei der Deinstallation erhalten. Mitgeliefert werden nur die Chromium-Sprachdateien Deutsch und Englisch.
