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


def geruest(skill):
    text = open(os.path.join(SKILLS, skill, "SKILL.md"), encoding="utf-8").read()
    teil = text.split("## Gerüst", 1)[1]
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
        p = self.run_py(os.path.join(".ask", "skills", "pruefen.py"), os.path.join("ausgabe", ausgabe))
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


if __name__ == "__main__":
    unittest.main()
