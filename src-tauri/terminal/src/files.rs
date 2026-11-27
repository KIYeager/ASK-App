//! Datei-API (Port der Endpunkte `/files/list|read|write|replace|glob|grep` aus open-terminal 0.14.2
//! mit der Ordnersperre aus guard.py).
//!
//! Alle Funktionen blockieren und gehören in `spawn_blocking`. Jeder Pfad aus einer Anfrage läuft durch
//! `FileApi::resolve`, das ihn bereinigt und mit der Ordnersperre prüft (Fehler 403). Beim Durchsuchen
//! werden Einträge, die über Verknüpfungen aus dem Arbeitsordner hinausführen, ausgelassen.
//!
//! Öffentliche Schnittstelle:
//! - `FileApi::new(workdir)`, `resolve(pfad)`, `list`, `read`, `write`, `replace`, `glob`, `grep`
//! - `ApiError { status, detail }` (JSON `{"detail": …}`), `Replacement`, `ReadResult`

use std::path::{Path, PathBuf};

use regex::Regex;
use serde_json::{json, Value};

use crate::documents;
use crate::guard::{normpath, Denied, PathGuard};

/// Fehler einer Anfrage: HTTP-Status und `detail` wie bei FastAPI
#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: u16,
    pub detail: Value,
}

impl ApiError {
    pub fn new(status: u16, detail: impl Into<String>) -> Self {
        ApiError { status, detail: Value::String(detail.into()) }
    }
}

impl From<Denied> for ApiError {
    fn from(d: Denied) -> Self {
        ApiError::new(403, d.0)
    }
}

type Res<T> = Result<T, ApiError>;

/// Wie `str(OSError)` in Python: «[Errno 13] Permission denied: 'pfad'»
pub fn os_error_text(e: &std::io::Error, path: &Path) -> String {
    let text = e.to_string();
    let message = text.split(" (os error").next().unwrap_or(&text).to_string();
    match e.raw_os_error() {
        Some(code) => format!("[Errno {code}] {message}: '{}'", path.display()),
        None => format!("{message}: '{}'", path.display()),
    }
}

/// Eine Ersetzung von `POST /files/replace`
#[derive(Debug, Clone, Default)]
pub struct Replacement {
    pub target: String,
    pub replacement: String,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub allow_multiple: bool,
}

/// Antwort von `read`: JSON oder die Datei selbst (Bilder)
pub enum ReadResult {
    Json(Value),
    Raw { bytes: Vec<u8>, mime: String },
}

pub struct FileApi {
    guard: PathGuard,
    home: PathBuf,
    /// Ordner, die nur gelesen werden dürfen (Vorlagen der Schule, C:\Vorlagen; ENTSCHEIDE E66)
    read_only: Vec<PathGuard>,
}

/// Zeilenenden wie `_LINE_BREAK` in open-terminal
fn line_break() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new("\r\n|[\r\n\u{b}\u{c}\u{1c}-\u{1e}\u{85}\u{2028}\u{2029}]").expect("Regex"))
}

/// Zeilen mit Zeilenende wie `text_chunks`
fn split_lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    for m in line_break().find_iter(text) {
        out.push(&text[start..m.end()]);
        start = m.end();
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

/// Start und Ende (Byteposition) des Zeilenbereichs wie `line_range_offsets`
fn line_range_offsets(text: &str, start_line: Option<u32>, end_line: Option<u32>) -> (usize, usize) {
    let start_boundary = start_line.unwrap_or(1).saturating_sub(1) as usize;
    let mut start = if start_boundary == 0 { 0 } else { text.len() };
    let mut end = text.len();
    let last_boundary = start_boundary.max(end_line.unwrap_or(0) as usize);
    if last_boundary != 0 {
        for (i, m) in line_break().find_iter(text).enumerate() {
            let number = i + 1;
            if number == start_boundary {
                start = m.end();
            }
            if Some(number as u32) == end_line {
                end = m.end();
            }
            if number >= last_boundary {
                break;
            }
        }
    }
    (start, end)
}

/// Textdatei lesen wie Python im Textmodus: UTF-8 streng, Zeilenenden vereinheitlicht
fn read_text(path: &Path) -> std::io::Result<Result<String, std::str::Utf8Error>> {
    let bytes = std::fs::read(path)?;
    Ok(std::str::from_utf8(&bytes).map(|s| s.replace("\r\n", "\n").replace('\r', "\n")))
}

/// Text schreiben wie Python im Textmodus (unter Windows wird aus «\n» «\r\n»)
fn write_text(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    if cfg!(windows) {
        std::fs::write(path, content.replace('\n', "\r\n"))
    } else {
        std::fs::write(path, content)
    }
}

/// `repr()` eines Strings wie in Python (für Fehlermeldungen)
fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::new();
    out.push(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `fnmatch.fnmatch`: «*», «?», «[…]»; unter Windows ohne Gross-/Kleinschreibung
fn fnmatch(name: &str, pattern: &str) -> bool {
    let mut re = String::from("(?s)^");
    if cfg!(windows) {
        re.insert_str(0, "(?i)");
    }
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        i += 1;
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '[' => {
                let mut j = i;
                if j < chars.len() && chars[j] == '!' {
                    j += 1;
                }
                if j < chars.len() && chars[j] == ']' {
                    j += 1;
                }
                while j < chars.len() && chars[j] != ']' {
                    j += 1;
                }
                if j >= chars.len() {
                    re.push_str("\\[");
                } else {
                    let mut stuff: String = chars[i..j].iter().collect();
                    stuff = stuff.replace('\\', "\\\\");
                    if let Some(rest) = stuff.strip_prefix('!') {
                        stuff = format!("^{rest}");
                    } else if stuff.starts_with('^') {
                        stuff = format!("\\{stuff}");
                    }
                    re.push('[');
                    re.push_str(&stuff.replace('[', "\\["));
                    re.push(']');
                    i = j + 1;
                }
            }
            c => re.push_str(&regex::escape(&c.to_string())),
        }
    }
    re.push('$');
    Regex::new(&re).map(|r| r.is_match(name)).unwrap_or(false)
}

fn mtime(meta: &std::fs::Metadata) -> f64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn writable(_path: &Path, meta: &std::fs::Metadata) -> bool {
    if cfg!(windows) && meta.is_dir() {
        return true;
    }
    !meta.permissions().readonly()
}

fn guess_mime(path: &str) -> String {
    let ext = Path::new(path).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "ico" => "image/vnd.microsoft.icon",
        "svg" => "image/svg+xml",
        "heic" => "image/heic",
        "mp3" => "audio/mpeg",
        "wav" => "audio/x-wav",
        "mp4" => "video/mp4",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
    .to_string()
}

impl FileApi {
    /// `workdir`: der Arbeitsordner; relative Pfade und der Standardordner beziehen sich darauf
    pub fn new(workdir: &Path) -> Self {
        let home = crate::guard::realpath(workdir);
        FileApi { guard: PathGuard::new(&home), home, read_only: Vec::new() }
    }

    /// Dazu Ordner, die nur gelesen werden dürfen (Auflisten, Lesen, Suchen); Schreiben bleibt im Arbeitsordner
    pub fn with_read_only(mut self, dirs: &[String]) -> Self {
        self.read_only = dirs.iter().map(Path::new).filter(|d| d.is_dir()).map(PathGuard::new).collect();
        self
    }

    /// Wie `resolve`, aber auch in den Ordnern nur zum Lesen
    pub fn resolve_read(&self, path: &str) -> Res<PathBuf> {
        let p = Path::new(path);
        let target = if p.is_absolute() { normpath(p) } else { normpath(&self.home.join(p)) };
        if self.read_only.iter().any(|g| g.inside(&target)) {
            return Ok(target);
        }
        self.guard.check(&target)?;
        Ok(target)
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn guard(&self) -> &PathGuard {
        &self.guard
    }

    /// `UserFS.resolve_path` mit der Sperre: bereinigter, absoluter Pfad, der im Arbeitsordner liegen muss
    pub fn resolve(&self, path: &str) -> Res<PathBuf> {
        let p = Path::new(path);
        let target = if p.is_absolute() { normpath(p) } else { normpath(&self.home.join(p)) };
        self.guard.check(&target)?;
        Ok(target)
    }

    /// Lesbar: im Arbeitsordner oder in einem Ordner nur zum Lesen
    fn inside(&self, path: &Path) -> bool {
        self.guard.inside(path) || self.read_only.iter().any(|g| g.inside(path))
    }

    // ---------- list ----------

    pub fn list(&self, directory: &str) -> Res<Value> {
        let target = self.resolve_read(directory)?;
        if !target.is_dir() {
            return Err(ApiError::new(404, "Directory not found"));
        }
        let read = std::fs::read_dir(&target).map_err(|e| ApiError::new(400, os_error_text(&e, &target)))?;
        let mut names: Vec<String> = read.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
        names.sort();
        let mut entries = Vec::new();
        for name in names {
            let full = target.join(&name);
            if !self.inside(&full) {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&full) else { continue };
            entries.push(json!({
                "name": name,
                "type": if meta.is_dir() { "directory" } else { "file" },
                "size": meta.len(),
                "modified": mtime(&meta),
                "writable": writable(&full, &meta),
            }));
        }
        let dir_writable = std::fs::metadata(&target).map(|m| writable(&target, &m)).unwrap_or(false);
        Ok(json!({ "dir": target.to_string_lossy(), "writable": dir_writable, "entries": entries }))
    }

    // ---------- read ----------

    pub fn read(&self, path: &str, start_line: Option<u32>, end_line: Option<u32>) -> Res<ReadResult> {
        let target = self.resolve_read(path)?;
        if !target.is_file() {
            return Err(ApiError::new(404, "File not found"));
        }
        let shown = target.to_string_lossy().to_string();
        match read_text(&target) {
            Err(e) => return Err(ApiError::new(400, os_error_text(&e, &target))),
            Ok(Ok(text)) => return Ok(ReadResult::Json(text_response(&shown, &text, start_line, end_line))),
            Ok(Err(_)) => {} // kein UTF-8: Dokument oder Binärdatei
        }
        if documents::supports(&shown) {
            return match documents::extract(&shown) {
                Ok(text) => Ok(ReadResult::Json(text_response(&shown, &text, start_line, end_line))),
                Err(e) => Err(ApiError::new(if e.permission { 403 } else { 422 }, e.message)),
            };
        }
        let mime = guess_mime(&shown);
        if mime.starts_with("image") {
            let bytes = std::fs::read(&target).map_err(|e| ApiError::new(400, os_error_text(&e, &target)))?;
            return Ok(ReadResult::Raw { bytes, mime });
        }
        let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        Err(ApiError::new(415, format!("Unsupported binary file type: {mime} ({size} bytes)")))
    }

    // ---------- write ----------

    pub fn write(&self, path: &str, content: &str) -> Res<Value> {
        let target = self.resolve(path)?;
        write_text(&target, content).map_err(|e| ApiError::new(400, os_error_text(&e, &target)))?;
        Ok(json!({ "path": target.to_string_lossy(), "size": content.len() }))
    }

    // ---------- replace ----------

    pub fn replace(&self, path: &str, replacements: &[Replacement]) -> Res<Value> {
        let target = self.resolve(path)?;
        if !target.is_file() {
            return Err(ApiError::new(404, "File not found"));
        }
        let mut content = match read_text(&target) {
            Err(e) => return Err(ApiError::new(400, os_error_text(&e, &target))),
            Ok(Err(e)) => return Err(ApiError::new(400, format!("'utf-8' codec can't decode the file: {e}"))),
            Ok(Ok(text)) => text,
        };
        for chunk in replacements {
            let ranged = chunk.start_line.is_some() || chunk.end_line.is_some();
            let (start, end) = if ranged { line_range_offsets(&content, chunk.start_line, chunk.end_line) } else { (0, content.len()) };
            let end = end.max(start); // end_line vor start_line: leerer Bereich statt Absturz beim Schneiden
            let region = &content[start..end];
            let count = if chunk.target.is_empty() { region.chars().count() + 1 } else { region.matches(chunk.target.as_str()).count() };
            if count == 0 {
                let head: String = chunk.target.chars().take(100).collect();
                return Err(ApiError::new(400, format!("Target string not found: {}", py_repr(&head))));
            }
            if count > 1 && !chunk.allow_multiple {
                return Err(ApiError::new(400, format!("Found {count} occurrences of target string but allow_multiple is false")));
            }
            let replaced = region.replace(chunk.target.as_str(), &chunk.replacement);
            content = format!("{}{}{}", &content[..start], replaced, &content[start.max(end)..]);
        }
        write_text(&target, &content).map_err(|e| ApiError::new(400, os_error_text(&e, &target)))?;
        Ok(json!({ "path": target.to_string_lossy(), "size": content.len() }))
    }

    // ---------- Verzeichnisbaum ----------

    /// Wie `os.walk` von oben nach unten: (Ordner, Unterordner, Dateien), je sortiert. Unterordner, die über
    /// Verknüpfungen hinausführen, werden nicht betreten.
    fn walk(&self, root: &Path) -> Vec<(PathBuf, Vec<String>, Vec<String>)> {
        let mut result = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(read) = std::fs::read_dir(&dir) else { continue };
            let (mut dirs, mut files) = (Vec::new(), Vec::new());
            for entry in read.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                match std::fs::metadata(entry.path()) {
                    Ok(m) if m.is_dir() => dirs.push(name),
                    Ok(_) => files.push(name),
                    Err(_) => files.push(name),
                }
            }
            dirs.sort();
            files.sort();
            dirs.retain(|d| self.inside(&dir.join(d)));
            files.retain(|f| self.inside(&dir.join(f)));
            for d in dirs.iter().rev() {
                let full = dir.join(d);
                let is_link = std::fs::symlink_metadata(&full).map(|m| m.file_type().is_symlink()).unwrap_or(true);
                if !is_link {
                    stack.push(full);
                }
            }
            result.push((dir, dirs, files));
        }
        result
    }

    // ---------- glob ----------

    pub fn glob(&self, pattern: &str, path: &str, exclude: &[String], kind: &str, max_results: usize) -> Res<Value> {
        let target = self.resolve_read(path)?;
        if !target.is_dir() {
            return Err(ApiError::new(404, "Search directory not found"));
        }
        let mut matches = Vec::new();
        let mut truncated = false;
        'outer: for (dirpath, dirs, files) in self.walk(&target) {
            let mut entries: Vec<(String, &str)> = Vec::new();
            if kind == "any" || kind == "directory" {
                entries.extend(dirs.iter().map(|d| (d.clone(), "directory")));
            }
            if kind == "any" || kind == "file" {
                entries.extend(files.iter().map(|f| (f.clone(), "file")));
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, entry_type) in entries {
                let full = dirpath.join(&name);
                let rel = full.strip_prefix(&target).unwrap_or(&full).to_string_lossy().to_string();
                if !fnmatch(&name, pattern) && !fnmatch(&rel, pattern) {
                    continue;
                }
                if exclude.iter().any(|x| fnmatch(&name, x) || fnmatch(&rel, x)) {
                    continue;
                }
                if let Ok(meta) = std::fs::metadata(&full) {
                    matches.push(json!({ "path": rel, "type": entry_type, "size": meta.len(), "modified": mtime(&meta) }));
                    if matches.len() >= max_results {
                        truncated = true;
                        break 'outer;
                    }
                }
            }
        }
        Ok(json!({ "pattern": pattern, "path": target.to_string_lossy(), "matches": matches, "truncated": truncated }))
    }

    // ---------- grep ----------

    #[allow(clippy::too_many_arguments)]
    pub fn grep(&self, query: &str, path: &str, regex: bool, case_insensitive: bool, include: &[String], match_per_line: bool, max_results: usize) -> Res<Value> {
        let target = self.resolve_read(path)?;
        if !target.exists() {
            return Err(ApiError::new(404, "Search path not found"));
        }
        let source = if regex { query.to_string() } else { regex::escape(query) };
        let pattern = regex::RegexBuilder::new(&source)
            .case_insensitive(case_insensitive)
            .build()
            .map_err(|e| ApiError::new(400, format!("Invalid regex: {e}")))?;
        let mut matches: Vec<Value> = Vec::new();
        let mut truncated = false;
        let search_file = |file: &Path, matches: &mut Vec<Value>, truncated: &mut bool| {
            if *truncated {
                return;
            }
            let Ok(bytes) = std::fs::read(file) else { return };
            // wie Python: bis zur ersten ungültigen UTF-8-Folge lesen, danach gilt die Datei als Binärdatei
            let text = match std::str::from_utf8(&bytes) {
                Ok(t) => t,
                Err(e) => std::str::from_utf8(&bytes[..e.valid_up_to()]).unwrap_or(""),
            };
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            for (i, line) in normalized.split_inclusive('\n').enumerate() {
                if pattern.is_match(line) {
                    if match_per_line {
                        matches.push(json!({ "file": file.to_string_lossy(), "line": i + 1, "content": line.trim_end_matches(['\n', '\r']) }));
                        if matches.len() >= max_results {
                            *truncated = true;
                            return;
                        }
                    } else {
                        matches.push(json!({ "file": file.to_string_lossy() }));
                        if matches.len() >= max_results {
                            *truncated = true;
                        }
                        return;
                    }
                }
            }
        };
        if target.is_file() {
            search_file(&target, &mut matches, &mut truncated);
        } else {
            'outer: for (dirpath, _dirs, files) in self.walk(&target) {
                if truncated {
                    break;
                }
                for name in files {
                    if !include.is_empty() && !include.iter().any(|g| fnmatch(&name, g)) {
                        continue;
                    }
                    search_file(&dirpath.join(&name), &mut matches, &mut truncated);
                    if truncated {
                        break 'outer;
                    }
                }
            }
        }
        Ok(json!({ "query": query, "path": target.to_string_lossy(), "matches": matches, "truncated": truncated }))
    }
}

fn text_response(path: &str, text: &str, start_line: Option<u32>, end_line: Option<u32>) -> Value {
    let lines = split_lines(text);
    let total = lines.len();
    let mut content = String::new();
    for (i, line) in lines.iter().enumerate() {
        let number = i as u32 + 1;
        if number >= start_line.unwrap_or(1) && end_line.is_none_or(|e| number <= e) {
            content.push_str(line);
        }
    }
    json!({ "path": path, "content": content, "total_lines": total })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api() -> (tempfile::TempDir, FileApi) {
        let base = tempfile::tempdir().unwrap();
        let work = base.path().join("Arbeitsordner Prüfung");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(base.path().join("ausserhalb")).unwrap();
        std::fs::write(base.path().join("ausserhalb").join("geheim.txt"), "nicht lesen").unwrap();
        std::fs::write(work.join("Übung.txt"), "Grüezi").unwrap();
        let api = FileApi::new(&work);
        (base, api)
    }

    #[test]
    fn zeilen_und_bereiche() {
        assert_eq!(split_lines("a\nb\r\nc"), vec!["a\n", "b\r\n", "c"]);
        let v = text_response("p", "eins\nzwei\ndrei\n", Some(2), Some(2));
        assert_eq!(v["content"], "zwei\n");
        assert_eq!(v["total_lines"], 3);
        let (s, e) = line_range_offsets("a\nb\nc\n", Some(2), Some(2));
        assert_eq!(&"a\nb\nc\n"[s..e], "b\n");
    }

    #[test]
    fn schreiben_lesen_ersetzen() {
        let (_b, api) = api();
        let r = api.write("ausgabe/test.txt", "Grüsse\nzwei\n").unwrap();
        assert_eq!(r["size"], "Grüsse\nzwei\n".len());
        let ReadResult::Json(v) = api.read("ausgabe/test.txt", None, None).unwrap() else { panic!() };
        assert_eq!(v["content"], "Grüsse\nzwei\n");
        assert_eq!(v["total_lines"], 2);
        let rep = |t: &str, r: &str, multi: bool| Replacement { target: t.into(), replacement: r.into(), allow_multiple: multi, ..Default::default() };
        api.replace("ausgabe/test.txt", &[rep("zwei", "drei", false)]).unwrap();
        let err = api.replace("ausgabe/test.txt", &[rep("nichts", "x", false)]).unwrap_err();
        assert_eq!((err.status, err.detail.as_str().unwrap()), (400, "Target string not found: 'nichts'"));
        api.write("m.txt", "a a").unwrap();
        let err = api.replace("m.txt", &[rep("a", "b", false)]).unwrap_err();
        assert!(err.detail.as_str().unwrap().starts_with("Found 2 occurrences"));
        api.replace("m.txt", &[rep("a", "b", true)]).unwrap();
        let ReadResult::Json(v) = api.read("m.txt", None, None).unwrap() else { panic!() };
        assert_eq!(v["content"], "b b");
        // end_line vor start_line: Meldung «nicht gefunden» statt Absturz
        api.write("z.txt", "a\nb\nc\nd\n").unwrap();
        let ranged = Replacement { target: "c".into(), replacement: "x".into(), start_line: Some(3), end_line: Some(1), ..Default::default() };
        assert_eq!(api.replace("z.txt", &[ranged]).unwrap_err().status, 400);
    }

    #[test]
    fn vorlagen_nur_lesen() {
        let base = tempfile::tempdir().unwrap();
        let work = base.path().join("projekt");
        let vorlagen = base.path().join("Vorlagen");
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(vorlagen.join("Word")).unwrap();
        std::fs::write(vorlagen.join("Word").join("brief.txt"), "Vorlage").unwrap();
        let api = FileApi::new(&work).with_read_only(&[vorlagen.to_string_lossy().to_string()]);
        let file = vorlagen.join("Word").join("brief.txt").to_string_lossy().to_string();
        assert!(matches!(api.read(&file, None, None).unwrap(), ReadResult::Json(_)));
        assert_eq!(api.list(&vorlagen.to_string_lossy()).unwrap()["entries"][0]["name"], "Word");
        assert_eq!(api.glob("*.txt", &vorlagen.to_string_lossy(), &[], "file", 10).unwrap()["matches"].as_array().unwrap().len(), 1);
        assert_eq!(api.write(&file, "x").unwrap_err().status, 403, "nur lesen");
        assert_eq!(api.write(&vorlagen.join("neu.txt").to_string_lossy(), "x").unwrap_err().status, 403);
        assert_eq!(std::fs::read_to_string(vorlagen.join("Word").join("brief.txt")).unwrap(), "Vorlage");
    }

    #[test]
    fn sperre_blockiert_ausserhalb() {
        let (base, api) = api();
        let outside = base.path().join("ausserhalb").join("geheim.txt").to_string_lossy().to_string();
        for p in [outside.as_str(), "../ausserhalb/geheim.txt"] {
            let Err(err) = api.read(p, None, None) else { panic!("{p}") };
            assert_eq!(err.status, 403, "{p}");
            assert!(err.detail.as_str().unwrap().starts_with("Zugriff verweigert:"));
        }
        assert_eq!(api.write("../ausserhalb/neu.txt", "x").unwrap_err().status, 403);
        assert!(!base.path().join("ausserhalb").join("neu.txt").exists());
        assert_eq!(api.list("..").unwrap_err().status, 403);
    }

    #[test]
    fn liste_glob_grep() {
        let (_b, api) = api();
        api.write("sub/a.py", "print('x')\nHallo Welt\n").unwrap();
        api.write("sub/b.txt", "hallo\n").unwrap();
        let v = api.list(".").unwrap();
        let names: Vec<&str> = v["entries"].as_array().unwrap().iter().map(|e| e["name"].as_str().unwrap()).collect();
        assert_eq!(names, vec!["sub", "Übung.txt"]);
        assert_eq!(v["entries"][0]["type"], "directory");
        let g = api.glob("*.py", ".", &[], "any", 50).unwrap();
        assert_eq!(g["matches"].as_array().unwrap().len(), 1);
        assert!(g["matches"][0]["path"].as_str().unwrap().ends_with("a.py"));
        let s = api.grep("hallo", ".", false, true, &[], true, 50).unwrap();
        assert_eq!(s["matches"].as_array().unwrap().len(), 2);
        assert_eq!(s["matches"][0]["line"], 2);
        assert_eq!(s["matches"][0]["content"], "Hallo Welt");
        let s = api.grep("hallo", ".", false, true, &["*.txt".into()], true, 50).unwrap();
        assert_eq!(s["matches"].as_array().unwrap().len(), 1);
        let s = api.grep("hallo", ".", false, true, &[], true, 1).unwrap();
        assert_eq!(s["truncated"], true);
        assert_eq!(api.grep("(", ".", true, false, &[], true, 5).unwrap_err().status, 400);
    }

    #[test]
    fn binaer_und_bild() {
        let (_b, api) = api();
        std::fs::write(api.home().join("b.bin"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::write(api.home().join("b.png"), [0x89, b'P', 0xff]).unwrap();
        assert_eq!(api.read("b.bin", None, None).err().unwrap().status, 415);
        assert!(matches!(api.read("b.png", None, None).unwrap(), ReadResult::Raw { mime, .. } if mime == "image/png"));
        assert_eq!(api.read("fehlt.txt", None, None).err().unwrap().status, 404);
    }

    #[test]
    fn fnmatch_wie_python() {
        assert!(fnmatch("a.py", "*.py"));
        assert!(fnmatch("sub/a.py", "sub/*"));
        assert!(fnmatch("a1", "a[0-9]"));
        assert!(!fnmatch("ab", "a[!b]"));
        assert!(!fnmatch("a.pyc", "*.py"));
    }
}
