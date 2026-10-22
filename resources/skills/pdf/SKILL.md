---
name: pdf
description: PDF-Dateien mit reportlab erstellen und mit pypdf lesen, zusammenfügen oder prüfen. Lesen, bevor eine .pdf-Datei entsteht.
---
# PDF erstellen oder bearbeiten

Werkzeuge: reportlab (erstellen), pypdf (lesen, zusammenfügen, teilen). Immer zuerst ein Skript schreiben, dann mit `python skript.py` ausführen.

## Welcher Weg?
- Dokument mit viel Text und Struktur, das auch als Word gebraucht wird: zuerst Word-Datei nach `.ask/skills/word/SKILL.md` bauen, dann mit `python .ask/skills/pruefen.py ausgabe/datei.docx --pdf` umwandeln lassen. Das PDF liegt danach in `.ask/pdf/`; nach `ausgabe/` kopieren.
- Reines PDF (Merkblatt, Bericht, Tabelle): reportlab mit Platypus wie im Gerüst.
- Bestehende PDFs lesen oder kombinieren: pypdf.

## Regeln
- Schrift Arial einbinden, damit Umlaute und Sonderzeichen sicher richtig erscheinen.
- Ränder 2 cm, Titel, Zwischentitel, Absätze und Tabellen über Stile, nicht von Hand positionieren.
- Speichern unter `ausgabe/`, nie eine bestehende Datei überschreiben.

## Gerüst (reportlab)
```python
from reportlab.lib import colors
from reportlab.lib.pagesizes import A4
from reportlab.lib.styles import getSampleStyleSheet, ParagraphStyle
from reportlab.lib.units import cm
from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.ttfonts import TTFont
from reportlab.platypus import SimpleDocTemplate, Paragraph, Spacer, Table, TableStyle, ListFlowable, ListItem

pdfmetrics.registerFont(TTFont("Arial", r"C:\Windows\Fonts\arial.ttf"))
pdfmetrics.registerFont(TTFont("Arial-Bold", r"C:\Windows\Fonts\arialbd.ttf"))
stile = getSampleStyleSheet()
text = ParagraphStyle("Text", parent=stile["BodyText"], fontName="Arial", fontSize=11, leading=15)
titel = ParagraphStyle("Titel", parent=stile["Title"], fontName="Arial-Bold")
zwischentitel = ParagraphStyle("Zwischentitel", parent=stile["Heading2"], fontName="Arial-Bold")

def fusszeile(canvas, dokument):
    canvas.setFont("Arial", 9)
    canvas.drawCentredString(A4[0] / 2, 1.2 * cm, f"Seite {dokument.page}")

inhalt = [
    Paragraph("Merkblatt Passwörter", titel),
    Paragraph("Warum sichere Passwörter wichtig sind", zwischentitel),
    Paragraph("Ein gutes Passwort ist lang und wird nur einmal verwendet.", text),
    ListFlowable([ListItem(Paragraph(p, text)) for p in ["Mindestens 12 Zeichen", "Passwort-Manager nutzen"]],
                 bulletType="bullet"),
    Spacer(1, 0.5 * cm),
]
daten = [["Methode", "Sicherheit"], ["Passwort-Manager", "hoch"], ["Zettel am Bildschirm", "tief"]]
tabelle = Table(daten, colWidths=[8 * cm, 5 * cm])
tabelle.setStyle(TableStyle([
    ("FONTNAME", (0, 0), (-1, -1), "Arial"), ("FONTNAME", (0, 0), (-1, 0), "Arial-Bold"),
    ("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#0B6E75")), ("TEXTCOLOR", (0, 0), (-1, 0), colors.white),
    ("GRID", (0, 0), (-1, -1), 0.5, colors.grey), ("VALIGN", (0, 0), (-1, -1), "TOP"),
]))
inhalt.append(tabelle)

doc = SimpleDocTemplate("ausgabe/Merkblatt.pdf", pagesize=A4, leftMargin=2 * cm, rightMargin=2 * cm,
                        topMargin=2 * cm, bottomMargin=2 * cm, title="Merkblatt Passwörter")
doc.build(inhalt, onFirstPage=fusszeile, onLaterPages=fusszeile)
print("gespeichert")
```

## pypdf
```python
from pypdf import PdfReader, PdfWriter
leser = PdfReader("eingabe.pdf"); print(len(leser.pages)); print(leser.pages[0].extract_text()[:500])
schreiber = PdfWriter()
for datei in ["teil1.pdf", "teil2.pdf"]:
    for seite in PdfReader(datei).pages:
        schreiber.add_page(seite)
schreiber.write("ausgabe/Zusammengefuegt.pdf")
```

## Prüfen
`python .ask/skills/pruefen.py ausgabe/Merkblatt.pdf`
Zeigt Seitenzahl, Seitengrösse und den Text jeder Seite. Fehlt Text, ist etwas schiefgelaufen.
