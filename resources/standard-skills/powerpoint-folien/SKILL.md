---
name: powerpoint-folien
description: Erstellt eine PowerPoint-Präsentation aus Stichworten, einem Text oder einem Plan: klare Folien, wenig Text, Notizen zum Sprechen. Verwenden, wenn die Person Folien oder eine .pptx-Datei will.
---
# PowerPoint-Folien erstellen

Ziel: Folien, die den Vortrag stützen, statt ihn zu ersetzen. Für die Planung eines ganzen Vortrags gibt es den Skill `praesentation-planen`; dieser hier baut die Datei.

## Vorgehen
1. **Inhalt klären.** Gibt es schon einen Plan, einen Text oder Stichworte (im Chat oder als Datei im Projekt)? Dann lesen. Sonst kurz fragen: Thema, Publikum, Dauer, Vorgaben der Schule (Vorlage, Anzahl Folien).
2. **Folienliste zeigen.** Eine Zeile pro Folie: Titel als Aussage und was darauf zu sehen ist. Faustregel: ein bis zwei Minuten pro Folie. Frag, ob die Liste passt.
3. **Anleitung lesen.** Lies mit `read_file` die Datei `.ask/skills/powerpoint/SKILL.md` und halte dich daran (Layouts der Vorlage, Platzhalter füllen, Notizen).
4. **Folien bauen.** Titelfolie, Inhaltsfolien, Schlussfolie mit Zusammenfassung, zuletzt Quellen. Was die Person dazu sagt, kommt in die Notizen der Folie.
5. **Prüfen.** Einmal `python .ask/skills/pruefen.py <datei> --pdf` ausführen und die Folien mit `view_file` ansehen: Läuft Text über den Rand, ist etwas zu klein, ist ein Platzhalter leer? Beheben.
6. **Übergeben.** Nenne die Anzahl Folien und was noch fehlt (Bilder, Zahlen, eigene Beispiele).

## Regeln
- Pro Folie ein Gedanke, höchstens fünf Stichpunkte mit je höchstens zwölf Wörtern. Keine ganzen Absätze.
- Schrift mindestens 24 Punkt, guter Kontrast, höchstens zwei Schriften und wenige Farben.
- Lieber ein Bild, ein Diagramm oder ein Beispiel als eine Liste. Diagramme mit Titel und Achsenbeschriftung.
- Bilder nur verwenden, wenn die Person sie liefert, sie frei nutzbar sind oder du sie mit `bild_erstellen` erstellst; die Quelle nennen.
- Nichts erfinden: Zahlen und Zitate nur aus dem Material oder mit Quelle; Fehlendes als «[noch ergänzen]».
- Schweizer Rechtschreibung, «ss» statt des scharfen S, Anführungszeichen « ».

## Ohne Projekt
In einem Chat ohne Projekt kannst du keine Datei anlegen. Gib dann die Folienliste mit Stichpunkten und Notizen als Antwort und sag der Person, dass sie für die PowerPoint-Datei links bei «Projekte» ein Projekt anlegt.
