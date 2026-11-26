//! Eigene Skills (ENTSCHEIDE E51): Anleitungen der Person für wiederkehrende Aufträge, wie Skills in
//! Claude. Jeder Skill ist eine Datei skills/<name>/SKILL.md in den Daten der App (userData), mit
//! Kopf («name», «description») und der Anleitung darunter. Teilen geht über Export und Import der Datei.
//!
//! Öffentliche Schnittstelle (Port von src/main/skills.js):
//! - `Skills::new(dir)`: `list`, `get`, `save(SkillInput, SaveOptions)`, `remove`, `export_text`,
//!   `Skills::read_import(bytes)`, `prompt`, `prompt_with(max)`
//! - freie Funktionen `slugify`, `check_name`, `parse`, `format`, `invocation`

use crate::memory::{collapse_ws, is_js_ws};
use crate::util::{char_len, take_chars};
use anyhow::{bail, Result};
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

pub const MAX_FILE: usize = 100 * 1024; // Import und Speichern: höchstens 100 KB
pub const MAX_DESCRIPTION: usize = 300;
pub const MAX_NAME: usize = 40;
pub const PROMPT_MAX: usize = 1800; // Liste im Systemprompt
pub const PROMPT_DESCRIPTION: usize = 160;
pub const MAX_SKILLS: usize = 100;
/// Befehle der App: ein Skill mit diesem Namen liesse sich mit /name nicht aufrufen
pub const RESERVED: &[&str] = &["neu", "projekt", "goal", "suchen", "ordner", "explorer", "dateien", "github", "modell", "denken", "web", "chats",
    "einstellungen", "update", "beta", "neustart", "abmelden", "gedaechtnis", "skills", "hilfe"];

fn is_reserved(slug: &str) -> bool {
    RESERVED.contains(&slug)
}

/// Name → Ordnername: klein, Umlaute ausgeschrieben, nur a-z, 0-9 und Bindestriche
pub fn slugify(name: &str) -> String {
    let lower = name.to_lowercase().replace('ä', "ae").replace('ö', "oe").replace('ü', "ue").replace('ß', "ss");
    let folded: String = lower.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    let mut out = String::new();
    for c in folded.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    take_chars(trimmed, MAX_NAME).trim_end_matches('-').to_string()
}

/// Fehlertext zu einem Namen, leer wenn er in Ordnung ist
pub fn check_name(slug: &str) -> String {
    if slug.is_empty() {
        return "Der Name fehlt (Buchstaben oder Ziffern).".to_string();
    }
    if is_reserved(slug) {
        return format!("«/{slug}» ist schon ein Befehl der App. Wähle einen anderen Namen.");
    }
    String::new()
}

fn one_line(t: &str, max: usize) -> String {
    take_chars(&collapse_ws(t), max)
}

/// Gelesener Skill (Kopf und Anleitung)
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub name: String,
    pub description: String,
    pub body: String,
}

fn unquote(v: &str) -> String {
    let chars: Vec<char> = v.chars().collect();
    let inner: String = if chars.len() >= 2 && (chars[0] == '\'' || chars[0] == '"') && chars[chars.len() - 1] == chars[0] {
        chars[1..chars.len() - 1].iter().collect()
    } else {
        v.to_string()
    };
    // Faltzeichen von YAML am Anfang («>-», «|»)
    let rest = match inner.strip_prefix(['>', '|']) {
        Some(r) => {
            let r = r.strip_prefix(['-', '+']).unwrap_or(r);
            r.trim_start_matches(is_js_ws).to_string()
        }
        None => inner,
    };
    rest.trim_matches(is_js_ws).to_string()
}

/// Kopf wie bei Claude-Skills: --- name: … description: … --- und danach die Anleitung
pub fn parse(text: &str) -> Result<Parsed> {
    static FRAME: OnceLock<Regex> = OnceLock::new();
    static KV: OnceLock<Regex> = OnceLock::new();
    static CONT: OnceLock<Regex> = OnceLock::new();
    let frame = FRAME.get_or_init(|| Regex::new(r"(?s)^---\n(.*?)\n---[ \t]*(?:\n|$)(.*)$").expect("gültiges Muster"));
    let kv = KV.get_or_init(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_-]*)\s*:\s*(.*)$").expect("gültiges Muster"));
    let cont = CONT.get_or_init(|| Regex::new(r"^\s+\S").expect("gültiges Muster"));
    let raw = text.strip_prefix('\u{FEFF}').unwrap_or(text).replace("\r\n", "\n").replace('\r', "\n");
    let Some(m) = frame.captures(&raw) else {
        bail!("Die Datei hat keinen Kopf mit «name» und «description» zwischen zwei Zeilen «---».");
    };
    let mut head: HashMap<String, String> = HashMap::new();
    let mut key = String::new();
    for line in m[1].split('\n') {
        if let Some(c) = kv.captures(line) {
            key = c[1].to_lowercase();
            head.insert(key.clone(), c[2].trim_matches(is_js_ws).to_string());
        } else if !key.is_empty() && cont.is_match(line) {
            let joined = format!("{} {}", head.get(&key).cloned().unwrap_or_default(), line.trim_matches(is_js_ws));
            head.insert(key.clone(), joined.trim_matches(is_js_ws).to_string()); // eingerückte Fortsetzung
        }
    }
    let name = unquote(head.get("name").map(String::as_str).unwrap_or(""));
    let description = one_line(&unquote(head.get("description").map(String::as_str).unwrap_or("")), MAX_DESCRIPTION);
    let body = m[2].trim_start_matches('\n').trim_end_matches(is_js_ws).to_string();
    if name.is_empty() {
        bail!("Im Kopf fehlt «name».");
    }
    if description.is_empty() {
        bail!("Im Kopf fehlt «description».");
    }
    if body.trim_matches(is_js_ws).is_empty() {
        bail!("Die Anleitung unter dem Kopf ist leer.");
    }
    Ok(Parsed { name, description, body })
}

/// Text der Datei: Kopf, Leerzeile, Anleitung
pub fn format(name: &str, description: &str, body: &str) -> String {
    let desc = one_line(description, MAX_DESCRIPTION);
    let body = body.replace("\r\n", "\n").replace('\r', "\n");
    format!("---\nname: {name}\ndescription: {desc}\n---\n\n{}\n", body.trim_matches(is_js_ws))
}

/// Ein Skill mit Anleitung
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Skill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub body: String,
    /// Änderungszeit in Millisekunden (wie `mtimeMs`)
    pub updated: f64,
    pub size: u64,
}

/// Eintrag der Liste (ohne Anleitung)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SkillInfo {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub updated: f64,
    pub size: u64,
}

/// Angaben zum Speichern
#[derive(Clone, Debug, Default)]
pub struct SkillInput {
    pub name: String,
    pub description: String,
    pub body: String,
}

#[derive(Clone, Debug, Default)]
pub struct SaveOptions {
    /// bisheriger Name, wenn der Skill umbenannt wird
    pub previous: String,
    /// einen anderen Skill mit demselben Namen ersetzen
    pub overwrite: bool,
}

pub struct Skills {
    dir: PathBuf,
}

impl Skills {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Skills { dir: dir.into() }
    }

    pub fn dir(&self) -> &std::path::Path {
        &self.dir
    }

    fn file_of(&self, slug: &str) -> PathBuf {
        self.dir.join(slug).join("SKILL.md")
    }

    /// Alle Skills, nach Name. Kaputte Dateien fallen weg.
    pub fn list(&self) -> Vec<SkillInfo> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let names: Vec<String> = rd
            .filter_map(Result::ok)
            .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
            .map(|d| d.file_name().to_string_lossy().to_string())
            .collect();
        let mut out = Vec::new();
        for slug in names {
            if slugify(&slug) != slug {
                continue;
            }
            if let Some(s) = self.get(&slug) {
                out.push(SkillInfo { slug: s.slug, name: s.name, description: s.description, updated: s.updated, size: s.size });
            }
        }
        out.sort_by(|a, b| a.slug.cmp(&b.slug));
        out
    }

    pub fn get(&self, name: &str) -> Option<Skill> {
        let slug = slugify(name);
        if slug.is_empty() {
            return None;
        }
        let file = self.file_of(&slug);
        let meta = std::fs::metadata(&file).ok()?;
        if meta.len() > MAX_FILE as u64 {
            return None;
        }
        let text = String::from_utf8_lossy(&std::fs::read(&file).ok()?).to_string();
        let parsed = parse(&text).ok()?;
        let updated = meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        Some(Skill { slug: slug.clone(), name: slug, description: parsed.description, body: parsed.body, updated, size: meta.len() })
    }

    /// Speichern (neu oder geändert)
    pub fn save(&self, input: &SkillInput, opts: &SaveOptions) -> Result<Skill> {
        let slug = slugify(&input.name);
        let problem = check_name(&slug);
        if !problem.is_empty() {
            bail!("{problem}");
        }
        let desc = one_line(&input.description, MAX_DESCRIPTION);
        if desc.is_empty() {
            bail!("Die Beschreibung fehlt: Wann soll ASK diesen Skill benutzen?");
        }
        if input.body.trim_matches(is_js_ws).is_empty() {
            bail!("Die Anleitung fehlt.");
        }
        let text = format(&slug, &desc, &input.body);
        if text.len() > MAX_FILE {
            bail!("Der Skill ist grösser als 100 KB.");
        }
        let prev = slugify(&opts.previous);
        let file = self.file_of(&slug);
        if !opts.overwrite && slug != prev && file.exists() {
            bail!("Es gibt schon einen Skill «{slug}».");
        }
        if !file.exists() && self.list().len() >= MAX_SKILLS {
            bail!("Höchstens {MAX_SKILLS} Skills.");
        }
        if let Some(dir) = file.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp = file.clone().into_os_string();
        tmp.push(".tmp");
        let tmp = PathBuf::from(tmp);
        std::fs::write(&tmp, &text)?;
        std::fs::rename(&tmp, &file)?;
        if !prev.is_empty() && prev != slug {
            self.remove(&prev)?;
        }
        match self.get(&slug) {
            Some(s) => Ok(s),
            None => bail!("Der Skill «{slug}» liess sich nach dem Speichern nicht lesen."),
        }
    }

    pub fn remove(&self, name: &str) -> Result<bool> {
        let slug = slugify(name);
        if slug.is_empty() || !self.file_of(&slug).exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(self.dir.join(&slug))?;
        Ok(true)
    }

    /// Text der Datei für den Export
    pub fn export_text(&self, name: &str) -> Option<String> {
        self.get(name).map(|s| format(&s.slug, &s.description, &s.body))
    }

    /// Import aus einer Datei: Grösse, Text, Kopf und Name prüfen. Liefert Name, Beschreibung und
    /// Anleitung (noch nicht gespeichert).
    pub fn read_import(buf: &[u8]) -> Result<Parsed> {
        if buf.len() > MAX_FILE {
            bail!("Die Datei ist grösser als 100 KB.");
        }
        if buf.is_empty() {
            bail!("Die Datei ist leer.");
        }
        if buf.contains(&0) {
            bail!("Das ist keine Textdatei.");
        }
        let text = String::from_utf8_lossy(buf);
        if text.contains('\u{FFFD}') {
            bail!("Die Datei ist nicht in UTF-8 gespeichert.");
        }
        let parsed = parse(&text)?;
        let slug = slugify(&parsed.name);
        let problem = check_name(&slug);
        if !problem.is_empty() {
            bail!("{problem}");
        }
        Ok(Parsed { name: slug, description: parsed.description, body: parsed.body })
    }

    /// Liste für den Systemprompt: Name und Beschreibung, kurz
    pub fn prompt(&self) -> String {
        self.prompt_with(PROMPT_MAX)
    }

    pub fn prompt_with(&self, max: usize) -> String {
        let list = self.list();
        if list.is_empty() {
            return String::new();
        }
        let mut text = String::from("\n\n# Eigene Skills\nAnleitungen der Person für wiederkehrende Aufträge. Passt ein Skill zum Auftrag, lies ihn zuerst mit skill_lesen und halte dich daran.");
        let mut shown = 0;
        for s in &list {
            let line = format!("\n- {}: {}", s.slug, one_line(&s.description, PROMPT_DESCRIPTION));
            if char_len(&text) + char_len(&line) > max {
                break;
            }
            text.push_str(&line);
            shown += 1;
        }
        if shown < list.len() {
            text.push_str(&format!("\n- … und {} weitere (skill_lesen mit dem Namen)", list.len() - shown));
        }
        text
    }
}

/// «/name Text» in der Eingabe: Name des Skills und der Rest
#[derive(Clone, Debug, PartialEq)]
pub struct Invocation {
    pub slug: String,
    pub rest: String,
}

pub fn invocation(text: &str) -> Option<Invocation> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"(?is)^/([a-z0-9][a-z0-9-]*)(?:\s+(.*))?$").expect("gültiges Muster"));
    let m = re.captures(text.trim_matches(is_js_ws))?;
    Some(Invocation { slug: m[1].to_lowercase(), rest: m.get(2).map(|r| r.as_str().trim_matches(is_js_ws).to_string()).unwrap_or_default() })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKILL: &str = "---\nname: wochenbericht\ndescription: Wochenbericht nach der Vorlage schreiben\n---\n\n1. Notizen lesen.\n2. Bericht schreiben.\n";

    fn skill(name: &str, description: &str, body: &str) -> SkillInput {
        SkillInput { name: name.into(), description: description.into(), body: body.into() }
    }

    fn err(r: Result<impl std::fmt::Debug>) -> String {
        r.unwrap_err().to_string()
    }

    #[test]
    fn namen_werden_zu_sicheren_ordnernamen() {
        assert_eq!(slugify("Wöchentlicher Bericht"), "woechentlicher-bericht");
        assert_eq!(slugify("  Grüsse & Strasse!  "), "gruesse-strasse");
        assert_eq!(slugify("Café Crème"), "cafe-creme");
        assert_eq!(slugify("../../etc/passwd"), "etc-passwd");
        assert_eq!(slugify("..\\..\\AppData\\Startup"), "appdata-startup");
        assert_eq!(slugify("CON"), "con");
        assert_eq!(slugify(&"a".repeat(80)).len(), 40);
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify("名前"), "");
        assert!(!slugify(&"x-".repeat(30)).ends_with('-'));
    }

    #[test]
    fn kopf_lesen_name_description_anleitung_fehler_mit_klarer_meldung() {
        assert_eq!(parse(SKILL).unwrap(), Parsed { name: "wochenbericht".into(), description: "Wochenbericht nach der Vorlage schreiben".into(), body: "1. Notizen lesen.\n2. Bericht schreiben.".into() });
        // Windows-Zeilenenden, BOM, Anführungszeichen, eingerückte Fortsetzung
        let crlf = "\u{FEFF}---\r\nname: \"code-erklaeren\"\r\ndescription: Erklärt Code\r\n  Zeile für Zeile\r\nlicense: MIT\r\n---\r\nText\r\n";
        assert_eq!(parse(crlf).unwrap(), Parsed { name: "code-erklaeren".into(), description: "Erklärt Code Zeile für Zeile".into(), body: "Text".into() });
        assert!(err(parse("# Nur Text")).contains("Kopf"));
        assert!(err(parse("---\ndescription: x\n---\nText")).contains("name"));
        assert!(err(parse("---\nname: x\n---\nText")).contains("description"));
        assert!(err(parse("---\nname: x\ndescription: y\n---\n   \n")).contains("leer"));
        assert_eq!(parse(&format("a", "b\nc", "d")).unwrap(), Parsed { name: "a".into(), description: "b c".into(), body: "d".into() });
    }

    #[test]
    fn speichern_liste_umbenennen_loeschen_nur_in_userdata() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("skills");
        let s = Skills::new(&dir);
        assert!(s.list().is_empty());
        assert_eq!(s.prompt(), "");
        let saved = s.save(&skill("Wochenbericht", "Bericht schreiben", "Schritte"), &SaveOptions::default()).unwrap();
        assert_eq!(saved.slug, "wochenbericht");
        assert!(dir.join("wochenbericht").join("SKILL.md").exists());
        let d = SaveOptions::default();
        assert!(err(s.save(&skill("wochenbericht", "x", "y"), &d)).contains("schon"));
        assert!(err(s.save(&skill("neu", "x", "y"), &d)).contains("Befehl der App"));
        assert!(err(s.save(&skill("!!!", "x", "y"), &d)).contains("Name fehlt"));
        assert!(err(s.save(&skill("leer", " ", "y"), &d)).contains("Beschreibung"));
        assert!(err(s.save(&skill("gross", "x", &"y".repeat(MAX_FILE)), &d)).contains("100 KB"));
        // Umbenennen: alter Ordner verschwindet
        s.save(&skill("bericht", "Bericht schreiben", "Neu"), &SaveOptions { previous: "wochenbericht".into(), overwrite: false }).unwrap();
        assert_eq!(s.list().iter().map(|x| x.slug.clone()).collect::<Vec<_>>(), ["bericht"]);
        assert_eq!(s.get("Bericht").unwrap().body, "Neu");
        // Kaputte Dateien und fremde Ordnernamen erscheinen nicht
        std::fs::create_dir(dir.join("kaputt")).unwrap();
        std::fs::write(dir.join("kaputt").join("SKILL.md"), "ohne Kopf").unwrap();
        std::fs::create_dir(dir.join("Gross Mit Leer")).unwrap();
        std::fs::write(dir.join("Gross Mit Leer").join("SKILL.md"), SKILL).unwrap();
        assert_eq!(s.list().iter().map(|x| x.slug.clone()).collect::<Vec<_>>(), ["bericht"]);
        assert_eq!(s.export_text("bericht").unwrap(), "---\nname: bericht\ndescription: Bericht schreiben\n---\n\nNeu\n");
        assert!(s.remove("bericht").unwrap());
        assert!(!s.remove("bericht").unwrap());
        assert!(s.get("../bericht").is_none());
    }

    #[test]
    fn import_prueft_groesse_text_kopf_und_namen() {
        let p = Skills::read_import(SKILL.as_bytes()).unwrap();
        assert_eq!(p, Parsed { name: "wochenbericht".into(), description: "Wochenbericht nach der Vorlage schreiben".into(), body: "1. Notizen lesen.\n2. Bericht schreiben.".into() });
        assert_eq!(Skills::read_import(SKILL.replace("name: wochenbericht", "name: ../../Mein Bericht").as_bytes()).unwrap().name, "mein-bericht");
        assert!(err(Skills::read_import(&vec![b'a'; MAX_FILE + 1])).contains("100 KB"));
        assert!(err(Skills::read_import(b"")).contains("leer"));
        assert!(err(Skills::read_import(&[0x2d, 0x2d, 0x2d, 0x00, 0x0a])).contains("keine Textdatei"));
        assert!(err(Skills::read_import(&[0x2d, 0x2d, 0x2d, 0x0a, 0xff, 0xfe, 0x0a])).contains("UTF-8"));
        assert!(err(Skills::read_import(b"# Anleitung ohne Kopf")).contains("Kopf"));
        assert!(err(Skills::read_import(SKILL.replace("name: wochenbericht", "name: skills").as_bytes())).contains("Befehl"));
        assert!(err(Skills::read_import(SKILL.replace("name: wochenbericht", "name: ***").as_bytes())).contains("Name fehlt"));
    }

    #[test]
    fn liste_im_systemprompt_ist_kurz_auch_bei_vielen_skills() {
        let t = tempfile::tempdir().unwrap();
        let s = Skills::new(t.path());
        let d = SaveOptions::default();
        s.save(&skill("bericht", "Bericht schreiben", "x"), &d).unwrap();
        let re = Regex::new(r"# Eigene Skills\n.*skill_lesen.*\n- bericht: Bericht schreiben$").unwrap();
        assert!(re.is_match(&s.prompt()), "{}", s.prompt());
        for i in 0..40 {
            s.save(&skill(&format!("skill-{i}"), &format!("Beschreibung {}", "lang ".repeat(60)), "x"), &d).unwrap();
        }
        let p = s.prompt();
        assert!(char_len(&p) < 1900, "{}", char_len(&p));
        assert!(Regex::new(r"… und \d+ weitere").unwrap().is_match(&p));
    }

    #[test]
    fn aufruf_mit_name_nur_vorhandene_skills() {
        assert_eq!(invocation("/bericht Woche 47"), Some(Invocation { slug: "bericht".into(), rest: "Woche 47".into() }));
        assert_eq!(invocation("  /Bericht"), Some(Invocation { slug: "bericht".into(), rest: String::new() }));
        assert_eq!(invocation("bitte /bericht"), None);
        assert_eq!(invocation("/"), None);
    }
}
