//! Mods (ENTSCHEIDE E72): Erweiterungen der Oberfläche, die die Person selbst anlegt, von ASK erstellen lässt
//! oder aus einem Katalog holt. Ein Mod ist ein Ordner `userData/mods/<id>/` mit `mod.json` und Dateien:
//! Aussehen (CSS), Code für die Oberfläche (JavaScript), eigene /Befehle, Zusatzanweisungen für ASK und
//! Regeln, die ASK nur strenger machen («nachfragen»).
//!
//! Sicherheit: Ein neuer oder geänderter Mod ist aus. Einschalten speichert einen Fingerabdruck (SHA-256) aller
//! Dateien; ändert sich danach eine Datei (auch von aussen, z. B. durch einen Befehl), gilt der Mod als geändert
//! und lädt erst wieder nach erneutem Einschalten. Solange ein Mod mit Code läuft, bestätigt die Person heikle
//! Schritte in einem Dialog von Windows statt in der Oberfläche (app.rs, `secure`), weil Code in der Oberfläche
//! dort jeden Knopf drücken könnte.
//!
//! Öffentliche Schnittstelle: `Mods::new(dir, examples)`, `list`, `get`, `save`, `remove`, `enable`, `active`,
//! `code_active`, `file`, `prompt`, `ask_before`, `examples`, `example_files`; freie Funktionen `parse_manifest`,
//! `check_id`, `check_path`.

use anyhow::{bail, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MAX_ID: usize = 40;
pub const MAX_FILES: usize = 60;
pub const MAX_TOTAL: usize = 3 * 1024 * 1024;
pub const MAX_TEXT_FILE: usize = 512 * 1024;
pub const MAX_INSTRUCTIONS: usize = 2000;
pub const MAX_COMMANDS: usize = 20;
pub const PROMPT_MAX: usize = 4000;
/// Arten von Schritten, vor denen ein Mod Rückfragen verlangen kann
pub const ASK_KINDS: &[&str] = &["loeschen", "schreiben", "befehle", "web"];
/// Erlaubte Dateiendungen (Text und Medien; keine Programme)
const EXTENSIONS: &[&str] = &["json", "css", "js", "md", "txt", "png", "jpg", "jpeg", "gif", "svg", "webp", "mp3", "wav", "ogg", "woff2", "woff", "ttf"];
const TEXT_EXTENSIONS: &[&str] = &["json", "css", "js", "md", "txt", "svg"];

/// Name → Kennung: klein, nur a-z, 0-9 und Bindestriche
pub fn slug(name: &str) -> String {
    crate::skills::slugify(name)
}

pub fn check_id(id: &str) -> Result<()> {
    if id.is_empty() {
        bail!("Die Kennung «id» fehlt (Buchstaben, Ziffern, Bindestriche).");
    }
    if id.chars().count() > MAX_ID || slug(id) != id {
        bail!("Die Kennung «{id}» geht nicht: nur Kleinbuchstaben, Ziffern und Bindestriche, höchstens {MAX_ID} Zeichen.");
    }
    Ok(())
}

/// Pfad einer Datei im Mod: relativ, ohne «..», nur erlaubte Endungen
pub fn check_path(path: &str) -> Result<String> {
    let p = path.replace('\\', "/");
    let p = p.trim_start_matches("./").to_string();
    if p.is_empty() || p.starts_with('/') || p.contains(':') || p.split('/').any(|s| s.is_empty() || s == "." || s == ".." || s.starts_with('.')) {
        bail!("Ungültiger Dateiname im Mod: «{path}».");
    }
    let ext = p.rsplit('.').next().unwrap_or("").to_lowercase();
    if !p.contains('.') || !EXTENSIONS.contains(&ext.as_str()) {
        bail!("Dateityp nicht erlaubt im Mod: «{path}» (erlaubt: {}).", EXTENSIONS.join(", "));
    }
    Ok(p)
}

fn is_text(path: &str) -> bool {
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    TEXT_EXTENSIONS.contains(&ext.as_str())
}

/// Eigener /Befehl eines Mods: schickt eine Vorlage, {{text}} ist der Rest der Eingabe
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModCommand {
    pub name: String,
    pub beschreibung: String,
    pub prompt: String,
}

/// Inhalt von mod.json
#[derive(Clone, Debug, PartialEq, Serialize, Default)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub beschreibung: String,
    pub autor: String,
    pub style: String,
    pub script: String,
    pub befehle: Vec<ModCommand>,
    pub anweisungen: String,
    pub nachfragen: Vec<String>,
}

fn s(v: &Value, k: &str, max: usize) -> String {
    crate::util::take_chars(v.get(k).and_then(Value::as_str).unwrap_or("").trim(), max)
}

/// mod.json lesen und prüfen
pub fn parse_manifest(text: &str) -> Result<Manifest> {
    let v: Value = serde_json::from_str(text.strip_prefix('\u{FEFF}').unwrap_or(text)).map_err(|e| anyhow::anyhow!("mod.json ist kein gültiges JSON: {e}"))?;
    if !v.is_object() {
        bail!("mod.json muss ein Objekt sein.");
    }
    let id = s(&v, "id", 80);
    check_id(&id)?;
    let name = s(&v, "name", 60);
    if name.is_empty() {
        bail!("In mod.json fehlt «name».");
    }
    let style = s(&v, "style", 200);
    let script = s(&v, "script", 200);
    for (f, ext) in [(&style, "css"), (&script, "js")] {
        if !f.is_empty() {
            let p = check_path(f)?;
            if !p.to_lowercase().ends_with(&format!(".{ext}")) {
                bail!("«{f}» muss eine .{ext}-Datei sein.");
            }
        }
    }
    let mut befehle = Vec::new();
    for c in v.get("befehle").and_then(Value::as_array).cloned().unwrap_or_default().iter().take(MAX_COMMANDS) {
        let name = slug(&s(c, "name", 40));
        let check = crate::skills::check_name(&name);
        if !check.is_empty() {
            bail!("Befehl im Mod: {check}");
        }
        let prompt = s(c, "prompt", 4000);
        if prompt.is_empty() {
            bail!("Befehl «/{name}» hat keinen «prompt».");
        }
        befehle.push(ModCommand { name, beschreibung: s(c, "beschreibung", 200), prompt });
    }
    let nachfragen: Vec<String> = v.get("nachfragen").and_then(Value::as_array).cloned().unwrap_or_default().iter()
        .filter_map(Value::as_str).map(str::to_string).filter(|k| ASK_KINDS.contains(&k.as_str())).collect();
    Ok(Manifest {
        id,
        name,
        version: s(&v, "version", 20),
        beschreibung: s(&v, "beschreibung", 300),
        autor: s(&v, "autor", 80),
        style: if style.is_empty() { style } else { check_path(&style)? },
        script: if script.is_empty() { script } else { check_path(&script)? },
        befehle,
        anweisungen: s(&v, "anweisungen", MAX_INSTRUCTIONS),
        nachfragen,
    })
}

/// Eintrag der Liste
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModInfo {
    #[serde(flatten)]
    pub manifest: Manifest,
    /// eingeschaltet und unverändert
    pub aktiv: bool,
    /// eingeschaltet, aber seither geändert: lädt erst nach erneutem Einschalten
    pub geaendert: bool,
    pub groesse: u64,
    pub dateien: usize,
    /// Fingerabdruck der Dateien (Adresse der Dateien in der Oberfläche, damit nichts aus dem Zwischenspeicher kommt)
    pub hash: String,
    /// Fehler in mod.json (dann ist der Mod nie aktiv)
    pub fehler: String,
}

/// Datei eines Mods für die Vorschau
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModFile {
    pub path: String,
    pub size: u64,
    /// Inhalt bei Textdateien, sonst leer
    pub text: String,
}

pub struct Mods {
    dir: PathBuf,
    /// Mitgelieferte Beispiele (resources/mods-beispiele), nur lesen
    examples: Option<PathBuf>,
}

impl Mods {
    pub fn new(dir: impl Into<PathBuf>, examples: Option<PathBuf>) -> Self {
        Mods { dir: dir.into(), examples }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn state_file(&self) -> PathBuf {
        self.dir.join("aktiv.json")
    }

    /// Eingeschaltete Mods: Kennung → Fingerabdruck beim Einschalten
    fn enabled(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(self.state_file()).ok()
            .and_then(|t| serde_json::from_str::<BTreeMap<String, String>>(&t).ok())
            .unwrap_or_default()
    }

    fn write_enabled(&self, map: &BTreeMap<String, String>) -> Result<()> {
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join("aktiv.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(map)?)?;
        std::fs::rename(&tmp, self.state_file())?;
        Ok(())
    }

    /// Alle Dateien eines Mod-Ordners (relative Pfade mit «/», sortiert)
    fn files_in(root: &Path) -> Vec<String> {
        let mut out: Vec<String> = walkdir::WalkDir::new(root).follow_links(false).into_iter().filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .filter_map(|e| e.path().strip_prefix(root).ok().map(|p| p.to_string_lossy().replace('\\', "/")))
            .collect();
        out.sort();
        out
    }

    /// Fingerabdruck über Namen und Inhalt aller Dateien
    fn fingerprint(root: &Path) -> (String, u64, usize) {
        let mut h = Sha256::new();
        let mut size = 0u64;
        let files = Self::files_in(root);
        for f in &files {
            let data = std::fs::read(root.join(f)).unwrap_or_default();
            size += data.len() as u64;
            h.update((f.len() as u64).to_le_bytes());
            h.update(f.as_bytes());
            h.update((data.len() as u64).to_le_bytes());
            h.update(&data);
        }
        (hex::encode(h.finalize()), size, files.len())
    }

    fn read(&self, root: &Path, id: &str, enabled: &BTreeMap<String, String>) -> Option<ModInfo> {
        let text = std::fs::read_to_string(root.join("mod.json")).ok()?;
        let (hash, groesse, dateien) = Self::fingerprint(root);
        let (manifest, fehler) = match parse_manifest(&text) {
            Ok(m) if m.id == id => (m, String::new()),
            Ok(m) => (m.clone(), format!("Die Kennung in mod.json («{}») passt nicht zum Ordner «{id}».", m.id)),
            Err(e) => (Manifest { id: id.to_string(), name: id.to_string(), ..Default::default() }, e.to_string()),
        };
        let on = enabled.get(id);
        let aktiv = fehler.is_empty() && on == Some(&hash);
        Some(ModInfo { manifest, aktiv, geaendert: on.is_some() && !aktiv, groesse, dateien, hash, fehler })
    }

    pub fn list(&self) -> Vec<ModInfo> {
        let enabled = self.enabled();
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut out: Vec<ModInfo> = rd.filter_map(Result::ok)
            .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
            .map(|d| d.file_name().to_string_lossy().to_string())
            .filter(|n| check_id(n).is_ok())
            .filter_map(|id| self.read(&self.dir.join(&id), &id, &enabled))
            .collect();
        out.sort_by_key(|m| m.manifest.name.to_lowercase());
        out
    }

    pub fn info(&self, id: &str) -> Option<ModInfo> {
        check_id(id).ok()?;
        self.read(&self.dir.join(id), id, &self.enabled())
    }

    /// Mod mit allen Dateien (Textdateien mit Inhalt) für die Vorschau vor dem Einschalten
    pub fn get(&self, id: &str) -> Option<Value> {
        let info = self.info(id)?;
        let root = self.dir.join(id);
        let files: Vec<ModFile> = Self::files_in(&root).into_iter().map(|p| {
            let size = std::fs::metadata(root.join(&p)).map(|m| m.len()).unwrap_or(0);
            let text = if is_text(&p) && size as usize <= MAX_TEXT_FILE { std::fs::read_to_string(root.join(&p)).unwrap_or_default() } else { String::new() };
            ModFile { path: p, size, text }
        }).collect();
        let mut v = serde_json::to_value(&info).unwrap_or(Value::Null);
        v["files"] = json!(files);
        Some(v)
    }

    /// Dateien prüfen: Anzahl, Grösse, Namen, mod.json; gibt das Manifest zurück
    pub fn check_files(files: &[(String, Vec<u8>)]) -> Result<Manifest> {
        if files.len() > MAX_FILES {
            bail!("Höchstens {MAX_FILES} Dateien pro Mod.");
        }
        let total: usize = files.iter().map(|(_, d)| d.len()).sum();
        if total > MAX_TOTAL {
            bail!("Der Mod ist grösser als {} MB.", MAX_TOTAL / 1024 / 1024);
        }
        for (p, d) in files {
            let p = check_path(p)?;
            if is_text(&p) && (d.len() > MAX_TEXT_FILE || std::str::from_utf8(d).is_err()) {
                bail!("«{p}» ist keine Textdatei in UTF-8 (oder grösser als 512 KB).");
            }
        }
        let Some((_, manifest)) = files.iter().find(|(p, _)| p == "mod.json") else { bail!("Die Datei mod.json fehlt.") };
        let m = parse_manifest(&String::from_utf8_lossy(manifest))?;
        for f in [&m.style, &m.script] {
            if !f.is_empty() && !files.iter().any(|(p, _)| check_path(p).ok().as_deref() == Some(f.as_str())) {
                bail!("mod.json nennt «{f}», die Datei fehlt aber.");
            }
        }
        Ok(m)
    }

    /// Mod speichern (ersetzt einen gleichnamigen ganz). Danach ist er aus, bis die Person ihn einschaltet.
    pub fn save(&self, files: &[(String, Vec<u8>)], overwrite: bool) -> Result<ModInfo> {
        let m = Self::check_files(files)?;
        let root = self.dir.join(&m.id);
        if root.exists() && !overwrite {
            bail!("Es gibt schon einen Mod «{}».", m.id);
        }
        if !root.exists() && self.list().len() >= 100 {
            bail!("Höchstens 100 Mods.");
        }
        std::fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(".{}.neu", m.id));
        let _ = std::fs::remove_dir_all(&tmp);
        for (p, d) in files {
            let p = check_path(p)?;
            let target = tmp.join(&p);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, d)?;
        }
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::rename(&tmp, &root)?;
        let mut en = self.enabled();
        if en.remove(&m.id).is_some() {
            self.write_enabled(&en)?;
        }
        self.info(&m.id).ok_or_else(|| anyhow::anyhow!("Der Mod liess sich nach dem Speichern nicht lesen."))
    }

    /// Dateien eines Ordners auf der Festplatte (Import)
    pub fn read_folder(folder: &Path) -> Result<Vec<(String, Vec<u8>)>> {
        if !folder.join("mod.json").is_file() {
            bail!("Im Ordner liegt keine mod.json.");
        }
        let files = Self::files_in(folder);
        if files.len() > MAX_FILES {
            bail!("Höchstens {MAX_FILES} Dateien pro Mod.");
        }
        files.into_iter().map(|p| Ok((p.clone(), std::fs::read(folder.join(&p))?))).collect()
    }

    pub fn remove(&self, id: &str) -> Result<bool> {
        check_id(id)?;
        let root = self.dir.join(id);
        if !root.exists() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&root)?;
        let mut en = self.enabled();
        if en.remove(id).is_some() {
            self.write_enabled(&en)?;
        }
        Ok(true)
    }

    /// Ein- oder ausschalten. Einschalten merkt den Fingerabdruck von jetzt.
    pub fn enable(&self, id: &str, on: bool) -> Result<bool> {
        let mut en = self.enabled();
        if on {
            let Some(info) = self.info(id) else { return Ok(false) };
            if !info.fehler.is_empty() {
                bail!("{}", info.fehler);
            }
            en.insert(id.to_string(), info.hash);
        } else if en.remove(id).is_none() {
            return Ok(false);
        }
        self.write_enabled(&en)?;
        Ok(true)
    }

    /// Alle aus (Ctrl+Shift+M, abgesicherter Start)
    pub fn disable_all(&self) -> Result<usize> {
        let n = self.enabled().len();
        if n > 0 {
            self.write_enabled(&BTreeMap::new())?;
        }
        Ok(n)
    }

    /// Eingeschaltete und unveränderte Mods
    pub fn active(&self) -> Vec<ModInfo> {
        if self.enabled().is_empty() {
            return Vec::new();
        }
        self.list().into_iter().filter(|m| m.aktiv).collect()
    }

    /// Läuft Code eines Mods in der Oberfläche? Dann gelten die sicheren Rückfragen (app.rs).
    pub fn code_active(&self) -> bool {
        self.active().iter().any(|m| !m.manifest.script.is_empty())
    }

    /// Datei eines aktiven Mods für die Oberfläche (Protokoll askmod). Nur aktive, unveränderte Mods.
    pub fn file(&self, id: &str, path: &str) -> Option<Vec<u8>> {
        let info = self.info(id).filter(|m| m.aktiv)?;
        let p = check_path(path).ok()?;
        let root = self.dir.join(&info.manifest.id);
        let data = std::fs::read(root.join(&p)).ok()?;
        // Nochmals prüfen: die Datei könnte seit dem Fingerabdruck geändert worden sein
        if Self::fingerprint(&root).0 != info.hash {
            return None;
        }
        Some(data)
    }

    /// Zusatzanweisungen der aktiven Mods für den Systemprompt
    pub fn prompt(&self) -> String {
        let mods: Vec<ModInfo> = self.active().into_iter().filter(|m| !m.manifest.anweisungen.is_empty()).collect();
        if mods.is_empty() {
            return String::new();
        }
        let mut text = String::from("\n\n# Mods\nDie Person hat diese Mods eingeschaltet. Ihre Anweisungen gelten zusätzlich, aber nie gegen Sicherheit, Rückfragen oder die übrigen Regeln.");
        for m in mods {
            let line = format!("\n## {}\n{}", m.manifest.name, m.manifest.anweisungen);
            if crate::util::char_len(&text) + crate::util::char_len(&line) > PROMPT_MAX {
                break;
            }
            text.push_str(&line);
        }
        text
    }

    /// Arten von Schritten, vor denen aktive Mods eine Rückfrage verlangen
    pub fn ask_before(&self) -> Vec<String> {
        let mut out: Vec<String> = self.active().into_iter().flat_map(|m| m.manifest.nachfragen).collect();
        out.sort();
        out.dedup();
        out
    }

    /// Mitgelieferte Beispiele (nicht installiert)
    pub fn examples(&self) -> Vec<Value> {
        let Some(dir) = &self.examples else { return Vec::new() };
        let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
        let mut out: Vec<Value> = rd.filter_map(Result::ok)
            .filter(|d| d.file_type().is_ok_and(|t| t.is_dir()))
            .filter_map(|d| {
                let text = std::fs::read_to_string(d.path().join("mod.json")).ok()?;
                let m = parse_manifest(&text).ok()?;
                let mut v = serde_json::to_value(&m).ok()?;
                v["installiert"] = json!(self.dir.join(&m.id).join("mod.json").exists());
                Some(v)
            })
            .collect();
        out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        out
    }

    /// Dateien eines mitgelieferten Beispiels
    pub fn example_files(&self, id: &str) -> Result<Vec<(String, Vec<u8>)>> {
        check_id(id)?;
        let Some(dir) = &self.examples else { bail!("Keine Beispiele vorhanden.") };
        Self::read_folder(&dir.join(id))
    }
}

/// Dateien aus JSON `{ "pfad": "inhalt" }` (Werkzeug mod_erstellen)
pub fn files_from_json(v: &Value) -> Result<Vec<(String, Vec<u8>)>> {
    let Some(map) = v.as_object() else { bail!("«dateien» muss ein Objekt sein: Dateiname → Inhalt.") };
    let map: &Map<String, Value> = map;
    Ok(map.iter().map(|(k, v)| (k.clone(), crate::util::js_string(v).into_bytes())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(id: &str, extra: &str) -> Vec<u8> {
        format!("{{\"id\":\"{id}\",\"name\":\"Test {id}\",\"version\":\"1.0.0\"{extra}}}").into_bytes()
    }

    #[test]
    fn kennung_und_pfade_werden_geprueft() {
        assert!(check_id("robo-helfer").is_ok());
        for bad in ["", "Robo", "a b", "../x", &"a".repeat(41)] {
            assert!(check_id(bad).is_err(), "{bad}");
        }
        assert_eq!(check_path("bilder/robo.png").unwrap(), "bilder/robo.png");
        assert_eq!(check_path("bilder\\robo.png").unwrap(), "bilder/robo.png");
        for bad in ["../x.js", "/x.js", "C:/x.js", "a/../b.js", ".hidden.js", "x.exe", "x.ps1", "x", "a//b.js"] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn manifest_lesen_befehle_und_regeln() {
        let m = parse_manifest(r#"{ "id": "vok", "name": "Vokabeln", "befehle": [{ "name": "Vokabeln!", "prompt": "Frag {{text}}" }],
            "nachfragen": ["loeschen", "alles-erlauben"], "anweisungen": "Sei kurz." }"#).unwrap();
        assert_eq!(m.befehle[0].name, "vokabeln");
        assert_eq!(m.nachfragen, vec!["loeschen"]);
        assert!(parse_manifest(r#"{ "id": "x", "name": "X", "befehle": [{ "name": "neu", "prompt": "p" }] }"#).is_err(), "Befehl der App");
        assert!(parse_manifest(r#"{ "id": "x", "name": "X", "script": "mod.css" }"#).is_err());
        assert!(parse_manifest("[]").is_err());
        assert!(parse_manifest(r#"{ "name": "X" }"#).is_err());
    }

    #[test]
    fn neu_ist_aus_einschalten_und_aenderung_schaltet_ab() {
        let t = tempfile::tempdir().unwrap();
        let mods = Mods::new(t.path().join("mods"), None);
        let files = vec![("mod.json".to_string(), manifest("robo", ",\"script\":\"mod.js\",\"anweisungen\":\"Sag hallo.\",\"nachfragen\":[\"web\"]")), ("mod.js".to_string(), b"console.log(1)".to_vec())];
        let info = mods.save(&files, false).unwrap();
        assert!(!info.aktiv);
        assert!(!mods.code_active());
        assert!(mods.file("robo", "mod.js").is_none(), "aus: keine Dateien");
        assert!(mods.enable("robo", true).unwrap());
        assert!(mods.code_active());
        assert_eq!(mods.file("robo", "mod.js").unwrap(), b"console.log(1)");
        assert!(mods.file("robo", "../aktiv.json").is_none());
        assert!(mods.prompt().contains("Sag hallo."));
        assert_eq!(mods.ask_before(), vec!["web"]);
        // Von aussen geändert: nicht mehr aktiv, Dateien nicht mehr geliefert
        std::fs::write(t.path().join("mods/robo/mod.js"), "alert(1)").unwrap();
        let i = mods.info("robo").unwrap();
        assert!(!i.aktiv && i.geaendert);
        assert!(mods.file("robo", "mod.js").is_none());
        assert!(!mods.code_active());
        // Neu speichern schaltet aus; erneut einschalten
        mods.enable("robo", true).unwrap();
        assert!(mods.code_active());
        assert!(mods.save(&files, false).is_err(), "schon vorhanden");
        let i = mods.save(&files, true).unwrap();
        assert!(!i.aktiv && !i.geaendert);
        mods.enable("robo", true).unwrap();
        assert_eq!(mods.disable_all().unwrap(), 1);
        assert!(mods.active().is_empty());
        assert!(mods.remove("robo").unwrap());
        assert!(mods.list().is_empty());
    }

    #[test]
    fn dateien_werden_vor_dem_speichern_geprueft() {
        let ok = ("mod.json".to_string(), manifest("a", ""));
        assert!(Mods::check_files(std::slice::from_ref(&ok)).is_ok());
        assert!(Mods::check_files(&[]).is_err());
        assert!(Mods::check_files(&[ok.clone(), ("x.exe".into(), vec![1])]).is_err());
        assert!(Mods::check_files(&[("mod.json".into(), manifest("a", ",\"style\":\"s.css\""))]).is_err(), "s.css fehlt");
        assert!(Mods::check_files(&[ok.clone(), ("big.png".into(), vec![0; MAX_TOTAL + 1])]).is_err());
        assert!(Mods::check_files(&[ok, ("x.js".into(), vec![0xff, 0xfe])]).is_err(), "kein UTF-8");
        assert_eq!(files_from_json(&json!({ "mod.json": "{}" })).unwrap()[0].0, "mod.json");
    }
}
