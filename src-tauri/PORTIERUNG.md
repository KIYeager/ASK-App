# Portierung Electron → Tauri 2 (Arbeitsregeln)

Die Logik aus `src/main/*.js` wird nach Rust portiert, Modul für Modul, in den Crate `ask-core`
(`src-tauri/core`). Die Tauri-Hülle (`src-tauri/src`) liefert Fenster, Webansichten und Dialoge.
Die Oberfläche (`src/renderer`, React) bleibt; nur `window.ask` kommt neu aus `src/renderer/src/bridge.js`.

## Regeln für jedes Modul

1. **Treu portieren.** `src/main/<name>.js` → `src-tauri/core/src/<name>.rs` (Bindestriche → `_`).
   Gleiches Verhalten, gleiche Texte (Deutsch, Schweizer Schreibweise ohne ß, «» als Anführungszeichen),
   gleiche Grenzen und Zeitlimits, gleiche Dateiformate (JSON-Dateien müssen zwischen Electron- und
   Tauri-Fassung austauschbar bleiben). Kommentare auf Deutsch wie im Original, inklusive Verweise auf
   ENTSCHEIDE. Nichts weglassen, nichts «verbessern» ohne Grund; offensichtliche Fehler des Originals
   beheben und im Bericht nennen.
2. **Namen:** JS-Funktionen in snake_case, Klassen als `struct` mit gleichem Namen (`AskSession`,
   `Snapshots` …), Konstanten in SCREAMING_CASE. Was andere Module brauchen, ist `pub`; oben im Modul
   ein Doc-Kommentar mit der öffentlichen Schnittstelle.
3. **Daten:** Was in JS untypisierte Objekte sind (Nachrichten an die Oberfläche, Antworten von ASK
   und GitHub, Chats auf der Platte), bleibt `serde_json::Value` oder bekommt `serde`-Structs mit
   `#[serde(rename_all = "camelCase")]`, die dasselbe JSON lesen und schreiben. Feldnamen im JSON
   ändern sich nicht. Hilfen in `util.rs` (`str_of`, `num_of`, `bool_of`, `truthy`, `js_string`,
   `take_chars`, `Emitter`, `now_ms`, `path_key`, `same_path`, `win_basename`).
4. **Asynchron:** tokio. `async fn` wo JS `async` ist oder auf Netz/Prozesse wartet; sonst sync.
   Geteilter Zustand: `Arc<…>` mit `parking_lot::Mutex` (nie über `.await` halten) oder
   `tokio::sync::Mutex`, wenn über `.await` nötig. Ereignisse wie `EventEmitter`: `util::Emitter`.
5. **Fehler:** `anyhow::Result`; die Fehlertexte sind die deutschen Meldungen aus dem Original
   (`anyhow::bail!("…")`), denn sie landen in der Oberfläche oder beim Modell.
6. **Abhängigkeiten von aussen** (in JS als Optionen hereingereicht: `fetchImpl`, `confirm`, `send`,
   `ask`, `spawn` …) bleiben hereingereicht: als Felder mit Typen aus `host.rs` (`SendFn`,
   `ConfirmFn`) oder als eigenes kleines Trait im Modul. HTTP über `reqwest::Client`
   (hereingereicht oder `Default`), damit Tests einen lokalen Server benutzen können.
7. **Electron-Teile:** `nativeImage` → Crate `image`; `net.fetch` → `reqwest`; `safeStorage`,
   Fenster und Webansichten → nicht im Core, sondern als Trait (das setzt die Hülle um). Windows-only
   (`execFile('icacls')`, PowerShell, Registry) mit `std::process::Command` und `#[cfg(windows)]`
   wo nötig; unter Linux muss alles kompilieren, und Tests, die Windows brauchen, bekommen
   `#[cfg(windows)]`.
8. **Tests:** Die Tests aus `tests/<name>.test.js` werden als `#[cfg(test)] mod tests` im selben
   Modul portiert (gleiche Fälle, gleiche Erwartungen). Temporäre Ordner mit `tempfile`. HTTP-Attrappen
   mit einem kleinen lokalen Server (`tokio::net::TcpListener`) statt Netzwerk. `cargo test -p ask-core`
   muss grün sein, ohne Warnungen in den eigenen Dateien (`cargo clippy` sauber ist erwünscht).
9. **Keine neuen Crates** ohne Grund; wenn nötig, in `src-tauri/core/Cargo.toml` ergänzen und im Bericht
   nennen. Keine Crates, die C-Code für Windows kompilieren müssen (kein `ring`, `openssl` nur über
   `native-tls`).
10. Nur die eigenen Dateien ändern (plus `Cargo.toml`, falls nötig). `lib.rs` deklariert alle Module
    schon; Platzhalter-Dateien einfach überschreiben.

Befehle (im Ordner `src-tauri`):

```
cargo test -p ask-core <modul>::
cargo clippy -p ask-core
```
