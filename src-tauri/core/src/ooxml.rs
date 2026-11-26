//! Prüfung von Word-, Excel- und PowerPoint-Dateien, bevor Office sie für die PDF-Umwandlung öffnet
//! (office.rs). Office läuft dabei mit den normalen Rechten der Person und holt verknüpfte Inhalte
//! ohne Rückfrage, auch von fremden Rechnern (dabei gehen Anmeldedaten mit). Abgelehnt werden darum
//! Makros, ActiveX, eingebettete Objekte, externe Verweise ausser gewöhnlichen Links, verknüpfte
//! Felder und eingebettete Fremddokumente. Die Dateien sind ZIP-Archive mit XML; alles, was sich
//! nicht sicher lesen lässt, gilt als abgelehnt.
//!
//! Öffentliche Schnittstelle:
//! - `check_ooxml(&[u8]) -> String`: Fehlertext, wenn Office die Datei nicht öffnen soll; sonst leer

use regex::{Captures, Regex};
use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::sync::LazyLock;
use zip::{CompressionMethod, ZipArchive};

const MAX_ENTRIES: usize = 5000;
const MAX_PART: u64 = 64 * 1024 * 1024; // entpackte Grösse eines geprüften Teils

/// Fehler der Prüfung: `Rejected` mit der Meldung für die Person, alles andere gilt als «lässt sich nicht prüfen».
enum Failure {
    Rejected(String),
    Other,
}

type Check<T> = Result<T, Failure>;

fn rejected<T>(message: &str) -> Check<T> {
    Err(Failure::Rejected(message.to_string()))
}

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("gültiger regulärer Ausdruck")
}

static ENTRY_DANGEROUS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)vbaProject|activeX|oleObject|embeddings/.*\.(bin|exe|dll|js|vbs|ps1|bat|cmd)$"));
static ENTRY_EXTERNAL_LINKS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^xl/externalLinks/"));
static ENTRY_RELS: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)\.rels$"));
static ENTRY_WORD_XML: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^word/.*\.xml$"));
static HEX_ENTITY: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)&#x([0-9a-f]+);"));
static DEC_ENTITY: LazyLock<Regex> = LazyLock::new(|| re(r"&#([0-9]+);"));
static ATTRIBUTE: LazyLock<Regex> = LazyLock::new(|| re(r#"([A-Za-z0-9_:.-]+)\s*=\s*(?:"([^"]*)"|'([^']*)')"#));
// Diesen Verweis entfernt office.rs aus der Kopie, bevor Word sie öffnet; er muss genau die Form haben, die dort erkannt wird
static TEMPLATE_REL: LazyLock<Regex> = LazyLock::new(|| re(r"^<Relationship [^>]*attachedTemplate[^>]*/>$"));
static RELATIONSHIP: LazyLock<Regex> = LazyLock::new(|| re(r"<(?:[A-Za-z0-9_.-]+:)?Relationship(?-u:\b)[^>]*>"));
static ALT_CHUNK: LazyLock<Regex> = LazyLock::new(|| re(r"<w:altChunk(?-u:\b)"));
static FIELD_TAGS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"<w:fldSimple(?-u:\b)[^>]*>|<w:fldChar(?-u:\b)[^>]*>|<w:(?:instrText|delInstrText)(?-u:\b)[^>]*>([^<]*)")
});
static FIELD_NAME: LazyLock<Regex> = LazyLock::new(|| re(r"^[\s\\]*([A-Za-z]+)"));

const FIELDS: [&str; 8] = ["DDE", "DDEAUTO", "INCLUDE", "INCLUDETEXT", "INCLUDEPICTURE", "IMPORT", "LINK", "RD"];

/// Entpackter Teil: ungepackt oder mit Deflate. Begrenzt auf `MAX_PART`.
fn read_entry(archive: &mut ZipArchive<Cursor<&[u8]>>, index: usize) -> Check<Vec<u8>> {
    let method = archive.by_index_raw(index).map_err(|_| Failure::Rejected("Die Datei ist beschädigt.".to_string()))?.compression();
    if !matches!(method, CompressionMethod::Stored | CompressionMethod::Deflated) {
        return rejected("Die Datei ist auf eine unbekannte Art gepackt.");
    }
    let file = archive.by_index(index).map_err(|_| Failure::Rejected("Ein Teil der Datei lässt sich nicht entpacken.".to_string()))?;
    let mut data = Vec::new();
    let mut limited = file.take(MAX_PART + 1);
    limited.read_to_end(&mut data).map_err(|_| Failure::Rejected("Ein Teil der Datei lässt sich nicht entpacken.".to_string()))?;
    if data.len() as u64 > MAX_PART {
        return rejected("Ein Teil der Datei lässt sich nicht entpacken.");
    }
    Ok(data)
}

/// XML als Text; UTF-16 erkennt man an der Marke am Anfang (sonst sähe die Prüfung nur Bytes)
fn xml_text(data: &[u8]) -> String {
    if data.len() >= 2 && data[0] == 0xff && data[1] == 0xfe {
        let units: Vec<u16> = data[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if data.len() >= 2 && data[0] == 0xfe && data[1] == 0xff {
        let units: Vec<u16> = data[2..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    String::from_utf8_lossy(data).to_string()
}

/// `String.fromCodePoint(parseInt(digits, radix) || 32)`: zu grosse Werte sind ein Fehler wie in JavaScript
fn code_point(digits: &str, radix: u32) -> Check<String> {
    let n = u64::from_str_radix(digits, radix).map_err(|_| Failure::Other)?;
    let n = if n == 0 { 32 } else { n };
    if n > 0x10FFFF {
        return Err(Failure::Other);
    }
    // einzelne Surrogate gibt es in Rust-Text nicht
    Ok(char::from_u32(n as u32).unwrap_or('\u{fffd}').to_string())
}

fn replace_entities(re: &Regex, text: &str, radix: u32) -> Check<String> {
    let mut error = false;
    let out = re
        .replace_all(text, |caps: &Captures| match code_point(&caps[1], radix) {
            Ok(s) => s,
            Err(_) => {
                error = true;
                String::new()
            }
        })
        .to_string();
    if error { Err(Failure::Other) } else { Ok(out) }
}

fn unescape_xml(s: &str) -> Check<String> {
    let s = replace_entities(&HEX_ENTITY, s, 16)?;
    let s = replace_entities(&DEC_ENTITY, &s, 10)?;
    Ok(s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&"))
}

/// Attribute eines Tags, mit beiden Arten von Anführungszeichen und beliebigen Abständen
fn attributes(tag: &str) -> Check<HashMap<String, String>> {
    let mut out = HashMap::new();
    for m in ATTRIBUTE.captures_iter(tag) {
        let value = m.get(2).or_else(|| m.get(3)).map_or("", |v| v.as_str());
        out.insert(m[1].to_string(), unescape_xml(value)?);
    }
    Ok(out)
}

fn check_rels(name: &str, xml: &str) -> Check<()> {
    for m in RELATIONSHIP.find_iter(xml) {
        let a = attributes(m.as_str())?;
        let kind = a.get("Type").map_or("", String::as_str);
        if kind.to_lowercase().ends_with("/afchunk") {
            return rejected("Die Datei enthält ein eingebettetes Fremddokument.");
        }
        if a.get("TargetMode").map_or("", String::as_str).trim().to_lowercase() != "external" {
            continue;
        }
        if kind.ends_with("/hyperlink") {
            continue; // gewöhnlicher Link, wird erst beim Anklicken geöffnet
        }
        if name == "word/_rels/settings.xml.rels" && kind.ends_with("/attachedTemplate") && TEMPLATE_REL.is_match(m.as_str()) {
            continue;
        }
        return rejected("Die Datei verweist auf externe Inhalte.");
    }
    Ok(())
}

fn check_instruction(instruction: &str) -> Check<()> {
    let name = FIELD_NAME.captures(instruction).map(|c| c[1].to_uppercase());
    if name.is_some_and(|n| FIELDS.contains(&n.as_str())) {
        return rejected("Die Datei enthält verknüpfte Felder.");
    }
    Ok(())
}

/// Felder in Word: <w:fldSimple w:instr="…"> oder fldChar begin … instrText … fldChar separate/end.
/// Der Feldname ist das erste Wort der Anweisung, auch wenn Word sie auf mehrere Stücke verteilt.
fn check_fields(xml: &str) -> Check<()> {
    if ALT_CHUNK.is_match(xml) {
        return rejected("Die Datei enthält ein eingebettetes Fremddokument.");
    }
    let mut open: Vec<String> = Vec::new(); // Anweisungen der offenen Felder (Felder können verschachtelt sein)
    let mut loose = String::new(); // Anweisung ohne Feldanfang
    for m in FIELD_TAGS.captures_iter(xml) {
        let whole = m.get(0).map_or("", |g| g.as_str());
        if whole.starts_with("<w:fldSimple") {
            check_instruction(attributes(whole)?.get("w:instr").map_or("", String::as_str))?;
        } else if whole.starts_with("<w:fldChar") {
            let attrs = attributes(whole)?;
            let kind = attrs.get("w:fldCharType").map(String::as_str);
            if kind == Some("begin") {
                open.push(String::new());
            } else if let Some(last) = open.last() {
                check_instruction(last)?;
            }
            if kind == Some("end") {
                open.pop();
            }
        } else {
            let text = unescape_xml(m.get(1).map_or("", |g| g.as_str()))?;
            match open.last_mut() {
                Some(last) => last.push_str(&text),
                None => loose.push_str(&text),
            }
        }
    }
    for instruction in &open {
        check_instruction(instruction)?;
    }
    check_instruction(&loose)
}

fn check(buf: &[u8]) -> Check<()> {
    let mut archive = ZipArchive::new(Cursor::new(buf)).map_err(|_| Failure::Rejected("Die Datei ist kein gültiges Office-Dokument.".to_string()))?;
    if archive.len() > MAX_ENTRIES {
        return rejected("Die Datei ist zu gross für die Prüfung.");
    }
    // Erst das ganze Inhaltsverzeichnis, dann die Teile (wie im Original)
    let mut names = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        let file = archive.by_index_raw(i).map_err(|_| Failure::Rejected("Die Datei ist beschädigt.".to_string()))?;
        if file.encrypted() {
            return rejected("Die Datei ist verschlüsselt.");
        }
        let limit = u64::from(u32::MAX);
        if file.compressed_size() >= limit || file.size() >= limit || file.header_start() >= limit {
            return rejected("Die Datei ist zu gross für die Prüfung.");
        }
        names.push(String::from_utf8_lossy(file.name_raw()).replace('\\', "/"));
    }
    for (i, name) in names.iter().enumerate() {
        if ENTRY_DANGEROUS.is_match(name) {
            return Err(Failure::Rejected(format!("Die Datei enthält {name}.")));
        }
        if ENTRY_EXTERNAL_LINKS.is_match(name) {
            return rejected("Die Datei enthält verknüpfte Felder.");
        }
        if ENTRY_RELS.is_match(name) {
            check_rels(name, &xml_text(&read_entry(&mut archive, i)?))?;
        } else if ENTRY_WORD_XML.is_match(name) {
            check_fields(&xml_text(&read_entry(&mut archive, i)?))?;
        }
    }
    Ok(())
}

/// Fehlertext, wenn Office die Datei nicht öffnen soll; sonst leer.
pub fn check_ooxml(buf: &[u8]) -> String {
    match check(buf) {
        Ok(()) => String::new(),
        Err(Failure::Rejected(message)) => message,
        Err(Failure::Other) => "Die Datei lässt sich nicht prüfen.".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    // Kleines ZIP-Archiv aus (Name, Inhalt); store: ungepackt ablegen
    fn zip_of(files: &[(&str, Vec<u8>)], store: bool) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let method = if store { CompressionMethod::Stored } else { CompressionMethod::Deflated };
        for (name, content) in files {
            w.start_file(*name, SimpleFileOptions::default().compression_method(method)).unwrap();
            w.write_all(content).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    fn zip(files: &[(&str, &str)]) -> Vec<u8> {
        zip_of(&files.iter().map(|(n, c)| (*n, c.as_bytes().to_vec())).collect::<Vec<_>>(), false)
    }

    const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

    fn rels(items: &[String]) -> String {
        format!(r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{}</Relationships>"#, items.join(""))
    }

    fn doc(body: &str) -> String {
        format!(r#"<?xml version="1.0"?><w:document xmlns:w="x"><w:body>{body}</w:body></w:document>"#)
    }

    fn field(parts: &[&str]) -> String {
        let instr: String = parts.iter().map(|p| format!(r#"<w:r><w:instrText xml:space="preserve">{p}</w:instrText></w:r>"#)).collect();
        format!(
            r#"<w:p><w:r><w:fldChar w:fldCharType="begin"/></w:r>{instr}<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>x</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r></w:p>"#
        )
    }

    fn docx(document_xml: &str, rels_xml: &str, extra: &[(&str, &str)]) -> Vec<u8> {
        let mut files = vec![("[Content_Types].xml", "<Types/>"), ("word/document.xml", document_xml), ("word/_rels/document.xml.rels", rels_xml)];
        files.extend_from_slice(extra);
        zip(&files)
    }

    #[test]
    fn gewoehnliche_dokumente_gehen_durch() {
        assert_eq!(check_ooxml(&docx(&doc("<w:p><w:r><w:t>Hallo</w:t></w:r></w:p>"), &rels(&[]), &[])), "");
        assert_eq!(check_ooxml(&zip_of(&[("word/document.xml", doc("<w:p/>").into_bytes())], true)), "");
        // Links, Inhaltsverzeichnis, Seitenzahl und ein Link, dessen Adresse «link» oder «dde» enthält
        let ok = doc(&format!(
            "{}{}{}{}{}",
            field(&[r#" TOC \o "1-3" "#]),
            field(&[" PAGE "]),
            field(&[r#" HYPERLINK "https://x.ch/link/dde" "#]),
            field(&[" HYPER", r#"LINK "https://x.ch" "#]),
            r#"<w:fldSimple w:instr=" DATE \@ &quot;dd.MM.yyyy&quot; "/>"#
        ));
        let link = format!(r#"<Relationship Id="r1" Type="{REL}/hyperlink" Target="https://x.ch" TargetMode="External"/>"#);
        assert_eq!(check_ooxml(&docx(&ok, &rels(&[link]), &[])), "");
        // Verweis auf eine Vorlage in der Form, die office.rs aus der Kopie entfernt
        let template = rels(&[format!(r#"<Relationship Id="r1" Type="{REL}/attachedTemplate" Target="file:///C:/v.dotx" TargetMode="External"/>"#)]);
        assert_eq!(check_ooxml(&docx(&doc("<w:p/>"), &rels(&[]), &[("word/_rels/settings.xml.rels", &template)])), "");
    }

    fn has(result: &str, parts: &[&str]) -> bool {
        parts.iter().any(|p| result.contains(p))
    }

    #[test]
    fn externe_verweise_werden_abgelehnt_auch_in_ungewohnter_schreibweise() {
        let image = format!("{REL}/image");
        let cases = [
            format!(r#"<Relationship Id="r1" Type="{image}" Target="\\boese\freigabe\x.png" TargetMode="External"/>"#),
            format!(r#"<Relationship Id="r1" Type="{image}" Target="https://boese.example/hyperlink" TargetMode="External"/>"#), // Ziel endet auf /hyperlink
            format!(r#"<Relationship Id='r1' Type='{image}' Target='\\boese\x.png' TargetMode='External'/>"#), // einfache Anführungszeichen
            format!("<Relationship\n\tId=\"r1\" Type=\"{image}\" Target=\"\\\\boese\\x.png\" TargetMode=\"External\"/>"), // Zeilenumbruch nach dem Namen
            format!(r#"<Relationship Id="r1" Type="{image}" Target="\\boese\x.png" TargetMode = "External" />"#), // Abstände um =
            format!(r#"<Relationship Id="r1" Type="{image}" Target="\\boese\x.png" TargetMode="external"/>"#),
            format!(r#"<Relationship Id="r1" Type="{REL}/hyperlink/x" Target="\\boese\x" TargetMode="External"/>"#),
            format!(r#"<pr:Relationship xmlns:pr="p" Id="r1" Type="{image}" Target="\\boese\x.png" TargetMode="External"/>"#),
            format!(r#"<Relationship Id="r1" Type="{REL}/aFChunk" Target="fremd.html"/>"#),
        ];
        for rel in cases {
            let r = check_ooxml(&docx(&doc("<w:p/>"), &rels(std::slice::from_ref(&rel)), &[]));
            assert!(has(&r, &["externe Inhalte", "Fremddokument"]), "{rel}: {r:?}");
        }
        // Vorlage in einer Form, die das Entfernen in office.rs nicht erkennt
        let template = rels(&[format!(r#"<Relationship Id='r1' Type='{REL}/attachedTemplate' Target='\\boese\v.dotx' TargetMode='External'></Relationship>"#)]);
        let r = check_ooxml(&docx(&doc("<w:p/>"), &rels(&[]), &[("word/_rels/settings.xml.rels", &template)]));
        assert!(r.contains("externe Inhalte"), "{r:?}");
        // als UTF-16 gespeichert
        let text = rels(&[format!(r#"<Relationship Id="r1" Type="{image}" Target="\\boese\x.png" TargetMode="External"/>"#)]);
        let mut utf16 = vec![0xff, 0xfe];
        for u in text.encode_utf16() {
            utf16.extend_from_slice(&u.to_le_bytes());
        }
        let files = [("[Content_Types].xml", b"<Types/>".to_vec()), ("word/document.xml", doc("<w:p/>").into_bytes()), ("word/_rels/document.xml.rels", utf16)];
        let r = check_ooxml(&zip_of(&files, false));
        assert!(r.contains("externe Inhalte"), "{r:?}");
    }

    #[test]
    fn verknuepfte_felder_makros_und_fremddokumente_werden_abgelehnt() {
        let bodies = [
            field(&[r#" INCLUDEPICTURE "\\\\boese\\x.png" \d "#]),
            field(&[r#" includepicture "\\\\boese\\x.png" "#]), // klein geschrieben
            field(&[" INCLUDE", r#"PICTURE "\\\\boese\\x.png" "#]), // auf zwei Stücke verteilt
            field(&[r#" DDEAUTO c:\\windows\\system32\\cmd.exe "/k calc" "#]),
            field(&[r#" LINK Excel.Sheet.12 "\\\\boese\\a.xlsx" "#]),
            field(&[" IF 1 = 1 \"", &field(&[r#" INCLUDETEXT "\\\\boese\\a.docx" "#]), "\" "]), // verschachtelt
            r#"<w:fldSimple w:instr=' INCLUDETEXT "x" '/>"#.to_string(),
            r#"<w:fldSimple w:instr=" RD &quot;\\boese\a.docx&quot; "/>"#.to_string(),
            r#"<w:p><w:r><w:instrText> INCLUDEPICTURE "x" </w:instrText></w:r></w:p>"#.to_string(), // ohne Feldanfang
            r#"<w:altChunk r:id="r1"/>"#.to_string(),
        ];
        for body in bodies {
            let r = check_ooxml(&docx(&doc(&body), &rels(&[]), &[]));
            assert!(has(&r, &["verknüpfte Felder", "Fremddokument"]), "{body}: {r:?}");
        }
        // auch in Fussnoten, nicht nur im Haupttext
        let footnotes = doc(&field(&[r#" INCLUDEPICTURE "x" "#]));
        assert!(check_ooxml(&docx(&doc("<w:p/>"), &rels(&[]), &[("word/footnotes.xml", &footnotes)])).contains("verknüpfte Felder"));
        assert!(check_ooxml(&docx(&doc("<w:p/>"), &rels(&[]), &[("word/vbaProject.bin", "x")])).contains("vbaProject"));
        assert!(check_ooxml(&zip(&[("xl/workbook.xml", "<x/>"), ("xl/externalLinks/externalLink1.xml", "<x/>")])).contains("verknüpfte Felder"));
    }

    #[test]
    fn was_sich_nicht_lesen_laesst_wird_abgelehnt() {
        assert_ne!(check_ooxml(b"kein zip"), "");
        assert_ne!(check_ooxml(&[]), "");
        let mut broken = docx(&doc("<w:p/>"), &rels(&[]), &[]);
        for b in &mut broken[40..60] {
            *b = 0xff; // gepackte Daten zerstört
        }
        assert_ne!(check_ooxml(&broken), "");
    }

    #[test]
    fn zahlen_in_xml_werden_wie_in_javascript_aufgeloest() {
        assert_eq!(unescape_xml("a&#x41;&#66;&lt;&amp;lt;").ok().as_deref(), Some("aAB<&lt;"));
        assert_eq!(unescape_xml("&#0;").ok().as_deref(), Some(" "));
        assert!(unescape_xml("&#x110000;").is_err());
    }
}
