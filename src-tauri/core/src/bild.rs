//! Bilder mit Flux (ENTSCHEIDE E31). Flux ist in ASK ein Server-Werkzeug («Flux Bild», Funktion
//! bild_erstellen). Die App kann es nicht selbst ausführen: Sie schickt ASK eine eigene, kurze Anfrage
//! mit tool_ids, so wie die ASK-Webseite, und holt danach die Bilder ab. Getrennt vom normalen Chat,
//! weil sich die Werkzeuge der App und die Server-Werkzeuge in einer Anfrage in die Quere kommen.
//!
//! Beim Bau liess sich Flux nicht ausprobieren (nur im Beta-Arbeitsbereich). Deshalb fragt die App wie
//! die Webseite in einem vorübergehenden Chat in ASK an (dort legt ASK die Dateien ab, die das Werkzeug
//! meldet) und sucht die Bilder an allen üblichen Stellen: im gestreamten Ergebnis, in den Dateien der
//! Antwort, als Bild-Link oder eingebettet im Text. Der vorübergehende Chat wird danach gelöscht.
//! Kommt kein Bild, legt die App die Antwort von ASK unter .ask/arbeit/<Chat>/ ab, damit sich der Weg
//! anpassen lässt.
//!
//! Portiert aus src/main/bild.js. Öffentliche Schnittstelle:
//! - `Images::new(ask, get_model)` mit `with_client` (für Bilder auf anderen Servern, JS `fetchImpl`),
//!   `with_allow_local`, `with_resize` (Standard `attachments::native_resize`), `with_poll_ms`
//! - `available`, `discover`, `forget`, `origin`, `create(root, args, CreateOptions) -> CreateResult`
//! - freie Funktionen `flux_arguments`, `find_image_refs`, `image_type`, `save_names`, `plain_text`, `slug`
//! - Konstanten `FUNCTION`, `FORMATS`, `QUALITIES`
//!
//! Unterschiede zu JS: `AbortController` ist ein `Signal` (`CancellationToken`); die Anfragen an ASK laufen
//! über `AskSession::raw`; ein Abbruch ist ein `AskError::abort()` (erkennbar mit `askapi::is_abort`).

#![allow(clippy::regex_creation_in_loops)] // `re!` legt jeden Ausdruck nur einmal an (OnceLock)

use crate::askapi::{AskError, AskSession, RequestOptions, Signal, SseParser, Utf8Stream};
use crate::attachments::{image_url, native_resize, Resized};
use crate::util::{js_string, take_chars, truthy};
use crate::web::{check_url, decode_entities};
use anyhow::{anyhow, bail, Result};
use futures::StreamExt;
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{json, Map, Value};
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

macro_rules! re {
    ($p:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($p).expect("regex"))
    }};
}

pub const FUNCTION: &str = "bild_erstellen";
pub const FORMATS: [&str; 6] = ["quadrat", "hochformat", "querformat", "breitbild", "handy", "panorama"];
pub const QUALITIES: [&str; 3] = ["schnell", "hoch", "maximal"];
const MAX_IMAGES: usize = 8;
const MAX_BYTES: usize = 40 * 1024 * 1024;
const TIME_LIMIT_MS: u64 = 5 * 60 * 1000;
const POLL_MS: u64 = 1500;
/// Bild da, Antworttext noch nicht: so lange höchstens noch warten
const AFTER_IMAGE_MS: i64 = 20000;
const TEMPLATE_TYPES: [&str; 5] = [".png", ".jpg", ".jpeg", ".webp", ".gif"];

const SYSTEM: &str = "Du erstellst Bilder für die App ASK Desktop. Rufe das Werkzeug bild_erstellen genau einmal auf, \
mit genau den Argumenten aus der Nachricht, ohne etwas wegzulassen oder hinzuzufügen. \
Gib danach das Ergebnis des Werkzeugs weiter, mit allen Bild-Links, und schreib höchstens einen kurzen Satz dazu.";

/// Das Flux-Werkzeug in ASK
#[derive(Clone, Debug, PartialEq)]
pub struct FluxTool {
    pub id: String,
    pub name: String,
}

// ---------- Argumente ----------

/// `Number(v)` wie in JavaScript (NaN, wenn keine Zahl)
fn js_number(v: Option<&Value>) -> f64 {
    match v {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(Value::String(s)) => {
            let t = s.trim();
            if t.is_empty() { 0.0 } else { t.parse::<f64>().unwrap_or(f64::NAN) }
        }
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Null) => 0.0,
        _ => f64::NAN,
    }
}

fn text_of(v: Option<&Value>) -> String {
    match v {
        Some(v) if truthy(v) => js_string(v),
        _ => String::new(),
    }
}

fn is_true(v: Option<&Value>) -> bool {
    matches!(v, Some(Value::Bool(true))) || matches!(v, Some(Value::String(s)) if s == "true")
}

/// Argumente für das Flux-Werkzeug: nur, was gesetzt ist, unbekannte Werte fallen weg
pub fn flux_arguments(a: &Value, editing: bool) -> Value {
    let mut out = Map::new();
    out.insert("beschreibung".into(), json!(text_of(a.get("beschreibung")).trim()));
    let pick = |v: Option<&Value>, list: &[&str]| {
        let s = text_of(v).trim().to_lowercase();
        if list.contains(&s.as_str()) { s } else { String::new() }
    };
    if editing {
        out.insert("bearbeiten".into(), json!(true));
    }
    let format = pick(a.get("format"), &FORMATS);
    if !format.is_empty() {
        out.insert("format".into(), json!(format));
    }
    let quality = pick(a.get("qualitaet"), &QUALITIES);
    if !quality.is_empty() {
        out.insert("qualitaet".into(), json!(quality));
    }
    let stil = text_of(a.get("stil"));
    if !stil.trim().is_empty() {
        out.insert("stil".into(), json!(stil.trim()));
    }
    let count = (js_number(a.get("anzahl")) + 0.5).floor(); // Math.round
    if count > 1.0 {
        out.insert("anzahl".into(), json!(count.min(4.0) as i64));
    }
    let refs = text_of(a.get("referenzen"));
    if !refs.trim().is_empty() && !editing {
        out.insert("referenzen".into(), json!(refs.trim()));
    }
    if is_true(a.get("tf_branding")) {
        out.insert("tf_branding".into(), json!(true));
    }
    if is_true(a.get("verwandeln")) {
        out.insert("verwandeln".into(), json!(true));
    }
    Value::Object(out)
}

/// Dateityp aus den ersten Bytes; `None`, wenn es kein Bild ist
pub fn image_type(buf: &[u8]) -> Option<&'static str> {
    if buf.len() < 12 {
        return None;
    }
    if buf[0] == 0x89 && &buf[1..4] == b"PNG" {
        return Some(".png");
    }
    if buf[0] == 0xff && buf[1] == 0xd8 && buf[2] == 0xff {
        return Some(".jpg");
    }
    if &buf[0..4] == b"RIFF" && &buf[8..12] == b"WEBP" {
        return Some(".webp");
    }
    if (&buf[0..5] == b"GIF87" || &buf[0..5] == b"GIF89") && buf[5] == b'a' {
        return Some(".gif");
    }
    None
}

// ---------- Bilder in Antworten finden ----------

fn data_re() -> &'static Regex {
    re!(r"data:image/(?:png|jpe?g|webp|gif);base64,[A-Za-z0-9+/=]{100,}")
}
fn ask_re() -> &'static Regex {
    re!(r#"(https?://[^\s"'<>()\\/]+)?/(api/v1/files/[A-Za-z0-9_-]+(?:/content)?|cache/image/[^\s"'<>()\\?#]+)"#)
}
fn md_re() -> &'static Regex {
    re!(r#"!\[[^\]]*\]\(\s*<?([^\s)>]+)>?(?:\s+"[^"]*")?\s*\)"#)
}
fn web_re() -> &'static Regex {
    re!(r#"(?i)https?://[^\s"'<>()\\]+\.(?:png|jpe?g|webp|gif)(?:\?[^\s"'<>()\\]*)?"#)
}
/// Referenzfotos des Werkzeugs, nicht das Ergebnis
fn reference_host(host: &str) -> bool {
    re!(r"(?i)(^|\.)(wikipedia|wikimedia)\.org$").is_match(host)
}

/// Gefundene Bilder: `strong` sind sicher ein Ergebnis (Dateien von ASK, eingebettete Bilder, Dateien vom Typ
/// Bild); `weak` sind Bild-Links auf andere Server, nur wenn es sonst nichts gibt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub strong: Vec<String>,
    pub weak: Vec<String>,
}

#[derive(Clone, Copy)]
enum List {
    Strong,
    Weak,
}

impl Found {
    fn add(&mut self, list: List, r: &str) {
        if r.is_empty() || self.strong.iter().any(|x| x == r) {
            return;
        }
        let target = match list {
            List::Strong => &mut self.strong,
            List::Weak => &mut self.weak,
        };
        if !target.iter().any(|x| x == r) {
            target.push(r.to_string());
        }
    }

    fn web(&mut self, url: &str, list: List) {
        if let Ok(u) = url::Url::parse(url) {
            if matches!(u.scheme(), "http" | "https") && !reference_host(u.host_str().unwrap_or("")) {
                self.add(list, u.as_str());
            }
        }
    }
}

/// `p.replace(/^api\/v1\/files\/([\w-]+)(?:\/content)?$/, 'api/v1/files/$1/content')`, davor `lead`
fn files_content(p: &str, lead: &str) -> String {
    let re = re!(r"^api/v1/files/([A-Za-z0-9_-]+)(?:/content)?$");
    match re.captures(p) {
        Some(m) => format!("{lead}api/v1/files/{}/content", &m[1]),
        None => format!("{lead}{p}"),
    }
}

fn from_text(found: &mut Found, raw: &str, origin: &str) {
    let text = decode_entities(raw);
    for m in data_re().find_iter(&text) {
        found.add(List::Strong, m.as_str());
    }
    for m in ask_re().captures_iter(&text) {
        let host = m.get(1).map(|h| h.as_str());
        if host.is_none() || host == Some(origin) {
            found.add(List::Strong, &files_content(&m[2], "/"));
        }
    }
    let prefix = format!("{origin}/");
    for m in md_re().captures_iter(&text) {
        let link = &m[1];
        if re!(r"(?i)^https?:").is_match(link) && !link.starts_with(&prefix) {
            found.web(link, List::Weak);
        }
    }
    for m in web_re().find_iter(&text) {
        if !m.as_str().starts_with(&prefix) {
            found.web(m.as_str(), List::Weak);
        }
    }
}

fn walk(found: &mut Found, v: &Value, origin: &str, depth: usize) {
    if depth > 12 {
        return;
    }
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
        Value::String(s) => {
            if s.chars().count() > 30 {
                from_text(found, s, origin);
            }
        }
        Value::Array(a) => {
            for x in a {
                walk(found, x, origin, depth + 1);
            }
        }
        Value::Object(o) => {
            // Datei vom Typ Bild, wie Werkzeuge sie über das Ereignis «files» melden
            let kind = [v.get("type"), v.get("content_type"), v.get("meta").and_then(|m| m.get("content_type"))]
                .into_iter()
                .flatten()
                .find(|x| truthy(x))
                .map(js_string)
                .unwrap_or_default();
            if kind.to_lowercase().contains("image") {
                let url = match v.get("url") {
                    Some(Value::String(s)) => s.clone(),
                    _ => match v.get("image_url").and_then(|i| i.get("url")) {
                        Some(Value::String(s)) => s.clone(),
                        _ => String::new(),
                    },
                };
                let prefix = format!("{origin}/");
                if url.starts_with("data:image/") {
                    found.add(List::Strong, &url);
                } else if re!(r"(?i)^https?:").is_match(&url) && !url.starts_with(&prefix) {
                    found.web(&url, List::Strong);
                } else if !url.is_empty() {
                    let rel = if let Some(rest) = url.strip_prefix(origin) {
                        rest.to_string()
                    } else if url.starts_with('/') {
                        url.clone()
                    } else {
                        format!("/{url}")
                    };
                    let rel = match rel.strip_prefix('/') {
                        Some(inner) => files_content(inner, "/"),
                        None => rel,
                    };
                    found.add(List::Strong, &rel);
                } else if let Some(Value::String(id)) = v.get("id") {
                    if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                        found.add(List::Strong, &format!("/api/v1/files/{id}/content"));
                    }
                }
            }
            for x in o.values() {
                walk(found, x, origin, depth + 1);
            }
        }
    }
}

/// Sucht Bilder in einer Antwort von ASK (beliebiges JSON oder Text). `origin`: Adresse von ASK.
pub fn find_image_refs(value: &Value, origin: &str, found: &mut Found) {
    walk(found, value, origin, 0);
}

/// Lesbarer Antworttext ohne Bilder, Werkzeugblöcke und Markup, gekürzt
pub fn plain_text(content: &str) -> String {
    let t = re!(r"(?is)<details.*?</details>").replace_all(content, "");
    let t = md_re().replace_all(&t, "");
    let t = data_re().replace_all(&t, "");
    let t = re!(r"<[^>]+>").replace_all(&t, "");
    let t = decode_entities(&t);
    let t = re!(r"\n{3,}").replace_all(&t, "\n\n");
    take_chars(t.trim(), 600)
}

pub fn slug(text: &str) -> String {
    let s = text
        .to_lowercase()
        .replace('ä', "ae")
        .replace('ö', "oe")
        .replace('ü', "ue")
        .replace('ß', "ss");
    let s = re!(r"[^a-z0-9]+").replace_all(&s, "-");
    let s = s.trim_matches('-');
    let s = take_chars(s, 40);
    let s = s.trim_end_matches('-');
    if s.is_empty() { "bild".to_string() } else { s.to_string() }
}

// ---------- Pfade und Namen ----------

/// `path.resolve` ohne Dateizugriff: «.» und «..» auflösen
fn normalize(p: &Path) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for c in p.components() {
        match c {
            Component::Prefix(x) => out.push(x.as_os_str().to_string_lossy().to_string()),
            Component::RootDir => out.push("/".to_string()),
            Component::CurDir => {}
            Component::ParentDir => {
                if out.len() > 1 || out.last().is_some_and(|l| l != "/") {
                    out.pop();
                }
            }
            Component::Normal(s) => out.push(s.to_string_lossy().to_string()),
        }
    }
    out
}

/// Pfad im Arbeitsordner (posix, relativ); Fehler, wenn er hinausführt
pub fn inside(root: &Path, rel: &str) -> Result<String> {
    let trimmed = rel.trim();
    let joined = if Path::new(trimmed).is_absolute() { PathBuf::from(trimmed) } else { root.join(trimmed) };
    let (full, base) = (normalize(&joined), normalize(root));
    let same = |a: &str, b: &str| if cfg!(windows) { a.to_lowercase() == b.to_lowercase() } else { a == b };
    let outside = || anyhow!("Der Pfad «{rel}» liegt nicht im Arbeitsordner.");
    if full.len() <= base.len() || !base.iter().zip(&full).all(|(a, b)| same(a, b)) {
        return Err(outside());
    }
    Ok(full[base.len()..].join("/"))
}

fn posix_ext(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 => name[i..].to_string(),
        _ => String::new(),
    }
}

fn posix_base(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn same_type(a: &str, b: &str) -> bool {
    a.to_lowercase().replace("jpeg", "jpg") == b.to_lowercase().replace("jpeg", "jpg")
}

/// Dateinamen für die Bilder, wie die App sonst im Ordner arbeitet (Rückgängig geht immer):
/// - mit «datei» genau dort, auch wenn es die Datei schon gibt
/// - beim Bearbeiten ersetzt das Ergebnis die Vorlage, ausser sie ist ein Anhang unter .ask
///   (dann kommt es nach bilder/) oder das Format ändert sich (dann daneben)
/// - sonst unter bilder/ mit einem Namen aus der Beschreibung, ohne Bestehendes zu überschreiben
pub fn save_names(root: &Path, datei: &str, beschreibung: &str, vorlage: &str, exts: &[&str]) -> Result<Vec<String>> {
    let mut dir = "bilder".to_string();
    let stem;
    let mut given = String::new(); // Endung der gewünschten Datei, wenn sie zum Bild passt
    let mut replace = false;
    let target = if !datei.trim().is_empty() {
        inside(root, datei)?
    } else if !vorlage.is_empty() && !re!(r"(?i)^\.ask/").is_match(vorlage) {
        vorlage.to_string()
    } else {
        String::new()
    };
    if !target.is_empty() {
        dir = match target.rfind('/') {
            Some(0) => "/".to_string(),
            Some(i) => target[..i].to_string(),
            None => ".".to_string(),
        };
        let name = posix_base(&target);
        given = posix_ext(name);
        stem = if !given.is_empty() && name != given { name[..name.len() - given.len()].to_string() } else { name.to_string() };
        replace = exts.len() == 1 && (!datei.trim().is_empty() || same_type(&given, exts[0]));
    } else if !vorlage.is_empty() {
        let name = posix_base(vorlage);
        let ext = posix_ext(name);
        stem = slug(if !ext.is_empty() && name != ext { &name[..name.len() - ext.len()] } else { name });
    } else {
        stem = slug(beschreibung);
    }
    let mut taken: HashSet<String> = HashSet::new();
    let join = |name: &str| if dir == "." { name.to_string() } else { format!("{dir}/{name}") };
    let exists = |rel: &str| root.join(rel.split('/').collect::<PathBuf>()).exists();
    let mut names = Vec::new();
    for (i, ext) in exts.iter().enumerate() {
        let end = if !given.is_empty() && same_type(&given, ext) { given.as_str() } else { ext };
        let base = if exts.len() > 1 { format!("{stem}-{}", i + 1) } else { stem.clone() };
        let mut rel = join(&format!("{base}{end}"));
        let mut n = 2;
        while !replace && (exists(&rel) || taken.contains(&rel)) {
            rel = join(&format!("{base}-{n}{end}"));
            n += 1;
        }
        taken.insert(rel.clone());
        names.push(rel);
    }
    Ok(names)
}

fn uuid4() -> String {
    use rand::Rng;
    let mut b: [u8; 16] = rand::rng().random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h = hex::encode(b);
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

fn base36(mut n: u64) -> String {
    if n == 0 {
        return "0".into();
    }
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let mut out = Vec::new();
    while n > 0 {
        out.push(digits[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

async fn sleep(ms: u64, signal: &Signal) -> Result<()> {
    if signal.is_cancelled() {
        return Err(AskError::abort().into());
    }
    tokio::select! {
        _ = signal.cancelled() => Err(AskError::abort().into()),
        _ = tokio::time::sleep(Duration::from_millis(ms)) => Ok(()),
    }
}

/// Bricht eine Hintergrundaufgabe beim Verlassen ab (wie `clearInterval` und `clearTimeout`)
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

// ---------- Ablaufprotokoll ----------

/// Ablauf einer Anfrage, für `.ask/arbeit/<Chat>/flux-….json`, wenn kein Bild kam
struct Trace {
    zeit: String,
    werkzeug: Value,
    argumente: Value,
    ablauf: Vec<String>,
    strom: Option<Vec<String>>,
    antwort: Option<Value>,
}

impl Trace {
    fn step(&mut self, text: impl Into<String>) {
        self.ablauf.push(text.into());
    }

    fn to_value(&self, text: &str) -> Value {
        let mut m = Map::new();
        m.insert("zeit".into(), json!(self.zeit));
        m.insert("werkzeug".into(), self.werkzeug.clone());
        m.insert("argumente".into(), self.argumente.clone());
        m.insert("ablauf".into(), json!(self.ablauf));
        if let Some(s) = &self.strom {
            m.insert("strom".into(), json!(s));
        }
        if let Some(a) = &self.antwort {
            m.insert("antwort".into(), a.clone());
        }
        m.insert("text".into(), json!(text));
        clip_data(&Value::Object(m))
    }
}

/// Eingebettete Bilder kürzen (`clipData` in JS)
fn clip_data(v: &Value) -> Value {
    match v {
        Value::String(s) if s.starts_with("data:") => json!(format!("{}… ({} Zeichen)", take_chars(s, 60), s.chars().count())),
        Value::Array(a) => Value::Array(a.iter().map(clip_data).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), clip_data(x))).collect()),
        other => other.clone(),
    }
}

// ---------- Images ----------

/// Bild zum Bearbeiten: Pfad im Ordner und das Bild als data-URL
#[derive(Clone, Debug)]
pub struct Template {
    pub path: String,
    pub url: String,
}

struct TempChat {
    id: String,
    answer: String,
}

/// Ergebnis von `request`
#[derive(Default)]
struct Run {
    content: String,
    background: bool,
    task: String,
    tool_call: bool,
    error: String,
}

/// Optionen von `create`
#[derive(Clone, Default)]
pub struct CreateOptions {
    /// Chat-Kennung für die Ablage der Antwort von ASK (Standard «chat»)
    pub chat: String,
    pub signal: Option<Signal>,
    pub on_progress: Option<ProgressFn>,
}

/// Was `create` liefert
#[derive(Clone, Debug, PartialEq)]
pub struct CreateResult {
    pub files: Vec<String>,
    pub text: String,
    pub seconds: i64,
    pub template: String,
}

/// Fortschritt (JS `onProgress(text)`)
pub type ProgressFn = Arc<dyn Fn(&str) + Send + Sync>;
pub type ModelFn = Arc<dyn Fn() -> String + Send + Sync>;
pub type ResizeImageFn = Arc<dyn Fn(&[u8], &str) -> Option<Resized> + Send + Sync>;

pub struct Images {
    ask: Arc<AskSession>,
    get_model: ModelFn,
    client: reqwest::Client,
    allow_local: bool,
    resize: Option<ResizeImageFn>,
    poll_ms: u64,
    /// { id, name } des Flux-Werkzeugs in ASK
    tool: Mutex<Option<FluxTool>>,
}

impl Images {
    /// ask: Sitzung bei ASK (`raw`, `base`); get_model: Modell der App
    pub fn new(ask: Arc<AskSession>, get_model: ModelFn) -> Self {
        Images {
            ask,
            get_model,
            client: reqwest::Client::new(),
            allow_local: false,
            resize: Some(Arc::new(native_resize)),
            poll_ms: POLL_MS,
            tool: Mutex::new(None),
        }
    }

    /// HTTP-Client für Bilder auf anderen Servern (`fetchImpl`)
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
    }

    pub fn with_allow_local(mut self, allow: bool) -> Self {
        self.allow_local = allow;
        self
    }

    pub fn with_resize(mut self, resize: Option<ResizeImageFn>) -> Self {
        self.resize = resize;
        self
    }

    pub fn with_poll_ms(mut self, ms: u64) -> Self {
        self.poll_ms = ms;
        self
    }

    pub fn available(&self) -> bool {
        self.tool.lock().is_some()
    }

    pub fn tool(&self) -> Option<FluxTool> {
        self.tool.lock().clone()
    }

    /// Bietet ASK diesem Konto das Flux-Werkzeug an? Sucht in den Werkzeugen nach bild_erstellen.
    pub async fn discover(&self) -> Option<FluxTool> {
        let found = self.find_tool().await.ok().flatten();
        *self.tool.lock() = found.clone();
        found
    }

    async fn find_tool(&self) -> Result<Option<FluxTool>> {
        let res = self.ask.raw("/api/v1/tools/", RequestOptions::default()).await?;
        if !res.status().is_success() {
            bail!("HTTP {}", res.status().as_u16());
        }
        let data: Value = res.json().await?;
        let list: Vec<Value> = match &data {
            Value::Array(a) => a.clone(),
            other => [other.get("items"), other.get("data")]
                .into_iter()
                .flatten()
                .find(|v| truthy(v))
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default(),
        };
        let by_spec = list.iter().find(|t| {
            t.get("specs").and_then(Value::as_array).is_some_and(|s| s.iter().any(|x| x.get("name").and_then(Value::as_str) == Some(FUNCTION)))
        });
        let by_name = || {
            list.iter().find(|t| {
                !t.get("specs").is_some_and(Value::is_array) && {
                    let (id, name) = (t.get("id").map(js_or_undefined), t.get("name").map(js_or_undefined));
                    re!(r"(?i)flux").is_match(&format!("{} {}", id.unwrap_or_else(|| "undefined".into()), name.unwrap_or_else(|| "undefined".into())))
                }
            })
        };
        Ok(by_spec.or_else(by_name).map(|t| {
            let id = js_string(t.get("id").unwrap_or(&Value::Null));
            let name = t.get("name").filter(|n| truthy(n)).map(js_string).unwrap_or_else(|| id.clone());
            FluxTool { id, name }
        }))
    }

    pub fn forget(&self) {
        *self.tool.lock() = None;
    }

    /// `new URL(ask.base).origin`
    pub fn origin(&self) -> String {
        url::Url::parse(&self.ask.base()).map(|u| u.origin().ascii_serialization()).unwrap_or_default()
    }

    async fn json(&self, p: &str, opts: RequestOptions) -> Result<Value> {
        let res = self.ask.raw(p, opts).await?;
        if !res.status().is_success() {
            return Err(AskError::new(format!("HTTP {}", res.status().as_u16())).with_status(res.status().as_u16()).into());
        }
        Ok(res.json().await?)
    }

    /// Vorlage zum Bearbeiten: Bild im Arbeitsordner als data-URL
    pub fn template(&self, root: &Path, rel: &str) -> Result<Template> {
        let clean = inside(root, rel)?;
        let ext = posix_ext(posix_base(&clean)).to_lowercase();
        if !TEMPLATE_TYPES.contains(&ext.as_str()) {
            bail!("Die Vorlage muss ein Bild sein (PNG, JPG, WebP oder GIF).");
        }
        if !root.join(clean.split('/').collect::<PathBuf>()).exists() {
            bail!("Die Vorlage «{clean}» gibt es nicht.");
        }
        let url = image_url(root, &clean, self.resize.as_ref().map(|r| &**r as &crate::attachments::ResizeFn))
            .ok_or_else(|| anyhow!("Die Vorlage ist zu gross oder nicht lesbar."))?;
        Ok(Template { path: clean, url })
    }

    /// Vorübergehender Chat in ASK, wie ihn die Webseite anlegt: Dort speichert ASK, was das Werkzeug
    /// meldet (Dateien, Status), auch wenn die Antwort im Hintergrund läuft.
    async fn temp_chat(&self, model: &str, text: &str, template: Option<&Template>) -> Result<TempChat> {
        let (user, answer) = (uuid4(), uuid4());
        let now = crate::util::now_ms() / 1000;
        let files = match template {
            Some(t) => json!([{ "type": "image", "url": t.url }]),
            None => json!([]),
        };
        let user_msg = json!({ "id": user, "parentId": null, "childrenIds": [answer], "role": "user", "content": text,
            "files": files, "timestamp": now, "models": [model] });
        let answer_msg = json!({ "id": answer, "parentId": user, "childrenIds": [], "role": "assistant", "content": "",
            "model": model, "modelName": model, "timestamp": now, "done": false });
        let mut messages = Map::new();
        messages.insert(user.clone(), user_msg.clone());
        messages.insert(answer.clone(), answer_msg.clone());
        let chat = json!({ "title": "ASK Desktop: Bild", "models": [model], "messages": [user_msg, answer_msg],
            "history": { "messages": messages, "currentId": answer }, "tags": [], "timestamp": crate::util::now_ms() });
        let data = self.json("/api/v1/chats/new", RequestOptions::post(json!({ "chat": chat }).to_string())).await?;
        match data.get("id") {
            Some(id) if truthy(id) => Ok(TempChat { id: js_string(id), answer }),
            _ => bail!("kein Chat"),
        }
    }

    /// (Antwort-Nachricht, andere Antworten des Assistenten)
    async fn answer_of(&self, chat: &TempChat) -> Result<(Option<Value>, Vec<Value>)> {
        let data = self.json(&format!("/api/v1/chats/{}", enc(&chat.id)), RequestOptions::default()).await?;
        let all = data.pointer("/chat/history/messages").and_then(Value::as_object).cloned().unwrap_or_default();
        let message = all.get(&chat.answer).filter(|m| truthy(m)).cloned();
        let others = all
            .values()
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant") && m.get("id").and_then(Value::as_str) != Some(chat.answer.as_str()))
            .cloned()
            .collect();
        Ok((message, others))
    }

    /// Läuft im Hintergrund noch eine Antwort in diesem Chat? `None`: weiss ASK nicht zu sagen.
    async fn running(&self, chat: &TempChat) -> Option<bool> {
        let data = self.json(&format!("/api/tasks/chat/{}", enc(&chat.id)), RequestOptions::default()).await.ok()?;
        data.get("task_ids").and_then(Value::as_array).map(|a| !a.is_empty())
    }

    /// Erstellt Bilder und speichert sie im Arbeitsordner.
    pub async fn create(&self, root: &Path, args: &Value, opts: CreateOptions) -> Result<CreateResult> {
        if self.tool().is_none() {
            self.discover().await;
        }
        let Some(tool) = self.tool() else {
            bail!("Flux ist in ASK für dein Konto nicht freigegeben, Bilder lassen sich darum nicht erstellen.");
        };
        let model = (self.get_model)();
        if model.is_empty() {
            bail!("Kein Modell gewählt.");
        }
        if text_of(args.get("beschreibung")).trim().is_empty() {
            bail!("Die Beschreibung des Bildes fehlt.");
        }
        let vorlage = text_of(args.get("vorlage"));
        let template = if !vorlage.trim().is_empty() { Some(self.template(root, &js_string(&args["vorlage"]))?) } else { None };
        let datei = text_of(args.get("datei"));
        if !datei.trim().is_empty() {
            inside(root, &js_string(&args["datei"]))?; // falscher Pfad: melden, bevor Flux arbeitet
        }
        let flux = flux_arguments(args, template.is_some());
        let text = format!("Rufe {FUNCTION} mit genau diesen Argumenten auf:\n{}", serde_json::to_string_pretty(&flux)?)
            + if template.is_some() { "\n\nDas Bild zum Bearbeiten ist dieser Nachricht angehängt." } else { "" };

        let controller = match &opts.signal {
            Some(s) => s.child_token(),
            None => Signal::new(),
        };
        let c2 = controller.clone();
        let _limit = AbortOnDrop(tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(TIME_LIMIT_MS)).await;
            c2.cancel();
        }));
        let started = crate::util::now_ms();
        let status = Arc::new(Mutex::new("Flux erstellt das Bild".to_string()));
        let progress = {
            let (status, on) = (status.clone(), opts.on_progress.clone());
            move || {
                if let Some(f) = &on {
                    f(&format!("{} … {} s", status.lock(), ((crate::util::now_ms() - started) as f64 / 1000.0).round() as i64));
                }
            }
        };
        progress();
        let _ticker = AbortOnDrop(tokio::spawn({
            let progress = progress.clone();
            async move {
                let mut tick = tokio::time::interval(Duration::from_secs(1));
                tick.tick().await;
                loop {
                    tick.tick().await;
                    progress();
                }
            }
        }));
        let mut trace = Trace {
            zeit: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            werkzeug: json!({ "id": tool.id, "name": tool.name }),
            argumente: flux.clone(),
            ablauf: Vec::new(),
            strom: None,
            antwort: None,
        };
        let mut temp: Option<TempChat> = None;
        let mut run: Option<Run> = None;
        let chat = if opts.chat.is_empty() { "chat".to_string() } else { opts.chat.clone() };
        let result = self
            .create_inner(root, args, &tool, &model, &text, &flux, template.as_ref(), &controller, &status, &chat, started, &mut trace, &mut temp, &mut run)
            .await;
        drop(_ticker);
        drop(_limit);
        let result = result.map_err(|err| {
            if controller.is_cancelled() && !opts.signal.as_ref().is_some_and(Signal::is_cancelled) {
                anyhow!("Flux hat nach {} Minuten kein Bild geliefert.", TIME_LIMIT_MS / 60000)
            } else {
                err
            }
        });
        // Abgebrochen: die Antwort im Hintergrund in ASK ebenfalls anhalten
        if controller.is_cancelled() {
            if let Some(task) = run.as_ref().map(|r| r.task.clone()).filter(|t| !t.is_empty()) {
                let ask = self.ask.clone();
                tokio::spawn(async move {
                    let _ = ask.raw(&format!("/api/tasks/stop/{}", enc(&task)), RequestOptions::default().with_method(reqwest::Method::POST)).await;
                });
            }
        }
        if let Some(t) = temp {
            let ask = self.ask.clone();
            tokio::spawn(async move {
                let _ = ask.raw(&format!("/api/v1/chats/{}", enc(&t.id)), RequestOptions::default().with_method(reqwest::Method::DELETE)).await;
            });
        }
        result
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_inner(
        &self,
        root: &Path,
        args: &Value,
        tool: &FluxTool,
        model: &str,
        text: &str,
        flux: &Value,
        template: Option<&Template>,
        controller: &Signal,
        status: &Arc<Mutex<String>>,
        chat: &str,
        started: i64,
        trace: &mut Trace,
        temp: &mut Option<TempChat>,
        run: &mut Option<Run>,
    ) -> Result<CreateResult> {
        match self.temp_chat(model, text, template).await {
            Ok(t) => {
                trace.step(format!("Vorübergehender Chat {}", t.id));
                *temp = Some(t);
            }
            Err(err) => trace.step(format!("Ohne Chat ({err})")),
        }
        let mut found = Found::default();
        let origin = self.origin();
        let r = self.request(tool, model, text, template, temp.as_ref(), controller, &mut found, &origin, trace).await?;
        let mut content = r.content.clone();
        let background = r.background;
        let (tool_call, error) = (r.tool_call, r.error.clone());
        *run = Some(r);
        if let Some(t) = temp.as_ref() {
            // Hintergrund: warten, bis ASK fertig ist; sonst einmal nachlesen, was ASK im Chat abgelegt hat
            let answer = if background {
                let status = status.clone();
                self.wait(t, controller, &mut found, &origin, trace, &move |s| *status.lock() = s.to_string()).await?
            } else {
                self.answer_of(t).await.ok().and_then(|a| a.0)
            };
            if let Some(answer) = answer {
                find_image_refs(&answer, &origin, &mut found);
                if content.is_empty() {
                    content = js_string(answer.get("content").unwrap_or(&Value::Null));
                }
                trace.antwort = Some(answer);
            }
        }
        let list = if found.strong.is_empty() { &found.weak } else { &found.strong };
        let refs: Vec<String> = list.iter().filter(|r| Some(r.as_str()) != template.map(|t| t.url.as_str())).take(MAX_IMAGES).cloned().collect();
        let mut images: Vec<(Vec<u8>, &'static str)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new(); // dasselbe Bild unter zwei Adressen (Datei und Link) nur einmal
        for r in &refs {
            match self.download(r, controller).await {
                Ok(data) => {
                    let ext = image_type(&data);
                    let hash = hex::encode(Sha1::digest(&data));
                    match ext {
                        Some(ext) if !seen.contains(&hash) => images.push((data, ext)),
                        Some(_) => {}
                        None => trace.step(format!("Kein Bild: {}", take_chars(r, 120))),
                    }
                    seen.insert(hash);
                }
                Err(err) => {
                    if controller.is_cancelled() {
                        return Err(err);
                    }
                    trace.step(format!("Nicht geladen: {} ({err})", take_chars(r, 120)));
                }
            }
        }
        if images.is_empty() {
            let saved = self.keep_trace(root, chat, trace, &content);
            let said = plain_text(&content);
            let reason = if tool_call {
                "ASK hat Flux nicht selbst ausgeführt, sondern den Aufruf zurückgegeben.".to_string()
            } else if !error.is_empty() {
                format!("ASK meldet: {error}")
            } else {
                "ASK hat kein Bild zurückgegeben.".to_string()
            };
            bail!(
                "{reason}{}{}",
                if said.is_empty() { String::new() } else { format!(" Antwort von ASK: «{}»", take_chars(&said, 300)) },
                if saved.is_empty() { String::new() } else { format!(" Details: {saved}") }
            );
        }
        let exts: Vec<&str> = images.iter().map(|i| i.1).collect();
        let names = save_names(
            root,
            &datei_of(args),
            &js_string(&flux["beschreibung"]),
            template.map(|t| t.path.as_str()).unwrap_or(""),
            &exts,
        )?;
        for (rel, (data, _)) in names.iter().zip(&images) {
            let full = root.join(rel.split('/').collect::<PathBuf>());
            if let Some(dir) = full.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(&full, data)?;
        }
        Ok(CreateResult {
            files: names,
            text: plain_text(&content),
            seconds: ((crate::util::now_ms() - started) as f64 / 1000.0).round() as i64,
            template: template.map(|t| t.path.clone()).unwrap_or_default(),
        })
    }

    /// Die Anfrage an ASK
    #[allow(clippy::too_many_arguments)]
    async fn request(
        &self,
        tool: &FluxTool,
        model: &str,
        text: &str,
        template: Option<&Template>,
        temp: Option<&TempChat>,
        signal: &Signal,
        found: &mut Found,
        origin: &str,
        trace: &mut Trace,
    ) -> Result<Run> {
        let send = |with_image: bool| {
            let user = match (with_image, template) {
                (true, Some(t)) => json!([{ "type": "text", "text": text }, { "type": "image_url", "image_url": { "url": t.url } }]),
                _ => json!(text),
            };
            let mut body = json!({
                "model": model,
                "stream": true,
                "messages": [{ "role": "system", "content": SYSTEM }, { "role": "user", "content": user }],
                "tool_ids": [tool.id],
                "features": { "web_search": false, "image_generation": false },
                "background_tasks": { "title_generation": false, "tags_generation": false, "follow_up_generation": false },
            });
            // Wie die Webseite: mit Chat, Nachricht und Sitzung führt ASK die Werkzeuge selbst aus
            if let Some(t) = temp {
                let o = body.as_object_mut().expect("Objekt");
                o.insert("chat_id".into(), json!(t.id));
                o.insert("id".into(), json!(t.answer));
                o.insert("session_id".into(), json!(format!("ask-desktop-{}", hex::encode(rand::random::<[u8; 6]>()))));
            }
            let opts = RequestOptions::post(body.to_string()).with_signal(signal.clone());
            let ask = self.ask.clone();
            async move { ask.raw("/api/chat/completions", opts).await }
        };
        let mut res = send(template.is_some()).await?;
        if !res.status().is_success() {
            let err = failure(res).await;
            // Nimmt das Modell keine Bilder an, liegt die Vorlage nur im Chat; das Werkzeug lädt sie selbst
            let status = crate::askapi::status_of(&err);
            if !(template.is_some() && status == Some(400) && re!(r"(?i)image|vision|multimodal|modalit").is_match(&err.to_string())) {
                return Err(err);
            }
            trace.step("Modell nimmt keine Bilder an: Vorlage nur im Chat");
            res = send(false).await?;
            if !res.status().is_success() {
                return Err(failure(res).await);
            }
        }
        let kind = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        if !kind.contains("text/event-stream") {
            let data: Value = res.json().await.unwrap_or_else(|_| json!({}));
            trace.step(format!("Antwort als JSON: {}", take_chars(&data.to_string(), 300)));
            if truthy(&data) && (data.get("task_id").is_some_and(truthy) || data.get("status") == Some(&json!(true))) && data.get("choices").is_none_or(|c| !truthy(c)) {
                return Ok(Run { background: true, task: data.get("task_id").filter(|t| truthy(t)).map(js_string).unwrap_or_default(), ..Default::default() });
            }
            find_image_refs(&data, origin, found);
            let message = data.pointer("/choices/0/message").cloned().unwrap_or_else(|| json!({}));
            let error = match data.get("detail") {
                Some(Value::String(s)) => s.clone(),
                _ => data.pointer("/error/message").map(js_string).unwrap_or_default(),
            };
            return Ok(Run {
                content: text_of(message.get("content")),
                tool_call: has_flux_call(&message),
                error,
                ..Default::default()
            });
        }
        trace.step("Antwort gestreamt");
        trace.strom = Some(Vec::new());
        let mut st = Run::default();
        let mut parser = SseParser::new();
        let mut decoder = Utf8Stream::default();
        let mut stream = res.bytes_stream();
        loop {
            let next = tokio::select! {
                _ = signal.cancelled() => return Err(AskError::abort().into()),
                n = stream.next() => n,
            };
            let Some(chunk) = next else { break };
            let bytes = chunk.map_err(|e| anyhow!("{e}"))?;
            for event in parser.push(&decoder.push(&bytes)) {
                handle_chunk(&event, &mut st, found, origin, trace);
            }
        }
        for event in parser.push("\n") {
            handle_chunk(&event, &mut st, found, origin, trace);
        }
        // Links kommen in kleinen Stücken: erst im ganzen Text auffindbar
        find_image_refs(&Value::String(st.content.clone()), origin, found);
        Ok(st)
    }

    /// Antwort im Hintergrund: den Chat in ASK lesen, bis die Antwort fertig ist
    async fn wait(&self, temp: &TempChat, signal: &Signal, found: &mut Found, origin: &str, trace: &mut Trace, on_status: &(dyn Fn(&str) + Send + Sync)) -> Result<Option<Value>> {
        let mut last: Option<Value> = None;
        let mut last_content = String::new();
        let mut first_image: i64 = 0;
        let mut round = 0;
        loop {
            sleep(self.poll_ms, signal).await?;
            let answer = match self.answer_of(temp).await {
                Ok(a) => a,
                Err(err) => {
                    trace.step(format!("Chat nicht lesbar: {err}"));
                    if round > 3 {
                        return Ok(last);
                    }
                    round += 1;
                    continue;
                }
            };
            round += 1;
            let Some(message) = answer.0.or_else(|| answer.1.last().cloned()) else { continue };
            last = Some(message.clone());
            let s = message
                .get("statusHistory")
                .and_then(Value::as_array)
                .and_then(|h| h.last().cloned())
                .filter(truthy)
                .or_else(|| message.get("status").cloned().filter(truthy));
            if let Some(d) = s.as_ref().and_then(|s| s.get("description")).filter(|d| truthy(d)) {
                on_status(&take_chars(&js_string(d), 80));
            }
            let before = found.strong.len();
            find_image_refs(&message, origin, found);
            if found.strong.len() > before && first_image == 0 {
                first_image = crate::util::now_ms();
            }
            let content = js_string(message.get("content").unwrap_or(&Value::Null));
            let busy = self.running(temp).await;
            if message.get("done") == Some(&json!(true)) || busy == Some(false) {
                return Ok(Some(message));
            }
            // ASK sagt nicht, ob es fertig ist: fertig, wenn der Text steht
            if busy.is_none() && !content.trim().is_empty() && content == last_content {
                return Ok(Some(message));
            }
            if first_image != 0 && crate::util::now_ms() - first_image > AFTER_IMAGE_MS {
                return Ok(Some(message));
            }
            last_content = content;
        }
    }

    /// Bild laden: eingebettet, von ASK (mit Anmeldung) oder von einem anderen Server (ohne)
    async fn download(&self, r: &str, signal: &Signal) -> Result<Vec<u8>> {
        use base64::Engine;
        if r.starts_with("data:") {
            let payload = &r[r.find(',').map(|i| i + 1).unwrap_or(r.len())..];
            return Ok(base64::engine::general_purpose::STANDARD.decode(payload.trim()).unwrap_or_default());
        }
        let origin = self.origin();
        let url = url::Url::parse(&origin)?.join(r)?;
        let res = if url.origin().ascii_serialization() == origin {
            let query = url.query().map(|q| format!("?{q}")).unwrap_or_default();
            self.ask.raw(&format!("{}{query}", url.path()), RequestOptions::default().with_signal(signal.clone())).await?
        } else {
            let target = check_url(url.as_str(), self.allow_local)?;
            let sent = self.client.get(target.as_str()).send();
            let res = tokio::select! {
                _ = signal.cancelled() => return Err(AskError::abort().into()),
                r = sent => r.map_err(|e| anyhow!("fetch failed: {e}"))?,
            };
            check_url(res.url().as_str(), self.allow_local)?; // Weiterleitung ins lokale Netz: Bild nicht übernehmen
            res
        };
        if !res.status().is_success() {
            bail!("HTTP {}", res.status().as_u16());
        }
        if res.content_length().unwrap_or(0) as usize > MAX_BYTES {
            bail!("grösser als 40 MB");
        }
        let mut data = Vec::new();
        let mut stream = res.bytes_stream();
        loop {
            let next = tokio::select! {
                _ = signal.cancelled() => return Err(AskError::abort().into()),
                n = stream.next() => n,
            };
            let Some(chunk) = next else { break };
            data.extend_from_slice(&chunk.map_err(|e| anyhow!("{e}"))?);
            if data.len() > MAX_BYTES {
                bail!("grösser als 40 MB");
            }
        }
        Ok(data)
    }

    /// Antwort von ASK zum Nachsehen, wenn kein Bild kam (unter .ask/arbeit, mit dem Chat gelöscht)
    fn keep_trace(&self, root: &Path, chat: &str, trace: &Trace, content: &str) -> String {
        let rel = format!(".ask/arbeit/{chat}/flux-{}.json", base36(crate::util::now_ms().max(0) as u64));
        let full = root.join(rel.split('/').collect::<PathBuf>());
        let write = || -> std::io::Result<()> {
            if let Some(dir) = full.parent() {
                std::fs::create_dir_all(dir)?;
            }
            let text = serde_json::to_string_pretty(&trace.to_value(content)).map_err(std::io::Error::other)?;
            std::fs::write(&full, text)
        };
        if write().is_ok() { rel } else { String::new() }
    }
}

fn datei_of(args: &Value) -> String {
    text_of(args.get("datei"))
}

fn js_or_undefined(v: &Value) -> String {
    match v {
        Value::Null => "null".into(),
        other => js_string(other),
    }
}

fn enc(s: &str) -> String {
    crate::github::encode_uri_component(s)
}

fn has_flux_call(message: &Value) -> bool {
    message
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|calls| calls.iter().any(|c| c.pointer("/function/name").and_then(Value::as_str) == Some(FUNCTION)))
}

/// Ein Stück der gestreamten Antwort
fn handle_chunk(chunk: &Value, st: &mut Run, found: &mut Found, origin: &str, trace: &mut Trace) {
    if let Some(strom) = trace.strom.as_mut().filter(|s| s.len() < 200) {
        strom.push(take_chars(&chunk.to_string(), 2000));
    }
    find_image_refs(chunk, origin, found);
    let choice = chunk.pointer("/choices/0");
    let d = choice.and_then(|c| c.get("delta").filter(|d| truthy(d)).or_else(|| c.get("message").filter(|m| truthy(m))));
    if let Some(d) = d {
        if let Some(Value::String(s)) = d.get("content") {
            st.content.push_str(s);
        }
        if has_flux_call(d) {
            st.tool_call = true;
        }
    }
    if let Some(e) = chunk.get("error").filter(|e| truthy(e)) {
        st.error = match e.get("message").filter(|m| truthy(m)) {
            Some(m) => js_string(m),
            None => e.to_string(),
        };
    }
}

/// Fehler einer Antwort von ASK mit Status (`failure` in JS)
async fn failure(res: reqwest::Response) -> anyhow::Error {
    let status = res.status().as_u16();
    let detail = res.text().await.unwrap_or_default();
    let mut message = detail.clone();
    if let Ok(v) = serde_json::from_str::<Value>(&detail) {
        let pick = [v.get("detail"), v.pointer("/error/message")].into_iter().flatten().find(|x| truthy(x));
        if let Some(p) = pick {
            message = match p {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            };
        }
    }
    let shown = take_chars(&message, 300);
    AskError::new(format!("ASK: {}", if shown.is_empty() { format!("HTTP {status}") } else { shown })).with_status(status).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::askapi::is_abort;
    use crate::askapi::test_http::{client, serve, Req, Resp};
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const ORIGIN: &str = "https://ask.test";

    fn png() -> Vec<u8> {
        let mut v = vec![0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend(std::iter::repeat_n(7u8, 120));
        v
    }

    fn jpg() -> Vec<u8> {
        let mut v = vec![0xff, 0xd8, 0xff, 0xe0];
        v.extend(std::iter::repeat_n(3u8, 120));
        v
    }

    fn data_url() -> String {
        use base64::Engine;
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png()))
    }

    fn sse(chunks: &[Value]) -> Resp {
        let mut text: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
        text.push_str("data: [DONE]\n\n");
        Resp::sse(&text)
    }

    fn delta(d: Value) -> Value {
        json!({ "choices": [{ "index": 0, "delta": d }] })
    }

    fn tools() -> Value {
        json!([{ "id": "moodle", "specs": [{ "name": "moodle_kurse" }] }, { "id": "flux_bild", "name": "Flux Bild", "specs": [{ "name": "bild_erstellen" }] }])
    }

    /// ASK zum Testen: handler(Anfrage) liefert die Antwort (None: 404)
    async fn fake_ask(handler: impl Fn(&Req) -> Option<Resp> + Send + Sync + 'static) -> (Arc<AskSession>, crate::askapi::test_http::TestServer) {
        let server = serve(move |req| handler(req).unwrap_or_else(|| Resp::json(404, &json!({ "detail": "not found" })))).await;
        let ask = Arc::new(AskSession::with_client(client()));
        ask.set_base(&server.url);
        ask.set_token("tok");
        (ask, server)
    }

    fn images(ask: Arc<AskSession>) -> Images {
        Images::new(ask, Arc::new(|| "qwen".to_string())).with_client(client()).with_poll_ms(20)
    }

    fn calls(server: &crate::askapi::test_http::TestServer) -> Vec<Req> {
        server.reqs.lock().clone()
    }

    #[test]
    fn argumente_fuer_flux_nur_gesetzte_und_gueltige_werte() {
        assert_eq!(
            flux_arguments(
                &json!({ "beschreibung": " Ein Fuchs ", "format": "Breitbild", "qualitaet": "super", "anzahl": 9, "stil": "", "referenzen": "Eiger = mountain", "tf_branding": true }),
                false
            ),
            json!({ "beschreibung": "Ein Fuchs", "format": "breitbild", "anzahl": 4, "referenzen": "Eiger = mountain", "tf_branding": true })
        );
        assert_eq!(
            flux_arguments(&json!({ "beschreibung": "Himmel blau", "referenzen": "Eiger = mountain", "anzahl": 1, "verwandeln": "true" }), true),
            json!({ "beschreibung": "Himmel blau", "bearbeiten": true, "verwandeln": true })
        );
    }

    #[test]
    fn bildtyp_an_den_ersten_bytes() {
        assert_eq!(image_type(&png()), Some(".png"));
        assert_eq!(image_type(&jpg()), Some(".jpg"));
        assert_eq!(image_type(b"<html>kein Bild</html>"), None);
    }

    #[test]
    fn bilder_in_antworten_von_ask_finden() {
        let value = json!({
            "files": [{ "type": "image", "url": "/api/v1/files/abc-1" }, { "type": "image", "id": "def-2" }, { "type": "file", "url": "/api/v1/files/doc/content" }],
            "content": format!("Fertig!\n![Bild]({ORIGIN}/cache/image/generations/x1.png)\n![extern](https://cdn.example.com/out/bild.webp)\n\
                <details type=\"tool_calls\" result=\"&quot;![b](/api/v1/files/ghi-3/content)&quot;\"></details>\
                ![ref](https://upload.wikimedia.org/eiger.jpg) ![fremd](https://andere.ch/api/v1/files/zzz/content)"),
            "embed": data_url(),
        });
        let mut found = Found::default();
        find_image_refs(&value, ORIGIN, &mut found);
        assert_eq!(
            found.strong,
            ["/api/v1/files/abc-1/content", "/api/v1/files/def-2/content", "/cache/image/generations/x1.png", "/api/v1/files/ghi-3/content", &data_url()]
        );
        assert_eq!(found.weak, ["https://cdn.example.com/out/bild.webp", "https://andere.ch/api/v1/files/zzz/content"]);
        assert_eq!(plain_text("Hier ist es:\n![b](/api/v1/files/x/content)\n<details>gross</details>"), "Hier ist es:");
    }

    #[test]
    fn dateinamen_neu_nummeriert_gewuenscht_vorlage_ersetzen() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir(root.join("bilder")).unwrap();
        std::fs::write(root.join("bilder").join("fuchs-im-schnee.png"), "alt").unwrap();
        let names = |datei: &str, beschreibung: &str, vorlage: &str, exts: &[&str]| save_names(root, datei, beschreibung, vorlage, exts).unwrap();
        assert_eq!(names("", "Fuchs im Schnee!", "", &[".png"]), ["bilder/fuchs-im-schnee-2.png"]);
        assert_eq!(names("", "Grüsse über Bern", "", &[".png", ".jpg"]), ["bilder/gruesse-ueber-bern-1.png", "bilder/gruesse-ueber-bern-2.jpg"]);
        assert_eq!(names("bilder/fuchs-im-schnee.png", "", "", &[".png"]), ["bilder/fuchs-im-schnee.png"]);
        assert_eq!(names("plakat/titel.jpg", "", "", &[".png"]), ["plakat/titel.png"]);
        assert_eq!(names("", "", "bilder/fuchs-im-schnee.png", &[".png"]), ["bilder/fuchs-im-schnee.png"]);
        assert_eq!(names("", "", "fotos/klasse.jpeg", &[".jpg"]), ["fotos/klasse.jpeg"]);
        assert_eq!(names("", "", "fotos/klasse.jpeg", &[".png"]), ["fotos/klasse.png"]);
        assert_eq!(names("", "", ".ask/anhaenge/c1/Mein Foto.png", &[".png"]), ["bilder/mein-foto.png"]);
        let err = save_names(root, "../draussen.png", "", "", &[".png"]).unwrap_err().to_string();
        assert!(err.contains("nicht im Arbeitsordner"), "{err}");
    }

    #[tokio::test]
    async fn flux_werkzeug_finden() {
        let (ask, _s) = fake_ask(|req| (req.path == "/api/v1/tools/").then(|| Resp::json(200, &tools()))).await;
        let images = images(ask);
        assert_eq!(images.discover().await, Some(FluxTool { id: "flux_bild".into(), name: "Flux Bild".into() }));
        assert!(images.available());
        let (ask, _s) = fake_ask(|req| (req.path == "/api/v1/tools/").then(|| Resp::json(200, &json!([tools()[0].clone()])))).await;
        assert_eq!(self::images(ask).discover().await, None);
        let (ask, _s) = fake_ask(|_| Some(Resp::json(401, &json!({ "detail": "nein" })))).await;
        assert_eq!(self::images(ask).discover().await, None);
    }

    #[tokio::test]
    async fn wie_die_webseite_antwort_im_hintergrund_bild_als_datei_im_voruebergehenden_chat() {
        let dir = tempfile::tempdir().unwrap();
        let polls = Arc::new(AtomicUsize::new(0));
        let chat_body: Arc<Mutex<Option<Value>>> = Arc::new(Mutex::new(None));
        let (p2, cb) = (polls.clone(), chat_body.clone());
        let (ask, server) = fake_ask(move |req| {
            let p = req.path.as_str();
            match (p, req.method.as_str()) {
                ("/api/v1/tools/", _) => Some(Resp::json(200, &tools())),
                ("/api/v1/chats/new", _) => {
                    *cb.lock() = Some(req.json()["chat"].clone());
                    Some(Resp::json(200, &json!({ "id": "tmp1", "chat": req.json()["chat"] })))
                }
                ("/api/chat/completions", _) => Some(Resp::json(200, &json!({ "status": true, "task_id": "t1" }))),
                ("/api/v1/chats/tmp1", "GET") => {
                    let n = p2.fetch_add(1, Ordering::SeqCst) + 1;
                    let chat = cb.lock().clone().unwrap();
                    let answer = chat["history"]["messages"].as_object().unwrap().values().find(|m| m["role"] == "assistant").unwrap().clone();
                    let mut m = answer.as_object().unwrap().clone();
                    m.insert("statusHistory".into(), json!([{ "description": "Bild wird erstellt" }]));
                    m.insert("files".into(), if n > 1 { json!([{ "type": "image", "url": "/api/v1/files/bild-1/content" }]) } else { json!([]) });
                    m.insert("content".into(), json!(if n > 1 { "Hier ist dein Fuchs." } else { "" }));
                    let id = answer["id"].as_str().unwrap().to_string();
                    Some(Resp::json(200, &json!({ "id": "tmp1", "chat": { "history": { "messages": { id: m } } } })))
                }
                ("/api/tasks/chat/tmp1", _) => Some(Resp::json(200, &json!({ "task_ids": if p2.load(Ordering::SeqCst) > 1 { json!([]) } else { json!(["t1"]) } }))),
                ("/api/v1/files/bild-1/content", _) => Some(Resp::text(200, "image/png", png())),
                ("/api/v1/chats/tmp1", "DELETE") => Some(Resp::json(200, &json!(true))),
                _ => None,
            }
        })
        .await;
        let progress: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let pr = progress.clone();
        let on_progress: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |t| pr.lock().push(t.to_string()));
        let images = images(ask);
        let r = images
            .create(dir.path(), &json!({ "beschreibung": "Ein Fuchs im Schnee", "format": "querformat" }), CreateOptions { chat: "c1".into(), signal: None, on_progress: Some(on_progress) })
            .await
            .unwrap();
        assert_eq!(r.files, ["bilder/ein-fuchs-im-schnee.png"]);
        assert_eq!(r.text, "Hier ist dein Fuchs.");
        assert_eq!(std::fs::read(dir.path().join("bilder/ein-fuchs-im-schnee.png")).unwrap(), png());
        let all = calls(&server);
        let sent = all.iter().find(|c| c.path == "/api/chat/completions").unwrap().json();
        assert_eq!(sent["tool_ids"], json!(["flux_bild"]));
        assert_eq!(sent["model"], "qwen");
        assert_eq!(sent["chat_id"], "tmp1");
        assert!(sent["id"].is_string() && sent["session_id"].is_string() && sent.get("tools").is_none(), "wie die Webseite, ohne Werkzeuge der App");
        assert!(sent["messages"][1]["content"].as_str().unwrap().contains("\"format\": \"querformat\""));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(calls(&server).iter().any(|c| c.path == "/api/v1/chats/tmp1" && c.method == "DELETE"), "vorübergehender Chat gelöscht");
        let re = Regex::new(r"^Flux erstellt das Bild … \d+ s$").unwrap();
        assert!(progress.lock().iter().any(|t| re.is_match(t)), "{:?}", progress.lock());
    }

    #[tokio::test]
    async fn gestreamte_antwort_mit_bild_link_ohne_chat_vorlage_bearbeiten() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("fotos")).unwrap();
        std::fs::write(dir.path().join("fotos").join("klasse.png"), png()).unwrap();
        let (ask, server) = fake_ask(|req| match req.path.as_str() {
            "/api/v1/tools/" => Some(Resp::json(200, &tools())),
            "/api/v1/chats/new" => Some(Resp::json(405, &json!({ "detail": "Method Not Allowed" }))),
            "/api/chat/completions" => {
                // Link in kleinen Stücken wie bei einem echten Strom
                let text: Vec<char> = "Bearbeitet: ![bild](/cache/image/generations/neu.jpg)".chars().collect();
                let mut parts: Vec<Value> = text.chunks(7).map(|c| delta(json!({ "content": c.iter().collect::<String>() }))).collect();
                parts.push(json!({ "files": [{ "type": "image", "url": "/api/v1/files/gleich" }] }));
                Some(sse(&parts))
            }
            "/cache/image/generations/neu.jpg" | "/api/v1/files/gleich/content" => Some(Resp::text(200, "image/jpeg", jpg())),
            _ => None,
        })
        .await;
        let images = images(ask);
        let r = images.create(dir.path(), &json!({ "beschreibung": "Himmel blau", "vorlage": "fotos/klasse.png" }), CreateOptions { chat: "c1".into(), ..Default::default() }).await.unwrap();
        assert_eq!(r.files, ["fotos/klasse.jpg"], "anderes Format: neben der Vorlage; dasselbe Bild unter zwei Adressen nur einmal");
        assert!(calls(&server).iter().any(|c| c.path == "/cache/image/generations/neu.jpg"), "Link aus Stücken zusammengesetzt");
        let err = images.create(dir.path(), &json!({ "beschreibung": "x", "datei": "../weg.png" }), CreateOptions::default()).await.unwrap_err().to_string();
        assert!(err.contains("nicht im Arbeitsordner"), "{err}");
        assert_eq!(calls(&server).iter().filter(|c| c.path == "/api/chat/completions").count(), 1, "falscher Pfad: Flux gar nicht erst gefragt");
        assert_eq!(std::fs::read(dir.path().join("fotos/klasse.jpg")).unwrap(), jpg());
        let all = calls(&server);
        let sent = all.iter().find(|c| c.path == "/api/chat/completions").unwrap().json();
        assert!(sent.get("chat_id").is_none() && sent.get("session_id").is_none());
        assert_eq!(sent["messages"][1]["content"][1]["image_url"]["url"], json!(data_url()), "Vorlage als Bild in der Nachricht");
        assert!(sent["messages"][1]["content"][0]["text"].as_str().unwrap().contains("\"bearbeiten\": true"));
    }

    #[tokio::test]
    async fn kein_bild_verstaendlicher_fehler_und_antwort_von_ask_zum_nachsehen() {
        let dir = tempfile::tempdir().unwrap();
        let (ask, _s) = fake_ask(|req| match req.path.as_str() {
            "/api/v1/tools/" => Some(Resp::json(200, &tools())),
            "/api/chat/completions" => Some(sse(&[delta(json!({ "content": "Das Bild konnte nicht erstellt werden." }))])),
            _ => None,
        })
        .await;
        let err = images(ask).create(dir.path(), &json!({ "beschreibung": "Fuchs" }), CreateOptions { chat: "c1".into(), ..Default::default() }).await.unwrap_err().to_string();
        let re = Regex::new(r"ASK hat kein Bild zurückgegeben\. Antwort von ASK: «Das Bild konnte nicht erstellt werden\.» Details: \.ask/arbeit/c1/flux-\w+\.json").unwrap();
        assert!(re.is_match(&err), "{err}");
        assert_eq!(std::fs::read_dir(dir.path().join(".ask/arbeit/c1")).unwrap().count(), 1);
        assert!(!dir.path().join("bilder").exists());

        let (ask, _s) = fake_ask(|req| match req.path.as_str() {
            "/api/v1/tools/" => Some(Resp::json(200, &tools())),
            "/api/chat/completions" => Some(sse(&[delta(json!({ "tool_calls": [{ "index": 0, "id": "x", "function": { "name": "bild_erstellen", "arguments": "{}" } }] }))])),
            _ => None,
        })
        .await;
        let err = images(ask).create(dir.path(), &json!({ "beschreibung": "Fuchs" }), CreateOptions { chat: "c1".into(), ..Default::default() }).await.unwrap_err().to_string();
        assert!(err.contains("nicht selbst ausgeführt"), "{err}");

        let (ask, _s) = fake_ask(|req| match req.path.as_str() {
            "/api/v1/tools/" => Some(Resp::json(200, &tools())),
            "/api/chat/completions" => Some(Resp::json(403, &json!({ "detail": "Tool not allowed" }))),
            _ => None,
        })
        .await;
        let err = images(ask).create(dir.path(), &json!({ "beschreibung": "Fuchs" }), CreateOptions::default()).await.unwrap_err().to_string();
        assert!(err.contains("ASK: Tool not allowed"), "{err}");

        let (ask, _s) = fake_ask(|_| Some(Resp::json(200, &json!([])))).await;
        let err = images(ask).create(dir.path(), &json!({ "beschreibung": "Fuchs" }), CreateOptions::default()).await.unwrap_err().to_string();
        assert!(err.contains("nicht freigegeben"), "{err}");
    }

    #[tokio::test]
    async fn stoppen_haelt_auch_die_antwort_in_ask_an() {
        let dir = tempfile::tempdir().unwrap();
        let (ask, server) = fake_ask(|req| match (req.path.as_str(), req.method.as_str()) {
            ("/api/v1/tools/", _) => Some(Resp::json(200, &tools())),
            ("/api/v1/chats/new", _) => Some(Resp::json(200, &json!({ "id": "tmp2" }))),
            ("/api/chat/completions", _) => Some(Resp::json(200, &json!({ "status": true, "task_id": "t9" }))),
            ("/api/v1/chats/tmp2", "GET") => Some(Resp::json(200, &json!({ "chat": { "history": { "messages": {} } } }))),
            ("/api/tasks/chat/tmp2", _) => Some(Resp::json(200, &json!({ "task_ids": ["t9"] }))),
            _ => Some(Resp::json(200, &json!(true))),
        })
        .await;
        let signal = Signal::new();
        let s2 = signal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(120)).await;
            s2.cancel();
        });
        let err = images(ask).create(dir.path(), &json!({ "beschreibung": "Fuchs" }), CreateOptions { signal: Some(signal), ..Default::default() }).await.unwrap_err();
        assert!(is_abort(&err), "{err}");
        tokio::time::sleep(Duration::from_millis(50)).await;
        let all = calls(&server);
        assert!(all.iter().any(|c| c.path == "/api/tasks/stop/t9" && c.method == "POST"));
        assert!(all.iter().any(|c| c.path == "/api/v1/chats/tmp2" && c.method == "DELETE"));
    }
}
