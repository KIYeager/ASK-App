"""Prüft eine erzeugte Datei und meldet Inhalt und Auffälligkeiten.

Aufruf:  python .ask/skills/pruefen.py <datei> [--pdf]
--pdf:   Word-, Excel- oder PowerPoint-Datei von der App mit Office in PDF umwandeln lassen
         und das PDF ebenfalls prüfen (nur wenn Office auf dem Laptop installiert ist).
Exitcode 0 = keine Fehler, 1 = Fehler gefunden oder Datei nicht lesbar,
2 = Datei in Ordnung, aber die verlangte PDF-Prüfung war nicht möglich.
"""
import json
import os
import re
import sys
import time
import uuid
import zipfile
from pathlib import Path

ORDNER = Path(__file__).resolve().parents[2]  # Arbeitsordner (.ask/skills/pruefen.py)
PLATZHALTER = re.compile(r"lorem ipsum|\bTODO\b|\bXXX\b|\[\.\.\.\]|\[hier [^\]]*\]|platzhalter", re.I)
FEHLERWERTE = ("#DIV/0!", "#REF!", "#NAME?", "#VALUE!", "#N/A", "#NUM!", "#NULL!", "#WERT!", "#BEZUG!", "#NAME?")

warnungen = []
fehler = []


def warn(text):
    warnungen.append(text)


def kurz(text, n=300):
    text = " ".join(str(text).split())
    return text if len(text) <= n else text[:n] + " …"


def pruefe_docx(pfad):
    from docx import Document
    doc = Document(pfad)
    absaetze = doc.paragraphs
    # Abschnitte direkt im XML zählen: python-docx sieht Abschnittswechsel in Inhaltssteuerelementen
    # (z. B. im Deckblatt der TF-Bern-Vorlage) nicht und liefert dann zu wenige doc.sections.
    with zipfile.ZipFile(pfad) as z:
        xml = z.read("word/document.xml").decode("utf-8", "ignore")
        teile = [n for n in z.namelist() if re.match(r"word/_rels/(header|footer)\d*\.xml\.rels$", n)]
        kopf_bilder = sum(z.read(n).decode("utf-8", "ignore").count("/image\"") for n in teile)
    abschnitte = len(re.findall(r"<w:sectPr[ >]", xml))
    print(f"Absätze: {len(absaetze)}, Tabellen: {len(doc.tables)}, Abschnitte: {abschnitte}")
    if abschnitte > len(doc.sections):
        warn(f"{abschnitte - len(doc.sections)} Abschnittswechsel stecken in Inhaltssteuerelementen (z. B. Deckblatt). "
             "doc.sections und section.header erreichen diese Abschnitte nicht; ihre Kopf- und Fusszeilen über "
             "doc.element.body.iter(qn('w:sectPr')) und die Beziehungen suchen.")
    bilder = sum(1 for r in doc.part.rels.values() if "image" in r.reltype)
    print(f"Bilder: {bilder} im Text, {kopf_bilder} in Kopf- und Fusszeilen")
    s = doc.sections[0]
    if s.page_width:
        print(f"Seite: {s.page_width.cm:.1f} × {s.page_height.cm:.1f} cm, Ränder links/rechts "
              f"{s.left_margin.cm:.1f}/{s.right_margin.cm:.1f} cm")
        a4 = sorted((round(s.page_width.cm, 1), round(s.page_height.cm, 1))) == [21.0, 29.7]
        if not a4:
            warn("Seitenformat ist nicht A4. section.page_width = Cm(21), section.page_height = Cm(29.7) setzen.")
    print("Gliederung:")
    ueberschriften = 0
    for p in absaetze:
        name = p.style.name if p.style is not None else ""
        if name.startswith("Heading") or name == "Title" or name.startswith("Überschrift"):
            ueberschriften += 1
            ebene = 0 if name == "Title" else int(re.sub(r"\D", "", name) or 1)
            print(f"  {'  ' * ebene}- {kurz(p.text, 90)} ({name})")
    if ueberschriften == 0:
        warn("Keine Überschriften mit Formatvorlage. add_heading bzw. style='Heading 1' verwenden.")
    leer_folge = 0
    max_leer = 0
    for p in absaetze:
        leer_folge = leer_folge + 1 if not p.text.strip() and not p._p.xpath(".//w:drawing") else 0
        max_leer = max(max_leer, leer_folge)
    if max_leer >= 3:
        warn(f"{max_leer} leere Absätze hintereinander. Abstände über Absatzformat statt Leerzeilen.")
    for t_nr, t in enumerate(doc.tables, 1):
        zeilen, spalten = len(t.rows), len(t.columns)
        print(f"Tabelle {t_nr}: {zeilen} × {spalten}, Kopf: {kurz(' | '.join(c.text for c in t.rows[0].cells), 120) if zeilen else '-'}")
        if zeilen and all(not c.text.strip() for c in t.rows[-1].cells):
            warn(f"Tabelle {t_nr} hat eine leere letzte Zeile.")
    lang = [p for p in absaetze if len(p.text) > 1500]
    if lang:
        warn(f"{len(lang)} sehr lange Absätze (über 1500 Zeichen). Eventuell aufteilen.")
    text = "\n".join(p.text for p in absaetze)
    if PLATZHALTER.search(text):
        warn(f"Platzhaltertext gefunden: «{PLATZHALTER.search(text).group(0)}»")
    if "ß" in text:
        warn("Text enthält «ß». In der Schweiz «ss» schreiben.")
    print(f"Wörter: {len(text.split())}")
    print(f"Anfang: {kurz(text, 500)}")


def pruefe_xlsx(pfad):
    from openpyxl import load_workbook
    wb = load_workbook(pfad)
    werte = load_workbook(pfad, data_only=True)
    print(f"Blätter: {', '.join(wb.sheetnames)}")
    formeln_ohne_wert = 0
    for ws in wb.worksheets:
        wsv = werte[ws.title]
        formeln = 0
        print(f"Blatt «{ws.title}»: Bereich {ws.dimensions}, {ws.max_row} Zeilen × {ws.max_column} Spalten,"
              f" fixiert: {ws.freeze_panes or 'nein'}, Diagramme: {len(ws._charts)}")
        if ws.max_row <= 1 and ws.max_column <= 1 and ws.cell(1, 1).value is None:
            warn(f"Blatt «{ws.title}» ist leer.")
        for zeile in ws.iter_rows():
            for z in zeile:
                if isinstance(z.value, str) and z.value.startswith("="):
                    formeln += 1
                    wert = wsv[z.coordinate].value
                    if wert is None:
                        formeln_ohne_wert += 1
                    elif isinstance(wert, str) and wert.upper() in FEHLERWERTE:
                        fehler.append(f"{ws.title}!{z.coordinate}: Formel ergibt {wert} ({z.value})")
                    if re.search(r"=\s*(SUMME|MITTELWERT|WENN|ANZAHL|RUNDEN)\(", z.value, re.I):
                        fehler.append(f"{ws.title}!{z.coordinate}: Deutscher Funktionsname {z.value}. In openpyxl englische Namen (SUM, AVERAGE, IF, COUNT, ROUND).")
                elif isinstance(z.value, str) and z.value.upper() in FEHLERWERTE:
                    fehler.append(f"{ws.title}!{z.coordinate}: Fehlerwert {z.value}")
        if formeln:
            print(f"  Formeln: {formeln}")
        for nr, zeile in enumerate(ws.iter_rows(min_row=1, max_row=min(ws.max_row, 5), values_only=True), 1):
            print(f"  Zeile {nr}: {kurz(' | '.join('' if v is None else str(v) for v in zeile), 160)}")
        # Kopfzeile ist die erste nicht leere Zeile (Vorlagen beginnen oft mit Titel- oder Leerzeilen)
        kopf = next((zeile for zeile in ws.iter_rows(min_row=1, max_row=min(ws.max_row, 30))
                     if any(z.value not in (None, "") for z in zeile)), None)
        if kopf and ws.max_row > kopf[0].row:
            erste = next(z for z in kopf if z.value not in (None, ""))
            if erste.font is not None and not erste.font.bold:
                warn(f"Blatt «{ws.title}»: Kopfzeile (Zeile {erste.row}) ist nicht fett.")
    if formeln_ohne_wert:
        print(f"Hinweis: {formeln_ohne_wert} Formeln haben noch keinen berechneten Wert. Excel berechnet sie beim "
              "Öffnen. Kontrollwerte in Python ausrechnen oder mit --pdf von Excel berechnen lassen.")


def pruefe_pptx(pfad):
    from pptx import Presentation
    from pptx.util import Emu
    prs = Presentation(pfad)
    breite, hoehe = Emu(prs.slide_width).inches, Emu(prs.slide_height).inches
    print(f"Folien: {len(prs.slides)}, Format: {breite:.2f} × {hoehe:.2f} Zoll"
          f" ({'16:9' if abs(breite / hoehe - 16 / 9) < 0.02 else '4:3' if abs(breite / hoehe - 4 / 3) < 0.02 else 'anderes'})")
    if abs(breite / hoehe - 16 / 9) >= 0.02:
        warn("Präsentation ist nicht im Format 16:9.")
    for nr, folie in enumerate(prs.slides, 1):
        titel = folie.shapes.title.text if folie.shapes.title is not None and folie.shapes.title.has_text_frame else ""
        punkte = 0
        zeichen = 0
        bilder = 0
        for form in folie.shapes:
            if form.shape_type == 13:
                bilder += 1
            if form.is_placeholder and form.has_text_frame and not form.text_frame.text.strip() \
                    and form.placeholder_format.type not in (None,):
                warn(f"Folie {nr}: leerer Platzhalter «{form.name}».")
            if form.has_text_frame and form != folie.shapes.title:
                for absatz in form.text_frame.paragraphs:
                    t = "".join(r.text for r in absatz.runs).strip()
                    if t:
                        punkte += 1
                        zeichen += len(t)
                        if len(t.split()) > 20:
                            warn(f"Folie {nr}: Punkt mit {len(t.split())} Wörtern: «{kurz(t, 60)}»")
                # Grobe Schätzung, ob Text aus dem Rahmen läuft
                if form.width and form.height:
                    groesse = 18
                    for absatz in form.text_frame.paragraphs:
                        for r in absatz.runs:
                            if r.font.size:
                                groesse = r.font.size.pt
                    platz = (Emu(form.width).pt / (groesse * 0.5)) * (Emu(form.height).pt / (groesse * 1.25))
                    text = form.text_frame.text
                    if len(text) > platz * 0.9 and len(text) > 40:
                        warn(f"Folie {nr}: Text in «{form.name}» passt vermutlich nicht in den Rahmen "
                             f"({len(text)} Zeichen, Platz für etwa {int(platz)}).")
        notiz = folie.notes_slide.notes_text_frame.text if folie.has_notes_slide else ""
        print(f"  Folie {nr}: «{kurz(titel, 60) or 'ohne Titel'}», {punkte} Textzeilen, {bilder} Bilder"
              f"{', Notizen' if notiz.strip() else ''}")
        if not titel.strip():
            warn(f"Folie {nr} hat keinen Titel.")
        if punkte > 7:
            warn(f"Folie {nr}: {punkte} Textzeilen. Höchstens 5 bis 6, sonst aufteilen.")


def pruefe_pdf(pfad):
    from pypdf import PdfReader
    leser = PdfReader(pfad)
    seiten = len(leser.pages)
    print(f"Seiten: {seiten}")
    if seiten:
        box = leser.pages[0].mediabox
        b, h = float(box.width) / 72 * 2.54, float(box.height) / 72 * 2.54
        print(f"Seitengrösse: {b:.1f} × {h:.1f} cm{' (A4)' if abs(b - 21) < 0.3 and abs(h - 29.7) < 0.3 else ''}")
    ohne_text = 0
    for nr, seite in enumerate(leser.pages[:20], 1):
        text = seite.extract_text() or ""
        if not text.strip():
            ohne_text += 1
        print(f"  Seite {nr}: {kurz(text, 250) or '(kein Text)'}")
    if seiten > 20:
        print(f"  … {seiten - 20} weitere Seiten")
    if ohne_text:
        warn(f"{ohne_text} Seite(n) ohne lesbaren Text (leer oder nur Bild).")
    if seiten and ohne_text == seiten:
        fehler.append("Das PDF enthält keinen Text.")


def pruefe_csv(pfad):
    import pandas as pd
    for enc in ("utf-8-sig", "cp1252"):
        try:
            df = pd.read_csv(pfad, sep=None, engine="python", encoding=enc)
            break
        except UnicodeDecodeError:
            continue
    else:
        fehler.append("Kodierung nicht erkannt")
        return
    print(f"Kodierung: {enc}, {len(df)} Zeilen × {len(df.columns)} Spalten")
    print(f"Spalten: {', '.join(map(str, df.columns))}")
    print(df.head(5).to_string())
    leer = df.isna().sum()
    for spalte, anzahl in leer.items():
        if anzahl:
            warn(f"Spalte «{spalte}»: {anzahl} leere Werte")


def pruefe_py(pfad):
    # Nur übersetzen, nichts schreiben (py_compile mit einer Zieldatei wie nul scheitert)
    try:
        with open(pfad, "rb") as f:
            compile(f.read(), str(pfad), "exec")
        print("Syntax: in Ordnung. Jetzt mit python ausführen und die Ausgabe prüfen.")
    except (SyntaxError, ValueError) as e:
        zeile = f" (Zeile {e.lineno})" if getattr(e, "lineno", None) else ""
        fehler.append(f"Syntaxfehler{zeile}: {getattr(e, 'msg', None) or e}")


def office_sicher(pfad):
    """Lehnt Dateien mit Makros oder externen Verweisen ab, bevor die App sie mit Office öffnet."""
    with zipfile.ZipFile(pfad) as z:
        for name in z.namelist():
            if re.search(r"vbaProject|activeX|oleObject|embeddings/.*\.(bin|exe|dll|js|vbs|ps1|bat|cmd)$", name, re.I):
                return f"enthält {name}"
    return ""


def als_pdf(pfad):
    """Lässt die App die Datei mit Office in PDF umwandeln (Auftrag über .ask/auftraege)."""
    grund = office_sicher(pfad)
    if grund:
        fehler.append(f"PDF-Umwandlung abgelehnt: Datei {grund}.")
        return None
    auftraege = ORDNER / ".ask" / "auftraege"
    auftraege.mkdir(parents=True, exist_ok=True)
    kennung = uuid.uuid4().hex[:12]
    auftrag = {"typ": "pdf", "quelle": os.path.relpath(pfad, ORDNER)}
    tmp = auftraege / f"{kennung}.tmp"
    tmp.write_text(json.dumps(auftrag), encoding="utf-8")
    tmp.rename(auftraege / f"{kennung}.json")
    print("PDF-Umwandlung durch die App angefragt, warte (bis 120 s) …", flush=True)
    ende = time.time() + 120
    while time.time() < ende:
        ok = auftraege / f"{kennung}.ok"
        nok = auftraege / f"{kennung}.fehler"
        if ok.exists():
            ziel = ORDNER / ok.read_text(encoding="utf-8").strip()
            ok.unlink(missing_ok=True)
            print(f"PDF erstellt: {os.path.relpath(ziel, ORDNER)}")
            return ziel
        if nok.exists():
            grund = nok.read_text(encoding="utf-8").strip()
            nok.unlink(missing_ok=True)
            pdf_fehlt(grund)
            return None
        time.sleep(0.5)
    pdf_fehlt("keine Antwort der App")
    return None


pdf_nicht_geprueft = []


def pdf_fehlt(grund):
    pdf_nicht_geprueft.append(grund)
    warn(f"PDF-Prüfung NICHT durchgeführt ({grund}). Seitenzahl, Seitenumbrüche und Inhaltsverzeichnis sind "
         "nicht geprüft. Nicht wiederholen und nicht selbst Word starten: am Schluss der Person sagen, dass sie "
         "das Dokument in Word ansehen soll.")


def main():
    argumente = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not argumente:
        print(__doc__)
        return 1
    pfad = Path(argumente[0])
    if not pfad.is_absolute():
        pfad = (Path.cwd() / pfad).resolve()
    if not pfad.exists():
        print(f"FEHLER: {argumente[0]} gibt es nicht.")
        return 1
    endung = pfad.suffix.lower()
    print(f"Datei: {os.path.relpath(pfad, ORDNER)} ({pfad.stat().st_size / 1024:.1f} KB)")
    pruefer = {".docx": pruefe_docx, ".xlsx": pruefe_xlsx, ".xlsm": pruefe_xlsx, ".pptx": pruefe_pptx,
               ".pdf": pruefe_pdf, ".csv": pruefe_csv, ".py": pruefe_py}.get(endung)
    try:
        if pruefer:
            pruefer(pfad)
        else:
            print("Für diesen Dateityp gibt es keine Detailprüfung.")
        if "--pdf" in sys.argv and endung in (".docx", ".xlsx", ".pptx"):
            pdf = als_pdf(pfad)
            if pdf:
                print("--- PDF ---")
                pruefe_pdf(pdf)
    except Exception as e:  # Datei kaputt oder falsches Format
        fehler.append(f"Datei lässt sich nicht lesen: {type(e).__name__}: {e}")
    print("--- Ergebnis ---")
    for f in fehler:
        print(f"FEHLER: {f}")
    for w in warnungen:
        print(f"WARNUNG: {w}")
    if not fehler and not warnungen:
        print("Keine Auffälligkeiten.")
    # 2: Datei in Ordnung, aber die verlangte PDF-Prüfung fehlt (nicht mit «bestanden» verwechseln)
    return 1 if fehler else 2 if pdf_nicht_geprueft else 0


if __name__ == "__main__":
    sys.exit(main())
