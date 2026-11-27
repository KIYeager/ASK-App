---
name: word-dokument
description: Erstellt ein sauber formatiertes Word-Dokument (Bericht, Anleitung, Dokumentation, Brief, Protokoll). Verwenden, wenn die Person einen Text als Word-Datei oder .docx will.
---
# Word-Dokument erstellen

Ziel: Ein Dokument, das man so abgeben kann: klarer Aufbau, einheitliche Formatierung, richtiger Inhalt.

## Vorgehen
1. **Zweck klären.** Frag kurz nach, was fehlt (nicht alles auf einmal):
   - Was für ein Dokument ist es (Bericht, Anleitung, Dokumentation, Brief, Protokoll)?
   - Für wen (Lehrperson, Ausbildner, Kundschaft, Klasse) und wie lang ungefähr?
   - Gibt es Vorgaben (Vorlage der Schule, Pflichtkapitel, Titelblatt, Abgabeform)?
   - Liegt Material vor (Notizen, Stichworte, Dateien im Projekt)? Dann zuerst lesen.
2. **Gliederung zeigen.** Schlag die Überschriften in ein bis zwei Ebenen vor und frag, ob sie passt. Bei kurzen Dokumenten (bis eine Seite) direkt schreiben.
3. **Anleitung lesen.** Lies mit `read_file` die Datei `.ask/skills/word/SKILL.md` und halte dich daran (Formatvorlagen, A4, Vorlage aus `.ask/skills/vorlagen/`, falls vorhanden).
4. **Dokument bauen.** Schreib den Inhalt aus dem Material der Person. Überschriften mit Formatvorlagen, Aufzählungen als Listen, Zahlen in Tabellen, Seitenzahl in der Fusszeile. Ab etwa drei Seiten ein Inhaltsverzeichnis.
5. **Prüfen.** Einmal `python .ask/skills/pruefen.py <datei> --pdf` ausführen und das Ergebnis mit `view_file` ansehen. Gemeldete Fehler und was schief aussieht beheben.
6. **Übergeben.** Sag in zwei, drei Sätzen, was im Dokument steht und was die Person noch ergänzen muss.

## Regeln
- Nichts erfinden: Namen, Daten, Zahlen und Zitate nur aus dem Material der Person oder mit Quelle. Fehlendes als «[noch ergänzen]» markieren und am Schluss auflisten.
- Kurze Sätze, aktive Sprache, Fachwörter beim ersten Auftreten erklären.
- Einheitlich bleiben: eine Schrift, dieselben Abstände, dieselbe Schreibweise für gleiche Dinge.
- Bilder und Tabellen bekommen eine Beschriftung und werden im Text erwähnt.
- Schweizer Rechtschreibung, «ss» statt des scharfen S, Anführungszeichen « ».
- Den langen Text nicht zusätzlich in den Chat schreiben: Er steht im Dokument.

## Ohne Projekt
In einem Chat ohne Projekt kannst du keine Datei anlegen. Schreib dann den Text gegliedert in die Antwort und sag der Person, dass sie für die Word-Datei links bei «Projekte» ein Projekt anlegt.
