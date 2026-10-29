"""Führt die Code-Gerüste aus resources/skills/*/SKILL.md aus und prüft die Ergebnisse mit pruefen.py.

Läuft mit dem mitgelieferten Python:  <python>\\python.exe -m unittest tests\\test_skills.py -v
"""
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SKILLS = os.path.join(ROOT, "resources", "skills")


def geruest(skill, abschnitt="## Gerüst"):
    with open(os.path.join(SKILLS, skill, "SKILL.md"), encoding="utf-8") as f:
        text = f.read()
    teil = text.split(abschnitt, 1)[1]
    return re.search(r"```python\n(.*?)```", teil, re.S).group(1)


class SkillTest(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.mkdtemp(prefix="ask-skills-")
        shutil.copytree(SKILLS, os.path.join(self.dir, ".ask", "skills"))
        os.makedirs(os.path.join(self.dir, "ausgabe"))
        with open(os.path.join(self.dir, "daten.csv"), "w", encoding="cp1252", newline="") as f:
            f.write("Datum;Kategorie;Betrag\n01.09.2026;Bücher;12.50\n02.09.2026;Verpflegung;8.20\n"
                    "03.09.2026;Bücher;30.00\n05.09.2026;Material;4.75\n")

    def tearDown(self):
        shutil.rmtree(self.dir, ignore_errors=True)

    def run_py(self, *args):
        env = {**os.environ, "PYTHONUTF8": "1", "MPLBACKEND": "Agg"}
        return subprocess.run([sys.executable, *args], cwd=self.dir, capture_output=True, text=True,
                              encoding="utf-8", env=env, timeout=120)

    def baue_und_pruefe(self, skill, ausgabe):
        with open(os.path.join(self.dir, f"{skill}.py"), "w", encoding="utf-8") as f:
            f.write(geruest(skill))
        r = self.run_py(f"{skill}.py")
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("gespeichert", r.stdout)
        p = self.run_py(os.path.join(".ask", "skills", "pruefen.py"), ausgabe)
        print(f"\n--- pruefen.py {ausgabe} ---\n{p.stdout}{p.stderr}")
        self.assertEqual(p.returncode, 0, p.stdout + p.stderr)
        self.assertNotIn("WARNUNG", p.stdout)
        return p.stdout

    def test_word(self):
        out = self.baue_und_pruefe("word", "Anleitung.docx")
        self.assertIn("Einleitung", out)
        self.assertIn("Tabelle 1: 3 × 2", out)

    def test_excel(self):
        out = self.baue_und_pruefe("excel", "Auswertung.xlsx")
        self.assertIn("Blätter: Daten, Auswertung", out)
        self.assertIn("Diagramme: 1", out)

    def test_powerpoint(self):
        out = self.baue_und_pruefe("powerpoint", "Praesentation.pptx")
        self.assertIn("Folien: 2", out)
        self.assertIn("16:9", out)

    def test_pdf(self):
        out = self.baue_und_pruefe("pdf", "Merkblatt.pdf")
        self.assertIn("Seiten: 1", out)
        self.assertIn("Passwörter", out)

    def bearbeite(self, skill):
        """Führt das Beispiel aus «Vorhandene Datei bearbeiten» unverändert aus, wie das Modell es täte."""
        os.makedirs(os.path.join(self.dir, ".ask", "skripte"), exist_ok=True)
        skript = os.path.join(".ask", "skripte", f"bearbeite_{skill}.py")
        with open(os.path.join(self.dir, skript), "w", encoding="utf-8") as f:
            f.write(geruest(skill, "## Vorhandene Datei bearbeiten"))
        r = self.run_py(skript)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)
        self.assertIn("gespeichert", r.stdout)
        return r.stdout

    def test_word_bearbeiten(self):
        from docx import Document
        doc = Document()
        doc.sections[0].header.paragraphs[0].text = "Proxmox 7 Handbuch"
        doc.add_paragraph("Bericht", style="Title")
        doc.add_heading("Installation", level=1)
        p = doc.add_paragraph("Wir nutzen ")
        p.add_run("Prox").bold = True
        p.add_run("mox 7")
        p.add_run(" im Labor.")
        doc.add_paragraph("Dieser Abschnitt ist veraltet.")
        doc.add_table(rows=1, cols=1, style="Table Grid").cell(0, 0).text = "Version: Proxmox 7"
        from docx.oxml import parse_xml
        doc.element.body.insert(0, parse_xml(  # Deckblatt wie in der TF-Bern-Vorlage: Inhaltssteuerelement
            '<w:sdt xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:sdtContent>'
            '<w:p><w:r><w:t>Deckblatt Proxmox 7</w:t></w:r></w:p></w:sdtContent></w:sdt>'))
        quelle = os.path.join(self.dir, "bericht.docx")
        doc.save(quelle)

        out = self.bearbeite("word")
        self.assertIn("4 Stellen ersetzt", out)
        self.assertIn("Deckblatt Proxmox 8", Document(quelle).element.body.xpath("string(.//w:sdt)"))
        neu = Document(quelle)  # direkt geändert
        texte = [p.text for p in neu.paragraphs]
        i = texte.index("Installation")
        self.assertEqual(texte[i + 1:i + 4], ["Vor der Installation ein Backup erstellen.", "Backup auf externe Festplatte",
                                              "Wir nutzen Proxmox 8 im Labor."])
        self.assertEqual(neu.paragraphs[i + 2].style.name, "List Bullet")
        self.assertNotIn("Dieser Abschnitt ist veraltet.", texte)
        fett = [r.text for r in neu.paragraphs[i + 3].runs if r.bold]
        self.assertEqual(fett, ["Proxmox 8"], "Formatierung im Absatz bleibt erhalten")
        self.assertEqual(neu.tables[0].cell(0, 0).text, "Version: Proxmox 8")
        self.assertEqual(neu.sections[0].header.paragraphs[0].text, "Proxmox 8 Handbuch")
        # pruefen.py zählt Kopfzeilen-Bilder und Abschnitte aus dem XML
        p = self.run_py(os.path.join(".ask", "skills", "pruefen.py"), "bericht.docx")
        self.assertIn("in Kopf- und Fusszeilen", p.stdout)
        p = self.run_py(os.path.join(".ask", "skills", "pruefen.py"), "bericht.docx")
        self.assertEqual(p.returncode, 0, p.stdout)

    def test_excel_bearbeiten(self):
        import zipfile
        from openpyxl import Workbook, load_workbook
        from openpyxl.chart import BarChart, Reference
        wb = Workbook()
        ws = wb.active
        ws.title = "Budget"
        for zeile in [("Posten", "Betrag"), ("Miete", 300), ("Essen", 200), ("Material", 50)]:
            ws.append(zeile)
        ws["A5"], ws["B5"] = "Total", "=SUM(B2:B4)"
        for z in range(2, 6):
            ws.cell(z, 2).number_format = "#,##0.00"
        diagramm = BarChart()
        diagramm.add_data(Reference(ws, min_col=2, min_row=1, max_row=4), titles_from_data=True)
        ws.add_chart(diagramm, "D2")
        wb.save(os.path.join(self.dir, "budget.xlsx"))

        out = self.bearbeite("excel")
        self.assertIn("Risiken: ['Diagramme bleiben", out)
        ziel = os.path.join(self.dir, "budget.xlsx")  # direkt geändert
        ws = load_workbook(ziel)["Budget"]
        self.assertEqual(ws["B3"].value, 450)
        self.assertEqual(ws["B5"].value, "=SUM(B2:B4)", "Formel bleibt")
        self.assertEqual((ws["A6"].value, ws["B6"].value, ws["B6"].number_format), ("Reserve", 100, "#,##0.00"))
        with zipfile.ZipFile(ziel) as z:
            self.assertEqual(sum(n.startswith("xl/charts/chart") for n in z.namelist()), 1, "Diagramm bleibt")

        sys.path.insert(0, SKILLS)
        from bearbeiten import excel_risiken
        heikel = os.path.join(self.dir, "heikel.xlsx")
        with zipfile.ZipFile(heikel, "w") as z:
            z.writestr("xl/vbaProject.bin", b"")
            z.writestr("xl/drawings/drawing1.xml", '<xdr:wsDr><xdr:twoCellAnchor><xdr:sp macro=""/></xdr:twoCellAnchor></xdr:wsDr>')
            z.writestr("xl/worksheets/sheet1.xml", "<worksheet><x14:sparklineGroups/></worksheet>")
        self.assertEqual(excel_risiken(heikel), ["Makros gehen verloren", "Formen und Textfelder gehen verloren", "Sparklines gehen verloren"])

    def test_powerpoint_bearbeiten(self):
        from pptx import Presentation
        prs = Presentation()
        titel = prs.slides.add_slide(prs.slide_layouts[0])
        titel.shapes.title.text = "Proxmox 7 Kurs"
        inhalt = prs.slides.add_slide(prs.slide_layouts[1])
        inhalt.shapes.title.text = "Inhalt"
        absatz = inhalt.placeholders[1].text_frame.paragraphs[0]
        absatz.add_run().text = "Wir lernen "
        fett = absatz.add_run()
        fett.text = "Proxmox 7"
        fett.font.bold = True
        inhalt.notes_slide.notes_text_frame.text = "Proxmox 7 live zeigen"
        prs.save(os.path.join(self.dir, "vortrag.pptx"))

        out = self.bearbeite("powerpoint")
        self.assertIn("3 Stellen ersetzt", out)
        neu = Presentation(os.path.join(self.dir, "vortrag.pptx"))
        self.assertEqual(len(neu.slides), 3)
        self.assertEqual(neu.slides[0].shapes.title.text, "Proxmox 8 Kurs")
        runs = neu.slides[1].placeholders[1].text_frame.paragraphs[0].runs
        self.assertEqual([(r.text, bool(r.font.bold)) for r in runs], [("Wir lernen ", False), ("Proxmox 8", True)])
        self.assertEqual(neu.slides[1].notes_slide.notes_text_frame.text, "Proxmox 8 live zeigen")
        self.assertEqual(neu.slides[2].shapes.title.text, "Fazit")

    def test_pdf_bearbeiten(self):
        from pypdf import PdfReader
        from reportlab.pdfgen import canvas
        c = canvas.Canvas(os.path.join(self.dir, "unterlagen.pdf"))
        for n in range(1, 4):
            c.drawString(100, 700, f"Seite {n}")
            c.showPage()
        c.save()

        self.bearbeite("pdf")
        neu = PdfReader(os.path.join(self.dir, "unterlagen.pdf"))
        self.assertEqual(len(neu.pages), 2)
        self.assertIn("Seite 3", neu.pages[1].extract_text())
        self.assertEqual(neu.pages[1].rotation, 90)

    def test_pruefen_findet_fehler(self):
        from docx import Document
        doc = Document()
        doc.add_paragraph("Lorem ipsum dolor")
        for _ in range(4):
            doc.add_paragraph("")
        doc.save(os.path.join(self.dir, "ausgabe", "schlecht.docx"))
        p = self.run_py(os.path.join(".ask", "skills", "pruefen.py"), "ausgabe/schlecht.docx")
        self.assertIn("Keine Überschriften", p.stdout)
        self.assertIn("leere Absätze", p.stdout)
        self.assertIn("Platzhaltertext", p.stdout)
        from openpyxl import Workbook
        wb = Workbook()
        wb.active["A1"] = "Wert"
        wb.active["A2"] = "=SUMME(A1:A1)"
        wb.save(os.path.join(self.dir, "ausgabe", "schlecht.xlsx"))
        p = self.run_py(os.path.join(".ask", "skills", "pruefen.py"), "ausgabe/schlecht.xlsx")
        self.assertEqual(p.returncode, 1, p.stdout)
        self.assertIn("Deutscher Funktionsname", p.stdout)
        # Kopfzeile ist die erste nicht leere Zeile, nicht A1 (Vorlage mit Titel- und Leerzeilen)
        from openpyxl.styles import Font
        for fett, erwartet in ((True, False), (False, True)):
            wb = Workbook()
            wb.active["B3"] = "Kompetenzband"
            wb.active["B3"].font = Font(bold=fett)
            wb.active["B4"] = "Netzwerk"
            wb.save(os.path.join(self.dir, "ausgabe", "matrix.xlsx"))
            p = self.run_py(os.path.join(".ask", "skills", "pruefen.py"), "ausgabe/matrix.xlsx")
            self.assertEqual("Kopfzeile (Zeile 3) ist nicht fett" in p.stdout, erwartet, p.stdout)


if __name__ == "__main__":
    unittest.main()
