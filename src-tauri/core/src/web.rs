//! Werkzeuge web_search und web_fetch (ENTSCHEIDE E26). Laufen im Hauptprozess: Suche über die
//! Websuche von ASK (dieselbe Suchmaschine wie auf der Webseite), sonst über DuckDuckGo; Seiten kommen
//! als lesbarer Text statt als HTML. Vorher behalf sich das Modell mit curl und rohem HTML.
//!
//! Portiert aus src/main/web.js. Öffentliche Schnittstelle: `Web::new(ask, client, allow_local)`,
//! `Web::search(query, signal) -> SearchResponse`, `Web::page(url, PageOptions) -> PageResult`,
//! `Web::client_builder(allow_local)` (HTTP-Client, der Weiterleitungen ins lokale Netz schon vor dem
//! Verbinden verweigert), `html_to_text`, `parse_duck_duck_go`, `check_url`, `decode_entities`, `PAGE_CHARS`.
//! `fetchImpl` aus JS ist ein `reqwest::Client`; `ask` ein `WebSearchBackend` (die `AskSession` ist eines).

#![allow(clippy::regex_creation_in_loops)] // `re!` legt jeden Ausdruck nur einmal an (OnceLock)

use crate::askapi::{AskError, AskSession, SearchResult, Signal};
use crate::util::take_chars;
use anyhow::{anyhow, bail, Result};
use async_trait::async_trait;
use futures::StreamExt;
use regex::Regex;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

macro_rules! re {
    ($p:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($p).expect("regex"))
    }};
}

const MAX_RESULTS: usize = 8;
const MAX_BYTES: usize = 3 * 1024 * 1024;
/// Text pro web_fetch-Aufruf, weiter mit «ab»
pub const PAGE_CHARS: usize = 12000;
const TIMEOUT: Duration = Duration::from_secs(20);
const AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/138.0 Safari/537.36 ASK-Desktop";
const LOCAL_BLOCKED: &str = "Adressen im lokalen Netz oder auf diesem Rechner sind gesperrt.";

const ENTITIES: &[(&str, &str)] = &[
    ("amp", "&"), ("lt", "<"), ("gt", ">"), ("quot", "\""), ("apos", "'"), ("nbsp", " "), ("shy", ""), ("auml", "ä"), ("ouml", "ö"), ("uuml", "ü"),
    ("Auml", "Ä"), ("Ouml", "Ö"), ("Uuml", "Ü"), ("szlig", "ß"), ("eacute", "é"), ("egrave", "è"), ("agrave", "à"), ("ccedil", "ç"), ("laquo", "«"),
    ("raquo", "»"), ("ndash", "–"), ("mdash", "—"), ("hellip", "…"), ("rsquo", "’"), ("lsquo", "‘"), ("rdquo", "”"), ("ldquo", "“"), ("copy", "©"),
    ("reg", "®"), ("euro", "€"), ("middot", "·"), ("bull", "•"), ("times", "×"),
];

fn entity(name: &str) -> Option<&'static str> {
    let find = |n: &str| ENTITIES.iter().find(|(k, _)| *k == n).map(|(_, v)| *v);
    find(name).or_else(|| find(&name.to_lowercase()))
}

pub fn decode_entities(text: &str) -> String {
    re!(r"(?i)&(#x[0-9a-f]+|#\d+|[a-z]+);")
        .replace_all(text, |c: &regex::Captures| {
            let whole = c[0].to_string();
            let e = &c[1];
            if let Some(num) = e.strip_prefix('#') {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16),
                    None => num.parse::<u32>(),
                };
                return match code.ok().filter(|c| *c > 0).and_then(char::from_u32) {
                    Some(ch) => ch.to_string(),
                    None => whole,
                };
            }
            entity(e).map(String::from).unwrap_or(whole)
        })
        .to_string()
}

// HTML → lesbarer Text: ohne Skripte, Stile, Navigation, Kopf- und Fussbereich; Überschriften mit #,
// Listenpunkte mit -, Tabellenzellen mit |. Links bleiben nur als Text, damit er kurz bleibt.
// Ein Durchgang über den Text: Auch eine kaputte oder böswillige Seite (tausende offene Tags) darf den
// Hauptprozess nicht blockieren. Formulare bleiben drin: Manche Seiten stecken ganz in einem <form>.
const RAW: [&str; 2] = ["script", "style"]; // Inhalt ist kein HTML, endet erst beim schliessenden Tag
const DROP: [&str; 11] = ["script", "style", "noscript", "svg", "template", "iframe", "head", "nav", "footer", "button", "select"];
const BREAK: [&str; 14] = ["p", "div", "section", "article", "tr", "table", "ul", "ol", "blockquote", "pre", "dd", "dt", "figure", "main"];

/// Titel und Text einer Seite
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HtmlText {
    pub title: String,
    pub text: String,
}

struct Acc<'a> {
    /// (Text, innerhalb von <main>/<article>, ausserhalb von <header>)
    pieces: Vec<(Cow<'a, str>, bool, bool)>,
    title: String,
    dropped: Vec<String>,
    in_main: i32,
    in_header: i32,
    in_title: bool,
}

impl<'a> Acc<'a> {
    fn emit(&mut self, text: impl Into<Cow<'a, str>>) {
        let text = text.into();
        if self.in_title {
            self.title.push_str(&text);
            return;
        }
        if !self.dropped.is_empty() {
            return;
        }
        self.pieces.push((text, self.in_main > 0, self.in_header == 0));
    }
}

/// `/^<(\/?)([a-zA-Z][a-zA-Z0-9]*)/` auf den ersten 40 Zeichen: (schliessend, Name in Kleinbuchstaben)
fn match_tag(window: &[u8]) -> Option<(bool, String)> {
    if window.first() != Some(&b'<') {
        return None;
    }
    let mut i = 1;
    let close = window.get(1) == Some(&b'/');
    if close {
        i = 2;
    }
    if !window.get(i).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let start = i;
    while window.get(i).is_some_and(u8::is_ascii_alphanumeric) {
        i += 1;
    }
    Some((close, String::from_utf8_lossy(&window[start..i]).to_ascii_lowercase()))
}

fn clean<'a>(parts: impl Iterator<Item = &'a str>) -> String {
    let joined: String = parts.collect();
    let decoded = decode_entities(&joined);
    let collapsed = re!(r"[ \t\x0C\x0B\x{00A0}]+").replace_all(&decoded, " ").to_string();
    let lines: Vec<String> = collapsed.split('\n').map(|l| l.trim().to_string()).collect();
    let kept: Vec<&str> = lines.iter().enumerate().filter(|(n, l)| !l.is_empty() || (*n > 0 && !lines[n - 1].is_empty())).map(|(_, l)| l.as_str()).collect();
    re!(r"\n{3,}").replace_all(&kept.join("\n"), "\n\n").trim().to_string()
}

pub fn html_to_text(html: &str) -> HtmlText {
    html_to_text_with(html, true)
}

fn html_to_text_with(h: &str, strict: bool) -> HtmlText {
    let lower = h.to_ascii_lowercase(); // gleiche Längen wie h: Positionen gelten für beide
    let bytes = h.as_bytes();
    let mut acc = Acc { pieces: Vec::new(), title: String::new(), dropped: Vec::new(), in_main: 0, in_header: 0, in_title: false };
    let mut i = 0usize;
    let mut gt: i64 = -1; // Position des nächsten «>», damit nicht jedes «<» den Rest neu absucht
    let mut ends: HashMap<String, i64> = HashMap::new(); // «</script» usw.: einmal nicht gefunden heisst nie mehr suchen
    while i < h.len() {
        let Some(lt) = h[i..].find('<').map(|p| p + i) else {
            acc.emit(&h[i..]);
            break;
        };
        if lt > i {
            acc.emit(&h[i..lt]);
        }
        if h[lt..].starts_with("<!--") {
            let end = if ends.get("-->") == Some(&-1) { -1 } else { h[lt + 4..].find("-->").map(|p| (p + lt + 4) as i64).unwrap_or(-1) };
            ends.insert("-->".to_string(), end);
            if end < 0 {
                break; // offener Kommentar: der Rest gehört dazu
            }
            i = end as usize + 3;
            continue;
        }
        if gt < lt as i64 && gt != -2 {
            gt = h[lt..].find('>').map(|p| (p + lt) as i64).unwrap_or(-2);
        }
        let next = bytes.get(lt + 1).copied();
        if (next == Some(b'!') || next == Some(b'?')) && gt >= 0 {
            i = gt as usize + 1; // <!DOCTYPE …>, <?xml …?>
            continue;
        }
        let m = match_tag(&bytes[lt..(lt + 40).min(bytes.len())]);
        let Some((close, name)) = m.filter(|_| gt != -2) else {
            // kein Tag, oder Tag ohne Ende
            if match_tag(&bytes[lt..(lt + 40).min(bytes.len())]).is_some() {
                break;
            }
            acc.emit("<");
            i = lt + 1;
            continue;
        };
        let gt_pos = gt as usize;
        i = gt_pos + 1;
        if !close && RAW.contains(&name.as_str()) {
            let key = format!("</{name}");
            let end = if ends.get(&key) == Some(&-1) { -1 } else { lower[i..].find(&key).map(|p| (p + i) as i64).unwrap_or(-1) };
            ends.insert(key, end);
            if end < 0 {
                break;
            }
            i = end as usize;
            continue;
        }
        if name == "title" {
            acc.in_title = !close && acc.title.is_empty();
            continue;
        }
        if DROP.contains(&name.as_str()) && (strict || name == "head") {
            if !close {
                if bytes[gt_pos - 1] != b'/' {
                    acc.dropped.push(name);
                }
            } else if let Some(pos) = acc.dropped.iter().rposition(|n| *n == name) {
                acc.dropped.truncate(pos);
            }
            acc.emit(" ");
            continue;
        }
        let delta = if close { -1 } else { 1 };
        if name == "main" || name == "article" {
            acc.in_main = (acc.in_main + delta).max(0);
        }
        if name == "header" {
            acc.in_header = (acc.in_header + delta).max(0);
            acc.emit(" ");
            continue;
        }
        let heading = name.len() == 2 && name.starts_with('h') && matches!(name.as_bytes()[1], b'1'..=b'6');
        if heading {
            if close {
                acc.emit("\n\n");
            } else {
                acc.emit(format!("\n\n{} ", "#".repeat((name.as_bytes()[1] - b'0') as usize)));
            }
        } else if name == "li" && !close {
            acc.emit("\n- ");
        } else if name == "br" {
            acc.emit("\n");
        } else if (name == "td" || name == "th") && !close {
            acc.emit(" | ");
        } else if BREAK.contains(&name.as_str()) && close {
            acc.emit("\n");
        } else {
            acc.emit(" ");
        }
    }
    // Ein Bereich wie <nav> wurde nie geschlossen: Dann fiele der ganze Rest weg. Nochmals, ohne Bereiche auszulassen.
    if strict && acc.dropped.iter().any(|n| n != "head") {
        return html_to_text_with(h, false);
    }
    let main = clean(acc.pieces.iter().filter(|p| p.1).map(|p| p.0.as_ref()));
    let title = decode_entities(re!(r"\s+").replace_all(&acc.title, " ").trim());
    let text = if main.chars().count() > 400 { main } else { clean(acc.pieces.iter().filter(|p| p.2).map(|p| p.0.as_ref())) };
    HtmlText { title, text }
}

/// Ergebnisse von lite.duckduckgo.com: Links sind teils Weiterleitungen (…/l/?uddg=<Adresse>)
pub fn parse_duck_duck_go(html: &str) -> Vec<SearchResult> {
    let mut out = Vec::new();
    let links: Vec<&str> = re!(r#"(?is)<a[^>]*class=['"]result-link['"][^>]*>.*?</a>"#).find_iter(html).map(|m| m.as_str()).collect();
    let snippets: Vec<&str> = re!(r#"(?is)<td[^>]*class=['"]result-snippet['"][^>]*>(.*?)</td>"#).captures_iter(html).filter_map(|c| c.get(1)).map(|m| m.as_str()).collect();
    let strip = |s: &str| {
        let text = decode_entities(&re!(r"<[^>]+>").replace_all(s, " "));
        re!(r"\s+").replace_all(&text, " ").trim().to_string()
    };
    for (i, m) in links.iter().enumerate() {
        let mut href = decode_entities(re!(r#"(?i)href=['"]([^'"]+)['"]"#).captures(m).and_then(|c| c.get(1)).map(|x| x.as_str()).unwrap_or(""));
        if let Some(r) = re!(r"[?&]uddg=([^&]+)").captures(&href).and_then(|c| c.get(1)).map(|x| x.as_str().to_string()) {
            href = percent_encoding::percent_decode_str(&r).decode_utf8_lossy().to_string();
        }
        if href.starts_with("//") {
            href = format!("https:{href}");
        }
        if !re!(r"(?i)^https?://").is_match(&href) || re!(r"duckduckgo\.com/(y\.js|l/)").is_match(&href) {
            continue;
        }
        out.push(SearchResult { url: href, title: strip(m), snippet: strip(snippets.get(i).copied().unwrap_or("")) });
    }
    out
}

/// Nur öffentliche http(s)-Adressen; keine Adressen im eigenen Rechner oder Schulnetz
pub fn check_url(raw: &str, allow_local: bool) -> Result<url::Url> {
    let url = url::Url::parse(raw.trim()).map_err(|_| anyhow!("Keine gültige Adresse: {raw}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("Nur http- und https-Adressen.");
    }
    // «localhost.» mit Punkt am Ende ist dieselbe Adresse
    let host = url.host_str().unwrap_or("");
    let host = host.strip_prefix('[').unwrap_or(host);
    let host = host.strip_suffix(']').unwrap_or(host).to_lowercase();
    let host = host.trim_end_matches('.').to_string();
    // IPv4 in IPv6-Schreibweise (::ffff:7f00:1 ist 127.0.0.1) zählt wie die IPv4-Adresse
    let v4 = match re!(r"^::ffff:([0-9a-f]{1,4}):([0-9a-f]{1,4})$").captures(&host) {
        Some(m) => {
            let a = u32::from_str_radix(&m[1], 16).unwrap_or(0);
            let b = u32::from_str_radix(&m[2], 16).unwrap_or(0);
            format!("{}.{}.{}.{}", a >> 8, a & 255, b >> 8, b & 255)
        }
        None => host.clone(),
    };
    let local = host == "localhost"
        || host.ends_with(".localhost")
        || host.ends_with(".local")
        || host.ends_with(".internal")
        || host.ends_with(".lan")
        || host == "::1"
        || host == "::"
        || re!(r"^fe[89ab][0-9a-f]:").is_match(&host)
        || re!(r"^(127|10|0)\.").is_match(&v4)
        || re!(r"^192\.168\.").is_match(&v4)
        || re!(r"^169\.254\.").is_match(&v4)
        || re!(r"^172\.(1[6-9]|2\d|3[01])\.").is_match(&v4)
        || re!(r"^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\.").is_match(&v4) // gemeinsam genutzter Bereich von Anbietern und VPN
        || re!(r"^(fc|fd)[0-9a-f]{2}:").is_match(&host)
        || (!host.contains('.') && !host.contains(':'));
    if local && !allow_local {
        bail!(LOCAL_BLOCKED);
    }
    Ok(url)
}

/// Adresse im eigenen Rechner, im lokalen Netz oder sonst nicht öffentlich (wie `check_url`, aber für die
/// Adressen, die ein Name ergibt)
pub fn is_local_ip(ip: std::net::IpAddr) -> bool {
    use std::net::IpAddr;
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || a == 0
                || (a == 100 && (64..=127).contains(&b)) // gemeinsam genutzter Bereich von Anbietern und VPN
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_local_ip(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            let first = seg[0];
            let embedded = |hi: u16, lo: u16| is_local_ip(IpAddr::V4(std::net::Ipv4Addr::from((u32::from(hi) << 16) | u32::from(lo))));
            // IPv4 in IPv6: NAT64 (64:ff9b::/96), 6to4 (2002::/16), alt «kompatibel» (::a.b.c.d)
            if seg[..6] == [0x64, 0xff9b, 0, 0, 0, 0] && embedded(seg[6], seg[7]) {
                return true;
            }
            if first == 0x2002 && embedded(seg[1], seg[2]) {
                return true;
            }
            if seg[..6] == [0, 0, 0, 0, 0, 0] && seg[6] != 0 && embedded(seg[6], seg[7]) {
                return true;
            }
            v6.is_loopback() || v6.is_unspecified() || v6.is_multicast() || (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
                || (first & 0xffc0) == 0xfec0 // site-local (veraltet, aber lokal)
        }
    }
}

/// Ein öffentlicher Name, der auf eine lokale Adresse zeigt (z. B. «127.0.0.1.nip.io» oder ein umgebogener
/// DNS-Eintrag), kommt an `check_url` vorbei. Darum den Namen selbst auflösen und ablehnen, sobald eine Adresse
/// lokal ist (bei gemischten Antworten könnte die Verbindung sonst zur lokalen gehen). Lässt er sich nicht auflösen (z. B. hinter einem Proxy, der selbst auflöst), entscheidet der Proxy.
async fn check_resolved(url: &url::Url, allow_local: bool) -> Result<()> {
    if allow_local {
        return Ok(());
    }
    let Some(url::Host::Domain(host)) = url.host() else { return Ok(()) }; // IP-Adressen prüft check_url
    let port = url.port_or_known_default().unwrap_or(443);
    let lookup = tokio::time::timeout(Duration::from_secs(5), tokio::net::lookup_host((host, port))).await;
    let Ok(Ok(addrs)) = lookup else { return Ok(()) };
    let ips: Vec<std::net::IpAddr> = addrs.map(|a| a.ip()).collect();
    if ips.iter().any(|ip| is_local_ip(*ip)) {
        bail!(LOCAL_BLOCKED);
    }
    Ok(())
}

fn timeout_error() -> anyhow::Error {
    anyhow!("Keine Antwort innerhalb von 20 Sekunden.")
}

async fn read_limited(res: reqwest::Response, signal: Option<&Signal>) -> Result<Vec<u8>> {
    let mut stream = res.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    loop {
        let next = match signal {
            Some(s) => tokio::select! {
                _ = s.cancelled() => return Err(AskError::abort().into()),
                n = stream.next() => n,
            },
            None => stream.next().await,
        };
        match next {
            None => break,
            Some(Err(e)) if e.is_timeout() => return Err(timeout_error()),
            Some(Err(e)) => return Err(e.into()),
            Some(Ok(chunk)) => {
                if buf.len() + chunk.len() > MAX_BYTES {
                    bail!("Die Seite ist grösser als 3 MB.");
                }
                buf.extend_from_slice(&chunk);
            }
        }
    }
    Ok(buf)
}

/// Zeichensatz aus dem Content-Type oder dem `<meta charset>`; ohne Angabe UTF-8
fn charset_of(content_type: &str, buf: &[u8]) -> String {
    let head: String = buf[..buf.len().min(2048)].iter().map(|b| *b as char).collect();
    let found = re!(r"(?i)charset=([A-Za-z0-9_:.-]+)")
        .captures(content_type)
        .or_else(|| re!(r#"(?i)<meta[^>]+charset=['"]?([A-Za-z0-9_:.-]+)"#).captures(&head))
        .map(|m| m[1].to_lowercase());
    found.unwrap_or_else(|| "utf-8".to_string())
}

/// Wie `TextDecoder(charset).decode(buf)` im Browser (Namen nach WHATWG, «iso-8859-1» ist windows-1252,
/// eine BOM hat Vorrang). Vorher wurden z. B. iso-8859-15, windows-1250 und utf-16be als UTF-8 gelesen und
/// die Umlaute zu «�». Unbekannte Namen: UTF-8.
fn decode_text(buf: &[u8], charset: &str) -> String {
    let encoding = encoding_rs::Encoding::for_label(charset.as_bytes()).unwrap_or(encoding_rs::UTF_8);
    encoding.decode(buf).0.into_owned()
}

/// Die Websuche von ASK, wie `Web` sie braucht (die `AskSession` ist eine; Tests nehmen eine Attrappe)
#[async_trait]
pub trait WebSearchBackend: Send + Sync {
    /// Websuche in ASK eingeschaltet und für dieses Konto erlaubt
    fn web_search(&self) -> bool;
    async fn search_web(&self, query: &str, signal: Option<Signal>) -> Result<Vec<SearchResult>>;
}

#[async_trait]
impl WebSearchBackend for AskSession {
    fn web_search(&self) -> bool {
        AskSession::web_search(self)
    }
    async fn search_web(&self, query: &str, signal: Option<Signal>) -> Result<Vec<SearchResult>> {
        AskSession::search_web(self, query, signal).await
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SearchResponse {
    pub engine: String,
    pub results: Vec<SearchResult>,
    /// Hinweis, warum nicht ASK gesucht hat (nur bei DuckDuckGo)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PageResult {
    pub url: String,
    pub title: String,
    pub text: String,
    /// bei PDF: Pfad der gespeicherten Datei relativ zum Projekt
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pdf: Option<String>,
    pub total: usize,
    pub from: usize,
    pub to: usize,
}

#[derive(Clone, Default)]
pub struct PageOptions {
    pub signal: Option<Signal>,
    /// ab diesem Zeichen weiterlesen
    pub ab: i64,
    /// Projektordner und Chat-Kennung: nur für PDF nötig
    pub root: Option<std::path::PathBuf>,
    pub chat: Option<String>,
}

pub struct Web {
    ask: Option<Arc<dyn WebSearchBackend>>,
    client: reqwest::Client,
    pub allow_local: bool,
    /// Weiterleitungen ins lokale Netz erlaubt (Standard wie allow_local; Tests prüfen damit die Sperre)
    pub redirect_local: bool,
    /// Websuche von ASK ging nicht: in dieser Sitzung gleich DuckDuckGo
    ask_failed: AtomicBool,
    /// Adresse von DuckDuckGo (für Tests änderbar)
    pub duck_url: String,
}

impl Web {
    /// ask: Websuche von ASK (kann fehlen), client: HTTP mit Proxy von Windows (siehe `client_builder`)
    pub fn new(ask: Option<Arc<dyn WebSearchBackend>>, client: reqwest::Client, allow_local: bool) -> Self {
        Web { ask, client, allow_local, redirect_local: allow_local, ask_failed: AtomicBool::new(false), duck_url: "https://lite.duckduckgo.com".to_string() }
    }

    /// HTTP-Client für `Web`: Weiterleitungen (höchstens 10) werden vor dem Verbinden geprüft, damit eine
    /// öffentliche Seite niemanden ins lokale Netz schicken kann (in Electron wurde erst die Zieladresse geladen
    /// und danach geprüft).
    pub fn client_builder(allow_local: bool) -> reqwest::ClientBuilder {
        reqwest::Client::builder().redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 10 {
                return attempt.error("Zu viele Weiterleitungen.");
            }
            match check_url(attempt.url().as_str(), allow_local) {
                Ok(_) => attempt.follow(),
                Err(e) => attempt.error(e.to_string()),
            }
        }))
    }

    /// Client für Seiten und Suche: folgt Weiterleitungen nicht selbst; `get` prüft jedes Ziel samt DNS, bevor
    /// es angefragt wird (sonst ging eine Anfrage an ein lokales Ziel hinaus, bevor die Prüfung danach griff)
    pub fn fetch_client_builder(allow_local: bool) -> reqwest::ClientBuilder {
        Self::client_builder(allow_local).redirect(reqwest::redirect::Policy::none())
    }

    async fn get(&self, url: &str, signal: Option<&Signal>, accept: &str) -> Result<reqwest::Response> {
        let mut current = url::Url::parse(url).map_err(|_| anyhow!("Ungültige Adresse."))?;
        for _ in 0..=10 {
            let res = self.get_once(current.as_str(), signal, accept).await?;
            if !res.status().is_redirection() {
                return Ok(res);
            }
            let Some(location) = res.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()) else { return Ok(res) };
            let next = current.join(location).map_err(|_| anyhow!("Ungültige Weiterleitung."))?;
            let next = check_url(next.as_str(), self.redirect_local)?;
            check_resolved(&next, self.redirect_local).await?;
            current = next;
        }
        bail!("Zu viele Weiterleitungen.")
    }

    async fn get_once(&self, url: &str, signal: Option<&Signal>, accept: &str) -> Result<reqwest::Response> {
        let sent = self
            .client
            .get(url)
            .timeout(TIMEOUT)
            .header("User-Agent", AGENT)
            .header("Accept", accept)
            .header("Accept-Language", "de-CH,de;q=0.9,en;q=0.7")
            .send();
        let res = match signal {
            Some(s) => tokio::select! {
                _ = s.cancelled() => return Err(AskError::abort().into()),
                r = sent => r,
            },
            None => sent.await,
        };
        res.map_err(|e| {
            if e.is_timeout() {
                return timeout_error();
            }
            if e.is_redirect() {
                use std::error::Error;
                if let Some(source) = e.source() {
                    return anyhow!(source.to_string());
                }
            }
            anyhow!("fetch failed: {e}")
        })
    }

    /// { engine, results: [{ url, title, snippet }] }
    pub async fn search(&self, query: &str, signal: Option<&Signal>) -> Result<SearchResponse> {
        let query = query.trim();
        if query.is_empty() {
            bail!("Leere Suchanfrage.");
        }
        let mut note = String::new();
        if let Some(ask) = self.ask.as_ref().filter(|a| a.web_search() && !self.ask_failed.load(Ordering::SeqCst)) {
            match ask.search_web(query, signal.cloned()).await {
                Ok(results) if !results.is_empty() => {
                    return Ok(SearchResponse { engine: "ASK".into(), results: results.into_iter().take(MAX_RESULTS).collect(), note: None });
                }
                Ok(_) => note = "ASK fand nichts".to_string(),
                Err(err) => {
                    if signal.is_some_and(|s| s.is_cancelled()) {
                        return Err(err);
                    }
                    self.ask_failed.store(true, Ordering::SeqCst);
                    note = err.to_string();
                }
            }
        }
        let q = percent_encoding::utf8_percent_encode(query, percent_encoding::NON_ALPHANUMERIC).to_string();
        let res = self.get(&format!("{}/lite/?q={q}&kl=ch-de", self.duck_url), signal, "text/html,*/*;q=0.8").await?;
        if !res.status().is_success() {
            let extra = if note.is_empty() { String::new() } else { format!(" ({note})") };
            bail!("DuckDuckGo antwortet mit HTTP {}{extra}.", res.status().as_u16());
        }
        let html = String::from_utf8_lossy(&read_limited(res, signal).await?).to_string();
        let results = parse_duck_duck_go(&html).into_iter().take(MAX_RESULTS).collect();
        Ok(SearchResponse { engine: "DuckDuckGo".into(), results, note: Some(note) })
    }

    /// Seite als Text. ab: ab diesem Zeichen weiterlesen. PDF wird gespeichert und mit read_file gelesen.
    pub async fn page(&self, raw_url: &str, opts: PageOptions) -> Result<PageResult> {
        let signal = opts.signal.as_ref();
        let url = check_url(raw_url, self.allow_local)?;
        check_resolved(&url, self.allow_local).await?;
        let res = self.get(url.as_str(), signal, "text/html,*/*;q=0.8").await?;
        if !res.status().is_success() {
            bail!("Die Seite antwortet mit HTTP {}.", res.status().as_u16());
        }
        let final_url = res.url().to_string();
        let checked = check_url(&final_url, self.allow_local)?; // Weiterleitung ins lokale Netz
        if checked.host_str() != url.host_str() {
            check_resolved(&checked, self.allow_local).await?; // Inhalt eines lokalen Ziels nicht weitergeben
        }
        let ctype = res.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
        let buf = read_limited(res, signal).await?;
        if re!(r"(?i)pdf").is_match(&ctype) || buf.starts_with(b"%PDF-") {
            let (Some(root), Some(chat)) = (&opts.root, &opts.chat) else { bail!("PDF kann hier nicht gespeichert werden.") };
            let path = url::Url::parse(&final_url).map(|u| u.path().to_string()).unwrap_or_default();
            let base = path.trim_end_matches('/').rsplit('/').next().unwrap_or("").to_string();
            let safe = re!(r"[^A-Za-z0-9_.-]+").replace_all(&base, "_").to_string();
            let safe = if safe.is_empty() { "dokument".to_string() } else { safe };
            let stem = if safe.to_lowercase().ends_with(".pdf") { safe[..safe.len() - 4].to_string() } else { safe };
            let dir = root.join(".ask").join("arbeit").join(chat).join("web");
            std::fs::create_dir_all(&dir)?;
            // Gleicher Name, anderes PDF (z. B. zweimal «download.pdf» von verschiedenen Seiten): nicht überschreiben,
            // sonst läse das Modell später das falsche. Ein Kürzel der Adresse macht den Namen eindeutig.
            let mut name = format!("{stem}.pdf");
            if std::fs::read(dir.join(&name)).is_ok_and(|old| old != buf) {
                use sha1::Digest;
                let tag = hex::encode(sha1::Sha1::digest(final_url.as_bytes()));
                name = format!("{stem}-{}.pdf", &tag[..8]);
            }
            let rel = format!(".ask/arbeit/{chat}/web/{name}");
            std::fs::write(dir.join(&name), &buf)?;
            return Ok(PageResult { url: final_url, title: name, text: String::new(), pdf: Some(rel), total: 0, from: 0, to: 0 });
        }
        if !re!(r"(?i)html|xml|text|json").is_match(&ctype) && !ctype.is_empty() {
            bail!("Kein Text, sondern {}.", ctype.split(';').next().unwrap_or(""));
        }
        let raw = decode_text(&buf, &charset_of(&ctype, &buf));
        let is_html = re!(r"(?i)html|xml").is_match(&ctype) || re!(r"(?i)<html|<body|<p[\s>]").is_match(&take_chars(&raw, 5000));
        let HtmlText { title, text } = if is_html { html_to_text(&raw) } else { HtmlText { title: String::new(), text: raw.trim().to_string() } };
        let total = text.chars().count();
        let from = (opts.ab.max(0) as usize).min(total);
        let shown: String = text.chars().skip(from).take(PAGE_CHARS).collect();
        let title = if title.is_empty() { url::Url::parse(&final_url).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default() } else { title };
        Ok(PageResult { url: final_url, title, text: shown, pdf: None, total, from, to: total.min(from + PAGE_CHARS) })
    }
}

/// Geht eine Anfrage an `url` direkt oder über einen Proxy? Für die Meldung «ASK ist nicht erreichbar» (ENTSCHEIDE E57),
/// wie `session.resolveProxy` in Electron: «DIRECT», «PROXY host:port» oder «PAC <Adresse>». Liest die Umgebung
/// (HTTPS_PROXY, ALL_PROXY, NO_PROXY) und unter Windows die Einstellungen von Windows (Internetoptionen des Kontos).
pub fn proxy_for(url: &str) -> String {
    let host = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default();
    let env = |k: &str| std::env::var(k).or_else(|_| std::env::var(k.to_lowercase())).unwrap_or_default();
    let no_proxy = env("NO_PROXY");
    let excluded = no_proxy.split(',').map(str::trim).filter(|p| !p.is_empty())
        .any(|p| p == "*" || host == p.trim_start_matches('.') || host.ends_with(&format!(".{}", p.trim_start_matches('.'))));
    if !excluded {
        for k in ["HTTPS_PROXY", "ALL_PROXY"] {
            let v = env(k);
            if !v.is_empty() {
                return format!("PROXY {}", v.trim_start_matches("http://").trim_start_matches("https://").trim_end_matches('/'));
            }
        }
    }
    #[cfg(windows)]
    {
        if let Some(p) = windows_proxy() {
            return p;
        }
    }
    "DIRECT".into()
}

#[cfg(windows)]
fn windows_proxy() -> Option<String> {
    use std::os::windows::process::CommandExt;
    let out = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings"])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let value = |name: &str| -> Option<String> {
        text.lines().find_map(|l| {
            let parts: Vec<&str> = l.split_whitespace().collect();
            (parts.len() >= 3 && parts[0].eq_ignore_ascii_case(name)).then(|| parts[2..].join(" "))
        })
    };
    if let Some(pac) = value("AutoConfigURL").filter(|v| !v.is_empty()) {
        return Some(format!("PAC {pac}"));
    }
    if value("ProxyEnable").is_some_and(|v| v.trim() == "0x1") {
        if let Some(server) = value("ProxyServer").filter(|v| !v.is_empty()) {
            // «http=host:port;https=host:port» oder «host:port»
            let https = server.split(';').find_map(|p| p.strip_prefix("https=")).unwrap_or(&server).to_string();
            return Some(format!("PROXY {https}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::askapi::test_http::{serve, Resp};
    
    #[test]
    fn html_wird_zu_lesbarem_text() {
        let HtmlText { title, text } = html_to_text(
            r#"<html><head><title>Test &amp; Co</title><style>p{}</style></head><body>
    <nav>Menü</nav><header>Kopf</header><h2>Abschnitt</h2><p>Grüsse&nbsp;aus &laquo;Bern&raquo; &#8211; &#x263A;</p>
    <ul><li>eins</li><li>zwei</li></ul><table><tr><td>A</td><td>B</td></tr></table><script>alert(1)</script><footer>Fuss</footer></body></html>"#,
        );
        assert_eq!(title, "Test & Co");
        assert!(text.contains("## Abschnitt"), "{text}");
        assert!(text.contains("Grüsse aus «Bern» – ☺"), "{text}");
        assert!(text.contains("- eins\n- zwei"), "{text}");
        assert!(text.contains("| A | B"), "{text}");
        assert!(!["Menü", "Kopf", "Fuss", "alert", "p{}"].iter().any(|bad| text.contains(bad)), "{text}");
    }

    #[test]
    fn html_ungeschlossene_bereiche_und_kaputte_seiten() {
        // <nav> nie geschlossen: der Rest bleibt trotzdem lesbar
        let t = html_to_text("<body><nav>Menü<p>Inhalt bleibt</p></body>");
        assert!(t.text.contains("Inhalt bleibt"), "{}", t.text);
        // offener Kommentar und halbes Tag am Ende
        assert_eq!(html_to_text("a<!-- offen b").text, "a");
        assert_eq!(html_to_text("a < b <div").text, "a < b");
        // tausende offene Tags blockieren nicht
        let big = "<div>".repeat(200_000) + "Ende";
        assert!(html_to_text(&big).text.contains("Ende"));
    }

    #[tokio::test]
    async fn namen_mit_lokaler_adresse_werden_abgelehnt() {
        for ip in ["127.0.0.1", "10.1.2.3", "192.168.0.1", "172.20.0.1", "169.254.1.1", "100.64.0.1", "0.0.0.0", "::1", "fd00::1", "fe80::1", "::ffff:127.0.0.1", "64:ff9b::7f00:1", "2002:c0a8:1::1", "::7f00:1", "fec0::1"] {
            assert!(is_local_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "195.176.1.1", "2a00:1450::1", "64:ff9b::808:808", "2002:808:808::1"] {
            assert!(!is_local_ip(ip.parse().unwrap()), "{ip}");
        }
        // «localhost» ist ein Name, aber immer lokal: wird nach dem Auflösen abgelehnt (ohne Ausnahme für Tests)
        let local = url::Url::parse("http://localhost.example.invalid/").unwrap();
        assert!(check_resolved(&local, false).await.is_ok(), "nicht auflösbar: der Proxy entscheidet");
        let lh = url::Url::parse("http://localhost:1/").unwrap();
        assert_eq!(check_resolved(&lh, false).await.unwrap_err().to_string(), LOCAL_BLOCKED);
        assert!(check_resolved(&lh, true).await.is_ok());
    }

    #[test]
    fn seiten_in_anderen_zeichensaetzen() {
        let dec = |ct: &str, buf: &[u8]| decode_text(buf, &charset_of(ct, buf));
        assert_eq!(dec("text/html; charset=ISO-8859-15", b"Gr\xfcezi \xa4"), "Grüezi €");
        assert_eq!(dec("text/html; charset=windows-1250", b"\x8a"), "Š");
        assert_eq!(dec("text/html", b"<meta charset=\"iso-8859-1\">\xe4\x80"), "<meta charset=\"iso-8859-1\">ä€", "latin1 wie im Browser: windows-1252");
        assert_eq!(dec("text/html; charset=utf-16be", &[0, b'a', 0, 0xfc]), "aü");
        assert_eq!(dec("text/html", "\u{feff}Grüezi".as_bytes()), "Grüezi", "BOM entfernt");
        assert_eq!(dec("text/html; charset=gibt-es-nicht", "ä".as_bytes()), "ä");
    }

    #[test]
    fn treffer_von_duckduckgo_weiterleitungen_aufgeloest() {
        let html = r#"<table><tr><td><a rel="nofollow" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.proxmox.com%2Fde%2Fnews&amp;rut=x" class='result-link'>Proxmox <b>News</b></a></td></tr>
    <tr><td class='result-snippet'>Neue <b>Version</b> 9</td></tr>
    <tr><td><a rel="nofollow" href="https://pve.proxmox.com/wiki/Roadmap" class='result-link'>Roadmap</a></td></tr>
    <tr><td class='result-snippet'>Pläne</td></tr>
    <tr><td><a href="https://duckduckgo.com/y.js?ad=1" class='result-link'>Werbung</a></td></tr></table>"#;
        let r = |url: &str, title: &str, snippet: &str| SearchResult { url: url.into(), title: title.into(), snippet: snippet.into() };
        assert_eq!(
            parse_duck_duck_go(html),
            vec![r("https://www.proxmox.com/de/news", "Proxmox News", "Neue Version 9"), r("https://pve.proxmox.com/wiki/Roadmap", "Roadmap", "Pläne")]
        );
    }

    #[test]
    fn nur_oeffentliche_http_s_adressen() {
        assert_eq!(check_url("https://example.ch/a", false).unwrap().host_str(), Some("example.ch"));
        for bad in [
            "file:///C:/x", "http://localhost:3000", "http://127.0.0.1/", "http://192.168.1.10", "http://10.0.0.1", "http://172.20.1.1", "http://[::1]/",
            "http://intranet/", "ftp://example.ch", "kein link",
        ] {
            assert!(check_url(bad, false).is_err(), "{bad}");
        }
        assert_eq!(check_url("http://127.0.0.1:8080/x", true).unwrap().port().map(|p| p.to_string()), Some("8080".to_string()));
    }

    #[test]
    fn lokale_adressen_auch_in_anderen_schreibweisen_gesperrt() {
        for u in [
            "http://localhost./", "http://foo.localhost./", "http://[::ffff:127.0.0.1]/", "http://[::ffff:7f00:1]/", "http://[::]/", "http://[0:0:0:0:0:0:0:0]/",
            "http://100.64.1.1/", "http://127.0.0.1./", "http://[::ffff:c0a8:101]/",
        ] {
            let err = check_url(u, false).unwrap_err();
            assert!(err.to_string().contains("gesperrt"), "{u}: {err}");
        }
        for u in ["https://www.tfbern.ch/", "https://100.200.1.1/", "https://[2001:db8::1]/"] {
            assert!(check_url(u, false).is_ok(), "{u}");
        }
    }

    struct FakeAsk {
        fail: bool,
    }

    #[async_trait]
    impl WebSearchBackend for FakeAsk {
        fn web_search(&self) -> bool {
            true
        }
        async fn search_web(&self, query: &str, _signal: Option<Signal>) -> Result<Vec<SearchResult>> {
            if self.fail {
                bail!("HTTP 403");
            }
            Ok(vec![SearchResult { url: "https://ask.ch/x".into(), title: format!("ASK {query}"), snippet: String::new() }])
        }
    }

    fn test_client() -> reqwest::Client {
        Web::fetch_client_builder(false).no_proxy().build().unwrap()
    }

    #[tokio::test]
    async fn suche_ask_zuerst_sonst_duckduckgo_seite_als_text_pdf_gespeichert_lokale_weiterleitung_gesperrt() {
        let ddg = r#"<a href="https://a.ch/" class='result-link'>A</a><td class='result-snippet'>a</td>"#;
        let page = format!("<title>Seite</title><main><p>{}</p></main>", "Inhalt ".repeat(3000));
        let server = serve(move |req| {
            let p = req.path.as_str();
            if p.starts_with("/lite/") {
                Resp::text(200, "text/html; charset=utf-8", ddg)
            } else if p.contains("pdf") {
                Resp::text(200, "application/pdf", format!("%PDF-1.4 {p}").into_bytes())
            } else if p.contains("weiter") {
                Resp::redirect("http://192.168.1.1/")
            } else {
                Resp::text(200, "text/html; charset=utf-8", page.clone())
            }
        })
        .await;
        let ask: Arc<dyn WebSearchBackend> = Arc::new(FakeAsk { fail: false });
        let mut web = Web::new(Some(ask), test_client(), true);
        web.redirect_local = false; // der Testserver ist lokal, eine Weiterleitung ins lokale Netz bleibt gesperrt
        web.duck_url = server.url.clone();
        assert_eq!(
            web.search("test", None).await.unwrap(),
            SearchResponse { engine: "ASK".into(), results: vec![SearchResult { url: "https://ask.ch/x".into(), title: "ASK test".into(), snippet: String::new() }], note: None }
        );
        assert_eq!(server.reqs.lock().len(), 0);

        let mut failing = Web::new(Some(Arc::new(FakeAsk { fail: true })), test_client(), true);
        failing.duck_url = server.url.clone();
        let r = failing.search("test", None).await.unwrap();
        assert_eq!(r.engine, "DuckDuckGo");
        assert_eq!(r.note.as_deref(), Some("HTTP 403"));
        assert_eq!(r.results[0].url, "https://a.ch/");
        failing.search("noch", None).await.unwrap();
        assert_eq!(server.paths().iter().filter(|p| p.starts_with("/lite/")).count(), 2, "nach Fehler gleich DuckDuckGo");

        let base = &server.url;
        let p = web.page(&format!("{base}/"), PageOptions::default()).await.unwrap();
        assert_eq!(p.title, "Seite");
        assert_eq!(p.from, 0);
        assert_eq!(p.text.chars().count(), 12000);
        assert!(p.total > 12000);
        let p2 = web.page(&format!("{base}/"), PageOptions { ab: p.to as i64, ..Default::default() }).await.unwrap();
        assert_eq!(p2.from, 12000);

        let root = tempfile::tempdir().unwrap();
        let pdf = web.page(&format!("{base}/datei.pdf"), PageOptions { root: Some(root.path().to_path_buf()), chat: Some("c1".into()), ..Default::default() }).await.unwrap();
        assert_eq!(pdf.pdf.as_deref(), Some(".ask/arbeit/c1/web/datei.pdf"));
        assert!(std::fs::read(root.path().join(".ask/arbeit/c1/web/datei.pdf")).unwrap().starts_with(b"%PDF"));
        // dasselbe PDF nochmals: gleicher Name; ein anderes mit gleichem Namen: eigener Name, das erste bleibt
        let again = web.page(&format!("{base}/datei.pdf"), PageOptions { root: Some(root.path().to_path_buf()), chat: Some("c1".into()), ..Default::default() }).await.unwrap();
        assert_eq!(again.pdf.as_deref(), Some(".ask/arbeit/c1/web/datei.pdf"));
        let other = web.page(&format!("{base}/anders/datei.pdf"), PageOptions { root: Some(root.path().to_path_buf()), chat: Some("c1".into()), ..Default::default() }).await.unwrap();
        let rel = other.pdf.unwrap();
        assert!(rel.starts_with(".ask/arbeit/c1/web/datei-") && rel.ends_with(".pdf"), "{rel}");
        assert!(std::fs::read_to_string(root.path().join(&rel)).unwrap().contains("/anders/datei.pdf"));
        assert!(std::fs::read_to_string(root.path().join(".ask/arbeit/c1/web/datei.pdf")).unwrap().ends_with(" /datei.pdf"));
        assert!(web.page(&format!("{base}/datei.pdf"), PageOptions::default()).await.unwrap_err().to_string().contains("PDF kann hier nicht gespeichert"));

        let err = web.page(&format!("{base}/weiter"), PageOptions::default()).await.unwrap_err();
        assert!(err.to_string().contains("lokalen Netz"), "{err}");
    }

    #[tokio::test]
    async fn seite_lokale_adresse_zeichensatz_und_fehler() {
        let server = serve(|req| match req.path.as_str() {
            "/latin" => Resp::text(200, "text/plain; charset=iso-8859-1", &b"Gr\xfcsse \x80"[..]),
            "/json" => Resp::text(200, "application/json", "{\"a\":1}"),
            "/bild" => Resp::text(200, "image/png", &b"\x89PNG"[..]),
            "/weg" => Resp::text(404, "text/plain", "weg"),
            _ => Resp::text(200, "text/html", "<p>x</p>"),
        })
        .await;
        let base = &server.url;
        let blocked = Web::new(None, test_client(), false);
        assert!(blocked.page(&format!("{base}/"), PageOptions::default()).await.unwrap_err().to_string().contains("gesperrt"));
        let web = Web::new(None, test_client(), true);
        assert_eq!(web.page(&format!("{base}/latin"), PageOptions::default()).await.unwrap().text, "Grüsse €");
        assert_eq!(web.page(&format!("{base}/json"), PageOptions::default()).await.unwrap().text, "{\"a\":1}");
        assert!(web.page(&format!("{base}/bild"), PageOptions::default()).await.unwrap_err().to_string().contains("Kein Text, sondern image/png."));
        assert!(web.page(&format!("{base}/weg"), PageOptions::default()).await.unwrap_err().to_string().contains("HTTP 404"));
        assert!(web.search("  ", None).await.unwrap_err().to_string().contains("Leere Suchanfrage"));
    }
}
