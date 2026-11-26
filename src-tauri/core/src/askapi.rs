//! Verbindung mit ASK (Open WebUI) über dieselbe Schnittstelle, die auch die ASK-Webseite benutzt
//! (ENTSCHEIDE E19). Angemeldet wird in der ASK-Seite, auch über das Microsoft-Konto. Danach liest
//! die App das Token aus der Seite und spricht das Modell direkt an. Die Werkzeuge führt die App
//! selbst aus (agent, tools); ASK braucht dafür keine Freigabe für Tool-Server.
//!
//! Portiert aus src/main/askapi.js. Unterschiede zu Electron:
//!
//! * Die ASK-Anmeldeseite ist eine Webansicht der Tauri-Hülle. Die Hülle setzt [`AskView`] um
//!   (Skript ausführen, Adresse, Laden, Cookies löschen) und meldet die Ereignisse der Seite über
//!   Methoden von [`AskSession`] statt über `webContents.on(...)`:
//!   - `did-finish-load` → [`AskSession::on_view_loaded`]
//!   - `did-fail-load` (nur Hauptframe) → [`AskSession::on_view_failed`] (Code -3 = abgebrochen wird ignoriert)
//!   - `did-start-navigation` → [`AskSession::on_view_navigation_started`]
//!   - `did-navigate` und `did-navigate-in-page` → [`AskSession::on_view_navigated`]
//!
//!   `attach(view)` hängt die Ansicht an und startet den Takt (alle 2 Sekunden prüfen); es muss in
//!   einer tokio-Laufzeit aufgerufen werden, deren Handle sich die Sitzung merkt, damit die `on_view_*`-Methoden
//!   auch von anderen Threads (Rückrufe der Webansicht) aus Aufgaben starten können.
//!
//! * `session.fetch` (Chromium mit den ASK-Cookies) ist ein geteilter `reqwest::Client`; entscheidend ist
//!   das Bearer-Token.
//! * `AbortSignal` ist ein `CancellationToken` ([`Signal`]).
//! * `on('status')` und `on('models')` laufen über [`Emitter`] (`AskSession::on`); die Nutzlast ist
//!   der Status als JSON (`{ state, user?, message?, detail?, loadError? }`) beziehungsweise die Modellliste.
//! * Fehler von ASK sind [`AskError`] (mit `status` und `partial` wie die Eigenschaften am Error in JS),
//!   «nicht angemeldet» ist [`AuthError`]; erkennen mit `status_of`, `partial_of`, `is_abort`, `is_auth`.
//!
//! Öffentliche Schnittstelle: `AskSession` (`new`, `with_client`, `attach`, `set_view`, `tick`, `check`,
//! `request`, `raw`, `json`, `load_models`, `search_web`, `chat`, `logout`, `set_base`, `status`, `models`, …),
//! `AskView`, `ChatApi`, `ChatRequest`, `Completion`, `SseParser`, `ThinkSplitter`, `parse_text_tool_calls`,
//! `parse_sources`, `web_search_allowed`, `is_stale_terminal`, `context_of`, `inband_error`, `same_origin`,
//! `nearest_effort`, `thinking_params`, `THINKING_LEVELS`.

#![allow(clippy::regex_creation_in_loops)] // `re!` legt jeden Ausdruck nur einmal an (OnceLock)

use crate::util::{js_string, now_ms, truthy, Emitter};
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use parking_lot::Mutex;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Wie `AbortSignal` in JavaScript: wer abbrechen will, ruft `cancel()`.
pub use tokio_util::sync::CancellationToken as Signal;

macro_rules! re {
    ($p:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($p).expect("regex"))
    }};
}

/// So oft lädt die ASK-Seite nach einem Ladefehler von selbst neu
const RETRY_LOAD_MS: i64 = 15000;
/// So lange darf ASK mitten in einer Antwort schweigen
const STALL_MS: u64 = 5 * 60 * 1000;
/// Grobe Schätzung, wenn ASK keine Tokenzahlen liefert (Deutsch, Englisch und Code gemischt).
const CHARS_PER_TOKEN: f64 = 3.6;

/// Denkaufwand-Stufen des Reglers
pub const THINKING_LEVELS: [&str; 5] = ["standard", "tief", "mittel", "hoch", "maximal"];

/// Zeit in Millisekunden (für Tests austauschbar)
pub type NowFn = Arc<dyn Fn() -> i64 + Send + Sync>;
/// `onDelta(kind, text)` mit kind «text» oder «reasoning»
pub type DeltaFn = Arc<dyn Fn(&str, &str) + Send + Sync>;

// ---------- Fehler ----------

/// Nicht (mehr) bei ASK angemeldet
#[derive(Debug, Clone)]
pub struct AuthError(pub String);

impl AuthError {
    pub fn new(message: &str) -> Self {
        AuthError(message.to_string())
    }
}

impl std::fmt::Display for AuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for AuthError {}

/// Fehler einer Anfrage an ASK. `status` ist der HTTP-Status (0: keiner, z. B. Verbindungsfehler),
/// `partial` die bis dahin angekommene Antwort (wie `err.partial` in JS), `aborted` ein Abbruch durch
/// die Person (in JS `name === 'AbortError'`).
pub struct AskError {
    pub message: String,
    pub status: u16,
    pub partial: Option<Box<Completion>>,
    pub aborted: bool,
}

impl AskError {
    pub fn new(message: impl Into<String>) -> Self {
        AskError { message: message.into(), status: 0, partial: None, aborted: false }
    }
    pub fn with_status(mut self, status: u16) -> Self {
        self.status = status;
        self
    }
    pub fn with_partial(mut self, partial: Completion) -> Self {
        self.partial = Some(Box::new(partial));
        self
    }
    /// Abbruch durch Stopp
    pub fn abort() -> Self {
        AskError { message: "abgebrochen".to_string(), status: 0, partial: None, aborted: true }
    }
}

impl std::fmt::Display for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::fmt::Debug for AskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AskError")
            .field("message", &self.message)
            .field("status", &self.status)
            .field("aborted", &self.aborted)
            .field("partial", &self.partial.is_some())
            .finish()
    }
}

impl std::error::Error for AskError {}

/// HTTP-Status eines Fehlers von ASK (`err.status`), falls er einen hat
pub fn status_of(err: &anyhow::Error) -> Option<u16> {
    err.downcast_ref::<AskError>().map(|e| e.status).filter(|s| *s != 0)
}

/// Was bis zum Fehler angekommen war (`err.partial`)
pub fn partial_of(err: &anyhow::Error) -> Option<&Completion> {
    err.downcast_ref::<AskError>().and_then(|e| e.partial.as_deref())
}

/// Abbruch durch Stopp (`err.name === 'AbortError'`)
pub fn is_abort(err: &anyhow::Error) -> bool {
    err.downcast_ref::<AskError>().is_some_and(|e| e.aborted)
}

/// «Nicht angemeldet» (`AuthError`)
pub fn is_auth(err: &anyhow::Error) -> bool {
    err.downcast_ref::<AuthError>().is_some()
}

/// Verbindungsfehler: Die Texte enthalten «fetch failed», damit die Wiederholungslogik des Agenten sie erkennt.
fn net_error(err: &reqwest::Error) -> AskError {
    AskError::new(format!("fetch failed: {}", error_chain(err)))
}

fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(s) = source {
        let part = s.to_string();
        if !text.contains(&part) {
            text.push_str(": ");
            text.push_str(&part);
        }
        source = s.source();
    }
    text
}

// ---------- Server-Sent Events ----------

/// Zerlegt einen Server-Sent-Events-Strom in JSON-Ereignisse («data: {...}»).
/// Anders als in JS (Rückruf im Konstruktor) liefert `push` die Ereignisse als Liste zurück.
#[derive(Default)]
pub struct SseParser {
    buffer: String,
    /// `[DONE]` gesehen
    pub done: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, text: &str) -> Vec<Value> {
        self.buffer.push_str(text);
        let mut events = Vec::new();
        while let Some(i) = self.buffer.find('\n') {
            let line: String = self.buffer[..i].strip_suffix('\r').unwrap_or(&self.buffer[..i]).to_string();
            self.buffer.drain(..=i);
            let Some(rest) = line.strip_prefix("data:") else { continue };
            let data = rest.trim();
            if data == "[DONE]" {
                self.done = true;
                continue;
            }
            if data.is_empty() {
                continue;
            }
            // kein JSON (Kommentar oder Statuszeile): übergehen
            if let Ok(v) = serde_json::from_str::<Value>(data) {
                events.push(v);
            }
        }
        events
    }
}

/// Wie `TextDecoder` mit `stream: true`: setzt Bytes zu Text zusammen, auch wenn ein Zeichen über zwei Teile geht.
#[derive(Default)]
struct Utf8Stream {
    rest: Vec<u8>,
}

impl Utf8Stream {
    fn push(&mut self, bytes: &[u8]) -> String {
        self.rest.extend_from_slice(bytes);
        let mut out = String::new();
        let mut start = 0;
        loop {
            match std::str::from_utf8(&self.rest[start..]) {
                Ok(s) => {
                    out.push_str(s);
                    start = self.rest.len();
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    out.push_str(&String::from_utf8_lossy(&self.rest[start..start + valid]));
                    start += valid;
                    match e.error_len() {
                        Some(n) => {
                            out.push('\u{FFFD}');
                            start += n;
                        }
                        None => break, // Zeichen unvollständig: auf den nächsten Teil warten
                    }
                }
            }
        }
        self.rest.drain(..start);
        out
    }
}

// ---------- Denk-Marken ----------

/// Art eines Textstücks
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Reasoning,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Reasoning => "reasoning",
        }
    }
}

/// Trennt <think>…</think> aus dem Text, auch wenn die Marken über mehrere Teile verteilt ankommen.
/// `push` und `flush` liefern die fertigen Stücke zurück (in JS rufen sie `emit(kind, text)` auf).
#[derive(Default)]
pub struct ThinkSplitter {
    pub in_think: bool,
    pending: String,
}

impl ThinkSplitter {
    pub fn new() -> Self {
        Self::default()
    }

    fn kind(&self) -> Kind {
        if self.in_think {
            Kind::Reasoning
        } else {
            Kind::Text
        }
    }

    pub fn push(&mut self, text: &str) -> Vec<(Kind, String)> {
        let mut out = Vec::new();
        let mut rest = std::mem::take(&mut self.pending) + text;
        while !rest.is_empty() {
            let tag = if self.in_think { "</think>" } else { "<think>" };
            if let Some(i) = rest.find(tag) {
                if i > 0 {
                    out.push((self.kind(), rest[..i].to_string()));
                }
                self.in_think = !self.in_think;
                rest = rest[i + tag.len()..].to_string();
                continue;
            }
            // Ein angefangenes Tag am Ende zurückhalten
            let bytes = rest.as_bytes();
            let mut keep = 0;
            for n in (1..=(tag.len() - 1).min(bytes.len())).rev() {
                if tag.as_bytes().starts_with(&bytes[bytes.len() - n..]) {
                    keep = n;
                    break;
                }
            }
            let cut = rest.len() - keep;
            if cut > 0 {
                out.push((self.kind(), rest[..cut].to_string()));
            }
            self.pending = rest[cut..].to_string();
            rest.clear();
        }
        out
    }

    pub fn flush(&mut self) -> Vec<(Kind, String)> {
        let mut out = Vec::new();
        if !self.pending.is_empty() {
            out.push((self.kind(), std::mem::take(&mut self.pending)));
        }
        self.pending.clear();
        out
    }
}

// ---------- Antwort sammeln ----------

/// Ein Werkzeugaufruf des Modells
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

/// Eine Quelle der Websuche von ASK
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub url: String,
    pub title: String,
}

/// Ausgabetoken und Zeiten einer Antwort (so gespeichert im Chat unter `stats`)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub tokens: i64,
    pub estimated: bool,
    pub prompt_tokens: i64,
    pub gen_ms: i64,
    pub first_ms: i64,
    pub total_ms: i64,
}

/// Sammelt eine (gestreamte oder ganze) Antwort im OpenAI-Format: Text, Denken, Werkzeugaufrufe,
/// dazu Tokenzahlen (falls ASK sie liefert) und Zeiten für die Anzeige der Geschwindigkeit.
pub struct Completion {
    pub content: String,
    pub reasoning: String,
    pub calls: Vec<ToolCall>,
    pub finish: String,
    pub error: String,
    pub error_code: i64,
    pub usage: Option<Value>,
    pub sources: Vec<Source>,
    /// Strom ohne [DONE] und ohne Ende-Kennung abgebrochen (Text bis dahin ist da)
    pub incomplete: bool,
    now: NowFn,
    pub started: i64,
    first_at: i64,
    last_at: i64,
    on_delta: DeltaFn,
    splitter: ThinkSplitter,
    remap: HashMap<i64, usize>,
}

impl std::fmt::Debug for Completion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Completion")
            .field("content", &self.content)
            .field("reasoning", &self.reasoning)
            .field("calls", &self.calls)
            .field("finish", &self.finish)
            .field("error", &self.error)
            .field("incomplete", &self.incomplete)
            .finish_non_exhaustive()
    }
}

impl Default for Completion {
    fn default() -> Self {
        Self::new()
    }
}

impl Completion {
    pub fn new() -> Self {
        Self::with(None, None)
    }

    /// `onDelta` und Uhr (für Tests) wie die Parameter des Konstruktors in JS
    pub fn with(on_delta: Option<DeltaFn>, now: Option<NowFn>) -> Self {
        let now: NowFn = now.unwrap_or_else(|| Arc::new(now_ms));
        let started = now();
        Completion {
            content: String::new(),
            reasoning: String::new(),
            calls: Vec::new(),
            finish: String::new(),
            error: String::new(),
            error_code: 0,
            usage: None,
            sources: Vec::new(),
            incomplete: false,
            now,
            started,
            first_at: 0,
            last_at: 0,
            on_delta: on_delta.unwrap_or_else(|| Arc::new(|_, _| {})),
            splitter: ThinkSplitter::new(),
            remap: HashMap::new(),
        }
    }

    fn deliver(&mut self, parts: Vec<(Kind, String)>) {
        for (kind, text) in parts {
            match kind {
                Kind::Text => self.content.push_str(&text),
                Kind::Reasoning => self.reasoning.push_str(&text),
            }
            (self.on_delta)(kind.as_str(), &text);
        }
    }

    pub fn add(&mut self, chunk: &Value) {
        let Some(obj) = chunk.as_object() else { return };
        if let Some(e) = obj.get("error").filter(|e| truthy(e)) {
            self.error = match e {
                Value::String(s) => s.clone(),
                _ => {
                    let m = e.get("message").filter(|v| truthy(v)).or_else(|| e.get("detail").filter(|v| truthy(v)));
                    m.map(js_string).unwrap_or_else(|| e.to_string())
                }
            };
            let candidates = [e.get("code"), e.get("status"), e.get("status_code"), chunk.get("status_code")];
            let first = candidates.iter().flatten().find(|v| !v.is_null());
            let n = first.map(|v| js_number(v)).unwrap_or(f64::NAN);
            self.error_code = if n.is_nan() { 0 } else { n as i64 };
            return;
        }
        if let Some(Value::String(detail)) = obj.get("detail") {
            self.error = detail.clone();
            return;
        }
        if let Some(usage) = obj.get("usage").filter(|u| truthy(u)) {
            if usage.get("completion_tokens").is_some_and(truthy) || usage.get("prompt_tokens").is_some_and(truthy) {
                self.usage = Some(usage.clone());
            }
        }
        if let Some(sources) = obj.get("sources").filter(|s| truthy(s)) {
            let known: HashSet<String> = self.sources.iter().map(|s| s.url.clone()).collect();
            let fresh: Vec<Source> = parse_sources(sources).into_iter().filter(|s| !known.contains(&s.url)).collect();
            self.sources.extend(fresh);
        }
        let Some(choice) = obj.get("choices").and_then(|c| c.get(0)).filter(|c| !c.is_null()) else { return };
        let delta = choice.get("delta").filter(|v| truthy(v));
        let message = choice.get("message").filter(|v| truthy(v));
        let empty = json!({});
        let d = delta.or(message).unwrap_or(&empty);
        let has = |k: &str| d.get(k).is_some_and(truthy);
        if has("content") || has("reasoning_content") || has("reasoning") || has("tool_calls") {
            self.last_at = (self.now)();
            if self.first_at == 0 {
                self.first_at = self.last_at;
            }
        }
        let reasoning = d.get("reasoning_content").filter(|v| !v.is_null()).or_else(|| d.get("reasoning").filter(|v| !v.is_null()));
        if let Some(r) = reasoning.filter(|r| truthy(r)) {
            let text = js_string(r);
            self.reasoning.push_str(&text);
            (self.on_delta)("reasoning", &text);
        }
        if let Some(c) = d.get("content").filter(|c| truthy(c)) {
            let parts = self.splitter.push(&js_string(c));
            self.deliver(parts);
        }
        let whole = delta.is_none() && message.is_some(); // ganze Antwort: Aufrufe der Reihe nach
        if let Some(list) = d.get("tool_calls").and_then(Value::as_array) {
            for (n, tc) in list.iter().enumerate() {
                let slot = self.slot_for(tc, if whole { Some(n) } else { None });
                if slot >= self.calls.len() {
                    self.calls.resize_with(slot + 1, ToolCall::default);
                }
                let call = &mut self.calls[slot];
                if let Some(id) = tc.get("id").filter(|v| truthy(v)) {
                    call.id = js_string(id);
                }
                let function = tc.get("function");
                if let Some(name) = function.and_then(|f| f.get("name")).filter(|v| truthy(v)).map(js_string) {
                    if call.name != name {
                        call.name.push_str(&name);
                    }
                }
                if let Some(args) = function.and_then(|f| f.get("arguments")).filter(|v| truthy(v)) {
                    match args {
                        Value::String(s) => call.arguments.push_str(s),
                        other => call.arguments.push_str(&other.to_string()),
                    }
                }
            }
        }
        if let Some(finish) = choice.get("finish_reason").filter(|v| truthy(v)) {
            self.finish = js_string(finish);
        }
    }

    /// Platz eines gestreamten Werkzeugaufrufs. Ohne «index» (manche Server) oder mit demselben index für
    /// verschiedene Aufrufe entscheidet die Kennung: eine neue Kennung ist ein neuer Aufruf, Teile ohne
    /// Kennung gehören zum letzten.
    fn slot_for(&mut self, tc: &Value, fixed: Option<usize>) -> usize {
        if let Some(f) = fixed {
            return f;
        }
        let id = tc.get("id").filter(|v| truthy(v)).map(js_string).unwrap_or_default();
        if !id.is_empty() {
            if let Some(i) = self.calls.iter().position(|c| c.id == id) {
                return i;
            }
        }
        let index = tc.get("index").and_then(|v| match v {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64)),
            _ => None,
        });
        if let Some(index) = index.filter(|i| *i >= 0) {
            let slot = self.remap.get(&index).copied().unwrap_or(index as usize);
            // gleicher index, aber eine andere Kennung: ein weiterer Aufruf; folgende Teile mit diesem index gehören zu ihm
            if let Some(at) = self.calls.get(slot) {
                if !id.is_empty() && !at.id.is_empty() && at.id != id {
                    self.remap.insert(index, self.calls.len());
                    return self.calls.len();
                }
            }
            return slot;
        }
        if self.calls.is_empty() || (!id.is_empty() && !self.calls.last().is_some_and(|c| c.id.is_empty())) {
            return self.calls.len();
        }
        self.calls.len() - 1
    }

    pub fn end(&mut self) -> &mut Self {
        let rest = self.splitter.flush();
        self.deliver(rest);
        self.calls.retain(|c| !c.name.is_empty());
        self
    }

    /// Ausgabetoken und Zeiten dieser Antwort. genMs: vom ersten bis zum letzten gestreamten Teil.
    pub fn stats(&self) -> Stats {
        let end = if self.last_at != 0 { self.last_at } else { (self.now)() };
        let utf16 = |s: &str| s.encode_utf16().count();
        let chars = utf16(&self.content) + utf16(&self.reasoning) + self.calls.iter().map(|c| utf16(&c.name) + utf16(&c.arguments)).sum::<usize>();
        let measured = self.usage.as_ref().and_then(|u| u.get("completion_tokens")).filter(|v| truthy(v)).map(|v| js_number(v).round() as i64);
        Stats {
            tokens: measured.unwrap_or_else(|| (chars as f64 / CHARS_PER_TOKEN).round() as i64),
            estimated: measured.is_none(),
            prompt_tokens: self.usage.as_ref().and_then(|u| u.get("prompt_tokens")).filter(|v| truthy(v)).map(|v| js_number(v) as i64).unwrap_or(0),
            gen_ms: if self.first_at != 0 { end - self.first_at } else { 0 },
            first_ms: if self.first_at != 0 { self.first_at - self.started } else { 0 },
            total_ms: end - self.started,
        }
    }
}

/// `Number(x)` aus JavaScript (NaN, wenn keine Zahl)
fn js_number(v: &Value) -> f64 {
    match v {
        Value::Number(n) => n.as_f64().unwrap_or(f64::NAN),
        Value::String(s) => {
            let t = s.trim();
            if t.is_empty() {
                0.0
            } else {
                t.parse::<f64>().unwrap_or(f64::NAN)
            }
        }
        Value::Bool(b) => f64::from(u8::from(*b)),
        Value::Null => 0.0,
        _ => f64::NAN,
    }
}

// ---------- Hilfen ----------

/// Werkzeugaufrufe im Text, wie Qwen sie schreibt, wenn der Server sie nicht selbst auswertet:
/// `<tool_call>{"name": "...", "arguments": {...}}</tool_call>`. Ergebnis: (Text ohne die Aufrufe, Aufrufe).
pub fn parse_text_tool_calls(content: &str) -> (String, Vec<ToolCall>) {
    let mut calls = Vec::new();
    let text = re!(r"(?s)<tool_call>\s*(.*?)\s*</tool_call>").replace_all(content, |caps: &regex::Captures| {
        let whole = caps.get(0).map(|m| m.as_str()).unwrap_or("");
        let body = re!(r"^```(?:json)?\s*|\s*```$").replace_all(&caps[1], "");
        if let Ok(json) = serde_json::from_str::<Value>(&body) {
            if let Some(name) = json.get("name").filter(|n| truthy(n)) {
                let args = json
                    .get("arguments")
                    .filter(|v| !v.is_null())
                    .or_else(|| json.get("parameters").filter(|v| !v.is_null()))
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                calls.push(ToolCall {
                    id: String::new(),
                    name: js_string(name),
                    arguments: match args {
                        Value::String(s) => s,
                        other => other.to_string(),
                    },
                });
                return String::new();
            }
        }
        // kein gültiger Aufruf, Text stehen lassen
        whole.to_string()
    });
    (text.trim().to_string(), calls)
}

/// Denkaufwand (Regler wie «Reasoning Effort» in ASK). reasoning_effort verstehen OpenAI-kompatible
/// Server und ASK selbst; für Hoch und Maximal zusätzlich vLLM (chat_template_kwargs) und Ollama (think).
/// Der Hauptprozess setzt dazu den Qwen-Schalter /think. «standard» schickt nichts, das Modell entscheidet selbst.
pub fn thinking_params(level: &str) -> Option<Value> {
    match level {
        "tief" => Some(json!({ "reasoning_effort": "low" })),
        "mittel" => Some(json!({ "reasoning_effort": "medium" })),
        "hoch" | "maximal" => Some(json!({ "reasoning_effort": "high", "think": true, "chat_template_kwargs": { "enable_thinking": true } })),
        _ => None,
    }
}

const EFFORTS: [&str; 6] = ["none", "minimal", "low", "medium", "high", "xhigh"];

/// Lehnt ein Modell eine Stufe ab und nennt die erlaubten («Unexpected reasoning effort high. Supported
/// types are xhigh (default), medium, and low»), die nächste erlaubte Stufe wählen, bei Gleichstand die höhere.
/// None: keine Liste in der Meldung oder die Stufe ist schon erlaubt.
pub fn nearest_effort(message: &str, wanted: &str) -> Option<String> {
    let list = re!(r"(?i)supported[^:.]*?(?:are|:)\s*([^.]+)").captures(message).and_then(|c| c.get(1)).map(|m| m.as_str()).unwrap_or("");
    let allowed: Vec<&str> = EFFORTS
        .iter()
        .copied()
        .filter(|e| Regex::new(&format!(r"(?i)\b{e}\b")).map(|r| r.is_match(list)).unwrap_or(false))
        .collect();
    if allowed.is_empty() || allowed.contains(&wanted) {
        return None;
    }
    let idx = |e: &str| EFFORTS.iter().position(|x| *x == e).map(|i| i as i32).unwrap_or(-1);
    let at = idx(wanted);
    let best = allowed.iter().copied().reduce(|best, e| {
        let d = (idx(e) - at).abs();
        let bd = (idx(best) - at).abs();
        if d < bd || (d == bd && idx(e) > idx(best)) {
            e
        } else {
            best
        }
    });
    best.map(String::from)
}

/// Websuche in ASK eingeschaltet und für dieses Konto erlaubt? (Admins dürfen immer.)
pub fn web_search_allowed(config: &Value, user: &Value) -> bool {
    if config.pointer("/features/enable_web_search") != Some(&Value::Bool(true)) {
        return false;
    }
    if user.get("role").and_then(Value::as_str) == Some("admin") {
        return true;
    }
    user.pointer("/permissions/features/web_search") != Some(&Value::Bool(false))
}

/// Quellen, die ASK bei der Websuche mitschickt («sources»), als Liste { url, title } ohne Doppelte.
pub fn parse_sources(list: &Value) -> Vec<Source> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let first_str = |m: &Value, keys: [&str; 2]| -> String {
        keys.iter().filter_map(|k| m.get(*k)).filter(|v| truthy(v)).find_map(|v| v.as_str().map(String::from)).unwrap_or_default()
    };
    for s in list.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        let metas: &[Value] = s.get("metadata").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let urls: &[Value] = s.pointer("/source/urls").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        let items: Vec<(String, String)> = if !metas.is_empty() {
            metas.iter().map(|m| (first_str(m, ["source", "url"]), first_str(m, ["title", "name"]))).collect()
        } else {
            urls.iter().map(|u| (u.as_str().unwrap_or("").to_string(), String::new())).collect()
        };
        for (url, title) in items {
            if url.is_empty() || !re!(r"(?i)^https?://").is_match(&url) || seen.contains(&url) {
                continue;
            }
            seen.insert(url.clone());
            let title = title.trim().to_string();
            let title = if title.is_empty() {
                re!(r"(?i)^https?://(www\.)?").replace(&url, "").split('/').next().unwrap_or("").to_string()
            } else {
                title
            };
            out.push(Source { url, title });
        }
    }
    out
}

/// Terminal-Einträge, die Versionen bis 29.10.2026 in den ASK-Einstellungen hinterlassen haben (E30).
/// Nur genau dieser Eintrag zählt, eigene Terminals der Person bleiben unberührt.
pub fn is_stale_terminal(s: &Value) -> bool {
    s.get("name").and_then(Value::as_str) == Some("ASK Desktop")
        && s.get("key").and_then(Value::as_str) == Some("ask-desktop")
        && re!(r"^http://127\.0\.0\.1:\d+/?$").is_match(&crate::util::str_of(s, "url"))
}

/// Kontextfenster eines Modells aus den Angaben von /api/models. Je nach Modellserver steht es an anderer Stelle
/// (Ollama: num_ctx, vLLM: max_model_len, andere: context_length oder context_window).
pub fn context_of(m: &Value) -> i64 {
    let paths = [
        "/info/params/num_ctx",
        "/context_length",
        "/context_window",
        "/max_model_len",
        "/info/meta/context_length",
        "/openai/context_length",
        "/openai/context_window",
        "/openai/max_model_len",
        "/top_provider/context_length",
    ];
    paths
        .iter()
        .map(|p| m.pointer(p).map(js_number).unwrap_or(f64::NAN))
        .find(|v| v.is_finite() && *v >= 1000.0)
        .map(|v| v.round() as i64)
        .unwrap_or(0)
}

/// Fehler, den ASK in einer Antwort mit Status 200 meldet (im Strom oder als JSON). Ohne Statuscode zählt
/// er wie eine Ablehnung (400), damit «Chat zu lang» und die anderen Rückfallmodi greifen; Überlastung
/// wie 503, damit die App nochmals versucht.
pub fn inband_error(mut completion: Completion) -> AskError {
    let message = completion.error.clone();
    let code = if (400..600).contains(&completion.error_code) { completion.error_code } else { 0 };
    let status = if code != 0 {
        code
    } else if re!(r"(?i)rate.?limit|too many requests").is_match(&message) {
        429
    } else if re!(r"(?i)overload|unavailable|temporar|try again|timeout|timed out").is_match(&message) {
        503
    } else {
        400
    };
    completion.end();
    AskError::new(format!("ASK: {message}")).with_status(status as u16).with_partial(completion)
}

async fn error_of(res: reqwest::Response) -> AskError {
    let status = res.status().as_u16();
    let text = res.text().await.unwrap_or_default();
    let mut detail = text.clone();
    if let Ok(v) = serde_json::from_str::<Value>(&text) {
        let d = v.get("detail").filter(|d| truthy(d)).or_else(|| v.pointer("/error/message").filter(|d| truthy(d)));
        detail = match d {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => text.clone(),
        };
    }
    let shown = if detail.is_empty() { format!("HTTP {status}") } else { detail };
    AskError::new(format!("ASK: {shown}")).with_status(status)
}

/// Gehört die Adresse zu ASK? Verglichen wird die ganze Herkunft, nicht nur der Anfang der Adresse
/// («https://ask.example.evil.org» beginnt auch mit «https://ask.example»).
pub fn same_origin(url: &str, base: &str) -> bool {
    if url.is_empty() || base.is_empty() {
        return false;
    }
    match (url::Url::parse(url), url::Url::parse(base)) {
        (Ok(a), Ok(b)) => a.origin().ascii_serialization() == b.origin().ascii_serialization(),
        _ => false,
    }
}

// ---------- Schnittstellen nach aussen ----------

/// Die ASK-Anmeldeseite in einer Webansicht der Hülle (in Electron die `WebContentsView`).
/// Die Ereignisse der Seite meldet die Hülle über `AskSession::on_view_*`.
#[async_trait]
pub trait AskView: Send + Sync {
    /// Skript in der Seite ausführen und sein Ergebnis liefern (`executeJavaScript`). Fehler, wenn die Seite
    /// nicht lesbar ist (z. B. Fehlerseite).
    async fn eval(&self, js: &str) -> Result<Value>;
    /// Aktuelle Adresse der Seite (`getURL`)
    fn url(&self) -> String;
    /// Seite lädt gerade (`isLoading`)
    fn is_loading(&self) -> bool;
    /// Webansicht gibt es nicht mehr (`isDestroyed`)
    fn is_destroyed(&self) -> bool;
    /// Adresse laden, ohne zu warten; scheitert das Laden, meldet die Hülle `on_view_failed` (`loadURL`)
    fn load_url(&self, url: &str);
    /// Cookies dieser Herkunft löschen (`clearStorageData({ origin, storages: ['cookies'] })`)
    async fn clear_cookies(&self, origin: &str);
}

/// Eine Anfrage an das Modell (die Felder von `chat({ ... })` in JS)
#[derive(Clone, Default)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<Value>,
    pub tools: Option<Vec<Value>>,
    pub web_search: bool,
    /// «standard», «tief», «mittel», «hoch», «maximal»
    pub thinking: String,
    pub signal: Option<Signal>,
    pub on_delta: Option<DeltaFn>,
}

/// Was Prüfer, Gedächtnis und Hilfsagenten von ASK brauchen (in Tests austauschbar)
#[async_trait]
pub trait ChatApi: Send + Sync {
    async fn chat(&self, req: ChatRequest) -> Result<Completion>;
}

/// Optionen von `request` und `raw` (die von `fetch`, die der Code benutzt)
#[derive(Clone, Default)]
pub struct RequestOptions {
    /// Standard GET
    pub method: Option<reqwest::Method>,
    pub body: Option<Vec<u8>>,
    /// ersetzt gleichnamige Standardköpfe (Authorization, Content-Type)
    pub headers: Vec<(String, String)>,
    pub signal: Option<Signal>,
}

impl RequestOptions {
    pub fn post(body: impl Into<Vec<u8>>) -> Self {
        RequestOptions { method: Some(reqwest::Method::POST), body: Some(body.into()), ..Default::default() }
    }
    pub fn with_method(mut self, method: reqwest::Method) -> Self {
        self.method = Some(method);
        self
    }
    pub fn with_signal(mut self, signal: Signal) -> Self {
        self.signal = Some(signal);
        self
    }
}

/// Modell in ASK
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub id: String,
    pub name: String,
    /// true/false, wenn ASK angibt, ob das Modell Bilder versteht; sonst nicht vorhanden
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vision: Option<bool>,
    /// Grösse des Kontextfensters in Token, falls ASK oder der Modellserver sie nennt; sonst 0
    pub context: i64,
}

/// Angemeldetes Konto
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub name: String,
    pub role: Value,
}

/// Treffer der Websuche
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchResult {
    pub url: String,
    pub title: String,
    pub snippet: String,
}

#[derive(Clone, Default)]
struct Fallbacks {
    no_usage: bool,
    no_think_params: bool,
    effort_for: HashMap<String, String>,
}

struct State {
    base: String,
    token: String,
    status: Value,
    models: Vec<Model>,
    user: Option<UserInfo>,
    web_search: bool,
    busy: bool,
    again: bool,
    last_check: i64,
    load_failed: bool,
    failed_at: i64,
    cleaned_for: String,
    view: Option<Arc<dyn AskView>>,
    timer: Option<tokio::task::JoinHandle<()>>,
    handle: Option<tokio::runtime::Handle>,
}

struct Inner {
    client: reqwest::Client,
    events: Emitter,
    st: Mutex<State>,
    fallbacks: Mutex<HashMap<String, Fallbacks>>,
}

/// Die Verbindung mit ASK. Klonen teilt den Zustand.
#[derive(Clone)]
pub struct AskSession {
    inner: Arc<Inner>,
}

impl Default for AskSession {
    fn default() -> Self {
        Self::new()
    }
}

fn recheck_ms(state: &str) -> i64 {
    match state {
        "nicht-angemeldet" => 3000,
        "unbekannt" => 1000,
        "pruefen" => 500,
        "fehler" => 30000,
        "angemeldet" => 120000,
        _ => 30000,
    }
}

fn state_of(status: &Value) -> String {
    status.get("state").and_then(Value::as_str).unwrap_or("").to_string()
}

/// Status mit einem zusätzlichen oder ersetzten Feld (`{ ...status, key: value }`)
fn with_field(status: &Value, key: &str, value: Value) -> Value {
    let mut next = status.clone();
    if let Some(o) = next.as_object_mut() {
        o.insert(key.to_string(), value);
    }
    next
}

impl AskSession {
    pub fn new() -> Self {
        Self::with_client(reqwest::Client::new())
    }

    /// Mit eigenem HTTP-Client (Proxy von Windows, Tests)
    pub fn with_client(client: reqwest::Client) -> Self {
        AskSession {
            inner: Arc::new(Inner {
                client,
                events: Emitter::new(),
                st: Mutex::new(State {
                    base: String::new(),
                    token: String::new(),
                    status: json!({ "state": "unbekannt" }),
                    models: Vec::new(),
                    user: None,
                    web_search: false,
                    busy: false,
                    again: false,
                    last_check: 0,
                    load_failed: false,
                    failed_at: 0,
                    cleaned_for: String::new(),
                    view: None,
                    timer: None,
                    handle: tokio::runtime::Handle::try_current().ok(),
                }),
                fallbacks: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.inner.client
    }

    /// Ereignisse: «status» (Status als JSON) und «models» (Liste der Modelle)
    pub fn events(&self) -> &Emitter {
        &self.inner.events
    }

    pub fn on(&self, event: &str, f: impl Fn(&Value) + Send + Sync + 'static) {
        self.inner.events.on(event, f);
    }

    // ----- Zustand lesen -----

    pub fn status(&self) -> Value {
        self.inner.st.lock().status.clone()
    }
    pub fn models(&self) -> Vec<Model> {
        self.inner.st.lock().models.clone()
    }
    pub fn user(&self) -> Option<UserInfo> {
        self.inner.st.lock().user.clone()
    }
    /// Websuche in ASK für dieses Konto erlaubt (`ask.webSearch`)
    pub fn web_search(&self) -> bool {
        self.inner.st.lock().web_search
    }
    pub fn token(&self) -> String {
        self.inner.st.lock().token.clone()
    }
    pub fn base(&self) -> String {
        self.inner.st.lock().base.clone()
    }
    pub fn set_base(&self, url: &str) {
        self.inner.st.lock().base = url.trim_end_matches('/').to_string();
    }
    /// Token setzen (Tests, Wiederherstellung); sonst liest `check` es aus der Seite
    pub fn set_token(&self, token: &str) {
        self.inner.st.lock().token = token.to_string();
    }

    fn publish(&self, status: Value) {
        self.inner.st.lock().status = status.clone();
        self.inner.events.emit("status", &status);
    }

    fn spawn<F: std::future::Future<Output = ()> + Send + 'static>(&self, f: F) {
        let handle = self.inner.st.lock().handle.clone();
        match handle {
            Some(h) => {
                h.spawn(f);
            }
            None => {
                tokio::spawn(f);
            }
        }
    }

    // ----- Webansicht -----

    /// Webansicht der ASK-Seite anhängen und den Takt starten (alle 2 Sekunden prüfen).
    /// Aufruf innerhalb der tokio-Laufzeit.
    pub fn attach(&self, view: Arc<dyn AskView>) {
        self.set_view(view);
        let weak = Arc::downgrade(&self.inner);
        let task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            interval.tick().await; // setInterval feuert erst nach der ersten Wartezeit
            loop {
                interval.tick().await;
                let Some(inner) = weak.upgrade() else { break };
                AskSession { inner }.tick().await;
            }
        });
        if let Some(old) = self.inner.st.lock().timer.replace(task) {
            old.abort();
        }
    }

    /// Wie `attach`, aber ohne Takt (Tests; die Hülle ruft `tick` selbst auf)
    pub fn set_view(&self, view: Arc<dyn AskView>) {
        let mut s = self.inner.st.lock();
        s.view = Some(view);
        // Konnte die ASK-Seite nicht laden (offline, Proxy, Zertifikat), meldet die Adresse trotzdem ASK,
        // aber in der Seite steckt die Fehlerseite: Token lesen geht dort nicht. Dann nicht prüfen, sonst
        // wird daraus «ASK: Fehler», und die Oberfläche blendet den Anmeldebereich mit «ASK ist nicht erreichbar» aus.
        // Der Ladefehler steht im Status (loadError), nicht in einem Ereignis: Scheitert das Laden gleich beim Start,
        // ist die Oberfläche noch nicht bereit und verpasst ein Ereignis; den Status holt sie beim Start ab (ENTSCHEIDE E53).
        s.load_failed = false;
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            s.handle = Some(h);
        }
    }

    /// Hülle: die Seite konnte nicht geladen werden (`did-fail-load` im Hauptframe). Code -3 (abgebrochen) ist kein Fehler.
    pub fn on_view_failed(&self, code: i64, description: &str, url: &str) {
        if code == -3 {
            return;
        }
        let status = {
            let mut s = self.inner.st.lock();
            s.load_failed = true;
            s.failed_at = now_ms();
            s.status.clone()
        };
        if state_of(&status) != "angemeldet" {
            self.publish(with_field(&status, "loadError", json!({ "code": code, "description": description, "url": url })));
        }
    }

    /// Hülle: eine Navigation beginnt (`did-start-navigation`). Nur Hauptframe und echte Seitenwechsel zählen.
    pub fn on_view_navigation_started(&self, is_main_frame: bool, is_same_document: bool) {
        if is_main_frame && !is_same_document {
            self.inner.st.lock().load_failed = false;
        }
    }

    /// Hülle: Seite fertig geladen (`did-finish-load`). Nach einem Ladefehler kommt auch für die Fehlerseite
    /// «fertig geladen»: dann bleibt der Fehler.
    pub fn on_view_loaded(&self) {
        let status = {
            let s = self.inner.st.lock();
            if s.load_failed {
                return;
            }
            s.status.clone()
        };
        if status.get("loadError").is_some() {
            let mut rest = status.clone();
            if let Some(o) = rest.as_object_mut() {
                o.remove("loadError");
            }
            self.publish(rest);
        }
        let me = self.clone();
        self.spawn(async move {
            me.check(true).await;
        });
    }

    /// Hülle: die Seite hat navigiert (`did-navigate`, `did-navigate-in-page`).
    /// Verlässt die ASK-Seite die Anmeldung (auch nach dem Microsoft-Login), ist man vermutlich angemeldet:
    /// sofort «prüfen» melden, damit die Oberfläche die Seite ausblendet und einen Ladebildschirm zeigt.
    pub fn on_view_navigated(&self, url: &str) {
        let Ok(target) = url::Url::parse(url) else { return };
        let (base, state) = {
            let s = self.inner.st.lock();
            (s.base.clone(), state_of(&s.status))
        };
        if !same_origin(target.as_str(), &base) {
            return; // fremde Anmeldeseite bleibt sichtbar
        }
        if state == "nicht-angemeldet" && !re!(r"^/auth\b").is_match(target.path()) {
            self.publish(json!({ "state": "pruefen" }));
        }
        let me = self.clone();
        self.spawn(async move {
            me.check(true).await;
        });
    }

    /// Alle 2 Sekunden: prüfen; konnte die Seite nicht laden, alle 15 Sekunden von selbst nochmals laden
    /// (z. B. WLAN oder VPN beim Start der App noch nicht bereit)
    pub async fn tick(&self) {
        let retry = {
            let mut s = self.inner.st.lock();
            match &s.view {
                Some(v) if s.load_failed && !v.is_destroyed() && !s.base.is_empty() && now_ms() - s.failed_at > RETRY_LOAD_MS => {
                    let v = v.clone();
                    let base = s.base.clone();
                    s.failed_at = now_ms();
                    Some((v, base))
                }
                _ => None,
            }
        };
        if let Some((view, base)) = retry {
            view.load_url(&base); // scheitert es wieder, meldet die Hülle den Ladefehler
            return;
        }
        self.check(false).await;
    }

    /// Liest das Token aus der ASK-Seite und prüft es mit einer Anfrage an ASK.
    /// (Ausdrücklich als `BoxFuture`: `check` startet sich selbst nochmals, das verträgt der Typ-Prüfer sonst nicht.)
    pub fn check(&self, force: bool) -> futures::future::BoxFuture<'static, Value> {
        let me = self.clone();
        Box::pin(async move { me.check_impl(force).await })
    }

    async fn check_impl(&self, force: bool) -> Value {
        let (view, base, state, load_failed) = {
            let mut s = self.inner.st.lock();
            if s.busy && force {
                s.again = true; // nach der laufenden Prüfung gleich nochmals
            }
            if s.view.is_none() || s.base.is_empty() || s.busy {
                return s.status.clone();
            }
            (s.view.clone().expect("view"), s.base.clone(), state_of(&s.status), s.load_failed)
        };
        if view.is_destroyed() || view.is_loading() || load_failed {
            return self.status();
        }
        let url = view.url();
        if !same_origin(&url, &base) {
            // Fertig geladen, aber nicht auf ASK (z. B. gleich zur Microsoft-Anmeldung weitergeleitet, oder eine Seite des
            // Netzes): die Seite zeigen, statt hinter «Verbinde mit ASK …» zu warten. Angemeldet ist man dort nicht.
            if ["unbekannt", "pruefen"].contains(&state.as_str()) && re!(r"(?i)^https?:").is_match(&url) {
                self.publish(json!({ "state": "nicht-angemeldet" }));
            }
            return self.status();
        }
        {
            let mut s = self.inner.st.lock();
            if s.busy {
                return s.status.clone();
            }
            if !force && now_ms() - s.last_check < recheck_ms(&state) {
                return s.status.clone();
            }
            s.last_check = now_ms();
            s.busy = true;
        }
        self.check_body(&view, force).await;
        let again = {
            let mut s = self.inner.st.lock();
            s.busy = false;
            std::mem::take(&mut s.again)
        };
        if again {
            let me = self.clone();
            self.spawn(async move {
                me.check(true).await;
            });
        }
        self.status()
    }

    async fn check_body(&self, view: &Arc<dyn AskView>, force: bool) {
        let token = match view.eval("localStorage.token || \"\"").await {
            Ok(v) if truthy(&v) => js_string(&v),
            Ok(_) => String::new(),
            Err(err) => {
                // Seite (noch) nicht lesbar, z. B. Fehlerseite: kein Fehler von ASK, später nochmals. Den Grund im Status
                // mitgeben, damit die Oberfläche ihn zeigen kann, wenn das Warten zu lange dauert.
                let detail = format!("Die ASK-Seite lässt sich nicht lesen: {err}");
                let status = self.status();
                if state_of(&status) != "angemeldet" && status.get("detail").and_then(Value::as_str) != Some(detail.as_str()) {
                    self.publish(with_field(&status, "detail", json!(detail)));
                }
                return;
            }
        };
        if token.is_empty() {
            self.inner.st.lock().token.clear();
            if state_of(&self.status()) != "nicht-angemeldet" {
                self.publish(json!({ "state": "nicht-angemeldet" }));
            }
            return;
        }
        {
            let mut s = self.inner.st.lock();
            if token == s.token && state_of(&s.status) == "angemeldet" && !force {
                return;
            }
            s.token = token.clone();
        }
        if let Err(err) = self.login(&token).await {
            if is_auth(&err) {
                self.publish(json!({ "state": "nicht-angemeldet" }));
            } else {
                self.publish(json!({ "state": "fehler", "message": err.to_string() }));
            }
        }
    }

    async fn login(&self, token: &str) -> Result<()> {
        let user = self.json("/api/v1/auths/").await?;
        let models = self.load_models().await?;
        let name = ["name", "email"].iter().filter_map(|k| user.get(*k)).find(|v| truthy(v)).map(js_string).unwrap_or_default();
        let config = self.json("/api/config").await.unwrap_or_else(|_| json!({}));
        {
            let mut s = self.inner.st.lock();
            s.user = Some(UserInfo { name: name.clone(), role: user.get("role").cloned().unwrap_or(Value::Null) });
            s.web_search = web_search_allowed(&config, &user);
        }
        self.publish(json!({ "state": "angemeldet", "user": name }));
        if models.is_empty() {
            self.publish(json!({ "state": "fehler", "message": "ASK bietet diesem Konto kein Modell an. Wer ASK betreut, muss deinem Konto zuerst ein Modell freigeben." }));
        }
        let me = self.clone();
        let token = token.to_string();
        self.spawn(async move { me.clean_stale_terminal(&token).await });
        Ok(())
    }

    // ----- Anfragen an ASK -----

    async fn send(&self, base: &str, token: &str, path: &str, opts: RequestOptions) -> Result<reqwest::Response> {
        use reqwest::header::{HeaderMap, HeaderName, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {token}")).map_err(|e| AskError::new(format!("Ungültiges Token: {e}")))?);
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        for (k, v) in &opts.headers {
            if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(k.as_bytes()), HeaderValue::from_str(v)) {
                headers.insert(name, value);
            }
        }
        let mut req = self.inner.client.request(opts.method.clone().unwrap_or(reqwest::Method::GET), format!("{base}{path}")).headers(headers);
        if let Some(body) = opts.body.clone() {
            req = req.body(body);
        }
        let sent = req.send();
        let res = match &opts.signal {
            Some(sig) => tokio::select! {
                _ = sig.cancelled() => return Err(AskError::abort().into()),
                r = sent => r,
            },
            None => sent.await,
        };
        res.map_err(|e| net_error(&e).into())
    }

    /// Anfrage an ASK mit der Abmelde-Logik: Nur 401 heisst «nicht mehr angemeldet». 403 kommt auch, wenn das
    /// Konto ein Modell oder eine Funktion nicht benutzen darf; das meldet der Aufrufer als Fehler, die Anmeldung bleibt.
    pub async fn request(&self, path: &str, opts: RequestOptions) -> Result<reqwest::Response> {
        let (token, base) = {
            let s = self.inner.st.lock();
            (s.token.clone(), s.base.clone())
        };
        if token.is_empty() {
            return Err(AuthError::new("Nicht bei ASK angemeldet.").into());
        }
        let res = self.send(&base, &token, path, opts).await?;
        if res.status().as_u16() == 401 {
            self.inner.st.lock().token.clear();
            self.publish(json!({ "state": "nicht-angemeldet" }));
            return Err(AuthError::new("Die Anmeldung bei ASK ist abgelaufen. Bitte neu anmelden.").into());
        }
        Ok(res)
    }

    /// Anfrage an ASK ohne die Abmelde-Logik von `request`, für Nebenwege wie Flux (bild), bei denen ein
    /// 401 oder 403 auch «nicht freigegeben» heissen kann.
    pub async fn raw(&self, path: &str, opts: RequestOptions) -> Result<reqwest::Response> {
        let (token, base) = {
            let s = self.inner.st.lock();
            (s.token.clone(), s.base.clone())
        };
        if token.is_empty() {
            return Err(AuthError::new("Nicht bei ASK angemeldet.").into());
        }
        self.send(&base, &token, path, opts).await
    }

    /// Entfernt den alten Terminal-Eintrag aus den ASK-Einstellungen, einmal pro Anmeldung. Ohne die
    /// Abmelde-Logik von `request`: Schlägt das fehl, bleibt die Anmeldung gültig, und der nächste Start versucht es wieder.
    pub async fn clean_stale_terminal(&self, token: &str) {
        let base = {
            let mut s = self.inner.st.lock();
            if s.cleaned_for == token {
                return;
            }
            s.cleaned_for = token.to_string();
            s.base.clone()
        };
        let _ = self.try_clean_stale_terminal(&base, token).await; // nicht wichtig genug für eine Meldung
    }

    async fn try_clean_stale_terminal(&self, base: &str, token: &str) -> Result<()> {
        let auth = format!("Bearer {token}");
        let res = self.inner.client.get(format!("{base}/api/v1/users/user/settings")).header("Authorization", &auth).header("Content-Type", "application/json").send().await?;
        if !res.status().is_success() {
            return Ok(());
        }
        let data: Value = res.json().await?;
        let Some(servers) = data.pointer("/ui/terminalServers").and_then(Value::as_array) else { return Ok(()) };
        if !servers.iter().any(is_stale_terminal) {
            return Ok(());
        }
        let kept: Vec<&Value> = servers.iter().filter(|s| !is_stale_terminal(s)).collect();
        self.inner
            .client
            .post(format!("{base}/api/v1/users/user/settings/update"))
            .header("Authorization", &auth)
            .header("Content-Type", "application/json")
            .body(json!({ "ui": { "terminalServers": kept } }).to_string())
            .send()
            .await?;
        Ok(())
    }

    /// Websuche von ASK (Open WebUI /api/v1/retrieval/process/web/search) für das Werkzeug web_search.
    /// Ohne die Abmelde-Logik von `request`: ein 403 heisst hier «Websuche nicht freigegeben».
    /// Ältere Open-WebUI-Versionen erwarten «query», neuere «queries»; beides geht mit.
    pub async fn search_web(&self, query: &str, signal: Option<Signal>) -> Result<Vec<SearchResult>> {
        let opts = RequestOptions { method: Some(reqwest::Method::POST), body: Some(json!({ "query": query, "queries": [query] }).to_string().into_bytes()), headers: Vec::new(), signal: signal.clone() };
        let res = self.raw("/api/v1/retrieval/process/web/search", opts).await?;
        if !res.status().is_success() {
            anyhow::bail!("Websuche von ASK: HTTP {}", res.status().as_u16());
        }
        let data: Value = match &signal {
            Some(sig) => tokio::select! {
                _ = sig.cancelled() => return Err(AskError::abort().into()),
                r = res.json() => r?,
            },
            None => res.json().await?,
        };
        let text = |i: &Value, keys: [&str; 2]| -> String {
            keys.iter().filter_map(|k| i.get(*k)).find(|v| truthy(v)).map(|v| js_string(v).trim().to_string()).unwrap_or_default()
        };
        let items = data.get("items").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
        Ok(items
            .iter()
            .map(|i| SearchResult { url: text(i, ["link", "url"]), title: text(i, ["title", "title"]), snippet: text(i, ["snippet", "content"]) })
            .filter(|i| re!(r"(?i)^https?://").is_match(&i.url))
            .collect())
    }

    /// GET mit der Abmelde-Logik, Antwort als JSON
    pub async fn json(&self, path: &str) -> Result<Value> {
        let res = self.request(path, RequestOptions::default()).await?;
        if !res.status().is_success() {
            anyhow::bail!("ASK antwortet mit HTTP {} ({})", res.status().as_u16(), path);
        }
        Ok(res.json().await?)
    }

    pub async fn load_models(&self) -> Result<Vec<Model>> {
        let data = self.json("/api/models").await?;
        let list = match data.get("data").filter(|d| truthy(d)) {
            Some(d) => d.clone(),
            None => data.clone(),
        };
        let models: Vec<Model> = list
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter(|m| m.get("id").is_some_and(truthy) && !m.pointer("/info/meta/hidden").is_some_and(truthy))
            .map(|m| {
                let id = crate::util::str_of(m, "id");
                let name = m.get("name").filter(|n| truthy(n)).map(js_string).unwrap_or_else(|| id.clone());
                Model {
                    id,
                    name,
                    vision: m.pointer("/info/meta/capabilities/vision").filter(|v| !v.is_null()).map(truthy),
                    context: context_of(m),
                }
            })
            .collect();
        self.inner.st.lock().models = models.clone();
        self.inner.events.emit("models", &serde_json::to_value(&models).unwrap_or(Value::Null));
        Ok(models)
    }

    /// Abmelden wie auf der Webseite: bei ASK abmelden, das Token aus der Seite und die Cookies von ASK löschen.
    /// Ohne das meldet die Anmeldeseite nach einem Login über Microsoft gleich wieder an (Token im Cookie).
    pub async fn logout(&self) {
        let (token, base, view) = {
            let mut s = self.inner.st.lock();
            (std::mem::take(&mut s.token), s.base.clone(), s.view.clone())
        };
        if !token.is_empty() && view.is_some() {
            let _ = self.inner.client.get(format!("{base}/api/v1/auths/signout")).header("Authorization", format!("Bearer {token}")).send().await;
        }
        if let Some(view) = view {
            let _ = view.eval("localStorage.removeItem(\"token\")").await;
            if let Ok(u) = url::Url::parse(&base) {
                view.clear_cookies(&u.origin().ascii_serialization()).await;
            }
            view.load_url(&format!("{base}/auth"));
        }
        self.publish(json!({ "state": "nicht-angemeldet" }));
    }

    // ----- Antworten des Modells -----

    /// Was ein Modell abgelehnt hat (Tokenzahlen im Strom, Denkparameter, Stufen des Denkaufwands), merkt sich
    /// die App pro Modell: Ein Modell, das «reasoning_effort» nicht kennt, schaltet es für die anderen nicht ab.
    fn fallbacks(&self, model: &str) -> Fallbacks {
        self.inner.fallbacks.lock().entry(model.to_string()).or_default().clone()
    }

    fn update_fallbacks(&self, model: &str, f: impl FnOnce(&mut Fallbacks)) {
        f(self.inner.fallbacks.lock().entry(model.to_string()).or_default());
    }

    /// Für Tests: für alle Modelle zurücksetzen oder setzen
    pub fn set_no_usage(&self, v: bool) {
        for f in self.inner.fallbacks.lock().values_mut() {
            f.no_usage = v;
        }
    }

    /// Für Tests: für alle Modelle zurücksetzen oder setzen
    pub fn set_no_think_params(&self, v: bool) {
        for f in self.inner.fallbacks.lock().values_mut() {
            f.no_think_params = v;
        }
    }

    async fn send_chat(&self, req: &ChatRequest, with_usage: bool, signal: &Signal) -> Result<reqwest::Response> {
        let fb = self.fallbacks(&req.model);
        let mut body = Map::new();
        body.insert("model".into(), json!(req.model));
        body.insert("messages".into(), Value::Array(req.messages.clone()));
        body.insert("stream".into(), json!(true));
        // Websuche macht ASK auf dem Server vor der Antwort und schickt die Quellen mit.
        if req.web_search {
            body.insert("features".into(), json!({ "web_search": true }));
        }
        if !fb.no_think_params {
            if let Some(Value::Object(params)) = thinking_params(&req.thinking) {
                for (k, v) in params {
                    body.insert(k, v);
                }
                let wanted = body.get("reasoning_effort").and_then(Value::as_str).unwrap_or("").to_string();
                if let Some(effort) = fb.effort_for.get(&wanted) {
                    body.insert("reasoning_effort".into(), json!(effort));
                }
            }
        }
        // Tokenzahlen am Ende des Stroms; manche Server lehnen das ab, dann ohne.
        if with_usage {
            body.insert("stream_options".into(), json!({ "include_usage": true }));
        }
        if let Some(tools) = req.tools.as_ref().filter(|t| !t.is_empty()) {
            body.insert("tools".into(), Value::Array(tools.clone()));
        }
        let opts = RequestOptions::post(Value::Object(body).to_string()).with_signal(signal.clone());
        self.request("/api/chat/completions", opts).await
    }

    /// Eine Antwort des Modells, gestreamt. `on_delta(kind, text)` für die Anzeige.
    /// Bricht die Antwort ab (Stopp, Verbindung weg), trägt der Fehler unter `partial` mit, was bis dahin ankam.
    pub async fn chat(&self, req: ChatRequest) -> Result<Completion> {
        let mut completion = Completion::with(req.on_delta.clone(), None);
        // Schweigt ASK mitten in der Antwort zu lange, hängt die Verbindung: abbrechen statt ewig warten
        let stalled = AtomicBool::new(false);
        let signal: Signal = req.signal.as_ref().map(|o| o.child_token()).unwrap_or_default();
        let model = req.model.clone();
        let mut res = self.send_chat(&req, !self.fallbacks(&model).no_usage, &signal).await?;
        let mut retry = 0;
        // Lehnt der Server einen Zusatz ab, ohne ihn nochmals versuchen und ihn künftig weglassen.
        let res = loop {
            if res.status().is_success() {
                break res;
            }
            let status = res.status().as_u16();
            let error = error_of(res).await;
            if status != 400 || retry >= 3 {
                return Err(error.into());
            }
            let fb = self.fallbacks(&model);
            let think = if fb.no_think_params { None } else { thinking_params(&req.thinking) };
            let effort = think.as_ref().and_then(|p| p.get("reasoning_effort")).and_then(Value::as_str).map(String::from);
            let sent = effort.as_ref().map(|e| fb.effort_for.get(e).cloned().unwrap_or_else(|| e.clone()));
            let other = match &sent {
                Some(s) if re!(r"(?i)reasoning[ _]effort").is_match(&error.message) => nearest_effort(&error.message, s),
                _ => None,
            };
            if !fb.no_usage && re!(r"(?i)stream_options|include_usage").is_match(&error.message) {
                self.update_fallbacks(&model, |f| f.no_usage = true);
            } else if let (Some(other), Some(effort)) = (other, effort) {
                // erlaubte Stufe statt gar keiner
                self.update_fallbacks(&model, |f| {
                    f.effort_for.insert(effort, other);
                });
            } else if think.is_some() && re!(r"(?i)reasoning[ _]effort|chat_template_kwargs|enable_thinking|\bthink\b").is_match(&error.message) {
                self.update_fallbacks(&model, |f| f.no_think_params = true);
            } else {
                return Err(error.into());
            }
            res = self.send_chat(&req, !self.fallbacks(&model).no_usage, &signal).await?;
            retry += 1;
        };
        let is_stream = res.headers().get(reqwest::header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|t| t.contains("text/event-stream"));
        if !is_stream {
            let body = tokio::select! {
                _ = signal.cancelled() => return Err(AskError::abort().into()),
                r = res.json::<Value>() => r.ok(),
            };
            if let Some(v) = body {
                completion.add(&v);
            }
            if !completion.error.is_empty() {
                return Err(inband_error(completion).into());
            }
            completion.end();
            return Ok(completion);
        }
        let mut parser = SseParser::new();
        let mut decoder = Utf8Stream::default();
        let mut stream = res.bytes_stream();
        let failure: Option<AskError> = loop {
            let next = tokio::select! {
                _ = signal.cancelled() => Err(()),
                r = tokio::time::timeout(Duration::from_millis(STALL_MS), stream.next()) => match r {
                    Ok(x) => Ok(x),
                    Err(_) => {
                        stalled.store(true, Ordering::SeqCst);
                        signal.cancel();
                        Err(())
                    }
                },
            };
            match next {
                Err(()) => {
                    break Some(if stalled.load(Ordering::SeqCst) && !req.signal.as_ref().is_some_and(|o| o.is_cancelled()) {
                        AskError::new("ASK hat fünf Minuten lang nichts mehr gesendet. Die Verbindung wurde getrennt; schreib «weiter».")
                    } else {
                        AskError::abort()
                    });
                }
                Ok(None) => break None,
                Ok(Some(Err(e))) => break Some(AskError::new(format!("terminated: {}", error_chain(&e)))),
                Ok(Some(Ok(bytes))) => {
                    let text = decoder.push(&bytes);
                    for event in parser.push(&text) {
                        completion.add(&event);
                    }
                }
            }
        };
        if let Some(err) = failure {
            completion.end();
            return Err(err.with_partial(completion).into());
        }
        for event in parser.push("\n") {
            completion.add(&event);
        }
        if !completion.error.is_empty() {
            return Err(inband_error(completion).into());
        }
        // Strom zu Ende ohne [DONE] und ohne finish_reason: die Verbindung brach ab. Ein halber Werkzeugaufruf
        // (Argumente kein gültiges JSON) darf nicht ausgeführt werden: als Verbindungsfehler neu versuchen.
        if !parser.done && completion.finish.is_empty() {
            completion.end();
            let broken = completion.calls.iter().any(|c| serde_json::from_str::<Value>(if c.arguments.is_empty() { "{}" } else { &c.arguments }).is_err());
            if broken {
                return Err(AskError::new("ASK: Die Verbindung brach mitten in einem Werkzeugaufruf ab (network).").with_status(502).with_partial(completion).into());
            }
            completion.incomplete = true;
            return Ok(completion);
        }
        completion.end();
        Ok(completion)
    }
}

#[async_trait]
impl ChatApi for AskSession {
    async fn chat(&self, req: ChatRequest) -> Result<Completion> {
        AskSession::chat(self, req).await
    }
}

// ---------- Tests ----------

/// Kleiner HTTP-Server für Tests (statt `fetch`-Attrappen): handgeschriebene HTTP/1.1-Antworten, auch SSE.
#[cfg(test)]
pub(crate) mod test_http {
    use parking_lot::Mutex;
    use serde_json::Value;
    use std::collections::HashMap;
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Clone, Debug)]
    pub struct Req {
        pub method: String,
        pub path: String,
        pub headers: HashMap<String, String>,
        pub body: String,
    }

    impl Req {
        pub fn json(&self) -> Value {
            serde_json::from_str(&self.body).unwrap_or(Value::Null)
        }
    }

    pub enum Resp {
        Full { status: u16, content_type: String, body: Vec<u8>, headers: Vec<(String, String)> },
        /// ohne Länge, Ende durch Schliessen (Server-Sent Events)
        Stream { chunks: Vec<Vec<u8>> },
        /// Verbindung ohne Antwort schliessen
        Drop,
    }

    impl Resp {
        pub fn json(status: u16, body: &Value) -> Resp {
            Resp::Full { status, content_type: "application/json".into(), body: body.to_string().into_bytes(), headers: vec![] }
        }
        pub fn text(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Resp {
            Resp::Full { status, content_type: content_type.into(), body: body.into(), headers: vec![] }
        }
        pub fn sse(text: &str) -> Resp {
            Resp::Stream { chunks: vec![text.as_bytes().to_vec()] }
        }
        pub fn redirect(location: &str) -> Resp {
            Resp::Full { status: 302, content_type: "text/plain".into(), body: vec![], headers: vec![("Location".into(), location.into())] }
        }
    }

    pub struct TestServer {
        pub url: String,
        pub reqs: Arc<Mutex<Vec<Req>>>,
        task: tokio::task::JoinHandle<()>,
    }

    impl Drop for TestServer {
        fn drop(&mut self) {
            self.task.abort();
        }
    }

    impl TestServer {
        pub fn paths(&self) -> Vec<String> {
            self.reqs.lock().iter().map(|r| r.path.clone()).collect()
        }
    }

    /// HTTP-Client ohne Proxy für Tests
    pub fn client() -> reqwest::Client {
        reqwest::Client::builder().no_proxy().build().expect("client")
    }

    pub async fn serve(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let url = format!("http://{}", listener.local_addr().expect("addr"));
        let reqs: Arc<Mutex<Vec<Req>>> = Arc::new(Mutex::new(Vec::new()));
        let handler = Arc::new(handler);
        let log = reqs.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { break };
                let handler = handler.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut tmp = [0u8; 4096];
                    let head_end = loop {
                        if let Some(p) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                            break p + 4;
                        }
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => return,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    };
                    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
                    let mut lines = head.split("\r\n");
                    let mut first = lines.next().unwrap_or("").split(' ');
                    let method = first.next().unwrap_or("").to_string();
                    let path = first.next().unwrap_or("").to_string();
                    let headers: HashMap<String, String> =
                        lines.filter_map(|l| l.split_once(':')).map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string())).collect();
                    let len: usize = headers.get("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
                    while buf.len() < head_end + len {
                        match sock.read(&mut tmp).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend_from_slice(&tmp[..n]),
                        }
                    }
                    let body = String::from_utf8_lossy(&buf[head_end..(head_end + len).min(buf.len())]).to_string();
                    let req = Req { method, path, headers, body };
                    log.lock().push(req.clone());
                    match handler(&req) {
                        Resp::Drop => {}
                        Resp::Full { status, content_type, body, headers } => {
                            let mut out = format!("HTTP/1.1 {status} X\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n", body.len());
                            for (k, v) in headers {
                                out.push_str(&format!("{k}: {v}\r\n"));
                            }
                            out.push_str("\r\n");
                            let _ = sock.write_all(out.as_bytes()).await;
                            let _ = sock.write_all(&body).await;
                        }
                        Resp::Stream { chunks } => {
                            let _ = sock.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").await;
                            for c in chunks {
                                let _ = sock.write_all(&c).await;
                                let _ = sock.flush().await;
                            }
                        }
                    }
                    let _ = sock.shutdown().await;
                });
            }
        });
        TestServer { url, reqs, task }
    }
}

#[cfg(test)]
mod tests {
    use super::test_http::{client, serve, Resp};
    use super::*;

    fn session() -> AskSession {
        AskSession::with_client(client())
    }

    #[test]
    fn sse_ereignisse_ueber_teilstuecke_done_und_kommentare_ignoriert() {
        let mut p = SseParser::new();
        let mut events = Vec::new();
        events.extend(p.push("data: {\"a\":"));
        events.extend(p.push("1}\n\n: kommentar\ndata: [DONE]\r\n"));
        events.extend(p.push("data: {\"b\":2}\n"));
        assert_eq!(events, vec![json!({ "a": 1 }), json!({ "b": 2 })]);
        assert!(p.done);
    }

    #[test]
    fn think_wird_getrennt_auch_wenn_die_marke_geteilt_ankommt() {
        let mut s = ThinkSplitter::new();
        let (mut text, mut reasoning) = (String::new(), String::new());
        let mut take = |parts: Vec<(Kind, String)>| {
            for (k, t) in parts {
                match k {
                    Kind::Text => text.push_str(&t),
                    Kind::Reasoning => reasoning.push_str(&t),
                }
            }
        };
        for piece in ["<thi", "nk>ich ", "denke</th", "ink>Antwort <", "b>"] {
            take(s.push(piece));
        }
        take(s.flush());
        assert_eq!(text, "Antwort <b>");
        assert_eq!(reasoning, "ich denke");
    }

    #[test]
    fn werkzeugaufrufe_aus_gestreamten_teilen_zusammensetzen() {
        let mut c = Completion::new();
        c.add(&json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "id": "a", "function": { "name": "run_command", "arguments": "" } }] } }] }));
        c.add(&json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "{\"command\":" } }] } }] }));
        c.add(&json!({ "choices": [{ "delta": { "tool_calls": [{ "index": 0, "function": { "arguments": "\"dir\"}" } }, { "index": 1, "id": "b", "function": { "name": "read_file", "arguments": "{\"path\":\"x\"}" } }] } }] }));
        c.add(&json!({ "choices": [{ "delta": { "reasoning_content": "hm" }, "finish_reason": "tool_calls" }] }));
        c.end();
        assert_eq!(
            c.calls,
            vec![
                ToolCall { id: "a".into(), name: "run_command".into(), arguments: "{\"command\":\"dir\"}".into() },
                ToolCall { id: "b".into(), name: "read_file".into(), arguments: "{\"path\":\"x\"}".into() },
            ]
        );
        assert_eq!(c.reasoning, "hm");
        assert_eq!(c.finish, "tool_calls");
    }

    #[test]
    fn ganze_antwort_ohne_streaming_und_fehlermeldung() {
        let mut c = Completion::new();
        c.add(&json!({ "choices": [{ "message": { "content": "Hallo", "tool_calls": [{ "id": "x", "function": { "name": "list_files", "arguments": {} } }] } }] }));
        c.end();
        assert_eq!(c.content, "Hallo");
        assert_eq!(c.calls, vec![ToolCall { id: "x".into(), name: "list_files".into(), arguments: "{}".into() }]);
        let mut e = Completion::new();
        e.add(&json!({ "error": { "message": "Modell nicht gefunden" } }));
        assert_eq!(e.error, "Modell nicht gefunden");
    }

    #[test]
    fn tokenzahlen_und_zeiten_gemessen_oder_geschaetzt() {
        let t = Arc::new(std::sync::atomic::AtomicI64::new(1000));
        let clock = t.clone();
        let now: NowFn = Arc::new(move || clock.load(Ordering::SeqCst));
        let mut c = Completion::with(None, Some(now.clone()));
        t.store(1800, Ordering::SeqCst);
        c.add(&json!({ "choices": [{ "delta": { "reasoning_content": "a".repeat(36) } }] }));
        t.store(3800, Ordering::SeqCst);
        c.add(&json!({ "choices": [{ "delta": { "content": "b".repeat(36) } }] }));
        t.store(3900, Ordering::SeqCst);
        c.add(&json!({ "choices": [], "usage": { "prompt_tokens": 500, "completion_tokens": 40 } }));
        c.end();
        assert_eq!(c.stats(), Stats { tokens: 40, estimated: false, prompt_tokens: 500, gen_ms: 2000, first_ms: 800, total_ms: 2800 });
        let mut e = Completion::with(None, Some(now));
        e.add(&json!({ "choices": [{ "delta": { "content": "x".repeat(72) } }] }));
        e.end();
        assert_eq!(e.stats().tokens, 20);
        assert!(e.stats().estimated);
    }

    #[test]
    fn websuche_freigabe_und_quellen_aus_ask() {
        let on = json!({ "features": { "enable_web_search": true } });
        assert!(web_search_allowed(&on, &json!({ "role": "user", "permissions": { "features": { "web_search": true } } })));
        assert!(!web_search_allowed(&on, &json!({ "role": "user", "permissions": { "features": { "web_search": false } } })));
        assert!(!web_search_allowed(&json!({ "features": { "enable_web_search": false } }), &json!({ "role": "admin" })));
        assert!(web_search_allowed(&on, &json!({ "role": "admin", "permissions": { "features": { "web_search": false } } })));
        let sources = parse_sources(&json!([
            { "source": { "type": "web_search" }, "metadata": [{ "source": "https://a.ch/x", "title": "A" }, { "source": "https://a.ch/x", "title": "A doppelt" }, { "source": "datei.pdf" }] },
            { "source": { "urls": ["https://www.b.ch/seite"] } },
        ]));
        assert_eq!(
            sources,
            vec![
                Source { url: "https://a.ch/x".into(), title: "A".into() },
                Source { url: "https://www.b.ch/seite".into(), title: "b.ch".into() },
            ]
        );
        let mut c = Completion::new();
        c.add(&json!({ "sources": [{ "metadata": [{ "source": "https://a.ch/x", "title": "A" }] }] }));
        c.add(&json!({ "sources": [{ "metadata": [{ "source": "https://a.ch/x", "title": "A" }] }] }));
        assert_eq!(c.sources.len(), 1);
    }

    #[test]
    fn alter_terminal_eintrag_nur_genau_dieser_gilt_als_alt() {
        let old = json!({ "name": "ASK Desktop", "key": "ask-desktop", "url": "http://127.0.0.1:30853" });
        assert!(is_stale_terminal(&old));
        assert!(!is_stale_terminal(&json!({ "name": "ASK Desktop", "key": "echter-key", "url": "http://127.0.0.1:30853" })));
        assert!(!is_stale_terminal(&json!({ "name": "Mein Terminal", "key": "ask-desktop", "url": "http://127.0.0.1:30853" })));
        assert!(!is_stale_terminal(&json!({ "name": "ASK Desktop", "key": "ask-desktop", "url": "https://terminal.example.ch" })));
        assert!(!is_stale_terminal(&Value::Null));
    }

    /// AskSession mit nachgebauter ASK-Schnittstelle; zeichnet die Schreibzugriffe auf.
    async fn cleaner_with(settings: impl Fn() -> Resp + Send + Sync + 'static) -> (AskSession, test_http::TestServer) {
        let server = serve(move |req| if req.path.ends_with("/settings/update") { Resp::json(200, &json!({})) } else { settings() }).await;
        let ask = session();
        ask.set_base(&server.url);
        (ask, server)
    }

    fn writes(server: &test_http::TestServer) -> Vec<Value> {
        server.reqs.lock().iter().filter(|r| r.path.ends_with("/settings/update")).map(|r| r.json()).collect()
    }

    #[tokio::test]
    async fn alter_terminal_eintrag_wird_einmal_entfernt_anderes_bleibt() {
        let mine = json!({ "name": "Mein Terminal", "key": "k", "url": "http://127.0.0.1:8000", "enabled": false });
        let servers = json!([mine, { "name": "ASK Desktop", "key": "ask-desktop", "url": "http://127.0.0.1:30853", "enabled": true }]);
        let (ask, server) = cleaner_with(move || Resp::json(200, &json!({ "ui": { "terminalServers": servers } }))).await;
        ask.clean_stale_terminal("token").await;
        ask.clean_stale_terminal("token").await;
        assert_eq!(writes(&server), vec![json!({ "ui": { "terminalServers": [mine] } })]);
    }

    #[tokio::test]
    async fn alter_terminal_eintrag_ohne_eintrag_oder_bei_fehlern_kein_schreiben_anmeldung_bleibt() {
        let (ask, server) = cleaner_with(|| Resp::json(200, &json!({ "ui": { "terminalServers": [{ "name": "Mein Terminal", "url": "http://127.0.0.1:8000" }] } }))).await;
        ask.clean_stale_terminal("token").await;
        assert!(writes(&server).is_empty());
        let (ask, server) = cleaner_with(|| Resp::json(200, &json!({}))).await;
        ask.clean_stale_terminal("token").await;
        assert!(writes(&server).is_empty());
        let (ask, server) = cleaner_with(|| Resp::json(403, &json!({}))).await;
        ask.set_token("token");
        ask.clean_stale_terminal("token").await;
        assert!(writes(&server).is_empty());
        assert_eq!(ask.token(), "token");
        let (ask, server) = cleaner_with(|| Resp::Drop).await;
        ask.clean_stale_terminal("token").await;
        assert!(writes(&server).is_empty());
    }

    #[test]
    fn werkzeugaufrufe_im_text_qwen_format() {
        let (text, calls) = parse_text_tool_calls("Los geht es.\n<tool_call>\n{\"name\": \"run_command\", \"arguments\": {\"command\": \"dir\"}}\n</tool_call>\n<tool_call>kaputt</tool_call>");
        assert_eq!(calls, vec![ToolCall { id: String::new(), name: "run_command".into(), arguments: "{\"command\":\"dir\"}".into() }]);
        assert_eq!(text, "Los geht es.\n\n<tool_call>kaputt</tool_call>");
    }

    #[test]
    fn werkzeugaufrufe_im_text_mit_zaun_und_parameters() {
        let (text, calls) = parse_text_tool_calls("<tool_call>```json\n{\"name\": \"x\", \"parameters\": {\"a\": 1}}\n```</tool_call>");
        assert_eq!(text, "");
        assert_eq!(calls, vec![ToolCall { id: String::new(), name: "x".into(), arguments: "{\"a\":1}".into() }]);
    }

    #[tokio::test]
    async fn denkaufwand_abgelehnte_stufe_durch_die_naechste_erlaubte_ersetzen() {
        let msg = "litellm.BadRequestError: OpenAIException - Unexpected reasoning effort high. Supported types are xhigh (default), medium, and low.";
        assert_eq!(nearest_effort(msg, "high").as_deref(), Some("xhigh"), "bei Gleichstand die höhere");
        assert_eq!(nearest_effort(msg, "low"), None, "erlaubt: nichts ändern");
        assert_eq!(nearest_effort("Unrecognized request argument: reasoning_effort", "high"), None, "ohne Liste: keine Wahl");

        let detail = msg.to_string();
        let server = serve(move |req| {
            if req.json()["reasoning_effort"] == "high" {
                Resp::json(400, &json!({ "detail": detail }))
            } else {
                Resp::json(200, &json!({ "choices": [{ "message": { "content": "ok" }, "finish_reason": "stop" }] }))
            }
        })
        .await;
        let ask = session();
        ask.set_base(&server.url);
        ask.set_token("t");
        let req = ChatRequest { model: "m".into(), thinking: "hoch".into(), ..Default::default() };
        let first = ask.chat(req.clone()).await.unwrap();
        assert_eq!(first.content, "ok");
        ask.chat(req).await.unwrap();
        {
            let reqs = server.reqs.lock();
            assert_eq!(reqs[0].method, "POST");
            assert_eq!(reqs[0].path, "/api/chat/completions");
            assert_eq!(reqs[0].headers.get("authorization").map(String::as_str), Some("Bearer t"));
        }
        let sent: Vec<Value> = server.reqs.lock().iter().map(|r| r.json()["reasoning_effort"].clone()).collect();
        assert_eq!(sent, vec![json!("high"), json!("xhigh"), json!("xhigh")], "einmal abgelehnt, danach gleich die erlaubte Stufe");
    }

    #[test]
    fn werkzeugaufrufe_ohne_index_oder_mit_demselben_index_fuer_verschiedene_aufrufe_bleiben_getrennt() {
        let mut c = Completion::new();
        let mut chunk = |tc: Value| c.add(&json!({ "choices": [{ "delta": { "tool_calls": [tc] } }] }));
        chunk(json!({ "id": "a", "function": { "name": "read_file", "arguments": "{\"path\":" } }));
        chunk(json!({ "function": { "arguments": "\"a.txt\"}" } }));
        chunk(json!({ "id": "b", "function": { "name": "read_file", "arguments": "{\"path\":\"b.txt\"}" } }));
        chunk(json!({ "index": 0, "id": "c", "function": { "name": "list_files", "arguments": "" } }));
        chunk(json!({ "index": 0, "function": { "arguments": "{}" } }));
        let calls: Vec<(String, String, String)> = c.end().calls.iter().map(|x| (x.id.clone(), x.name.clone(), x.arguments.clone())).collect();
        let expected = [("a", "read_file", "{\"path\":\"a.txt\"}"), ("b", "read_file", "{\"path\":\"b.txt\"}"), ("c", "list_files", "{}")];
        assert_eq!(calls, expected.iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c.to_string())).collect::<Vec<_>>());
        // ganze Antwort (ohne Strom, ohne index und Kennung): der Reihe nach
        let mut w = Completion::new();
        w.add(&json!({ "choices": [{ "message": { "tool_calls": [{ "function": { "name": "x", "arguments": "{}" } }, { "function": { "name": "y", "arguments": "{}" } }] } }] }));
        assert_eq!(w.end().calls.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), vec!["x", "y"]);
    }

    #[test]
    fn fehler_in_einer_antwort_mit_status_200_bekommen_einen_status() {
        let make = |chunk: Value| {
            let mut c = Completion::new();
            c.add(&chunk);
            inband_error(c)
        };
        assert_eq!(make(json!({ "error": { "message": "This model's maximum context length is 8192 tokens" } })).status, 400);
        assert_eq!(make(json!({ "error": { "message": "Server overloaded, try again" } })).status, 503);
        assert_eq!(make(json!({ "error": { "message": "x", "code": 429 } })).status, 429);
        assert_eq!(make(json!({ "error": "Rate limit exceeded" })).status, 429);
    }

    #[tokio::test]
    async fn abgelehnte_zusaetze_gelten_nur_fuer_das_modell_json_fehler_und_abgebrochene_straeme() {
        let next: Arc<Mutex<Option<Resp>>> = Arc::new(Mutex::new(None));
        let n = next.clone();
        let server = serve(move |req| {
            let body = req.json();
            if let Some(r) = n.lock().take() {
                return r;
            }
            if body["model"] == "a" && !body["reasoning_effort"].is_null() {
                return Resp::json(400, &json!({ "detail": "Unrecognized request argument: reasoning_effort" }));
            }
            Resp::json(200, &json!({ "choices": [{ "message": { "content": "ok" }, "finish_reason": "stop" }] }))
        })
        .await;
        let ask = session();
        ask.set_base(&server.url);
        ask.set_token("t");
        let req = |model: &str, thinking: &str| ChatRequest { model: model.into(), thinking: thinking.into(), ..Default::default() };
        ask.chat(req("a", "hoch")).await.unwrap();
        ask.chat(req("b", "hoch")).await.unwrap();
        let sent: Vec<(Value, Value)> = server.reqs.lock().iter().map(|r| (r.json()["model"].clone(), r.json()["reasoning_effort"].clone())).collect();
        assert_eq!(sent, vec![(json!("a"), json!("high")), (json!("a"), Value::Null), (json!("b"), json!("high"))]);

        *next.lock() = Some(Resp::json(200, &json!({ "error": { "message": "maximum context length is 8192 tokens" } })));
        let err = ask.chat(req("b", "")).await.unwrap_err();
        assert_eq!(status_of(&err), Some(400));
        assert!(err.to_string().contains("context length"));

        *next.lock() = Some(Resp::sse("data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"x\",\"function\":{\"name\":\"write_file\",\"arguments\":\"{\\\"pa\"}}]}}]}\n"));
        let err = ask.chat(req("b", "")).await.unwrap_err();
        assert_eq!(status_of(&err), Some(502));
        assert!(partial_of(&err).is_some());

        *next.lock() = Some(Resp::sse("data: {\"choices\":[{\"delta\":{\"content\":\"Hal\"}}]}\n"));
        let cut = ask.chat(req("b", "")).await.unwrap();
        assert_eq!(cut.content, "Hal");
        assert!(cut.incomplete);

        *next.lock() = Some(Resp::sse("data: {\"choices\":[{\"delta\":{\"content\":\"Hallo\"}}]}\ndata: [DONE]\n"));
        assert!(!ask.chat(req("b", "")).await.unwrap().incomplete);
    }

    #[tokio::test]
    async fn chat_strom_mit_denkmarken_und_umlauten_ueber_teilgrenzen() {
        let chunk = "data: {\"choices\":[{\"delta\":{\"content\":\"<think>Grüsse</think>Hä\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"llo\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n";
        let bytes = chunk.as_bytes().to_vec();
        // mitten im «ü» (2 Bytes) trennen
        let cut = chunk.find('ü').unwrap() + 1;
        let server = serve(move |_| Resp::Stream { chunks: vec![bytes[..cut].to_vec(), bytes[cut..].to_vec()] }).await;
        let ask = session();
        ask.set_base(&server.url);
        ask.set_token("t");
        let deltas = Arc::new(Mutex::new(Vec::new()));
        let d = deltas.clone();
        let on_delta: DeltaFn = Arc::new(move |kind, text| d.lock().push((kind.to_string(), text.to_string())));
        let c = ask.chat(ChatRequest { model: "m".into(), on_delta: Some(on_delta), ..Default::default() }).await.unwrap();
        assert_eq!(c.reasoning, "Grüsse");
        assert_eq!(c.content, "Hällo");
        assert_eq!(c.finish, "stop");
        assert!(!deltas.lock().is_empty());
    }

    #[tokio::test]
    async fn anmeldung_abgelaufen_bei_401_und_stopp_bricht_ab() {
        let server = serve(|req| if req.path.starts_with("/api/models") { Resp::json(401, &json!({})) } else { Resp::json(200, &json!({})) }).await;
        let ask = session();
        ask.set_base(&server.url);
        ask.set_token("t");
        let err = ask.json("/api/models").await.unwrap_err();
        assert!(is_auth(&err));
        assert_eq!(ask.token(), "");
        assert_eq!(ask.status()["state"], "nicht-angemeldet");
        let err = ask.json("/api/models").await.unwrap_err();
        assert!(is_auth(&err), "ohne Token gar nicht erst fragen");

        let stop = Signal::new();
        stop.cancel();
        ask.set_token("t");
        let err = ask.chat(ChatRequest { model: "m".into(), signal: Some(stop), ..Default::default() }).await.unwrap_err();
        assert!(is_abort(&err));
    }

    #[tokio::test]
    async fn modelle_und_konto_laden_dann_angemeldet() {
        let server = serve(|req| {
            let p = req.path.as_str();
            if p == "/api/v1/auths/" {
                Resp::json(200, &json!({ "name": "Anna", "role": "user", "permissions": { "features": { "web_search": true } } }))
            } else if p == "/api/models" {
                Resp::json(200, &json!({ "data": [
                    { "id": "a", "name": "Modell A", "info": { "meta": { "capabilities": { "vision": true } }, "params": { "num_ctx": 32768 } } },
                    { "id": "b", "info": { "meta": { "hidden": true } } },
                    { "id": "c" },
                ] }))
            } else if p == "/api/config" {
                Resp::json(200, &json!({ "features": { "enable_web_search": true } }))
            } else {
                Resp::json(404, &json!({}))
            }
        })
        .await;
        let ask = session();
        ask.set_base(&format!("{}/", server.url));
        let view = Arc::new(FakeView::new(&format!("{}/", server.url), || Ok(json!("tok"))));
        ask.set_view(view);
        let st = ask.check(true).await;
        assert_eq!(st, json!({ "state": "angemeldet", "user": "Anna" }));
        assert!(ask.web_search());
        let models = ask.models();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0], Model { id: "a".into(), name: "Modell A".into(), vision: Some(true), context: 32768 });
        assert_eq!(models[1], Model { id: "c".into(), name: "c".into(), vision: None, context: 0 });
    }

    /// Nachgebaute ASK-Seite für `check`
    struct FakeView {
        url: Mutex<String>,
        read: Box<dyn Fn() -> Result<Value> + Send + Sync>,
        loads: Mutex<Vec<String>>,
    }

    impl FakeView {
        fn new(url: &str, read: impl Fn() -> Result<Value> + Send + Sync + 'static) -> Self {
            FakeView { url: Mutex::new(url.to_string()), read: Box::new(read), loads: Mutex::new(Vec::new()) }
        }
    }

    #[async_trait]
    impl AskView for FakeView {
        async fn eval(&self, _js: &str) -> Result<Value> {
            (self.read)()
        }
        fn url(&self) -> String {
            self.url.lock().clone()
        }
        fn is_loading(&self) -> bool {
            false
        }
        fn is_destroyed(&self) -> bool {
            false
        }
        fn load_url(&self, url: &str) {
            self.loads.lock().push(url.to_string());
        }
        async fn clear_cookies(&self, _origin: &str) {}
    }

    fn fake_page(url: &str, read: impl Fn() -> Result<Value> + Send + Sync + 'static) -> (AskSession, Arc<FakeView>) {
        let view = Arc::new(FakeView::new(url, read));
        let ask = session();
        ask.set_base("https://ask.example.ch");
        ask.set_view(view.clone());
        (ask, view)
    }

    #[tokio::test]
    async fn ask_seite_laedt_nicht_kein_fehler_status_bleibt() {
        let (ask, _view) = fake_page("https://ask.example.ch/auth", || anyhow::bail!("Script failed to execute"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        ask.on("status", move |v| s.lock().push(state_of(v)));
        ask.on_view_failed(-105, "ERR_NAME_NOT_RESOLVED", "https://ask.example.ch/");
        assert_eq!(state_of(&ask.check(true).await), "unbekannt");
        // auch ohne Meldung des Ladefehlers: Token nicht lesbar ist kein Fehler von ASK
        ask.on_view_navigation_started(true, false);
        assert_eq!(state_of(&ask.check(true).await), "unbekannt");
        let seen = seen.lock();
        assert!(!seen.contains(&"fehler".to_string()) && !seen.contains(&"nicht-angemeldet".to_string()), "{seen:?}");
    }

    #[tokio::test]
    async fn ask_seite_geladen_ohne_token_nicht_angemeldet() {
        let (ask, _view) = fake_page("https://ask.example.ch/auth", || Ok(json!("")));
        assert_eq!(state_of(&ask.check(true).await), "nicht-angemeldet");
    }

    #[tokio::test]
    async fn ladefehler_steht_im_status_und_verschwindet_nach_dem_laden() {
        let (ask, _view) = fake_page("https://ask.example.ch/auth", || Ok(json!("")));
        ask.on_view_failed(-202, "ERR_CERT_AUTHORITY_INVALID", "https://ask.example.ch/");
        assert_eq!(ask.status(), json!({ "state": "unbekannt", "loadError": { "code": -202, "description": "ERR_CERT_AUTHORITY_INVALID", "url": "https://ask.example.ch/" } }));
        ask.on_view_loaded(); // auch die Fehlerseite meldet «fertig geladen»; der Fehler bleibt
        assert_eq!(ask.status()["loadError"]["code"], -202);
        ask.on_view_failed(-3, "ERR_ABORTED", "https://ask.example.ch/"); // Abbruch ist kein Fehler
        ask.on_view_navigation_started(true, false);
        ask.on_view_loaded();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(ask.status().get("loadError").is_none());
        assert_eq!(state_of(&ask.status()), "nicht-angemeldet");
    }

    #[tokio::test]
    async fn nach_einem_ladefehler_laedt_die_seite_nach_15_s_von_selbst_nochmals() {
        let (ask, view) = fake_page("https://ask.example.ch/auth", || Ok(json!("")));
        ask.on_view_failed(-106, "ERR_INTERNET_DISCONNECTED", "https://ask.example.ch/");
        ask.tick().await;
        assert!(view.loads.lock().is_empty(), "nicht sofort");
        ask.inner.st.lock().failed_at -= 16000;
        ask.tick().await;
        assert_eq!(*view.loads.lock(), vec!["https://ask.example.ch".to_string()]);
    }

    #[tokio::test]
    async fn seite_fertig_aber_nicht_auf_ask_zeigen_statt_warten() {
        let (ask, _view) = fake_page("https://login.microsoftonline.com/abc/oauth2/authorize", || anyhow::bail!("nie gelesen"));
        assert_eq!(state_of(&ask.check(true).await), "nicht-angemeldet");
    }

    #[tokio::test]
    async fn token_nicht_lesbar_status_bleibt_grund_steht_im_status() {
        let (ask, _view) = fake_page("https://ask.example.ch/auth", || anyhow::bail!("Script failed to execute"));
        let st = ask.check(true).await;
        assert_eq!(state_of(&st), "unbekannt");
        assert!(st["detail"].as_str().unwrap().contains("Script failed to execute"));
    }

    #[test]
    fn herkunft_und_kontextfenster() {
        assert!(same_origin("https://ask.example.ch/x", "https://ask.example.ch"));
        assert!(!same_origin("https://ask.example.ch.evil.org/", "https://ask.example.ch"));
        assert!(!same_origin("", "https://ask.example.ch"));
        assert_eq!(context_of(&json!({ "info": { "params": { "num_ctx": 8192 } } })), 8192);
        assert_eq!(context_of(&json!({ "max_model_len": "32768", "context_length": 5 })), 32768);
        assert_eq!(context_of(&json!({})), 0);
    }
}
