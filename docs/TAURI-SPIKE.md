# Tauri-Spike: Läuft ASK Desktop auf Tauri 2?

Stand 25.11.2026, Branch `tauri-spike`. Zeitlich begrenzter Versuch, kein Port. Die Electron-App auf `main` ist unverändert. Prototypen in [`spike/tauri/`](../spike/tauri/), Vorgeschichte in ENTSCHEIDE E9 und E42.

## Kurzfassung

| Frage | Ergebnis | Getestet |
|---|---|---|
| 1. Oberfläche unverändert mit Shim | geht mit kleinen Anpassungen | Linux, ganzer Ablauf |
| 2. Mehrere Webviews in einem Fenster | geht, unter Linux mit Umweg | Linux; Windows nur kompiliert |
| 3. Fernsteuerung des Browser-Bereichs | geht mit eigenem WebView2-Code | Linux teilweise; Windows nur kompiliert |
| 4. Terminal mit Low Integrity im Job | geht, ohne Änderung am Python-Starter | Startmuster unter Linux; Windows nur kompiliert |
| 5. Hauptprozess | Node als Sidecar ist machbar, Rust-Neubau teuer | Sidecar mit echtem `workspace.js` unter Linux |
| 6. Updater und Installer | geht mit Umbau der Release-Kette | Windows-Installer gebaut (cargo-xwin), nicht installiert |

**Empfehlung: bei Electron bleiben (E9, E42 gelten weiter).** Tauri 2 ist heute technisch möglich, deutlich mehr als zur Zeit von E9: Berechtigungen pro Webview, Downloads sperren, Popups abfangen und JavaScript mit Ergebnis gibt es jetzt eingebaut. Aber der Gewinn ist klein und der Aufwand gross: Die vernünftige Variante (Node als Sidecar) spart rund 60 MB von rund 210 MB des Installers, kostet grob 5 bis 8 Personenwochen und bringt neue Risiken (WebView2-Version je Laptop, Proxy des Schulnetzes, Feature `unstable`, Tauri 3 schon in Alpha). Ein Neubau in Rust kostet grob 12 bis 20 Personenwochen. Neu prüfen lohnt sich, wenn Tauri 3 stabil ist und Multiwebview nicht mehr `unstable` braucht.

## Testumgebung

- Container: Ubuntu 24.04 (Linux), Xvfb, WebKitGTK 2.52.6. **Kein Windows, kein WebView2.** Was nur unter Windows läuft, ist mit `cargo check --target x86_64-pc-windows-msvc` geprüft und mit cargo-xwin gebaut, aber nie ausgeführt.
- Tauri 2.12.7 (tauri-runtime-wry 2.12.5, wry 0.57.4, tao 0.37.3), Feature `unstable`, `@tauri-apps/cli` 2.12.4, webview2-com 0.39.1, windows 0.62.2, webkit2gtk-rs 2.0.2, Rust 1.97.0, Node 22.23.5. Neuste Tauri-Version am Testtag: 2.12.7 (18.11.2026); daneben 3.0.0-alpha.6.
- Durchlauf: `spike/tauri/run-linux.sh`. Protokoll [`results/linux/run.log`](../spike/tauri/results/linux/run.log) (Zeilen `SPIKE {…}`), Bildschirmfotos daneben. Für Windows liegt `spike/tauri/run-windows.ps1` bereit (ungetestet).

## Aufbau des Prototyps

```
spike/tauri/
  app/src/main.rs     Fenster mit drei Webviews (ui, ask, browser), Brücke window.ask -> Rust -> Sidecar
  app/src/remote.rs   Fernsteuerung (Linux: WebKitGTK; Windows: WebView2 über CDP) und automatischer Ablauf
  app/src/lowjob.rs   Python-Starter starten; Low+Job zum Vergleich direkt in Rust (nur Windows)
  app/shim/shim.js    window.ask für Tauri, erzeugt aus src/preload/shell.js (tools/gen-shim.mjs)
  sidecar/sidecar.mjs Node-Sidecar: echte Module aus src/main, sonst die Attrappe der Oberfläche
  fixtures/server.mjs Testseiten auf 127.0.0.1: Attrappe der ASK-Anmeldung, Testseite für den Browser
```

Ablauf: `window.ask.x()` → `invoke('ask_invoke', {channel, args})` → Rust. Lage und Sichtbarkeit der Webviews erledigt Rust selbst, alles andere geht als JSON-Zeile an den Node-Sidecar. Ereignisse des Sidecars (`chat:event` usw.) gibt Rust mit `emit_to("ui", …)` weiter. Der Sidecar beantwortet `workspace:list`, `workspace:files` und `workspace:import` mit dem echten `src/main/workspace.js`, unverändert; die übrigen Kanäle mit `src/renderer/src/dev/mockApi.js`.

## 1. Oberfläche unverändert

**Geht mit kleinen Anpassungen.**

- `gen-shim.mjs` liest alle 112 Methoden aus `shell.js` (96 Aufrufe, 16 Ereignisse) und baut daraus `window.ask`. Der Shim läuft als `initialization_script` nur in der Webview `ui`. `src/renderer` ist unverändert, gebaut mit `vite build`.
- Im Durchlauf: Oberfläche startet mit 112 Methoden (`ui-rendered`), lädt `state:get`, `chat:groups`, `skills:list`, `chat:transcript`, `workspace:files`, `workspace:list`. Ein Auftrag über das echte Eingabefeld (Text, Enter) geht als `chat:send` an den Sidecar; 32 `chat:event` kommen gestreamt zurück; die Frage erscheint, der Knopf «Nichts löschen» schickt `chat:answer`, «Ablehnen» schickt `chat:approve`, danach steht «Erledigt.» im Chat (Bilder `5-chat-frage.png`, `6-chat-fertig.png`). Die Rust-Meldung «Download blockiert» erscheint über `browser:notice` in der Oberfläche (`3-browser-bereich.png`).

Nötige Anpassungen:

- **Binärdaten.** Anhänge und hineingezogene Dateien gehen als `Uint8Array` (Composer.jsx, RightPanel.jsx). Electron überträgt das direkt, Tauri nur JSON. Der Shim packt sie als Base64, der Sidecar entpackt sie zu `Buffer` (`binary-import` im Protokoll: 300'000 Bytes kommen unversehrt im Ordner an). Bei 50 MB (Grenze für Anhänge) kostet das Zeit und Speicher; besser wäre ein eigener Befehl mit Roh-Body.
- **Hineinziehen.** Tauri fängt Dateien aus dem Explorer selbst ab; `disable_drag_drop_handler()` an der Webview `ui` ist nötig, sonst kommt kein `dataTransfer.files` an. Unter Windows nicht getestet.
- **`askfile://`.** Die Oberfläche lädt Bilder über `askfile://datei/…` (`lib/format.js`). Eigene Schemata heissen unter WebView2 `http://askfile.localhost/…`; `fileUrl()` muss je nach Plattform eine andere Adresse bauen. Eine Zeile, aber eine Änderung in `src/renderer`.
- `ipcRenderer.send` (Lage der Webviews) wird zu einem `invoke`, dessen Antwort der Shim verwirft. Funktioniert, ist aber ein Rundweg mehr.

## 2. Mehrere Webviews in einem Fenster

**Geht, Feature `unstable`; unter Linux mit Umweg.** Getestet mit Tauri 2.12.7.

- `WindowBuilder` plus drei `window.add_child(WebviewBuilder …)`: `ui` (lokal, füllt das Fenster), `ask` und `browser` (fremde Seiten, je eigener `data_directory`, entspricht `persist:ask` und `persist:ask-browser`).
- Die Oberfläche meldet `#ask-host` und `#browser-host` wie heute (`setAskBounds`, `setBrowserBounds`); Rust setzt Lage, Grösse und Sichtbarkeit. Im Durchlauf liegt die Anmeldeseite genau im Chatbereich (810 × 825) und die Testseite rechts im Browser-Reiter (360 × 530), siehe `2-anmeldung.png` und `3-browser-bereich.png`. Das Token liest Rust wie `askapi.js` aus `localStorage.token` (`ask-token`: gelesen, 16 Zeichen).
- **Linux-Fund:** tauri-runtime-wry 2.12.5 legt Kind-Webviews unter Linux in eine senkrechte `gtk::Box` (`WebviewKind::WindowChild` → `build_gtk(default_vbox)`); `set_position` und `set_size` wirken dort nicht, die Webviews stapeln sich untereinander. Der Spike hängt sie in ein `gtk::Fixed` um (`mod linux` in `main.rs`). Unter Windows und macOS baut Tauri echte Kind-Ansichten (`build_as_child`) mit Koordinaten; das ist der Weg für ASK, aber hier nicht ausgeführt.
- Zoom (Ctrl+Plus) und die Umrechnung von CSS-Pixeln mit dem Zoomfaktor (`window.js`) sind nicht nachgebaut. Tauri hat `set_zoom`, aber keinen Zoom-Wert der Oberfläche; das müsste die Oberfläche selbst melden.
- **Sicherheit:** Die fremden Seiten haben zwar `__TAURI_INTERNALS__`, aber jeder Aufruf scheitert: `ask_invoke not allowed` (`ipc-from-browser`, `ipc-from-ask`). Grund: Die Capability `ui-only` gilt nur für die Webview `ui` mit lokalem Inhalt; zusätzlich prüft `ask_invoke` das Label.

## 3. Fernsteuerung des Browser-Bereichs

**Geht mit eigenem WebView2-Code.** Vergleich mit `visual.js` und `window.js`:

| Electron heute | Tauri 2.12 eingebaut | Sonst | Linux-Test |
|---|---|---|---|
| `executeJavaScript` mit Ergebnis | `eval_with_callback` | – | geht |
| `loadURL`, Zurück, Vor, Neu laden | `navigate`, `reload`; Zurück/Vor über JS `history` | – | geht |
| `will-navigate` sperren | `on_navigation` | – | geht (`/gesperrt` blockiert, Seite bleibt) |
| `will-download` sperren | `on_download` → `false` | – | geht |
| `setWindowOpenHandler`, Popup als kleines Fenster | `on_new_window` (`Deny`, `Allow`, `Create`) | – | Sperre geht; Popup-Fenster nicht getestet |
| Berechtigungen | `on_permission_request` (pro Webview, Ursprung über `url()`) | Local Network Access fehlt in `PermissionKind` (nur `Other`) | Handler gebaut, nicht ausgelöst |
| `clearStorageData` («Anmeldungen löschen») | `clear_all_browsing_data`, `cookies`, `delete_cookie` | – | nicht getestet |
| `capturePage` | **fehlt** | Windows: CDP `Page.captureScreenshot` oder `CapturePreview`; Linux: `webkit_web_view_get_snapshot` | Linux geht (`4-browser-snapshot.png`) |
| `sendInputEvent` (echte Klicks, Tasten) | **fehlt** | Windows: CDP `Input.dispatchMouseEvent`, `Input.dispatchKeyEvent` | Linux: keine API; nur JS-Ereignisse mit `isTrusted=false` |
| `insertText` | **fehlt** | Windows: CDP `Input.insertText` | Linux: nur JS |
| `console-message` | **fehlt** | Windows: CDP `Runtime.consoleAPICalled`; sonst Skript in der Seite (kann die Seite fälschen) | Skript in der Seite geht |
| `input-event` (Eingriff der Person erkennen) | **fehlt** | Skript in der Seite mit `isTrusted`; CDP-Eingaben sind auch `isTrusted` | nicht gebaut |
| Unsichtbares Fenster 1280 × 800 (E27) | verstecktes Fenster möglich | WebView2 rendert verborgene Ansichten nicht zuverlässig; offen | nicht getestet |

Die Windows-Teile stehen in `remote.rs` (`mod win`): `with_webview` → `controller().CoreWebView2()` → `CallDevToolsProtocolMethod` bzw. `GetDevToolsProtocolEventReceiver`. Das ist dieselbe webview2-com-Version wie in Tauri und kompiliert für `x86_64-pc-windows-msvc`. **Ob die CDP-Aufrufe unter WebView2 wirken, ist nicht getestet.** Aufwand für den Browser-Bereich insgesamt: rund 600 bis 900 Zeilen Rust statt der heutigen ~430 Zeilen `visual.js` plus Teile von `window.js`.

## 4. Terminal mit Low Integrity im Job-Objekt

**Geht, und zwar ohne Änderung am Python-Starter.** Die Absenkung auf Low und das Job-Objekt macht heute nicht Node, sondern `python/ask_terminal/launcher.py` mit `winproc.py`. Node startet nur `python -E -s -B -X utf8 launcher.py`, schreibt die Einstellungen als JSON auf stdin und liest JSON-Zeilen von stdout (`backend/native-low.js`). Dasselbe macht Rust mit `std::process`/`tokio::process` (`lowjob.rs`, `start_launcher`). Der Node-Sidecar des Spikes läuft nach genau diesem Muster und funktioniert (Linux).

- Ein Tauri-«Sidecar» (`bundle.externalBin`, tauri-plugin-shell) braucht es dafür nicht; das Python liegt ohnehin als Archiv bei und wird entpackt (E39).
- Zum Vergleich steht in `low_job_spawn` dieselbe Folge direkt in Rust (Token duplizieren, `S-1-16-4096` setzen, `CreateProcessAsUserW` angehalten, `AssignProcessToJobObject` mit `KILL_ON_JOB_CLOSE`, fortsetzen). Kompiliert für Windows, **nicht ausgeführt**; Pipes für stdin/stdout fehlen noch. Kein Grund, den funktionierenden Python-Starter zu ersetzen.
- Offen bleibt, was beim Absturz der App passiert: Heute hält der Python-Starter den Job und beobachtet den Elternprozess (`watch_parent`); das gilt mit Tauri gleich.

## 5. Hauptprozess

`src/main` hat heute 9'073 Zeilen (E42 nannte rund 5'100, seither gewachsen), dazu 3'733 Zeilen Unit-Tests mit `node --test`. Direkt an Electron hängen nur `main.js` (1'205), `window.js` (251), `visual.js` (436), `updater.js` (274), Teile von `askapi.js` (12 Stellen mit `session.fetch`/`executeJavaScript`), `paths.js`, `settings.js` (`app.getPath`) und `attachments.js` (`nativeImage`). Der Rest (Agent, Werkzeuge, GitHub, Sicherung, Gedächtnis, Server, Office, OOXML usw.) ist gewöhnliches Node.

| Variante | Was bleibt | Was neu | Grobaufwand |
|---|---|---|---|
| A. Neubau in Rust | nichts aus `src/main`, keine Unit-Tests | ~9'000 Zeilen Logik plus Tests; SSE-Streaming, Werkzeuge, OOXML-Prüfung, GitHub, Sicherung | 12–20 Personenwochen |
| B. Node als Sidecar | ~6'500 Zeilen und die Unit-Tests unverändert | Rust-Hülle (Webviews, Browser über CDP, Updater, Dialoge, ~1'500–2'500 Zeilen); `main.js` als Sidecar-Einstieg mit Rückrufen zu Rust für alles mit Webviews | 5–8 Personenwochen |
| C. Bei Electron bleiben | alles | – | 0 |

Was bei B ersetzt werden muss (aus `main.js`): `ipcMain` (88 Stellen, im Spike generisch gelöst), `dialog` (tauri-plugin-dialog), `shell` (tauri-plugin-opener), `clipboard`, `Notification`, `Menu`/Tastenkürzel, `protocol.handle('askfile')` (eigenes URI-Schema), `nativeImage` (Bild verkleinern), `safeStorage` für das GitHub-Token (DPAPI über Rust oder keyring), `net.fetch` und `session.fetch`.

Risiko bei B und A: **`net.fetch` nutzt heute den Netzwerkstapel von Chromium mit dem Proxy von Windows** (Kommentar in `main.js`). `fetch` in Node und `reqwest` in Rust lesen keine PAC-Dateien. Im Schulnetz müsste das vor jedem Entscheid geprüft werden. Dazu: Bei B muss `node.exe` mit (87 MB, gepackt 36 MB, Node 22.23.5); die E2E-Tests (`tests/e2e`, starten Electron) müssten auf tauri-driver/WebDriver umgebaut werden.

Weitere Varianten (Bun- oder Deno-Einzeldatei statt node.exe, JS-Engine in Rust) sparen keinen Platz oder fehlen Node-APIs; nicht weiter verfolgt.

## 6. Updater und Installer

**Geht mit Umbau der Release-Kette.**

- **Updater:** tauri-plugin-updater (2.14.1). Er liest ein statisches `latest.json` von einer festen Adresse; `https://github.com/KIYeager/ASK-Releases/releases/latest/download/latest.json` braucht keine GitHub-API und passt damit zu E37 (60 Anfragen pro Stunde ohne Token). Fortschritt kommt pro Datenblock, der Balken von E40 lässt sich nachbauen; Herunterladen und Installieren sind getrennte Schritte, «Später» (E40) geht also: beim Beenden installieren.
- **Beta (E37, E38):** Kein `allowPrerelease`. Eine Vorabversion ist nie «latest»; es braucht eine zweite feste Adresse (z. B. `beta.json` in einem festen Release), die die App je nach Schalter wählt (`updater_builder().endpoints(…)`). `release.yml` müsste beide Dateien pflegen.
- **Signatur:** Tauri verlangt signierte Updates (Schlüsselpaar, privater Schlüssel als Secret im Workflow). Strenger als heute (E37: unsigniert, nur SHA-512), aber ein weiteres Geheimnis zum Verwalten.
- **Installer:** Tauri baut NSIS mit `installMode: currentUser` (ohne Admin, wie E17). `scripts/installer.nsh` (Python-Ordner nur bei Deinstallation löschen) wird zu NSIS-Hooks; Verknüpfung reparieren (E39) bleibt eigener Code. WebView2: in Windows 10/11 normalerweise da; sonst lädt der Installer den Bootstrapper (`downloadBootstrapper`). Eine feste WebView2-Version (wie E9 «überall dieselbe Chromium-Version») müsste die ganze Runtime mitliefern (Grösse nicht gemessen) und frisst den Platzgewinn.
- **Einzelne EXE (E18):** Tauri hat kein Portable-Ziel; `build-portable.js` müsste neu gebaut werden.

Grösse (gebaut im Container mit `npx tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc --bundles nsis`, Release mit LTO, ohne Python und ohne Node; Bauzeit 4 Minuten):

| | Grösse |
|---|---|
| `ask-tauri-spike.exe` (Hülle mit eingebetteter Oberfläche) | 6.1 MiB |
| NSIS-Installer davon | 1.8 MiB (unsigniert, gebaut mit makensis 3.09 unter Linux) |
| Linux-Binary (zum Vergleich) | 5.6 MiB |
| Heutiger Installer ASK (E40) | rund 210 MB, davon Python-Archiv und Electron |
| Electron-Anteil (E9) | rund 100 MB |
| node.exe für Variante B | 87 MB, gepackt 36 MB |

Variante B landet damit bei grob 150 MB (210 − ~100 Electron + ~2 Tauri + ~36 Node); Variante A grob 110 MB statt 210 MB. Die 290 MB Python (entpackt) bleiben in jedem Fall.

## Was nicht geprüft ist

- Alles unter Windows zur Laufzeit: WebView2, CDP-Eingaben, Bildschirmfoto, Popups für Microsoft-Anmeldung, Low+Job aus Rust, Installer, Updater. `run-windows.ps1` ist der nächste Schritt, falls das Thema wieder aufkommt.
- Die echte ASK-Anmeldung (Microsoft-Konto) in einer Tauri-Webview; der Spike nutzt eine Attrappe.
- Proxy und PAC im Schulnetz ohne Chromium-Netzwerkstapel.
- Arbeitsspeicher und Startzeit im Vergleich.
