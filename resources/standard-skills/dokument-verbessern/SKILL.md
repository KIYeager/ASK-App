---
name: dokument-verbessern
description: Überarbeitet ein vorhandenes Word-Dokument oder eine Präsentation: Rechtschreibung, Sprache, Aufbau und Formatierung, ohne den Inhalt zu verändern. Verwenden, wenn die Person eine Datei korrigieren oder verschönern lassen will.
---
# Vorhandenes Dokument verbessern

Ziel: Dasselbe Dokument, nur besser. Der Inhalt und die Stimme der Person bleiben.

## Vorgehen
1. **Lesen.** Die Datei mit `read_file` ganz lesen; kommt es auf das Aussehen an (Layout, Folien), zusätzlich mit `view_file` ansehen.
2. **Umfang klären.** Frag, was gewünscht ist, wenn es nicht klar ist:
   - nur Rechtschreibung und Grammatik,
   - auch Sprache und Stil (kürzer, klarer, sachlicher),
   - auch Aufbau (Reihenfolge, Überschriften, Lücken),
   - auch Formatierung (einheitliche Schriften, Abstände, Tabellen, Folienlayout).
3. **Befund zeigen.** Liste die wichtigsten Punkte kurz auf, geordnet nach Wichtigkeit, bevor du viel änderst. Bei kleinen Korrekturen direkt ändern.
4. **Anleitung lesen.** Mit `read_file` die passende Datei lesen: `.ask/skills/word/SKILL.md` oder `.ask/skills/powerpoint/SKILL.md`, Abschnitt «Vorhandene Datei bearbeiten».
5. **Gezielt ändern.** Die Datei öffnen und nur die betroffenen Stellen ändern, nicht neu aufbauen: So bleiben Formatvorlagen, Kopf- und Fusszeilen, Bilder und Tabellen erhalten. Unter demselben Namen speichern; die App sichert den Stand davor, «Rückgängig» holt ihn zurück.
6. **Prüfen.** Einmal `python .ask/skills/pruefen.py <datei> --pdf` ausführen und mit `view_file` ansehen.
7. **Bericht.** Sag, was du geändert hast (zusammengefasst nach Art, mit zwei, drei Beispielen) und was du bewusst gelassen hast.

## Regeln
- Den Inhalt nicht verändern: keine neuen Aussagen, Zahlen oder Beispiele dazuerfinden, nichts weglassen, was die Person gemeint hat. Fehlt etwas, vorschlagen statt einfügen.
- Die Stimme der Person behalten: nicht alles umschreiben, nur was unklar, falsch oder holprig ist.
- Ist es eine Arbeit, die bewertet wird: Die Person muss sie selbst verstehen und vertreten können. Erkläre grössere Änderungen, statt sie nur vorzunehmen.
- Unsicheres nachfragen (Eigennamen, Fachbegriffe, gewollte Schreibweisen), nicht «korrigieren».
- Einheitlich machen, was gleich sein soll: Schreibweisen, Datumsform, Aufzählungszeichen, Überschriften.
- Schweizer Rechtschreibung, «ss» statt des scharfen S, Anführungszeichen « ».

## Ohne Projekt
In einem Chat ohne Projekt kannst du die Datei lesen, aber nicht ändern. Gib dann die Verbesserungen als Liste («Stelle → Vorschlag») und sag der Person, dass du die Datei in einem Projekt direkt überarbeiten kannst.
