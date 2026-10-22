---
name: excel
description: Excel-Dateien (.xlsx) und CSV-Auswertungen mit pandas und openpyxl erstellen und prüfen. Lesen, bevor eine .xlsx-Datei entsteht.
---
# Excel-Datei erstellen

Werkzeuge: pandas zum Einlesen und Rechnen, openpyxl zum Formatieren. Immer zuerst ein Skript schreiben (z. B. `auswertung.py`), dann `python auswertung.py` ausführen.

## Regeln
- CSV robust einlesen (Schweizer Dateien haben oft `;` und Windows-Kodierung), siehe Gerüst.
- Zuerst die Daten ansehen: `print(df.head())`, `print(df.dtypes)`. Erst dann auswerten.
- Rohdaten und Auswertung auf getrennte Blätter: «Daten» und «Auswertung».
- Kopfzeile fett mit Hintergrund, Kopfzeile fixieren, Autofilter, sinnvolle Spaltenbreiten, Zahlenformate.
- Summen und Mittelwerte als Excel-Formel (`=SUMME` heisst in openpyxl `=SUM`), damit sie nachvollziehbar sind. Die Werte zusätzlich in Python berechnen und zur Kontrolle ausgeben, denn Excel rechnet Formeln erst beim Öffnen.
- Speichern unter `ausgabe/`, nie eine bestehende Datei überschreiben.

## Gerüst
```python
import pandas as pd
from openpyxl import load_workbook
from openpyxl.styles import Font, PatternFill, Alignment
from openpyxl.chart import BarChart, Reference
from openpyxl.utils import get_column_letter

def lies_csv(pfad):
    for enc in ("utf-8-sig", "cp1252"):
        try:
            return pd.read_csv(pfad, sep=None, engine="python", encoding=enc)
        except UnicodeDecodeError:
            continue
    raise ValueError("Kodierung nicht erkannt")

df = lies_csv("daten.csv")
print(df.head()); print(df.dtypes)

auswertung = df.groupby("Kategorie", as_index=False)["Betrag"].sum()
print(auswertung)                                   # Kontrollwerte

ziel = "ausgabe/Auswertung.xlsx"
with pd.ExcelWriter(ziel, engine="openpyxl") as w:
    df.to_excel(w, sheet_name="Daten", index=False)
    auswertung.to_excel(w, sheet_name="Auswertung", index=False)

wb = load_workbook(ziel)
for ws in wb.worksheets:
    for zelle in ws[1]:
        zelle.font = Font(bold=True, color="FFFFFF")
        zelle.fill = PatternFill("solid", fgColor="0B6E75")
        zelle.alignment = Alignment(horizontal="center")
    ws.freeze_panes = "A2"
    ws.auto_filter.ref = ws.dimensions
    for spalte in ws.columns:
        breite = max(len(str(z.value or "")) for z in spalte) + 2
        ws.column_dimensions[get_column_letter(spalte[0].column)].width = min(max(breite, 10), 50)

ws = wb["Auswertung"]
letzte = ws.max_row
ws.cell(row=letzte + 1, column=1, value="Total").font = Font(bold=True)
ws.cell(row=letzte + 1, column=2, value=f"=SUM(B2:B{letzte})").font = Font(bold=True)
for zeile in ws.iter_rows(min_row=2, min_col=2, max_col=2):
    for zelle in zeile:
        zelle.number_format = "#,##0.00"

diagramm = BarChart()
diagramm.title = "Betrag nach Kategorie"
diagramm.add_data(Reference(ws, min_col=2, min_row=1, max_row=letzte), titles_from_data=True)
diagramm.set_categories(Reference(ws, min_col=1, min_row=2, max_row=letzte))
ws.add_chart(diagramm, "D2")

wb.save(ziel)
print("gespeichert")
```

## Zahlenformate
`"#,##0.00"` Betrag, `"0.0%"` Prozent (Wert 0.25 = 25 %), `"DD.MM.YYYY"` Datum, `"0"` ganze Zahl.

## Typische Fehler
- Spaltennamen raten: zuerst `print(df.columns.tolist())`.
- Zahlen als Text (z. B. «1'250.50»): mit `pd.to_numeric(df[s].astype(str).str.replace("'", ""), errors="coerce")` umwandeln.
- Datumsangaben: `pd.to_datetime(df[s], dayfirst=True, errors="coerce")`.

## Prüfen
`python .ask/skills/pruefen.py ausgabe/Auswertung.xlsx`
Zeigt Blätter, Grösse, Formeln, Fehlerwerte und die ersten Zeilen. Mit `--pdf` zusätzlich Umwandlung durch Excel (falls installiert).
