"""Text aus Dokumenten für read_file, wie open_terminal.utils.documents, aber mit vollständigem Word-Text.

open-terminal liest bei .docx nur Absätze und Tabellen direkt im Dokumentkörper. Inhalt in
Inhaltssteuerelementen (<w:sdt>, z. B. Deckblatt und Inhaltsverzeichnis der TF-Bern-Vorlage),
verschachtelte Tabellen sowie Kopf- und Fusszeilen fehlen dann. server.py startet für read_file
dieses Skript statt des Originals. Alle anderen Formate gehen unverändert an open-terminal.
"""
import json
import mimetypes
import sys

from open_terminal.utils import documents as base


def _ohne_alte_fassungen(body):
    """Nachverfolgte Änderungen wie bei open-terminal: aktueller Text, gelöschtes weg."""
    from docx.oxml.ns import qn
    from lxml import etree

    for node in list(body.iter()):
        if isinstance(node.tag, str) and node.tag.startswith(qn("w:")) and node.tag.endswith("Change"):
            node.getparent().remove(node)
    for node in body.xpath(".//w:tr[w:trPr/w:del] | .//w:tc[w:tcPr/w:cellDel]"):
        node.getparent().remove(node)
    etree.strip_elements(body, qn("w:del"), qn("w:moveFrom"), with_tail=False)
    etree.strip_tags(body, qn("w:ins"), qn("w:moveTo"))


def _bloecke(element, parent):
    """Text aller Absätze und Tabellen in Dokumentreihenfolge, auch in Inhaltssteuerelementen."""
    from docx.table import Table
    from docx.text.paragraph import Paragraph

    for child in element:
        tag = child.tag.rsplit("}", 1)[-1] if isinstance(child.tag, str) else ""
        if tag == "p":
            yield Paragraph(child, parent).text
        elif tag == "tbl":
            for row in Table(child, parent).rows:
                zellen = []
                for cell in row.cells:
                    zellen.append(" ".join(t for t in _bloecke(cell._tc, cell) if t.strip()))
                yield "\t".join(zellen)
        elif tag in ("sdt", "sdtContent", "customXml", "smartTag"):
            yield from _bloecke(child, parent)


@base.text_extractor()
def extract_docx(file_path: str):
    from docx import Document

    doc = Document(file_path)
    _ohne_alte_fassungen(doc.element.body)
    yield from _bloecke(doc.element.body, doc)
    gesehen = set()
    for nummer, section in enumerate(doc.sections, start=1):
        for name, teil in (("Kopfzeile", section.header), ("Fusszeile", section.footer),
                           ("Kopfzeile erste Seite", section.first_page_header),
                           ("Fusszeile erste Seite", section.first_page_footer)):
            if teil is None or teil.is_linked_to_previous or id(teil._element) in gesehen:
                continue
            gesehen.add(id(teil._element))
            text = [t for t in _bloecke(teil._element, teil) if t.strip()]
            if text:
                yield f"--- {name} (Abschnitt {nummer}) ---"
                yield from text


def main() -> None:
    try:
        payload = json.load(sys.stdin)
        path = payload["path"]
        mime = mimetypes.guess_type(path)[0]
        extractor = next(fn for kind, suffix, fn in base.EXTRACTORS
                         if kind and kind == mime or suffix and path.lower().endswith(suffix))
        if extractor is base.extract_docx:
            extractor = extract_docx
        with open(payload["output"], "w", encoding="utf-8", newline="") as output:
            if hasattr(extractor, "write_to"):
                extractor.write_to(path, output)
            else:
                output.write(extractor(path))
        result = {}
    except Exception as exc:
        result = {"error": str(exc), "error_type": type(exc).__name__}
    json.dump(result, sys.stdout)


if __name__ == "__main__":
    main()
