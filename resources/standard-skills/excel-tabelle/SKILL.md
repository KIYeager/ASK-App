---
name: excel-tabelle
description: Erstellt oder ergänzt eine Excel-Tabelle: Daten ordnen, rechnen mit Formeln, Summen pro Gruppe, Diagramm. Verwenden, wenn die Person Daten auswerten, eine Liste, ein Budget oder eine .xlsx-Datei will.
---
# Excel-Tabelle erstellen oder auswerten

Ziel: Eine Tabelle, in der man sieht, woher jede Zahl kommt, und die sich weiter benutzen lässt.

## Vorgehen
1. **Daten ansehen.** Liegen Daten vor (CSV, Excel, Text im Chat)? Zuerst lesen: Welche Spalten gibt es, was steht drin, was fehlt oder ist doppelt? Sag kurz, was du gefunden hast.
2. **Ziel klären.** Was soll herauskommen: eine saubere Liste, Summen pro Kategorie, ein Vergleich, ein Verlauf über die Zeit, ein Diagramm? Frag nach, wenn es nicht klar ist.
3. **Anleitung lesen.** Lies mit `read_file` die Datei `.ask/skills/excel/SKILL.md` und halte dich daran (CSV richtig einlesen, Blätter «Daten» und «Auswertung», Formeln, Zahlenformate).
4. **Tabelle bauen.** Rohdaten unverändert auf ein Blatt, die Auswertung auf ein zweites. Kopfzeile fett und fixiert, Filter, passende Spaltenbreiten, Beträge mit zwei Nachkommastellen und Einheit in der Überschrift.
5. **Rechnen mit Formeln.** Summen, Mittelwerte und Anteile als Formel in der Zelle, nicht als feste Zahl. Rechne die Werte zur Kontrolle auch im Skript aus und vergleiche.
6. **Diagramm, wenn es hilft.** Balken für Vergleiche, Linien für Verläufe, Kreis nur bei wenigen Teilen eines Ganzen. Mit Titel und Achsenbeschriftung.
7. **Prüfen.** Einmal `python .ask/skills/pruefen.py <datei>` ausführen; kommt es auf das Aussehen an, mit `view_file` ansehen.
8. **Übergeben.** Nenne die wichtigsten Ergebnisse in zwei, drei Sätzen und sag, auf welchem Blatt sie stehen.

## Regeln
- Die Rohdaten der Person nie verändern oder überschreiben: Auswertungen kommen auf ein eigenes Blatt oder in eine neue Datei.
- Auffälliges melden statt still korrigieren: leere Zellen, doppelte Zeilen, Ausreisser, gemischte Schreibweisen derselben Kategorie.
- Eine Tabelle pro Blatt, keine verbundenen Zellen in Datenbereichen, keine leeren Zeilen als Abstand.
- Datum als Datum, Zahl als Zahl (nicht als Text), in der Schweiz üblich: Dezimalpunkt und Tausenderstrich wie 1’234.50.
- Nichts erfinden: fehlende Werte bleiben leer und werden genannt.
- Schweizer Rechtschreibung, «ss» statt des scharfen S.

## Ohne Projekt
In einem Chat ohne Projekt kannst du keine Datei anlegen. Werte dann aus, was sich aus dem angehängten oder eingefügten Material sagen lässt, zeig eine kleine Tabelle in der Antwort und sag der Person, dass sie für die Excel-Datei links bei «Projekte» ein Projekt anlegt.
