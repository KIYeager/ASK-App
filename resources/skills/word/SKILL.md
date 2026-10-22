---
name: word
description: Word-Dokumente (.docx) mit python-docx sauber formatiert erstellen und prüfen. Lesen, bevor eine .docx-Datei entsteht.
---
# Word-Dokument erstellen

Werkzeug: python-docx. Immer zuerst ein Skript schreiben (z. B. `erstelle_dokument.py`), dann `python erstelle_dokument.py` ausführen.

## Regeln
- Struktur über Formatvorlagen, nicht über Schriftgrösse von Hand: `Title`, `Heading 1`, `Heading 2`, `List Bullet`, `List Number`, Tabellenstil `Table Grid`.
- Liegt in `.ask/skills/vorlagen/` eine Word-Vorlage (`.docx`), damit beginnen (siehe unten). Sonst die Grundeinstellungen aus dem Gerüst übernehmen.
- Schweizer Rechtschreibung: «ss» statt «ß», Anführungszeichen « ».
- Speichern unter `ausgabe/`. Bestehende Dateien nie überschreiben, sondern neuen Namen wählen.

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
doc.save("ausgabe/Anleitung.docx")
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

## Typische Fehler
- Absätze mit `\n` füllen statt mehrere `add_paragraph`: ergibt schlecht formatierten Text.
- Leere Absätze als Abstand: lieber `paragraph_format.space_after = Pt(6)`.
- Überschriften fett formatieren statt `add_heading`: dann fehlt die Gliederung.

## Prüfen
`python .ask/skills/pruefen.py ausgabe/Anleitung.docx --pdf`
Zeigt Überschriften, Tabellen, Bilder und Auffälligkeiten. Mit `--pdf` wandelt die App die Datei mit Word in PDF um (wenn Office installiert ist) und meldet Seitenzahl und Text. Fehler beheben und erneut prüfen.
