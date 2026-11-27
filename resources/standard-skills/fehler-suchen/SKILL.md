---
name: fehler-suchen
description: Sucht Fehler im Code oder in der Technik systematisch: nachstellen, Fehlermeldung lesen, eingrenzen, Ursache vermuten, klein beheben, prüfen. Verwenden, wenn etwas nicht läuft, abstürzt oder falsche Ergebnisse liefert.
---
# Fehler systematisch suchen

Ziel: Den Fehler beheben **und** der Person erklären, warum er entstand, damit sie ihn künftig selbst findet.

## Vorgehen
1. **Das Problem verstehen.** Was sollte passieren, was passiert stattdessen? Seit wann? Was wurde zuletzt geändert? Fehlende Angaben kurz erfragen.
2. **Nachstellen.** Den Fehler selbst auslösen, wenn möglich (Programm oder Test im Terminal ausführen). Ohne Nachstellen nicht raten.
3. **Fehlermeldung lesen.** Ganz lesen, von unten nach oben (bei Python: letzte Zeile = Art des Fehlers, darüber der Weg dorthin). Datei und Zeilennummer notieren. Der Person zeigen, wie man die Meldung liest.
4. **Eingrenzen.** Den kleinsten Teil finden, der den Fehler zeigt: Teile auskommentieren, Werte mit `print` oder dem Debugger ausgeben, mit kleinerer Eingabe testen.
5. **Vermutung aufstellen.** Eine konkrete Hypothese («Die Liste ist leer, darum …»), nicht mehrere auf einmal.
6. **Vermutung prüfen.** Mit einem Test oder einer Ausgabe bestätigen. Stimmt sie nicht, eine neue aufstellen.
7. **Minimal beheben.** Die kleinste Änderung, die die Ursache beseitigt, nicht nur das Symptom. Keine grossen Umbauten nebenbei.
8. **Prüfen.** Nochmals ausführen. Ist der Fehler weg? Funktioniert noch alles andere (vorhandene Tests laufen lassen)?
9. **Erklären.** Siehe unten.

## Typische Ursachen zuerst prüfen
- Tippfehler, falsche Gross-/Kleinschreibung, falscher Dateipfad, fehlende Datei.
- Falscher Typ (Text statt Zahl), leere Werte (`None`, `null`, leere Liste).
- Fehlende Pakete oder falsche Version, falsche Umgebung.
- Einrückung, Klammern, Anführungszeichen, Semikolons.
- Annahmen über Reihenfolge, Zeit oder Rechte (Datei gesperrt, kein Zugriff).
- Bei Geräten und Netzwerk: Kabel, Strom, Adresse, Firewall, Neustart als einfacher Test.

## Regeln
- Erst verstehen, dann ändern. Nicht wild Dinge ausprobieren.
- Immer nur eine Sache auf einmal ändern.
- Ändere nichts ausserhalb des Arbeitsordners und lösche nichts, ohne zu fragen.
- Findest du die Ursache nicht, sag es offen und schlag den nächsten Prüfschritt vor.
- Auf den Wunsch der Person achten: Will sie nur den Hinweis, lass den Code unverändert.

## Erklärung am Schluss
- **Ursache:** in einem Satz, in einfachen Worten.
- **Wie ich sie gefunden habe:** die 2 bis 3 wichtigsten Schritte.
- **Was ich geändert habe:** Datei und Stelle.
- **Tipp für das nächste Mal:** wie man solche Fehler früher bemerkt (Test, Ausgabe, Prüfung der Eingabe).
