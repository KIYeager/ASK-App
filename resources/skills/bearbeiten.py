"""Hilfsfunktionen, um vorhandene Word-, Excel- und PowerPoint-Dateien zu ändern, ohne die Formatierung
zu zerstören. Einbinden in einem Skript unter .ask/skripte/:

    import sys; sys.path.insert(0, ".ask/skills")
    from bearbeiten import word_ersetzen, word_finde, word_absatz_nach, excel_risiken, pptx_ersetzen
"""
import copy
import re
import zipfile


# ---------- Text in Absätzen ersetzen (Word und PowerPoint) ----------

def _ersetze_in_runs(runs, alt, neu):
    """Ersetzt alt durch neu über Formatierungsabschnitte (runs) hinweg. Der neue Text übernimmt die
    Formatierung des Abschnitts, in dem der alte Text beginnt; alle anderen Abschnitte bleiben, wie sie sind."""
    if not alt:
        return 0
    anzahl = 0
    start = 0
    while True:
        text = "".join(r.text for r in runs)
        pos = text.find(alt, start)
        if pos < 0:
            return anzahl
        ende = pos + len(alt)
        offset = 0
        erster = True
        for run in runs:
            laenge = len(run.text)
            a, b = max(pos, offset), min(ende, offset + laenge)
            if a < b:
                vorher, nachher = run.text[: a - offset], run.text[b - offset:]
                run.text = vorher + (neu if erster else "") + nachher
                erster = False
            offset += laenge
        anzahl += 1
        start = pos + len(neu)


def _word_absaetze(doc):
    """Alle Absätze: Haupttext, Tabellen (auch verschachtelt), Inhaltssteuerelemente (z. B. Deckblatt
    der TF-Bern-Vorlage) und alle Kopf- und Fusszeilen, auch die von Abschnitten im Deckblatt."""
    from docx.oxml.ns import qn
    from docx.text.paragraph import Paragraph

    for p in doc.element.body.iter(qn("w:p")):
        yield Paragraph(p, doc)
    for rel in doc.part.rels.values():
        if rel.reltype.endswith(("/header", "/footer")):
            teil = rel.target_part
            for p in teil.element.iter(qn("w:p")):
                yield Paragraph(p, teil)


def word_ersetzen(doc, alt, neu):
    """Ersetzt Text im ganzen Dokument und behält fett, kursiv, Farbe usw. Liefert die Anzahl Stellen."""
    # Die Elemente selbst merken, nicht id(): python-docx legt die Hüllen laufend neu an, und Python
    # vergibt die Adresse einer freigegebenen Hülle gleich wieder. Dann gälten andere Absätze als «gesehen».
    gesehen = set()
    anzahl = 0
    for p in _word_absaetze(doc):
        if p._p in gesehen:
            continue
        gesehen.add(p._p)
        if alt in p.text:
            anzahl += _ersetze_in_runs(p.runs, alt, neu)
    return anzahl


def word_finde(doc, text):
    """Erster Absatz im Haupttext (auch in Tabellen und Inhaltssteuerelementen), der text enthält."""
    from docx.oxml.ns import qn
    from docx.text.paragraph import Paragraph

    for p in doc.element.body.iter(qn("w:p")):
        absatz = Paragraph(p, doc)
        if text in absatz.text:
            return absatz
    return None


def word_absatz_nach(absatz, text, stil=None):
    """Fügt nach absatz einen neuen Absatz ein. Ohne stil übernimmt er den Stil von absatz."""
    from docx.text.paragraph import Paragraph

    neu = copy.deepcopy(absatz._p)
    for kind in list(neu):
        if not kind.tag.endswith("}pPr"):
            neu.remove(kind)
    absatz._p.addnext(neu)
    p = Paragraph(neu, absatz._parent)
    if stil:
        p.style = stil
    if text:
        p.add_run(text)
    return p


def word_loeschen(absatz):
    """Entfernt einen Absatz."""
    absatz._p.getparent().remove(absatz._p)


# ---------- Excel ----------

def excel_risiken(pfad):
    """Was beim Öffnen und Speichern mit openpyxl verloren gehen oder sich ändern kann. Leere Liste: unbedenklich."""
    with zipfile.ZipFile(pfad) as z:
        namen = z.namelist()
        xml = "".join(z.read(n).decode("utf-8", "ignore") for n in namen
                      if n.startswith("xl/worksheets/") or n.startswith("xl/drawings/drawing"))
    risiken = []
    if any(n.endswith("vbaProject.bin") for n in namen):
        risiken.append("Makros gehen verloren")
    if re.search(r"<xdr:sp[ >]", xml):
        risiken.append("Formen und Textfelder gehen verloren")
    if any(n.startswith("xl/ctrlProps/") for n in namen):
        risiken.append("Formular-Steuerelemente gehen verloren")
    if any(n.startswith(("xl/slicers/", "xl/slicerCaches/", "xl/timelines/")) for n in namen):
        risiken.append("Datenschnitte und Zeitachsen gehen verloren")
    if "sparklineGroup" in xml:
        risiken.append("Sparklines gehen verloren")
    if any(n.startswith("xl/charts/chart") for n in namen):
        risiken.append("Diagramme bleiben, ihr Aussehen kann sich aber leicht ändern")
    if any(n.startswith("xl/pivotTables/") for n in namen):
        risiken.append("Pivot-Tabellen bleiben, werden aber erst in Excel aktualisiert")
    return risiken


# ---------- PowerPoint ----------

def _pptx_textrahmen(prs):
    def aus(formen):
        for form in formen:
            if form.has_text_frame:
                yield form.text_frame
            if getattr(form, "has_table", False) and form.has_table:
                for zeile in form.table.rows:
                    for zelle in zeile.cells:
                        yield zelle.text_frame
            if hasattr(form, "shapes"):  # Gruppe
                yield from aus(form.shapes)

    for folie in prs.slides:
        yield from aus(folie.shapes)
        if folie.has_notes_slide:
            yield folie.notes_slide.notes_text_frame


def pptx_ersetzen(prs, alt, neu):
    """Ersetzt Text auf allen Folien (auch Tabellen, Gruppen, Notizen) und behält die Formatierung."""
    anzahl = 0
    for rahmen in _pptx_textrahmen(prs):
        for p in rahmen.paragraphs:
            if alt in "".join(r.text for r in p.runs):
                anzahl += _ersetze_in_runs(p.runs, alt, neu)
    return anzahl
