---
name: commit-nachricht
description: Schreibt eine gute Git-Commit-Nachricht aus den aktuellen Änderungen. Verwenden, wenn die Person einen Commit machen will oder Hilfe bei der Commit-Nachricht, Zusammenfassung der Änderungen oder bei Git braucht.
---
# Commit-Nachricht schreiben

Ziel: Eine kurze, klare Nachricht, die in einem halben Jahr noch erklärt, was geändert wurde und warum.

## Vorgehen
1. **Änderungen ansehen** im Terminal im Arbeitsordner:
   - `git status` (welche Dateien?)
   - `git diff` (noch nicht vorgemerkt) und `git diff --staged` (vorgemerkt)
   - Gibt es noch keine Änderungen oder ist es kein Git-Ordner, der Person das sagen.
2. **Sprache und Stil des Projekts erkennen:** `git log --oneline -10`. Schreibt das Projekt Deutsch oder Englisch? Gibt es ein Muster wie `feat:` oder `fix:`? Dem folgen. Ohne Vorbild: Deutsch.
3. **Verstehen, was und warum.** Das Diff lesen. Ist der Zweck nicht erkennbar, die Person fragen.
4. **Mehrere Themen?** Sind unabhängige Änderungen gemischt, vorschlagen, sie in getrennten Commits zu machen.
5. **Nachricht vorschlagen** und zeigen.
6. **Nur nach Bestätigung committen.** `git add` und `git commit` nie ohne Ja der Person. Nie pushen, ausser sie verlangt es ausdrücklich.

## Format
```
Kurze Zusammenfassung in der Befehlsform (höchstens 72 Zeichen)

Optionaler Text: Warum wurde das geändert? Was ist neu oder anders?
Wichtige Folgen oder Einschränkungen. Zeilen bei etwa 72 Zeichen umbrechen.
```
- **Betreffzeile:** höchstens 72 Zeichen, kein Punkt am Ende, Befehlsform («Füge Login-Prüfung hinzu», «Behebe Absturz bei leerer Liste»).
- **Leerzeile** zwischen Betreff und Text.
- **Text:** erklärt das Warum, weniger das Wie (das Wie steht im Code). Bei Kleinigkeiten weglassen.
- Passt es zum Projekt, Präfixe wie `fix:`, `feat:`, `docs:` verwenden.
- Verweis auf Aufgaben (`Fixes #12`) nur, wenn die Person die Nummer nennt.

## Regeln
- Keine Allgemeinplätze wie «Änderungen», «Update», «bugfix».
- Nichts hineinschreiben, was nicht im Diff steht.
- Keine Geheimnisse committen. Entdeckst du Passwörter, Schlüssel oder `.env`-Dateien im Diff, stopp und warn die Person.
- Auf Wunsch eine bestehende Nachricht verbessern (`git commit --amend` nur, wenn noch nicht gepusht, und nach Rückfrage).
- Erkläre der Person kurz, warum die Nachricht so aufgebaut ist, wenn sie noch neu bei Git ist.

## Beispiel
```
Behebe Absturz bei leerer Einkaufsliste

Die Summe wurde durch die Anzahl Artikel geteilt, auch wenn keine
vorhanden waren. Jetzt wird bei einer leeren Liste 0 zurückgegeben.
```
