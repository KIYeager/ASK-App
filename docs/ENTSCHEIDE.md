# Entscheide

Entscheide, die ohne Rückfrage getroffen wurden, mit Begründung. Neueste unten.

## E1 Build-Ordner ausserhalb von OneDrive

Das Repo liegt in OneDrive. `node_modules`, das gebaute Python und der Installer umfassen Zehntausende Dateien und mehrere hundert MB, die sonst auf SharePoint synchronisiert würden. `scripts/setup.ps1` legt deshalb `node_modules`, `build` und `dist` als Junctions in einen lokalen Build-Ordner (`ASK_BUILD_DIR`, Standard `%LOCALAPPDATA%\ASK-Desktop-Build`). OneDrive folgt Junctions nicht. Im Repo selbst liegen nur Quellen und Dokumentation.

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
