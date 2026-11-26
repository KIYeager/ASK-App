//! Anhänge aus dem Chat (eingefügt, hineingezogen oder mit «+» gewählt). Sie landen versteckt unter
//! .ask/anhaenge/<Chat>/, damit das Modell sie mit seinen Werkzeugen lesen kann; sie verschwinden mit
//! dem Chat oder nach 14 Tagen (cleanup.rs). Bilder gehen zusätzlich direkt an das Modell.
//!
//! Öffentliche Schnittstelle:
//! - `save_attachments(folder, &[AttachmentFile], chat) -> Result<Vec<Attachment>>`
//! - `image_url(folder, path, Option<&ResizeFn>) -> Option<String>`
//! - `native_resize(data, ext) -> Option<Resized>` (mit dem Crate `image`, ersetzt `nativeImage`)
//! - `clean_name(name, type)`, `free_name(dir, name)`
//! - `INBOX`, `MAX_FILES`, `MAX_SIZE`, `MAX_IMAGE_SIDE`

use anyhow::{bail, Result};
use base64::Engine;
use regex::Regex;
use serde::Serialize;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::LazyLock;

pub const INBOX: &str = ".ask/anhaenge";
pub const MAX_FILES: usize = 10;
pub const MAX_SIZE: usize = 50 * 1024 * 1024;
const MAX_IMAGE_SIZE: usize = 20 * 1024 * 1024; // grössere Bilder nur als Datei
pub const MAX_IMAGE_SIDE: u32 = 2048; // grössere Bilder werden für das Modell verkleinert

fn image_type(ext: &str) -> Option<&'static str> {
    match ext {
        ".png" => Some("image/png"),
        ".jpg" | ".jpeg" => Some("image/jpeg"),
        ".gif" => Some("image/gif"),
        ".webp" => Some("image/webp"),
        _ => None,
    }
}

static SCREENSHOT_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^image\.(png|jpe?g|gif|webp|bmp)$").expect("gültiger Ausdruck"));
static DEVICE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(con|prn|aux|nul|com[0-9]|lpt[0-9])(\.|$)").expect("gültiger Ausdruck"));
static CHAT_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z0-9_-]+$").expect("gültiger Ausdruck"));

fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Endung wie `path.extname`: ab dem letzten Punkt, aber nicht bei Namen wie «.gitignore»
fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_string(),
        _ => String::new(),
    }
}

/// Dateiname ohne Pfad und ohne Zeichen, die Windows nicht erlaubt. Eingefügte Bildschirmfotos
/// heissen in Chromium «image.png»: dann ein Name mit Datum und Zeit.
pub fn clean_name(name: &str, kind: &str) -> String {
    // posix: «a:b» ist kein Laufwerk
    let unified = name.replace('\\', "/");
    let last = unified.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    let replaced: String = last
        .chars()
        .map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20 { '_' } else { c })
        .collect();
    let mut base = replaced.trim_matches(|c: char| c == '.' || c.is_whitespace() || c == '\u{feff}').to_string();
    if base.is_empty() || SCREENSHOT_NAME.is_match(&base) {
        let mut ext = ext_of(&base);
        if ext.is_empty() {
            ext = match kind {
                "image/jpeg" => ".jpg",
                "image/gif" => ".gif",
                "image/webp" => ".webp",
                _ => ".png",
            }
            .to_string();
        }
        base = format!("bild-{}{}", stamp(), ext.to_lowercase());
    }
    if base.chars().count() > 120 {
        let ext = ext_of(&base);
        let keep = 120usize.saturating_sub(ext.chars().count());
        base = format!("{}{}", crate::util::take_chars(&base, keep), ext);
    }
    // Gerätenamen von Windows (nul.txt landet sonst im Nichts)
    if DEVICE_NAME.is_match(&base) {
        base = format!("_{base}");
    }
    base
}

/// Freier Name: «bericht.docx», sonst «bericht (2).docx» usw.
pub fn free_name(dir: &Path, name: &str) -> String {
    let ext = ext_of(name);
    let stem = &name[..name.len() - ext.len()];
    let mut candidate = name.to_string();
    let mut i = 2;
    while dir.join(&candidate).exists() {
        candidate = format!("{stem} ({i}){ext}");
        i += 1;
    }
    candidate
}

fn is_image(name: &str) -> bool {
    image_type(&ext_of(name).to_lowercase()).is_some()
}

/// Eine Datei aus der Oberfläche
#[derive(Clone, Debug, Default)]
pub struct AttachmentFile {
    pub name: String,
    /// MIME-Typ, den der Browser angibt (leer, wenn unbekannt)
    pub kind: String,
    pub data: Vec<u8>,
}

/// Ein gespeicherter Anhang (`{ path, name, size, image }` für die Oberfläche)
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Attachment {
    pub path: String,
    pub name: String,
    pub size: usize,
    pub image: bool,
}

/// `.ask/anhaenge/<chat>/…` im Arbeitsordner; nichts wird überschrieben.
pub fn save_attachments(folder: &Path, files: &[AttachmentFile], chat: &str) -> Result<Vec<Attachment>> {
    if files.len() > MAX_FILES {
        bail!("Höchstens {MAX_FILES} Anhänge auf einmal.");
    }
    if !CHAT_ID.is_match(chat) {
        bail!("Ungültiger Chat");
    }
    let sub = format!("{INBOX}/{chat}");
    let mut dir = folder.to_path_buf();
    for part in sub.split('/') {
        dir.push(part);
    }
    std::fs::create_dir_all(&dir)?;
    let mut saved = Vec::new();
    for f in files {
        if f.data.len() > MAX_SIZE {
            let who = if f.name.is_empty() { "Ein Anhang" } else { f.name.as_str() };
            bail!("{who} ist grösser als 50 MB.");
        }
        let name = free_name(&dir, &clean_name(&f.name, &f.kind));
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(dir.join(&name))?;
        file.write_all(&f.data)?;
        saved.push(Attachment { path: format!("{sub}/{name}"), name: name.clone(), size: f.data.len(), image: is_image(&name) });
    }
    Ok(saved)
}

/// Verkleinertes Bild: neue Daten und ihr Typ
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resized {
    pub data: Vec<u8>,
    pub kind: String,
}

/// `resize(daten, endung)` darf ein kleineres Bild liefern (`native_resize`), sonst geht das Bild unverändert.
pub type ResizeFn<'a> = dyn Fn(&[u8], &str) -> Option<Resized> + 'a;

/// Bild als data-URL für das Modell. `path` ist der Pfad des Anhangs im Arbeitsordner
/// (`.ask/anhaenge/<Chat>/name.png`), `None` bei fehlender Datei, Nicht-Bild oder zu grossem Bild.
pub fn image_url(folder: &Path, path: &str, resize: Option<&ResizeFn>) -> Option<String> {
    // Der Pfad kommt aus der Chat-Datei, die das Modell ändern kann: nur Pfade im Ordner
    let full = crate::safe::safe_join(folder, path).ok()?;
    let ext = ext_of(&full.file_name()?.to_string_lossy()).to_lowercase();
    let mut kind = image_type(&ext)?.to_string();
    let mut data = std::fs::read(&full).ok()?; // Datei inzwischen gelöscht oder unlesbar: kein Bild
    if let Some(smaller) = resize.and_then(|r| r(&data, &ext)) {
        data = smaller.data;
        kind = smaller.kind;
    }
    if data.len() > MAX_IMAGE_SIZE {
        return None;
    }
    Some(format!("data:{kind};base64,{}", base64::engine::general_purpose::STANDARD.encode(&data)))
}

/// Verkleinern mit dem Crate `image` (PNG und JPEG); `None`, wenn nicht nötig oder nicht möglich.
pub fn native_resize(data: &[u8], ext: &str) -> Option<Resized> {
    use image::codecs::jpeg::JpegEncoder;
    use image::imageops::FilterType;
    use image::{ExtendedColorType, ImageEncoder, ImageFormat};

    if ![".png", ".jpg", ".jpeg"].contains(&ext) {
        return None;
    }
    let img = image::load_from_memory(data).ok()?; // leer oder nicht lesbar
    let (width, height) = (img.width(), img.height());
    if width == 0 || height == 0 || width.max(height) <= MAX_IMAGE_SIDE {
        return None;
    }
    let side = f64::from(MAX_IMAGE_SIDE);
    let (new_w, new_h) = if width >= height {
        (MAX_IMAGE_SIDE, ((f64::from(height) * side / f64::from(width)).round() as u32).max(1))
    } else {
        (((f64::from(width) * side / f64::from(height)).round() as u32).max(1), MAX_IMAGE_SIDE)
    };
    let scaled = img.resize_exact(new_w, new_h, FilterType::Lanczos3);
    let mut out = Cursor::new(Vec::new());
    if ext == ".png" {
        scaled.write_to(&mut out, ImageFormat::Png).ok()?;
        Some(Resized { data: out.into_inner(), kind: "image/png".to_string() })
    } else {
        let rgb = scaled.to_rgb8();
        JpegEncoder::new_with_quality(&mut out, 85).write_image(rgb.as_raw(), rgb.width(), rgb.height(), ExtendedColorType::Rgb8).ok()?;
        Some(Resized { data: out.into_inner(), kind: "image/jpeg".to_string() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(text: &str) -> Vec<u8> {
        text.as_bytes().to_vec()
    }

    fn file(name: &str, data: &str) -> AttachmentFile {
        AttachmentFile { name: name.to_string(), kind: String::new(), data: bytes(data) }
    }

    fn matches(pattern: &str, text: &str) -> bool {
        Regex::new(pattern).unwrap().is_match(text)
    }

    #[test]
    fn namen_ohne_pfad_ohne_verbotene_zeichen_bildschirmfoto_mit_datum() {
        assert_eq!(clean_name("..\\..\\Windows\\win.ini", ""), "win.ini");
        assert_eq!(clean_name("a:b*c?.txt", ""), "a_b_c_.txt");
        assert!(matches(r"^bild-\d{8}-\d{6}\.png$", &clean_name("image.png", "image/png")));
        assert!(matches(r"^bild-\d{8}-\d{6}\.jpg$", &clean_name("", "image/jpeg")));
    }

    #[test]
    fn geraetenamen_von_windows_bei_anhaengen() {
        assert_eq!(clean_name("nul.txt", ""), "_nul.txt");
        assert_eq!(clean_name("CON", ""), "_CON");
        assert_eq!(clean_name("console.txt", ""), "console.txt");
    }

    #[test]
    fn lange_namen_werden_gekuerzt_die_endung_bleibt() {
        let name = format!("{}.docx", "a".repeat(200));
        let clean = clean_name(&name, "");
        assert_eq!(clean.chars().count(), 120);
        assert!(clean.ends_with(".docx"));
    }

    #[test]
    fn anhaenge_landen_im_chat_ordner_nichts_wird_ueberschrieben() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let first = save_attachments(folder, &[file("bericht.docx", "eins"), file("foto.PNG", "png")], "c1").unwrap();
        let got: Vec<(String, bool)> = first.iter().map(|a| (a.path.clone(), a.image)).collect();
        assert_eq!(got, [(".ask/anhaenge/c1/bericht.docx".to_string(), false), (".ask/anhaenge/c1/foto.PNG".to_string(), true)]);
        let second = save_attachments(folder, &[file("bericht.docx", "zwei")], "c1").unwrap();
        assert_eq!(second[0].path, ".ask/anhaenge/c1/bericht (2).docx");
        assert_eq!(std::fs::read_to_string(folder.join(".ask/anhaenge/c1/bericht.docx")).unwrap(), "eins");
        assert_eq!(save_attachments(folder, &[file("bericht.docx", "drei")], "c2").unwrap()[0].path, ".ask/anhaenge/c2/bericht.docx");
        let many: Vec<AttachmentFile> = (0..11).map(|i| file(&format!("{i}.txt"), "x")).collect();
        assert!(save_attachments(folder, &many, "c1").unwrap_err().to_string().contains("Höchstens 10"));
        assert!(save_attachments(folder, &[], "../x").unwrap_err().to_string().contains("Ungültiger Chat"));
    }

    #[test]
    fn bild_als_data_url_verkleinert_wenn_moeglich_fehlende_datei_ohne_fehler() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path();
        let saved = save_attachments(folder, &[file("a.png", "PNGDATA"), file("b.txt", "t")], "c1").unwrap();
        let (img, doc) = (&saved[0], &saved[1]);
        let b64 = |s: &str| base64::engine::general_purpose::STANDARD.encode(s);
        assert_eq!(image_url(folder, &img.path, None), Some(format!("data:image/png;base64,{}", b64("PNGDATA"))));
        let klein = |_: &[u8], _: &str| Some(Resized { data: bytes("klein"), kind: "image/jpeg".to_string() });
        assert_eq!(image_url(folder, &img.path, Some(&klein)), Some(format!("data:image/jpeg;base64,{}", b64("klein"))));
        assert_eq!(image_url(folder, &doc.path, None), None);
        assert_eq!(image_url(folder, ".ask/anhaenge/c1/weg.png", None), None);
        assert_eq!(image_url(folder, "../ausserhalb.png", None), None);
    }

    #[test]
    fn grosse_bilder_werden_verkleinert() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(3000, 100)).write_to(&mut png, image::ImageFormat::Png).unwrap();
        let out = native_resize(png.get_ref(), ".png").expect("verkleinert");
        assert_eq!(out.kind, "image/png");
        let small = image::load_from_memory(&out.data).unwrap();
        assert_eq!((small.width(), small.height()), (2048, 68));
        let jpg = native_resize(png.get_ref(), ".jpg").expect("verkleinert");
        assert_eq!(jpg.kind, "image/jpeg");
        // kleine Bilder, andere Formate und kaputte Daten bleiben unberührt
        let mut little = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::new(100, 100)).write_to(&mut little, image::ImageFormat::Png).unwrap();
        assert!(native_resize(little.get_ref(), ".png").is_none());
        assert!(native_resize(png.get_ref(), ".gif").is_none());
        assert!(native_resize(b"kein bild", ".png").is_none());
    }
}
