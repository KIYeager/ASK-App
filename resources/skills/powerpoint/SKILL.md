---
name: powerpoint
description: PowerPoint-Präsentationen (.pptx) mit python-pptx übersichtlich erstellen und prüfen. Lesen, bevor eine .pptx-Datei entsteht.
---
# PowerPoint-Präsentation erstellen

Werkzeug: python-pptx. Immer zuerst ein Skript schreiben (z. B. `praesentation.py`), dann `python praesentation.py` ausführen.

## Regeln
- Erst die Gliederung festlegen: Titelfolie, 4 bis 10 Inhaltsfolien, Schlussfolie.
- Pro Folie ein Gedanke, ein klarer Titel, höchstens 5 Stichpunkte mit je höchstens 12 Wörtern.
- Ausführliche Erklärungen gehören in die Notizen, nicht auf die Folie.
- Liegt in `.ask/skills/vorlagen/` eine PowerPoint-Vorlage (`.pptx`), damit beginnen und ihre Layouts verwenden.
- Format 16:9. Speichern unter `ausgabe/`, nie eine bestehende Datei überschreiben.

## Gerüst
```python
from pptx import Presentation
from pptx.util import Inches, Pt

prs = Presentation()                       # oder Presentation(".ask/skills/vorlagen/NAME.pptx")
prs.slide_width, prs.slide_height = Inches(13.333), Inches(7.5)
TITEL, TITEL_INHALT, NUR_TITEL = 0, 1, 5   # Layout-Nummern der Standardvorlage

folie = prs.slides.add_slide(prs.slide_layouts[TITEL])
folie.shapes.title.text = "Netzwerke verstehen"
folie.placeholders[1].text = "Lernende TF Bern · Oktober 2026"

def inhaltsfolie(titel, punkte, notiz=""):
    f = prs.slides.add_slide(prs.slide_layouts[TITEL_INHALT])
    f.shapes.title.text = titel
    rahmen = f.placeholders[1].text_frame
    rahmen.clear()
    for i, punkt in enumerate(punkte):
        absatz = rahmen.paragraphs[0] if i == 0 else rahmen.add_paragraph()
        absatz.text = punkt
        absatz.font.size = Pt(24)
    if notiz:
        f.notes_slide.notes_text_frame.text = notiz
    return f

inhaltsfolie("Was ist ein Netzwerk?", ["Geräte tauschen Daten aus", "Kabel oder Funk", "Regeln heissen Protokolle"],
             notiz="Beispiel aus dem Alltag nennen: WLAN zu Hause.")

# Bild auf einer Folie mit nur Titel:
# f = prs.slides.add_slide(prs.slide_layouts[NUR_TITEL]); f.shapes.title.text = "Aufbau"
# f.shapes.add_picture("diagramm.png", Inches(1.5), Inches(1.6), height=Inches(5.3))

prs.save("ausgabe/Praesentation.pptx")
print("gespeichert")
```

## Diagramm als Bild
Mit matplotlib ein PNG erzeugen (`plt.savefig("diagramm.png", dpi=200, bbox_inches="tight")`) und mit `add_picture` einfügen.

## Typische Fehler
- Zu viel Text: Folien mit mehr als 5 Punkten aufteilen.
- Breitbild vergessen: ohne Anpassung ist die Präsentation 4:3.
- Platzhalter leer lassen: `pruefen.py` meldet leere Platzhalter, diese füllen oder Layout wechseln.

## Prüfen
`python .ask/skills/pruefen.py ausgabe/Praesentation.pptx --pdf`
Zeigt pro Folie Titel, Anzahl Punkte und Warnungen bei zu viel Text. Mit `--pdf` wandelt die App die Präsentation mit PowerPoint in PDF um (falls installiert).
