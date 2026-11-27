---
name: skill-erstellen
description: Führt Schritt für Schritt durch das Schreiben eines eigenen Skills (wiederverwendbare Anleitung für ASK). Verwenden, wenn die Person einen eigenen Skill, eine Vorlage oder eine feste Arbeitsweise für ASK anlegen oder verbessern möchte.
---
# Eigenen Skill erstellen

Ein Skill ist eine kurze Anleitung, die ASK bei einer bestimmten Aufgabe nachliest. ASK sieht im Alltag nur **Name** und **Beschreibung**. Die ganze Anleitung wird erst gelesen, wenn die Beschreibung zur Aufgabe passt. Die Beschreibung ist also der Auslöser und das Wichtigste am Skill.

## Vorgehen
0. **Gibt es ihn schon?** Mit `katalog_suchen` (art «skill») nachsehen. Passt ein Eintrag, ihn mit `katalog_hinzufuegen` anbieten (die Person bestätigt), statt neu zu schreiben.
1. **Aufgabe klären.** Stell der Person wenige, klare Fragen (nicht alle auf einmal):
   - Welche Aufgabe kommt immer wieder vor?
   - Wann soll ASK den Skill brauchen? Welche Wörter oder Situationen sind typisch?
   - Welche Schritte machst du selbst, wenn du die Aufgabe von Hand löst?
   - Gibt es ein Beispiel für ein gutes Ergebnis (Text, Datei, Format)?
   - Gibt es Regeln, die immer gelten (Sprache, Aufbau, Länge, Dinge, die nie vorkommen sollen)?
2. **Entwurf zeigen.** Schreib Name, Beschreibung und Anleitung und zeig sie der Person. Frag, was fehlt oder zu viel ist.
3. **Nach Bestätigung speichern** mit dem Werkzeug `skill_erstellen`. Nie ohne Ja der Person speichern.
4. **Ausprobieren.** Sag der Person, dass sie den Skill mit `/name Text` aufrufen kann. Biete an, ihn nach einem Test zu verbessern.

## Name
- Klein geschrieben, nur a-z, 0-9 und Bindestriche, kurz und sprechend (`wochenbericht`, `vokabeln-abfragen`).
- Keine Umlaute, keine Leerzeichen.

## Beschreibung (der Auslöser)
- Höchstens etwa 300 Zeichen, ein bis zwei Sätze.
- Sagt, **was** der Skill macht **und wann** er gebraucht wird.
- Nennt typische Stichwörter, damit ASK ihn zur richtigen Zeit findet.
- Schlecht: «Hilft bei Berichten.» Gut: «Schreibt den Wochenbericht für die Berufsschule aus Stichworten. Verwenden, wenn die Person einen Wochenbericht oder ein Lernjournal braucht.»

## Anleitung schreiben
- Befehlsform und konkrete Schritte: «Frag nach …», «Schreib in der Ich-Form …».
- Kurz bleiben. Nur das schreiben, was ASK nicht schon von selbst richtig macht. 20 bis 80 Zeilen sind meist genug.
- Zuerst das Wichtigste: Ziel, dann Schritte, dann feste Regeln.
- Feste Regeln mit Grund begründen («weil die Lehrperson …»), das hilft ASK bei Sonderfällen.
- Ein kurzes Beispiel (Eingabe und gewünschtes Ergebnis) wirkt stärker als lange Erklärungen.
- Auch sagen, was ASK **nicht** tun soll (nichts erfinden, nachfragen statt raten).
- Schweizer Rechtschreibung, wenn die Texte für die Schweiz sind: «ss» statt «ß», Anführungszeichen « ».
- Soll der Skill Dateien erzeugen: sagen, wo sie gespeichert werden und wie sie heissen.
- Soll ein anderer Skill helfen (z. B. Word oder PowerPoint): `skill_lesen("word")` in den Schritten nennen.

## Gerüst
```
---
name: wochenbericht
description: Schreibt aus Stichworten einen Wochenbericht in der Ich-Form. Verwenden, wenn die Person einen Wochenbericht verlangt.
---
# Wochenbericht schreiben

1. Frag nach den Stichworten der Woche und nach Problemen.
2. Schreib in der Ich-Form, 5 bis 8 Sätze, ohne erfundene Details.
3. Schluss: ein Satz zu dem, was ich nächste Woche mache.

Beispiel: «Mo: Switch konfiguriert» wird zu «Am Montag habe ich einen Switch konfiguriert.»
```

## Prüfen vor dem Speichern
- Würde ASK anhand der Beschreibung merken, wann der Skill dran ist?
- Sind die Schritte so klar, dass eine fremde Person sie befolgen könnte?
- Steht nichts darin, was nur für einen einzelnen Auftrag gilt?
