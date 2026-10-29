---
name: word
description: Word-Dokumente (.docx) mit python-docx sauber formatiert erstellen, vorhandene bearbeiten und prüfen. Lesen, bevor eine .docx-Datei entsteht oder geändert wird.
---
# Word-Dokument erstellen oder bearbeiten

Werkzeug: python-docx. Immer zuerst ein Skript schreiben (z. B. `.ask/skripte/erstelle_dokument.py`), dann `python .ask/skripte/erstelle_dokument.py` ausführen.

## Regeln
- Struktur über Formatvorlagen, nicht über Schriftgrösse von Hand: `Title`, `Heading 1`, `Heading 2`, `List Bullet`, `List Number`, Tabellenstil `Table Grid`.
- Liegt in `.ask/skills/vorlagen/` eine Word-Vorlage (`.docx`), damit beginnen (siehe unten). Sonst die Grundeinstellungen aus dem Gerüst übernehmen.
- Schweizer Rechtschreibung: «ss» statt «ß», Anführungszeichen « ».
- Neue Dateien im Arbeitsordner speichern, dort wo sie hingehören, mit sprechendem Namen. Vorhandene Dateien direkt ändern und unter demselben Namen speichern, keine Kopien wie `_V2`. Die App sichert den Ordner vor jedem Auftrag, die Person kann ihn rückgängig machen.
- Soll ein vorhandenes Dokument geändert werden: nicht neu aufbauen, sondern öffnen und nur die verlangten Stellen ändern (siehe «Vorhandene Datei bearbeiten»).

## Gerüst
```python
from docx import Document
from docx.shared import Pt, Cm
from docx.enum.text import WD_ALIGN_PARAGRAPH
from docx.oxml import OxmlElement
from docx.oxml.ns import qn

doc = Document()
normal = doc.styles["Normal"]
normal.font.name = "Arial"
normal.font.size = Pt(11)
for section in doc.sections:
    section.page_width, section.page_height = Cm(21), Cm(29.7)   # A4, nicht US-Letter
    section.top_margin = section.bottom_margin = Cm(2.5)
    section.left_margin = section.right_margin = Cm(2.5)

def seitenzahl(absatz):
    """Fügt das Feld PAGE ein (Seitenzahl in der Fusszeile)."""
    run = absatz.add_run()
    for typ, text in (("begin", None), (None, "PAGE"), ("end", None)):
        if typ:
            el = OxmlElement("w:fldChar"); el.set(qn("w:fldCharType"), typ)
        else:
            el = OxmlElement("w:instrText"); el.set(qn("xml:space"), "preserve"); el.text = text
        run._r.append(el)

fuss = doc.sections[0].footer.paragraphs[0]
fuss.alignment = WD_ALIGN_PARAGRAPH.CENTER
seitenzahl(fuss)

doc.add_paragraph("Titel des Dokuments", style="Title")
doc.add_heading("Einleitung", level=1)
doc.add_paragraph("Ein normaler Absatz mit vollständigen Sätzen.")
doc.add_paragraph("Erster Punkt", style="List Bullet")
doc.add_paragraph("Zweiter Punkt", style="List Bullet")
doc.add_paragraph("Schritt eins", style="List Number")

daten = [("Begriff", "Erklärung"), ("RAM", "Arbeitsspeicher"), ("CPU", "Prozessor")]
tabelle = doc.add_table(rows=0, cols=2, style="Table Grid")
for i, zeile in enumerate(daten):
    zellen = tabelle.add_row().cells
    for zelle, text in zip(zellen, zeile):
        zelle.text = text
        if i == 0:
            zelle.paragraphs[0].runs[0].bold = True

# Bild: doc.add_picture("bild.png", width=Cm(15))
doc.save("Anleitung.docx")
print("gespeichert")
```

## Mit Vorlage beginnen
```python
doc = Document(".ask/skills/vorlagen/NAME.docx")
body = doc.element.body
for el in list(body):
    if el.tag != qn("w:sectPr"):      # Inhalt der Vorlage leeren, Kopf/Fuss und Ränder behalten
        body.remove(el)
```

## Vorhandene Datei bearbeiten
Zuerst mit `read_file` lesen. Dann öffnen, nur die verlangten Stellen ändern, speichern. So bleiben Formatvorlagen, Kopf- und Fusszeilen, Bilder und Tabellen erhalten.
Die Datei wird direkt geändert und unter demselben Namen gespeichert. Will die Person ausdrücklich eine Kopie, `ziel` auf einen neuen Namen setzen.
```python
import sys; sys.path.insert(0, ".ask/skills")
from docx import Document
from bearbeiten import word_ersetzen, word_finde, word_absatz_nach, word_loeschen

quelle = "bericht.docx"
ziel = quelle                      # direkt ändern
doc = Document(quelle)
print(word_ersetzen(doc, "Proxmox 7", "Proxmox 8"), "Stellen ersetzt")   # auch in Tabellen, Kopf- und Fusszeilen
titel = word_finde(doc, "Installation")
if titel:
    neu = word_absatz_nach(titel, "Vor der Installation ein Backup erstellen.", stil="Normal")
    word_absatz_nach(neu, "Backup auf externe Festplatte", stil="List Bullet")
alt = word_finde(doc, "Dieser Abschnitt ist veraltet")
if alt:
    word_loeschen(alt)
doc.save(ziel)
print("gespeichert")
```
- Text nie mit `absatz.text = …` ersetzen: das löscht fett, kursiv und Links im ganzen Absatz. `word_ersetzen` behält die Formatierung.
- Meldet `word_ersetzen` 0 Stellen, steht der Text anders im Dokument (Leerzeichen, Silbentrennung): mit `read_file` nachsehen.
- Nicht zuverlässig möglich: Änderungen nachverfolgen, Kommentare, Textfelder, SmartArt, Inhaltsverzeichnis aktualisieren (das macht Word beim Öffnen mit F9). Das der Person sagen statt es zu versuchen.
- Danach wie unten prüfen. Lässt sich die Datei nicht speichern, ist sie in Word offen: die Person bitten, sie zu schliessen.

## Typische Fehler
- Absätze mit `\n` füllen statt mehrere `add_paragraph`: ergibt schlecht formatierten Text.
- Leere Absätze als Abstand: lieber `paragraph_format.space_after = Pt(6)`.
- Überschriften fett formatieren statt `add_heading`: dann fehlt die Gliederung.

## Prüfen
`python .ask/skills/pruefen.py Anleitung.docx --pdf`
Zeigt Überschriften, Tabellen, Bilder und Auffälligkeiten. Mit `--pdf` wandelt die App die Datei mit Word in PDF um (wenn Office installiert ist) und meldet Seitenzahl und Text. Fehler beheben und erneut prüfen.
