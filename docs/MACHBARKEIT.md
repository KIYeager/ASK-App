# Machbarkeit (Phase 0)

Stand 22.10.2026. Geprüft mit open-terminal 0.14.2 (PyPI), Open WebUI 0.12.3 (Quellcode, Stand 16.10.2026), Electron 44.6.0 (Chromium 152), Python 3.12.7, Windows 11 Pro 26200 mit Standardkonto ohne Admin-Rechte.

Kurzfassung: Das Vorhaben ist machbar. Die grösste offene Frage liegt nicht in der Technik auf dem Laptop, sondern in den Freigaben von ASK: Persönliche Terminal-Verbindungen sind in Open WebUI standardmässig ausgeschaltet und können nur von den ASK-Admins freigegeben werden (Abschnitt 3). Das muss mit dem Handtest in Abschnitt 6 geklärt werden.

## 1. open-terminal unter Windows

open-terminal läuft nativ unter Windows. Es ist eine FastAPI-Anwendung, die über uvicorn läuft; unter Windows kommt pywinpty (ConPTY) dazu.

**Shell.** Befehle des Modells (`POST /execute`) laufen als `%COMSPEC% /c <befehl>`, also in `cmd.exe`, in einem ConPTY. Das interaktive Terminal (WebSocket) startet ebenfalls `cmd.exe`. Ohne pywinpty fällt open-terminal auf einen Pipe-Runner zurück (`asyncio.create_subprocess_shell`, ebenfalls cmd.exe).

Zwei Probleme unter Windows sind im Test aufgefallen. Der eingebaute Systemprompt und die Beschreibung des Werkzeugs `run_command` nennen die Shell aus der Umgebungsvariable `SHELL`. Die ist unter Windows meist nicht gesetzt, dann steht dort `/bin/sh`, auf diesem Rechner sogar Git-Bash. Das Modell bekommt also eine falsche Shell genannt. Zudem enthält die ConPTY-Ausgabe Steuersequenzen (`\u001b[?9001h` usw.) und zerstückelt Zeilen; bei `python -c "print('äöü')"` ging die Ausgabe im Test ganz verloren. Die App setzt deshalb einen eigenen Systemprompt, setzt `SHELL=cmd.exe` und nutzt für Modellbefehle den Pipe-Runner mit UTF-8 (`PYTHONUTF8=1`, Codepage 65001).

**Einstellungen.** Reihenfolge: CLI-Option, Umgebungsvariable, `~/.config/open-terminal/config.toml`, Standardwert.

| Zweck | CLI | Umgebung | Standard |
|---|---|---|---|
| Host | `--host` | – | `0.0.0.0` (alle Netzwerke!) |
| Port | `--port` | – | 8000 |
| Arbeitsordner | `--cwd` | – | aktueller Ordner |
| API-Key | `--api-key` | `OPEN_TERMINAL_API_KEY`, `OPEN_TERMINAL_API_KEY_FILE` | zufällig, wird ausgegeben |
| CORS | `--cors-allowed-origins` | `OPEN_TERMINAL_CORS_ALLOWED_ORIGINS` | `*` |
| Systemprompt | – | `OPEN_TERMINAL_SYSTEM_PROMPT`, `OPEN_TERMINAL_ENABLE_SYSTEM_PROMPT` | Text mit Systeminfo |
| Zusatz zur Befehls-Beschreibung | – | `OPEN_TERMINAL_EXECUTE_DESCRIPTION` | leer |
| Logs | – | `OPEN_TERMINAL_LOG_DIR` | `~/.local/state/open-terminal/logs` |

Das Arbeitsverzeichnis ist das Prozessverzeichnis. Daraus leitet open-terminal sein «home» ab, und daran hängen Startordner für Befehle, Wurzel des Dateibrowsers in ASK, die Skills (`<home>/.agents/skills/…`) und die `AGENTS.md`, die Open WebUI liest.

**Keine Ordnersperre.** Die Datei-API von open-terminal schränkt Pfade nicht ein. Im Test liessen sich mit `..` und absoluten Pfaden Dateien ausserhalb des Arbeitsordners schreiben und `C:\Windows\win.ini` lesen. Eine Sperre muss die App selbst einbauen.

**API-Key.** Ohne Key verweigert der Server den Start, alle Endpunkte ausser `/health` und `/api/config` verlangen `Authorization: Bearer <key>` (getestet: 401 ohne Key). Die CLI legt den Key in die Prozessumgebung, von wo ihn jeder Befehl des Modells erben würde; als `--api-key` stünde er zudem in der Prozessliste. Die App startet open-terminal deshalb über ein eigenes Startskript, übergibt den Key über eine Pipe und entfernt ihn nach dem Import wieder aus der Umgebung.

**CORS.** open-terminal registriert Starlettes `CORSMiddleware` mit den konfigurierten Origins, `allow_credentials=True` und allen Methoden und Headern. In der mitgelieferten Starlette 1.9 wird ein Preflight mit `Access-Control-Request-Private-Network: true` mit 400 «Disallowed CORS private-network» abgelehnt, weil open-terminal `allow_private_network` nicht setzt. Das Startskript der App setzt diese Option.

## 2. Anweisungen: wie Open WebUI sie einbindet

Für persönliche Terminals holt der Browser beim Verbinden `/openapi.json` (daraus werden die Werkzeuge), dann `/api/config` und `/system`. Den Text aus `/system` hängt Open WebUI an die Systemnachricht jedes Chats mit diesem Terminal. Das gibt es seit Open WebUI 0.8.11.

Seit 0.11.4 liest Open WebUI zusätzlich bei jeder Nachricht `<home>/AGENTS.md` (höchstens 32 KB) und fügt sie als Nutzernachricht vor der ersten Nachricht ein. Für persönliche Terminals auf Windows funktioniert das erst ab 0.12.0 (vorher wurde der Windows-Pfad nicht erkannt bzw. nur Admin-Terminals berücksichtigt). Ebenfalls ab 0.12.0 meldet Open WebUI die Skills eines persönlichen Terminals (`GET /skills`, Ordner `<home>/.agents/skills/<name>/SKILL.md` mit `name` und `description` im Kopf) dem Modell als Liste, sofern native Werkzeugnutzung aktiv ist.

Entscheid für die App: Die Kernanweisungen gehen über `/system`. Das funktioniert ab 0.8.11, braucht keine Datei im Arbeitsordner und landet in der Systemnachricht, die ein schwächeres Modell eher beachtet als eine eingeschobene Nutzernachricht. Die Anleitungen (Skills) liegen als Kopie in `.ask/skills/` im Arbeitsordner, damit das Modell sie mit kurzen relativen Pfaden lesen kann; das Startskript meldet diesen Ordner zusätzlich über `/skills`, damit neuere ASK-Versionen sie als echte Skills anzeigen. Eine eigene `AGENTS.md` der Lernenden im Arbeitsordner wirkt ab 0.12.0 weiterhin zusätzlich.

## 3. Verbindung mit ASK automatisch setzen

Open WebUI speichert persönliche Terminals in den Nutzereinstellungen unter `ui.terminalServers` als Liste von Einträgen `{url, key, name, enabled, auth_type, path, config}`. Geschrieben wird mit `POST /api/v1/users/user/settings/update` und `{"ui": {"terminalServers": [...]}}`; das Backend übernimmt `ui` feldweise, andere Einstellungen bleiben unberührt. Welches Terminal im Chat ausgewählt ist, steht im Browser unter `localStorage.selectedTerminalId` (die URL). Die App kann beides über die angemeldete ASK-Seite setzen, mit dem Token, das ASK selbst in `localStorage.token` hält. Das ist technisch dasselbe, was die Einstellungsseite tut.

Die Einschränkung liegt in den Freigaben, die nur ASK-Admins setzen können:

| Voraussetzung | Standard in Open WebUI |
|---|---|
| Tool-Server global an (`ENABLE_TOOL_SERVERS`) | an |
| Direkte Integrationen (`ENABLE_DIRECT_INTEGRATIONS`) | **aus** |
| Nutzerberechtigung «Direkte Tool-Server» (`features.direct_tool_servers`) | **aus** |
| Modell-Fähigkeit «Terminal» beim Qwen-Modell | an |

Ohne die zweite und dritte Freigabe zeigt ASK den Reiter «Integrationen» gar nicht an. Das Backend prüft beim Speichern von `terminalServers` die Berechtigung zwar nicht, und das Frontend lädt gespeicherte Terminals auch ohne den Reiter. Die App nutzt diese Lücke bewusst nicht: Wenn die Freigabe fehlt, wäre das Setzen per API eine Umgehung eines Admin-Entscheids. Die App verbindet deshalb nur automatisch, wenn `/api/config` und `/api/v1/auths/` die Freigabe bestätigen. Sonst zeigt sie einen Dialog mit Erklärung und den Werten zum Kopieren.

Zum API-Key: In ASK wird nur ein Platzhalter gespeichert. Den echten Key pro Sitzung kennt nur der Electron-Hauptprozess; er setzt ihn in jede Anfrage der ASK-Ansicht an den lokalen Port ein (`session.webRequest.onBeforeSendHeaders`, für WebSocket-Verbindungen ebenso). Im Test hat das Ersetzen funktioniert. Weil der Platzhalter gleich bleibt und die App pro Nutzer einen festen Port wählt (nur bei Belegung einen neuen), muss die Verbindung in ASK nur einmal eingerichtet werden, auch im manuellen Fall.

## 4. Zugriff einer HTTPS-Seite auf http://127.0.0.1

Getestet in Electron 44 (Chromium 152) mit einer öffentlichen HTTPS-Seite (`https://example.com`), die `fetch` an `http://127.0.0.1:<port>` schickt, mit Authorization-Header und JSON-POST (also mit Preflight).

Mixed Content ist kein Problem: `127.0.0.1` und `localhost` gelten als vertrauenswürdige Ursprünge und werden von HTTPS-Seiten aus nicht blockiert. Der CORS-Preflight ging durch, sobald der ASK-Origin erlaubt ist. Chromium 152 schickt keinen Private-Network-Preflight mehr; diese Prüfung (Private Network Access) wurde durch «Local Network Access» ersetzt, eine Berechtigungsabfrage, die Chrome seit Version 142 zeigt, wenn eine öffentliche Seite auf das lokale Netz oder Loopback zugreift. In Electron wurde im Test keine solche Abfrage ausgelöst, der Zugriff klappte direkt. Die App beantwortet Berechtigungsanfragen trotzdem selbst und erlaubt sie nur für den ASK-Origin. `allow_private_network` bleibt gesetzt, für Browser, die den alten Preflight noch senden.

Im normalen Chrome oder Edge (Handtest in Abschnitt 6) erscheint beim ersten Zugriff eine Abfrage zum Zugriff auf Geräte bzw. Dienste auf diesem Gerät. Sie muss erlaubt werden. Wer sie ablehnt, wird nicht erneut gefragt und muss es in den Website-Einstellungen von ASK ändern.

## 5. Isolation

Kriterien: läuft ohne Admin-Rechte, nur der gewählte Ordner ist sichtbar, Aufwand für Lernende.

| Variante | Ohne Admin | Was das Modell sieht | Aufwand |
|---|---|---|---|
| Nativ, nur Ordnersperre in der Datei-API | ja | Datei-API nur Ordner; über die Shell alles, was der Nutzer sieht, mit Schreibrecht | keiner |
| Nativ, Ordnersperre plus Low Integrity Level | ja, getestet | Lesen wie oben; **Schreiben und Löschen nur im Arbeitsordner**, von Windows erzwungen | keiner |
| AppContainer | ja | nur freigegebene Ordner, auch beim Lesen | keiner für Lernende, für die App hoch (siehe unten) |
| Windows Sandbox | nein, Feature muss ein Admin aktivieren; nur Pro/Education/Enterprise | nur gemappter Ordner | Start 10–20 s, eine Instanz, eigene IP; Verbindung vom Browser umständlich |
| WSL2 | nein, Installation braucht Admin | standardmässig alle Laufwerke unter `/mnt/c` (abschaltbar) | Linux-Distribution pro Nutzer, mehrere GB |
| Docker Desktop | nein, Installation braucht Admin | nur gemounteter Ordner | gross; Lizenzfrage für Organisationen |

Auf dem Testrechner sind weder Windows Sandbox noch WSL noch Docker vorhanden; das dürfte für Schullaptops typisch sein.

**Low Integrity Level** ist der Mechanismus, mit dem Windows z. B. den alten Internet Explorer im geschützten Modus eingeschränkt hat. Ein Prozess mit niedriger Integritätsstufe darf in Ordner mit normaler Stufe nicht schreiben und dort nichts löschen. Getestet ohne Admin-Rechte: Der Nutzer kann den eigenen Arbeitsordner mit `icacls <ordner> /setintegritylevel (OI)(CI)L` kennzeichnen. Ein Startprozess erzeugt open-terminal angehalten, senkt dessen Token auf Low (`SetTokenInformation`), weist ihn einem Job-Objekt zu und lässt ihn laufen. Ergebnis: Der ganze Server samt aller Befehle läuft mit Low; Schreiben im Arbeitsordner geht, Schreiben in andere Ordner, ins Benutzerprofil und das Löschen fremder Dateien verweigert Windows mit «Zugriff verweigert», auch wenn open-terminal selbst nichts prüft. Lauschen auf 127.0.0.1 funktioniert, Python mit python-docx und openpyxl läuft.

Grenzen von Low IL: Lesen bleibt überall erlaubt, das Modell kann also über die Shell z. B. Dokumente ausserhalb des Ordners lesen und hat Internetzugang. Werkzeuge auf msys-Basis (z. B. die Unix-Werkzeuge von Git for Windows) starten nicht (getestet). Office-Automatisierung über COM (Word nach PDF) ist von Low aus voraussichtlich blockiert. Temporäre Ordner, Logs und Python-Benutzerpakete müssen in einen Low-Ordner umgeleitet werden. Der Arbeitsordner muss auf NTFS liegen und dem Nutzer gehören; auf FAT/exFAT-USB-Sticks gibt es keine Kennzeichnung. Die Kennzeichnung bleibt am Ordner, das ist harmlos (andere Low-Prozesse dürften dort ebenfalls schreiben).

**AppContainer** würde auch das Lesen auf den Ordner beschränken und braucht keine Admin-Rechte. Verbindungen über Loopback in einen AppContainer hinein sind aber ohne Admin-Ausnahme (`CheckNetIsolation`) gesperrt. Der Server müsste also aussen laufen und nur die Befehle des Modells im Container starten; open-terminal startet Befehle über ConPTY bzw. `create_subprocess_shell` ohne Möglichkeit, das einzuhängen. Das wäre ein eigener Prozess-Starter mit ctypes und lohnt sich erst, wenn die Grundversion steht.

**Empfehlung:** Nativ mit Low Integrity Level, Ordnersperre in der Datei-API und den Ordnersperren aus dem Auftrag (OneDrive, SharePoint, Laufwerkswurzeln, Benutzerprofil, Windows, Programme). Wenn das Kennzeichnen scheitert (z. B. USB-Stick), lehnt die App den Ordner ab, statt ohne Schreibschutz zu laufen. Als bekannte Grenze bleibt: Lesen ausserhalb des Ordners und Netzwerkzugriff sind möglich. In der App ist das Starten des Terminals hinter einer Schnittstelle (`src/main/backend/`) gekapselt, damit später AppContainer, Sandbox oder WSL als weitere Variante dazukommen können.

**Saubere Prozessbeendigung** ist ebenfalls getestet: Der Startprozess hält ein Job-Objekt mit «Kill on Job Close». Wird er hart beendet, beendet Windows open-terminal und alle Kindprozesse. Electron startet den Startprozess als Kind; stirbt Electron, schliesst sich dessen Pipe und der Startprozess endet.

## 6. Handtest mit ASK

Diesen Test bitte selbst ausführen, er braucht dein ASK-Login.

1. In ASK oben rechts das Profil öffnen, «Einstellungen». Gibt es den Reiter **«Integrationen»** mit dem Abschnitt **«Open Terminal»**? Wenn nicht, fehlt die Freigabe aus Abschnitt 3; dann ab hier nicht weiter, sondern die Admins fragen. Bitte auch die ASK-Version notieren (Einstellungen, «Info»).
2. In PowerShell im Projektordner starten (Python muss installiert sein; die URL von ASK einsetzen):
   `powershell -ExecutionPolicy Bypass -File scripts\phase0-test.ps1 -AskUrl https://<ASK-URL>`
   Das Skript legt `%USERPROFILE%\ASK-Test\` an (Python-Umgebung und Testordner `ordner` mit `notiz.txt`) und zeigt URL und Key an.
3. In ASK unter «Integrationen», «Open Terminal» auf «Verbindung hinzufügen» (+). URL `http://127.0.0.1:8765`, Key aus dem Fenster einfügen, speichern und die Verbindung aktivieren. Fragt der Browser nach Zugriff auf Geräte bzw. Dienste auf diesem Gerät: erlauben.
4. Neuen Chat mit dem Qwen-Modell öffnen. Im Eingabefeld das Terminal-Symbol wählen und das Terminal auswählen. Rechts sollte der Dateibrowser mit `notiz.txt` erscheinen.
5. Auftrag: «Lies notiz.txt und schreibe eine Datei antwort.txt mit einer kurzen Zusammenfassung davon.» Erwartet: Das Modell ruft Werkzeuge auf (sichtbar als Werkzeugaufrufe im Chat), im PowerShell-Fenster erscheinen Anfragen, und `antwort.txt` liegt in `%USERPROFILE%\ASK-Test\ordner`.
6. Beenden mit Ctrl+C. Der Ordner `%USERPROFILE%\ASK-Test` kann danach gelöscht werden, die Verbindung in ASK ebenfalls.

Bitte zurückmelden: ob der Reiter da war, die ASK-Version, ob das Modell die Werkzeuge benutzt hat, und Fehlermeldungen aus dem Browser oder dem PowerShell-Fenster.

## 7. Grundsatzfragen und Risiken

**Freigabe in ASK.** Fehlt die Freigabe für persönliche Terminals, funktioniert die App so nicht. Ein Terminal auf Admin-Ebene ist keine Alternative, weil dann der ASK-Server die Anfragen stellt und den Laptop nicht erreicht. Alternativen wären: die Admins um Freigabe bitten (der saubere Weg), oder die App führt die Werkzeugschleife selbst aus und nutzt ASK nur als Modell-Schnittstelle (`/api/chat/completions` mit einem persönlichen API-Key, falls ASK API-Keys erlaubt). Die zweite Variante wäre ein anderes Produkt mit eigener Chat-Oberfläche.

**Werkzeugnutzung von Qwen.** Ob das Modell in ASK mit nativer Werkzeugnutzung läuft und die Terminal-Werkzeuge zuverlässig aufruft, zeigt erst der Handtest.

**Verwaltete Schullaptops.** Richtlinien wie AppLocker oder Intune können Programme blockieren, die pro Nutzer unter `AppData` installiert werden. Ein unsignierter Installer löst zudem eine SmartScreen-Warnung aus. Beides muss mit der Informatik der TF Bern geklärt werden; eine Code-Signatur wäre nötig, um die Warnung zu vermeiden.

**Pfadlänge.** Beim Installieren der Python-Pakete in einen tief verschachtelten Ordner scheiterte pip an der 260-Zeichen-Grenze von Windows (Typ-Stubs von `jedi`). Das mitgelieferte Python wird darum mit kurzen Pfaden gebaut und ohne diese Stubs ausgeliefert.

## 8. Nachtrag aus dem Bau der App

Beim Bau kamen drei Befunde dazu. Windows PowerShell 5.1 startet mit Low Integrity nicht (die .NET-Laufzeit bricht mit 0x80131623 ab); das Modell arbeitet deshalb mit `cmd.exe` und Python. Word, Excel und PowerPoint lassen sich aus einem Low-Prozess nicht per COM steuern; die Umwandlung in PDF übernimmt darum die App selbst (ENTSCHEIDE E15). Und Open WebUI holt Werkzeuge und Systemprompt eines persönlichen Terminals nur beim Laden der Seite; die App lädt ASK deshalb erst, wenn das Terminal läuft (ENTSCHEIDE E16). Python wird als offizielles NuGet-Paket 3.14.8 mitgeliefert (ENTSCHEIDE E10).
