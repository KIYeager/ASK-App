---
name: code-review
description: Prüft Code auf Richtigkeit, Lesbarkeit, Sicherheit und Namensgebung und listet Funde nach Wichtigkeit mit konkreten Vorschlägen. Verwenden, wenn die Person ihren Code kontrollieren oder Feedback dazu will.
---
# Code-Review

Ziel: Hilfreiches, freundliches Feedback, aus dem die Person etwas lernt. Es geht um den Code, nicht um die Person.

## Vorgehen
1. **Umfang klären.** Welche Dateien oder Änderungen? Bei einem Git-Projekt: `git status` und `git diff` zeigen die Änderungen. Sonst die genannte Datei mit `read_file` lesen. Frag, worauf die Person besonders achten soll.
2. **Zweck verstehen.** Was soll der Code tun? Wenn unklar, nachfragen, bevor du urteilst.
3. **Ausführen, wenn möglich.** Programm oder vorhandene Tests laufen lassen, um Vermutungen zu prüfen.
4. **Prüfen nach diesen Punkten:**
   - **Richtigkeit:** Tut er, was er soll? Randfälle (leer, 0, sehr gross), Fehlerbehandlung, Off-by-one, Typen.
   - **Sicherheit:** Passwörter oder Schlüssel im Code, ungeprüfte Eingaben, SQL aus Text zusammengebaut, Dateipfade aus Eingaben, unsichere Befehle.
   - **Lesbarkeit:** Klare Struktur, nicht zu lange Funktionen, keine Doppelungen, sinnvolle Kommentare (erklären das Warum).
   - **Namen:** Variablen und Funktionen sagen, was sie sind. Einheitlicher Stil.
   - **Wartbarkeit:** Tests, magische Zahlen, tote Codestücke, Abhängigkeiten.
5. **Funde nach Wichtigkeit ordnen** und schreiben.

## Aufbau der Antwort
1. **Gesamteindruck:** 2 bis 3 Sätze, mit etwas Gutem zuerst (nur Echtes loben).
2. **Wichtig (muss behoben werden):** Fehler, Sicherheitslücken, Datenverlust.
3. **Sollte verbessert werden:** Lesbarkeit, Struktur, fehlende Prüfungen.
4. **Kleinigkeiten (Nice-to-have):** Namen, Stil, Kommentare.
5. **Nächste Schritte:** was die Person als Erstes angehen kann.

Jeder Fund: **Stelle** (Datei, Zeile), **Problem** in einem Satz, **Warum es wichtig ist**, **konkreter Vorschlag** mit kurzem Codebeispiel.

## Regeln
- Freundlich und sachlich. «Hier könnte …» statt «Das ist falsch».
- Immer begründen. Wer das Warum versteht, macht es künftig selbst richtig.
- Nicht übertreiben: wenige wichtige Funde sind mehr wert als 40 Kleinigkeiten. Bei vielen gleichen Funden einmal erklären und die Stellen nennen.
- Nichts ändern, solange die Person nicht darum bittet. Biete an, die Verbesserungen umzusetzen.
- Unsichere Funde als Vermutung kennzeichnen.
- Vorhandenen Stil des Projekts respektieren, nicht den eigenen Geschmack durchsetzen.
