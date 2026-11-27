---
name: webseite-gestalten
description: Erstellt schön gestaltete Webseiten mit HTML und CSS: klare Gestaltungsrichtung, Schrift, Abstände, Farben als Variablen, responsiv und gut lesbar. Verwenden, wenn die Person eine Webseite, Landingpage oder ein Layout bauen oder verschönern will.
---
# Webseite gestalten

Ziel: Eine Seite, die eigenständig aussieht, gut lesbar ist und auf Handy und Computer funktioniert. Kein austauschbarer Einheitslook.

## Vorgehen
1. **Klären.** Wofür ist die Seite (Portfolio, Verein, Projekt, Aufgabe)? Für wen? Welche Inhalte (Texte, Bilder, Bereiche)? Gibt es Farben, Logo oder Vorbilder? Fehlt Inhalt, mit Platzhaltern arbeiten und das sagen. Nichts erfinden, was nach echten Fakten aussieht.
2. **Gestaltungsrichtung festlegen.** Wähle bewusst einen Stil und halte ihn durch: z. B. ruhig und minimal, verspielt und bunt, editorial (viel Schrift), technisch (Mono-Schrift, dunkel), warm und handgemacht. Sag der Person in einem Satz, was du gewählt hast.
3. **Projekt prüfen.** Gibt es schon ein Framework (`package.json`, React, Vue …)? Dann dessen Struktur nutzen. Sonst: eine `index.html` und eine `style.css` im Arbeitsordner, ohne Build-Werkzeug.
4. **Zuerst die Grundlagen als Variablen** am Anfang der CSS-Datei, dann Seite bauen.
5. **Ansehen.** Die Seite im Browser oder in der Vorschau von ASK öffnen und kontrollieren, auch in schmaler Breite (Handy). Fehler beheben.
6. **Der Person zeigen, was wo steht**, damit sie selbst Texte und Farben ändern kann.

## Gestaltungsregeln
- **Schrift:** Höchstens zwei Schriften (z. B. eine für Titel, eine für Text). Systemschriften sind in Ordnung. Zeilenlänge 45 bis 75 Zeichen, Zeilenhöhe 1.5 bis 1.7 für Fliesstext. Keine Webschriften von fremden Servern, wenn die Seite offline funktionieren soll.
- **Grössen und Abstände:** Feste Skala verwenden (z. B. 4, 8, 16, 24, 32, 48, 64 px), nicht jedes Mal einen neuen Wert. Genug Weissraum.
- **Farben:** 1 Hauptfarbe, 1 Akzent, Neutralfarben für Hintergrund und Text. Als CSS-Variablen anlegen:
  ```css
  :root { --bg:#fbf8f3; --text:#1f1b16; --accent:#c2410c; --radius:12px; --space:16px; }
  ```
- **Kontrast:** Text auf Hintergrund mindestens 4.5 : 1. Nie hellgrau auf weiss. Information nie nur durch Farbe zeigen.
- **Responsiv:** Mit `<meta name="viewport" content="width=device-width, initial-scale=1">`, Layout mit Flexbox oder Grid, `max-width` statt fester Breiten, Breakpoint ab etwa 640 und 960 px. Bilder `max-width:100%`.
- **Zugänglich:** Sinnvolle Tags (`header`, `nav`, `main`, `section`, `footer`), Überschriften in Reihenfolge, `alt`-Texte bei Bildern, sichtbarer Fokus bei Tastaturbedienung, `lang="de"`.
- **Eigenständig wirken:** Nicht der Standard aus violettem Verlauf und weissen Karten mit Schatten. Mutig bei Schriftgrösse, Ausschnitt und Ausrichtung, aber lesbar bleiben.
- **Bewegung:** Sparsam, kurze Übergänge; `prefers-reduced-motion` respektieren.

## Technik
- Kein JavaScript, wenn CSS reicht.
- Bilder und Dateien im Arbeitsordner ablegen und relativ verlinken (`bilder/foto.jpg`).
- Keine fremden Skripte oder Tracker einbinden.
- Texte in Schweizer Rechtschreibung («ss», « »), wenn die Seite für die Schweiz ist.

## Abschluss
Nenne die Dateien, wie die Person die Seite öffnet, und was sie als Nächstes anpassen kann (Farben in `:root`, Texte in `index.html`).
