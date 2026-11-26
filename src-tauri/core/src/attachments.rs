// Noch nicht portiert (src/main/attachments.js).

// TEMP-STUB (wird beim Zusammenführen ersetzt)
// Nur damit workspace.rs übersetzt und seine Tests laufen; die echte Fassung kommt mit dem Port von
// attachments.js (`clean_name` und `free_name` sind hier schon wörtlich übernommen).
/// Dateiname ohne Pfad und ohne Zeichen, die Windows nicht erlaubt (`cleanName(name, type)`).
pub fn clean_name(name: &str, kind: &str) -> String {
    let unified = name.replace('\\', "/");
    let mut base = unified.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
    base = base.chars().map(|c| if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || (c as u32) < 0x20 { '_' } else { c }).collect();
    base = base.trim_matches(|c: char| c == '.' || c.is_whitespace()).to_string();
    let ext_of = |s: &str| match s.rfind('.') {
        Some(i) if i > 0 => s[i..].to_string(),
        _ => String::new(),
    };
    let lower = base.to_lowercase();
    let is_image = ["png", "jpg", "jpeg", "gif", "webp", "bmp"].iter().any(|e| lower == format!("image.{e}"));
    if base.is_empty() || is_image {
        let ext = Some(ext_of(&base)).filter(|e| !e.is_empty()).unwrap_or_else(|| {
            match kind {
                "image/jpeg" => ".jpg",
                "image/gif" => ".gif",
                "image/webp" => ".webp",
                _ => ".png",
            }
            .to_string()
        });
        base = format!("bild-{}{}", chrono::Local::now().format("%Y%m%d-%H%M%S"), ext.to_lowercase());
    }
    if base.chars().count() > 120 {
        let ext = ext_of(&base);
        let keep = 120 - ext.chars().count();
        base = base.chars().take(keep).collect::<String>() + &ext;
    }
    // Gerätenamen von Windows (nul.txt landet sonst im Nichts)
    let lower = base.to_lowercase();
    let stem = lower.split('.').next().unwrap_or("");
    let device = ["con", "prn", "aux", "nul"].contains(&stem)
        || ((stem.starts_with("com") || stem.starts_with("lpt")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if device {
        base = format!("_{base}");
    }
    base
}

/// Freier Name: «bericht.docx», sonst «bericht (2).docx» usw.
pub fn free_name(dir: &std::path::Path, name: &str) -> String {
    let ext = match name.rfind('.') {
        Some(i) if i > 0 => &name[i..],
        _ => "",
    };
    let stem = &name[..name.len() - ext.len()];
    let mut candidate = name.to_string();
    let mut i = 2;
    while dir.join(&candidate).exists() {
        candidate = format!("{stem} ({i}){ext}");
        i += 1;
    }
    candidate
}
// TEMP-STUB Ende
