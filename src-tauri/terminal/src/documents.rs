//! Text aus Dokumenten für `GET /files/read` (Port von open_terminal/utils/documents.py und
//! python/ask_terminal/documents.py).
//!
//! open-terminal wandelt beim Lesen Dokumente in Text um, wenn die Datei kein UTF-8 ist. Die Formate und die
//! Form des Textes sind dieselben (Trennzeilen `--- Blatt ---`, Tabulator zwischen Zellen); Word wird wie
//! in ASK vollständig gelesen: Inhaltssteuerelemente (`<w:sdt>`), verschachtelte Tabellen, Kopf- und
//! Fusszeilen. Alles ist reines Rust (zip, XML), ohne C-Code.
//!
//! Öffentliche Schnittstelle:
//! - `supports(pfad)`: gehört die Dateiendung zu einem Format, das hier umgewandelt wird?
//! - `extract(pfad) -> Result<String, DocError>`
//! - Formate: .pdf .docx .xlsx .pptx .rtf .xls .odt .ods .odp .epub .eml

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::path::Path;

use roxmltree::{Document, Node};

/// Fehler beim Umwandeln. `permission`: Zugriff verweigert (wird zu HTTP 403, sonst 422).
#[derive(Debug, Clone)]
pub struct DocError {
    pub message: String,
    pub permission: bool,
}

impl DocError {
    fn new(message: impl Into<String>) -> Self {
        DocError { message: message.into(), permission: false }
    }
}

impl std::fmt::Display for DocError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<std::io::Error> for DocError {
    fn from(e: std::io::Error) -> Self {
        DocError { message: e.to_string(), permission: e.kind() == std::io::ErrorKind::PermissionDenied }
    }
}

impl From<zip::result::ZipError> for DocError {
    fn from(e: zip::result::ZipError) -> Self {
        DocError::new(format!("Ungültiges Dokument: {e}"))
    }
}

impl From<roxmltree::Error> for DocError {
    fn from(e: roxmltree::Error) -> Self {
        DocError::new(format!("Ungültiges XML: {e}"))
    }
}

type Res<T> = Result<T, DocError>;

const EXTENSIONS: [&str; 11] = ["pdf", "docx", "xlsx", "pptx", "rtf", "xls", "odt", "ods", "odp", "epub", "eml"];

/// Gehört die Dateiendung zu einem Format, das in Text umgewandelt wird?
pub fn supports(path: &str) -> bool {
    extension(path).is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
}

fn extension(path: &str) -> Option<String> {
    Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase())
}

/// Der Text eines Dokuments, nach Endung
pub fn extract(path: &str) -> Res<String> {
    let ext = extension(path).unwrap_or_default();
    match ext.as_str() {
        "pdf" => extract_pdf_safe(path),
        "docx" => extract_docx(&std::fs::read(path)?),
        "xlsx" => extract_xlsx(&std::fs::read(path)?),
        "pptx" => extract_pptx(&std::fs::read(path)?),
        "rtf" => Ok(extract_rtf(&std::fs::read(path)?)),
        "xls" => extract_xls(&std::fs::read(path)?),
        "odt" => extract_odt(&std::fs::read(path)?),
        "ods" => extract_ods(&std::fs::read(path)?),
        "odp" => extract_odp(&std::fs::read(path)?),
        "epub" => extract_epub(&std::fs::read(path)?),
        "eml" => extract_eml(&std::fs::read(path)?),
        _ => Err(DocError::new(format!("Kein Textformat: {path}"))),
    }
}

// ---------- Hilfen für XML und Zip ----------

fn open_zip(data: &[u8]) -> Res<zip::ZipArchive<Cursor<&[u8]>>> {
    Ok(zip::ZipArchive::new(Cursor::new(data))?)
}

/// Eine Datei aus dem Zip als Text; `None`, wenn es sie nicht gibt
fn zip_text(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Res<Option<String>> {
    let mut file = match zip.by_name(name) {
        Ok(f) => f,
        Err(zip::result::ZipError::FileNotFound) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    // Grenze gegen Zip-Bomben: ein XML-Teil ist nie grösser als 512 MiB
    let mut text = String::new();
    file.by_ref().take(512 * 1024 * 1024).read_to_string(&mut text).map_err(|e| DocError::new(format!("{name}: {e}")))?;
    Ok(Some(text.trim_start_matches('\u{feff}').to_string()))
}

fn zip_required(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Res<String> {
    zip_text(zip, name)?.ok_or_else(|| DocError::new(format!("In der Datei fehlt {name}")))
}

fn local<'a>(node: Node<'a, '_>) -> &'a str {
    node.tag_name().name()
}

fn is(node: Node, ns: &str, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name && node.tag_name().namespace() == Some(ns)
}

fn children_named<'a, 'b: 'a>(node: Node<'a, 'b>, ns: &'a str, name: &'a str) -> impl Iterator<Item = Node<'a, 'b>> {
    node.children().filter(move |c| is(*c, ns, name))
}

fn attr_ns<'a>(node: Node<'a, '_>, ns: &str, name: &str) -> Option<&'a str> {
    node.attribute((ns, name))
}

/// Wie lxml `itertext`: alle Textknoten unter dem Knoten, in Dokumentreihenfolge
fn itertext(node: Node) -> String {
    node.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).collect()
}

/// Beziehungen einer Teildatei: Id → Ziel (voller Pfad im Zip)
fn relationships(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, part: &str) -> Res<HashMap<String, String>> {
    let (dir, file) = part.rsplit_once('/').unwrap_or(("", part));
    let rels_name = if dir.is_empty() { format!("_rels/{file}.rels") } else { format!("{dir}/_rels/{file}.rels") };
    let mut map = HashMap::new();
    let Some(text) = zip_text(zip, &rels_name)? else { return Ok(map) };
    let doc = Document::parse(&text)?;
    for rel in doc.descendants().filter(|n| n.is_element() && local(*n) == "Relationship") {
        let (Some(id), Some(target)) = (rel.attribute("Id"), rel.attribute("Target")) else { continue };
        map.insert(id.to_string(), resolve_target(dir, target));
    }
    Ok(map)
}

fn resolve_target(dir: &str, target: &str) -> String {
    let joined = if let Some(abs) = target.strip_prefix('/') { abs.to_string() } else if dir.is_empty() { target.to_string() } else { format!("{dir}/{target}") };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            other => parts.push(other),
        }
    }
    parts.join("/")
}

// ---------- Word ----------

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";
const R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

fn extract_docx(data: &[u8]) -> Res<String> {
    let mut zip = open_zip(data)?;
    let main = zip_required(&mut zip, "word/document.xml")?;
    let doc = Document::parse(&main)?;
    let body = doc.root_element().children().find(|n| is(*n, W, "body")).ok_or_else(|| DocError::new("Kein Dokumentkörper in word/document.xml"))?;
    let mut lines = blocks(body);
    // Kopf- und Fusszeilen je Abschnitt (wie python-docx: Abschnitte in Dokumentreihenfolge)
    let rels = relationships(&mut zip, "word/document.xml")?;
    let sections: Vec<Node> = body
        .children()
        .filter(|n| is(*n, W, "p"))
        .filter_map(|p| p.children().find(|c| is(*c, W, "pPr")))
        .filter_map(|ppr| ppr.children().find(|c| is(*c, W, "sectPr")))
        .chain(body.children().filter(|n| is(*n, W, "sectPr")))
        .collect();
    let mut seen: Vec<String> = Vec::new();
    for (index, sect) in sections.iter().enumerate() {
        let number = index + 1;
        for (name, tag, kind) in [
            ("Kopfzeile", "headerReference", "default"),
            ("Fusszeile", "footerReference", "default"),
            ("Kopfzeile erste Seite", "headerReference", "first"),
            ("Fusszeile erste Seite", "footerReference", "first"),
        ] {
            // ohne Verweis: «mit vorherigem verknüpft», wird übersprungen
            let Some(reference) = children_named(*sect, W, tag).find(|r| attr_ns(*r, W, "type").unwrap_or("default") == kind) else { continue };
            let Some(target) = attr_ns(reference, R, "id").and_then(|id| rels.get(id)) else { continue };
            if seen.contains(target) {
                continue;
            }
            seen.push(target.clone());
            let Some(text) = zip_text(&mut zip, target)? else { continue };
            let part = Document::parse(&text)?;
            let part_lines: Vec<String> = blocks(part.root_element()).into_iter().filter(|t| !t.trim().is_empty()).collect();
            if !part_lines.is_empty() {
                lines.push(format!("--- {name} (Abschnitt {number}) ---"));
                lines.extend(part_lines);
            }
        }
    }
    Ok(lines.join("\n"))
}

/// Text aller Absätze und Tabellen in Dokumentreihenfolge, auch in Inhaltssteuerelementen
fn blocks(element: Node) -> Vec<String> {
    let mut out = Vec::new();
    for child in element.children().filter(|c| c.is_element()) {
        if child.tag_name().namespace() != Some(W) {
            continue;
        }
        match local(child) {
            "p" => out.push(paragraph_text(child)),
            "tbl" => out.extend(table_rows(child)),
            "sdt" | "sdtContent" | "customXml" | "smartTag" => out.extend(blocks(child)),
            _ => {}
        }
    }
    out
}

/// Text eines Absatzes wie python-docx (Läufe und Links), dazu Läufe in Inhaltssteuerelementen und Feldern.
/// Gelöschter Text (`w:del`, `w:moveFrom`) fehlt, eingefügter (`w:ins`, `w:moveTo`) zählt.
fn paragraph_text(p: Node) -> String {
    let mut out = String::new();
    paragraph_inline(p, &mut out);
    out
}

fn paragraph_inline(node: Node, out: &mut String) {
    for child in node.children().filter(|c| c.is_element()) {
        if child.tag_name().namespace() != Some(W) {
            continue;
        }
        match local(child) {
            "r" => run_text(child, out),
            "hyperlink" | "ins" | "moveTo" | "sdt" | "sdtContent" | "smartTag" | "customXml" | "fldSimple" => paragraph_inline(child, out),
            _ => {}
        }
    }
}

fn run_text(run: Node, out: &mut String) {
    for c in run.children().filter(|c| c.is_element()) {
        if c.tag_name().namespace() != Some(W) {
            continue;
        }
        match local(c) {
            "t" => out.push_str(&itertext(c)),
            "tab" | "ptab" => out.push('\t'),
            "br" => {
                if matches!(attr_ns(c, W, "type"), None | Some("textWrapping")) {
                    out.push('\n');
                }
            }
            "cr" => out.push('\n'),
            "noBreakHyphen" => out.push('-'),
            _ => {}
        }
    }
}

/// Zeilen einer Tabelle: Zellen mit Tabulator getrennt. Verbundene Zellen wiederholen den Text wie bei
/// python-docx (`gridSpan` wiederholt die Zelle, `vMerge` übernimmt die Zelle darüber).
fn table_rows(tbl: Node) -> Vec<String> {
    let col_count = children_named(tbl, W, "tblGrid").next().map(|g| children_named(g, W, "gridCol").count()).unwrap_or(0);
    let mut cells: Vec<Option<usize>> = Vec::new(); // Index in `texts`; None: Zelle fehlt
    let mut texts: Vec<String> = Vec::new();
    let mut row_count = 0;
    for tr in children_named(tbl, W, "tr") {
        // gelöschte Zeilen (nachverfolgte Änderung) fehlen
        if children_named(tr, W, "trPr").any(|pr| children_named(pr, W, "del").next().is_some()) {
            continue;
        }
        row_count += 1;
        for tc in children_named(tr, W, "tc") {
            let tc_pr = children_named(tc, W, "tcPr").next();
            if tc_pr.is_some_and(|pr| children_named(pr, W, "cellDel").next().is_some()) {
                continue;
            }
            let span = tc_pr
                .and_then(|pr| children_named(pr, W, "gridSpan").next())
                .and_then(|g| attr_ns(g, W, "val"))
                .and_then(|v| v.parse::<usize>().ok())
                .filter(|v| *v >= 1)
                .unwrap_or(1);
            let v_continue = tc_pr
                .and_then(|pr| children_named(pr, W, "vMerge").next())
                .is_some_and(|m| matches!(attr_ns(m, W, "val"), None | Some("continue")));
            let text = blocks(tc).into_iter().filter(|t| !t.trim().is_empty()).collect::<Vec<_>>().join(" ");
            for i in 0..span {
                if v_continue {
                    let above = cells.len().checked_sub(col_count).and_then(|k| cells.get(k).copied().flatten());
                    cells.push(above);
                } else if i > 0 {
                    let last = cells.last().copied().flatten();
                    cells.push(last);
                } else {
                    texts.push(text.clone());
                    cells.push(Some(texts.len() - 1));
                }
            }
        }
    }
    let width = if col_count == 0 { 0 } else { col_count };
    (0..row_count)
        .map(|r| {
            let from = (r * width).min(cells.len());
            let to = ((r + 1) * width).min(cells.len());
            cells[from..to].iter().map(|c| c.map(|i| texts[i].as_str()).unwrap_or("")).collect::<Vec<_>>().join("\t")
        })
        .collect()
}

// ---------- Excel (.xlsx) ----------

const MAIN: &str = "http://schemas.openxmlformats.org/spreadsheetml/2006/main";

fn extract_xlsx(data: &[u8]) -> Res<String> {
    let mut zip = open_zip(data)?;
    let workbook_xml = zip_required(&mut zip, "xl/workbook.xml")?;
    let workbook = Document::parse(&workbook_xml)?;
    let rels = relationships(&mut zip, "xl/workbook.xml")?;
    let shared = match zip_text(&mut zip, "xl/sharedStrings.xml")? {
        Some(text) => {
            let doc = Document::parse(&text)?;
            doc.root_element().children().filter(|n| is(*n, MAIN, "si")).map(|si| rich_text(si)).collect::<Vec<_>>()
        }
        None => Vec::new(),
    };
    let date_styles = match zip_text(&mut zip, "xl/styles.xml")? {
        Some(text) => date_style_flags(&Document::parse(&text)?),
        None => Vec::new(),
    };
    let epoch_1904 = workbook
        .descendants()
        .find(|n| is(*n, MAIN, "workbookPr"))
        .and_then(|n| n.attribute("date1904"))
        .is_some_and(|v| v == "1" || v == "true");
    let mut lines = Vec::new();
    let sheets = workbook.descendants().find(|n| is(*n, MAIN, "sheets"));
    for sheet in sheets.into_iter().flat_map(|s| s.children().filter(|c| is(*c, MAIN, "sheet"))) {
        let name = sheet.attribute("name").unwrap_or("Sheet");
        let Some(target) = attr_ns(sheet, R, "id").and_then(|id| rels.get(id)) else { continue };
        let Some(sheet_xml) = zip_text(&mut zip, target)? else { continue };
        let doc = Document::parse(&sheet_xml)?;
        lines.push(format!("--- {name} ---"));
        lines.extend(sheet_rows(&doc, &shared, &date_styles, epoch_1904));
    }
    Ok(lines.join("\n"))
}

/// Text eines `<si>`/`<is>`: alle `<t>` ausser der Lautschrift (`<rPh>`)
fn rich_text(node: Node) -> String {
    let mut out = String::new();
    for d in node.descendants().filter(|d| is(*d, MAIN, "t")) {
        if d.ancestors().any(|a| is(a, MAIN, "rPh")) {
            continue;
        }
        out.push_str(&itertext(d));
    }
    out
}

/// Pro Zellformat (`cellXfs`): ist es ein Datumsformat?
fn date_style_flags(styles: &Document) -> Vec<bool> {
    let mut custom: HashMap<u32, String> = HashMap::new();
    for n in styles.descendants().filter(|n| is(*n, MAIN, "numFmt")) {
        if let (Some(id), Some(code)) = (n.attribute("numFmtId").and_then(|v| v.parse().ok()), n.attribute("formatCode")) {
            custom.insert(id, code.to_string());
        }
    }
    let Some(xfs) = styles.descendants().find(|n| is(*n, MAIN, "cellXfs")) else { return Vec::new() };
    xfs.children()
        .filter(|n| is(*n, MAIN, "xf"))
        .map(|xf| {
            let id: u32 = xf.attribute("numFmtId").and_then(|v| v.parse().ok()).unwrap_or(0);
            match custom.get(&id) {
                Some(code) => is_date_format(code),
                None => matches!(id, 14..=22 | 45..=47),
            }
        })
        .collect()
}

/// Wie `openpyxl.styles.numbers.is_date_format`
fn is_date_format(format: &str) -> bool {
    let first = format.split(';').next().unwrap_or("");
    let stripped = regex::Regex::new(r#"\[[^\]]*\]|"[^"]*"|[_*\\].|\\."#).expect("Regex").replace_all(first, "");
    let chars: Vec<char> = stripped.chars().collect();
    chars.windows(2).any(|w| w[0] != '\\' && "dmhysDMHYS".contains(w[1]))
}

fn sheet_rows(doc: &Document, shared: &[String], date_styles: &[bool], epoch_1904: bool) -> Vec<String> {
    let Some(data) = doc.root_element().children().find(|n| is(*n, MAIN, "sheetData")) else { return Vec::new() };
    let mut cells: HashMap<(usize, usize), String> = HashMap::new();
    let (mut max_row, mut max_col) = (0usize, 0usize);
    let mut next_row = 0usize;
    for row in data.children().filter(|n| is(*n, MAIN, "row")) {
        let row_index = row.attribute("r").and_then(|v| v.parse::<usize>().ok()).unwrap_or(next_row + 1);
        next_row = row_index;
        let mut next_col = 0usize;
        for c in row.children().filter(|n| is(*n, MAIN, "c")) {
            let col = c.attribute("r").and_then(column_of).unwrap_or(next_col + 1);
            next_col = col;
            let value = cell_value(c, shared, date_styles, epoch_1904);
            max_row = max_row.max(row_index);
            max_col = max_col.max(col);
            if let Some(v) = value {
                cells.insert((row_index, col), v);
            }
        }
    }
    // Ausdehnung wie openpyxl aus <dimension>, mindestens aber der gefundene Inhalt
    if let Some(dim) = doc.root_element().children().find(|n| is(*n, MAIN, "dimension")).and_then(|d| d.attribute("ref")) {
        let last = dim.rsplit(':').next().unwrap_or(dim);
        if let Some(col) = column_of(last) {
            max_col = max_col.max(col);
        }
        if let Ok(row) = last.trim_start_matches(|c: char| c.is_ascii_alphabetic()).parse::<usize>() {
            max_row = max_row.max(row);
        }
    }
    (1..=max_row).map(|r| (1..=max_col).map(|c| cells.get(&(r, c)).map(String::as_str).unwrap_or("")).collect::<Vec<_>>().join("\t")).collect()
}

/// Spalte aus einem Zellbezug: «B3» → 2
fn column_of(reference: &str) -> Option<usize> {
    let letters: String = reference.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    if letters.is_empty() {
        return None;
    }
    Some(letters.to_ascii_uppercase().bytes().fold(0usize, |acc, b| acc * 26 + (b - b'A' + 1) as usize))
}

/// Wert einer Zelle als Text wie `str(zelle)` in openpyxl (mit `data_only`: gespeicherte Ergebnisse)
fn cell_value(c: Node, shared: &[String], date_styles: &[bool], epoch_1904: bool) -> Option<String> {
    let kind = c.attribute("t").unwrap_or("n");
    let v = c.children().find(|n| is(*n, MAIN, "v")).map(itertext);
    match kind {
        "s" => v.and_then(|i| i.trim().parse::<usize>().ok()).and_then(|i| shared.get(i).cloned()),
        "inlineStr" => c.children().find(|n| is(*n, MAIN, "is")).map(rich_text),
        "str" | "e" => v,
        "b" => v.map(|b| if b.trim() == "1" { "True".to_string() } else { "False".to_string() }),
        _ => {
            let raw = v?;
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let style: usize = c.attribute("s").and_then(|s| s.parse().ok()).unwrap_or(0);
            let number: f64 = raw.parse().ok()?;
            if date_styles.get(style).copied().unwrap_or(false) {
                return Some(excel_date_text(number, epoch_1904));
            }
            if raw.contains('.') || raw.contains('E') || raw.contains('e') {
                Some(py_float(number))
            } else {
                Some(raw.parse::<i64>().map(|i| i.to_string()).unwrap_or_else(|_| py_float(number)))
            }
        }
    }
}

/// Wie `repr(float)` in Python: «3.0», «0.1», «1e+22», «1.5e-07»
pub fn py_float(f: f64) -> String {
    if f.is_nan() {
        return "nan".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{:e}", f.abs());
    let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(|c| c.is_ascii_digit()).collect();
    let decpt = exp + 1;
    let sign = if f < 0.0 { "-" } else { "" };
    let body = if -4 < decpt && decpt <= 16 {
        if decpt <= 0 {
            format!("0.{}{}", "0".repeat((-decpt) as usize), digits)
        } else if decpt as usize >= digits.len() {
            format!("{}{}.0", digits, "0".repeat(decpt as usize - digits.len()))
        } else {
            format!("{}.{}", &digits[..decpt as usize], &digits[decpt as usize..])
        }
    } else {
        let rest = &digits[1..];
        let e = decpt - 1;
        format!("{}{}e{}{:02}", &digits[..1], if rest.is_empty() { String::new() } else { format!(".{rest}") }, if e < 0 { "-" } else { "+" }, e.abs())
    };
    format!("{sign}{body}")
}

/// Datum und Zeit wie `str(datetime)` in Python («2024-01-31 00:00:00», nur Zeit: «13:45:00»)
fn excel_date_text(value: f64, epoch_1904: bool) -> String {
    let mut value = value;
    if !epoch_1904 && value > 1.0 && value < 60.0 {
        value += 1.0; // Schaltjahrfehler von Excel für den 29.02.1900
    }
    let day = value.floor();
    let millis_total = ((value - day) * 86_400_000.0).round() as i64;
    let (day, millis_total) = if millis_total >= 86_400_000 { (day + 1.0, millis_total - 86_400_000) } else { (day, millis_total) };
    let (h, m, s, ms) = (millis_total / 3_600_000, millis_total / 60_000 % 60, millis_total / 1000 % 60, millis_total % 1000);
    let time = if ms == 0 { format!("{h:02}:{m:02}:{s:02}") } else { format!("{h:02}:{m:02}:{s:02}.{:06}", ms * 1000) };
    if (0.0..1.0).contains(&value) {
        return time;
    }
    // Tage seit 1899-12-30 (1904-01-01 bei Mac-Arbeitsmappen) in ein Datum umrechnen
    let epoch_days_from_civil = if epoch_1904 { days_from_civil(1904, 1, 1) } else { days_from_civil(1899, 12, 30) };
    let (y, mo, d) = civil_from_days(epoch_days_from_civil + day as i64);
    format!("{y:04}-{mo:02}-{d:02} {time}")
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// ---------- PowerPoint (.pptx) ----------

const A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";

fn extract_pptx(data: &[u8]) -> Res<String> {
    let mut zip = open_zip(data)?;
    let presentation_xml = zip_required(&mut zip, "ppt/presentation.xml")?;
    let presentation = Document::parse(&presentation_xml)?;
    let rels = relationships(&mut zip, "ppt/presentation.xml")?;
    let mut lines = Vec::new();
    let ids = presentation.descendants().filter(|n| is(*n, P, "sldId"));
    for (i, sld) in ids.enumerate() {
        lines.push(format!("--- Slide {} ---", i + 1));
        let Some(target) = attr_ns(sld, R, "id").and_then(|id| rels.get(id)) else { continue };
        let Some(slide_xml) = zip_text(&mut zip, target)? else { continue };
        let slide = Document::parse(&slide_xml)?;
        let Some(tree) = slide.descendants().find(|n| is(*n, P, "spTree")) else { continue };
        // nur Formen mit Textrahmen auf oberster Ebene (Gruppen, Bilder und Tabellen haben keinen)
        for sp in children_named(tree, P, "sp") {
            let paragraphs: Vec<String> = children_named(sp, P, "txBody").next().map(|tx| children_named(tx, A, "p").map(pptx_paragraph).collect()).unwrap_or_default();
            lines.push(paragraphs.join("\n"));
        }
    }
    Ok(lines.join("\n"))
}

/// Absatztext wie python-pptx: Zeilenumbruch als senkrechter Tabulator
fn pptx_paragraph(p: Node) -> String {
    let mut out = String::new();
    for c in p.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(A)) {
        match local(c) {
            "r" | "fld" => {
                for t in children_named(c, A, "t") {
                    out.push_str(&itertext(t));
                }
            }
            "br" => out.push('\u{b}'),
            _ => {}
        }
    }
    out
}

// ---------- RTF ----------

/// Port von `striprtf.striprtf.rtf_to_text` (Version 0.0.34)
fn extract_rtf(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let pattern = regex::Regex::new(r"(?i)\\([a-z]{1,32})(-?\d{1,10})?[ ]?|\\'([0-9a-f]{2})|\\([^a-z])|([{}])|[\r\n]+|(.)").expect("Regex");
    let mut stack: Vec<(i64, bool)> = Vec::new();
    let mut ignorable = false;
    let mut ucskip: i64 = 1;
    let mut curskip: i64 = 0;
    let mut out = String::new();
    for caps in pattern.captures_iter(&text) {
        let word = caps.get(1).map(|m| m.as_str().to_string());
        let arg = caps.get(2).map(|m| m.as_str());
        if let Some(brace) = caps.get(5).map(|m| m.as_str()) {
            curskip = 0;
            if brace == "{" {
                stack.push((ucskip, ignorable));
            } else if let Some((u, i)) = stack.pop() {
                ucskip = u;
                ignorable = i;
            }
        } else if let Some(ch) = caps.get(4).map(|m| m.as_str()) {
            curskip = 0;
            match ch {
                "~" => {
                    if !ignorable {
                        out.push('\u{a0}');
                    }
                }
                "{" | "}" | "\\" => {
                    if !ignorable {
                        out.push_str(ch);
                    }
                }
                "*" => ignorable = true,
                _ => {}
            }
        } else if let Some(word) = word {
            curskip = 0;
            let word = word.to_lowercase();
            if RTF_DESTINATIONS.contains(&word.as_str()) {
                ignorable = true;
            } else if ignorable {
            } else if let Some(special) = rtf_special(&word) {
                out.push_str(special);
            } else if word == "uc" {
                ucskip = arg.and_then(|a| a.parse().ok()).unwrap_or(1);
            } else if word == "u" {
                if let Some(mut c) = arg.and_then(|a| a.parse::<i64>().ok()) {
                    if c < 0 {
                        c += 0x10000;
                    }
                    out.push(char::from_u32(c as u32).unwrap_or('\u{fffd}'));
                    curskip = ucskip;
                }
            }
        } else if let Some(hex) = caps.get(3).map(|m| m.as_str()) {
            if curskip > 0 {
                curskip -= 1;
            } else if !ignorable {
                let byte = u8::from_str_radix(hex, 16).unwrap_or(b'?');
                let bytes = [byte];
                let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(&bytes);
                out.push_str(&decoded);
            }
        } else if let Some(c) = caps.get(6).map(|m| m.as_str()) {
            if curskip > 0 {
                curskip -= 1;
            } else if !ignorable {
                out.push_str(c);
            }
        }
    }
    out
}

fn rtf_special(word: &str) -> Option<&'static str> {
    Some(match word {
        "par" | "line" => "\n",
        "sect" | "page" => "\n\n",
        "tab" => "\t",
        "emdash" => "\u{2014}",
        "endash" => "\u{2013}",
        "emspace" => "\u{2003}",
        "enspace" => "\u{2002}",
        "qmspace" => "\u{2005}",
        "bullet" => "\u{2022}",
        "lquote" => "\u{2018}",
        "rquote" => "\u{2019}",
        "ldblquote" => "\u{201C}",
        "rdblquote" => "\u{201D}",
        _ => return None,
    })
}

/// Steuerwörter, deren Gruppe kein Text ist (Schriftentabelle, Farben, Kopfzeilen, Bilder …)
const RTF_DESTINATIONS: &[&str] = &[
    "aftncn", "aftnsep", "aftnsepc", "annotation", "atnauthor", "atndate", "atnicn", "atnid", "atnparent", "atnref", "atntime", "atrfend",
    "atrfstart", "author", "background", "bkmkend", "bkmkstart", "blipuid", "buptim", "category", "colorschememapping", "colortbl",
    "comment", "company", "creatim", "datafield", "datastore", "defchp", "defpap", "do", "doccomm", "docvar", "dptxbxtext", "ebcend",
    "ebcstart", "factoidname", "falt", "fchars", "ffdeftext", "ffentrymcr", "ffexitmcr", "ffformat", "ffhelptext", "ffl", "ffname",
    "ffstattext", "field", "file", "filetbl", "fldinst", "fldtype", "fname", "fontemb", "fontfile", "fonttbl", "footer", "footerf",
    "footerl", "footerr", "footnote", "formfield", "ftncn", "ftnsep", "ftnsepc", "g", "generator", "gridtbl", "header", "headerf",
    "headerl", "headerr", "hl", "hlfr", "hlinkbase", "hlloc", "hlsrc", "hsv", "htmltag", "info", "keycode", "keywords", "latentstyles",
    "lchars", "levelnumbers", "leveltext", "lfolevel", "linkval", "list", "listlevel", "listname", "listoverride", "listoverridetable",
    "listpicture", "liststylename", "listtable", "listtext", "lsdlockedexcept", "mailmerge", "manager", "mhtmltag", "mmaddfieldname",
    "mmconnectstr", "mmconnectstrdata", "mmdatasource", "mmheadersource", "mmmailsubject", "mmodso", "mmquery", "nesttableprops",
    "nextfile", "nonesttables", "objalias", "objclass", "objdata", "object", "objname", "objsect", "objtime", "oldcprops", "oldpprops",
    "oldsprops", "oldtprops", "oleclsid", "operator", "panose", "password", "passwordhash", "pgp", "pgptbl", "picprop", "pict", "pn",
    "pnseclvl", "pntext", "pntxta", "pntxtb", "printim", "private", "propname", "protend", "protstart", "protusertbl", "pxe", "result",
    "revtbl", "revtim", "rsidtbl", "rxe", "shp", "shpgrp", "shpinst", "shppict", "shprslt", "shptxt", "sn", "sp", "staticval",
    "stylesheet", "subject",
];

// ---------- Excel (.xls) ----------

fn extract_xls(data: &[u8]) -> Res<String> {
    use calamine::{Data, Reader, Xls};
    let mut workbook: Xls<_> = Xls::new(Cursor::new(data.to_vec())).map_err(|e| DocError::new(format!("Ungültige XLS-Datei: {e}")))?;
    let mut lines = Vec::new();
    for (name, range) in workbook.worksheets() {
        lines.push(format!("--- {name} ---"));
        // wie xlrd: Zeilen und Spalten zählen ab A1, auch wenn vorne nichts steht
        let Some(end) = range.end() else { continue };
        let (rows, cols) = (end.0 as usize + 1, end.1 as usize + 1);
        for r in 0..rows {
            let line: Vec<String> = (0..cols)
                .map(|c| match range.get_value((r as u32, c as u32)) {
                    None | Some(Data::Empty) => String::new(),
                    Some(Data::String(s)) => s.clone(),
                    Some(Data::Float(f)) => py_float(*f),
                    Some(Data::Int(i)) => py_float(*i as f64),
                    Some(Data::Bool(b)) => (*b as i32).to_string(),
                    Some(Data::DateTime(d)) => py_float(d.as_f64()),
                    Some(Data::DateTimeIso(s)) | Some(Data::DurationIso(s)) => s.clone(),
                    Some(Data::Error(_)) => "0".to_string(),
                })
                .collect();
            lines.push(line.join("\t"));
        }
    }
    Ok(lines.join("\n"))
}

// ---------- OpenDocument ----------

const TEXT: &str = "urn:oasis:names:tc:opendocument:xmlns:text:1.0";
const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
const DRAW: &str = "urn:oasis:names:tc:opendocument:xmlns:drawing:1.0";

fn content_xml(data: &[u8]) -> Res<String> {
    let mut zip = open_zip(data)?;
    zip_required(&mut zip, "content.xml")
}

fn extract_odt(data: &[u8]) -> Res<String> {
    let xml = content_xml(data)?;
    let doc = Document::parse(&xml)?;
    Ok(doc.descendants().filter(|n| is(*n, TEXT, "p")).map(itertext).collect::<Vec<_>>().join("\n"))
}

fn extract_ods(data: &[u8]) -> Res<String> {
    let xml = content_xml(data)?;
    let doc = Document::parse(&xml)?;
    let mut lines = Vec::new();
    for table in doc.descendants().filter(|n| is(*n, TABLE, "table")) {
        lines.push(format!("--- {} ---", attr_ns(table, TABLE, "name").unwrap_or("Sheet")));
        for row in table.descendants().filter(|n| is(*n, TABLE, "table-row")) {
            let cells: Vec<String> = row
                .descendants()
                .filter(|n| is(*n, TABLE, "table-cell"))
                .map(|cell| cell.descendants().filter(|n| is(*n, TEXT, "p")).map(itertext).collect::<Vec<_>>().join(" "))
                .collect();
            lines.push(cells.join("\t"));
        }
    }
    Ok(lines.join("\n"))
}

fn extract_odp(data: &[u8]) -> Res<String> {
    let xml = content_xml(data)?;
    let doc = Document::parse(&xml)?;
    let mut lines = Vec::new();
    for (i, page) in doc.descendants().filter(|n| is(*n, DRAW, "page")).enumerate() {
        lines.push(format!("--- Slide {} ---", i + 1));
        for p in page.descendants().filter(|n| is(*n, TEXT, "p")) {
            let text = itertext(p);
            let text = text.trim();
            if !text.is_empty() {
                lines.push(text.to_string());
            }
        }
    }
    Ok(lines.join("\n"))
}

// ---------- EPUB ----------

fn extract_epub(data: &[u8]) -> Res<String> {
    let mut zip = open_zip(data)?;
    let container_xml = zip_required(&mut zip, "META-INF/container.xml")?;
    let container = Document::parse(&container_xml)?;
    let rootfile = container.descendants().find(|n| n.is_element() && local(*n) == "rootfile").and_then(|n| n.attribute("full-path").map(str::to_string));
    let mut chapters: Vec<String> = Vec::new();
    let mut chapter = |zip: &mut zip::ZipArchive<Cursor<&[u8]>>, path: &str| {
        let Ok(mut file) = zip.by_name(path) else { return };
        let mut bytes = Vec::new();
        if file.read_to_end(&mut bytes).is_err() {
            return;
        }
        if let Some(body) = html_body_text(&String::from_utf8_lossy(&bytes)) {
            chapters.push(body.trim().to_string());
        }
    };
    match rootfile {
        Some(opf_path) => {
            let opf_dir = opf_path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
            let opf_xml = zip_required(&mut zip, &opf_path)?;
            let opf = Document::parse(&opf_xml)?;
            let manifest: HashMap<&str, &str> =
                opf.descendants().filter(|n| n.is_element() && local(*n) == "item").filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?))).collect();
            let spine: Vec<&str> = opf.descendants().filter(|n| n.is_element() && local(*n) == "itemref").filter_map(|n| n.attribute("idref")).collect();
            for idref in spine {
                let href = manifest.get(idref).copied().unwrap_or("");
                let item_path = if href.starts_with('/') { href.trim_start_matches('/').to_string() } else { format!("{opf_dir}{href}") };
                chapter(&mut zip, &item_path);
            }
        }
        None => {
            let names: Vec<String> = zip.file_names().filter(|n| n.ends_with(".html") || n.ends_with(".xhtml") || n.ends_with(".htm")).map(str::to_string).collect();
            for name in names {
                chapter(&mut zip, &name);
            }
        }
    }
    Ok(chapters.join("\n\n"))
}

// ---------- HTML ----------

/// Text unter `<body>` (alle Textknoten, ohne Tags und Kommentare); `None` ohne `<body>`
fn html_body_text(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start = lower.find("<body")?;
    let open_end = html[start..].find('>')? + start + 1;
    let end = lower.rfind("</body").filter(|e| *e >= open_end).unwrap_or(html.len());
    Some(html_text(&html[open_end..end]))
}

/// Text eines HTML-Stücks: Tags und Kommentare entfernen, Zeichenverweise auflösen
fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&decode_entities(&rest[..i]));
        let after = &rest[i..];
        if let Some(comment) = after.strip_prefix("<!--") {
            rest = comment.find("-->").map(|e| &comment[e + 3..]).unwrap_or("");
        } else if let Some(e) = after.find('>') {
            rest = &after[e + 1..];
        } else {
            rest = "";
        }
    }
    out.push_str(&decode_entities(rest));
    out
}

fn decode_entities(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let pattern = regex::Regex::new(r"&(#[0-9]+|#[xX][0-9a-fA-F]+|[A-Za-z][A-Za-z0-9]*);").expect("Regex");
    pattern
        .replace_all(text, |caps: &regex::Captures| {
            let name = &caps[1];
            let decoded = if let Some(num) = name.strip_prefix('#') {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse().ok(),
                };
                code.and_then(char::from_u32).map(|c| c.to_string())
            } else {
                named_entity(name).map(str::to_string)
            };
            decoded.unwrap_or_else(|| caps[0].to_string())
        })
        .into_owned()
}

fn named_entity(name: &str) -> Option<&'static str> {
    Some(match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        "nbsp" => "\u{a0}",
        "copy" => "\u{a9}",
        "reg" => "\u{ae}",
        "hellip" => "\u{2026}",
        "mdash" => "\u{2014}",
        "ndash" => "\u{2013}",
        "lsquo" => "\u{2018}",
        "rsquo" => "\u{2019}",
        "ldquo" => "\u{201C}",
        "rdquo" => "\u{201D}",
        "laquo" => "\u{ab}",
        "raquo" => "\u{bb}",
        "euro" => "\u{20ac}",
        "auml" => "ä",
        "ouml" => "ö",
        "uuml" => "ü",
        "Auml" => "Ä",
        "Ouml" => "Ö",
        "Uuml" => "Ü",
        "szlig" => "ß",
        "eacute" => "é",
        "egrave" => "è",
        "ecirc" => "ê",
        "agrave" => "à",
        "acirc" => "â",
        "ccedil" => "ç",
        "Eacute" => "É",
        "iuml" => "ï",
        "ocirc" => "ô",
        "ucirc" => "û",
        _ => return None,
    })
}

// ---------- E-Mail (.eml) ----------

fn extract_eml(data: &[u8]) -> Res<String> {
    let mail = mailparse::parse_mail(data).map_err(|e| DocError::new(format!("Ungültige E-Mail: {e}")))?;
    let mut lines = Vec::new();
    for header in ["From", "To", "Cc", "Date", "Subject"] {
        if let Some(value) = mail.headers.iter().find(|h| h.get_key().eq_ignore_ascii_case(header)).map(|h| h.get_value()) {
            if !value.is_empty() {
                lines.push(format!("{header}: {value}"));
            }
        }
    }
    lines.push(String::new()); // Leerzeile nach den Kopfzeilen
    // Der Text: zuerst text/plain, sonst text/html (Anhänge zählen nicht)
    fn find<'a>(part: &'a mailparse::ParsedMail<'a>, kind: &str) -> Option<&'a mailparse::ParsedMail<'a>> {
        let attachment = part.get_content_disposition().disposition == mailparse::DispositionType::Attachment;
        if part.subparts.is_empty() {
            return (!attachment && part.ctype.mimetype.eq_ignore_ascii_case(kind)).then_some(part);
        }
        part.subparts.iter().find_map(|p| find(p, kind))
    }
    if let Some(body) = find(&mail, "text/plain") {
        lines.push(body.get_body().map_err(|e| DocError::new(e.to_string()))?);
    } else if let Some(body) = find(&mail, "text/html") {
        lines.push(html_text(&body.get_body().map_err(|e| DocError::new(e.to_string()))?));
    }
    Ok(lines.join("\n"))
}

// ---------- PDF ----------

/// Wie `extract_pdf`, aber ein Fehler im PDF-Parser (Panic bei kaputten oder ungewöhnlichen Dateien) wird zur
/// Fehlermeldung statt den ganzen Terminal-Server zu beenden. Das Release-Profil wickelt dafür ab (kein `panic = "abort"`).
fn extract_pdf_safe(path: &str) -> Res<String> {
    let p = path.to_string();
    std::panic::catch_unwind(move || extract_pdf(&p))
        .unwrap_or_else(|_| Err(DocError::new("PDF nicht lesbar: Die Datei hat einen Aufbau, den der Leser nicht versteht.")))
}

/// Text aller Seiten (lopdf, reines Rust), jede Seite mit einer Trennzeile `--- Seite N ---` davor (wie
/// `--- Slide N ---` bei PowerPoint), damit das Modell Seiten nennen kann (ENTSCHEIDE E84). Seiten ohne
/// lesbaren Text geben eine leere Zeile.
fn extract_pdf(path: &str) -> Res<String> {
    let pages = pdf_pages(path)?;
    let mut lines = Vec::with_capacity(pages.len() * 2);
    for (i, text) in pages.into_iter().enumerate() {
        lines.push(format!("--- Seite {} ---", i + 1));
        lines.push(text);
    }
    Ok(lines.join("\n"))
}

/// Text jeder Seite eines PDF, in der Reihenfolge der Seiten
fn pdf_pages(path: &str) -> Res<Vec<String>> {
    let doc = lopdf::Document::load(path).map_err(|e| DocError::new(format!("PDF nicht lesbar: {e}")))?;
    if doc.is_encrypted() {
        return Err(DocError::new("Verschlüsselte PDF-Datei. Bitte eine unverschlüsselte Kopie angeben."));
    }
    let pages: Vec<u32> = doc.get_pages().keys().copied().collect();
    Ok(pages.into_iter().map(|page| doc.extract_text(&[page]).unwrap_or_default()).collect())
}

/// Wie `pdf_pages`, aber ein Panic im PDF-Parser wird zum Fehler (für die Quellen, ENTSCHEIDE E84)
pub fn pdf_pages_safe(path: &str) -> Res<Vec<String>> {
    let p = path.to_string();
    std::panic::catch_unwind(move || pdf_pages(&p))
        .unwrap_or_else(|_| Err(DocError::new("PDF nicht lesbar: Die Datei hat einen Aufbau, den der Leser nicht versteht.")))
}

/// Word in Abschnitten nach Überschriften (für die Quellen, ENTSCHEIDE E84): (Überschrift, Text). Der Text vor
/// der ersten Überschrift hat eine leere Überschrift. Überschrift ist ein Absatz mit einer Formatvorlage
/// «Heading …», «Überschrift …» (Kennung `berschrift…`), «Title»/«Titel» oder mit Gliederungsebene.
pub fn docx_sections(data: &[u8]) -> Res<Vec<(String, String)>> {
    let mut zip = open_zip(data)?;
    let main = zip_required(&mut zip, "word/document.xml")?;
    let doc = Document::parse(&main)?;
    let body = doc.root_element().children().find(|n| is(*n, W, "body")).ok_or_else(|| DocError::new("Kein Dokumentkörper in word/document.xml"))?;
    let mut sections: Vec<(String, Vec<String>)> = vec![(String::new(), Vec::new())];
    for child in body.children().filter(|c| c.is_element() && c.tag_name().namespace() == Some(W)) {
        if local(child) == "p" && is_heading(child) {
            let title = paragraph_text(child).trim().to_string();
            if !title.is_empty() {
                sections.push((title, Vec::new()));
                continue;
            }
        }
        let lines = match local(child) {
            "p" => vec![paragraph_text(child)],
            "tbl" => table_rows(child),
            "sdt" | "customXml" => blocks(child),
            _ => Vec::new(),
        };
        if let Some(last) = sections.last_mut() {
            last.1.extend(lines);
        }
    }
    Ok(sections.into_iter().map(|(t, lines)| (t, lines.join("\n"))).filter(|(t, text)| !t.is_empty() || !text.trim().is_empty()).collect())
}

fn is_heading(p: Node) -> bool {
    let Some(ppr) = p.children().find(|c| is(*c, W, "pPr")) else { return false };
    if children_named(ppr, W, "outlineLvl").next().is_some() {
        return true;
    }
    let style = children_named(ppr, W, "pStyle").next().and_then(|s| attr_ns(s, W, "val")).unwrap_or("").to_lowercase();
    style.starts_with("heading") || style.contains("berschrift") || style == "title" || style == "titel"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_of(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (name, content) in files {
                zip.start_file(*name, options).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    const WNS: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;

    #[test]
    fn docx_mit_inhaltssteuerelementen_tabelle_und_kopfzeile() {
        // Port von test_read_docx_with_content_controls: Deckblatt in <w:sdt>, dazu Kopfzeile und Tabelle
        let document = format!(
            r#"<w:document {WNS}><w:body>
            <w:sdt><w:sdtContent><w:p><w:r><w:t>Deckblatt Titel</w:t></w:r></w:p>
              <w:tbl><w:tblGrid><w:gridCol/></w:tblGrid><w:tr><w:tc><w:p><w:r><w:t>Name: Kayu</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:sdtContent></w:sdt>
            <w:p><w:r><w:t>Haupttext</w:t></w:r><w:r><w:tab/><w:t>danach</w:t></w:r></w:p>
            <w:p><w:del><w:r><w:delText>gelöscht</w:delText></w:r></w:del><w:ins><w:r><w:t>eingefügt</w:t></w:r></w:ins></w:p>
            <w:sectPr><w:headerReference w:type="default" r:id="rId1"/></w:sectPr></w:body></w:document>"#
        );
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x" Target="header1.xml"/></Relationships>"#;
        let header = format!(r#"<w:hdr {WNS}><w:p><w:r><w:t>Kopf mit Logo</w:t></w:r></w:p></w:hdr>"#);
        let data = zip_of(&[("word/document.xml", &document), ("word/_rels/document.xml.rels", rels), ("word/header1.xml", &header)]);
        let text = extract_docx(&data).unwrap();
        for wanted in ["Deckblatt Titel", "Name: Kayu", "Haupttext\tdanach", "eingefügt", "Kopf mit Logo", "--- Kopfzeile (Abschnitt 1) ---"] {
            assert!(text.contains(wanted), "{wanted} fehlt in {text:?}");
        }
        assert!(!text.contains("gelöscht"));
    }

    #[test]
    fn docx_verbundene_zellen_wiederholen_den_text() {
        let document = format!(
            r#"<w:document {WNS}><w:body><w:tbl><w:tblGrid><w:gridCol/><w:gridCol/></w:tblGrid>
            <w:tr><w:tc><w:tcPr><w:gridSpan w:val="2"/></w:tcPr><w:p><w:r><w:t>breit</w:t></w:r></w:p></w:tc></w:tr>
            <w:tr><w:tc><w:p><w:r><w:t>a</w:t></w:r></w:p></w:tc><w:tc><w:p><w:r><w:t>b</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#
        );
        let data = zip_of(&[("word/document.xml", &document)]);
        assert_eq!(extract_docx(&data).unwrap(), "breit\tbreit\na\tb");
    }

    #[test]
    fn xlsx_zahlen_texte_und_datum() {
        let ns = r#"xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let workbook = format!(r#"<workbook {ns}><sheets><sheet name="Tabelle1" sheetId="1" r:id="rId1"/></sheets></workbook>"#);
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="x" Target="worksheets/sheet1.xml"/></Relationships>"#;
        let shared = format!(r#"<sst {ns}><si><t>Name</t></si><si><r><t>Grü</t></r><r><t>ezi</t></r></si></sst>"#);
        let styles = format!(r#"<styleSheet {ns}><cellXfs><xf numFmtId="0"/><xf numFmtId="14"/></cellXfs></styleSheet>"#);
        let sheet = format!(
            r#"<worksheet {ns}><dimension ref="A1:C3"/><sheetData>
            <row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row>
            <row r="3"><c r="A3"><v>3</v></c><c r="B3"><v>2.5</v></c><c r="C3" s="1"><v>45322</v></c></row></sheetData></worksheet>"#
        );
        let data = zip_of(&[
            ("xl/workbook.xml", &workbook),
            ("xl/_rels/workbook.xml.rels", rels),
            ("xl/sharedStrings.xml", &shared),
            ("xl/styles.xml", &styles),
            ("xl/worksheets/sheet1.xml", &sheet),
        ]);
        assert_eq!(extract_xlsx(&data).unwrap(), "--- Tabelle1 ---\nName\t\tGrüezi\n\t\t\n3\t2.5\t2024-01-31 00:00:00");
    }

    #[test]
    fn python_gleitkommadarstellung() {
        assert_eq!(py_float(3.0), "3.0");
        assert_eq!(py_float(0.1), "0.1");
        assert_eq!(py_float(-2.5), "-2.5");
        assert_eq!(py_float(1e22), "1e+22");
        assert_eq!(py_float(1.5e-7), "1.5e-07");
        assert_eq!(py_float(123456789012345680.0), "1.2345678901234568e+17");
        assert_eq!(py_float(0.0001), "0.0001");
        assert_eq!(py_float(1e16), "1e+16");
    }

    #[test]
    fn pptx_folien_und_formen() {
        let ns = r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
        let presentation = format!(r#"<p:presentation {ns}><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst></p:presentation>"#);
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId2" Type="x" Target="slides/slide1.xml"/></Relationships>"#;
        let slide = format!(
            r#"<p:sld {ns}><p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>Titel</a:t></a:r></a:p><a:p><a:r><a:t>Zeile</a:t></a:r><a:br/><a:r><a:t>zwei</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
        );
        let data = zip_of(&[("ppt/presentation.xml", &presentation), ("ppt/_rels/presentation.xml.rels", rels), ("ppt/slides/slide1.xml", &slide)]);
        assert_eq!(extract_pptx(&data).unwrap(), "--- Slide 1 ---\nTitel\nZeile\u{b}zwei");
    }

    #[test]
    fn odt_ods_odp() {
        let odt = zip_of(&[("content.xml", r#"<o xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"><text:p>Eins <text:span>zwei</text:span></text:p><text:p>Drei</text:p></o>"#)]);
        assert_eq!(extract_odt(&odt).unwrap(), "Eins zwei\nDrei");
        let ods = zip_of(&[(
            "content.xml",
            r#"<o xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"><table:table table:name="Blatt"><table:table-row><table:table-cell><text:p>a</text:p></table:table-cell><table:table-cell><text:p>b</text:p></table:table-cell></table:table-row></table:table></o>"#,
        )]);
        assert_eq!(extract_ods(&ods).unwrap(), "--- Blatt ---\na\tb");
        let odp = zip_of(&[(
            "content.xml",
            r#"<o xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"><draw:page><text:p> Titel </text:p><text:p/></draw:page></o>"#,
        )]);
        assert_eq!(extract_odp(&odp).unwrap(), "--- Slide 1 ---\nTitel");
    }

    #[test]
    fn rtf_text_umlaute_und_gruppen() {
        let rtf = br"{\rtf1\ansi{\fonttbl{\f0 Arial;}}\f0 Gr\'fc\'dfe\par Zweite Zeile \u8364? ende}";
        assert_eq!(extract_rtf(rtf), "Gr\u{fc}\u{df}e\nZweite Zeile \u{20ac} ende");
    }

    #[test]
    fn epub_kapitel_in_der_reihenfolge_des_buchs() {
        let container = r#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="OEBPS/content.opf"/></rootfiles></container>"#;
        let opf = r#"<package><manifest><item id="a" href="a.xhtml"/><item id="b" href="b.xhtml"/></manifest><spine><itemref idref="b"/><itemref idref="a"/></spine></package>"#;
        let data = zip_of(&[
            ("META-INF/container.xml", container),
            ("OEBPS/content.opf", opf),
            ("OEBPS/a.xhtml", "<html><body><p>Erstens &amp; zweitens</p></body></html>"),
            ("OEBPS/b.xhtml", "<html><body><h1>Titel</h1><!-- weg --></body></html>"),
        ]);
        assert_eq!(extract_epub(&data).unwrap(), "Titel\n\nErstens & zweitens");
    }

    #[test]
    fn eml_kopfzeilen_und_text() {
        let mail = b"From: a@example.ch\r\nTo: b@example.ch\r\nSubject: =?UTF-8?Q?Gr=C3=BCezi?=\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nHallo Welt\r\n";
        assert_eq!(extract_eml(mail).unwrap(), "From: a@example.ch\nTo: b@example.ch\nSubject: Grüezi\n\nHallo Welt\r\n");
    }

    #[test]
    fn pdf_text() {
        let content = "BT /F1 18 Tf 20 100 Td (Hallo PDF-Welt) Tj ET";
        let mut pdf = String::from("%PDF-1.4\n");
        let mut offsets = Vec::new();
        let objects = [
            "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
            "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_string(),
            format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_string(),
        ];
        for (i, o) in objects.iter().enumerate() {
            offsets.push(pdf.len());
            pdf.push_str(&format!("{} 0 obj\n{o}\nendobj\n", i + 1));
        }
        let xref = pdf.len();
        pdf.push_str(&format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1));
        for o in &offsets {
            pdf.push_str(&format!("{o:010} 00000 n \n"));
        }
        pdf.push_str(&format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.pdf");
        std::fs::write(&path, pdf).unwrap();
        let text = extract_pdf(&path.to_string_lossy()).unwrap();
        assert!(text.contains("Hallo PDF-Welt"), "{text:?}");
        assert!(text.starts_with("--- Seite 1 ---\n"), "Seite vor dem Text: {text:?}");
        assert_eq!(pdf_pages_safe(&path.to_string_lossy()).unwrap().len(), 1);
    }

    #[test]
    fn word_in_abschnitten() {
        let doc = format!(
            r#"<w:document xmlns:w="{W}"><w:body>
            <w:p><w:r><w:t>Vorwort</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="berschrift1"/></w:pPr><w:r><w:t>Zellen</w:t></w:r></w:p>
            <w:p><w:r><w:t>Zellen haben einen Kern.</w:t></w:r></w:p>
            <w:p><w:pPr><w:pStyle w:val="Heading2"/></w:pPr><w:r><w:t>Photosynthese</w:t></w:r></w:p>
            <w:p><w:r><w:t>Licht wird zu Zucker.</w:t></w:r></w:p>
            <w:p><w:pPr><w:outlineLvl w:val="0"/></w:pPr><w:r><w:t>Anhang</w:t></w:r></w:p>
            </w:body></w:document>"#
        );
        let data = zip_of(&[("word/document.xml", &doc)]);
        let s = docx_sections(&data).unwrap();
        let titles: Vec<&str> = s.iter().map(|(t, _)| t.as_str()).collect();
        assert_eq!(titles, ["", "Zellen", "Photosynthese", "Anhang"]);
        assert_eq!(s[0].1, "Vorwort");
        assert_eq!(s[2].1, "Licht wird zu Zucker.");
        assert_eq!(s[3].1, "");
    }

    #[test]
    fn nur_bekannte_endungen() {
        assert!(supports("a/B.DOCX"));
        assert!(supports("x.pdf"));
        assert!(!supports("x.doc"));
        assert!(!supports("x.png"));
    }
}
