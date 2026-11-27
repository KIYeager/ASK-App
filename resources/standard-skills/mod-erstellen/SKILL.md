---
name: mod-erstellen
description: Führt durch das Erstellen eines ASK-Mods (eigenes Aussehen, /Befehle, Seitenpanel, schwebende Figur, Verhalten). Verwenden, wenn die Person ASK anpassen, verschönern oder erweitern möchte.
---
# ASK-Mod erstellen

Ein Mod verändert ASK: Aussehen (CSS), Verhalten (JavaScript), eigene `/befehle` und zusätzliche Anweisungen. Ein Mod ist ein Ordner mit einer Datei `mod.json` und optionalen weiteren Dateien.

## Vorgehen
1. **Wunsch klären.** Frag, was der Mod tun soll. Mögliche Arten, auch kombinierbar:
   - **Aussehen:** Farben, Rundungen, Schrift (nur CSS).
   - **Befehl:** ein `/befehl`, der eine Vorlage an ASK schickt (nur `mod.json`).
   - **Seitenpanel:** ein eigener Reiter rechts (Timer, Notizen, Spickzettel).
   - **Schwebende Figur:** ein Maskottchen, das auf ASK reagiert.
   - **Verhalten:** reagiert auf Ereignisse (Meldung bei «fertig», Knopf in der Kopfzeile).
2. **Plan vorschlagen.** Nenne Id, Name, welche Dateien entstehen und was die Person am Ende sieht. Hol ein Ja ein.
3. **Mod erstellen** mit dem Werkzeug `mod_erstellen` (Dateien: `mod.json`, `style.css`, `mod.js`, Bilder).
4. **Der Person erklären:** Neue oder geänderte Mods sind **ausgeschaltet**. Sie wirken erst, wenn die Person sie unter **Einstellungen → Mods** nach einer Vorschau einschaltet. Sag das immer, sonst sucht sie vergeblich nach der Wirkung.
5. **Nachbessern**, wenn die Person etwas ändern will. Nach jeder Änderung muss sie den Mod erneut in der Vorschau bestätigen.

## Grenzen (Sicherheit)
- Keine Sicherheitseinstellungen ändern, keine Rückfragen («Darf ASK …?») automatisch bestätigen, keine Zugangsdaten oder Schlüssel lesen. Die App blockiert das ohnehin. Versuche es nicht und baue es nicht ein.
- **Kein Netzwerk:** Anfragen ins Internet (fetch, externe Bilder, Schriften, Skripte) sind gesperrt. Alles Nötige gehört in den Mod-Ordner.
- Kein verschleierter Code. Der Code soll lesbar sein, die Person darf ihn prüfen.
- CSS **eingrenzen**: Klassen mit der Mod-Id als Anfang (`.robo-helfer-blase`). Nicht ganze Seiten mit `* { … }` umstellen. Farben immer über die Variablen (unten), damit Hell und Dunkel funktionieren.
- Bilder und Töne in den Mod-Ordner legen und mit `ASK.datei('bild.png')` verwenden.
- Lieber kleine, einfache Mods. Das Ganze soll die Person verstehen können.

## Aufbau
Ordner `<id>/` mit:
```json
{
  "id": "robo-helfer",
  "name": "Robo-Helfer",
  "version": "1.0.0",
  "beschreibung": "Ein kleiner Roboter, der auf ASK reagiert.",
  "autor": "Name",
  "style": "style.css",
  "script": "mod.js",
  "befehle": [
    { "name": "vokabeln", "beschreibung": "Vokabeln abfragen", "prompt": "Frag mich 10 Vokabeln ab zum Thema: {{text}}" }
  ],
  "anweisungen": "Optional: zusätzliche Anweisungen für ASK.",
  "nachfragen": ["loeschen", "schreiben", "befehle", "web"]
}
```
- `id`: nur a-z, 0-9 und Bindestriche, höchstens 40 Zeichen, gleich wie der Ordnername. Pflicht, ebenso `name` und `version`.
- `style`, `script`, `befehle`, `anweisungen`, `nachfragen`, `autor`, `beschreibung` sind optional.
- `befehle`: eigene `/name text`. `{{text}}` wird durch den Text nach dem Befehl ersetzt.
- `anweisungen`: kurzer Text, der ASKs Anweisungen ergänzt (z. B. «Erkläre immer mit einem Beispiel aus dem Alltag»).
- `nachfragen`: ASK muss bei diesen Arten von Schritten **zuerst fragen**. Möglich: `loeschen`, `schreiben`, `befehle`, `web`. Ein Mod kann ASK nur **strenger** machen, nie lockerer.

## CSS-Variablen
Verwende diese, dann passt der Mod zu Hell und Dunkel:
`--bg` (Hintergrund), `--side` (linke Leiste), `--pane` (Flächen), `--card` (Karten), `--hover`, `--line` (Linien), `--text`, `--muted`, `--dim` (leise Schrift), `--acc` (Akzentfarbe, TF Blau), `--acc-ink` (Schrift auf der Akzentfarbe), `--acc-soft`, `--ok`, `--bad`, `--warn`, `--r-sm`, `--r`, `--r-lg` (Rundungen), `--sans`, `--mono` (Schriften).
Das Wurzelelement trägt `data-theme="dark"` oder `data-theme="light"`:
```css
:root[data-theme="dark"]  { --acc: #22d3ee; }
:root[data-theme="light"] { --acc: #0e7490; }
```

## JavaScript-Schnittstelle (`ASK`)
`mod.js` ist ein normales Skript (kein `import`). Alles in eine sofort ausgeführte Funktion packen, die `ASK` festhält: `(function (ASK) { … })(ASK);`. So trifft jeder spätere Aufruf (z. B. in Ereignissen) die Ablage und Dateien des eigenen Mods.

| Aufruf | Wirkung |
|---|---|
| `ASK.on(ereignis, fn)` | Auf ein Ereignis hören. Gibt eine Funktion zum Abmelden zurück. |
| `ASK.eingabe.lesen()` | Text im Eingabefeld lesen |
| `ASK.eingabe.setzen(text)` | Eingabefeld ersetzen |
| `ASK.eingabe.einfuegen(text)` | Text ins Eingabefeld einfügen |
| `ASK.senden(text)` | Nachricht senden, als hätte die Person sie geschrieben |
| `ASK.chat()` | `{ id, titel, nachrichten: [{ rolle: 'person'\|'ask', text }] }` |
| `ASK.meldung(text, { dauer })` | Kleine Einblendung (Toast) |
| `ASK.befehl({ name, beschreibung, ausfuehren(text) })` | `/befehl`, der in JavaScript abläuft |
| `ASK.panel({ id, titel, render(element) })` | Reiter in der rechten Seitenleiste. `render` bekommt ein leeres `div`. |
| `ASK.knopf({ titel, symbol, klick })` | Kleiner Knopf in der Kopfzeile (`symbol`: Emoji oder kurzer Text) |
| `ASK.datei(pfad)` | URL einer Datei im Mod-Ordner (Bild, Ton) |
| `ASK.speicher.lesen(key)` / `.schreiben(key, wert)` | Kleiner Speicher nur für diesen Mod (JSON-Werte) |
| `ASK.dunkel()` | `true`, wenn das dunkle Design aktiv ist |

**Ereignisse** (`ASK.on`):
- `'gesendet'` `{ text }`: die Person hat gesendet
- `'antwort'` `{ text }`: eine Antwort von ASK ist fertig
- `'fertig'` `{}`: der Auftrag ist erledigt
- `'fehler'` `{ meldung }`
- `'frage'` `{ text }`: ASK fragt etwas oder braucht eine Erlaubnis
- `'chat'` `{ id, titel }`: anderer Chat geöffnet
- `'theme'` `{ dunkel }`: Hell/Dunkel gewechselt

Voller DOM-Zugriff ist erlaubt (z. B. `document.body.appendChild(...)` für eine schwebende Figur). Eigene Elemente mit Klassen, die mit der Mod-Id beginnen.

## Beispiel 1: Aussehen (nur CSS)
`mod.json`
```json
{ "id": "sonnenuntergang", "name": "Sonnenuntergang", "version": "1.0.0",
  "beschreibung": "Warme Farben und runde Ecken.", "style": "style.css" }
```
`style.css`
```css
:root[data-theme="dark"]  { --acc: #fb923c; --acc-ink: #1a1008; --r: 14px; }
:root[data-theme="light"] { --acc: #c2410c; --acc-ink: #ffffff; --r: 14px; }
```

## Beispiel 2: Schwebende Figur, die reagiert
`mod.json`
```json
{ "id": "wuerfel-freund", "name": "Würfel-Freund", "version": "1.0.0",
  "beschreibung": "Ein kleiner Würfel feiert, wenn ASK fertig ist.", "style": "style.css", "script": "mod.js" }
```
`style.css`
```css
.wuerfel-freund-figur { position: fixed; right: 20px; bottom: 110px; z-index: 50;
  width: 44px; height: 44px; border-radius: var(--r); background: var(--acc);
  color: var(--acc-ink); display: grid; place-items: center; font-size: 22px;
  transition: transform .2s; }
.wuerfel-freund-figur.wuerfel-freund-froh { transform: translateY(-10px) rotate(10deg); }
@media (prefers-reduced-motion: reduce) { .wuerfel-freund-figur { transition: none; } }
```
`mod.js`
```js
(function (ASK) {
  var figur = document.createElement('div');
  figur.className = 'wuerfel-freund-figur';
  figur.textContent = '🎲';
  document.body.appendChild(figur);

  ASK.on('fertig', function () {
    figur.classList.add('wuerfel-freund-froh');
    ASK.meldung('Fertig!', { dauer: 2000 });
    setTimeout(function () { figur.classList.remove('wuerfel-freund-froh'); }, 1200);
  });
  ASK.on('fehler', function () { figur.textContent = '😟'; });
  ASK.on('gesendet', function () { figur.textContent = '🎲'; });
})(ASK);
```

## Vor dem Erstellen prüfen
- Id, Ordnername und Dateinamen in `mod.json` stimmen überein.
- JSON ist gültig (keine Kommentare, keine Kommas am Ende).
- Kein Netzwerkzugriff, keine fremden Adressen.
- Mod-Ids und Klassennamen kollidieren nicht mit anderen.
- Funktioniert es in Hell und Dunkel? Auf dem Handy ist ASK nicht relevant, aber das Fenster kann schmal sein.
