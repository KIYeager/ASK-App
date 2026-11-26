//! Hilfsagenten (ENTSCHEIDE E55): Das Modell gibt eine Teilfrage ab, ein eigener kleiner Agent klärt sie mit
//! lesenden Werkzeugen und gibt nur das Ergebnis zurück. So bleibt der Verlauf des Chats kurz, und mehrere
//! unabhängige Suchen laufen gleichzeitig. Hilfsagenten ändern nichts (keine Befehle, kein Schreiben), sehen den
//! Chat nicht und starten keine weiteren Hilfsagenten.
//!
//! Portiert aus src/main/helpers.js. Öffentliche Schnittstelle:
//! - `helper_tool() -> Value` (JS `HELPER_TOOL`, Werkzeugbeschreibung im OpenAI-Format), `HELPER`, `HELPER_TOOLS`,
//!   `PARALLEL`, `MAX_STEPS`
//! - `run_helper(raw_args, &HelperDeps) -> ToolResult`: einen Hilfsagenten ausführen
//! - `pool(tasks, limit)`: Aufgaben mit höchstens `limit` gleichzeitig, Ergebnisse in derselben Reihenfolge
//! - `helper_prompt(folder)`
//!
//! `HelperDeps` entspricht `deps` in JS. `apiMessages` und `textModePrompt` kommen aus agent.js und werden
//! hereingereicht (`ApiMessagesFn`, `TextModePromptFn`).

use crate::askapi::{parse_text_tool_calls, ChatApi, ChatRequest, Signal, ToolCall};
use crate::tools::{clean_args, clip, describe, run_tool, tool_specs, ProgressFn, TerminalEndpoint, ToolCtx, ToolResult};
use crate::util::{js_string, now_ms, take_chars, truthy};
use crate::web::Web;
use futures::StreamExt;
use serde_json::{json, Value};
use std::future::Future;
use std::sync::Arc;

pub const HELPER: &str = "hilfsagent";
pub const HELPER_TOOLS: [&str; 6] = ["read_file", "list_files", "find_files", "search_text", "web_search", "web_fetch"];
/// danach muss der Hilfsagent ohne Werkzeuge zusammenfassen
pub const MAX_STEPS: usize = 20;
/// so viele Hilfsagenten eines Chats arbeiten gleichzeitig
pub const PARALLEL: usize = 3;
/// so viel vom Ergebnis geht an das Modell des Chats
const RESULT_CHARS: usize = 8000;
/// so viel von einem Werkzeugergebnis sieht der Hilfsagent
const TOOL_CHARS: usize = 20000;

/// Werkzeug des Chats, mit dem das Modell einen Hilfsagenten startet
pub fn helper_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": HELPER,
            "description": format!(
                concat!(
                    "Startet einen Hilfsagenten, der eine Teilfrage selbständig klärt und nur sein Ergebnis zurückgibt. Er kann Dateien ",
                    "im Projekt suchen und lesen und im Web recherchieren, ändert aber nichts und sieht diesen Chat nicht: Beschreibe den Auftrag ",
                    "vollständig (Ziel, wo suchen, was zurückmelden). Gut für breite Suchen über viele Dateien und für mehrere unabhängige ",
                    "Recherchen: Mehrere Aufrufe im selben Schritt laufen gleichzeitig (höchstens {}). Eine einzelne bekannte Datei liest du selbst."
                ),
                PARALLEL
            ),
            "parameters": {
                "type": "object",
                "properties": {
                    "titel": { "type": "string", "description": "Kurzer Titel für die Anzeige, z. B. «Verwendung von config.json finden»" },
                    "auftrag": { "type": "string", "description": "Vollständiger Auftrag: was herausfinden, wo suchen, in welcher Form antworten" },
                },
                "required": ["titel", "auftrag"],
            },
        },
    })
}

pub fn helper_prompt(folder: &str) -> String {
    format!(
        "Du bist ein Hilfsagent von ASK Desktop. Ein anderer Agent hat dir eine Teilaufgabe gegeben; du arbeitest selbständig und \
fragst nicht zurück. Arbeitsordner: {}. Pfade sind relativ zu diesem Ordner.\n\
Du hast nur lesende Werkzeuge: Dateien auflisten, suchen und lesen, im Web suchen und Seiten lesen. Du änderst nichts.\n\
Geh zielgerichtet vor: zuerst suchen, dann nur das Nötige lesen. Hör auf, sobald du die Frage beantworten kannst.\n\
Deine letzte Antwort (ohne Werkzeugaufruf) ist dein Ergebnis für den anderen Agenten: knapp und konkret, mit Dateipfaden und \
Zeilennummern oder Adressen als Beleg. Sag ehrlich, was du nicht gefunden hast.",
        if folder.is_empty() { "(keiner)" } else { folder }
    )
}

const FINAL_PROMPT: &str = "\n\nDu hast die Höchstzahl an Schritten erreicht. Rufe kein Werkzeug mehr auf und fasse jetzt zusammen, \
was du herausgefunden hast und was offen bleibt.";

/// `apiMessages(system, messages, textMode)` aus agent.js: die Nachrichten für ASK
pub type ApiMessagesFn = Arc<dyn Fn(&str, &[Value], bool) -> Vec<Value> + Send + Sync>;
/// `textModePrompt(specs)` aus agent.js: Beschreibung der Werkzeuge als Text
pub type TextModePromptFn = Arc<dyn Fn(&[Value]) -> String + Send + Sync>;

/// Was ein Hilfsagent braucht (`deps` in JS)
#[derive(Clone)]
pub struct HelperDeps {
    pub ask: Arc<dyn ChatApi>,
    pub model: String,
    pub thinking: String,
    pub terminal: Option<Arc<dyn TerminalEndpoint>>,
    pub web: Option<Arc<Web>>,
    pub folder: String,
    pub chat: String,
    pub text_mode: bool,
    pub api_messages: ApiMessagesFn,
    pub text_mode_prompt: TextModePromptFn,
    pub signal: Option<Signal>,
    pub on_progress: Option<ProgressFn>,
}

/// Zahl mit Tausendertrenner wie `toLocaleString('de-CH')` (Apostroph)
fn swiss(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push('’');
        }
        out.push(c);
    }
    if n < 0 { format!("-{out}") } else { out }
}

fn random_id() -> String {
    format!("call_{}", hex::encode(rand::random::<[u8; 6]>()))
}

/// Einen Hilfsagenten ausführen. Liefert ein Ergebnis wie ein Werkzeug.
pub async fn run_helper(raw_args: &str, deps: &HelperDeps) -> ToolResult {
    let args: Value = serde_json::from_str(if raw_args.is_empty() { "{}" } else { raw_args }).ok().filter(truthy).unwrap_or_else(|| json!({}));
    let text_of = |keys: &[&str]| keys.iter().filter_map(|k| args.get(*k)).find(|v| truthy(v)).map(js_string).unwrap_or_default();
    let task = text_of(&["auftrag", "task"]).trim().to_string();
    let title = {
        let t = text_of(&["titel", "title"]);
        let t = take_chars(regex::Regex::new(r"\s+").expect("regex").replace_all(&t, " ").trim(), 120);
        if t.is_empty() { clip(&task, 80) } else { t }
    };
    if task.is_empty() {
        return ToolResult::new(false, "auftrag fehlt.", "ohne Auftrag");
    }
    if deps.model.is_empty() {
        return ToolResult::new(false, "Kein Modell für Hilfsagenten.", "kein Modell");
    }
    let cancelled = || deps.signal.as_ref().is_some_and(Signal::is_cancelled);
    let specs: Vec<Value> = tool_specs("").into_iter().filter(|s| HELPER_TOOLS.contains(&s["function"]["name"].as_str().unwrap_or(""))).collect();
    let mut messages: Vec<Value> = vec![json!({ "role": "user", "content": task, "time": now_ms() })];
    let mut steps: Vec<String> = Vec::new();
    let mut tokens: i64 = 0;
    let progress = |steps: &[String]| {
        if let Some(f) = &deps.on_progress {
            f(&format!("{title}\n{}", steps.join("\n")));
        }
    };
    progress(&steps);
    for n in 0..=MAX_STEPS {
        if cancelled() {
            return ToolResult::new(false, "Von der Person abgebrochen.", "abgebrochen");
        }
        let last = n == MAX_STEPS;
        let text_mode = deps.text_mode;
        let system = format!(
            "{}{}",
            helper_prompt(&deps.folder),
            if last { FINAL_PROMPT.to_string() } else if text_mode { (deps.text_mode_prompt)(&specs) } else { String::new() }
        );
        let request = ChatRequest {
            model: deps.model.clone(),
            messages: (deps.api_messages)(&system, &messages, text_mode),
            tools: if last || text_mode { None } else { Some(specs.clone()) },
            web_search: false,
            thinking: deps.thinking.clone(),
            signal: deps.signal.clone(),
            on_delta: Some(Arc::new(|_, _| {})),
        };
        let completion = match deps.ask.chat(request).await {
            Ok(c) => c,
            Err(err) => {
                if cancelled() {
                    return ToolResult::new(false, "Von der Person abgebrochen.", "abgebrochen");
                }
                return ToolResult { display: Some(steps.join("\n")), ..ToolResult::new(false, format!("Der Hilfsagent konnte ASK nicht erreichen: {err}"), "Fehler bei ASK") };
            }
        };
        tokens += completion.stats().tokens;
        let mut content = completion.content.clone();
        let mut calls: Vec<ToolCall> = completion.calls.clone();
        if calls.is_empty() && (text_mode || content.contains("<tool_call>")) {
            let (text, parsed) = parse_text_tool_calls(&content);
            if !parsed.is_empty() {
                calls = parsed;
                content = text;
            }
        }
        if last || calls.is_empty() {
            let text = if content.trim().is_empty() { "Der Hilfsagent hat kein Ergebnis geschrieben.".to_string() } else { content.trim().to_string() };
            let summary = format!(
                "{} {}{}",
                steps.len(),
                if steps.len() == 1 { "Schritt" } else { "Schritte" },
                if tokens > 0 { format!(" · {} Token", swiss(tokens)) } else { String::new() }
            );
            return ToolResult {
                ok: !content.trim().is_empty(),
                output: format!("Ergebnis des Hilfsagenten «{title}»:\n{}", clip(&text, RESULT_CHARS)),
                summary,
                display: Some(format!("{title}\n\n{}Ergebnis:\n{text}", if steps.is_empty() { String::new() } else { format!("Schritte:\n{}\n\n", steps.join("\n")) })),
                ..Default::default()
            };
        }
        let tool_calls: Vec<Value> = calls
            .iter()
            .map(|c| {
                json!({ "id": if c.id.is_empty() { random_id() } else { c.id.clone() }, "type": "function",
                    "function": { "name": c.name, "arguments": clean_args(&Value::String(c.arguments.clone())) } })
            })
            .collect();
        messages.push(json!({ "role": "assistant", "content": content, "tool_calls": tool_calls, "textMode": text_mode, "time": now_ms() }));
        for call in &tool_calls {
            let name = js_string(&call["function"]["name"]);
            let arguments = js_string(&call["function"]["arguments"]);
            let parsed: Value = serde_json::from_str(&arguments).unwrap_or_else(|_| json!({})); // roh
            let result = if HELPER_TOOLS.contains(&name.as_str()) {
                let ctx = ToolCtx { signal: deps.signal.clone(), web: deps.web.clone(), folder: deps.folder.clone(), chat: deps.chat.clone(), ..Default::default() };
                run_tool(deps.terminal.as_deref(), &name, &Value::String(arguments), &ctx).await
            } else {
                ToolResult::new(false, format!("Hilfsagenten haben nur diese Werkzeuge: {}.", HELPER_TOOLS.join(", ")), "nicht erlaubt")
            };
            messages.push(json!({ "role": "tool", "tool_call_id": call["id"], "name": name, "content": clip(&result.output, TOOL_CHARS), "ok": result.ok, "textMode": text_mode }));
            let d = describe(&name, &parsed);
            steps.push(format!(
                "{} {}({}){}",
                if result.ok { "●" } else { "✗" },
                d.label,
                clip(&d.detail, 80),
                if result.summary.is_empty() { String::new() } else { format!(" – {}", clip(&result.summary, 80)) }
            ));
            progress(&steps);
            if cancelled() {
                break;
            }
        }
    }
    ToolResult::new(false, "Der Hilfsagent ist ohne Ergebnis zu Ende gegangen.", "ohne Ergebnis")
}

/// Führt die Aufgaben mit höchstens `limit` gleichzeitig aus; Ergebnisse in derselben Reihenfolge
pub async fn pool<T, Fut>(tasks: Vec<Fut>, limit: usize) -> Vec<T>
where
    Fut: Future<Output = T>,
{
    futures::stream::iter(tasks).buffered(limit.max(1)).collect().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::askapi::{Completion, ToolCall};
    use crate::tools::EndpointSnapshot;
    use anyhow::Result;
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// ASK-Ersatz: liefert nacheinander die vorbereiteten Antworten und merkt sich die Anfragen
    struct Script {
        answers: Mutex<Vec<(String, Vec<ToolCall>)>>,
        requests: Mutex<Vec<ChatRequest>>,
    }

    #[async_trait]
    impl ChatApi for Script {
        async fn chat(&self, req: ChatRequest) -> Result<Completion> {
            self.requests.lock().push(req);
            let (content, calls) = self.answers.lock().remove(0);
            let mut c = Completion::new();
            c.content = content;
            c.calls = calls;
            Ok(c)
        }
    }

    fn call(id: &str, name: &str, args: Value) -> ToolCall {
        ToolCall { id: id.into(), name: name.into(), arguments: args.to_string() }
    }

    /// Ersatz für `apiMessages` aus agent.js (reicht für diese Tests): System, dann die Nachrichten mit Rolle und Inhalt
    fn api_messages() -> ApiMessagesFn {
        Arc::new(|system, messages, _text_mode| {
            let mut out = vec![json!({ "role": "system", "content": system })];
            out.extend(messages.iter().map(|m| json!({ "role": m["role"], "content": m["content"] })));
            out
        })
    }

    fn deps(ask: Arc<dyn ChatApi>) -> HelperDeps {
        HelperDeps {
            ask,
            model: "klein".into(),
            thinking: String::new(),
            terminal: Some(Arc::new(EndpointSnapshot { state: "gestoppt".into(), port: 0, key: String::new() })),
            web: None,
            folder: std::env::temp_dir().to_string_lossy().to_string(),
            chat: String::new(),
            text_mode: false,
            api_messages: api_messages(),
            text_mode_prompt: Arc::new(|_| String::new()),
            signal: None,
            on_progress: None,
        }
    }

    #[tokio::test]
    async fn hilfsagent_nur_lesende_werkzeuge_ergebnis_mit_schritten_abbruch() {
        let ask = Arc::new(Script {
            answers: Mutex::new(vec![
                (String::new(), vec![call("a", "write_file", json!({ "path": "x.txt", "content": "x" })), call("b", "list_files", json!({}))]),
                ("In a.txt steht die Antwort (Zeile 3).".into(), vec![]),
            ]),
            requests: Mutex::new(Vec::new()),
        });
        let progress: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let pr = progress.clone();
        let mut d = deps(ask.clone());
        d.on_progress = Some(Arc::new(move |p| pr.lock().push(p.to_string())));
        let r = run_helper(&json!({ "titel": "Suche", "auftrag": "Finde die Antwort" }).to_string(), &d).await;
        assert!(r.ok);
        assert!(r.output.starts_with("Ergebnis des Hilfsagenten «Suche»:\nIn a.txt steht die Antwort"), "{}", r.output);
        assert!(r.summary.starts_with("2 Schritte"), "{}", r.summary);
        assert!(regex::Regex::new(r"Schritte:\n✗ .*nicht erlaubt").unwrap().is_match(r.display.as_deref().unwrap()), "{:?}", r.display);
        {
        let requests = ask.requests.lock();
        assert_eq!(requests[0].model, "klein");
        let mut names: Vec<String> = requests[0].tools.as_ref().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap().to_string()).collect();
        names.sort();
        let mut expected: Vec<String> = HELPER_TOOLS.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(names, expected);
        assert!(!requests[0].tools.as_ref().unwrap().iter().any(|t| t["function"]["name"] == "hilfsagent"), "keine Hilfsagenten im Hilfsagenten");
        assert!(requests[0].messages[0]["content"].as_str().unwrap().starts_with("Du bist ein Hilfsagent"));
        let tool_msg = requests[1].messages.iter().find(|m| m["role"] == "tool").unwrap();
        assert!(tool_msg["content"].as_str().unwrap().contains("nur diese Werkzeuge"), "{tool_msg}");
        assert!(progress.lock().len() >= 2);
        }

        let stopped = Signal::new();
        stopped.cancel();
        let mut d2 = deps(ask.clone());
        d2.signal = Some(stopped);
        let s = run_helper(&json!({ "auftrag": "x" }).to_string(), &d2).await;
        assert_eq!(s.summary, "abgebrochen");
        assert!(!run_helper("{}", &deps(ask.clone())).await.ok);
        let mut no_model = deps(ask);
        no_model.model = String::new();
        assert_eq!(run_helper(&json!({ "auftrag": "x" }).to_string(), &no_model).await.summary, "kein Modell");
    }

    #[tokio::test]
    async fn pool_hoechstens_limit_gleichzeitig_reihenfolge_bleibt() {
        let running = Arc::new(AtomicUsize::new(0));
        let most = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = [30u64, 5, 20, 1, 10]
            .into_iter()
            .enumerate()
            .map(|(i, ms)| {
                let (running, most) = (running.clone(), most.clone());
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    most.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    i
                }
            })
            .collect();
        assert_eq!(pool(tasks, 3).await, vec![0, 1, 2, 3, 4]);
        assert_eq!(most.load(Ordering::SeqCst), 3);
    }

    #[test]
    fn werkzeug_und_hilfen() {
        let t = helper_tool();
        assert_eq!(t["function"]["name"], "hilfsagent");
        assert!(t["function"]["description"].as_str().unwrap().contains("(höchstens 3). Eine einzelne bekannte Datei liest du selbst."));
        assert_eq!(swiss(1234567), "1’234’567");
        assert_eq!(swiss(12), "12");
        assert!(helper_prompt("").contains("Arbeitsordner: (keiner)."));
    }
}
