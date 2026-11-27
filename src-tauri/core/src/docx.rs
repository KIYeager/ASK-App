//! Word-Datei aus einem Doc (ENTSCHEIDE E81): Markdown wird zu einer .docx mit echten Formatvorlagen (Überschrift 1
//! bis 6, Zitat, Code, Listen, Tabellen), damit Inhaltsverzeichnis, Navigationsbereich und eigene Formatierung in Word
//! wie gewohnt gehen. Ohne Python und ohne Office: Die Datei ist ein ZIP mit XML, das hier geschrieben wird.
//!
//! Verstanden wird das Markdown, das der Editor im Reiter «Docs» schreibt und das ASK schreiben soll: Überschriften
//! mit #, Absätze, Listen mit -, * oder 1. (eingerückt verschachtelt), Zitate mit >, Codeblöcke mit ```, Tabellen
//! mit |, Trennlinien und im Text **fett**, *kursiv*, ~~durchgestrichen~~, `Code` und [Links](https://…).
//!
//! Öffentliche Schnittstelle: `markdown_to_docx(markdown, titel) -> Result<Vec<u8>>`, `parse_blocks`, `parse_inline`,
//! `Block`, `Span`.

use anyhow::Result;
use std::io::Write;

/// Ein Stück Text mit Auszeichnung
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub link: Option<String>,
}

/// Ein Block des Docs
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Heading(usize, String),
    Paragraph(String),
    /// Listenpunkt: Tiefe (0 = oben), nummeriert, Text, Nummer der Liste (neue Zählung je Liste)
    Item { level: usize, ordered: bool, text: String, list: usize },
    Quote(String),
    Code(String),
    Table(Vec<Vec<String>>),
    Rule,
}

fn is_rule(t: &str) -> bool {
    let c: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    c.len() >= 3 && (c.chars().all(|x| x == '-') || c.chars().all(|x| x == '*') || c.chars().all(|x| x == '_'))
}

fn heading(t: &str) -> Option<(usize, String)> {
    let n = t.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&n) && t[n..].starts_with(' ') {
        return Some((n, t[n..].trim().trim_end_matches('#').trim().to_string()));
    }
    None
}

/// Listenpunkt: (Einrückung, nummeriert, Text)
fn list_item(line: &str) -> Option<(usize, bool, String)> {
    let indent = line.chars().take_while(|c| *c == ' ' || *c == '\t').map(|c| if c == '\t' { 4 } else { 1 }).sum::<usize>();
    let t = line.trim_start();
    for m in ["- ", "* ", "+ "] {
        if let Some(rest) = t.strip_prefix(m) {
            return Some((indent, false, rest.to_string()));
        }
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &t[digits..];
        if let Some(r) = rest.strip_prefix(". ").or_else(|| rest.strip_prefix(") ")) {
            return Some((indent, true, r.to_string()));
        }
    }
    None
}

fn table_cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    // \| bleibt ein Zeichen in der Zelle
    let mut cells = vec![String::new()];
    let mut chars = t.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'|') {
            cells.last_mut().expect("Zelle").push('|');
            chars.next();
        } else if c == '|' {
            cells.push(String::new());
        } else {
            cells.last_mut().expect("Zelle").push(c);
        }
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

fn is_table_separator(line: &str) -> bool {
    let t = line.trim();
    t.contains('-') && t.contains('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' ' | '\t'))
}

/// Markdown in Blöcke
pub fn parse_blocks(md: &str) -> Vec<Block> {
    let lines: Vec<&str> = md.lines().collect();
    let mut out = Vec::new();
    let mut para: Vec<String> = Vec::new();
    let mut list_no = 0usize;
    let mut in_list = false;
    let mut top_ordered: Option<bool> = None;
    let mut indents: Vec<usize> = Vec::new();
    let flush = |para: &mut Vec<String>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Paragraph(para.join("\n")));
            para.clear();
        }
    };
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if t.starts_with("```") || t.starts_with("~~~") {
            flush(&mut para, &mut out);
            in_list = false;
            let fence = &t[..3];
            let mut code = Vec::new();
            i += 1;
            while i < lines.len() && !lines[i].trim().starts_with(fence) {
                code.push(lines[i]);
                i += 1;
            }
            out.push(Block::Code(code.join("\n")));
            i += 1;
            continue;
        }
        if t.is_empty() {
            flush(&mut para, &mut out);
            // Eine Leerzeile beendet die Liste erst, wenn danach kein Listenpunkt mehr kommt
            let next = lines[i + 1..].iter().find(|l| !l.trim().is_empty());
            if !next.is_some_and(|l| list_item(l).is_some()) {
                in_list = false;
            }
            i += 1;
            continue;
        }
        if let Some((level, text)) = heading(t) {
            flush(&mut para, &mut out);
            in_list = false;
            out.push(Block::Heading(level, text));
            i += 1;
            continue;
        }
        if is_rule(t) && para.is_empty() {
            in_list = false;
            out.push(Block::Rule);
            i += 1;
            continue;
        }
        if t.starts_with('|') && i + 1 < lines.len() && is_table_separator(lines[i + 1]) {
            flush(&mut para, &mut out);
            in_list = false;
            let mut rows = vec![table_cells(t)];
            i += 2;
            while i < lines.len() && lines[i].trim().starts_with('|') {
                rows.push(table_cells(lines[i]));
                i += 1;
            }
            out.push(Block::Table(rows));
            continue;
        }
        if let Some((indent, ordered, text)) = list_item(line) {
            flush(&mut para, &mut out);
            if !in_list {
                indents.clear();
            }
            // Tiefe aus den Einrückungen der Punkte darüber (wie in Markdown: unter «1. » drei, unter «- » zwei Leerzeichen)
            while indents.last().is_some_and(|top| indent < *top) {
                indents.pop();
            }
            if indents.last().is_none_or(|top| indent > *top) {
                indents.push(indent);
            }
            let level = (indents.len() - 1).min(8);
            // Neue Liste: nach anderem Inhalt, oder oben ein Wechsel zwischen Aufzählung und Nummerierung
            if !in_list || (level == 0 && top_ordered != Some(ordered)) {
                list_no += 1;
                in_list = true;
            }
            if level == 0 {
                top_ordered = Some(ordered);
            }
            out.push(Block::Item { level, ordered, text, list: list_no });
            i += 1;
            continue;
        }
        if let Some(rest) = t.strip_prefix('>') {
            flush(&mut para, &mut out);
            in_list = false;
            let mut q = vec![rest.trim_start().to_string()];
            i += 1;
            while i < lines.len() && lines[i].trim().starts_with('>') {
                q.push(lines[i].trim()[1..].trim_start().to_string());
                i += 1;
            }
            for part in q.join("\n").split("\n\n") {
                out.push(Block::Quote(part.to_string()));
            }
            continue;
        }
        // Fortsetzung eines Listenpunkts (eingerückt, direkt darunter)
        if in_list && para.is_empty() && line.starts_with([' ', '\t']) {
            if let Some(Block::Item { text, .. }) = out.last_mut() {
                text.push(' ');
                text.push_str(t);
                i += 1;
                continue;
            }
        }
        in_list = false;
        // Zeilenumbruch nur mit zwei Leerzeichen oder \ am Ende, sonst gehört die Zeile zum Absatz
        let hard = line.ends_with("  ") || line.ends_with('\\');
        let text = t.trim_end_matches('\\').to_string();
        match para.last_mut() {
            Some(last) if !last.ends_with('\n') => { last.push(' '); last.push_str(&text); }
            Some(last) => last.push_str(&text),
            None => para.push(text),
        }
        if hard {
            if let Some(last) = para.last_mut() { last.push('\n'); }
        }
        i += 1;
    }
    flush(&mut para, &mut out);
    out
}

fn closing(chars: &[char], from: usize, marker: &[char]) -> bool {
    let n = marker.len();
    (from..chars.len().saturating_sub(n - 1)).any(|k| chars[k..k + n] == *marker && (k == 0 || chars[k - 1] != '\\'))
}

/// Text mit Auszeichnungen in Stücke
pub fn parse_inline(text: &str) -> Vec<Span> {
    let chars: Vec<char> = text.chars().collect();
    let mut spans: Vec<Span> = Vec::new();
    let (mut bold, mut italic, mut strike) = (false, false, false);
    let mut buf = String::new();
    let push = |buf: &mut String, spans: &mut Vec<Span>, bold: bool, italic: bool, strike: bool| {
        if !buf.is_empty() {
            spans.push(Span { text: std::mem::take(buf), bold, italic, strike, ..Default::default() });
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        if c == '\\' && next.is_some_and(|n| "\\`*_~[]()#|>!-+.".contains(n)) {
            buf.push(next.expect("Zeichen"));
            i += 2;
            continue;
        }
        if c == '`' {
            if let Some(end) = (i + 1..chars.len()).find(|k| chars[*k] == '`') {
                push(&mut buf, &mut spans, bold, italic, strike);
                spans.push(Span { text: chars[i + 1..end].iter().collect(), bold, italic, strike, code: true, link: None });
                i = end + 1;
                continue;
            }
        }
        if c == '[' {
            // [Text](Adresse)
            if let Some(mid) = (i + 1..chars.len()).find(|k| chars[*k] == ']') {
                if chars.get(mid + 1) == Some(&'(') {
                    if let Some(end) = (mid + 2..chars.len()).find(|k| chars[*k] == ')') {
                        push(&mut buf, &mut spans, bold, italic, strike);
                        let label: String = chars[i + 1..mid].iter().collect();
                        let url: String = chars[mid + 2..end].iter().collect();
                        for mut s in parse_inline(&label) {
                            s.bold |= bold;
                            s.italic |= italic;
                            s.strike |= strike;
                            s.link = Some(url.trim().to_string());
                            spans.push(s);
                        }
                        i = end + 1;
                        continue;
                    }
                }
            }
        }
        let two = next.map(|n| [c, n]);
        if two == Some(['*', '*']) || two == Some(['_', '_']) {
            let m = [c, c];
            if bold || closing(&chars, i + 2, &m) {
                push(&mut buf, &mut spans, bold, italic, strike);
                bold = !bold;
                i += 2;
                continue;
            }
        }
        if two == Some(['~', '~']) && (strike || closing(&chars, i + 2, &['~', '~'])) {
            push(&mut buf, &mut spans, bold, italic, strike);
            strike = !strike;
            i += 2;
            continue;
        }
        let word_inside = |k: usize| k > 0 && chars[k - 1].is_alphanumeric() && chars.get(k + 1).is_some_and(|x| x.is_alphanumeric());
        if (c == '*' || (c == '_' && !word_inside(i))) && (italic || (next.is_some_and(|n| !n.is_whitespace()) && closing(&chars, i + 1, &[c]))) {
            push(&mut buf, &mut spans, bold, italic, strike);
            italic = !italic;
            i += 1;
            continue;
        }
        buf.push(c);
        i += 1;
    }
    push(&mut buf, &mut spans, bold, italic, strike);
    spans
}

fn esc(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            // In XML 1.0 verboten
            c if (c as u32) < 0x20 && !matches!(c, '\t' | '\n' | '\r') => {}
            c => o.push(c),
        }
    }
    o
}

struct Writer {
    body: String,
    links: Vec<String>,
    /// Nummern der Listen mit Zählung (je eine num in numbering.xml)
    ordered_lists: Vec<usize>,
}

impl Writer {
    fn runs(&mut self, text: &str, base_bold: bool) -> String {
        let mut out = String::new();
        for s in parse_inline(text) {
            let mut rpr = String::new();
            if s.link.is_some() {
                rpr.push_str("<w:rStyle w:val=\"Hyperlink\"/>");
            } else if s.code {
                rpr.push_str("<w:rStyle w:val=\"CodeZeichen\"/>");
            }
            if s.bold || base_bold {
                rpr.push_str("<w:b/>");
            }
            if s.italic {
                rpr.push_str("<w:i/>");
            }
            if s.strike {
                rpr.push_str("<w:strike/>");
            }
            let mut r = String::new();
            for (k, part) in s.text.split('\n').enumerate() {
                if k > 0 {
                    r.push_str(&format!("<w:r>{}<w:br/></w:r>", if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") }));
                }
                if !part.is_empty() {
                    r.push_str(&format!("<w:r>{}<w:t xml:space=\"preserve\">{}</w:t></w:r>", if rpr.is_empty() { String::new() } else { format!("<w:rPr>{rpr}</w:rPr>") }, esc(part)));
                }
            }
            match s.link.filter(|u| u.starts_with("https://") || u.starts_with("http://") || u.starts_with("mailto:")) {
                Some(url) => {
                    self.links.push(url);
                    out.push_str(&format!("<w:hyperlink r:id=\"rIdL{}\" w:history=\"1\">{r}</w:hyperlink>", self.links.len()));
                }
                None => out.push_str(&r),
            }
        }
        out
    }

    fn para(&mut self, style: Option<&str>, extra_ppr: &str, text: &str, bold: bool) {
        let mut ppr = String::new();
        if let Some(s) = style {
            ppr.push_str(&format!("<w:pStyle w:val=\"{s}\"/>"));
        }
        ppr.push_str(extra_ppr);
        let runs = self.runs(text, bold);
        self.body.push_str(&format!("<w:p>{}{runs}</w:p>", if ppr.is_empty() { String::new() } else { format!("<w:pPr>{ppr}</w:pPr>") }));
    }

    fn block(&mut self, b: &Block) {
        match b {
            Block::Heading(level, text) => self.para(Some(&format!("Heading{level}")), "", text, false),
            Block::Paragraph(text) => self.para(None, "", text, false),
            Block::Quote(text) => self.para(Some("Quote"), "", text, false),
            Block::Item { level, ordered, text, list } => {
                let num = if *ordered {
                    let k = match self.ordered_lists.iter().position(|l| l == list) {
                        Some(k) => k,
                        None => { self.ordered_lists.push(*list); self.ordered_lists.len() - 1 }
                    };
                    k + 2
                } else {
                    1
                };
                self.para(Some("ListParagraph"), &format!("<w:numPr><w:ilvl w:val=\"{level}\"/><w:numId w:val=\"{num}\"/></w:numPr>"), text, false);
            }
            Block::Code(code) => {
                for line in code.split('\n') {
                    self.body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"Code\"/></w:pPr>{}</w:p>",
                        if line.is_empty() { String::new() } else { format!("<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r>", esc(line)) }));
                }
            }
            Block::Rule => self.body.push_str("<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"A6A6A6\"/></w:pBdr></w:pPr></w:p>"),
            Block::Table(rows) => {
                let cols = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
                let width = 9000 / cols;
                let grid: String = (0..cols).map(|_| format!("<w:gridCol w:w=\"{width}\"/>")).collect();
                let mut t = format!("<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblLook w:val=\"04A0\" w:firstRow=\"1\"/></w:tblPr><w:tblGrid>{grid}</w:tblGrid>");
                for (r, row) in rows.iter().enumerate() {
                    t.push_str(&format!("<w:tr>{}", if r == 0 { "<w:trPr><w:tblHeader/></w:trPr>" } else { "" }));
                    for c in 0..cols {
                        let text = row.get(c).cloned().unwrap_or_default();
                        let shade = if r == 0 { "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"E8EEF5\"/>" } else { "" };
                        let runs = self.runs(&text, r == 0);
                        t.push_str(&format!("<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>{shade}</w:tcPr><w:p><w:pPr><w:spacing w:before=\"40\" w:after=\"40\"/></w:pPr>{runs}</w:p></w:tc>"));
                    }
                    t.push_str("</w:tr>");
                }
                t.push_str("</w:tbl>");
                self.body.push_str(&t);
                self.body.push_str("<w:p/>"); // Word braucht nach einer Tabelle einen Absatz
            }
        }
    }
}

const NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"";

fn styles() -> String {
    let mut headings = String::new();
    let sizes = [36, 30, 26, 24, 22, 22];
    for (k, size) in sizes.iter().enumerate() {
        let n = k + 1;
        headings.push_str(&format!(
            "<w:style w:type=\"paragraph\" w:styleId=\"Heading{n}\"><w:name w:val=\"heading {n}\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:uiPriority w:val=\"9\"/><w:qFormat/>\
<w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"{}\" w:after=\"80\"/><w:outlineLvl w:val=\"{k}\"/></w:pPr>\
<w:rPr><w:b/><w:color w:val=\"{}\"/><w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/></w:rPr></w:style>",
            if n == 1 { 0 } else { 240 }, if n <= 2 { "1F4E79" } else { "262626" }));
    }
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:styles {NS}>\
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"Calibri\" w:cs=\"Calibri\"/><w:sz w:val=\"22\"/><w:szCs w:val=\"22\"/><w:lang w:val=\"de-CH\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"276\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
{headings}\
<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:pBdr><w:left w:val=\"single\" w:sz=\"18\" w:space=\"8\" w:color=\"9DB3C8\"/></w:pBdr><w:ind w:left=\"340\"/></w:pPr><w:rPr><w:i/><w:color w:val=\"404040\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/><w:pPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F2F2F2\"/><w:spacing w:after=\"0\" w:line=\"240\" w:lineRule=\"auto\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/><w:sz w:val=\"19\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"ListParagraph\"><w:name w:val=\"List Paragraph\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/><w:pPr><w:spacing w:after=\"40\"/><w:contextualSpacing/></w:pPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"CodeZeichen\"><w:name w:val=\"Code (Zeichen)\"/><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F2F2F2\"/></w:rPr></w:style>\
<w:style w:type=\"table\" w:default=\"1\" w:styleId=\"TableNormal\"><w:name w:val=\"Normal Table\"/><w:tblPr><w:tblInd w:w=\"0\" w:type=\"dxa\"/><w:tblCellMar><w:top w:w=\"0\" w:type=\"dxa\"/><w:left w:w=\"108\" w:type=\"dxa\"/><w:bottom w:w=\"0\" w:type=\"dxa\"/><w:right w:w=\"108\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:basedOn w:val=\"TableNormal\"/><w:tblPr><w:tblBorders>\
<w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:left w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
<w:right w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"BFBFBF\"/>\
</w:tblBorders></w:tblPr></w:style></w:styles>")
}

fn numbering(ordered: usize) -> String {
    let bullets = ["•", "◦", "▪"];
    let mut bullet_levels = String::new();
    let mut number_levels = String::new();
    for l in 0..9 {
        let ind = 360 + 360 * l;
        bullet_levels.push_str(&format!(
            "<w:lvl w:ilvl=\"{l}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"{}\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{ind}\" w:hanging=\"360\"/></w:pPr></w:lvl>",
            bullets[l % 3]));
        let fmt = ["decimal", "lowerLetter", "lowerRoman"][l % 3];
        number_levels.push_str(&format!(
            "<w:lvl w:ilvl=\"{l}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{fmt}\"/><w:lvlText w:val=\"%{}.\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{ind}\" w:hanging=\"360\"/></w:pPr></w:lvl>",
            l + 1));
    }
    let mut nums = "<w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>".to_string();
    for k in 0..ordered {
        // Jede nummerierte Liste zählt neu ab 1
        let overrides: String = (0..9).map(|l| format!("<w:lvlOverride w:ilvl=\"{l}\"><w:startOverride w:val=\"1\"/></w:lvlOverride>")).collect();
        nums.push_str(&format!("<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"1\"/>{overrides}</w:num>", k + 2));
    }
    format!("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:numbering {NS}>\
<w:abstractNum w:abstractNumId=\"0\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{bullet_levels}</w:abstractNum>\
<w:abstractNum w:abstractNumId=\"1\"><w:multiLevelType w:val=\"hybridMultilevel\"/>{number_levels}</w:abstractNum>{nums}</w:numbering>")
}

/// Word-Datei aus Markdown
pub fn markdown_to_docx(md: &str, title: &str) -> Result<Vec<u8>> {
    let mut w = Writer { body: String::new(), links: Vec::new(), ordered_lists: Vec::new() };
    for b in parse_blocks(md) {
        w.block(&b);
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:document {NS}><w:body>{}\
<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/><w:pgMar w:top=\"1417\" w:right=\"1417\" w:bottom=\"1134\" w:left=\"1417\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>",
        w.body);
    let mut rels = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rIdS\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
<Relationship Id=\"rIdN\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/numbering\" Target=\"numbering.xml\"/>");
    for (k, url) in w.links.iter().enumerate() {
        rels.push_str(&format!("<Relationship Id=\"rIdL{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>", k + 1, esc(url)));
    }
    rels.push_str("</Relationships>");
    let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ");
    let core = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
<dc:title>{}</dc:title><dc:creator>ASK</dc:creator><dc:language>de-CH</dc:language>\
<dcterms:created xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:created><dcterms:modified xsi:type=\"dcterms:W3CDTF\">{now}</dcterms:modified></cp:coreProperties>",
        esc(title));
    let parts: Vec<(&str, String)> = vec![
        ("[Content_Types].xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/></Types>".to_string()),
        ("_rels/.rels", "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/></Relationships>".to_string()),
        ("word/document.xml", document),
        ("word/_rels/document.xml.rels", rels),
        ("word/styles.xml", styles()),
        ("word/numbering.xml", numbering(w.ordered_lists.len())),
        ("docProps/core.xml", core),
    ];
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, content) in parts {
        zip.start_file(name, opts)?;
        zip.write_all(content.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bloecke() {
        let md = "# Titel\n\nErster Absatz\nzweite Zeile.\n\n## Liste\n\n- a\n  - b\n- c\n\n1. eins\n   1. unter eins\n2. zwei\n\n> Zitat\n\n```\ncode  hier\n```\n\n| A | B |\n|---|---|\n| 1 | 2 \\| 3 |\n\n---\n";
        let b = parse_blocks(md);
        assert_eq!(b[0], Block::Heading(1, "Titel".into()));
        assert_eq!(b[1], Block::Paragraph("Erster Absatz zweite Zeile.".into()));
        assert_eq!(b[3], Block::Item { level: 0, ordered: false, text: "a".into(), list: 1 });
        assert_eq!(b[4], Block::Item { level: 1, ordered: false, text: "b".into(), list: 1 });
        assert_eq!(b[6], Block::Item { level: 0, ordered: true, text: "eins".into(), list: 2 }, "neue Liste nach Leerzeile mit anderem Typ zählt neu");
        assert_eq!(b[7], Block::Item { level: 1, ordered: true, text: "unter eins".into(), list: 2 }, "drei Leerzeichen unter «1. »");
        assert_eq!(b[8], Block::Item { level: 0, ordered: true, text: "zwei".into(), list: 2 });
        assert!(b.contains(&Block::Quote("Zitat".into())));
        assert!(b.contains(&Block::Code("code  hier".into())));
        assert!(b.contains(&Block::Table(vec![vec!["A".into(), "B".into()], vec!["1".into(), "2 | 3".into()]])));
        assert_eq!(b.last(), Some(&Block::Rule));
    }

    #[test]
    fn auszeichnungen() {
        let s = parse_inline("Ein **fettes** und *kursives* Wort, `x*y`, ~~weg~~ und [Link](https://example.org).");
        let find = |t: &str| s.iter().find(|x| x.text == t).cloned().unwrap_or_else(|| panic!("{t}: {s:?}"));
        assert!(find("fettes").bold);
        assert!(find("kursives").italic);
        assert!(find("x*y").code);
        assert!(find("weg").strike);
        assert_eq!(find("Link").link.as_deref(), Some("https://example.org"));
        // Einzelne Sterne und Unterstriche in Wörtern bleiben Text
        let plain: String = parse_inline("3 * 4 = 12, datei_name_v2").iter().map(|x| x.text.clone()).collect();
        assert_eq!(plain, "3 * 4 = 12, datei_name_v2");
        assert_eq!(parse_inline("\\*kein\\*")[0].text, "*kein*");
    }

    #[test]
    fn word_datei_ist_gueltig_und_lesbar() {
        let md = "# Bericht <Test> & Co\n\n## Einleitung\n\nText mit **fett** und [Link](https://tfbern.ch).\n\n1. eins\n2. zwei\n\n- Punkt\n\n| Spalte | Wert |\n|---|---|\n| a | 1 |\n";
        let bytes = markdown_to_docx(md, "Bericht").unwrap();
        // Dieselbe Prüfung wie vor der Umwandlung in PDF (ENTSCHEIDE E33): nichts Verbotenes, alles lesbar
        assert_eq!(crate::ooxml::check_ooxml(&bytes), "");
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("b.docx");
        std::fs::write(&p, &bytes).unwrap();
        let text = ask_terminal::documents::extract(&p.to_string_lossy()).unwrap();
        for part in ["Bericht <Test> & Co", "Einleitung", "fett", "Link", "eins", "Spalte", "Wert"] {
            assert!(text.contains(part), "{part} fehlt in {text}");
        }
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut doc = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut doc).unwrap();
        assert!(doc.contains("w:val=\"Heading2\""));
        assert!(doc.contains("<w:numId w:val=\"2\"/>"), "nummerierte Liste");
        assert!(doc.contains("<w:numId w:val=\"1\"/>"), "Aufzählung");
        let mut rels = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/_rels/document.xml.rels").unwrap(), &mut rels).unwrap();
        assert!(rels.contains("https://tfbern.ch") && rels.contains("TargetMode=\"External\""));
    }

    #[test]
    fn nur_web_links_werden_links() {
        let bytes = markdown_to_docx("[x](file:///C:/geheim) [y](javascript:alert(1))", "t").unwrap();
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let mut rels = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/_rels/document.xml.rels").unwrap(), &mut rels).unwrap();
        assert!(!rels.contains("hyperlink"));
    }
}
