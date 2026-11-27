---
name: interaktive-antworten
description: Format und Regeln für interaktive Antworten im Chat: Diagramme, Quiz, Karteikarten, Rechner, Auswahl, Formulare, Schritte, Vergleiche (```ask-ui) und eigenes HTML (```ask-html). Lesen, bevor du einen solchen Baustein schreibst.
---
# Interaktive Antworten

Die Oberfläche von ASK zeigt bestimmte Codeblöcke in deiner Antwort nicht als Code, sondern als interaktive Bausteine. So kann die Person mit Zahlen spielen, üben oder mit einem Klick antworten, statt nur zu lesen.

## 1. Wann ein Baustein, wann Text

Benutze einen Baustein von selbst, wenn er **deutlich mehr hilft als Text**:

| Situation | Baustein |
|---|---|
| Zahlen vergleichen, Verlauf, Anteile | `diagramm` |
| Gelerntes prüfen, Prüfung vorbereiten | `quiz` |
| Begriffe, Vokabeln, Definitionen lernen | `karteikarten` |
| «Was wäre, wenn …?», Budget, Zinsen, Umrechnen | `rechner` |
| Die Person soll aus wenigen Möglichkeiten wählen (nächster Schritt, Thema, Niveau) | `auswahl` |
| Du brauchst mehrere Angaben auf einmal | `formular` |
| Anleitung zum Abarbeiten | `schritte` |
| Zwei bis vier Dinge gegenüberstellen | `vergleich` |
| Wichtige Warnung oder Tipp hervorheben | `hinweis` |
| Etwas, das keiner der Bausteine kann (Simulation, kleines Spiel, Animation, Zeichenfläche) | `ask-html` |

**Feste Bausteine zuerst.** Diagramm, Quiz, Karteikarten, Rechner, Auswahl, Formular, Schritte, Vergleich und Hinweis gibt es fertig als `ask-ui`: Sie sehen aus wie die App, funktionieren in Hell und Dunkel und brauchen keinen Code. Baue sie **nie in HTML nach**, auch nicht, um sie «schöner» oder in Reitern zu zeigen. Mehrere Bausteine kommen als Liste in einen `ask-ui`-Block. `ask-html` nur, wenn wirklich keiner passt (Simulation, Spiel, Animation, Zeichnung, eigene Bedienung).

**Kein Baustein** bei kurzen Antworten, reinen Erklärungen, Code, den die Person kopieren soll, und wenn du gerade mit Werkzeugen arbeitest. Höchstens zwei Blöcke pro Antwort; immer mindestens ein Satz Text davor, der sagt, was der Baustein zeigt. Ein Baustein ersetzt keine Erklärung, er ergänzt sie.

Fehlt dir für deine Arbeit eine Entscheidung, nimm weiterhin `frage_stellen`. `auswahl` ist für Angebote an die Person («Womit machen wir weiter?»), nicht für Rückfragen, ohne die du nicht weiterarbeiten kannst.

Schreib Bausteine **direkt in die Antwort**, nie in eine Datei. Ist «Interaktive Antworten» in den Einstellungen aus, erscheint der Block als Code; dann antworte nur mit Text.

## 2. Feste Bausteine: ```ask-ui

Ein Codeblock mit der Sprache `ask-ui` und **gültigem JSON** (doppelte Anführungszeichen, keine Kommentare, keine Kommas am Ende). Der Inhalt ist ein Objekt oder eine Liste von höchstens 8 Objekten. Jedes Objekt hat `typ`, fast alle ein freiwilliges `titel`. Alle Texte sind einfacher Text (kein Markdown, kein HTML), auf Deutsch mit Schweizer Rechtschreibung (ss statt Eszett).

```ask-ui
{"typ": "hinweis", "art": "tipp", "titel": "Tipp", "text": "Speichere vor dem Ausprobieren eine Kopie."}
```

### hinweis
`art`: `info` (Standard), `tipp`, `warnung` oder `erfolg`; `text` (nötig), `titel`.

### schritte
`schritte`: Liste von Texten oder Objekten `{"titel": "…", "text": "…"}`. Die Person kann Schritte abhaken.
```json
{"typ": "schritte", "titel": "Git einrichten", "schritte": [{"titel": "Git installieren", "text": "git-scm.com, Standard-Einstellungen"}, "Namen setzen: git config --global user.name", "Repository klonen"]}
```

### vergleich
`spalten`: 2 bis 4 Objekte mit `titel`, `untertitel`, `punkte` (Liste von Texten), `hervorheben` (true für die Empfehlung).
```json
{"typ": "vergleich", "titel": "Laptop oder Desktop", "spalten": [{"titel": "Laptop", "untertitel": "mobil", "punkte": ["überall dabei", "teurer pro Leistung"]}, {"titel": "Desktop", "untertitel": "stark", "punkte": ["aufrüstbar", "günstiger"], "hervorheben": true}]}
```

### diagramm
`art`: `balken` (Standard), `linie` oder `kreis`. `beschriftungen`: Liste der Kategorien. `reihen`: Liste von `{"name": "…", "werte": [Zahlen]}`, jede Reihe genau so viele Werte wie Beschriftungen, höchstens 6 Reihen. `einheit` (z. B. "CHF", "%"). Bei `kreis` zählt nur die erste Reihe, Werte ab 0. Werte sind Zahlen, keine Texte (`12.5`, nicht `"12,5 CHF"`).
```json
{"typ": "diagramm", "titel": "Ausgaben 2026", "art": "balken", "einheit": "CHF", "beschriftungen": ["Jan", "Feb", "Mär"], "reihen": [{"name": "Freizeit", "werte": [120, 95, 140]}, {"name": "ÖV", "werte": [80, 80, 80]}]}
```
Nimm nur Zahlen, die du wirklich kennst (aus Dateien, Rechnungen oder dem Gespräch). Erfinde keine Daten; sind sie geschätzt, sag es im Text.

### quiz
`fragen`: Liste von `{"frage": "…", "antworten": ["…", "…"], "richtig": 0, "erklaerung": "…"}`. `richtig` ist die **Nummer ab 0** der richtigen Antwort. 2 bis 8 Antworten, nur eine richtig. Die Erklärung erscheint nach dem Klick und sagt, warum. Gute Quizfragen: 3 bis 5 Fragen, plausible falsche Antworten, richtige Antwort nicht immer an derselben Stelle.
```json
{"typ": "quiz", "titel": "Subnetze", "fragen": [{"frage": "Wie viele nutzbare Adressen hat ein /24-Netz?", "antworten": ["256", "254", "255"], "richtig": 1, "erklaerung": "Netz- und Broadcast-Adresse fallen weg: 256 − 2 = 254."}]}
```

### karteikarten
`karten`: Liste von `{"vorne": "…", "hinten": "…"}`. Vorne kurz (Begriff, Frage), hinten die Antwort in einem oder zwei Sätzen.

### rechner
`eingaben`: Liste von `{"name": "betrag", "label": "Betrag pro Monat", "wert": 50, "min": 0, "max": 500, "schritt": 10, "einheit": "CHF"}`. `name` nur aus Buchstaben, Ziffern und `_` (ohne Umlaute). Jede Eingabe hat einen Regler und ein Zahlenfeld.
`ergebnisse`: Liste von `{"label": "…", "formel": "…", "einheit": "…", "stellen": 2}`. Die Formel rechnet mit den Namen der Eingaben: `+ - * / % ^`, Klammern, Zahlen mit Punkt, Funktionen `min max round(x, stellen) abs sqrt floor ceil pow log ln exp sin cos tan`, Konstanten `pi` und `e`. Keine Bedingungen, keine Texte.
```json
{"typ": "rechner", "titel": "Zinseszins", "eingaben": [{"name": "start", "label": "Startbetrag", "wert": 1000, "min": 0, "max": 20000, "schritt": 100, "einheit": "CHF"}, {"name": "zins", "label": "Zins pro Jahr", "wert": 2, "min": 0, "max": 10, "schritt": 0.1, "einheit": "%"}, {"name": "jahre", "label": "Jahre", "wert": 10, "min": 1, "max": 40}], "ergebnisse": [{"label": "Endbetrag", "formel": "start * (1 + zins / 100) ^ jahre", "einheit": "CHF", "stellen": 2}, {"label": "Davon Zins", "formel": "start * (1 + zins / 100) ^ jahre - start", "einheit": "CHF", "stellen": 2}]}
```
Prüfe jede Formel im Kopf mit den Startwerten, bevor du sie schreibst.

### auswahl
`frage`, `optionen` (2 bis 12 kurze Texte), `mehrfach` (true: mehrere ankreuzen, dann «Auswahl senden»). Ein Klick schickt den Text der Option als neue Nachricht der Person. Formuliere die Optionen so, dass sie als Nachricht Sinn ergeben («Mehr Übungen», nicht «A»).

### formular
`felder`: Liste von `{"name": "…", "label": "…", "art": "text|zahl|auswahl|mehrzeilig", "optionen": […], "platzhalter": "…", "pflicht": true}`; `senden`: Text des Knopfs. Beim Senden kommt eine Nachricht mit dem Titel und einer Zeile «- Label: Wert» pro ausgefülltem Feld. Höchstens 12 Felder; frag nur, was du wirklich brauchst.

## 3. Eigenes HTML: ```ask-html

Für alles, was die festen Bausteine nicht können. Der Block enthält HTML mit `<style>` und `<script>` und läuft **abgeschottet**:
- **Kein Internet.** Keine Bibliotheken von CDNs, keine Webschriften, kein `fetch`, keine Bilder von Adressen. Alles muss im Block selbst stehen (Bilder höchstens als `data:`-Adresse oder SVG). Lädst du etwas von aussen, bleibt der Baustein leer oder kaputt.
- Kein Zugriff auf ASK, die Dateien des Projekts oder andere Fenster. `localStorage` und Cookies gibt es nicht.
- Höchstens 200 KB. Halte es klein: ein Bildschirm, eine Aufgabe.
- **Benutze den Bausatz** (unten), statt eigenes CSS für Knöpfe, Felder, Karten oder Balken zu schreiben. Eigenes CSS nur für das, was der Bausatz nicht hat.
- **Layout ohne feste Breiten:** keine Breiten in Pixeln für Spalten oder Beschriftungen, kein `position: absolute` für Text. Flex oder Grid mit `minmax(0, …)`, lange Texte mit `overflow-wrap` oder `text-overflow: ellipsis`. Sonst überlappen Beschriftungen bei schmalem Chat.
- Schreib nur den Inhalt (kein `<html>`, `<head>` oder `<body>` nötig). Die Breite ist die des Chats (etwa 600 bis 900 Pixel); die Höhe passt sich an, bis 900 Pixel. Mach es auch bei 360 Pixel Breite bedienbar.
- Brauchst du eigene Farben, nimm die der App, dann passt es zu Hell und Dunkel: `var(--ask-bg)`, `--ask-card`, `--ask-line`, `--ask-line-2`, `--ask-text`, `--ask-muted`, `--ask-acc` (Blau), `--ask-acc-ink` (Schrift auf Blau), `--ask-acc-soft`, `--ask-ok`, `--ask-bad`, `--ask-orange`, Schrift `--ask-font` und `--ask-mono`. Der Hintergrund ist durchsichtig.
- Zurück in den Chat: `ask.senden("Text")` schickt den Text als Nachricht der Person (nur nach einem Klick in deinem Baustein, sonst landet er im Eingabefeld); `ask.eingeben("Text")` schreibt ihn ins Eingabefeld. Ruf beides nur auf einen Klick der Person auf, nie von selbst.
- Kein `alert`, `prompt`, `confirm`, keine neuen Fenster, keine Formulare, die irgendwohin senden.

### Bausatz für HTML
Schon ohne eigenes CSS sehen `h1`–`h4`, `p`, `button`, `input`, `select`, `textarea`, `table` (Zahlenspalten mit `class="zahl"`) und `code` aus wie in der App. Dazu diese Klassen:

| Klasse | Wofür |
|---|---|
| `ask-card` | Kasten mit Rahmen und Hintergrund |
| `ask-stack` / `ask-row` / `ask-grid` | untereinander / nebeneinander mit Umbruch / Kacheln, die sich der Breite anpassen |
| `ask-primary` | Hauptknopf (blau) |
| `ask-muted`, `ask-big`, `ask-badge`, `ask-ok`, `ask-bad` | Nebentext, grosse Zahl, Etikett, grün, rot |
| `ask-tabs` | Reiter ohne eigenes JavaScript: Knöpfe mit `data-tab="x"` in `<div class="ask-tabs">`, daneben (im selben Eltern-Element) Bereiche mit `data-panel="x"` |
| `ask-bars`, `ask-bar` | Balkenliste: jede Zeile Beschriftung, `<span class="ask-bar" style="--wert: 82">` (0 bis 100), Wert; Farbe mit `--farbe` |

```ask-html
<div class="ask-stack">
  <div class="ask-row"><h3 style="margin:0">Würfel</h3><span class="ask-badge">1 bis 6</span></div>
  <div class="ask-card ask-row"><span class="ask-big" id="z">–</span><span class="ask-muted" id="info">Noch nicht gewürfelt</span></div>
  <div class="ask-row">
    <button class="ask-primary" id="b">Würfeln</button>
    <button id="s" disabled>Ergebnis senden</button>
  </div>
</div>
<script>
  let wert = 0, wuerfe = 0;
  b.onclick = () => { wert = 1 + Math.floor(Math.random() * 6); wuerfe++; z.textContent = wert; info.textContent = wuerfe + ". Wurf"; s.disabled = false; };
  s.onclick = () => ask.senden("Ich habe eine " + wert + " gewürfelt.");
</script>
```

## 4. Vor dem Senden prüfen

1. Die Zaunzeile ist genau ` ```ask-ui ` oder ` ```ask-html `, das Ende eine Zeile nur mit ` ``` `.
2. JSON gültig? Alle Schlüssel und Texte in doppelten Anführungszeichen, Zahlen ohne Anführungszeichen, keine Kommentare.
3. Pflichtfelder da (Diagramm: gleich viele Werte wie Beschriftungen; Quiz: `richtig` ab 0; Rechner: jede Formel nutzt nur vorhandene Namen).
4. HTML nur, wenn kein fester Baustein passt; dann ohne Adressen nach aussen (`http`, `//cdn`, `@import url(`), mit dem Bausatz und ohne feste Pixelbreiten für Text.
5. Ein Satz Text vor dem Baustein; danach nicht nochmals dasselbe als Liste.

Kann die Oberfläche einen Block nicht zeigen, erscheint er als Code mit der Meldung, was falsch ist. Sagt die Person, ein Baustein sei kaputt, korrigiere ihn nach dieser Meldung.
