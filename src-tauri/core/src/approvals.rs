//! Rückfrage vor heiklen Schritten (ENTSCHEIDE E33). Das Modell hält an, die Person entscheidet im Chat:
//! Erlauben, Immer in diesem Chat, Ablehnen. Stufen in den Einstellungen:
//!   aus     nie fragen
//!   heikel  vor Löschen, vor Überschreiben vorhandener Dateien und vor Befehlen mit Pfaden ausserhalb des Ordners
//!   alle    zusätzlich vor jedem Befehl und jeder Änderung an vorhandenen Dateien
//! Erkannt wird, was im Befehl selbst steht. Ein Python-Skript, das intern löscht, fällt nicht darunter;
//! dafür bleibt «Rückgängig» (snapshot.rs).
//!
//! Öffentliche Schnittstelle:
//! - `assess_risk(name, args, &RiskCtx) -> Option<Risk>`
//! - `command_words(command) -> Vec<String>`
//! - `analyze(command, depth) -> Found`
//! - `MODES`

use regex::Regex;
use serde_json::Value;
use std::sync::LazyLock;

pub const MODES: [&str; 3] = ["aus", "heikel", "alle"];

// Befehle, die Dateien oder Ordner löschen (cmd, PowerShell, Unix-Werkzeuge in Git for Windows)
const DELETE_WORDS: &[&str] = &[
    "del", "erase", "rd", "rmdir", "rm", "remove-item", "ri", "format", "cipher", "sdelete", "clear-content", "clc", "shred",
];
// Befehle, die Dateien verschieben oder ohne Rückfrage überschreiben
const MOVE_WORDS: &[&str] = &["move", "move-item", "mi", "mv", "robocopy", "set-content", "sc", "out-file"];
// Programme, die einen weiteren Befehl ausführen: dieser wird ebenfalls geprüft
const SHELLS: &[&str] = &["cmd", "powershell", "pwsh"];
const PASS_THROUGH: &[&str] = &["start", "call", "do", "else", "then"];

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("gültiger regulärer Ausdruck")
}

// Löschen in Python- oder PowerShell-Einzeilern
static DELETE_CALLS: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)(?-u:\b)(os\.(remove|unlink|rmdir|removedirs)|shutil\.rmtree|\.unlink\s*\(|\.rmdir\s*\(|send2trash|Remove-Item|\[System\.IO\.(File|Directory)\]::Delete)(?-u:\b)")
});
// Verschieben oder Überschreiben in Python-Einzeilern
static MOVE_CALLS: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"(?i)(?-u:\b)(shutil\.(move|copy[A-Za-z0-9_]*)|os\.(replace|rename)|\.(replace|rename|write_text|write_bytes)\s*\()|(?-u:\b)open\s*\([^)]*,\s*(?:mode\s*=\s*)?['"][wx]"#)
});
// git-Befehle, die Änderungen im Ordner verwerfen oder Dateien löschen
static GIT_DISCARD: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?i)^\s*git\s+(?:-c\s+\S+\s+)*(?:reset\s+.*--hard|clean(?-u:\b)|checkout\s+(?:.*\s)?(?:--|\.)(?:\s|$)|restore(?-u:\b)|rm(?-u:\b)|stash\s+(?:drop|clear)(?-u:\b))")
});
static SWITCH_WORD: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^[a-z][A-Za-z0-9_.-]*/"));
static EXE_SUFFIX: LazyLock<Regex> = LazyLock::new(|| re(r"\.(exe|cmd|bat|com)$"));
static IF_COND: LazyLock<Regex> = LazyLock::new(|| {
    re(r#"(?i)^(?:(?:exist|errorlevel|defined)\s+(?:"[^"]*"|\S+)|(?:"[^"]*"|\S+?)\s*==\s*(?:"[^"]*"|\S+)|(?:"[^"]*"|\S+)\s+(?:equ|neq|lss|leq|gtr|geq)\s+(?:"[^"]*"|\S+))\s*"#)
});
static IF_I: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^/i\s+"));
static IF_NOT: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^not\s+"));
static NULL_TARGET: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)^(nul|con|\$null|/dev/null)$"));
static FOR_DO: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)(?-u:\b)do\s+(.*)$"));
static START_PREFIX: LazyLock<Regex> = LazyLock::new(|| re(r#"^\s*(?:"[^"]*"\s*)?(?:/\w+\s*)*"#));
static FIND_DELETE: LazyLock<Regex> = LazyLock::new(|| re(r"\s-delete(?-u:\b)"));
static ROBO_DEL: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)/(mir|purge|mov|move)(?-u:\b)"));
static COPY_FORCE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(^|\s)(/y|-force|-f)(?-u:\b)"));
static ENCODED: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(^|\s)-(e|ec|enc|encodedcommand)\s"));
static CMD_ARG: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)(?:^|\s)/[ck]\s*(.*)$"));
static PS_ARG: LazyLock<Regex> = LazyLock::new(|| re(r"(?is)(?:^|\s)-(?:c|command)\s+(.*)$"));
static PS_FILE: LazyLock<Regex> = LazyLock::new(|| re(r"(?i)(^|\s)-(f|file)\s"));
static CODE_FLAG: LazyLock<Regex> = LazyLock::new(|| re(r"(^|\s)-[ce]\s"));
static SPAWNS: LazyLock<Regex> = LazyLock::new(|| re(r"(?-u:\b)(os\.system|os\.popen|subprocess|execSync|spawnSync|exec\()"));
static CODE_START: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)^.*?(?:^|\s)-[ce]\s+"));
static LEADING_SLASH: LazyLock<Regex> = LazyLock::new(|| re(r"^[\\/]"));

// Pfade, die aus dem Arbeitsordner hinausführen: Laufwerk (C:\…), UNC, «..» als Teil eines Pfads, Pfade ab der
// Wurzel (\Windows, /c/Users), ~ und Umgebungsvariablen, die auf Orte ausserhalb zeigen
const ENV_OUTSIDE: &str = r"userprofile|appdata|localappdata|homepath|homedrive|homeshare|windir|systemroot|systemdrive|programfiles(?:\(x86\))?|programw6432|programdata|commonprogramfiles(?:\(x86\))?|allusersprofile|public|temp|tmp|onedrive[A-Za-z0-9_]*|userdomain_roamingprofile";
static OUTSIDE: LazyLock<Regex> = LazyLock::new(|| {
    let parts = [
        // «\(?![\\"])\S» aus dem Original: ein Rückstrich, danach ein Zeichen, das weder Leerraum noch \ noch " ist
        r#"(^|[\s"'=(<>,;])(?:[a-z]:(?:[\\/]|$|[\s"'])|\\\\[^\\\s]+|\\[^\s\\"]|/[a-z]/[A-Za-z0-9_.-]{2,}|~(?:[\\/]|$|[\s"']))"#.to_string(),
        r#"(^|[\s"'=(<>\\/,;])\.\.(?:[\\/]|$|[\s"';&|)])"#.to_string(),
        format!("%(?:{ENV_OUTSIDE})%"),
        format!(r"\$env:(?:{ENV_OUTSIDE})(?-u:\b)"),
        r"\$(?:home|profile|pshome)(?-u:\b)|\$\{env:".to_string(),
        format!(r#"expanduser\s*\(|Path\.home\s*\(|getenv\s*\(\s*['"](?:{ENV_OUTSIDE})['"]"#),
    ];
    re(&format!("(?i){}", parts.join("|")))
});

/// Leerraum wie `\s` in JavaScript (zusätzlich die Bytemarke U+FEFF)
fn is_space(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

fn trim_js(s: &str) -> &str {
    s.trim_matches(is_space)
}

/// Teil nach dem Trimmen und ohne führendes «@» (`echo off` mit `@echo off`)
fn trimmed_part(part: &str) -> &str {
    let s = trim_js(part);
    s.strip_prefix('@').unwrap_or(s)
}

/// Erstes Wort eines Befehlsteils: «del/q» und «rd/s/q» sind gültiges cmd, der Schalter gehört nicht zum Wort
fn first_word(part: &str) -> String {
    let s = trimmed_part(part);
    let raw = match s.strip_prefix('"') {
        Some(r) => r.split('"').next().unwrap_or(""),
        None => s.split(is_space).next().unwrap_or(""),
    };
    let word = if SWITCH_WORD.is_match(raw) {
        raw.split('/').next().unwrap_or("")
    } else {
        raw.rsplit(['\\', '/']).next().unwrap_or("")
    };
    let lower = word.to_lowercase();
    EXE_SUFFIX.replace(&lower, "").to_string()
}

/// Rest nach dem ersten Wort (immer ein Ende von `part`)
fn after_first(part: &str) -> &str {
    let s = trimmed_part(part);
    if let Some(inner) = s.strip_prefix('"') {
        return match inner.find('"') {
            Some(i) => &inner[i + 1..],
            None => "",
        };
    }
    let mut chars = s.char_indices();
    let Some((_, c0)) = chars.next() else { return s };
    if is_space(c0) {
        return s;
    }
    let mut end = c0.len_utf8();
    for (i, c) in chars {
        if c == '/' || is_space(c) {
            break;
        }
        end = i + c.len_utf8();
    }
    &s[end..]
}

fn unquote(s: &str) -> String {
    let mut t = trim_js(s).to_string();
    for q in ['"', '\''] {
        if t.chars().count() >= 2 && t.starts_with(q) && t.ends_with(q) {
            t = t[1..t.len() - 1].to_string();
        }
    }
    t
}

/// «if exist a del a», «if not errorlevel 1 …», «if a==b …»: Bedingung überspringen, Befehl prüfen
fn after_if(rest: &str) -> String {
    let t = trim_js(rest);
    let t = IF_I.replace(t, "");
    let t = IF_NOT.replace(&t, "");
    match IF_COND.find(&t) {
        Some(m) => t[m.end()..].to_string(),
        None => String::new(),
    }
}

/// Ziele von «> datei» (nicht «>>», nicht «2>&1»). Handarbeit, weil der Ausdruck im Original
/// Rückblick und Vorausschau braucht, die die Regex-Bibliothek nicht kennt:
/// `(?<![>\d&])\d?>(?![>&])\s*("[^"]+"|[^\s&|<>()]+)`
fn redirect_targets(text: &str) -> Vec<String> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let prev = if i > 0 { Some(c[i - 1]) } else { None };
        let look_behind_ok = !prev.is_some_and(|p| p == '>' || p == '&' || p.is_ascii_digit());
        let mut gt = None;
        if look_behind_ok {
            if c[i].is_ascii_digit() && c.get(i + 1) == Some(&'>') {
                gt = Some(i + 1);
            } else if c[i] == '>' {
                gt = Some(i);
            }
        }
        let Some(gt) = gt else {
            i += 1;
            continue;
        };
        if matches!(c.get(gt + 1), Some('>') | Some('&')) {
            i += 1;
            continue;
        }
        let mut j = gt + 1;
        while j < c.len() && is_space(c[j]) {
            j += 1;
        }
        let mut matched: Option<(String, usize)> = None;
        if j < c.len() && c[j] == '"' {
            // "[^"]+": mindestens ein Zeichen bis zum nächsten Anführungszeichen
            if let Some(off) = c[j + 1..].iter().position(|&x| x == '"') {
                if off >= 1 {
                    let k = j + 1 + off;
                    matched = Some((c[j..=k].iter().collect(), k + 1));
                }
            }
        }
        if matched.is_none() {
            let mut k = j;
            while k < c.len() && !is_space(c[k]) && !matches!(c[k], '&' | '|' | '<' | '>' | '(' | ')') {
                k += 1;
            }
            if k > j {
                matched = Some((c[j..k].iter().collect(), k));
            }
        }
        match matched {
            Some((target, end)) => {
                out.push(target);
                i = end;
            }
            None => i += 1,
        }
    }
    out
}

/// Zeichenketten in Anführungszeichen: `(["'])((?:(?!\1).)+)\1` (ohne Zeilenumbruch darin)
fn quoted_strings(code: &str) -> Vec<String> {
    let c: Vec<char> = code.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        let q = c[i];
        if q == '"' || q == '\'' {
            let mut j = i + 1;
            while j < c.len() && c[j] != q && !matches!(c[j], '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                j += 1;
            }
            if j < c.len() && c[j] == q && j > i + 1 {
                out.push(c[i + 1..j].iter().collect());
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Ergebnis von `analyze`: Was der Befehl tut, soweit er sich lesen lässt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// löscht Dateien oder Ordner
    pub del: bool,
    /// verschiebt oder überschreibt Dateien
    pub mv: bool,
    /// verschlüsselt oder zu stark verschachtelt
    pub unclear: bool,
    /// Ziele von «>»
    pub writes: Vec<String>,
}

impl Found {
    fn merge(&mut self, f: Found) {
        self.del |= f.del;
        self.mv |= f.mv;
        self.unclear |= f.unclear;
        self.writes.extend(f.writes);
    }
}

/// Befehl in Teile zerlegen und jeden Teil untersuchen.
pub fn analyze(command: &str, depth: u32) -> Found {
    let mut found = Found::default();
    let text = command;
    if DELETE_CALLS.is_match(text) {
        found.del = true;
    }
    if MOVE_CALLS.is_match(text) {
        found.mv = true;
    }
    if depth > 3 {
        found.unclear = true;
        return found;
    }
    // Umleitung in eine Datei: «> datei» überschreibt sie (nicht «>>», nicht «2>&1», nicht «> nul»)
    for raw in redirect_targets(text) {
        let target = unquote(&raw);
        if !NULL_TARGET.is_match(&target) {
            found.writes.push(target);
        }
    }
    for part in text.split(['&', '|', ';', '(', ')', '\n']) {
        let word = first_word(part);
        if word.is_empty() {
            continue;
        }
        let mut rest = after_first(part);
        if word == "if" {
            found.merge(analyze(&after_if(rest), depth + 1));
            continue;
        }
        if word == "for" {
            if let Some(m) = FOR_DO.captures(rest) {
                found.merge(analyze(&m[1], depth + 1));
            }
            continue;
        }
        if PASS_THROUGH.contains(&word.as_str()) {
            let owned;
            if word == "start" {
                // Fenstertitel und Schalter
                owned = START_PREFIX.replace(rest, "").to_string();
                rest = &owned;
            }
            found.merge(analyze(rest, depth + 1));
            continue;
        }
        if DELETE_WORDS.contains(&word.as_str()) || (word == "find" && FIND_DELETE.is_match(rest)) {
            found.del = true;
        }
        if MOVE_WORDS.contains(&word.as_str()) {
            found.mv = true;
        }
        if word == "robocopy" && ROBO_DEL.is_match(rest) {
            found.del = true;
        }
        if matches!(word.as_str(), "copy" | "xcopy" | "copy-item" | "cp" | "cpi") && COPY_FORCE.is_match(rest) {
            found.mv = true;
        }
        if word == "git" && GIT_DISCARD.is_match(trimmed_part(part)) {
            found.del = true;
        }
        // Der Teil wurde auch an Zeichen in Anführungszeichen getrennt: für Shells und Einzeiler den ganzen Rest der Zeile nehmen
        let start = text.find(part).unwrap_or(0) + part.len() - rest.len();
        let whole = &text[start..];
        let padded = format!(" {rest} ");
        if SHELLS.contains(&word.as_str()) {
            if ENCODED.is_match(&padded) {
                found.unclear = true;
                continue;
            }
            let m = if word == "cmd" { CMD_ARG.captures(whole) } else { PS_ARG.captures(whole) };
            if let Some(m) = m {
                found.merge(analyze(&unquote(&m[1]), depth + 1));
            } else if word != "cmd" && PS_FILE.is_match(&padded) {
                continue; // Skriptdatei: wie python skript.py
            }
        }
        if matches!(word.as_str(), "python" | "python3" | "py" | "node") && CODE_FLAG.is_match(&padded) && SPAWNS.is_match(whole) {
            // Befehle, die der Einzeiler selbst startet: Zeichenketten im Code wie Befehle prüfen
            let code = unquote(&CODE_START.replace(whole, ""));
            for s in quoted_strings(&code) {
                found.merge(analyze(&s, depth + 1));
            }
        }
    }
    found
}

/// Erstes Wort jedes Befehlsteils (getrennt durch &, |, ;, Klammern, Zeilenumbruch)
pub fn command_words(command: &str) -> Vec<String> {
    command.split(['&', '|', ';', '(', ')', '\n']).map(first_word).filter(|w| !w.is_empty()).collect()
}

fn shorten(text: &str, max: usize) -> String {
    let collapsed = text.split(is_space).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let t = collapsed.trim();
    if t.chars().count() > max {
        format!("{} …", crate::util::take_chars(t, max))
    } else {
        t.to_string()
    }
}

/// Antwort von `assess_risk`: Was die Person gefragt wird.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Risk {
    pub kind: String,
    pub title: String,
    pub detail: String,
    pub reason: String,
}

impl Risk {
    fn new(kind: &str, title: impl Into<String>, detail: impl Into<String>, reason: &str) -> Self {
        Risk { kind: kind.to_string(), title: title.into(), detail: detail.into(), reason: reason.to_string() }
    }

    /// Wie das Objekt `{ kind, title, detail, reason }` des Originals
    pub fn to_json(&self) -> Value {
        serde_json::json!({ "kind": self.kind, "title": self.title, "detail": self.detail, "reason": self.reason })
    }
}

/// Zusammenhang eines Werkzeugaufrufs: Stufe, ob eine Datei schon da ist und ob das Modell sie in
/// diesem Auftrag selbst erstellt hat.
pub struct RiskCtx<'a> {
    pub mode: &'a str,
    pub exists: &'a dyn Fn(&str) -> bool,
    pub made_this_turn: &'a dyn Fn(&str) -> bool,
}

impl<'a> RiskCtx<'a> {
    /// Nur eine Stufe, keine Datei ist vorhanden (wie `{ mode }` im Original)
    pub fn mode_only(mode: &'a str) -> Self {
        RiskCtx { mode, exists: &|_| false, made_this_turn: &|_| false }
    }
}

/// `String(args[key] || '')`
fn text_arg(args: &Value, key: &str) -> String {
    match args.get(key) {
        Some(v) if crate::util::truthy(v) => crate::util::js_string(v),
        _ => String::new(),
    }
}

/// Braucht dieser Werkzeugaufruf eine Rückfrage? Im Zweifel fragt die App.
pub fn assess_risk(name: &str, args: &Value, ctx: &RiskCtx) -> Option<Risk> {
    let args = if args.is_object() { args } else { &Value::Null };
    let mode = if MODES.contains(&ctx.mode) { ctx.mode } else { "heikel" };
    if mode == "aus" {
        return None;
    }
    // server_starten (ENTSCHEIDE E48) startet einen Befehl wie run_command, er läuft nur weiter
    if name == "run_command" || name == "server_starten" {
        let command = text_arg(args, if name == "server_starten" { "befehl" } else { "command" });
        let found = analyze(&command, 0);
        if found.del {
            return Some(Risk::new("loeschen", "Darf ASK Dateien löschen?", shorten(&command, 300),
                "Sie kommen in die Sicherung. «Rückgängig» holt sie zurück."));
        }
        if OUTSIDE.is_match(&crate::vorlagen::without_templates(&command)) {
            return Some(Risk::new("ausserhalb", "Befehl mit Pfad ausserhalb des Ordners", shorten(&command, 300),
                "Der Befehl nennt einen Ort ausserhalb des Projekts."));
        }
        if found.unclear {
            return Some(Risk::new("unklar", "Befehl, den ASK nicht lesen kann", shorten(&command, 300),
                "Der Befehl ist verschlüsselt oder stark verschachtelt. Was er tut, lässt sich nicht prüfen."));
        }
        let overwritten: Vec<&String> = found.writes.iter().filter(|t| (ctx.exists)(t) && !(ctx.made_this_turn)(t)).collect();
        if found.mv || !overwritten.is_empty() {
            let title = match overwritten.first() {
                Some(first) => format!("Darf ASK «{first}» überschreiben?"),
                None => "Darf ASK Dateien verschieben oder überschreiben?".to_string(),
            };
            return Some(Risk::new("verschieben", title, shorten(&command, 300),
                "Der alte Stand kommt in die Sicherung. «Rückgängig» holt ihn zurück."));
        }
        if mode == "alle" {
            let title = if name == "server_starten" { "Server starten?" } else { "Befehl ausführen?" };
            return Some(Risk::new("befehl", title, shorten(&command, 300), ""));
        }
        return None;
    }
    // Docs (ENTSCHEIDE E81) ändern wie edit_file: im Projekt, mit Sicherung; nur im Modus «Nachfragen» eine Rückfrage
    if name == "doc_bearbeiten" {
        let Ok(rel) = crate::docs::doc_rel(&text_arg(args, "pfad")) else { return None };
        return assess_risk("edit_file", &serde_json::json!({ "path": rel }), ctx);
    }
    if name == "write_file" || name == "edit_file" {
        let rel = text_arg(args, "path");
        if rel.is_empty() {
            return None;
        }
        if OUTSIDE.is_match(&format!(" {rel}")) || LEADING_SLASH.is_match(&rel) {
            return Some(Risk::new("ausserhalb", "Datei ausserhalb des Ordners?", rel, "Der Pfad liegt ausserhalb des Projekts."));
        }
        if !(ctx.exists)(&rel) || (ctx.made_this_turn)(&rel) {
            return None;
        }
        if name == "write_file" {
            return Some(Risk::new("ueberschreiben", format!("Darf ASK «{rel}» überschreiben?"), rel.clone(),
                "Der alte Stand kommt in die Sicherung."));
        }
        if mode == "alle" {
            return Some(Risk::new("aendern", format!("Darf ASK «{rel}» ändern?"), rel, ""));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cmd(command: &str, mode: &str) -> Option<Risk> {
        assess_risk("run_command", &json!({ "command": command }), &RiskCtx::mode_only(mode))
    }
    fn kind(r: Option<Risk>) -> Option<String> {
        r.map(|r| r.kind)
    }
    fn heikel(c: &str) -> Option<String> {
        kind(cmd(c, "heikel"))
    }

    #[test]
    fn loeschbefehle_werden_erkannt() {
        for c in ["del alt.txt", "echo x & del /q *.tmp", "rmdir /s /q build", "rd build", "rm -rf out", "Remove-Item a.txt",
            "powershell -c \"Remove-Item a.txt\"", "python -c \"import os; os.remove('a.txt')\"", "python -c \"import shutil; shutil.rmtree('x')\"",
            "\"C:\\Windows\\System32\\del.exe\" x"] {
            assert_eq!(heikel(c).as_deref(), Some("loeschen"), "{c}");
        }
        for c in ["python auswertung.py", "dir", "echo deleted", "type readme.md", "python -c \"print(1)\"", "git status"] {
            assert_eq!(heikel(c), None, "{c}");
        }
    }

    #[test]
    fn pfade_ausserhalb_des_ordners() {
        for c in ["type C:\\Users\\x\\geheim.txt", "copy a.txt ..\\b.txt", "dir %USERPROFILE%", "type \\\\server\\share\\a"] {
            assert_eq!(heikel(c).as_deref(), Some("ausserhalb"), "{c}");
        }
        assert_eq!(heikel("python skript.py --out ergebnis.xlsx"), None);
    }

    #[test]
    fn stufen_aus_und_alle() {
        assert!(cmd("del a.txt", "aus").is_none());
        assert_eq!(kind(cmd("python x.py", "alle")).as_deref(), Some("befehl"));
        assert!(cmd("python x.py", "unbekannt").is_none()); // unbekannt gilt als «heikel»
    }

    #[test]
    fn ueberschreiben_vorhandener_dateien_ausser_selbst_erstellt() {
        let run = |name: &str, path: &str, exists: bool, made: bool, mode: &str| {
            let ex = move |_: &str| exists;
            let mk = move |_: &str| made;
            kind(assess_risk(name, &json!({ "path": path }), &RiskCtx { mode, exists: &ex, made_this_turn: &mk }))
        };
        assert_eq!(run("write_file", "bericht.docx", true, false, "heikel").as_deref(), Some("ueberschreiben"));
        assert_eq!(run("write_file", "neu.txt", false, false, "heikel"), None);
        assert_eq!(run("write_file", "neu.txt", true, true, "heikel"), None);
        assert_eq!(run("edit_file", "a.py", true, false, "heikel"), None);
        assert_eq!(run("edit_file", "a.py", true, false, "alle").as_deref(), Some("aendern"));
        assert_eq!(run("write_file", "C:\\temp\\x.txt", false, false, "heikel").as_deref(), Some("ausserhalb"));
        assert_eq!(run("read_file", "a.txt", true, false, "alle"), None);
    }

    #[test]
    fn command_words_trennt_befehle() {
        assert_eq!(command_words("@echo off && del \"a b.txt\" | more"), vec!["echo", "del", "more"]);
    }

    #[test]
    fn server_starten_wird_wie_ein_befehl_geprueft() {
        let s = |befehl: &str, mode: &str| assess_risk("server_starten", &json!({ "befehl": befehl }), &RiskCtx::mode_only(mode));
        assert!(s("npm run dev", "heikel").is_none());
        assert_eq!(kind(s("del /q dist & npm run dev", "heikel")).as_deref(), Some("loeschen"));
        assert_eq!(kind(s("python -m http.server -d C:\\Users", "heikel")).as_deref(), Some("ausserhalb"));
        assert_eq!(s("npm run dev", "alle").map(|r| r.title).as_deref(), Some("Server starten?"));
    }

    #[test]
    fn loeschen_in_schaltern_wrappern_bedingungen_schleifen_und_git() {
        for c in ["del/q f", "rd/s/q src", "rmdir/s x", "cmd /c del x", "cmd /c \"del x\"", "powershell -c \"ri x\"", "pwsh -Command Remove-Item x",
            "if exist a del a", "if not exist b (rd /s /q c)", "for %f in (*.tmp) do del %f", "start \"\" cmd /c del x", "call del x", "find . -delete",
            "git clean -fdx", "git reset --hard", "git reset --hard HEAD~1", "git checkout -- .", "git checkout .", "git restore a.py", "git rm a.py",
            "robocopy a b /mir", "robocopy a b /move", "python -c \"import os; os.system('rm -r x')\"",
            "python -c \"import subprocess; subprocess.run(['cmd', '/c', 'del x'])\""] {
            assert_eq!(heikel(c).as_deref(), Some("loeschen"), "{c}");
        }
        for c in ["git status", "git checkout main", "git commit -m \"rd notes\"", "echo \"del x\"", "dir /s /b", "findstr /s foo *.py",
            "python -c \"import os; print(os.getcwd())\"", "node -e \"console.log(1)\""] {
            assert_eq!(heikel(c), None, "{c}");
        }
    }

    #[test]
    fn verschieben_ueberschreiben_und_unlesbare_befehle() {
        for c in ["move a b", "mv a b", "copy /y a b", "xcopy /y a b", "robocopy a b", "Move-Item a b",
            "python -c \"open('a.txt', 'w').write('x')\"", "python -c \"import shutil; shutil.move('a', 'b')\""] {
            assert_eq!(heikel(c).as_deref(), Some("verschieben"), "{c}");
        }
        assert_eq!(heikel("pwsh -enc SQBFAFgA").as_deref(), Some("unklar"));
        // Umleitung: nur, wenn die Datei schon da ist und das Modell sie nicht selbst geschrieben hat
        let run = |c: &str, exists: bool, made: bool| {
            let ex = move |p: &str| exists && p == "main.py";
            let mk = move |_: &str| made;
            kind(assess_risk("run_command", &json!({ "command": c }), &RiskCtx { mode: "heikel", exists: &ex, made_this_turn: &mk }))
        };
        assert_eq!(run("echo x > main.py", true, false).as_deref(), Some("verschieben"));
        assert_eq!(run("echo x > main.py", false, false), None);
        assert_eq!(run("echo x > main.py", true, true), None);
        for c in ["echo x >> main.py", "python x.py 2>&1", "python x.py > nul", "copy a.txt b.txt"] {
            assert_eq!(run(c, true, false), None, "{c}");
        }
    }

    #[test]
    fn weitere_pfade_ausserhalb_des_ordners() {
        for c in ["type sub\\..\\..\\x", "echo hi >C:\\Users\\a\\x", "cd ..", "cd .. && dir", "type \\Windows\\win.ini", "ls /c/Users/x",
            "cat $HOME/x", "dir %ProgramData%", "dir %HOMEDRIVE%", "dir %OneDrive%", "type $env:APPDATA\\x", "cd ~", "dir C:",
            "python -c \"import os; print(os.path.expanduser('~'))\""] {
            assert_eq!(heikel(c).as_deref(), Some("ausserhalb"), "{c}");
        }
        for c in ["rd /s/q build", "python skript.py --out ergebnis.xlsx", "echo %cd%", "dir /s/b"] {
            assert_ne!(heikel(c).as_deref(), Some("ausserhalb"), "{c}");
        }
        let w = |p: &str| kind(assess_risk("write_file", &json!({ "path": p }), &RiskCtx::mode_only("heikel")));
        assert_eq!(w("sub/../../x").as_deref(), Some("ausserhalb"));
        assert_eq!(w("/etc/x").as_deref(), Some("ausserhalb"));
        assert_eq!(w("\\Windows\\x").as_deref(), Some("ausserhalb"));
        assert_eq!(w("ordner/a.txt"), None);
    }

    #[test]
    fn argumente_null_oder_kein_objekt_werfen_nicht() {
        assert!(assess_risk("run_command", &Value::Null, &RiskCtx::mode_only("heikel")).is_none());
        assert!(assess_risk("write_file", &json!("x"), &RiskCtx::mode_only("heikel")).is_none());
    }
}
