---
name: tests-schreiben
description: Schreibt automatische Tests für vorhandenen Code und führt sie aus. Verwenden, wenn die Person Tests, Unit-Tests oder eine Prüfung ihres Programms möchte oder wissen will, ob der Code richtig funktioniert.
---
# Tests für vorhandenen Code schreiben

Ziel: Tests, die das Verhalten des Codes absichern, verständlich geschrieben und wirklich ausgeführt.

## Vorgehen
1. **Code verstehen.** Mit `read_file` lesen, was die Funktion oder das Modul tun soll. Unklar? Die Person fragen.
2. **Test-Werkzeug des Projekts finden.** Schau im Arbeitsordner nach:
   - Python: `pytest` oder `unittest`, Ordner `tests/`, Datei `pytest.ini`, `pyproject.toml`.
   - JavaScript/TypeScript: `package.json` (jest, vitest, mocha), Dateien `*.test.js`.
   - Java: JUnit, C#: xUnit/NUnit, sonst das Übliche der Sprache.
   Verwende, was schon da ist. Gibt es nichts, wähle das Einfachste der Sprache (Python: `pytest`, sonst `unittest`) und sag es der Person.
3. **Testfälle planen.** Liste sie kurz auf, bevor du schreibst:
   - **Normalfall:** typische, gültige Eingaben.
   - **Randfälle:** leer, 0, 1, sehr gross, Grenzwerte, Sonderzeichen.
   - **Fehlerfälle:** ungültige Eingaben, fehlende Datei; erwartet wird eine klare Fehlermeldung oder Ausnahme.
4. **Tests schreiben.** Eine Idee pro Test, sprechender Name (`test_leere_liste_gibt_null`), Aufbau: vorbereiten, ausführen, prüfen.
5. **Ausführen** im Terminal. Fehlt das Test-Paket, der Person sagen, was installiert werden muss, bevor du es installierst.
6. **Auswerten.**
   - Test schlägt fehl: Ist der Test falsch oder der Code? Liegt ein Fehler im Code, melde ihn, ändere den Code aber nur, wenn die Person es will.
   - Alle grün: kurz sagen, was abgedeckt ist und was nicht.
7. **Berichten:** Dateien, Anzahl Tests, Ergebnis, offene Lücken.

## Regeln
- Tests ablegen, wo das Projekt sie sonst hat (`tests/`, neben der Datei). Nicht den ursprünglichen Code ändern, nur um ihn testbar zu machen, ausser die Person stimmt zu.
- Keine Tests, die vom Zufall, der Uhrzeit, dem Netzwerk oder echten Daten abhängen. Bei Bedarf Testdaten oder temporäre Dateien verwenden.
- Keine echten Mails senden, nichts löschen, keine Zugangsdaten verwenden.
- Erwartete Werte selbst herleiten, nicht einfach die aktuelle Ausgabe des Codes übernehmen, sonst prüft der Test nichts.
- Tests einfach halten: Wenn die Person sie nicht lesen kann, helfen sie nicht beim Lernen. Wichtige Stellen kurz erklären.
- Sprache: Namen und Kommentare in der Sprache des Projekts.

## Mini-Beispiel (Python, pytest)
```python
from rechner import teile

def test_normal():
    assert teile(10, 2) == 5

def test_durch_null_gibt_fehler():
    import pytest
    with pytest.raises(ValueError):
        teile(1, 0)
```
Ausführen: `python -m pytest -q`
