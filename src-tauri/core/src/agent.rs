//! Arbeitsschleife (ENTSCHEIDE E19): Auftrag an das Modell in ASK, Werkzeugaufrufe lokal ausführen,
//! Ergebnisse zurückgeben, bis das Modell ohne Werkzeug antwortet. Chats liegen im Arbeitsordner
//! unter .ask/chats/<id>.json, damit sie zum Projekt gehören.
//!
//! Portiert aus src/main/agent.js. Öffentliche Schnittstelle:
//! - freie Funktionen: `mentions_in`, `api_messages`, `short_args`, `text_mode_prompt`, `loop_reason`,
//!   `retryable`, `goal_tool`, `question_tool`, `plan_tool`; Konstanten `NEW`, `NORMAL`, `TIGHT`, `TOO_LONG`
//! - `AgentOptions` (ein Feld pro Option des JS-Konstruktors, `AgentOptions::new(ask, get_folder, get_model)`
//!   setzt überall dieselben Vorgaben wie JS) und `Agent::new(opts) -> Arc<Agent>`
//! - Ereignisse: `agent.events` (`util::Emitter`) mit «event» (die JS-Nutzlast als JSON) und «changed»
//! - Abfragen: `running`, `undoing`, `chat_id`, `chat_title`, `get_folder`, `pending_questions`,
//!   `pending_approvals`, `turn_started` (AtomicI64), `state`, `list`, `transcript`, `context`, `messages`
//! - Aktionen: `new_chat`, `open`, `open_latest`, `remove`, `rename`, `pin`, `set_goal`, `reset_modes`, `stop`,
//!   `send` (startet den Auftrag in einer eigenen Aufgabe und kehrt sofort zurück), `run_turn` (wie `send`,
//!   wartet aber auf das Ende des Auftrags), `undo`, `rewind`, `answer_question`, `answer_approval`
//!
//! Unterschiede zu JS:
//! * `send` kehrt sofort zurück (main.js wartet nicht darauf); `run_turn` ist das awaitbare `send` aus JS (Tests).
//! * Rückfragen und Fragen des Modells werden vor dem Ereignis «approval» bzw. «question» angemeldet, nicht
//!   danach: Wer im Ereignis gleich antwortet, geht nicht ins Leere (in JS hilft `setImmediate`).
//! * Der Chat bleibt als `serde_json::Value` im Speicher (dieselben Felder und dieselbe Reihenfolge wie in JS),
//!   damit Electron- und Tauri-Fassung die Dateien gegenseitig lesen.
//! * `terminal` ist ein `Arc<dyn TerminalEndpoint>` (ein `Terminal` oder ein Ersatz), `get_snapshots` liefert
//!   `Option<Arc<Snapshots>>`, `auto_approve`/`auto_answer`/`review` sind asynchrone Funktionen.
//! * Ein Fehler der Anfrage ist ein `anyhow::Error` mit `AskError` darin (Status, Teilantwort, Abbruch).

#![allow(clippy::regex_creation_in_loops)] // `re!` legt jeden Ausdruck nur einmal an (OnceLock)

use crate::approvals::{assess_risk, Risk, RiskCtx};
use crate::askapi::{is_abort, is_auth, parse_text_tool_calls, partial_of, status_of, AskError, ChatApi, ChatRequest, Completion, DeltaFn, Signal, Source};
use crate::cleanup::WORK_DIRS;
use crate::github::GitHub;
use crate::helpers::{pool, run_helper, helper_tool, ApiMessagesFn, HelperDeps, TextModePromptFn, HELPER, PARALLEL};
use crate::host::{fixed_confirm, ConfirmFn, ConfirmOptions, Confirmed};
use crate::memorytools::{KnowledgeCtx, MEMORY_TOOLS};
use crate::review::{ReviewFn, ReviewRequest};
use crate::servers::Servers;
use crate::snapshot::Snapshots;
use crate::tools::{clean_args, clip, describe, run_tool, tool_specs, GithubTrust, LiveFn, ProgressFn, TerminalEndpoint, ToolCtx, ToolResult, VisualTools};
use crate::util::{js_string, now_ms, take_chars, truthy, win_basename, same_path, Emitter};
use crate::web::Web;
use crate::bild::Images;
use anyhow::{bail, Result};
use futures::future::BoxFuture;
use parking_lot::Mutex;
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::{Duration, SystemTime};
use tokio::sync::oneshot;

macro_rules! re {
    ($p:expr) => {{
        static R: OnceLock<Regex> = OnceLock::new();
        R.get_or_init(|| Regex::new($p).expect("regex"))
    }};
}

// Keine feste Schrittgrenze für normale Aufträge; der Schleifenschutz (loop_reason) hält ein
// festgefahrenes Modell an. MAX_STEPS ist nur die Notbremse.
const MAX_STEPS: usize = 500;
const SAME_CALL_LIMIT: usize = 3; // derselbe Aufruf so oft hintereinander
const FAIL_LIMIT: usize = 8; // so viele fehlgeschlagene Werkzeugaufrufe hintereinander
const GOAL_IDLE_LIMIT: usize = 2; // Ziel-Modus: so oft ohne Arbeitsschritt anstossen
const GOAL_NUDGE: &str = "Das Ziel ist noch nicht erreicht. Arbeite selbständig weiter. Ist es erreicht, rufe goal_done auf. Brauchst du wirklich eine Antwort der Person, frag mit frage_stellen.";

/// Werkzeug goal_done (Ziel-Modus, /goal)
pub fn goal_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "goal_done",
            "description": "Meldet, dass das Ziel dieses Chats vollständig erreicht ist. Erst aufrufen, wenn alles erledigt und geprüft ist.",
            "parameters": { "type": "object", "properties": { "zusammenfassung": { "type": "string", "description": "Was erreicht wurde, ein bis zwei Sätze" } }, "required": ["zusammenfassung"] },
        },
    })
}

/// Werkzeug denkaufwand (ENTSCHEIDE E62): Bei «Automatisch» wählt das Modell selbst, wie gründlich es die
/// nächsten Schritte überlegt. Jeder Auftrag beginnt bei «mittel».
pub const EFFORT_TOOL: &str = "denkaufwand";
pub const AUTO_START_EFFORT: &str = "mittel";
pub fn effort_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": EFFORT_TOOL,
            "description": "Stellt ein, wie gründlich du die nächsten Schritte dieses Auftrags überlegst. Jeder Auftrag beginnt bei «mittel». Wähle «maximal» für schwierige Schritte (Fehlersuche, Aufbau eines Programms, knifflige Logik, Beweise, lange Texte genau prüfen) und «tief» für einfache, mechanische Schritte (Dateien umbenennen, kurze Antworten, Formatieren). Gilt ab der nächsten Antwort bis zum Ende des Auftrags oder bis du es wieder änderst. Nicht bei jedem Schritt wechseln.",
            "parameters": {
                "type": "object",
                "properties": {
                    "stufe": { "type": "string", "enum": ["tief", "mittel", "maximal"], "description": "tief, mittel oder maximal" },
                    "grund": { "type": "string", "description": "Warum, in wenigen Wörtern" },
                },
                "required": ["stufe"],
            },
        },
    })
}

/// Werkzeug frage_stellen: Das Modell fragt die Person im Chat und wartet auf die Antwort (wie
/// AskUserQuestion in Claude Code). Die App zeigt die Frage mit Antworten zum Anklicken und einem Feld
/// für eine eigene Antwort; auch eine Eingabe im normalen Eingabefeld gilt als Antwort.
pub fn question_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "frage_stellen",
            "description": "Stellt der Person eine Frage und wartet auf ihre Antwort. Nur, wenn für den Auftrag wirklich eine Entscheidung oder Angabe der Person fehlt. Gib wenn möglich 2 bis 4 kurze Antworten zum Anklicken an; die Person kann immer auch frei antworten.",
            "parameters": {
                "type": "object",
                "properties": {
                    "frage": { "type": "string", "description": "Die Frage, kurz und klar, in der Du-Form" },
                    "optionen": { "type": "array", "items": { "type": "string" }, "description": "Antworten zum Anklicken, je höchstens ein paar Wörter (optional, höchstens 6)" },
                    "mehrfach": { "type": "boolean", "description": "true, wenn mehrere Antworten gewählt werden dürfen" },
                },
                "required": ["frage"],
            },
        },
    })
}

/// Planmodus (ENTSCHEIDE E52): Das Modell schaut sich nur um (lesen, suchen, Web) und legt dann mit
/// plan_vorlegen einen Plan vor. Der Auftrag endet dort; die Person setzt ihn um oder ändert ihn.
pub fn plan_tool() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": "plan_vorlegen",
            "description": "Legt der Person den fertigen Plan vor und beendet den Auftrag. Nur im Planmodus.",
            "parameters": {
                "type": "object",
                "properties": { "plan": { "type": "string", "description": "Der Plan in Markdown: kurzer Titel, nummerierte Schritte, betroffene Dateien" } },
                "required": ["plan"],
            },
        },
    })
}

/// Werkzeuge, die im Planmodus erlaubt sind: nichts, was den Ordner oder etwas ausserhalb verändert
const PLAN_ALLOWED: &[&str] = &["read_file", "list_files", "find_files", "search_text", "quellen_suchen", "quelle_lesen", "view_file", "web_search", "web_fetch", HELPER,
    "chat_search", "chat_read", "github_status", "server_liste", "skill_lesen", "mod_liste", "mod_lesen", "katalog_suchen", "update_tasks", "frage_stellen", "plan_vorlegen", EFFORT_TOOL,
    "doc_liste", "doc_lesen"];
/// Chat ohne Projekt (ENTSCHEIDE E78): Gespräch, Web, Bilder, Anhänge lesen und Gedächtnis. Nichts, was Dateien
/// ändert, Befehle ausführt oder nach aussen wirkt; andere Aufrufe lehnt `step` auch dann ab, wenn das Modell sie
/// trotzdem schickt. Docs (ENTSCHEIDE E81) gibt es auch hier: Sie liegen im Ordner dieser Chats unter docs/.
const FREE_ALLOWED: &[&str] = &["read_file", "view_file", "web_search", "web_fetch", "bild_erstellen", "chat_search", "chat_read", "skill_lesen",
    "merken", "vergessen", "update_tasks", "frage_stellen", "goal_done", EFFORT_TOOL,
    "doc_liste", "doc_lesen", "doc_erstellen", "doc_bearbeiten", "doc_kommentieren"];
/// Nebenfrage mit /btw (ENTSCHEIDE E86): eigene Anfrage neben dem Auftrag, ohne Werkzeuge
const BTW_PROMPT: &str = "\n\n# Nebenfrage\nDie Person stellt mit /btw eine Nebenfrage, während du am Auftrag arbeitest (oder danach). Beantworte nur diese Frage, kurz und direkt, aus dem bisherigen Chat und deinem Wissen. Hier hast du keine Werkzeuge: Du kannst nichts lesen, suchen, ändern oder ausführen, und du rufst keine Werkzeuge auf. Der Auftrag läuft unabhängig weiter; ändere ihn nicht und versprich nichts für ihn. Braucht die Frage Werkzeuge, oder soll sich am Auftrag etwas ändern, sag der Person in einem Satz, sie soll es als normale Nachricht schicken.";
const BTW_HEAD: &str = "[Nebenfrage, der Auftrag läuft weiter] ";
/// Ergebnis eines Schritts, der während der Nebenfrage noch läuft (statt «abgebrochen» aus `api_messages`)
const BTW_RUNNING: &str = "Läuft noch: Der Auftrag arbeitet gerade an diesem Schritt.";
const NOT_RUN: &str = "Nicht ausgeführt: Der Schritt wurde abgebrochen, bevor ein Ergebnis da war.";

/// Werkzeuge der Quellen (ENTSCHEIDE E84): nur angeboten, wenn der Ordner «Quellen» Unterlagen hat
const SOURCE_TOOLS: [&str; 2] = ["quellen_suchen", "quelle_lesen"];
const FREE_PROMPT: &str = "\n\n# Chat ohne Projekt\nDieser Chat gehört zu keinem Projekt. Hier beantwortest du Fragen, erklärst, suchst im Web und erstellst auf Wunsch Bilder. Du kannst hier keine Dateien schreiben oder ändern, keine Befehle ausführen und keinen Server starten; es gibt keinen Projektordner. Docs (doc_erstellen, doc_bearbeiten) gehen trotzdem: Sie liegen bei den Chats ohne Projekt. Angehängte Dateien liest du mit read_file, angehängte Bilder siehst du direkt. Will die Person, dass du Dateien erstellst oder bearbeitest, Programme ausführst oder mit einem Ordner arbeitest, sag ihr in einem Satz, dass das in einem Projekt geht: links bei «Projekte» auf + klicken und den Auftrag dort stellen. Code, Texte und Tabellen kannst du hier direkt in der Antwort schreiben.";
const PLAN_PROMPT: &str = "\n\n# Planmodus\nDie Person will zuerst einen Plan sehen. Ändere nichts: keine Befehle, keine Dateien schreiben. Sieh dir mit den Lese- und Suchwerkzeugen an, was du brauchst, frag bei Unklarem mit frage_stellen, und lege dann mit plan_vorlegen einen kurzen, konkreten Plan vor (Schritte, betroffene Dateien). Danach hörst du auf.";

/// Verbindung zu ASK weg oder ASK kurz überlastet: so lange warten und nochmals versuchen (Sekunden)
pub const RETRY_WAITS: [f64; 5] = [2.0, 5.0, 10.0, 20.0, 30.0];

/// Lohnt sich ein neuer Versuch? (Verbindungsfehler und kurze Überlastung ja, Abbruch und Anmeldung nein)
pub fn retryable(err: &anyhow::Error) -> bool {
    if is_abort(err) || is_auth(err) {
        return false;
    }
    let message = err.to_string();
    if message.contains("fünf Minuten lang nichts") {
        return false; // hing schon lange, die Person entscheidet
    }
    if let Some(status) = status_of(err) {
        return [429, 502, 503, 504].contains(&status);
    }
    re!(r"(?i)net::|fetch failed|ECONN|ETIMEDOUT|ENOTFOUND|EAI_AGAIN|socket|network|terminated|other side closed").is_match(&message)
}

/// Wartet `seconds`, bricht beim Stoppen ab
async fn pause(seconds: f64, signal: &Signal) -> Result<()> {
    if signal.is_cancelled() {
        return Err(AskError::abort().into());
    }
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.0))) => Ok(()),
        _ = signal.cancelled() => Err(AskError::abort().into()),
    }
}

fn goal_prompt(goal: &Value) -> String {
    format!(
        "\n\nZiel dieses Chats (von der Person mit /goal gesetzt): {}\nArbeite selbständig und ohne Rückfragen weiter, bis das Ziel vollständig erreicht und geprüft ist. Kündige Arbeit nicht nur an, sondern führe sie mit den Werkzeugen aus. Ist das Ziel erreicht, rufe goal_done auf.",
        js_string(&goal["text"])
    )
}

const KEEP_FULL_TOOL_RESULTS: usize = 8; // ältere Werkzeugergebnisse gehen gekürzt an das Modell

/// Wie viel vom Verlauf an das Modell geht. Ältere Schritte werden gekürzt: Ergebnisse auf «clip» Zeichen,
/// lange Argumente früherer Aufrufe (der Inhalt einer geschriebenen Datei) auf «args» Zeichen; die Datei
/// selbst liegt ja im Ordner. TIGHT gilt, sobald ASK meldet, dass der Verlauf zu lang für das Modell ist.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    pub keep: usize,
    pub clip: usize,
    pub args: usize,
}

pub const NORMAL: Budget = Budget { keep: KEEP_FULL_TOOL_RESULTS, clip: 1500, args: 1200 };
pub const TIGHT: Budget = Budget { keep: 3, clip: 400, args: 200 };

/// Meldungen von ASK, die sagen, dass der Verlauf zu lang für das Modell ist
pub static TOO_LONG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)context length|context window|maximum context|context_length|too many tokens|input tokens|prompt is too long|exceeds? the (model'?s? )?(maximum|max)|reduce the length").expect("regex")
});

/// Zusammenfassen des Verlaufs (ENTSCHEIDE E78): Wird der Chat für das Modell zu lang, fasst ASK die älteren
/// Aufträge zusammen und arbeitet weiter. Die Zusammenfassung hängt an der Eingabe, mit der der erhaltene Teil
/// beginnt (`summaryBefore`); nichts wird gelöscht, die Person sieht weiter den ganzen Verlauf.
const COMPACT_INPUT_MAX: usize = 60_000; // Zeichen des alten Verlaufs, die höchstens in die Anfrage gehen
const COMPACT_TIMEOUT: Duration = Duration::from_secs(150);
const COMPACT_SYSTEM: &str = "Du fasst einen langen Chat zwischen einer Person und dem Assistenten ASK zusammen, damit ASK danach nahtlos weiterarbeiten kann, ohne den alten Verlauf zu sehen. Schreib auf Deutsch (Schweizer Rechtschreibung, ss statt ß), sachlich, in Stichpunkten, höchstens 450 Wörter, mit diesen Teilen:\n1. Ziel: was die Person erreichen will.\n2. Vorgaben und Entscheide der Person (Stil, Werkzeuge, was sie abgelehnt hat).\n3. Erledigt: was schon gemacht ist, mit Dateien und Pfaden.\n4. Offen: was als Nächstes ansteht, bekannte Fehler.\n5. Wichtige Angaben: Namen, Zahlen, Befehle, Adressen, die später gebraucht werden.\nNichts erfinden. Keine Anrede, kein Schlusswort.";
const SUMMARY_HEAD: &str = "[Zusammenfassung des bisherigen Chats. Der frühere Verlauf ist für dich nicht mehr sichtbar, weil er zu lang wurde; die Person sieht ihn noch.]\n";
const COMPACT_NOTICE: &str = "Der Chat ist lang geworden: Ältere Aufträge gehen ab hier als Zusammenfassung an das Modell. Du siehst weiterhin den ganzen Verlauf.";

/// Ordner verschieben: umbenennen, über Laufwerke hinweg kopieren und danach löschen
fn move_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    fn copy(a: &Path, b: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(b)?;
        for entry in std::fs::read_dir(a)? {
            let entry = entry?;
            let target = b.join(entry.file_name());
            if entry.file_type()?.is_dir() { copy(&entry.path(), &target)?; } else { std::fs::copy(entry.path(), &target)?; }
        }
        Ok(())
    }
    if let Err(e) = copy(from, to) {
        let _ = std::fs::remove_dir_all(to);
        return Err(e);
    }
    std::fs::remove_dir_all(from)
}

/// Beginnt hier ein Auftrag der Person? (nicht: Anstoss im Ziel-Modus, Bilder einer Ansicht)
fn turn_start(m: &Value) -> bool {
    m["role"] == "user" && !truthy(&m["auto"]) && m["note"] != "view"
}

/// Stelle, ab der der Verlauf wörtlich an das Modell geht: die letzte Eingabe mit Zusammenfassung davor
pub fn summary_index(messages: &[Value]) -> Option<usize> {
    messages.iter().rposition(|m| truthy(&m["summaryBefore"]["content"]))
}

/// Was zusammengefasst wird: (Anfang, Schnitt). Ab dem Schnitt bleiben die letzten `keep` Aufträge wörtlich.
/// `None`: Es gibt davor nichts zusammenzufassen.
pub fn compact_range(messages: &[Value], keep: usize) -> Option<(usize, usize)> {
    let start = summary_index(messages).unwrap_or(0);
    let turns: Vec<usize> = (start..messages.len()).filter(|i| turn_start(&messages[*i])).collect();
    let cut = *turns.get(turns.len().checked_sub(keep.max(1))?)?;
    // Mindestens ein ganzer Auftrag muss vor dem Schnitt liegen
    (cut > start && messages[start..cut].iter().any(|m| m["role"] == "assistant" || m["role"] == "tool")).then_some((start, cut))
}

/// Alter Verlauf als Text für die Anfrage zum Zusammenfassen: Eingaben, Antworten und Werkzeuge kurz
pub fn compact_text(messages: &[Value], max: usize) -> String {
    let mut lines: Vec<String> = Vec::new();
    for m in messages {
        if truthy(&m["summaryBefore"]["content"]) {
            lines.push(format!("Frühere Zusammenfassung:\n{}", js_string(&m["summaryBefore"]["content"])));
        }
        match m["role"].as_str().unwrap_or("") {
            "user" if m["note"] == "view" => {}
            "user" if truthy(&m["auto"]) => {}
            "user" => {
                let files: Vec<String> = arr(&m["attachments"]).iter().map(|a| js_string(&a["name"])).collect();
                let tail = if files.is_empty() { String::new() } else { format!(" [Anhänge: {}]", files.join(", ")) };
                lines.push(format!("Person: {}{tail}", clip(&js_string(&m["content"]), 2000)));
            }
            "assistant" => {
                let text = js_string(&m["content"]);
                if !text.trim().is_empty() {
                    lines.push(format!("ASK: {}", clip(&text, 2000)));
                }
                for c in arr(&m["tool_calls"]) {
                    lines.push(format!("ASK ruft {} auf: {}", js_string(&c["function"]["name"]), clip(&js_string(&c["function"]["arguments"]), 240)));
                }
            }
            "tool" => {
                let ok = if m["ok"] == json!(false) { " (fehlgeschlagen)" } else { "" };
                let text = if truthy(&m["summary"]) { js_string(&m["summary"]) } else { clip(&js_string(&m["content"]), 240) };
                lines.push(format!("Ergebnis {}{ok}: {text}", js_string(&m["name"])));
            }
            _ => {}
        }
    }
    clip(&lines.join("\n"), max)
}

const IMAGE_TURNS: usize = 2; // Bilder aus so vielen der letzten Eingaben gehen an das Modell
const VIEW_TURNS: usize = 2; // ebenso aus so vielen der letzten Ansichten (view_file, browser)
const CHAT_DIR: [&str; 2] = [".ask", "chats"];
/// openLatest: leeren Chat beginnen
pub const NEW: &str = "neu";

/// `normRel`: Pfad in Kleinschrift mit «/» ohne führenden Punkt und Trenner
fn norm_rel(p: &str) -> String {
    re!(r"^\.?[\\/]+").replace(p.trim(), "").replace('\\', "/").to_lowercase()
}

fn new_id() -> String {
    format!("{}-{}", chrono::Utc::now().format("%Y%m%d%H%M%S"), hex::encode(rand::random::<[u8; 3]>()))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn arr(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// Steckt das Modell in einer Schleife? Betrachtet nur den laufenden Auftrag (seit der letzten Eingabe).
pub fn loop_reason(messages: &[Value]) -> String {
    let start = messages.iter().rposition(|m| m["role"] == "user" && m["note"] != "view").map(|i| i + 1).unwrap_or(0);
    let turn = &messages[start..];
    let mut results: Vec<bool> = Vec::new();
    // Ergebnis pro Aufruf: Abfragen wie command_status mit neuer Ausgabe sind keine Schleife
    let mut output: HashMap<Option<String>, String> = HashMap::new();
    for m in turn {
        if m["role"] == "tool" {
            results.push(m["ok"] != json!(false));
            output.insert(m.get("tool_call_id").map(js_string), take_chars(&js_string(&m["content"]), 4000));
        }
    }
    let mut calls: Vec<String> = Vec::new();
    for m in turn {
        if m["role"] == "assistant" {
            for c in arr(&m["tool_calls"]) {
                let out = output.get(&c.get("id").map(js_string)).cloned().unwrap_or_default();
                calls.push(format!("{} {}\u{0}{}", js_string(&c["function"]["name"]), js_string(&c["function"]["arguments"]), out));
            }
        }
    }
    let last = &calls[calls.len().saturating_sub(SAME_CALL_LIMIT)..];
    if last.len() == SAME_CALL_LIMIT && last.iter().all(|c| *c == last[0]) {
        return format!(
            "Das Modell hat {SAME_CALL_LIMIT}-mal hintereinander genau denselben Schritt mit demselben Ergebnis ausgeführt ({}).",
            last[0].split(' ').next().unwrap_or("")
        );
    }
    let fails = &results[results.len().saturating_sub(FAIL_LIMIT)..];
    if fails.len() == FAIL_LIMIT && fails.iter().all(|ok| !ok) {
        return format!("{FAIL_LIMIT} Werkzeugaufrufe hintereinander sind fehlgeschlagen.");
    }
    String::new()
}

/// Werkzeuge im Systemprompt, falls ASK den Parameter «tools» nicht weitergibt (Textmodus).
pub fn text_mode_prompt(specs: &[Value]) -> String {
    let list = specs.iter().map(|s| s["function"].to_string()).collect::<Vec<_>>().join("\n");
    format!(
        "\n\n# Werkzeuge\nDu kannst diese Werkzeuge aufrufen:\n<tools>\n{list}\n</tools>\nFür einen Aufruf schreibst du genau:\n<tool_call>\n{{\"name\": \"<werkzeug>\", \"arguments\": {{...}}}}\n</tool_call>\nDanach wartest du auf das Ergebnis in <tool_response>."
    )
}

/// Text einer Eingabe mit Anhängen: das Modell erfährt, wo die Dateien liegen.
/// Mit /name aufgerufener Skill (skills.rs): Die Anleitung geht vor dem Text an das Modell, die Person sieht nur ihre Eingabe.
fn user_text(m: &Value) -> String {
    let files: Vec<String> = arr(&m["attachments"]).iter().map(|a| js_string(&a["path"])).collect();
    let skill = if truthy(&m["skill"]["text"]) {
        format!(
            "[Die Person ruft ihren Skill «{}» auf. Anleitung des Skills:]\n{}\n[Ende der Anleitung]\n\n",
            js_string(&m["skill"]["name"]),
            js_string(&m["skill"]["text"])
        )
    } else {
        String::new()
    };
    let content = js_string(&m["content"]);
    let mut text = content.clone();
    if !files.is_empty() {
        let shown = if content.is_empty() { "Siehe Anhang.".to_string() } else { content };
        text = format!("{shown}\n\n[Angehängt, liegt im Arbeitsordner: {}]", files.join(", "));
    }
    let (templates, mentions): (Vec<String>, Vec<String>) = arr(&m["mentions"]).iter().map(js_string).partition(|p| crate::vorlagen::is_template_ref(p));
    if !mentions.is_empty() {
        text.push_str(&format!("\n\n[Mit @ erwähnt, liegt im Arbeitsordner: {}. Lies es, wenn du es brauchst.]", mentions.join(", ")));
    }
    if !templates.is_empty() {
        // Vorlagen der TF Bern (ENTSCHEIDE E66): nur lesen, in eine Kopie im Projekt
        text.push_str(&format!(
            "\n\n[Gewählte Vorlage der TF Bern (nur lesen): {}. Kopiere sie in den Arbeitsordner und arbeite in der Kopie.]",
            templates.iter().map(|p| p.replace('/', "\\")).collect::<Vec<_>>().join(", ")
        ));
    }
    skill + &text
}

/// @Datei im Eingabefeld (@pfad oder @"pfad mit Leerzeichen"): nur Pfade, die es im Ordner gibt
pub fn mentions_in(text: &str, exists: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for m in re!(r#"(?:^|\s)@(?:"([^"\n]+)"|([^\s"]+))"#).captures_iter(text) {
        let raw = m.get(1).or_else(|| m.get(2)).map(|g| g.as_str()).unwrap_or("");
        let rel = re!(r"[.,;:!?)]+$").replace(raw, "").to_string();
        if !rel.is_empty() && !out.contains(&rel) && exists(&rel) {
            out.push(rel);
        }
    }
    out.truncate(20);
    out
}

/// Lange Texte in den Argumenten eines früheren Aufrufs kürzen; das Ergebnis bleibt gültiges JSON.
pub fn short_args(raw: &str, max: usize) -> String {
    if raw.is_empty() || raw.chars().count() <= max {
        return raw.to_string();
    }
    let Ok(args) = serde_json::from_str::<Value>(raw) else { return raw.to_string() };
    let cut = |v: &Value| -> Value {
        match v {
            Value::String(s) if s.chars().count() > max => json!(format!(
                "{}\n… (für den Verlauf gekürzt, {} Zeichen; den heutigen Stand zeigt read_file)",
                take_chars(s, max / 2),
                s.encode_utf16().count()
            )),
            other => other.clone(),
        }
    };
    match &args {
        Value::Object(o) => Value::Object(o.iter().map(|(k, v)| (k.clone(), cut(v))).collect::<Map<_, _>>()).to_string(),
        Value::Array(a) => Value::Object(a.iter().enumerate().map(|(i, v)| (i.to_string(), cut(v))).collect::<Map<_, _>>()).to_string(),
        _ => raw.to_string(),
    }
}

/// Anhang → data-URL, geliehen (`imageUrl` in `apiMessages`)
pub type ImageFn = dyn Fn(&Value) -> Option<String>;

/// Anhang → data-URL für das Modell (`getImageUrl`)
pub type ImageUrlFn = Arc<dyn Fn(&Value) -> Option<String> + Send + Sync>;

/// Verlauf für die Anfrage: ohne Denktexte, alte Ergebnisse gekürzt, im Textmodus ohne tool-Rollen
/// (auch für frühere Schritte, die noch mit Werkzeugliste liefen). `image_url(anhang)` liefert Bilder
/// als data-URL; sie gehen nur bei den letzten IMAGE_TURNS Eingaben mit, ältere nur als Pfad.
const WEEKDAYS: [&str; 7] = ["Montag", "Dienstag", "Mittwoch", "Donnerstag", "Freitag", "Samstag", "Sonntag"];
const MONTHS: [&str; 12] = ["Januar", "Februar", "März", "April", "Mai", "Juni", "Juli", "August", "September", "Oktober", "November", "Dezember"];

/// Wann die Person eine Nachricht geschickt hat, für das Modell: «[Sa 28.11.2026 14:05]» in Ortszeit
/// (ENTSCHEIDE E85). Leer ohne gültige Zeit.
pub fn time_stamp(ms: i64) -> String {
    use chrono::{Datelike, TimeZone};
    match chrono::Local.timestamp_millis_opt(ms).single() {
        Some(t) if ms > 0 => format!("[{} {}]", take_chars(WEEKDAYS[t.weekday().num_days_from_monday() as usize], 2), t.format("%d.%m.%Y %H:%M")),
        _ => String::new(),
    }
}

/// Abschnitt für den Systemprompt: das heutige Datum (nur das Datum, damit der Anfang der Anfrage über den Tag
/// gleich bleibt) und was die Zeitstempel vor den Nachrichten bedeuten (E85)
pub fn date_prompt(now: chrono::DateTime<chrono::Local>) -> String {
    use chrono::Datelike;
    format!(
        "\n\n# Datum\nHeute ist {}, {}. {} {}. Vor jeder Nachricht der Person steht in eckigen Klammern, wann sie sie geschickt hat (Ortszeit). Nutze das für «heute», «morgen», «letzte Woche» und Fristen; schreib diese Zeitangaben nicht selbst in deine Antworten.",
        WEEKDAYS[now.weekday().num_days_from_monday() as usize],
        now.day(),
        MONTHS[now.month0() as usize],
        now.year()
    )
}

pub fn api_messages(system: &str, messages: &[Value], text_mode: bool, image_url: Option<&ImageFn>, budget: &Budget) -> Vec<Value> {
    // Zusammengefasster Verlauf (E78): Das Modell bekommt die Zusammenfassung und alles ab der Eingabe, an der sie hängt
    let (summary, messages) = match summary_index(messages) {
        Some(i) => (Some(js_string(&messages[i]["summaryBefore"]["content"])), &messages[i..]),
        None => (None, messages),
    };
    let tool_indexes: Vec<usize> = messages.iter().enumerate().filter(|(_, m)| m["role"] == "tool").map(|(i, _)| i).collect();
    let recent_list = &tool_indexes[tool_indexes.len().saturating_sub(budget.keep)..];
    let recent: HashSet<usize> = recent_list.iter().copied().collect();
    let first_recent = recent_list.first().copied().unwrap_or(messages.len());
    // Aufrufe eines Schritts, dessen Ergebnisse schon gekürzt sind: auch die Argumente kürzen
    let calls = |m: &Value, i: usize| -> Vec<Value> {
        let list = arr(&m["tool_calls"]);
        if i + list.len() < first_recent {
            list.iter()
                .map(|c| {
                    let mut c = c.clone();
                    if let Some(raw) = c["function"]["arguments"].as_str() {
                        c["function"]["arguments"] = json!(short_args(raw, budget.args));
                    }
                    c
                })
                .collect()
        } else {
            list.to_vec()
        }
    };
    // Bilder: Anhänge der letzten IMAGE_TURNS Eingaben, Ansichten (view_file, browser) der letzten VIEW_TURNS
    let with_image = |view: bool| -> Vec<usize> {
        messages
            .iter()
            .enumerate()
            .filter(|(_, m)| m["role"] == "user" && (m["note"] == "view") == view && arr(&m["attachments"]).iter().any(|a| truthy(&a["image"])))
            .map(|(i, _)| i)
            .collect()
    };
    let with_images: HashSet<usize> = if image_url.is_some() {
        let (plain, views) = (with_image(false), with_image(true));
        plain[plain.len().saturating_sub(IMAGE_TURNS)..].iter().chain(views[views.len().saturating_sub(VIEW_TURNS)..].iter()).copied().collect()
    } else {
        HashSet::new()
    };
    let mut out = vec![json!({ "role": "system", "content": system })];
    if let Some(summary) = summary {
        out.push(json!({ "role": "user", "content": format!("{SUMMARY_HEAD}{summary}") }));
    }
    for (i, m) in messages.iter().enumerate() {
        match m["role"].as_str().unwrap_or("") {
            "user" => {
                let urls: Vec<String> = match image_url {
                    Some(f) if with_images.contains(&i) => arr(&m["attachments"]).iter().filter(|a| truthy(&a["image"])).filter_map(f).filter(|u| !u.is_empty()).collect(),
                    _ => Vec::new(),
                };
                let text = if m["note"] == "view" {
                    if urls.is_empty() { format!("{}: nicht mehr mitgeschickt.", js_string(&m["content"])) } else { js_string(&m["content"]) }
                } else {
                    // Zeitpunkt der Eingabe (E85), nur bei echten Eingaben der Person, nicht bei Hinweisen der App
                    let own = m.get("note").is_none_or(Value::is_null) && !truthy(&m["auto"]) && !truthy(&m["helper"]);
                    let stamp = if own { time_stamp(m["time"].as_i64().unwrap_or(0)) } else { String::new() };
                    if stamp.is_empty() { user_text(m) } else { format!("{stamp} {}", user_text(m)) }
                };
                if urls.is_empty() {
                    out.push(json!({ "role": "user", "content": text }));
                } else {
                    let mut parts = vec![json!({ "type": "text", "text": text })];
                    parts.extend(urls.iter().map(|url| json!({ "type": "image_url", "image_url": { "url": url } })));
                    out.push(json!({ "role": "user", "content": parts }));
                }
            }
            "assistant" => {
                let has_calls = !arr(&m["tool_calls"]).is_empty();
                let old = has_calls && i + arr(&m["tool_calls"]).len() < first_recent;
                let m_text_mode = truthy(&m["textMode"]);
                if m_text_mode && !old {
                    let raw = if truthy(&m["raw"]) { js_string(&m["raw"]) } else { js_string(&m["content"]) };
                    out.push(json!({ "role": "assistant", "content": raw }));
                } else if (text_mode || m_text_mode) && has_calls {
                    let written: Vec<String> = calls(m, i).iter().map(|c| {
                        let args = js_string(&c["function"]["arguments"]);
                        format!(
                            "<tool_call>\n{{\"name\": {}, \"arguments\": {}}}\n</tool_call>",
                            Value::String(js_string(&c["function"]["name"])),
                            if args.is_empty() { "{}".to_string() } else { args }
                        )
                    }).collect();
                    let content = std::iter::once(js_string(&m["content"])).filter(|c| !c.is_empty()).chain(written).collect::<Vec<_>>().join("\n");
                    out.push(json!({ "role": "assistant", "content": content }));
                } else {
                    let mut msg = json!({ "role": "assistant", "content": js_string(&m["content"]) });
                    if has_calls {
                        msg["tool_calls"] = json!(calls(m, i));
                    }
                    out.push(msg);
                }
            }
            "tool" => {
                let content = if recent.contains(&i) { js_string(&m["content"]) } else { clip(&js_string(&m["content"]), budget.clip) };
                if truthy(&m["textMode"]) || text_mode {
                    out.push(json!({ "role": "user", "content": format!("<tool_response>\n{content}\n</tool_response>") }));
                } else {
                    let mut msg = json!({ "role": "tool", "content": content });
                    if let Some(id) = m.get("tool_call_id") {
                        msg = json!({ "role": "tool", "tool_call_id": id, "content": msg["content"] });
                    }
                    out.push(msg);
                }
            }
            _ => {}
        }
    }
    // Aufrufe ohne Ergebnis (die App wurde während des Schritts beendet): ASK lehnt einen Verlauf ab, in dem auf
    // tool_calls nicht für jeden Aufruf eine Antwort folgt, und der Chat wäre für immer unbrauchbar
    let mut i = 0;
    while i < out.len() {
        let ids: Vec<Value> = if out[i]["role"] == "assistant" { arr(&out[i]["tool_calls"]).iter().map(|c| c["id"].clone()).collect() } else { Vec::new() };
        i += 1;
        if ids.is_empty() {
            continue;
        }
        let mut answered: Vec<Value> = Vec::new();
        while i < out.len() && out[i]["role"] == "tool" {
            answered.push(out[i]["tool_call_id"].clone());
            i += 1;
        }
        for id in ids.into_iter().filter(|id| !answered.contains(id)) {
            out.insert(i, json!({ "role": "tool", "tool_call_id": id, "content": NOT_RUN }));
            i += 1;
        }
    }
    out
}

/// Titel und Kurztext eines Aufrufs für die Oberfläche, dazu bei Fragen die Antworten zum Anklicken
fn describe_call(call: &Value) -> Map<String, Value> {
    let name = js_string(&call["function"]["name"]);
    let raw = js_string(&call["function"]["arguments"]);
    let args = serde_json::from_str::<Value>(if raw.is_empty() { "{}" } else { &raw }).ok().filter(Value::is_object).unwrap_or_else(|| json!({}));
    let text_of = |keys: &[&str]| keys.iter().filter_map(|k| args.get(*k)).find(|v| truthy(v)).map(js_string).unwrap_or_default();
    let mut out = Map::new();
    if name == HELPER {
        out.insert("label".into(), json!("Hilfsagent"));
        out.insert("detail".into(), json!(clip(&text_of(&["titel", "auftrag"]), 120)));
    } else if name == "frage_stellen" {
        let optionen: Vec<String> = arr(&args["optionen"]).iter().map(|o| js_string(o).trim().to_string()).filter(|o| !o.is_empty()).take(6).collect();
        out.insert("label".into(), json!("Frage"));
        out.insert("detail".into(), json!(text_of(&["frage"])));
        out.insert("optionen".into(), json!(optionen));
    } else if name == "plan_vorlegen" {
        let plan = text_of(&["plan"]);
        let first = plan.split('\n').map(|l| re!(r"^#+\s*").replace(l, "").trim().to_string()).find(|l| !l.is_empty()).unwrap_or_default();
        out.insert("label".into(), json!("Plan"));
        out.insert("detail".into(), json!(clip(&first, 120)));
    } else {
        let d = describe(&name, &args);
        out.insert("label".into(), json!(d.label));
        out.insert("detail".into(), json!(d.detail));
    }
    out
}

/// ASK hat die Grenze des Kontextfensters genannt: (Modell, Token)
pub type ContextLimitFn = Arc<dyn Fn(&str, i64) + Send + Sync>;
/// Antwort statt der Person auf eine Frage des Modells: (Frage, Antworten) → Antwort oder None (gestoppt)
pub type AutoAnswerFn = Arc<dyn Fn(String, Vec<String>) -> BoxFuture<'static, Option<String>> + Send + Sync>;

/// Optionen des Agenten: ein Feld pro Option des JS-Konstruktors (main.js). `AgentOptions::new` setzt dieselben
/// Vorgaben wie JS; Felder, die JS ohne Vorgabe verlangt, bekommen leere Funktionen.
#[derive(Clone)]
pub struct AgentOptions {
    pub ask: Arc<dyn ChatApi>,
    /// Das lokale Terminal (`Terminal` oder ein Ersatz); fehlt es, scheitern Werkzeuge, die es brauchen
    pub terminal: Option<Arc<dyn TerminalEndpoint>>,
    pub get_folder: Arc<dyn Fn() -> String + Send + Sync>,
    pub get_model: Arc<dyn Fn() -> String + Send + Sync>,
    pub get_system_prompt: Arc<dyn Fn(&str) -> String + Send + Sync>,
    pub get_command_hint: Arc<dyn Fn(&str) -> String + Send + Sync>,
    pub get_web_search: Arc<dyn Fn() -> bool + Send + Sync>,
    pub get_thinking: Arc<dyn Fn() -> String + Send + Sync>,
    pub github: Option<Arc<GitHub>>,
    pub github_trust: Option<Arc<GithubTrust>>,
    pub visual: Option<Arc<dyn VisualTools>>,
    pub web: Option<Arc<Web>>,
    pub images: Option<Arc<Images>>,
    /// Rückfrage vor Aktionen nach aussen und vor dem Zurücksetzen; ohne: immer Nein
    pub confirm: Option<ConfirmFn>,
    /// Anhang → data-URL für das Modell
    pub get_image_url: Option<ImageUrlFn>,
    /// false: Modell versteht laut ASK keine Bilder; None: unbekannt
    pub get_vision: Arc<dyn Fn() -> Option<bool> + Send + Sync>,
    pub get_snapshots: Arc<dyn Fn() -> Option<Arc<Snapshots>> + Send + Sync>,
    /// Schalter «Chats»: frühere Chats lesen (history.rs)
    pub get_chat_memory: Arc<dyn Fn() -> bool + Send + Sync>,
    /// Kontextfenster des Modells in Token, 0: unbekannt
    pub get_context_limit: Arc<dyn Fn(&str) -> i64 + Send + Sync>,
    /// ASK hat die Grenze in einer Fehlermeldung genannt (Modell, Token)
    pub on_context_limit: ContextLimitFn,
    /// Stufe der Rückfrage vor heiklen Schritten: aus | heikel | alle
    pub get_approval_mode: Arc<dyn Fn() -> String + Send + Sync>,
    pub file_exists: Arc<dyn Fn(&str) -> bool + Send + Sync>,
    /// nur in automatischen Tests: antwortet statt der Person auf Rückfragen («yes», «always», «no»)
    pub auto_approve: Option<Arc<dyn Fn(Risk) -> BoxFuture<'static, String> + Send + Sync>>,
    /// nur in automatischen Tests: antwortet statt der Person auf Fragen des Modells (Frage, Antworten)
    pub auto_answer: Option<AutoAnswerFn>,
    /// auto | nachfragen | plan (Eingabefeld)
    pub get_mode: Arc<dyn Fn() -> String + Send + Sync>,
    /// Wartezeiten zwischen den Versuchen in Sekunden (Standard `RETRY_WAITS`)
    pub retry_waits: Option<Vec<f64>>,
    pub servers: Option<Arc<Servers>>,
    /// Modus «Auto»: Prüfer in ASK (review.rs)
    pub review: Option<ReviewFn>,
    /// Gedächtnis und eigene Skills (memorytools.rs); `memory_on()` aus: ohne merken/vergessen
    pub knowledge: Option<KnowledgeCtx>,
    /// Titel der anderen Chats, die gerade im selben Ordner arbeiten
    pub get_siblings: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    /// Modell der Hilfsagenten; leer: Modell des Chats
    pub get_helper_model: Arc<dyn Fn() -> String + Send + Sync>,
    /// Regeln eingeschalteter Mods: vor diesen Arten von Schritten immer fragen (mods.rs, E72)
    pub get_ask_before: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
    /// Nach der ersten Antwort einen Titel von ASK schreiben lassen (in Tests aus, sonst bekäme die Attrappe eine Anfrage mehr)
    pub auto_title: bool,
    /// Chat ohne Projekt (ENTSCHEIDE E78): nur die Werkzeuge aus `FREE_ALLOWED`, kein Planmodus
    pub free: bool,
    /// Reserve des Kontextfensters in Token: Ist der Verlauf bis auf diese Reserve voll, fasst ASK vor dem nächsten
    /// Auftrag die älteren Aufträge zusammen (E78). 0: nie vorbeugend, nur wenn ASK den Verlauf ablehnt.
    pub get_compact_reserve: Arc<dyn Fn() -> i64 + Send + Sync>,
    /// Quellen des Projekts (ENTSCHEIDE E84): (gibt es Unterlagen, «Nur aus Quellen»), bei jedem Schritt gelesen
    pub get_sources: Arc<dyn Fn() -> (bool, bool) + Send + Sync>,
}

impl AgentOptions {
    pub fn new(ask: Arc<dyn ChatApi>, get_folder: impl Fn() -> String + Send + Sync + 'static, get_model: impl Fn() -> String + Send + Sync + 'static) -> Self {
        AgentOptions {
            ask,
            terminal: None,
            get_folder: Arc::new(get_folder),
            get_model: Arc::new(get_model),
            get_system_prompt: Arc::new(|_| String::new()),
            get_command_hint: Arc::new(|_| String::new()),
            get_web_search: Arc::new(|| false),
            get_thinking: Arc::new(|| "normal".to_string()),
            github: None,
            github_trust: None,
            visual: None,
            web: None,
            images: None,
            confirm: None,
            get_image_url: None,
            get_vision: Arc::new(|| None),
            get_snapshots: Arc::new(|| None),
            get_chat_memory: Arc::new(|| false),
            get_context_limit: Arc::new(|_| 0),
            on_context_limit: Arc::new(|_, _| {}),
            get_approval_mode: Arc::new(|| "aus".to_string()),
            file_exists: Arc::new(|_| false),
            auto_approve: None,
            auto_answer: None,
            get_mode: Arc::new(|| "nachfragen".to_string()),
            retry_waits: None,
            servers: None,
            review: None,
            knowledge: None,
            get_siblings: Arc::new(Vec::new),
            get_helper_model: Arc::new(String::new),
            get_ask_before: Arc::new(Vec::new),
            auto_title: false,
            free: false,
            get_compact_reserve: Arc::new(|| 0),
            get_sources: Arc::new(|| (false, false)),
        }
    }
}

/// Antwort des Modells → Titel: erste Zeile ohne Denkblock, Vorsilben, Anführungszeichen und Schlusspunkt
pub fn clean_title(text: &str) -> String {
    let without_think = re!(r"(?s)<think>.*?</think>").replace_all(text, "");
    let line = without_think.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    let line = re!(r"^(?i)(titel|title)\s*:\s*").replace(line, "");
    let line = line.trim_matches(|c: char| "«»\"'„“”*#`.!:".contains(c) || c.is_whitespace());
    let clean = re!(r"\s+").replace_all(line, " ").to_string();
    // Eine Frage oder ein ganzer Satz ist kein Titel (das Modell hat dann die Rückfrage von ASK übernommen):
    // lieber den vorläufigen Titel behalten und nach dem nächsten Auftrag nochmals versuchen
    if clean.chars().count() < 2 || clean.contains('?') || clean.split_whitespace().count() > 8 { String::new() } else { take_chars(&clean, 60) }
}

/// Rückfrage aus einer Regel eines Mods («nachfragen»): Mods können ASK nur strenger machen
pub fn mod_rule_risk(name: &str, args: &Value, kinds: &[String]) -> Option<Risk> {
    let has = |k: &str| kinds.iter().any(|x| x == k);
    let command = || js_string(args.get(if name == "server_starten" { "befehl" } else { "command" }).unwrap_or(&Value::Null));
    let risk = |kind: &str, title: String, detail: String| Some(Risk { kind: format!("mod-{kind}"), title, detail, reason: "Ein eingeschalteter Mod verlangt hier eine Rückfrage.".into() });
    match name {
        "run_command" | "server_starten" if has("loeschen") && crate::approvals::analyze(&command(), 0).del => risk("loeschen", "Darf ASK Dateien löschen?".into(), take_chars(&command(), 300)),
        "run_command" | "server_starten" if has("befehle") => risk("befehle", "Befehl ausführen?".into(), take_chars(&command(), 300)),
        "write_file" | "edit_file" if has("schreiben") => {
            let path = js_string(args.get("path").unwrap_or(&Value::Null));
            risk("schreiben", format!("Darf ASK «{path}» schreiben?"), path)
        }
        "doc_erstellen" | "doc_bearbeiten" if has("schreiben") => {
            let path = if name == "doc_erstellen" { js_string(args.get("titel").unwrap_or(&Value::Null)) } else { js_string(args.get("pfad").unwrap_or(&Value::Null)) };
            risk("schreiben", format!("Darf ASK das Doc «{path}» schreiben?"), path)
        }
        // Hilfsagenten suchen und lesen selbst im Web (helpers.rs), ohne eigene Rückfrage: darum einmal vor dem Start
        HELPER if has("web") => {
            let what = ["titel", "auftrag"].iter().find_map(|k| args.get(*k).filter(|v| truthy(v)).map(js_string)).unwrap_or_default();
            risk("web", "Darf der Hilfsagent ins Internet?".into(), take_chars(&what, 300))
        }
        "web_search" | "web_fetch" | "browser" if has("web") => {
            let what = ["query", "url", "address", "action"].iter().find_map(|k| args.get(*k).filter(|v| truthy(v)).map(js_string)).unwrap_or_default();
            risk("web", "Darf ASK ins Internet?".into(), take_chars(&what, 300))
        }
        _ => None,
    }
}

/// Eine Rückfrage vor einem Schritt: das Risiko und ob der Prüfer es festgestellt hat
struct Asked {
    risk: Risk,
    reviewed: bool,
}

/// Quellen dieses Auftrags: mit web_fetch gelesene Seiten und Treffer von web_search
#[derive(Default)]
struct TurnSources {
    read: Vec<Source>,
    found: Vec<Source>,
}

struct Inner {
    chat: Value,
    allowed: HashSet<String>,
    reviewed_safe: HashSet<String>,
    made_this_turn: HashSet<String>,
    /// Denkaufwand, den das Modell bei «Automatisch» für diesen Auftrag gewählt hat (leer: noch nichts)
    auto_effort: String,
    text_mode_for: HashSet<String>,
    no_images_for: HashSet<String>,
    tight_for: HashSet<String>,
    last_skipped: String,
    turn_sources: Option<TurnSources>,
    /// Grösse des Verlaufs: (Chat, Anzahl Nachrichten, Token), nur für die Schätzung in `context`
    /// Schätzung der Token: (Chat, Anzahl Nachrichten, Stelle der Zusammenfassung, Token)
    sized: Option<(String, usize, usize, i64)>,
    summaries: HashMap<PathBuf, (SystemTime, Value)>,
}

/// Hilfsagenten eines Schritts: laufen gleichzeitig (höchstens `PARALLEL`) in einer eigenen Aufgabe
struct HelperBatch {
    handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<Vec<ToolResult>>>>,
    results: tokio::sync::OnceCell<Vec<ToolResult>>,
}

impl HelperBatch {
    async fn get(&self, k: usize) -> ToolResult {
        let all = self
            .results
            .get_or_init(|| async {
                match self.handle.lock().await.take() {
                    Some(h) => h.await.unwrap_or_default(),
                    None => Vec::new(),
                }
            })
            .await;
        all.get(k).cloned().unwrap_or_else(|| ToolResult::new(false, "Fehler im Hilfsagenten: ohne Ergebnis", "Fehler"))
    }
}

pub struct Agent {
    /// «event» (Nutzlast wie in JS) und «changed»
    pub events: Emitter,
    /// main.js setzt es beim Ereignis «user» (Zeit des Auftragsbeginns)
    pub turn_started: AtomicI64,
    o: AgentOptions,
    inner: Mutex<Inner>,
    running: AtomicBool,
    undoing: AtomicBool,
    controller: Mutex<Option<Signal>>,
    approvals: Mutex<HashMap<String, oneshot::Sender<String>>>,
    /// Titel und Details offener Rückfragen (für die Bestätigung bei eingeschalteten Mods, E72)
    approval_text: Mutex<HashMap<String, (String, String)>>,
    questions: Mutex<HashMap<String, oneshot::Sender<String>>>,
    /// Ereignisse offener Fragen und Rückfragen (Kennung → Ereignis). Wer in einen Chat wechselt, der im Hintergrund
    /// auf eine Antwort wartet, bekommt sie mit dem Verlauf nochmals (`pending_events`), sonst sähe man die Frage nicht
    /// mehr und könnte nur noch stoppen.
    open_events: Mutex<Vec<(String, Value)>>,
    /// Es läuft gerade eine Anfrage zum Zusammenfassen
    compacting: AtomicBool,
    /// Das letzte Speichern ist gescheitert (einmal gemeldet, bis es wieder klappt)
    save_failed: AtomicBool,
    /// Ein Schreibvorgang des Chats nach dem anderen (Auftrag und Nebenfrage speichern gleichzeitig)
    save_lock: Mutex<()>,
}

fn blank() -> Value {
    let now = now_ms();
    json!({ "id": new_id(), "title": "", "created": now, "updated": now, "messages": [] })
}

impl Agent {
    pub fn new(opts: AgentOptions) -> Arc<Agent> {
        Arc::new(Agent {
            events: Emitter::new(),
            turn_started: AtomicI64::new(0),
            o: opts,
            inner: Mutex::new(Inner {
                chat: blank(),
                allowed: HashSet::new(),
                reviewed_safe: HashSet::new(),
                made_this_turn: HashSet::new(),
                auto_effort: String::new(),
                text_mode_for: HashSet::new(),
                no_images_for: HashSet::new(),
                tight_for: HashSet::new(),
                last_skipped: String::new(),
                turn_sources: None,
                sized: None,
                summaries: HashMap::new(),
            }),
            running: AtomicBool::new(false),
            undoing: AtomicBool::new(false),
            controller: Mutex::new(None),
            approvals: Mutex::new(HashMap::new()),
            approval_text: Mutex::new(HashMap::new()),
            questions: Mutex::new(HashMap::new()),
            open_events: Mutex::new(Vec::new()),
            compacting: AtomicBool::new(false),
            save_failed: AtomicBool::new(false),
            save_lock: Mutex::new(()),
        })
    }

    // ---------- Abfragen ----------

    pub fn running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    pub fn undoing(&self) -> bool {
        self.undoing.load(Ordering::SeqCst)
    }

    pub fn chat_id(&self) -> String {
        js_string(&self.inner.lock().chat["id"])
    }

    pub fn chat_title(&self) -> String {
        js_string(&self.inner.lock().chat["title"])
    }

    pub fn get_folder(&self) -> String {
        (self.o.get_folder)()
    }

    pub fn pending_questions(&self) -> usize {
        self.questions.lock().len()
    }

    pub fn pending_approvals(&self) -> usize {
        self.approvals.lock().len()
    }

    /// Ereignisse der offenen Fragen und Rückfragen in der Reihenfolge, in der sie gestellt wurden
    pub fn pending_events(&self) -> Value {
        Value::Array(self.open_events.lock().iter().map(|(_, ev)| ev.clone()).collect())
    }

    fn open_event(&self, id: &str, ev: &Value) {
        self.open_events.lock().push((id.to_string(), ev.clone()));
    }

    fn close_event(&self, id: &str) {
        self.open_events.lock().retain(|(k, _)| k != id);
    }

    /// Offene Fragen und Rückfragen (für die Oberfläche nach dem Neuladen): `{ questions: [id], approvals: [id] }`
    pub fn pending(&self) -> Value {
        json!({ "questions": self.questions.lock().keys().collect::<Vec<_>>(), "approvals": self.approvals.lock().keys().collect::<Vec<_>>() })
    }

    /// Kopie der Nachrichten des offenen Chats (`chat.messages`)
    pub fn messages(&self) -> Vec<Value> {
        arr(&self.inner.lock().chat["messages"]).to_vec()
    }

    /// Kopie des offenen Chats, so wie er auf der Platte steht
    pub fn chat(&self) -> Value {
        self.inner.lock().chat.clone()
    }

    /// Zugriff auf den offenen Chat (JS: `agent.chat`), z. B. für Tests; ändert nichts auf der Platte
    pub fn with_chat<R>(&self, f: impl FnOnce(&mut Value) -> R) -> R {
        f(&mut self.inner.lock().chat)
    }

    /// Anzahl der Arten, die die Person mit «Immer in diesem Chat» erlaubt hat
    pub fn allowed_count(&self) -> usize {
        self.inner.lock().allowed.len()
    }

    /// Bilder und Werkzeugliste lehnt ein bestimmtes Modell ab, nicht ASK als Ganzes: Nach einem Wechsel
    /// des Modells gilt wieder der normale Weg.
    pub fn text_mode(&self) -> bool {
        let model = (self.o.get_model)();
        self.inner.lock().text_mode_for.contains(&model)
    }

    pub fn set_text_mode(&self, on: bool) {
        let model = (self.o.get_model)();
        let mut g = self.inner.lock();
        if on { g.text_mode_for.insert(model); } else { g.text_mode_for.remove(&model); }
    }

    pub fn no_images(&self) -> bool {
        let model = (self.o.get_model)();
        self.inner.lock().no_images_for.contains(&model)
    }

    pub fn set_no_images(&self, on: bool) {
        let model = (self.o.get_model)();
        let mut g = self.inner.lock();
        if on { g.no_images_for.insert(model); } else { g.no_images_for.remove(&model); }
    }

    /// Darf das Modell Bilder bekommen? (ASK meldet Bildfähigkeit nicht ab und hat keine abgelehnt)
    fn can_see(&self) -> bool {
        self.o.get_image_url.is_some() && !self.no_images() && (self.o.get_vision)() != Some(false)
    }

    fn emit_changed(&self) {
        self.events.emit("changed", &Value::Null);
    }

    /// Hinweise und Fehler eines Auftrags bleiben im Chat stehen (das Modell bekommt sie nicht): Wer den
    /// Chat später öffnet, sieht, warum ein Auftrag aufgehört hat.
    fn event(&self, payload: Value) {
        let kind = payload["type"].as_str().unwrap_or("");
        if self.running() && (kind == "notice" || kind == "error") && truthy(&payload["text"]) {
            self.push(json!({ "role": "note", "kind": kind, "content": js_string(&payload["text"]), "time": now_ms() }));
        }
        self.events.emit("event", &payload);
    }

    fn notice(&self, text: impl Into<String>) {
        self.event(json!({ "type": "notice", "text": text.into() }));
    }

    fn push(&self, message: Value) {
        if let Some(list) = self.inner.lock().chat["messages"].as_array_mut() {
            list.push(message);
        }
    }

    pub fn state(&self) -> Value {
        let context = self.context();
        let g = self.inner.lock();
        let model = (self.o.get_model)();
        json!({
            "id": g.chat["id"], "title": g.chat["title"], "running": self.running(), "undoing": self.undoing(),
            "textMode": g.text_mode_for.contains(&model),
            "goal": if truthy(&g.chat["goal"]) { g.chat["goal"].clone() } else { Value::Null },
            "pinned": truthy(&g.chat["pinned"]), "context": context,
            // Beginn des laufenden Auftrags: Wer in einen Chat wechselt, der schon arbeitet, sieht die richtige Dauer
            "started": if self.running() { self.turn_started.load(Ordering::SeqCst) } else { 0 },
        })
    }

    /// Wie voll ist das Kontextfenster? Aus den Tokenzahlen der letzten Antwort (Eingabe und Ausgabe); liefert ASK
    /// keine, grob aus der Länge des Verlaufs geschätzt. limit 0: Die App kennt die Grösse des Fensters nicht.
    pub fn context(&self) -> Value {
        let limit = (self.o.get_context_limit)(&(self.o.get_model)());
        let mut g = self.inner.lock();
        let cut = summary_index(arr(&g.chat["messages"]));
        // Seit der letzten Antwort zusammengefasst? Dann stimmen deren Tokenzahlen nicht mehr: schätzen
        let summarized_at = cut.and_then(|i| g.chat["messages"][i]["summaryBefore"]["time"].as_i64()).unwrap_or(0);
        let last = arr(&g.chat["messages"]).iter().rev().find(|m| m["role"] == "assistant" && truthy(&m["stats"])).cloned();
        if let Some(last) = &last {
            if truthy(&last["stats"]["promptTokens"]) && last["time"].as_i64().unwrap_or(0) >= summarized_at {
                let tokens = last["stats"]["promptTokens"].as_i64().unwrap_or(0) + last["stats"]["tokens"].as_i64().unwrap_or(0);
                return json!({ "tokens": tokens, "estimated": false, "limit": limit });
            }
        }
        if last.is_none() {
            return json!({ "tokens": 0, "estimated": true, "limit": limit });
        }
        let id = js_string(&g.chat["id"]);
        let n = arr(&g.chat["messages"]).len();
        let from = cut.unwrap_or(0);
        if !matches!(&g.sized, Some((c, k, f, _)) if *c == id && *k == n && *f == from) {
            let summary = cut.map(|i| js_string(&g.chat["messages"][i]["summaryBefore"]["content"]).encode_utf16().count()).unwrap_or(0);
            let chars: usize = arr(&g.chat["messages"])[from..]
                .iter()
                .filter(|m| m["role"] != "note")
                .map(|m| {
                    let content = if truthy(&m["content"]) { js_string(&m["content"]).encode_utf16().count() } else { 0 };
                    content + arr(&m["tool_calls"]).iter().map(|t| if truthy(&t["function"]["arguments"]) { js_string(&t["function"]["arguments"]).encode_utf16().count() } else { 0 }).sum::<usize>()
                })
                .sum();
            g.sized = Some((id, n, from, ((chars + summary) as f64 / 3.6).round() as i64));
        }
        let tokens = g.sized.as_ref().map(|s| s.3).unwrap_or(0);
        json!({ "tokens": tokens, "estimated": true, "limit": limit })
    }

    /// Ältere Aufträge zusammenfassen (E78). `keep`: so viele der letzten Aufträge bleiben wörtlich. true, wenn
    /// zusammengefasst wurde; false, wenn es nichts zusammenzufassen gab oder die Anfrage scheiterte (dann bleibt
    /// der Chat, wie er war).
    pub async fn compact(&self, keep: usize, signal: Option<&Signal>) -> bool {
        if self.compacting.swap(true, Ordering::SeqCst) {
            return false; // läuft schon
        }
        let done = self.compact_inner(keep, signal).await;
        self.compacting.store(false, Ordering::SeqCst);
        done
    }

    async fn compact_inner(&self, keep: usize, signal: Option<&Signal>) -> bool {
        let model = (self.o.get_model)();
        let limit = (self.o.get_context_limit)(&model);
        let (chat_id, text, marker) = {
            let g = self.inner.lock();
            let msgs = arr(&g.chat["messages"]);
            let Some((start, cut)) = compact_range(msgs, keep) else { return false };
            // Höchstens ein Drittel des Fensters für den alten Verlauf (3.6 Zeichen pro Token), sonst passt die Anfrage selbst nicht
            let max = if limit > 0 { ((limit as f64) * 3.6 / 3.0) as usize } else { COMPACT_INPUT_MAX }.clamp(6_000, COMPACT_INPUT_MAX);
            // Woran die Stelle wiederzuerkennen ist, falls sich der Chat während der Anfrage ändert
            (js_string(&g.chat["id"]), compact_text(&msgs[start..cut], max), (cut, msgs[cut]["time"].clone(), js_string(&msgs[cut]["content"])))
        };
        if model.is_empty() || text.trim().is_empty() {
            return false;
        }
        let req = ChatRequest {
            model,
            messages: vec![json!({ "role": "system", "content": COMPACT_SYSTEM }), json!({ "role": "user", "content": format!("Der bisherige Chat:\n\n{text}") })],
            tools: None,
            web_search: false,
            thinking: "tief".into(),
            signal: signal.cloned(),
            on_delta: None,
        };
        let Ok(Ok(done)) = tokio::time::timeout(COMPACT_TIMEOUT, self.o.ask.chat(req)).await else { return false };
        let summary = re!(r"(?s)<think>.*?</think>").replace_all(&done.content, "").trim().to_string();
        if summary.chars().count() < 20 || signal.is_some_and(Signal::is_cancelled) {
            return false;
        }
        {
            let mut g = self.inner.lock();
            // Anderer Chat geöffnet oder die Eingabe zurückgenommen, während die Anfrage lief: nichts ändern
            let (cut, time, content) = &marker;
            let same = js_string(&g.chat["id"]) == chat_id && g.chat["messages"].get(*cut).is_some_and(|m| turn_start(m) && m["time"] == *time && js_string(&m["content"]) == *content);
            if !same {
                return false;
            }
            g.chat["messages"][*cut]["summaryBefore"] = json!({ "content": take_chars(&summary, 6000), "time": now_ms() });
            g.tight_for.remove(&chat_id); // der Verlauf ist wieder kurz: nichts mehr stark kürzen
            g.sized = None;
        }
        self.save();
        // Nicht als Notiz speichern: Der Hinweis entsteht beim Laden aus `summaryBefore` (transcript)
        self.events.emit("event", &json!({ "type": "notice", "text": COMPACT_NOTICE }));
        self.emit_changed();
        true
    }

    /// Befehl /zusammenfassen: jetzt zusammenfassen, auch wenn noch Platz ist. Nicht während eines Auftrags.
    pub async fn compact_now(&self) -> Value {
        if self.running() || self.undoing() {
            return json!({ "ok": false, "reason": "Das geht erst, wenn der Auftrag fertig ist." });
        }
        let has = compact_range(arr(&self.inner.lock().chat["messages"]), 1).is_some();
        if !has {
            return json!({ "ok": false, "reason": "Der Chat ist noch kurz: Es gibt nichts zusammenzufassen." });
        }
        if self.compact(1, None).await {
            json!({ "ok": true })
        } else {
            json!({ "ok": false, "reason": "Das Zusammenfassen hat nicht geklappt. Der Chat bleibt, wie er war." })
        }
    }

    /// /goal: Ziel des Chats. Das Modell arbeitet weiter, bis es goal_done aufruft (siehe `run_turn`).
    pub fn set_goal(&self, text: &str) {
        let text = text.trim();
        self.inner.lock().chat["goal"] = if text.is_empty() { Value::Null } else { json!({ "text": text, "done": false, "since": now_ms() }) };
        self.save();
        self.emit_changed();
    }

    fn chat_dir_of(folder: &str) -> PathBuf {
        if folder.is_empty() {
            return PathBuf::new();
        }
        CHAT_DIR.iter().fold(PathBuf::from(folder), |p, s| p.join(s))
    }

    fn chat_dir(&self) -> PathBuf {
        Self::chat_dir_of(&(self.o.get_folder)())
    }

    fn save(&self) {
        let dir = self.chat_dir();
        if dir.as_os_str().is_empty() {
            return;
        }
        // Stand lesen und schreiben ohne Lücke: Sonst konnte ein älterer Stand zuletzt auf der Platte landen
        let _writing = self.save_lock.lock();
        let (id, text) = {
            let mut g = self.inner.lock();
            if arr(&g.chat["messages"]).is_empty() {
                return;
            }
            g.chat["updated"] = json!(now_ms());
            (js_string(&g.chat["id"]), g.chat.to_string())
        };
        let file = dir.join(format!("{id}.json"));
        // Erst daneben schreiben, dann umbenennen: Ein Absturz mitten im Schreiben zerstört den Chat nicht
        let tmp = dir.join(format!("{id}.json.tmp"));
        let written = std::fs::create_dir_all(&dir).is_ok()
            && (std::fs::write(&tmp, &text).and_then(|_| std::fs::rename(&tmp, &file)).is_ok() || {
                let _ = std::fs::remove_file(&tmp);
                std::fs::write(&file, &text).is_ok()
            });
        // Ordner nicht beschreibbar (gesperrt, Datenträger voll): Der Chat bleibt nur im Speicher. Das einmal sagen,
        // sonst ist er nach dem Schliessen der App ohne Vorwarnung weg.
        if written {
            self.save_failed.store(false, Ordering::SeqCst);
        } else if !self.save_failed.swap(true, Ordering::SeqCst) {
            self.events.emit("event", &json!({ "type": "notice", "text": "Der Chat lässt sich gerade nicht speichern (Ordner gesperrt oder Datenträger voll?). Er bleibt offen, ist aber nach dem Schliessen von ASK weg, wenn das so bleibt." }));
        }
    }

    /// Chats des aktuellen Ordners, neueste zuerst (`list()` in JS ohne Argument)
    pub fn list_current(&self) -> Value {
        self.list(&(self.o.get_folder)())
    }

    /// Chats eines Ordners, neueste zuerst, angeheftete zuerst. Zusammenfassungen werden pro Datei
    /// zwischengespeichert, solange sie sich nicht ändert; Chats mit Spielen können gross sein.
    pub fn list(&self, folder: &str) -> Value {
        let dir = Self::chat_dir_of(folder);
        let Ok(entries) = std::fs::read_dir(&dir) else { return json!([]) };
        let mut out: Vec<Value> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let file = dir.join(&name);
            let Ok(mtime) = std::fs::metadata(&file).and_then(|m| m.modified()) else { continue };
            if let Some((t, summary)) = self.inner.lock().summaries.get(&file) {
                if *t == mtime {
                    out.push(summary.clone());
                    continue;
                }
            }
            let Some(c) = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { continue };
            let Some(messages) = c["messages"].as_array() else { continue };
            let summary = json!({
                "id": c["id"],
                "title": if truthy(&c["title"]) { c["title"].clone() } else { json!("(ohne Titel)") },
                "updated": c["updated"], "pinned": truthy(&c["pinned"]),
                "count": messages.iter().filter(|m| m["role"] == "user" && !truthy(&m["auto"])).count(),
            });
            self.inner.lock().summaries.insert(file, (mtime, summary.clone()));
            out.push(summary);
        }
        out.sort_by(|a, b| {
            let (pa, pb) = (a["pinned"] == true, b["pinned"] == true);
            pb.cmp(&pa).then_with(|| b["updated"].as_f64().unwrap_or(0.0).partial_cmp(&a["updated"].as_f64().unwrap_or(0.0)).unwrap_or(std::cmp::Ordering::Equal))
        });
        Value::Array(out)
    }

    /// Titel oder Anheften eines gespeicherten Chats ändern (auch in einem anderen Ordner als dem offenen).
    fn patch(&self, folder: &str, id: &str, change: impl FnOnce(&mut Value)) -> bool {
        if !valid_id(id) || folder.is_empty() {
            return false;
        }
        let current = id == self.chat_id() && same_path(folder, &(self.o.get_folder)());
        if current {
            change(&mut self.inner.lock().chat);
            self.save();
        } else if !Self::patch_file(folder, id, change) {
            return false;
        }
        self.emit_changed();
        true
    }

    /// Einen gespeicherten Chat in der Datei ändern (gibt es sie nicht mehr: false)
    fn patch_file(folder: &str, id: &str, change: impl FnOnce(&mut Value)) -> bool {
        if !valid_id(id) || folder.is_empty() {
            return false;
        }
        let file = Self::chat_dir_of(folder).join(format!("{id}.json"));
        (|| -> Option<()> {
            let mut chat: Value = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
            change(&mut chat);
            let tmp = PathBuf::from(format!("{}.tmp", file.display()));
            std::fs::write(&tmp, chat.to_string()).ok()?;
            std::fs::rename(&tmp, &file).ok()
        })()
        .is_some()
    }

    pub fn rename(&self, folder: &str, id: &str, title: &str) -> bool {
        let clean = take_chars(re!(r"\s+").replace_all(title, " ").trim(), 80);
        !clean.is_empty() && self.patch(folder, id, |chat| {
            chat["title"] = json!(clean);
            if let Some(o) = chat.as_object_mut() { o.remove("titleAuto"); } // selbst benannt: ASK ändert ihn nicht mehr
        })
    }

    pub fn pin(&self, folder: &str, id: &str, on: bool) -> bool {
        self.patch(folder, id, |chat| {
            if on {
                chat["pinned"] = json!(true);
            } else if let Some(o) = chat.as_object_mut() {
                o.shift_remove("pinned");
            }
        })
    }

    /// Letzte Eingabe zurücknehmen, um sie zu ändern oder neu zu schicken: Der Chat endet wieder davor, und der
    /// Ordner geht (nach Rückfrage) auf den Stand vor diesem Auftrag. Liefert Text und Anhänge für das Eingabefeld.
    pub async fn rewind(&self) -> Value {
        if self.running() {
            return json!({ "ok": false, "reason": "Es läuft gerade ein Auftrag." });
        }
        // Während Rückfrage und Zurücksetzen startet kein Auftrag (send prüft undoing) und kein zweites Rückgängig
        if self.undoing.swap(true, Ordering::SeqCst) {
            return json!({ "ok": false, "busy": true, "reason": "Es wird schon zurückgesetzt." });
        }
        self.emit_changed();
        let result = match self.rewind_now().await {
            Ok(v) => v,
            Err(err) => json!({ "ok": false, "reason": format!("Zurücknehmen nicht möglich: {err}") }),
        };
        self.undoing.store(false, Ordering::SeqCst);
        self.emit_changed();
        result
    }

    fn confirm_fn(&self) -> ConfirmFn {
        self.o.confirm.clone().unwrap_or_else(|| fixed_confirm(Confirmed::default()))
    }

    async fn rewind_now(&self) -> Result<Value> {
        let (index, message) = {
            let g = self.inner.lock();
            let list = arr(&g.chat["messages"]);
            match list.iter().rposition(|m| m["role"] == "user" && !truthy(&m["auto"])) {
                Some(i) => (i, list[i].clone()),
                None => return Ok(json!({ "ok": false, "reason": "In diesem Chat gibt es keine Eingabe." })),
            }
        };
        let snapshots = (self.o.get_snapshots)();
        let mut failed: Vec<String> = Vec::new();
        if let (true, Some(snapshots), false) = (truthy(&message["snapshot"]), &snapshots, truthy(&message["undone"])) {
            let snap = js_string(&message["snapshot"]);
            let d = snapshots.diff(&snap).await.ok(); // Sicherung gibt es nicht mehr: nur den Chat zurücknehmen
            let total = d.as_ref().map(|d| d.changed.len() + d.added.len() + d.removed.len()).unwrap_or(0);
            if let (Some(d), true) = (d, total > 0) {
                let names: Vec<&String> = d.changed.iter().chain(&d.added).chain(&d.removed).collect();
                let answer = (self.confirm_fn())(
                    "Auch die Dateien auf den Stand vor dieser Eingabe zurücksetzen?".to_string(),
                    format!(
                        "Seit dieser Eingabe geändert, angelegt oder gelöscht:\n{}{}\n\nJa: Der Ordner ist danach wie vor der Eingabe. Nein: Die Eingabe wird nicht zurückgenommen.{}",
                        names.iter().take(12).map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n"),
                        if names.len() > 12 { format!("\n  … und {} weitere", names.len() - 12) } else { String::new() },
                        self.sibling_warning()
                    ),
                    ConfirmOptions::default(),
                )
                .await;
                if !answer.ok {
                    return Ok(json!({ "ok": false, "cancelled": true }));
                }
                failed = snapshots.restore(&snap).await?.failed;
            }
        }
        {
            let mut g = self.inner.lock();
            if let Some(list) = g.chat["messages"].as_array_mut() {
                list.truncate(index);
            }
            if arr(&g.chat["messages"]).is_empty() {
                g.chat["title"] = json!(""); // der Titel kam aus dieser Eingabe
            }
            if g.chat["goal"]["done"] == true {
                g.chat["goal"]["done"] = json!(false);
            }
            g.sized = None;
        }
        if !self.messages().is_empty() {
            self.save();
        } else {
            let _ = std::fs::remove_file(self.chat_dir().join(format!("{}.json", self.chat_id())));
        }
        self.emit_changed();
        Ok(json!({ "ok": true, "text": js_string(&message["content"]), "attachments": if message["attachments"].is_array() { message["attachments"].clone() } else { json!([]) }, "failed": failed }))
    }

    /// Neuer Chat oder anderer Chat: Rückfallmodi neu prüfen (Textmodus, keine Bilder)
    pub fn reset_modes(&self) {
        self.set_text_mode(false);
        self.set_no_images(false);
    }

    pub fn new_chat(&self) -> bool {
        if self.running() || self.undoing() {
            return false;
        }
        {
            let mut g = self.inner.lock();
            g.allowed.clear();
            g.reviewed_safe.clear();
        }
        self.reset_modes();
        self.inner.lock().chat = blank();
        self.emit_changed();
        true
    }

    /// Beim Ordnerwechsel: gewünschten Chat (id oder NEW) oder sonst den letzten Chat des Ordners öffnen.
    pub fn open_latest(&self, wanted: Option<&str>) {
        if self.running() || self.undoing() {
            return; // Ordnerwechsel sind während eines Auftrags gesperrt (main.js)
        }
        if let Some(w) = wanted.filter(|w| !w.is_empty() && *w != NEW) {
            if self.open(w) {
                return;
            }
        }
        // Der zuletzt benutzte Chat. Die Liste führt angeheftete zuerst; sonst öffnete jeder Start und jeder Wechsel
        // ins Projekt den angehefteten statt den, an dem man zuletzt war.
        let latest = if wanted == Some(NEW) { None } else {
            self.list_current().as_array().and_then(|l| l.iter().max_by(|a, b| a["updated"].as_f64().unwrap_or(0.0).total_cmp(&b["updated"].as_f64().unwrap_or(0.0))).cloned())
        };
        let opened = latest.as_ref().is_some_and(|l| self.open(&js_string(&l["id"])));
        if !opened {
            self.inner.lock().chat = blank();
            self.emit_changed();
        }
    }

    /// Löscht einen gespeicherten Chat. Ist es der offene, beginnt ein neuer.
    pub fn remove(&self, folder: &str, id: &str) -> bool {
        if !valid_id(id) {
            return false;
        }
        let current = id == self.chat_id() && same_path(folder, &(self.o.get_folder)());
        if current && self.running() {
            return false;
        }
        if std::fs::remove_file(Self::chat_dir_of(folder).join(format!("{id}.json"))).is_err() {
            return false;
        }
        // Skripte und Zwischendateien dieses Chats gehören mit dazu
        for sub in WORK_DIRS {
            let _ = std::fs::remove_dir_all(Path::new(folder).join(".ask").join(sub).join(id));
        }
        if current {
            self.new_chat();
        }
        true
    }

    /// Einen gespeicherten Chat in einen anderen Ordner verschieben (anderes Projekt oder Chats ohne Projekt), mit
    /// seinen Anhängen, Skripten und Zwischendateien. Was zum alten Ordner gehört, bleibt dort: Die Sicherungen für
    /// «Rückgängig» gelten für dessen Dateien, darum verliert der verschobene Chat seine Verweise darauf (sonst setzte
    /// «Rückgängig» den neuen Ordner auf einen Stand des alten zurück).
    pub fn move_chat(from: &str, to: &str, id: &str) -> Result<()> {
        if !valid_id(id) || from.is_empty() || to.is_empty() {
            bail!("Der Chat lässt sich nicht verschieben.");
        }
        if same_path(from, to) {
            bail!("Der Chat liegt schon dort.");
        }
        let source = Self::chat_dir_of(from).join(format!("{id}.json"));
        let target_dir = Self::chat_dir_of(to);
        let target = target_dir.join(format!("{id}.json"));
        if target.exists() {
            bail!("Dort gibt es schon einen Chat mit derselben Kennung.");
        }
        let mut chat: Value = serde_json::from_str(&std::fs::read_to_string(&source).map_err(|_| anyhow::anyhow!("Diesen Chat gibt es nicht mehr."))?)
            .map_err(|_| anyhow::anyhow!("Der Chat lässt sich nicht lesen."))?;
        std::fs::create_dir_all(&target_dir).map_err(|e| anyhow::anyhow!("Der Zielordner ist nicht beschreibbar: {e}"))?;
        // Zuerst die Dateien des Chats, dann der Chat selbst: Scheitert etwas, liegt er noch am alten Ort
        let mut moved: Vec<(PathBuf, PathBuf)> = Vec::new();
        for sub in WORK_DIRS {
            let (a, b) = (Path::new(from).join(".ask").join(sub).join(id), Path::new(to).join(".ask").join(sub).join(id));
            if !a.exists() {
                continue;
            }
            let done = b.parent().map(std::fs::create_dir_all).transpose().is_ok() && !b.exists() && move_dir(&a, &b).is_ok();
            if !done {
                for (x, y) in moved.iter().rev() {
                    let _ = move_dir(y, x); // zurück an den alten Ort
                }
                bail!("Die Dateien des Chats liessen sich nicht verschieben (eine davon ist vielleicht geöffnet).");
            }
            moved.push((a, b));
        }
        if let Some(list) = chat["messages"].as_array_mut() {
            for m in list.iter_mut() {
                if let Some(o) = m.as_object_mut() {
                    o.remove("snapshot");
                    o.remove("changes");
                    o.remove("undone");
                }
                // Bilder, die im alten Ordner bleiben, nicht mehr als Vorschau zeigen (sie fehlten im neuen)
                if let Some(images) = m.get("images").and_then(Value::as_array).cloned() {
                    m["images"] = json!(images.into_iter().filter(|p| p.as_str().is_some_and(|p| Path::new(to).join(p).is_file())).collect::<Vec<_>>());
                }
            }
        }
        chat["updated"] = json!(now_ms());
        let tmp = target_dir.join(format!("{id}.json.tmp"));
        let written = std::fs::write(&tmp, chat.to_string()).and_then(|_| std::fs::rename(&tmp, &target));
        if let Err(e) = written {
            let _ = std::fs::remove_file(&tmp);
            for (x, y) in moved.iter().rev() {
                let _ = move_dir(y, x);
            }
            bail!("Der Chat liess sich nicht speichern: {e}");
        }
        let _ = std::fs::remove_file(&source);
        Ok(())
    }

    /// Ein gespeicherter Chat als Markdown: Eingaben, Antworten und die Arbeitsschritte kurz (zum Abgeben oder Aufbewahren)
    pub fn export_markdown(folder: &str, id: &str, place: &str) -> Option<String> {
        if !valid_id(id) || folder.is_empty() {
            return None;
        }
        let chat: Value = serde_json::from_str(&std::fs::read_to_string(Self::chat_dir_of(folder).join(format!("{id}.json"))).ok()?).ok()?;
        let messages = chat["messages"].as_array()?;
        let when = |m: &Value| m["time"].as_f64().filter(|t| *t > 0.0).map(crate::history::date).filter(|d| !d.is_empty());
        let title = { let t = js_string(&chat["title"]); if t.trim().is_empty() { "Chat".to_string() } else { t } };
        let mut out = format!("# {title}\n\nAus ASK, {place}. Exportiert am {}.\n", crate::history::date(now_ms() as f64));
        // Ergebnis zu jedem Aufruf: Kurztext und ob er geklappt hat
        let results: HashMap<String, (&Value, bool)> = messages.iter().filter(|m| m["role"] == "tool")
            .map(|m| (js_string(&m["tool_call_id"]), (m, m["ok"] != json!(false)))).collect();
        for m in messages {
            match m["role"].as_str().unwrap_or("") {
                "user" if m["note"] == "view" || truthy(&m["auto"]) => {}
                "user" => {
                    out.push_str(&format!("\n## Du{}\n\n{}\n", when(m).map(|d| format!(" · {d}")).unwrap_or_default(), js_string(&m["content"]).trim()));
                    let files: Vec<String> = arr(&m["attachments"]).iter().map(|a| js_string(&a["name"])).filter(|n| !n.is_empty()).collect();
                    if !files.is_empty() {
                        out.push_str(&format!("\nAnhänge: {}\n", files.join(", ")));
                    }
                }
                "assistant" => {
                    let steps: Vec<String> = arr(&m["tool_calls"]).iter().map(|c| {
                        let d = describe_call(c);
                        let (label, detail) = (d.get("label").map(js_string).unwrap_or_default(), d.get("detail").map(js_string).unwrap_or_default());
                        let result = results.get(&js_string(&c["id"])).map(|(r, ok)| {
                            let s = js_string(&r["summary"]);
                            if !ok { format!(" → fehlgeschlagen{}", if s.is_empty() { String::new() } else { format!(" ({s})") }) } else if s.is_empty() { String::new() } else { format!(" → {s}") }
                        }).unwrap_or_default();
                        format!("- {label}{}{result}", if detail.is_empty() { String::new() } else { format!(": `{}`", clip(&detail.replace('`', "'"), 160)) })
                    }).collect();
                    let text = js_string(&m["content"]);
                    if text.trim().is_empty() && steps.is_empty() {
                        continue;
                    }
                    out.push_str("\n## ASK\n");
                    if !text.trim().is_empty() {
                        out.push_str(&format!("\n{}\n", text.trim()));
                    }
                    if !steps.is_empty() {
                        out.push_str(&format!("\n{}\n", steps.join("\n")));
                    }
                }
                "note" if truthy(&m["content"]) => out.push_str(&format!("\n> {}: {}\n", if m["kind"] == "error" { "Fehler" } else { "Hinweis" }, js_string(&m["content"]).replace('\n', " "))),
                _ => {}
            }
        }
        Some(out)
    }

    pub fn open(&self, id: &str) -> bool {
        if self.running() || self.undoing() || !valid_id(id) {
            return false;
        }
        {
            let mut g = self.inner.lock();
            g.allowed.clear();
            g.reviewed_safe.clear();
        }
        self.reset_modes();
        let Some(chat) = std::fs::read_to_string(self.chat_dir().join(format!("{id}.json"))).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(|c| c["messages"].is_array()) else {
            return false;
        };
        self.inner.lock().chat = chat;
        self.emit_changed();
        true
    }

    pub fn stop(&self) {
        if let Some(c) = self.controller.lock().as_ref() {
            c.cancel();
        }
    }

    // ---------- Auftrag ----------

    /// Startet einen Auftrag in einer eigenen Aufgabe und kehrt sofort zurück (main.js wartet nicht darauf).
    /// attachments: bereits in .ask/anhaenge/<Chat>/ gespeicherte Anhänge (siehe attachments.rs);
    /// extra.skill: `{ name, text }` eines mit /name aufgerufenen Skills.
    pub fn send(self: &Arc<Self>, text: &str, attachments: Vec<Value>, extra: Value) {
        if let Some((signal, index)) = self.begin(text, attachments, &extra) {
            let me = self.clone();
            tokio::spawn(async move { me.work(signal, index).await });
        }
    }

    /// Wie `send`, wartet aber auf das Ende des Auftrags (das `await agent.send(...)` aus JS)
    pub async fn run_turn(self: &Arc<Self>, text: &str, attachments: Vec<Value>, extra: Value) {
        if let Some((signal, index)) = self.begin(text, attachments, &extra) {
            self.work(signal, index).await;
        }
    }

    /// Der Teil von `send` bis zum ersten Warten: Eingabe in den Chat, Zustand «läuft». None: nichts zu tun.
    fn begin(&self, text: &str, attachments: Vec<Value>, extra: &Value) -> Option<(Signal, usize)> {
        let text = text.trim().to_string();
        if (text.is_empty() && attachments.is_empty()) || self.undoing() {
            return None;
        }
        if self.running.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst).is_err() {
            return None;
        }
        if self.chat_title().is_empty() {
            let source = if text.is_empty() { attachments.iter().map(|a| js_string(&a["name"])).collect::<Vec<_>>().join(", ") } else { text.clone() };
            let mut g = self.inner.lock();
            g.chat["title"] = json!(take_chars(&re!(r"\s+").replace_all(&source, " "), 80));
            // Vorläufig aus der Eingabe; nach der ersten Antwort schreibt ASK einen kurzen Titel (auto_title)
            g.chat["titleAuto"] = json!(true);
        }
        let mut message = json!({ "role": "user", "content": text, "time": now_ms() });
        if !attachments.is_empty() {
            message["attachments"] = json!(attachments);
        }
        let mentions = mentions_in(&text, &|rel| (self.o.file_exists)(rel) || crate::vorlagen::template_exists(rel));
        if !mentions.is_empty() {
            message["mentions"] = json!(mentions);
        }
        if truthy(&extra["skill"]["text"]) {
            message["skill"] = json!({ "name": js_string(&extra["skill"]["name"]), "text": js_string(&extra["skill"]["text"]) });
        }
        self.push(message);
        let index = self.messages().len() - 1;
        self.event(json!({ "type": "user", "text": text, "attachments": attachments }));
        {
            let mut g = self.inner.lock();
            g.turn_sources = Some(TurnSources::default()); // web_fetch und web_search in diesem Auftrag
            g.made_this_turn = HashSet::new();
            g.auto_effort = String::new(); // «Automatisch»: jeder Auftrag beginnt bei «mittel»
        }
        let signal = Signal::new();
        *self.controller.lock() = Some(signal.clone());
        self.emit_changed();
        let others = (self.o.get_siblings)();
        if !others.is_empty() {
            self.notice(format!(
                "Im selben Projekt arbeitet gerade auch {}. Beide Chats ändern dieselben Dateien, und «Rückgängig» setzt die Änderungen beider zurück.",
                others.iter().map(|t| format!("«{}»", clip(t, 60))).collect::<Vec<_>>().join(", ")
            ));
        }
        Some((signal, index))
    }

    async fn work(self: &Arc<Self>, signal: Signal, index: usize) {
        let snapshots = (self.o.get_snapshots)();
        let mut failed = false;
        // Stand des Ordners vor dem Auftrag, für «Rückgängig»
        if let Some(s) = &snapshots {
            self.backup(s, index).await;
        }
        // Kontext fast voll (bis auf die Reserve): ältere Aufträge zusammenfassen, bevor das Modell den Anfang verliert
        let reserve = (self.o.get_compact_reserve)();
        if reserve > 0 {
            let c = self.context();
            let (tokens, limit) = (c["tokens"].as_i64().unwrap_or(0), c["limit"].as_i64().unwrap_or(0));
            if limit > 0 && tokens >= (limit - reserve.min(limit / 2)).max(1) {
                // Die letzten zwei Aufträge bleiben wörtlich; gibt es erst zwei, nur die neue Eingabe
                let keep = if compact_range(arr(&self.inner.lock().chat["messages"]), 2).is_some() { 2 } else { 1 };
                self.event(json!({ "type": "compacting" }));
                self.compact(keep, Some(&signal)).await;
            }
        }
        let mut idle = 0; // Anstösse ohne Arbeitsschritt hintereinander (Ziel-Modus)
        for step in 0..MAX_STEPS {
            if signal.is_cancelled() {
                break;
            }
            // Websuche nur bei der ersten Anfrage eines Auftrags, sonst sucht ASK bei jedem Werkzeugschritt neu.
            let web_search = step == 0 && (self.o.get_web_search)();
            let done = match self.step(&signal, web_search).await {
                Ok(d) => d,
                Err(err) => {
                    failed = true;
                    if !signal.is_cancelled() && !is_abort(&err) {
                        self.event(json!({ "type": "error", "text": err.to_string() }));
                    }
                    break;
                }
            };
            self.save();
            if done {
                // Ziel-Modus: aufgehört, ohne das Ziel als erreicht zu melden? Dann anstossen, höchstens
                // zweimal ohne Arbeitsschritt dazwischen; danach wartet das Modell offenbar auf die Person.
                let goal = self.inner.lock().chat["goal"].clone();
                // Im Planmodus fehlt goal_done: dort wartet der Plan auf die Person, nicht auf einen Anstoss
                if !truthy(&goal) || truthy(&goal["done"]) || signal.is_cancelled() || (!self.o.free && (self.o.get_mode)() == "plan") {
                    break;
                }
                if idle >= GOAL_IDLE_LIMIT {
                    self.notice("Das Modell arbeitet nicht weiter am Ziel und wartet offenbar auf dich. Antworte oder schreib «weiter».");
                    break;
                }
                idle += 1;
                self.push(json!({ "role": "user", "content": GOAL_NUDGE, "auto": true, "time": now_ms() }));
                self.event(json!({ "type": "user", "text": GOAL_NUDGE, "auto": true }));
                continue;
            }
            idle = 0;
            let reason = loop_reason(&self.messages());
            if !reason.is_empty() {
                self.notice(format!("Angehalten: {reason} Schreib, wie es weitergehen soll, oder «weiter»."));
                break;
            }
            if step == MAX_STEPS - 1 {
                self.notice(format!("Nach {MAX_STEPS} Schritten angehalten. Schreib «weiter», um fortzufahren."));
            }
        }
        if signal.is_cancelled() {
            self.notice("Gestoppt.");
        }
        let has_snapshot = truthy(&self.inner.lock().chat["messages"][index]["snapshot"]);
        if let (Some(s), true) = (&snapshots, has_snapshot) {
            self.record_changes(s, index).await;
        }
        // Zuerst das Signal dieses Auftrags weg, dann «arbeitet» aus: Sonst könnte ein Auftrag, der genau dazwischen
        // beginnt, sein eigenes Signal verlieren, und «Stoppen» täte nichts
        *self.controller.lock() = None;
        self.running.store(false, Ordering::SeqCst);
        self.save();
        self.event(json!({ "type": "done", "stopped": signal.is_cancelled(), "failed": failed })); // main.js: danach im Hintergrund merken (memory.rs)
        self.emit_changed();
        let (title_auto, turns) = {
            let g = self.inner.lock();
            let n = g.chat["messages"].as_array().map(|m| m.iter().filter(|x| x["role"] == "user" && !truthy(&x["auto"])).count()).unwrap_or(0);
            (g.chat["titleAuto"] == true, n)
        };
        // Höchstens in den ersten drei Aufträgen; danach bleibt der vorläufige Titel, falls keiner gepasst hat
        if self.o.auto_title && title_auto && turns <= 3 && !signal.is_cancelled() {
            let me = self.clone();
            tokio::spawn(async move { me.auto_title().await });
        }
    }

    /// Titel des Chats von ASK statt der ersten Eingabe: nach der ersten Antwort, im Hintergrund, mit dem Modell
    /// der Hilfsagenten (sonst dem des Chats). Ein Titel, den die Person selbst gesetzt hat, bleibt.
    async fn auto_title(self: Arc<Self>) {
        let (id, folder, request, answer) = {
            let g = self.inner.lock();
            let msgs = g.chat["messages"].as_array().cloned().unwrap_or_default();
            // Die ersten Eingaben (höchstens drei): Folgt auf eine Rückfrage von ASK die Antwort der Person, gehört sie dazu
            let request = msgs.iter().filter(|m| m["role"] == "user" && !truthy(&m["auto"])).take(3)
                .map(|m| js_string(&m["content"])).filter(|t| !t.trim().is_empty()).collect::<Vec<_>>().join("\n---\n");
            let answer = msgs.iter().rev().find(|m| m["role"] == "assistant" && truthy(&m["content"])).map(|m| js_string(&m["content"])).unwrap_or_default();
            (js_string(&g.chat["id"]), (self.o.get_folder)(), request, answer)
        };
        if request.trim().is_empty() {
            return;
        }
        let helper = (self.o.get_helper_model)();
        let model = if helper.is_empty() { (self.o.get_model)() } else { helper };
        if model.is_empty() {
            return;
        }
        let prompt = format!("Eingaben der Person:\n{}\n\nAntwort von ASK (Anfang, nur als Hilfe):\n{}", take_chars(&request, 1500), take_chars(&answer, 600));
        let req = ChatRequest {
            model,
            messages: vec![
                json!({ "role": "system", "content": "Du gibst Chats einen Titel. Der Titel fasst zusammen, was die Person will, nicht die Antwort. Antworte nur mit dem Titel: 2 bis 6 Wörter auf Deutsch (Schweizer Rechtschreibung, ss statt ß), ohne Anführungszeichen, ohne Punkt am Ende. Nie eine Frage, nie eine Rückfrage, auch wenn der Auftrag unklar ist; dann nenne das Thema, so gut es geht (z. B. «App-Idee besprechen»)." }),
                json!({ "role": "user", "content": prompt }),
            ],
            tools: None,
            web_search: false,
            thinking: "standard".into(),
            signal: None,
            on_delta: None,
        };
        let Ok(Ok(done)) = tokio::time::timeout(Duration::from_secs(45), self.o.ask.chat(req)).await else { return };
        let title = clean_title(&done.content);
        if title.is_empty() {
            return;
        }
        let change = |chat: &mut Value| {
            if chat["titleAuto"] == true {
                chat["title"] = json!(title);
                if let Some(o) = chat.as_object_mut() { o.remove("titleAuto"); }
            }
        };
        // Immer die Datei ändern, nie den ganzen Chat aus dem Speicher schreiben: Bis der Titel da ist (bis 45 s),
        // kann die Person den Chat in einem anderen Agenten weitergeführt oder gelöscht haben, und ein Speichern von
        // hier überschriebe das mit dem alten Stand.
        if !Self::patch_file(&folder, &id, change) {
            return;
        }
        if self.chat_id() == id && same_path(&folder, &(self.o.get_folder)()) {
            change(&mut self.inner.lock().chat);
        }
        self.emit_changed();
    }

    /// Eine Antwort des Modells und die verlangten Werkzeuge. true, wenn das Modell fertig ist.
    async fn step(self: &Arc<Self>, signal: &Signal, web_search: bool) -> Result<bool> {
        let model = (self.o.get_model)();
        if model.is_empty() {
            bail!("Kein Modell gewählt.");
        }
        let goal = { let g = self.inner.lock().chat["goal"].clone(); if truthy(&g) && !truthy(&g["done"]) { Some(g) } else { None } };
        let memory = (self.o.get_chat_memory)();
        let flux = self.o.images.as_ref().is_some_and(|i| i.available());
        let remember = self.o.knowledge.as_ref().and_then(|k| k.memory_on.as_ref()).is_none_or(|f| f());
        // Quellen (E84): Werkzeuge nur mit Unterlagen; «Nur aus Quellen» ohne Internet
        let (has_sources, strict) = (self.o.get_sources)();
        let offered = |name: &str| {
            (memory || !name.starts_with("chat_"))
                && (flux || name != "bild_erstellen")
                && (remember || !MEMORY_TOOLS.contains(&name))
                && (has_sources || !SOURCE_TOOLS.contains(&name))
                && !(strict && matches!(name, "web_search" | "web_fetch"))
        };
        let free = self.o.free;
        let plan = !free && (self.o.get_mode)() == "plan"; // ohne Projekt gibt es nichts zu planen
        let chat_id = self.chat_id();
        let name_of = |s: &Value| s["function"]["name"].as_str().unwrap_or("").to_string();
        let mut specs: Vec<Value> = tool_specs(&(self.o.get_command_hint)(&chat_id)).into_iter().filter(|s| offered(&name_of(s))).collect();
        specs.push(question_tool());
        specs.push(helper_tool());
        if goal.is_some() {
            specs.push(goal_tool());
        }
        if (self.o.get_thinking)() == "standard" {
            specs.push(effort_tool());
        }
        if plan {
            specs.push(plan_tool());
            specs.retain(|s| PLAN_ALLOWED.contains(&name_of(s).as_str()));
        }
        if free {
            specs.retain(|s| FREE_ALLOWED.contains(&name_of(s).as_str()));
        }
        let sent_images = AtomicBool::new(false);
        let request = |this: Arc<Agent>| {
            let (specs, goal, model, sent_images) = (&specs, &goal, &model, &sent_images);
            async move { this.request(model, specs, goal.as_ref(), plan, web_search, signal, sent_images).await }
        };
        // Jede Abhilfe höchstens einmal pro Schritt, dann nochmals anfragen: stark kürzen, zusammenfassen (E78),
        // ohne Bilder, Werkzeuge im Text. Früher galt nur die erste; scheiterte die Anfrage danach nochmals (zu lang
        // auch gekürzt), stand die rohe Fehlermeldung von ASK im Chat.
        const GIVE_UP: &str = "Der Chat ist zu lang für das Modell, auch gekürzt und zusammengefasst. Beginne einen neuen Chat (/neu); mit dem Schalter «Chats» kann das Modell diesen hier nachlesen.";
        let (mut compacted, mut no_images, mut as_text) = (false, false, false);
        let mut completion = loop {
            let err = match request(self.clone()).await {
                Ok(c) => break c,
                Err(err) => err,
            };
            self.keep_partial(&err);
            let status = status_of(&err).unwrap_or(0);
            let message = err.to_string();
            let too_long = status >= 400 && TOO_LONG.is_match(&message);
            let limit: i64 = if too_long {
                re!(r"(?i)(?:context length|context window|maximum)[^\d]{0,40}(\d{4,8})").captures(&message).and_then(|c| c[1].parse().ok()).unwrap_or(0)
            } else {
                0
            };
            if limit != 0 {
                (self.o.on_context_limit)(&model, limit); // für die Anzeige, wie voll der Verlauf ist
            }
            let tight = self.inner.lock().tight_for.contains(&chat_id);
            if too_long && !tight {
                // Der Verlauf ist zu lang für das Modell: ältere Schritte stärker kürzen und nochmals
                self.inner.lock().tight_for.insert(chat_id.clone());
                self.notice("Der Chat ist für das Modell zu lang geworden. Ältere Arbeitsschritte gehen ab jetzt stark gekürzt an das Modell.");
            } else if too_long && !compacted {
                // Auch gekürzt zu lang: ältere Aufträge zusammenfassen und nochmals. Hilft auch das nicht (der
                // laufende Auftrag allein ist zu lang), bleibt nur ein neuer Chat.
                compacted = true;
                self.event(json!({ "type": "compacting" }));
                if !self.compact(1, Some(signal)).await {
                    if signal.is_cancelled() {
                        return Err(err);
                    }
                    bail!(GIVE_UP);
                }
            } else if too_long {
                bail!(GIVE_UP);
            } else if !no_images && sent_images.load(Ordering::SeqCst) && status >= 400 && re!(r"(?i)image|vision|multimodal|modalit").is_match(&message) {
                // Nimmt das Modell keine Bilder an, gehen sie künftig nur als Pfad mit.
                no_images = true;
                self.set_no_images(true);
                self.notice("Das Modell in ASK nimmt keine Bilder an. Die Bilder liegen als Datei bei den Anhängen, das Modell kann sie nur so bearbeiten, nicht ansehen.");
            } else if !as_text && !self.text_mode() && status == 400 && re!(r"(?i)tool").is_match(&message) {
                // Manche Server lehnen «tools» ab: dann Werkzeuge im Text beschreiben.
                as_text = true;
                self.set_text_mode(true);
                self.notice("ASK nimmt keine Werkzeugliste an, die App beschreibt die Werkzeuge im Text.");
            } else {
                return Err(err);
            }
        };

        let stats = serde_json::to_value(completion.stats())?;
        let mut content = std::mem::take(&mut completion.content);
        let mut calls = std::mem::take(&mut completion.calls);
        let mut text_mode = false;
        if calls.is_empty() && content.contains("<tool_call>") {
            let (text, parsed) = parse_text_tool_calls(&content);
            if !parsed.is_empty() {
                calls = parsed;
                text_mode = true;
                self.set_text_mode(true);
                let raw = std::mem::replace(&mut content, text);
                completion.content = raw;
            }
        }
        let tool_calls: Vec<Value> = calls
            .iter()
            .map(|c| {
                let id = if c.id.is_empty() { format!("call_{}", hex::encode(rand::random::<[u8; 6]>())) } else { c.id.clone() };
                json!({ "id": id, "type": "function", "function": { "name": c.name, "arguments": clean_args(&Value::String(c.arguments.clone())) } })
            })
            .collect();
        let mut message = json!({ "role": "assistant", "content": content, "time": now_ms(), "stats": stats });
        // Quellen unter der Schlussantwort: Websuche von ASK und die Seiten, die das Modell mit web_fetch
        // gelesen hat (ohne gelesene Seiten die ersten Treffer von web_search)
        let mut sources: Vec<Source> = completion.sources.clone();
        if tool_calls.is_empty() {
            // Verweise auf Unterlagen in der Antwort zuerst (E84)
            sources.splice(0..0, crate::sources::cited(&content));
            let web: Option<Vec<Source>> = self.inner.lock().turn_sources.as_ref().map(|t| if t.read.is_empty() { t.found.iter().take(5).cloned().collect() } else { t.read.clone() });
            if let Some(web) = web {
                let mut seen: Vec<Source> = Vec::new();
                for s in sources.into_iter().chain(web) {
                    if !s.url.is_empty() && !seen.iter().any(|t| t.url == s.url) {
                        seen.push(s);
                    }
                }
                seen.truncate(10);
                sources = seen;
            }
        }
        if !sources.is_empty() {
            message["sources"] = json!(sources);
        }
        if !completion.reasoning.is_empty() {
            message["reasoning"] = json!(completion.reasoning);
        }
        if !tool_calls.is_empty() {
            message["tool_calls"] = json!(tool_calls);
        }
        if text_mode {
            message["textMode"] = json!(true);
            message["raw"] = json!(completion.content);
        }
        self.push(message.clone());
        let call_infos: Vec<Value> = tool_calls.iter().map(|c| {
            let mut o = Map::new();
            o.insert("id".into(), c["id"].clone());
            o.insert("name".into(), c["function"]["name"].clone());
            o.extend(describe_call(c));
            Value::Object(o)
        }).collect();
        self.event(json!({ "type": "assistant-end", "content": content, "stats": message["stats"], "sources": message.get("sources").cloned().unwrap_or(json!([])), "calls": call_infos }));
        if completion.incomplete {
            self.notice("Die Antwort von ASK endete ohne Abschluss; sie ist vielleicht unvollständig. Schreib «weiter», falls etwas fehlt.");
        }
        if completion.finish == "length" {
            self.notice("ASK hat die Antwort wegen der Längengrenze abgeschnitten. Schreib «weiter», oder lass grosse Dateien in mehreren Teilen schreiben.");
        }
        if tool_calls.is_empty() {
            if content.trim().is_empty() {
                self.notice("Das Modell hat keine Antwort geschrieben.");
            }
            return Ok(true);
        }

        let mut seen: Vec<(String, Vec<String>)> = Vec::new(); // Bilder aus view_file und browser, gehen danach als Nachricht an das Modell
        let can_see = self.can_see();
        let mut planned = false;
        let mut helper_runs: HashMap<String, (Arc<HelperBatch>, usize)> = HashMap::new(); // Hilfsagenten dieses Schritts: Kennung → Ergebnis
        let mut decided: HashMap<String, String> = HashMap::new(); // schon vor dem Start entschieden (Hilfsagenten)
        for (ci, call) in tool_calls.iter().enumerate() {
            let name = js_string(&call["function"]["name"]);
            let id = js_string(&call["id"]);
            if signal.is_cancelled() {
                self.push(json!({ "role": "tool", "tool_call_id": id, "name": name, "content": "Von der Person abgebrochen.", "ok": false, "summary": "abgebrochen", "textMode": text_mode }));
                continue;
            }
            // Hilfsagenten, die direkt hintereinander aufgerufen werden, laufen gleichzeitig (höchstens 3, helpers.rs)
            if name == HELPER && !free && !helper_runs.contains_key(&id) && !decided.contains_key(&id) {
                let batch: Vec<&Value> = tool_calls[ci..].iter().take_while(|c| c["function"]["name"] == HELPER).collect();
                for b in &batch {
                    let mut ev = json!({ "type": "tool-start", "id": b["id"], "name": HELPER });
                    ev.as_object_mut().expect("Objekt").extend(describe_call(b));
                    self.event(ev);
                }
                // Erst prüfen, dann starten: Sonst liefe ein abgelehnter Hilfsagent trotzdem (mit Web- und Modellaufrufen)
                let mut approved: Vec<&Value> = Vec::new();
                for b in &batch {
                    let v = if plan && !PLAN_ALLOWED.contains(&HELPER) { "plan".to_string() } else { self.approve(b, signal).await };
                    if v == "yes" {
                        approved.push(b);
                    }
                    decided.insert(js_string(&b["id"]), v);
                }
                if !approved.is_empty() {
                    let futures: Vec<_> = approved.iter().map(|b| { let (me, b, sig) = (self.clone(), (*b).clone(), signal.clone()); async move { me.run_helper_call(&b, &sig).await } }).collect();
                    let handle = tokio::spawn(async move { pool(futures, PARALLEL).await });
                    let shared = Arc::new(HelperBatch { handle: tokio::sync::Mutex::new(Some(handle)), results: tokio::sync::OnceCell::new() });
                    for (k, b) in approved.iter().enumerate() {
                        helper_runs.insert(js_string(&b["id"]), (shared.clone(), k));
                    }
                }
            }
            let started = now_ms();
            if !helper_runs.contains_key(&id) && !decided.contains_key(&id) {
                let mut ev = json!({ "type": "tool-start", "id": id, "name": name });
                ev.as_object_mut().expect("Objekt").extend(describe_call(call));
                self.event(ev);
            }
            // Ein Fehler in der App darf den Verlauf nicht kaputt machen: Jeder Aufruf bekommt ein Ergebnis,
            // sonst lehnt ASK jede weitere Anfrage dieses Chats ab (tool_calls ohne Antwort)
            let verdict = if let Some(v) = decided.remove(&id) {
                v
            } else if plan && !PLAN_ALLOWED.contains(&name.as_str()) {
                "plan".to_string()
            } else if free && !FREE_ALLOWED.contains(&name.as_str()) {
                "frei".to_string()
            } else {
                self.approve(call, signal).await
            };
            let arguments = js_string(&call["function"]["arguments"]);
            let result = if verdict == "plan" {
                ToolResult::new(false, "Im Planmodus änderst du nichts. Lege mit plan_vorlegen einen Plan vor.", "im Planmodus nicht erlaubt")
            } else if verdict == "frei" {
                ToolResult::new(false, "Das geht in einem Chat ohne Projekt nicht: Hier kannst du keine Dateien ändern und keine Befehle ausführen. Sag der Person, dass sie dafür links bei «Projekte» ein Projekt anlegt oder öffnet.", "ohne Projekt nicht möglich")
            } else if name == "plan_vorlegen" {
                propose_plan(&arguments)
            } else if verdict != "yes" {
                ToolResult::new(
                    false,
                    if signal.is_cancelled() { "Von der Person abgebrochen." } else { "Die Person hat diesen Schritt abgelehnt. Mach ohne ihn weiter oder frag kurz nach, was sie stattdessen möchte." },
                    "abgelehnt",
                )
            } else if name == "goal_done" {
                self.finish_goal(&arguments)
            } else if name == EFFORT_TOOL {
                self.set_effort(&arguments)
            } else if name == "frage_stellen" {
                self.ask_question(call, signal).await
            } else if let Some((batch, k)) = helper_runs.get(&id) {
                batch.get(*k).await
            } else {
                self.run_plain_tool(&id, &name, &arguments, signal, flux, memory, can_see).await
            };
            if result.ok && name == "write_file" {
                if let Ok(a) = serde_json::from_str::<Value>(if arguments.is_empty() { "{}" } else { &arguments }) {
                    self.inner.lock().made_this_turn.insert(norm_rel(&js_string(&a["path"])));
                }
            }
            if let Some(made) = result.made.as_ref().filter(|_| result.ok) {
                self.inner.lock().made_this_turn.insert(norm_rel(made)); // z. B. ein neues Doc (E81)
            }
            if !result.sources.is_empty() {
                if let Some(t) = self.inner.lock().turn_sources.as_mut() {
                    if name == "web_fetch" { t.read.extend(result.sources.iter().cloned()); } else { t.found.extend(result.sources.iter().cloned()); }
                }
            }
            if result.plan.as_ref().is_some_and(truthy) {
                planned = true;
            }
            let ms = now_ms() - started;
            let display = clip(result.display.as_deref().unwrap_or(&result.output), 6000);
            let mut tool_msg = json!({ "role": "tool", "tool_call_id": id, "name": name, "content": result.output, "ok": result.ok, "summary": result.summary, "display": display });
            if let Some(t) = &result.tasks {
                tool_msg["tasks"] = json!(t);
            }
            tool_msg["images"] = json!(result.images);
            tool_msg["textMode"] = json!(text_mode);
            tool_msg["ms"] = json!(ms);
            if let Some(p) = result.plan.as_ref().filter(|p| truthy(p)) {
                tool_msg["plan"] = p.clone();
            }
            self.push(tool_msg);
            let mut end = json!({ "type": "tool-end", "id": id, "ok": result.ok, "summary": result.summary, "output": display, "images": result.images, "ms": ms, "plan": result.plan.clone().unwrap_or(Value::Null) });
            if let Some(t) = &result.tasks {
                end["tasks"] = json!(t);
            }
            self.event(end);
            if can_see && !result.images.is_empty() && result.view != Some(false) {
                let detail = describe_call(call).get("detail").map(js_string).unwrap_or_default();
                seen.push((format!("{name}({detail})"), result.images.clone()));
            }
        }
        if !seen.is_empty() && !signal.is_cancelled() {
            let attachments: Vec<Value> = seen.iter().flat_map(|s| s.1.iter()).map(|p| json!({ "path": p, "name": win_basename(p), "image": true })).collect();
            self.push(json!({ "role": "user", "auto": true, "note": "view", "time": now_ms(),
                "content": format!("[Ansicht] Bilder zu {}", seen.iter().map(|s| s.0.as_str()).collect::<Vec<_>>().join(", ")), "attachments": attachments }));
        }
        Ok(planned) // nach plan_vorlegen ist der Auftrag zu Ende, die Person entscheidet
    }

    /// Eine Anfrage an ASK; bricht die Verbindung ab, wartet die App und versucht es nochmals (die halbe Antwort verfällt)
    #[allow(clippy::too_many_arguments)]
    async fn request(self: &Arc<Self>, model: &str, specs: &[Value], goal: Option<&Value>, plan: bool, web_search: bool, signal: &Signal, sent_images: &AtomicBool) -> Result<Completion> {
        let waits: Vec<f64> = self.o.retry_waits.clone().unwrap_or_else(|| RETRY_WAITS.to_vec());
        let mut n = 0;
        loop {
            match self.attempt(model, specs, goal, plan, web_search, signal, sent_images).await {
                Ok(c) => return Ok(c),
                Err(err) => {
                    if signal.is_cancelled() || n >= waits.len() || !retryable(&err) {
                        return Err(err);
                    }
                    let wait = waits[n];
                    let shown = if wait.fract() == 0.0 { json!(wait as i64) } else { json!(wait) };
                    self.event(json!({ "type": "retry", "attempt": n + 1, "of": waits.len(), "wait": shown,
                        "text": format!("Verbindung zu ASK unterbrochen. Neuer Versuch in {wait} s ({}/{}).", n + 1, waits.len()) }));
                    pause(wait, signal).await?;
                    n += 1;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn attempt(self: &Arc<Self>, model: &str, specs: &[Value], goal: Option<&Value>, plan: bool, web_search: bool, signal: &Signal, sent_images: &AtomicBool) -> Result<Completion> {
        let chat_id = self.chat_id();
        let text_mode = self.text_mode();
        let system = (self.o.get_system_prompt)(&chat_id)
            + &goal.map(goal_prompt).unwrap_or_default()
            + if plan { PLAN_PROMPT } else { "" }
            + if self.o.free { FREE_PROMPT } else { "" }
            + &if text_mode { text_mode_prompt(specs) } else { String::new() };
        let image_url = if self.can_see() { self.o.get_image_url.clone() } else { None };
        let budget = if self.inner.lock().tight_for.contains(&chat_id) { TIGHT } else { NORMAL };
        let history = self.messages();
        let messages = api_messages(&system, &history, text_mode, image_url.as_deref().map(|f| f as &dyn Fn(&Value) -> Option<String>), &budget);
        sent_images.store(messages.iter().any(|m| m["content"].is_array()), Ordering::SeqCst);
        self.event(json!({ "type": "assistant-start", "webSearch": web_search }));
        let me = self.clone();
        let on_delta: DeltaFn = Arc::new(move |kind, text| me.event(json!({ "type": kind, "text": text })));
        self.o
            .ask
            .chat(ChatRequest {
                model: model.to_string(),
                messages,
                tools: if text_mode { None } else { Some(specs.to_vec()) },
                web_search,
                thinking: self.thinking(),
                signal: Some(signal.clone()),
                on_delta: Some(on_delta),
            })
            .await
    }

    /// Ein Werkzeug aus tools.rs ausführen (alle außer den Werkzeugen der Arbeitsschleife und den Hilfsagenten)
    #[allow(clippy::too_many_arguments)]
    async fn run_plain_tool(self: &Arc<Self>, id: &str, name: &str, arguments: &str, signal: &Signal, flux: bool, memory: bool, can_see: bool) -> ToolResult {
        let me = self.clone();
        let call_id = id.to_string();
        let on_progress: ProgressFn = Arc::new(move |output| me.event(json!({ "type": "tool-progress", "id": call_id, "output": clip(output, 4000) })));
        let me = self.clone();
        let call_id = id.to_string();
        let on_live: LiveFn = Arc::new(move |data| {
            let mut ev = json!({ "type": "live", "id": call_id });
            if let (Some(o), Some(d)) = (ev.as_object_mut(), data.as_object()) {
                o.extend(d.iter().map(|(k, v)| (k.clone(), v.clone())));
            }
            me.event(ev);
        });
        let (chat, chat_title) = (self.chat_id(), self.chat_title());
        let ctx = ToolCtx {
            signal: Some(signal.clone()),
            on_progress: Some(on_progress),
            on_live: Some(on_live),
            github: self.o.github.clone(),
            github_trust: self.o.github_trust.clone(),
            visual: self.o.visual.clone(),
            servers: self.o.servers.clone(),
            web: self.o.web.clone(),
            images: if flux { self.o.images.clone() } else { None },
            chat_memory: memory,
            can_see,
            chat,
            chat_title,
            knowledge: self.o.knowledge.clone(),
            folder: (self.o.get_folder)(),
            confirm: Some(self.confirm_fn()),
            panel: false,
        };
        run_tool(self.o.terminal.as_deref(), name, &Value::String(arguments.to_string()), &ctx).await
    }

    /// Denkaufwand für die nächste Anfrage: die Einstellung, bei «Automatisch» die Wahl des Modells
    /// in diesem Auftrag (anfangs «mittel»)
    pub fn thinking(&self) -> String {
        let setting = (self.o.get_thinking)();
        if setting != "standard" {
            return setting;
        }
        let chosen = self.inner.lock().auto_effort.clone();
        if chosen.is_empty() { AUTO_START_EFFORT.to_string() } else { chosen }
    }

    /// Werkzeug denkaufwand: Wahl des Modells bei «Automatisch»
    fn set_effort(&self, raw_args: &str) -> ToolResult {
        if (self.o.get_thinking)() != "standard" {
            return ToolResult::new(false, "Der Denkaufwand ist von der Person fest eingestellt.", "fest eingestellt");
        }
        let args = serde_json::from_str::<Value>(if raw_args.is_empty() { "{}" } else { raw_args }).unwrap_or(Value::Null);
        let level = js_string(&args["stufe"]).to_lowercase();
        let level = if level == "hoch" { "maximal".to_string() } else { level };
        if !["tief", "mittel", "maximal"].contains(&level.as_str()) {
            return ToolResult::new(false, "Unbekannte Stufe. Erlaubt: tief, mittel, maximal.", "unbekannte Stufe");
        }
        self.inner.lock().auto_effort = level.clone();
        self.event(json!({ "type": "effort", "level": level }));
        let name = match level.as_str() { "tief" => "Tief", "maximal" => "Maximal", _ => "Mittel" };
        ToolResult::new(true, format!("Denkaufwand für die nächsten Schritte: {level}."), name)
    }

    /// Werkzeug goal_done: Ziel als erreicht markieren
    fn finish_goal(&self, raw_args: &str) -> ToolResult {
        let summary = serde_json::from_str::<Value>(if raw_args.is_empty() { "{}" } else { raw_args }).ok().map(|a| js_string(&a["zusammenfassung"])).unwrap_or_default();
        {
            let mut g = self.inner.lock();
            if !truthy(&g.chat["goal"]) {
                return ToolResult::new(false, "Es ist kein Ziel gesetzt.", "kein Ziel");
            }
            g.chat["goal"]["done"] = json!(true);
            g.chat["goal"]["summary"] = json!(summary);
            g.chat["goal"]["doneAt"] = json!(now_ms());
        }
        self.emit_changed();
        ToolResult::new(true, "Ziel als erreicht markiert. Schreib jetzt die kurze Schlussantwort.", if summary.is_empty() { "Ziel erreicht".to_string() } else { summary })
    }

    /// Rückfrage vor heiklen Schritten: «yes» oder «no». Wartet auf die Antwort der Person im Chat
    /// (`answer_approval`); Stoppen gilt als Nein.
    async fn approve(self: &Arc<Self>, call: &Value, signal: &Signal) -> String {
        let name = js_string(&call["function"]["name"]);
        let raw = js_string(&call["function"]["arguments"]);
        let Ok(args) = serde_json::from_str::<Value>(if raw.is_empty() { "{}" } else { &raw }) else { return "yes".into() }; // run_tool meldet den Fehler
        // Auto: der Prüfer entscheidet; Nachfragen: Stufe aus den Einstellungen (vor jedem Befehl)
        let asked = if (self.o.get_mode)() == "auto" {
            self.auto_risk(call, &args, signal).await
        } else {
            let mode = (self.o.get_approval_mode)();
            let exists = |rel: &str| (self.o.file_exists)(rel);
            let made = |rel: &str| self.inner.lock().made_this_turn.contains(&norm_rel(rel));
            assess_risk(&name, &args, &RiskCtx { mode: &mode, exists: &exists, made_this_turn: &made }).map(|risk| Asked { risk, reviewed: false })
        };
        if signal.is_cancelled() {
            return "no".into();
        }
        let asked = asked.or_else(|| mod_rule_risk(&name, &args, &(self.o.get_ask_before)()).map(|risk| Asked { risk, reviewed: false }));
        let Some(asked) = asked else { return "yes".into() };
        if self.inner.lock().allowed.contains(&asked.risk.kind) {
            return "yes".into();
        }
        let id = format!("frage_{}", hex::encode(rand::random::<[u8; 4]>()));
        // Anmelden vor dem Ereignis: Wer darin gleich antwortet, geht nicht ins Leere
        let (tx, rx) = oneshot::channel::<String>();
        if self.o.auto_approve.is_none() {
            self.approvals.lock().insert(id.clone(), tx);
            self.approval_text.lock().insert(id.clone(), (asked.risk.title.clone(), asked.risk.detail.clone()));
        }
        let ev = json!({ "type": "approval", "id": id, "kind": asked.risk.kind, "title": asked.risk.title, "detail": asked.risk.detail,
            "reason": asked.risk.reason, "reviewed": asked.reviewed });
        if self.o.auto_approve.is_none() {
            self.open_event(&id, &ev);
        }
        self.event(ev);
        let answer = if let Some(auto) = &self.o.auto_approve {
            auto(asked.risk.clone()).await
        } else {
            tokio::select! {
                a = rx => a.unwrap_or_else(|_| "no".into()),
                _ = signal.cancelled() => "no".into(),
            }
        };
        self.approvals.lock().remove(&id);
        self.approval_text.lock().remove(&id);
        self.close_event(&id);
        if answer == "always" {
            self.inner.lock().allowed.insert(asked.risk.kind.clone());
        }
        let verdict = if answer == "yes" || answer == "always" { "yes" } else { "no" };
        self.event(json!({ "type": "approval-end", "id": id, "answer": if verdict == "yes" { answer.as_str() } else { "no" } }));
        verdict.to_string()
    }

    /// Modus «Auto»: Befehle und heikle Schritte (Löschen, ausserhalb, Überschreiben) prüft ein Modell in
    /// ASK. Harmlos: ohne Rückfrage. Gefährlich: Rückfrage mit dem Grund. Ist die Prüfung nicht möglich
    /// (Fehler, keine verwertbare Antwort), gelten die festen Regeln (approvals.rs, Stufe «heikel»).
    async fn auto_risk(self: &Arc<Self>, call: &Value, args: &Value, signal: &Signal) -> Option<Asked> {
        let name = js_string(&call["function"]["name"]);
        let exists = |rel: &str| (self.o.file_exists)(rel);
        let made = |rel: &str| self.inner.lock().made_this_turn.contains(&norm_rel(rel));
        let rule = assess_risk(&name, args, &RiskCtx { mode: "heikel", exists: &exists, made_this_turn: &made });
        let is_command = name == "run_command" || name == "server_starten";
        let command: Option<String> = if name == "run_command" {
            args.get("command").filter(|v| !v.is_null()).map(js_string)
        } else if name == "server_starten" {
            [args.get("befehl"), args.get("command")].into_iter().flatten().find(|v| truthy(v)).or(args.get("command")).filter(|v| !v.is_null()).map(js_string)
        } else {
            None
        };
        if !is_command && rule.is_none() {
            return None; // Lesen, Suchen, neue Dateien im Projekt: ohne Prüfung
        }
        let Some(review) = &self.o.review else { return rule.map(|risk| Asked { risk, reviewed: false }) };
        let shown = command.clone().or_else(|| args.get("path").filter(|v| !v.is_null()).map(js_string)).unwrap_or_default();
        let key = format!("{name}:{shown}");
        if self.inner.lock().reviewed_safe.contains(&key) {
            return None;
        }
        self.event(json!({ "type": "review", "id": call["id"] }));
        let task = self.messages().iter().rev().find(|m| m["role"] == "user" && !truthy(&m["auto"])).map(|m| js_string(&m["content"])).unwrap_or_default();
        let verdict = review(ReviewRequest { name: name.clone(), args: args.clone(), task, folder: (self.o.get_folder)(), signal: Some(signal.clone()) }).await.ok().flatten();
        if signal.is_cancelled() {
            return None;
        }
        let Some(verdict) = verdict else {
            return rule.map(|r| {
                let reason = [r.reason.as_str(), "Die Prüfung durch ASK war nicht möglich."].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
                Asked { risk: Risk { reason, ..r }, reviewed: false }
            });
        };
        if !verdict.dangerous {
            self.inner.lock().reviewed_safe.insert(key);
            return None;
        }
        let detail = match &rule {
            Some(r) if !r.detail.is_empty() => r.detail.clone(),
            _ => take_chars(&re!(r"\s+").replace_all(&shown, " "), 300),
        };
        Some(Asked {
            risk: Risk {
                kind: match &rule { Some(r) if !r.kind.is_empty() => format!("{}:{key}", r.kind), _ => format!("gefahr:{key}") },
                title: match &rule {
                    Some(r) if !r.title.is_empty() => r.title.clone(),
                    _ if is_command => "Diesen Befehl ausführen?".to_string(),
                    _ => "Diesen Schritt ausführen?".to_string(),
                },
                detail,
                reason: if verdict.reason.is_empty() { "ASK hält den Schritt für gefährlich.".to_string() } else { verdict.reason },
            },
            reviewed: true,
        })
    }

    /// Werkzeug frage_stellen: Frage im Chat zeigen und auf die Antwort warten (`answer_question`)
    async fn ask_question(self: &Arc<Self>, call: &Value, signal: &Signal) -> ToolResult {
        let raw = js_string(&call["function"]["arguments"]);
        let args = serde_json::from_str::<Value>(if raw.is_empty() { "{}" } else { &raw }).unwrap_or_else(|_| json!({}));
        let first = |keys: &[&str]| keys.iter().filter_map(|k| args.get(*k)).find(|v| truthy(v)).map(js_string).unwrap_or_default();
        let frage = first(&["frage", "question"]).trim().to_string();
        if frage.is_empty() {
            return ToolResult::new(false, "frage fehlt.", "ohne Frage");
        }
        let list = if args["optionen"].is_array() { &args["optionen"] } else if args["options"].is_array() { &args["options"] } else { &Value::Null };
        let optionen: Vec<String> = arr(list)
            .iter()
            .map(|o| {
                if o.is_object() {
                    let t = ["text", "label"].iter().filter_map(|k| o.get(*k)).find(|v| truthy(v)).map(js_string).unwrap_or_default();
                    t.trim().to_string()
                } else {
                    js_string(o).trim().to_string()
                }
            })
            .filter(|o| !o.is_empty())
            .take(6)
            .collect();
        let id = js_string(&call["id"]);
        // Anmelden vor dem Ereignis: Wer darin gleich antwortet, geht nicht ins Leere
        let (tx, rx) = oneshot::channel::<String>();
        if self.o.auto_answer.is_none() {
            self.questions.lock().insert(id.clone(), tx);
        }
        let ev = json!({ "type": "question", "id": id, "frage": frage, "optionen": optionen, "mehrfach": truthy(&args["mehrfach"]) });
        if self.o.auto_answer.is_none() {
            self.open_event(&id, &ev);
        }
        self.event(ev);
        let answer: Option<String> = if let Some(auto) = &self.o.auto_answer {
            auto(frage.clone(), optionen.clone()).await
        } else {
            tokio::select! {
                a = rx => a.ok(),
                _ = signal.cancelled() => None,
            }
        };
        self.questions.lock().remove(&id);
        self.close_event(&id);
        let Some(answer) = answer else {
            self.event(json!({ "type": "question-end", "id": id, "answer": "" }));
            return ToolResult::new(false, "Die Person hat nicht geantwortet (gestoppt).", "nicht beantwortet");
        };
        let text = if answer.trim().is_empty() { "(leer)".to_string() } else { answer.trim().to_string() };
        self.event(json!({ "type": "question-end", "id": id, "answer": text }));
        let mut r = ToolResult::new(true, format!("Antwort der Person: {text}"), clip(&text, 200));
        r.display = Some(format!("Frage: {frage}\nAntwort: {text}"));
        r
    }

    /// Antwort der Person auf eine Frage des Modells
    pub fn answer_question(&self, id: &str, text: &str) -> bool {
        let Some(tx) = self.questions.lock().remove(id) else { return false };
        let _ = tx.send(take_chars(text, 4000));
        true
    }

    /// Titel und Details einer offenen Rückfrage
    pub fn approval_text(&self, id: &str) -> Option<(String, String)> {
        self.approval_text.lock().get(id).cloned()
    }

    /// Antwort der Person auf eine Rückfrage: «yes» | «always» | «no»
    pub fn answer_approval(&self, id: &str, answer: &str) -> bool {
        let Some(tx) = self.approvals.lock().remove(id) else { return false };
        let _ = tx.send(if ["yes", "always", "no"].contains(&answer) { answer.to_string() } else { "no".to_string() });
        true
    }

    /// Was vom Modell schon angekommen war, als die Antwort abbrach (Stopp, Verbindung weg), bleibt im Chat:
    /// Die Person hat es gesehen, und das Modell soll beim nächsten Auftrag wissen, wie weit es war.
    fn keep_partial(&self, err: &anyhow::Error) {
        let Some(p) = partial_of(err) else { return };
        if p.content.trim().is_empty() && p.reasoning.trim().is_empty() {
            return;
        }
        let mut message = json!({ "role": "assistant", "content": p.content, "time": now_ms(), "stats": serde_json::to_value(p.stats()).unwrap_or(Value::Null), "stopped": true });
        if !p.reasoning.is_empty() {
            message["reasoning"] = json!(p.reasoning);
        }
        self.push(message);
    }

    async fn backup(&self, snapshots: &Snapshots, index: usize) {
        let label = self.chat_title();
        match snapshots.take(&label).await {
            Ok(stand) => {
                self.inner.lock().chat["messages"][index]["snapshot"] = json!(stand.id);
                let skipped = stand.skipped.join("\n");
                if !skipped.is_empty() && skipped != self.inner.lock().last_skipped {
                    self.notice(format!(
                        "Nicht gesichert (grösser als 100 MB oder gesperrt): {}{}. Änderungen daran lassen sich nicht rückgängig machen.",
                        stand.skipped.iter().take(5).cloned().collect::<Vec<_>>().join(", "),
                        if stand.skipped.len() > 5 { format!(" und {} weitere", stand.skipped.len() - 5) } else { String::new() }
                    ));
                }
                self.inner.lock().last_skipped = skipped;
            }
            Err(err) => self.notice(format!("Der Ordner wurde nicht gesichert: {err} Änderungen dieses Auftrags lassen sich nicht rückgängig machen.")),
        }
    }

    /// Nach dem Auftrag: was hat sich im Ordner geändert? Anzeige mit Knopf «Rückgängig».
    async fn record_changes(&self, snapshots: &Snapshots, index: usize) {
        let snapshot = js_string(&self.inner.lock().chat["messages"][index]["snapshot"]);
        let Ok(d) = snapshots.diff(&snapshot).await else { return }; // Sicherung verschwunden: kein Rückgängig
        if d.changed.is_empty() && d.added.is_empty() && d.removed.is_empty() {
            return;
        }
        let changes = json!({
            "changed": d.changed.iter().take(50).collect::<Vec<_>>(), "added": d.added.iter().take(50).collect::<Vec<_>>(),
            "removed": d.removed.iter().take(50).collect::<Vec<_>>(), "counts": [d.changed.len(), d.added.len(), d.removed.len()],
        });
        self.inner.lock().chat["messages"][index]["changes"] = changes.clone();
        self.event(json!({ "type": "changes", "snapshot": snapshot, "changes": changes }));
    }

    /// Ordner auf den Stand vor einem Auftrag zurücksetzen (nach Rückfrage). Das Modell erfährt es im Verlauf.
    /// Während des Zurücksetzens (Rückfrage, Sicherung, Kopieren) startet kein Auftrag und kein anderer Chat.
    pub async fn undo(&self, id: &str) -> Value {
        if self.running() {
            return json!({ "ok": false, "reason": "Es läuft gerade ein Auftrag." });
        }
        if self.undoing.swap(true, Ordering::SeqCst) {
            return json!({ "ok": false, "busy": true, "reason": "Es wird schon zurückgesetzt." });
        }
        let result = match self.undo_now(id).await {
            Ok(v) => v,
            Err(err) => json!({ "ok": false, "reason": format!("Zurücksetzen nicht möglich: {err}") }),
        };
        self.undoing.store(false, Ordering::SeqCst);
        self.emit_changed();
        result
    }

    async fn undo_now(&self, id: &str) -> Result<Value> {
        let found = arr(&self.inner.lock().chat["messages"]).iter().position(|m| m["role"] == "user" && m["snapshot"] == id);
        let snapshots = (self.o.get_snapshots)();
        let (Some(at), Some(snapshots)) = (found, snapshots) else {
            return Ok(json!({ "ok": false, "reason": "Zu diesem Auftrag gibt es keine Sicherung." }));
        };
        let Ok(d) = snapshots.diff(id).await else {
            return Ok(json!({ "ok": false, "reason": "Die Sicherung gibt es nicht mehr (sie wird nach 14 Tagen gelöscht) oder ist nicht lesbar." }));
        };
        let total = d.changed.len() + d.added.len() + d.removed.len();
        if total == 0 {
            return Ok(json!({ "ok": false, "reason": "Seit diesem Auftrag hat sich im Ordner nichts geändert." }));
        }
        let list = |title: &str, files: &[String]| -> String {
            if files.is_empty() {
                return String::new();
            }
            format!(
                "{title}:\n{}{}",
                files.iter().take(12).map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n"),
                if files.len() > 12 { format!("\n  … und {} weitere", files.len() - 12) } else { String::new() }
            )
        };
        let detail = [list("Wird auf den alten Stand gesetzt", &d.changed), list("Wird gelöscht (neu seit dem Auftrag)", &d.added), list("Wird wiederhergestellt", &d.removed)]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        // Rückgängig bis zu einem früheren Schritt: Was spätere Aufträge geändert haben, geht mit
        let later: Vec<usize> = {
            let g = self.inner.lock();
            arr(&g.chat["messages"]).iter().enumerate().skip(at + 1).filter(|(_, m)| m["role"] == "user" && truthy(&m["snapshot"]) && !truthy(&m["undone"])).map(|(i, _)| i).collect()
        };
        let warn = if later.is_empty() {
            "Auch spätere Änderungen an diesen Dateien gehen dabei verloren.".to_string()
        } else {
            format!("Auch die Änderungen {} danach gehen dabei verloren.", if later.len() == 1 { "des Auftrags".to_string() } else { format!("der {} Aufträge", later.len()) })
        };
        let answer = (self.confirm_fn())("Ordner auf den Stand vor diesem Auftrag zurücksetzen?".to_string(), format!("{detail}\n\n{warn}{}", self.sibling_warning()), ConfirmOptions::default()).await;
        if !answer.ok {
            return Ok(json!({ "ok": false, "cancelled": true }));
        }
        let r = snapshots.restore(id).await?;
        let (content, undid) = {
            let mut g = self.inner.lock();
            let mut undid = Vec::new();
            for i in std::iter::once(&at).chain(later.iter()) {
                if let Some(m) = g.chat["messages"].get_mut(*i) {
                    m["undone"] = json!(true);
                    undid.push(m["snapshot"].clone());
                }
            }
            let m = &g.chat["messages"][at];
            (clip(&if truthy(&m["content"]) { js_string(&m["content"]) } else { "Anhang".to_string() }, 80), undid)
        };
        let done = total.saturating_sub(r.failed.len());
        let text = format!(
            "Rückgängig gemacht: {done} {} auf den Stand vor dem Auftrag gesetzt.{}",
            if done == 1 { "Datei" } else { "Dateien" },
            if r.failed.is_empty() { String::new() } else { format!(" Nicht möglich (wohl geöffnet): {}.", r.failed.iter().take(5).cloned().collect::<Vec<_>>().join(", ")) }
        );
        // redo: der Stand direkt vor dem Zurücksetzen (Sicherung von restore), für «Wiederherstellen»
        let redo = r.safety.clone().map(Value::from).unwrap_or(Value::Null);
        self.push(json!({ "role": "user", "auto": true, "note": "undo", "summary": text, "time": now_ms(), "redo": redo, "undid": undid,
            "content": format!("[Hinweis der App: Die Person hat den Auftrag «{content}» rückgängig gemacht. Der Arbeitsordner ist wieder auf dem Stand vor diesem Auftrag; was dabei geändert oder angelegt wurde, gibt es so nicht mehr.]") }));
        self.save();
        self.event(json!({ "type": "undo", "snapshot": id, "text": text, "redo": redo }));
        Ok(json!({ "ok": true, "text": text }))
    }

    /// «Wiederherstellen»: ein Rückgängig zurücknehmen. `id` ist der Stand, den Rückgängig vorher gesichert hat
    /// (`redo` im Hinweis). Die rückgängig gemachten Aufträge gelten danach wieder; geht nur einmal pro Rückgängig.
    pub async fn redo(&self, id: &str) -> Value {
        if self.running() {
            return json!({ "ok": false, "reason": "Es läuft gerade ein Auftrag." });
        }
        if self.undoing.swap(true, Ordering::SeqCst) {
            return json!({ "ok": false, "busy": true, "reason": "Es wird schon zurückgesetzt." });
        }
        let result = match self.redo_now(id).await {
            Ok(v) => v,
            Err(err) => json!({ "ok": false, "reason": format!("Wiederherstellen nicht möglich: {err}") }),
        };
        self.undoing.store(false, Ordering::SeqCst);
        self.emit_changed();
        result
    }

    async fn redo_now(&self, id: &str) -> Result<Value> {
        let found = arr(&self.inner.lock().chat["messages"]).iter().position(|m| m["note"] == "undo" && m["redo"] == id && !truthy(&m["redone"]));
        let snapshots = (self.o.get_snapshots)();
        // nur das letzte Rückgängig: nach einem weiteren stimmten die Markierungen der Aufträge sonst nicht mehr
        let newer = found.is_some_and(|at| arr(&self.inner.lock().chat["messages"]).iter().skip(at + 1).any(|m| m["note"] == "undo" || m["note"] == "redo"));
        let (Some(at), Some(snapshots), false) = (found, snapshots, newer) else {
            return Ok(json!({ "ok": false, "reason": "Dieses Rückgängig lässt sich nicht mehr zurücknehmen." }));
        };
        let Ok(d) = snapshots.diff(id).await else {
            return Ok(json!({ "ok": false, "reason": "Die Sicherung gibt es nicht mehr (sie wird nach 14 Tagen gelöscht) oder ist nicht lesbar." }));
        };
        let total = d.changed.len() + d.added.len() + d.removed.len();
        let list = |title: &str, files: &[String]| -> String {
            if files.is_empty() {
                return String::new();
            }
            format!(
                "{title}:\n{}{}",
                files.iter().take(12).map(|f| format!("  {f}")).collect::<Vec<_>>().join("\n"),
                if files.len() > 12 { format!("\n  … und {} weitere", files.len() - 12) } else { String::new() }
            )
        };
        let detail = [list("Kommt wieder in die Fassung nach dem Auftrag", &d.changed), list("Wird gelöscht (neu seit dem Rückgängig)", &d.added), list("Wird wiederhergestellt", &d.removed)]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n");
        // Aufträge seit dem Rückgängig: Ihre Änderungen gehen mit, wie bei Rückgängig zu einem früheren Auftrag
        let later: Vec<usize> = {
            let g = self.inner.lock();
            arr(&g.chat["messages"]).iter().enumerate().skip(at + 1).filter(|(_, m)| m["role"] == "user" && truthy(&m["snapshot"]) && !truthy(&m["undone"])).map(|(i, _)| i).collect()
        };
        let warn = if later.is_empty() {
            String::new()
        } else {
            format!("\n\nAuch die Änderungen {} seit dem Rückgängig gehen dabei verloren.", if later.len() == 1 { "des Auftrags".to_string() } else { format!("der {} Aufträge", later.len()) })
        };
        if total > 0 {
            let answer = (self.confirm_fn())("Rückgängig zurücknehmen?".to_string(), format!("{detail}{warn}{}", self.sibling_warning()), ConfirmOptions::default()).await;
            if !answer.ok {
                return Ok(json!({ "ok": false, "cancelled": true }));
            }
        }
        let r = if total > 0 { Some(snapshots.restore(id).await?) } else { None };
        let failed = r.as_ref().map(|r| r.failed.clone()).unwrap_or_default();
        let (undid, later_ids) = {
            let mut g = self.inner.lock();
            let undid: Vec<Value> = arr(&g.chat["messages"][at]["undid"]).to_vec();
            if let Some(list) = g.chat["messages"].as_array_mut() {
                for m in list.iter_mut() {
                    if m["role"] == "user" && truthy(&m["snapshot"]) && undid.contains(&m["snapshot"]) {
                        m["undone"] = json!(false);
                    }
                }
            }
            let mut later_ids = Vec::new();
            for i in &later {
                if let Some(m) = g.chat["messages"].get_mut(*i) {
                    m["undone"] = json!(true);
                    later_ids.push(m["snapshot"].clone());
                }
            }
            g.chat["messages"][at]["redone"] = json!(true);
            (undid, later_ids)
        };
        let done = total.saturating_sub(failed.len());
        let text = if total == 0 && failed.is_empty() {
            "Wiederhergestellt: Die Aufträge gelten wieder. Dateien waren nicht betroffen.".to_string()
        } else { format!(
            "Wiederhergestellt: {done} {} wieder auf dem Stand vor dem Rückgängig.{}",
            if done == 1 { "Datei" } else { "Dateien" },
            if failed.is_empty() { String::new() } else { format!(" Nicht möglich (wohl geöffnet): {}.", failed.iter().take(5).cloned().collect::<Vec<_>>().join(", ")) }
        ) };
        self.push(json!({ "role": "user", "auto": true, "note": "redo", "summary": text, "time": now_ms(),
            "content": "[Hinweis der App: Die Person hat das letzte Rückgängig zurückgenommen. Der Arbeitsordner ist wieder auf dem Stand vor dem Rückgängig; die Änderungen der rückgängig gemachten Aufträge gelten wieder.]" }));
        self.save();
        self.event(json!({ "type": "redo", "redo": id, "snapshots": undid, "later": later_ids, "text": text }));
        Ok(json!({ "ok": true, "text": text }))
    }

    /// Zusatz für die Rückfrage vor dem Zurücksetzen, wenn ein anderer Chat gerade im selben Ordner arbeitet
    fn sibling_warning(&self) -> String {
        let others = (self.o.get_siblings)();
        if others.is_empty() {
            return String::new();
        }
        format!(
            "\n\nAchtung: Gerade arbeitet auch {} in diesem Ordner. Was dieser Chat geändert hat, wird ebenfalls zurückgesetzt.",
            others.iter().map(|t| format!("«{}»", clip(t, 60))).collect::<Vec<_>>().join(", ")
        )
    }

    /// Einen Hilfsagenten ausführen (helpers.rs), mit eigenem Modell aus den Einstellungen
    async fn run_helper_call(self: &Arc<Self>, call: &Value, signal: &Signal) -> ToolResult {
        let helper_model = (self.o.get_helper_model)();
        let id = js_string(&call["id"]);
        let me = self.clone();
        let api_messages_fn: ApiMessagesFn = Arc::new(|system, messages, text_mode| api_messages(system, messages, text_mode, None, &NORMAL));
        let prompt_fn: TextModePromptFn = Arc::new(text_mode_prompt);
        let (sources, strict) = (self.o.get_sources)();
        let deps = HelperDeps {
            ask: self.o.ask.clone(),
            model: if helper_model.is_empty() { (self.o.get_model)() } else { helper_model },
            thinking: self.thinking(),
            terminal: self.o.terminal.clone(),
            web: self.o.web.clone(),
            folder: (self.o.get_folder)(),
            chat: self.chat_id(),
            text_mode: self.text_mode(),
            api_messages: api_messages_fn,
            text_mode_prompt: prompt_fn,
            signal: Some(signal.clone()),
            on_progress: Some(Arc::new(move |output| me.event(json!({ "type": "tool-progress", "id": id, "output": clip(output, 4000) })))),
            sources,
            strict,
        };
        run_helper(&js_string(&call["function"]["arguments"]), &deps).await
    }

    /// Für die Anzeige eines gespeicherten Chats.
    /// Nebenfrage mit /btw (ENTSCHEIDE E86): eine eigene Anfrage an ASK mit dem Chat bis jetzt und der Frage, ohne
    /// Werkzeuge, neben dem laufenden Auftrag. Die Antwort kommt als Ereignisse `btw-start`, `btw-delta`, `btw-end`
    /// und steht danach im Chat (Rolle `btw`); an das Modell des Auftrags geht sie nicht (`api_messages` überspringt sie).
    pub fn btw(self: &Arc<Self>, question: &str) -> Value {
        let question = question.trim().to_string();
        if question.is_empty() {
            return json!({ "ok": false, "reason": "Nach /btw fehlt die Frage." });
        }
        if (self.o.get_model)().is_empty() {
            return json!({ "ok": false, "reason": "Kein Modell gewählt." });
        }
        // Zu welchem Auftrag die Nebenfrage gehört: Zeit der letzten Eingabe der Person (so steht sie auch nach dem
        // Neuladen dort, obwohl sie erst nach der Antwort am Ende des Chats gespeichert wird)
        let after = self.messages().iter().rev()
            .find(|m| m["role"] == "user" && m.get("note").is_none_or(Value::is_null) && !truthy(&m["auto"]))
            .map(|m| m["time"].clone());
        let Some(after) = after else {
            return json!({ "ok": false, "reason": "/btw ist für eine Nebenfrage, während ASK an etwas arbeitet. Stell deine Frage hier als normale Nachricht." });
        };
        let id = format!("btw-{}", hex::encode(rand::random::<[u8; 4]>()));
        let time = now_ms();
        let chat = self.chat_id();
        self.event(json!({ "type": "btw-start", "id": id, "question": question, "time": time }));
        let me = self.clone();
        let task_id = id.clone();
        tokio::spawn(async move { me.answer_btw(task_id, question, time, chat, after).await });
        json!({ "ok": true, "id": id })
    }

    /// Nachrichten für die Nebenfrage: Systemprompt des Chats mit dem Abschnitt «Nebenfrage», der Verlauf wie für den
    /// Auftrag (Schritte ohne Ergebnis heissen «läuft noch»), dazu die Frage mit Zeitstempel
    fn btw_messages(&self, question: &str, time: i64) -> Vec<Value> {
        let chat_id = self.chat_id();
        let system = (self.o.get_system_prompt)(&chat_id) + if self.o.free { FREE_PROMPT } else { "" } + BTW_PROMPT;
        let budget = if self.inner.lock().tight_for.contains(&chat_id) { TIGHT } else { NORMAL };
        let mut messages = api_messages(&system, &self.messages(), self.text_mode(), None, &budget);
        for m in messages.iter_mut().filter(|m| m["role"] == "tool" && m["content"] == NOT_RUN) {
            m["content"] = json!(BTW_RUNNING);
        }
        let stamp = time_stamp(time);
        messages.push(json!({ "role": "user", "content": format!("{}{BTW_HEAD}{question}", if stamp.is_empty() { String::new() } else { format!("{stamp} ") }) }));
        messages
    }

    async fn answer_btw(self: Arc<Self>, id: String, question: String, time: i64, chat: String, after: Value) {
        let me = self.clone();
        let delta_id = id.clone();
        let delta_chat = chat.clone();
        // Ereignisse nur, solange derselbe Chat geladen ist: Sonst stünde die Karte im Chat, zu dem gewechselt wurde
        let on_delta: DeltaFn = Arc::new(move |kind, text| {
            if kind == "text" && me.chat_id() == delta_chat {
                me.event(json!({ "type": "btw-delta", "id": delta_id, "text": text }));
            }
        });
        let req = ChatRequest {
            model: (self.o.get_model)(),
            messages: self.btw_messages(&question, time),
            tools: None,
            web_search: false,
            thinking: (self.o.get_thinking)(),
            signal: None,
            on_delta: Some(on_delta),
        };
        let (answer, error) = match tokio::time::timeout(Duration::from_secs(300), self.o.ask.chat(req)).await {
            Ok(Ok(done)) => {
                let text = re!(r"(?s)<think>.*?</think>").replace_all(&done.content, "").trim().to_string();
                if text.is_empty() { (String::new(), "ASK hat keine Antwort geschickt.".to_string()) } else { (text, String::new()) }
            }
            Ok(Err(err)) => (String::new(), format!("Keine Antwort: {err}")),
            Err(_) => (String::new(), "ASK hat nicht innert 5 Minuten geantwortet.".to_string()),
        };
        // Im Chat speichern, wenn er noch geladen ist (sonst gehört die Antwort zu einem Chat, der nicht mehr hier ist)
        if self.chat_id() != chat {
            return;
        }
        self.push(json!({ "role": "btw", "id": id, "question": question, "answer": answer, "error": error, "time": time, "after": after }));
        self.save();
        self.event(json!({ "type": "btw-end", "id": id, "question": question, "time": time, "text": answer, "error": error }));
    }

    pub fn transcript(&self) -> Value {
        let list = self.messages();
        let call_list = |m: &Value| -> Vec<Value> {
            arr(&m["tool_calls"])
                .iter()
                .map(|c| {
                    let mut o = Map::new();
                    o.insert("id".into(), c["id"].clone());
                    o.insert("name".into(), c["function"]["name"].clone());
                    o.extend(describe_call(c));
                    Value::Object(o)
                })
                .collect()
        };
        let or = |v: &Value, d: Value| if truthy(v) { v.clone() } else { d };
        let item = |m: &Value| -> Value {
            if m["role"] == "note" {
                return json!({ "role": if m["kind"] == "error" { "error" } else { "notice" }, "text": m["content"] });
            }
            if m["role"] == "assistant" {
                return json!({ "role": "assistant", "content": m["content"], "reasoning": or(&m["reasoning"], json!("")), "stats": or(&m["stats"], Value::Null),
                    "sources": or(&m["sources"], json!([])), "time": m["time"], "stopped": truthy(&m["stopped"]), "calls": call_list(m) });
            }
            if m["role"] == "tool" {
                let mut t = json!({ "role": "tool", "id": m["tool_call_id"], "ok": m["ok"] != json!(false), "summary": or(&m["summary"], json!("")),
                    "output": if m["display"].is_null() { m["content"].clone() } else { m["display"].clone() }, "images": or(&m["images"], json!([])), "plan": or(&m["plan"], Value::Null) });
                if let Some(tasks) = m.get("tasks").filter(|t| !t.is_null()) {
                    t["tasks"] = tasks.clone();
                }
                return t;
            }
            if m["note"] == "undo" {
                return json!({ "role": "undo", "text": m["summary"], "redo": m["redo"], "redone": truthy(&m["redone"]) });
            }
            if m["note"] == "redo" {
                return json!({ "role": "redo", "text": m["summary"] });
            }
            if m["role"] == "btw" {
                return json!({ "role": "btw", "id": m["id"], "question": m["question"], "answer": m["answer"], "error": m["error"], "time": m["time"], "after": m["after"] });
            }
            json!({ "role": if truthy(&m["auto"]) { json!("auto") } else { m["role"].clone() }, "content": m["content"], "time": m["time"],
                "attachments": or(&m["attachments"], json!([])), "snapshot": or(&m["snapshot"], Value::Null), "changes": or(&m["changes"], Value::Null), "undone": truthy(&m["undone"]) })
        };
        Value::Array(
            list.iter()
                .filter(|m| m["note"] != "view")
                .flat_map(|m| {
                    // Ab hier geht der ältere Verlauf als Zusammenfassung an das Modell (E78): Hinweis vor der Eingabe
                    let mark = truthy(&m["summaryBefore"]["content"]).then(|| json!({ "role": "notice", "text": COMPACT_NOTICE }));
                    mark.into_iter().chain(std::iter::once(item(m)))
                })
                .collect(),
        )
    }
}

/// Werkzeug plan_vorlegen: Plan im Chat zeigen (Knöpfe «Umsetzen» und «Ändern» in der Oberfläche)
fn propose_plan(raw_args: &str) -> ToolResult {
    let text = serde_json::from_str::<Value>(if raw_args.is_empty() { "{}" } else { raw_args }).ok().map(|a| js_string(&a["plan"]).trim().to_string()).unwrap_or_default();
    if text.is_empty() {
        return ToolResult::new(false, "plan fehlt.", "ohne Plan");
    }
    let mut r = ToolResult::new(true, "Der Plan ist der Person vorgelegt. Hör jetzt auf und warte auf ihre Antwort.", "Plan vorgelegt");
    r.plan = Some(json!(clip(&text, 20000)));
    r
}

#[cfg(test)]
mod tests {

    #[test]
    fn titel_aus_der_antwort_des_modells() {
        assert_eq!(clean_title("<think>hm</think>\n«Budget 2027 planen».\nmehr"), "Budget 2027 planen");
        assert_eq!(clean_title("Titel: Python-Passwortprüfer"), "Python-Passwortprüfer");
        assert_eq!(clean_title("**Excel-Auswertung der Ausgaben**"), "Excel-Auswertung der Ausgaben");
        assert_eq!(clean_title("  \n"), "");
        assert_eq!(clean_title(&"x".repeat(100)).chars().count(), 60);
        assert_eq!(clean_title("Für welche App genau? Sag mir mehr."), "", "Rückfrage ist kein Titel");
        assert_eq!(clean_title("«Welche App meinst du?»"), "");
        assert_eq!(clean_title("Ich habe für dich einen schönen Titel gefunden der passt"), "", "ganzer Satz");
    }

    #[test]
    fn regeln_der_mods_machen_nur_strenger() {
        let k = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert!(mod_rule_risk("run_command", &json!({ "command": "dir" }), &[]).is_none());
        assert_eq!(mod_rule_risk("run_command", &json!({ "command": "dir" }), &k(&["befehle"])).unwrap().kind, "mod-befehle");
        assert_eq!(mod_rule_risk("run_command", &json!({ "command": "del a.txt" }), &k(&["loeschen"])).unwrap().kind, "mod-loeschen");
        assert!(mod_rule_risk("run_command", &json!({ "command": "dir" }), &k(&["loeschen"])).is_none());
        assert_eq!(mod_rule_risk("write_file", &json!({ "path": "a.md" }), &k(&["schreiben"])).unwrap().title, "Darf ASK «a.md» schreiben?");
        assert_eq!(mod_rule_risk("web_fetch", &json!({ "url": "https://x.ch" }), &k(&["web"])).unwrap().detail, "https://x.ch");
        assert!(mod_rule_risk("read_file", &json!({ "path": "a.md" }), &k(&["schreiben", "web", "befehle", "loeschen"])).is_none());
        assert_eq!(mod_rule_risk(HELPER, &json!({ "titel": "Preise", "auftrag": "x" }), &k(&["web"])).unwrap().detail, "Preise");
        assert!(mod_rule_risk(HELPER, &json!({ "titel": "Preise" }), &k(&["schreiben"])).is_none());
    }
    use super::*;
    use crate::askapi::ToolCall;
    use crate::memory::Memory;
    use crate::tools::EndpointSnapshot;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicUsize;

    // ---------- Ersatz für ASK ----------

    type Reply = Box<dyn FnOnce(&ChatRequest) -> Result<Completion> + Send>;

    /// ASK-Ersatz: liefert nacheinander die vorbereiteten Antworten (danach «Fertig.») und merkt sich die Anfragen
    struct Fake {
        answers: Mutex<VecDeque<Reply>>,
        requests: Mutex<Vec<ChatRequest>>,
    }

    impl Fake {
        fn new(replies: Vec<Reply>) -> Arc<Fake> {
            Arc::new(Fake { answers: Mutex::new(replies.into()), requests: Mutex::new(Vec::new()) })
        }
        fn requests(&self) -> Vec<ChatRequest> {
            self.requests.lock().clone()
        }
    }

    #[async_trait]
    impl ChatApi for Fake {
        async fn chat(&self, req: ChatRequest) -> Result<Completion> {
            self.requests.lock().push(req.clone());
            let next = self.answers.lock().pop_front();
            match next {
                Some(f) => f(&req),
                None => Ok(completion("Fertig.", vec![])),
            }
        }
    }

    fn completion(content: &str, calls: Vec<ToolCall>) -> Completion {
        let mut c = Completion::new();
        c.content = content.to_string();
        c.calls = calls;
        c.finish = "stop".into();
        c
    }

    fn text(s: &str) -> Reply {
        let s = s.to_string();
        Box::new(move |_| Ok(completion(&s, vec![])))
    }

    fn calls(list: Vec<ToolCall>) -> Reply {
        Box::new(move |_| Ok(completion("", list)))
    }

    fn fail(err: anyhow::Error) -> Reply {
        Box::new(move |_| Err(err))
    }

    fn status(message: &str, status: u16) -> anyhow::Error {
        AskError::new(message).with_status(status).into()
    }

    fn call(id: &str, name: &str, args: Value) -> ToolCall {
        ToolCall { id: id.into(), name: name.into(), arguments: args.to_string() }
    }

    fn del(id: &str) -> ToolCall {
        call(id, "run_command", json!({ "command": "del alt.txt" }))
    }

    struct T {
        agent: Arc<Agent>,
        ask: Arc<Fake>,
        events: Arc<Mutex<Vec<Value>>>,
        folder: tempfile::TempDir,
        model: Arc<Mutex<String>>,
    }

    impl T {
        fn folder(&self) -> String {
            self.folder.path().to_string_lossy().to_string()
        }
        fn events_of(&self, kind: &str) -> Vec<Value> {
            self.events.lock().iter().filter(|e| e["type"] == kind).cloned().collect()
        }
        fn roles(&self) -> Vec<String> {
            self.agent.transcript().as_array().unwrap().iter().map(|i| js_string(&i["role"])).collect()
        }
        fn saved(&self) -> Value {
            let file = self.folder.path().join(".ask").join("chats").join(format!("{}.json", self.agent.chat_id()));
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap()
        }
        async fn send(&self, text: &str) {
            self.agent.run_turn(text, vec![], json!({})).await;
        }
    }

    fn make(ask: Arc<Fake>, tweak: impl FnOnce(&mut AgentOptions)) -> T {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().to_string_lossy().to_string();
        let model = Arc::new(Mutex::new("m".to_string()));
        let m2 = model.clone();
        let mut o = AgentOptions::new(ask.clone(), move || path.clone(), move || m2.lock().clone());
        o.terminal = Some(Arc::new(EndpointSnapshot { state: "gestoppt".into(), port: 0, key: String::new() })); // Befehle scheitern, das genügt hier
        o.get_system_prompt = Arc::new(|_| "sys".into());
        tweak(&mut o);
        let agent = Agent::new(o);
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        agent.events.on("event", move |e| ev.lock().push(e.clone()));
        T { agent, ask, events, folder, model }
    }

    // ---------- Nebenfrage mit /btw (E86) ----------

    #[tokio::test]
    async fn nebenfrage_ohne_werkzeuge_mit_dem_chat_und_nicht_im_verlauf_des_auftrags() {
        let t = make(Fake::new(vec![text("Antwort eins"), text("Das ist die Nebenantwort."), text("Antwort zwei")]), |_| {});
        assert_eq!(t.agent.btw("Frage vor allem")["ok"], false, "ohne Eingabe im Chat keine Nebenfrage");
        t.send("Schreib einen Bericht").await;
        assert_eq!(t.agent.btw("   ")["ok"], false, "ohne Frage");
        let r = t.agent.btw(" Was heisst eigentlich Bericht? ");
        assert_eq!(r["ok"], true, "{r}");
        for _ in 0..300 {
            if !t.events_of("btw-end").is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let start = &t.events_of("btw-start")[0];
        assert_eq!(start["question"], "Was heisst eigentlich Bericht?");
        let end = &t.events_of("btw-end")[0];
        assert_eq!((end["id"].clone(), end["text"].clone(), end["error"].clone()), (start["id"].clone(), json!("Das ist die Nebenantwort."), json!("")));
        let req = &t.ask.requests()[1];
        assert!(req.tools.is_none(), "keine Werkzeuge");
        assert!(js_string(&req.messages[0]["content"]).contains("# Nebenfrage"));
        assert!(req.messages.iter().any(|m| js_string(&m["content"]) == "Antwort eins"), "kennt den Chat");
        let last = js_string(&req.messages.last().unwrap()["content"]);
        assert!(last.ends_with("[Nebenfrage, der Auftrag läuft weiter] Was heisst eigentlich Bericht?") && last.starts_with('['), "{last}");
        // Im Chat gespeichert und angezeigt, aber nicht im Verlauf für den Auftrag
        assert_eq!(t.roles(), ["user", "assistant", "btw"]);
        assert_eq!(t.agent.transcript()[2]["answer"], "Das ist die Nebenantwort.");
        assert!(arr(&t.saved()["messages"]).iter().any(|m| m["role"] == "btw"));
        t.send("weiter").await;
        let next = &t.ask.requests()[2];
        assert!(!next.messages.iter().any(|m| js_string(&m["content"]).contains("Nebenantwort") || js_string(&m["content"]).contains("Nebenfrage")));
    }

    #[test]
    fn nebenfrage_waehrend_eines_schritts_der_noch_laeuft() {
        let t = make(Fake::new(vec![]), |_| {});
        t.agent.push(json!({ "role": "user", "content": "Mach ein Dokument", "time": 1 }));
        t.agent.push(json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "write_file", "arguments": "{}" } }] }));
        let messages = t.agent.btw_messages("Wie lange dauert das?", 0);
        let tool = messages.iter().find(|m| m["role"] == "tool").unwrap();
        assert_eq!(tool["content"], BTW_RUNNING, "nicht «abgebrochen»: der Schritt läuft noch");
        assert_eq!(messages.last().unwrap()["content"], "[Nebenfrage, der Auftrag läuft weiter] Wie lange dauert das?");
    }

    // ---------- agent.test.js ----------

    #[tokio::test]
    async fn hinweise_und_fehler_bleiben_im_chat_gehen_aber_nicht_an_das_modell() {
        let t = make(Fake::new(vec![fail(status("ASK: kaputt", 500)), text("Hallo")]), |_| {});
        t.send("eins").await;
        assert_eq!(t.roles(), ["user", "error"]);
        t.send("zwei").await;
        let roles = |i: usize| t.ask.requests()[i].messages.iter().map(|m| js_string(&m["role"])).collect::<Vec<_>>();
        assert_eq!(roles(1), ["system", "user", "user"]);
        assert_eq!(t.roles(), ["user", "error", "user", "assistant"]);
        let saved = t.saved();
        assert_eq!(saved["messages"][1]["role"], "note");
        assert!(!t.folder.path().join(".ask/chats").join(format!("{}.json.tmp", t.agent.chat_id())).exists());
    }

    #[tokio::test]
    async fn gestoppte_antwort_was_schon_da_war_bleibt_im_chat() {
        let partial: Reply = Box::new(|req| {
            (req.on_delta.as_ref().unwrap())("text", "Ich habe angefangen");
            let mut c = Completion::new();
            c.content = "Ich habe angefangen".into();
            req.signal.as_ref().unwrap().cancel();
            let mut e = AskError::abort();
            e.partial = Some(Box::new(c));
            Err(e.into())
        });
        let t = make(Fake::new(vec![partial]), |_| {});
        t.send("los").await;
        let items = t.agent.transcript();
        assert_eq!(t.roles(), ["user", "assistant", "notice"]);
        assert_eq!(items[1]["content"], "Ich habe angefangen");
        assert_eq!(items[1]["stopped"], true);
        assert_eq!(items[2]["text"], "Gestoppt.");
    }

    #[tokio::test]
    async fn verlauf_zu_lang_einmal_staerker_gekuerzt_nochmals_sonst_klare_meldung() {
        let long = || fail(status("ASK: This model's maximum context length is 8192 tokens.", 400));
        let limits = Arc::new(Mutex::new(Vec::new()));
        let l2 = limits.clone();
        let t = make(Fake::new(vec![long(), text("jetzt geht es"), long()]), |o| o.on_context_limit = Arc::new(move |m, l| l2.lock().push((m.to_string(), l))));
        t.send("eins").await;
        assert_eq!(t.ask.requests().len(), 2);
        assert!(t.events_of("notice").iter().any(|e| js_string(&e["text"]).contains("zu lang geworden")));
        assert_eq!(t.agent.transcript().as_array().unwrap().last().unwrap()["content"], "jetzt geht es");
        assert_eq!(*limits.lock(), [("m".to_string(), 8192)]);
        t.send("zwei").await;
        // Dritte Anfrage scheitert, die vierte ist der Versuch zusammenzufassen (die Attrappe antwortet zu kurz dafür)
        assert_eq!(t.ask.requests().len(), 4);
        assert!(t.events_of("error").iter().any(|e| js_string(&e["text"]).contains("zu lang für das Modell, auch gekürzt")));
        assert!(!t.agent.text_mode(), "eine Meldung zur Länge schaltet nicht in den Textmodus");
    }

    #[tokio::test]
    async fn textmodus_gilt_fuer_das_modell_das_die_werkzeugliste_abgelehnt_hat() {
        let t = make(Fake::new(vec![fail(status("ASK: tools are not supported", 400)), text("ok"), text("ok")]), |_| {});
        t.send("eins").await;
        assert!(t.agent.text_mode());
        assert!(t.ask.requests()[1].tools.is_none());
        *t.model.lock() = "modell-b".into();
        assert!(!t.agent.text_mode());
        t.send("zwei").await;
        assert!(t.ask.requests()[2].tools.is_some());
    }

    #[tokio::test]
    async fn abgebrochene_werkzeugaufrufe_gelten_als_fehlgeschlagen() {
        let stop: Reply = Box::new(|req| {
            req.signal.as_ref().unwrap().cancel();
            Ok(completion("", vec![call("c1", "list_files", json!({ "path": "." }))]))
        });
        let t = make(Fake::new(vec![stop]), |_| {});
        t.send("los").await;
        let items = t.agent.transcript();
        let tool = items.as_array().unwrap().iter().find(|i| i["role"] == "tool").unwrap();
        assert_eq!(tool["ok"], false);
        assert_eq!(tool["summary"], "abgebrochen");
    }

    #[tokio::test]
    async fn letzte_eingabe_zuruecknehmen_chat_endet_davor_text_und_anhaenge_kommen_zurueck() {
        let t = make(Fake::new(vec![text("Antwort eins"), text("Antwort zwei")]), |_| {});
        t.send("eins").await;
        let att = json!({ "path": format!(".ask/anhaenge/{}/bild.png", t.agent.chat_id()), "name": "bild.png", "size": 3, "image": true });
        t.agent.run_turn("zwei", vec![att], json!({})).await;
        let r = t.agent.rewind().await;
        assert_eq!(r["ok"], true);
        assert_eq!(r["text"], "zwei");
        assert_eq!(r["attachments"][0]["name"], "bild.png");
        let contents: Vec<Value> = t.agent.transcript().as_array().unwrap().iter().map(|i| i["content"].clone()).collect();
        assert_eq!(contents, [json!("eins"), json!("Antwort eins")]);
        assert_eq!(t.saved()["messages"].as_array().unwrap().len(), 2);
        assert_eq!(t.agent.rewind().await["text"], "eins");
        assert_eq!(t.agent.chat_title(), "");
        assert!(!t.folder.path().join(".ask/chats").join(format!("{}.json", t.agent.chat_id())).exists(), "leerer Chat wird nicht gespeichert");
        assert_eq!(t.agent.rewind().await["ok"], false);
    }

    #[tokio::test]
    async fn umbenennen_und_anheften_auch_fuer_einen_nicht_geoeffneten_chat_kontext_aus_den_tokenzahlen() {
        let t = make(Fake::new(vec![text("a"), text("b")]), |_| {});
        t.send("erster Chat").await;
        let first = t.agent.chat_id();
        t.agent.new_chat();
        tokio::time::sleep(Duration::from_millis(10)).await; // damit die Reihenfolge nach Zeit eindeutig ist
        t.send("zweiter Chat").await;
        let folder = t.folder();
        let titles = |l: Value| l.as_array().unwrap().iter().map(|c| js_string(&c["title"])).collect::<Vec<_>>();
        assert_eq!(titles(t.agent.list(&folder)), ["zweiter Chat", "erster Chat"]);
        assert!(t.agent.pin(&folder, &first, true));
        assert!(t.agent.rename(&folder, &first, "  Bericht   Proxmox "));
        let l = t.agent.list(&folder);
        let pairs: Vec<(String, bool)> = l.as_array().unwrap().iter().map(|c| (js_string(&c["title"]), c["pinned"] == true)).collect();
        assert_eq!(pairs, [("Bericht Proxmox".to_string(), true), ("zweiter Chat".to_string(), false)]);
        assert!(t.agent.rename(&folder, &t.agent.chat_id(), "offen"));
        assert_eq!(t.agent.state()["title"], "offen");
        assert!(!t.agent.rename(&folder, &first, "   "));
        assert!(!t.agent.rename(&folder, "../x", "y"));
        let context = t.agent.state()["context"].clone();
        assert!(context["tokens"].as_i64().unwrap() > 0 && context["estimated"] == true && context["limit"] == 0);
        let own = t.agent.with_chat(|c| {
            let last = c["messages"].as_array_mut().unwrap().last_mut().unwrap();
            last["stats"]["promptTokens"] = json!(1200);
            last["stats"]["tokens"].as_i64().unwrap()
        });
        assert_eq!(t.agent.state()["context"], json!({ "tokens": 1200 + own, "estimated": false, "limit": 0 }));
    }

    // ---------- rueckfrage.test.js ----------

    /// Antwortet auf Rückfragen der Reihe nach mit den vorgegebenen Antworten
    fn answer_approvals(t: &T, answers: &[&str]) {
        let answers: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(answers.iter().map(|s| s.to_string()).collect()));
        let agent = t.agent.clone();
        t.agent.events.on("event", move |ev| {
            if ev["type"] == "approval" {
                let a = answers.lock().pop_front().unwrap_or_else(|| "no".into());
                agent.answer_approval(&js_string(&ev["id"]), &a);
            }
        });
    }

    fn heikel(o: &mut AgentOptions) {
        o.get_approval_mode = Arc::new(|| "heikel".into());
    }

    #[tokio::test]
    async fn abgelehnter_loeschbefehl_wird_nicht_ausgefuehrt_das_modell_erfaehrt_es() {
        let t = make(Fake::new(vec![calls(vec![del("a")])]), heikel);
        answer_approvals(&t, &["no"]);
        t.send("Räum auf").await;
        let approval = t.events_of("approval").into_iter().next().expect("Rückfrage gestellt");
        assert_eq!(approval["kind"], "loeschen");
        assert_eq!(t.events_of("approval-end")[0], json!({ "type": "approval-end", "id": approval["id"], "answer": "no" }));
        let end = &t.events_of("tool-end")[0];
        assert_eq!(end["ok"], false);
        assert_eq!(end["summary"], "abgelehnt");
        let tool = t.ask.requests()[1].messages.iter().find(|m| m["role"] == "tool").cloned().unwrap();
        assert!(js_string(&tool["content"]).contains("abgelehnt"));
    }

    #[tokio::test]
    async fn immer_in_diesem_chat_fragt_danach_nicht_mehr_ein_neuer_chat_wieder() {
        let t = make(Fake::new(vec![calls(vec![del("a")]), calls(vec![del("b")])]), heikel);
        answer_approvals(&t, &["always"]);
        t.send("Räum auf").await;
        assert_eq!(t.events_of("approval").len(), 1);
        let ends = t.events_of("tool-end");
        assert_eq!(ends.len(), 2);
        assert!(ends.iter().all(|e| e["summary"] != "abgelehnt"), "beide ausgeführt (das Terminal läuft hier nicht)");
        t.agent.new_chat();
        assert_eq!(t.agent.allowed_count(), 0);
    }

    #[tokio::test]
    async fn stoppen_waehrend_der_rueckfrage_gilt_als_nein() {
        let t = make(Fake::new(vec![calls(vec![del("a")])]), heikel);
        let agent = t.agent.clone();
        t.agent.events.on("event", move |ev| {
            if ev["type"] == "approval" {
                agent.stop();
            }
        });
        t.send("Räum auf").await;
        assert_eq!(t.events_of("approval-end")[0]["answer"], "no");
        assert!(t.events_of("notice").iter().any(|e| e["text"] == "Gestoppt."));
        assert!(!t.agent.running());
        assert_eq!(t.agent.pending_approvals(), 0);
    }

    #[tokio::test]
    async fn stufe_aus_fragt_nie() {
        let t = make(Fake::new(vec![calls(vec![del("a")])]), |_| {});
        t.send("Räum auf").await;
        assert!(t.events_of("approval").is_empty());
    }

    fn answer_questions(t: &T, answer: &'static str) {
        let agent = t.agent.clone();
        t.agent.events.on("event", move |ev| {
            if ev["type"] == "question" {
                agent.answer_question(&js_string(&ev["id"]), answer);
            }
        });
    }

    #[tokio::test]
    async fn frage_stellen_frage_mit_antworten_die_antwort_geht_an_das_modell() {
        let args = json!({ "frage": "Welche Datei?", "optionen": ["a.csv", "b.csv", ""] });
        let t = make(Fake::new(vec![calls(vec![call("q1", "frage_stellen", args)])]), heikel);
        answer_questions(&t, "b.csv");
        t.send("Werte aus").await;
        let q = &t.events_of("question")[0];
        assert_eq!(q["frage"], "Welche Datei?");
        assert_eq!(q["optionen"], json!(["a.csv", "b.csv"]));
        assert_eq!(t.events_of("question-end")[0]["answer"], "b.csv");
        let requests = t.ask.requests();
        let tool = requests[1].messages.iter().find(|m| m["role"] == "tool").unwrap();
        assert_eq!(tool["content"], "Antwort der Person: b.csv");
        assert!(requests[0].tools.as_ref().unwrap().iter().any(|s| s["function"]["name"] == "frage_stellen"));
        let items = t.agent.transcript();
        let c = &items.as_array().unwrap().iter().find(|m| m["role"] == "assistant").unwrap()["calls"][0];
        assert_eq!([c["label"].clone(), c["detail"].clone(), c["optionen"].clone()], [json!("Frage"), json!("Welche Datei?"), json!(["a.csv", "b.csv"])]);
    }

    #[tokio::test]
    async fn frage_stellen_stoppen_beendet_die_frage_ohne_antwort() {
        let t = make(Fake::new(vec![calls(vec![call("q1", "frage_stellen", json!({ "frage": "Weiter?" }))])]), heikel);
        let agent = t.agent.clone();
        t.agent.events.on("event", move |ev| {
            if ev["type"] == "question" {
                agent.stop();
            }
        });
        t.send("x").await;
        assert_eq!(t.events_of("question-end")[0]["answer"], "");
        assert_eq!(t.events_of("tool-end")[0]["summary"], "nicht beantwortet");
        assert_eq!(t.agent.pending_questions(), 0);
    }

    #[tokio::test]
    async fn argumente_null_bringen_den_chat_nicht_durcheinander_jeder_aufruf_bekommt_ein_ergebnis() {
        let list = vec![ToolCall { id: "n".into(), name: "run_command".into(), arguments: "null".into() }, ToolCall { id: "q".into(), name: "frage_stellen".into(), arguments: "[1]".into() }];
        let t = make(Fake::new(vec![calls(list)]), heikel);
        t.send("Los").await;
        let requests = t.ask.requests();
        let ids: Vec<String> = requests[1].messages.iter().filter(|m| m["role"] == "tool").map(|m| js_string(&m["tool_call_id"])).collect();
        assert_eq!(ids, ["n", "q"]);
        let args: Vec<String> = requests[1].messages.iter().find(|m| m.get("tool_calls").is_some()).unwrap()["tool_calls"].as_array().unwrap().iter().map(|c| js_string(&c["function"]["arguments"])).collect();
        assert_eq!(args, ["{}", "{}"]);
    }

    fn turn(outputs: &[&str]) -> Vec<Value> {
        let mut out = vec![json!({ "role": "user", "content": "x" })];
        for (i, o) in outputs.iter().enumerate() {
            out.push(json!({ "role": "assistant", "tool_calls": [{ "id": format!("c{i}"), "function": { "name": "command_status", "arguments": "{\"id\":\"1\"}" } }] }));
            out.push(json!({ "role": "tool", "tool_call_id": format!("c{i}"), "content": o }));
        }
        out
    }

    #[test]
    fn gleicher_aufruf_mit_neuer_ausgabe_ist_keine_schleife_gleicher_aufruf_mit_gleichem_ergebnis_schon() {
        assert_eq!(loop_reason(&turn(&["a", "ab", "abc"])), "");
        assert!(loop_reason(&turn(&["a", "a", "a"])).contains("denselben Schritt"));
    }

    /// Echte Sicherung eines Ordners, die ein Test über `get_snapshots` hereinreicht
    type SnapshotSlot = Arc<Mutex<Option<Arc<Snapshots>>>>;

    #[tokio::test]
    async fn waehrend_eingabe_aendern_startet_kein_auftrag_und_kein_zweites_zuruecknehmen() {
        let slot: SnapshotSlot = Arc::new(Mutex::new(None));
        let (release_tx, release_rx) = oneshot::channel::<bool>();
        let gate = Arc::new(Mutex::new(Some(release_rx)));
        let s2 = slot.clone();
        let t = make(Fake::new(vec![text("Ok.")]), |o| {
            o.get_snapshots = Arc::new(move || s2.lock().clone());
            o.confirm = Some(Arc::new(move |_m, _d, _o| {
                let rx = gate.lock().take();
                Box::pin(async move { Confirmed { ok: match rx { Some(rx) => rx.await.unwrap_or(false), None => false }, checked: false } })
            }));
        });
        t.send("Erste Eingabe").await;
        let snaps = Arc::new(Snapshots::new(t.folder.path(), None));
        let stand = snaps.take("x").await.unwrap();
        std::fs::write(t.folder.path().join("a.txt"), "neu").unwrap();
        t.agent.with_chat(|c| c["messages"][0]["snapshot"] = json!(stand.id));
        *slot.lock() = Some(snaps);
        let a2 = t.agent.clone();
        let first = tokio::spawn(async move { a2.rewind().await });
        for _ in 0..50 {
            if t.agent.undoing() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(t.agent.undoing());
        assert_eq!(t.agent.rewind().await["ok"], false);
        t.send("Dazwischen").await;
        assert_eq!(t.ask.requests().len(), 1, "kein neuer Auftrag während des Zurücknehmens");
        release_tx.send(true).unwrap();
        assert_eq!(first.await.unwrap()["ok"], true);
        assert!(!t.agent.undoing());
    }

    #[tokio::test]
    async fn arbeitet_ein_anderer_chat_im_selben_ordner_warnt_die_app() {
        let t = make(Fake::new(vec![text("Ok.")]), |o| o.get_siblings = Arc::new(|| vec!["Bericht schreiben".to_string()]));
        t.send("Los").await;
        assert!(t.events_of("notice").iter().any(|e| js_string(&e["text"]).contains("«Bericht schreiben»")));
    }

    // ---------- modi.test.js ----------

    fn auto(o: &mut AgentOptions) {
        o.get_mode = Arc::new(|| "auto".into());
        o.confirm = Some(fixed_confirm(Confirmed { ok: true, checked: false }));
    }

    fn tool_names(req: &ChatRequest) -> Vec<String> {
        req.tools.as_ref().unwrap().iter().map(|s| js_string(&s["function"]["name"])).collect()
    }

    #[tokio::test]
    async fn planmodus_nur_lesewerkzeuge_schreiben_wird_abgewiesen_plan_vorlegen_beendet_den_auftrag() {
        let ask = Fake::new(vec![
            calls(vec![call("a", "write_file", json!({ "path": "x.txt", "content": "hallo" }))]),
            calls(vec![call("b", "plan_vorlegen", json!({ "plan": "# Website\n1. index.html anlegen" }))]),
            text("sollte nicht mehr kommen"),
        ]);
        let t = make(ask, |o| o.get_mode = Arc::new(|| "plan".into()));
        t.send("Mach mir eine Website").await;
        let requests = t.ask.requests();
        let names = tool_names(&requests[0]);
        assert!(names.contains(&"read_file".to_string()) && names.contains(&"plan_vorlegen".to_string()));
        assert!(!names.contains(&"write_file".to_string()) && !names.contains(&"run_command".to_string()), "nichts, was ändert");
        assert!(js_string(&requests[0].messages[0]["content"]).contains("Planmodus"));
        let ends = t.events_of("tool-end");
        assert_eq!(ends[0]["ok"], false);
        assert!(js_string(&ends[0]["summary"]).contains("Planmodus"));
        assert_eq!(ends[1]["plan"], "# Website\n1. index.html anlegen");
        assert_eq!(requests.len(), 2, "nach dem Plan keine weitere Anfrage");
        let items = t.agent.transcript();
        assert!(items.as_array().unwrap().iter().any(|i| i["role"] == "tool" && truthy(&i["plan"])), "Plan bleibt im gespeicherten Chat");
    }

    #[tokio::test]
    async fn chat_ohne_projekt_nur_gespraech_web_bilder_und_lesen_schreiben_wird_abgewiesen() {
        let ask = Fake::new(vec![
            calls(vec![call("a", "write_file", json!({ "path": "x.txt", "content": "hallo" })), call("b", "run_command", json!({ "command": "del x.txt" }))]),
            text("Das geht nur in einem Projekt."),
        ]);
        // Auch im Planmodus: ohne Projekt gibt es keinen Plan, der Modus zählt dort nicht
        let t = make(ask, |o| { o.free = true; o.get_mode = Arc::new(|| "plan".into()); });
        t.send("Schreib mir eine Datei").await;
        let requests = t.ask.requests();
        let names = tool_names(&requests[0]);
        for allowed in ["read_file", "web_search", "web_fetch", "frage_stellen"] {
            assert!(names.contains(&allowed.to_string()), "{allowed} fehlt");
        }
        for denied in ["write_file", "edit_file", "run_command", "server_starten", "github_push", "browser", "hilfsagent", "plan_vorlegen", "skill_erstellen", "mod_erstellen"] {
            assert!(!names.contains(&denied.to_string()), "{denied} darf ohne Projekt nicht angeboten werden");
        }
        assert!(names.iter().all(|n| FREE_ALLOWED.contains(&n.as_str())), "nur Werkzeuge aus FREE_ALLOWED: {names:?}");
        let system = js_string(&requests[0].messages[0]["content"]);
        assert!(system.contains("Chat ohne Projekt") && !system.contains("Planmodus"));
        let ends = t.events_of("tool-end");
        assert_eq!(ends.len(), 2);
        assert!(ends.iter().all(|e| e["ok"] == false && js_string(&e["summary"]).contains("ohne Projekt")), "beide Aufrufe abgewiesen: {ends:?}");
        assert!(!t.folder.path().join("x.txt").exists());
        assert_eq!(t.agent.messages().last().unwrap()["content"], "Das geht nur in einem Projekt.");
    }

    #[tokio::test]
    async fn offene_frage_steht_bei_den_offenen_ereignissen_bis_sie_beantwortet_ist() {
        let ask = Fake::new(vec![calls(vec![call("q1", "frage_stellen", json!({ "frage": "Welche Farbe?", "optionen": ["Rot", "Blau"] }))]), text("Blau also.")]);
        let t = make(ask, |_| {});
        t.agent.send("Mal etwas", vec![], json!({}));
        for _ in 0..400 {
            if t.agent.pending_questions() > 0 { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        // Wer jetzt erst in den Chat wechselt, bekommt die Frage mit dem Verlauf
        let open = t.agent.pending_events();
        assert_eq!(open.as_array().unwrap().len(), 1);
        assert_eq!(open[0]["type"], "question");
        assert_eq!(open[0]["id"], "q1");
        assert_eq!(open[0]["frage"], "Welche Farbe?");
        assert!(t.agent.answer_question("q1", "Blau"));
        for _ in 0..400 {
            if !t.agent.running() { break; }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        assert!(!t.agent.running());
        assert!(t.agent.pending_events().as_array().unwrap().is_empty(), "nach der Antwort ist nichts mehr offen");
    }

    #[tokio::test]
    async fn beim_oeffnen_des_projekts_der_zuletzt_benutzte_chat_nicht_der_angeheftete() {
        let t = make(Fake::new(vec![text("eins"), text("zwei")]), |_| {});
        t.send("alter Chat").await;
        let old = t.agent.chat_id();
        let folder = t.folder.path().to_string_lossy().to_string();
        assert!(t.agent.pin(&folder, &old, true));
        tokio::time::sleep(Duration::from_millis(15)).await;
        assert!(t.agent.new_chat());
        t.send("neuer Chat").await;
        let new = t.agent.chat_id();
        assert_ne!(old, new);
        // Die Liste führt den angehefteten zuerst, geöffnet wird aber der, an dem man zuletzt war
        assert_eq!(t.agent.list_current()[0]["id"], old);
        t.agent.open(&old);
        t.agent.open_latest(None);
        assert_eq!(t.agent.chat_id(), new);
    }

    #[test]
    fn aufrufe_ohne_ergebnis_bekommen_eines_sonst_lehnt_ask_den_chat_ab() {
        let tc = |id: &str| json!({ "id": id, "type": "function", "function": { "name": "read_file", "arguments": "{}" } });
        // App während des zweiten Schritts beendet: «b» hat ein Ergebnis, «c» und «d» nicht
        let messages = vec![
            json!({ "role": "user", "content": "los" }),
            json!({ "role": "assistant", "content": "", "tool_calls": [tc("a")] }),
            json!({ "role": "tool", "tool_call_id": "a", "content": "A" }),
            json!({ "role": "assistant", "content": "", "tool_calls": [tc("b"), tc("c")] }),
            json!({ "role": "tool", "tool_call_id": "b", "content": "B" }),
            json!({ "role": "user", "content": "weiter" }),
            json!({ "role": "assistant", "content": "", "tool_calls": [tc("d")] }),
        ];
        let out = api_messages("sys", &messages, false, None, &NORMAL);
        let shape: Vec<String> = out.iter().map(|m| format!("{}:{}", js_string(&m["role"]), m["tool_call_id"].as_str().unwrap_or(""))).collect();
        assert_eq!(shape, ["system:", "user:", "assistant:", "tool:a", "assistant:", "tool:b", "tool:c", "user:", "assistant:", "tool:d"]);
        assert!(js_string(&out[6]["content"]).contains("abgebrochen"));
        // Ein vollständiger Verlauf bleibt unverändert
        let whole = api_messages("sys", &messages[..3], false, None, &NORMAL);
        assert_eq!(whole.len(), 4);
    }

    #[tokio::test]
    async fn chat_verschieben_mit_anhaengen_ohne_verweise_auf_die_sicherung_des_alten_ordners() {
        let ask = Fake::new(vec![calls(vec![call("a", "list_files", json!({ "path": "." }))]), text("Fertig in A.")]);
        let t = make(ask, |_| {});
        t.send("Auftrag in A").await;
        let id = t.agent.chat_id();
        let from = t.folder.path().to_string_lossy().to_string();
        let to_dir = tempfile::tempdir().unwrap();
        let to = to_dir.path().to_string_lossy().to_string();
        // Anhang des Chats und ein Verweis auf eine Sicherung, wie ihn ein Auftrag mit «Rückgängig» hinterlässt
        let att = t.folder.path().join(".ask/anhaenge").join(&id);
        std::fs::create_dir_all(&att).unwrap();
        std::fs::write(att.join("notiz.txt"), "hallo").unwrap();
        let file = t.folder.path().join(".ask/chats").join(format!("{id}.json"));
        let mut chat: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        chat["messages"][0]["snapshot"] = json!("s1");
        chat["messages"][0]["changes"] = json!({ "added": ["x.txt"], "changed": [], "removed": [] });
        std::fs::write(&file, chat.to_string()).unwrap();

        assert!(Agent::move_chat(&from, &from, &id).is_err(), "derselbe Ordner");
        assert!(Agent::move_chat(&from, &to, "../boese").is_err(), "nur gültige Kennungen");
        Agent::move_chat(&from, &to, &id).unwrap();
        assert!(!file.exists() && !att.exists(), "am alten Ort ist nichts mehr");
        let moved: Value = serde_json::from_str(&std::fs::read_to_string(to_dir.path().join(".ask/chats").join(format!("{id}.json"))).unwrap()).unwrap();
        assert_eq!(moved["messages"][0]["content"], "Auftrag in A");
        assert!(moved["messages"][0].get("snapshot").is_none() && moved["messages"][0].get("changes").is_none(), "kein «Rückgängig» für den alten Ordner");
        assert_eq!(std::fs::read_to_string(to_dir.path().join(".ask/anhaenge").join(&id).join("notiz.txt")).unwrap(), "hallo");
        assert!(Agent::move_chat(&from, &to, &id).is_err(), "nochmals: den Chat gibt es am alten Ort nicht mehr");

        // Export: Titel, Eingabe, Antwort und der Arbeitsschritt kurz
        let md = Agent::export_markdown(&to, &id, "Projekt «B»").unwrap();
        assert!(md.starts_with("# Auftrag in A\n"), "{md}");
        assert!(md.contains("Aus ASK, Projekt «B».") && md.contains("## Du") && md.contains("Auftrag in A") && md.contains("## ASK") && md.contains("Fertig in A."), "{md}");
        assert!(md.contains("- Ordner"), "Arbeitsschritt als Zeile: {md}");
        assert!(Agent::export_markdown(&to, "gibt-es-nicht", "x").is_none());
    }

    // ---------- Zusammenfassen des Verlaufs (E78) ----------

    const SUMMARY: &str = "Ziel: eine Webseite für den Verein. Erledigt: index.html angelegt. Offen: Farben.";

    #[tokio::test]
    async fn zu_langer_chat_wird_zusammengefasst_und_der_auftrag_geht_weiter() {
        let too_long = || fail(status("This model's maximum context length is 8192 tokens", 400));
        // Auftrag eins; Auftrag zwei: zu lang, auch stark gekürzt zu lang, dann Zusammenfassung, dann die Antwort
        let t = make(Fake::new(vec![text("Antwort eins"), too_long(), too_long(), text(SUMMARY), text("Weiter geht es.")]), |_| {});
        t.send("eins").await;
        t.send("zwei").await;
        assert!(t.events_of("error").is_empty(), "kein Abbruch: {:?}", t.events_of("error"));
        assert_eq!(t.agent.messages().last().unwrap()["content"], "Weiter geht es.");
        let requests = t.ask.requests();
        assert_eq!(requests.len(), 5);
        // Die Anfrage zum Zusammenfassen: eigener Systemprompt, der alte Verlauf als Text, keine Werkzeuge
        assert!(js_string(&requests[3].messages[0]["content"]).starts_with("Du fasst einen langen Chat"));
        let old = js_string(&requests[3].messages[1]["content"]);
        assert!(old.contains("Person: eins") && old.contains("ASK: Antwort eins") && !old.contains("zwei"), "{old}");
        assert!(requests[3].tools.is_none());
        // Danach: Systemprompt, Zusammenfassung, die neue Eingabe; der alte Verlauf geht nicht mehr mit
        let sent: Vec<String> = requests[4].messages.iter().map(|m| js_string(&m["content"])).collect();
        assert_eq!(sent.len(), 3, "{sent:?}");
        assert!(sent[1].contains("Zusammenfassung des bisherigen Chats") && sent[1].contains(SUMMARY));
        assert_eq!(ohne_zeit(&sent[2]), "zwei");
        // Gespeichert an der Eingabe, mit der der erhaltene Teil beginnt; nichts ist gelöscht
        let saved = t.saved();
        assert_eq!(saved["messages"][0]["content"], "eins");
        assert_eq!(saved["messages"][2]["summaryBefore"]["content"], SUMMARY);
        // Wer den Chat öffnet, sieht den ganzen Verlauf und an der Stelle einen Hinweis
        let items = t.agent.transcript();
        let roles: Vec<String> = items.as_array().unwrap().iter().map(|i| js_string(&i["role"])).collect();
        let at = roles.iter().rposition(|r| r == "user").unwrap();
        assert_eq!(items[at]["content"], "zwei");
        assert_eq!(roles[at - 1], "notice");
        assert!(js_string(&items[at - 1]["text"]).contains("Zusammenfassung"));
        assert!(roles.iter().filter(|r| *r == "user").count() == 2);
    }

    #[tokio::test]
    async fn kontext_bis_auf_die_reserve_voll_fasst_vor_dem_auftrag_zusammen() {
        let long = "Wort ".repeat(900); // rund 1250 Token geschätzt
        let t = make(Fake::new(vec![text(&long), text(SUMMARY), text("Antwort zwei")]), |o| {
            o.get_context_limit = Arc::new(|_| 1500);
            o.get_compact_reserve = Arc::new(|| 400);
        });
        t.send("eins").await;
        assert_eq!(t.ask.requests().len(), 1, "noch Platz: nichts zusammengefasst");
        t.send("zwei").await;
        let requests = t.ask.requests();
        assert_eq!(requests.len(), 3, "Zusammenfassung vor der Antwort");
        assert!(js_string(&requests[1].messages[0]["content"]).starts_with("Du fasst einen langen Chat"));
        let sent: Vec<String> = requests[2].messages.iter().map(|m| js_string(&m["content"])).collect();
        assert!(sent.len() == 3 && sent[1].contains(SUMMARY) && ohne_zeit(&sent[2]) == "zwei", "{sent:?}");
        assert_eq!(t.events_of("compacting").len(), 1);
        // Die Anzeige rechnet nach dem Zusammenfassen mit dem kurzen Verlauf
        let c = t.agent.context();
        assert!(c["tokens"].as_i64().unwrap() < 300, "{c}");
        // Der nächste Auftrag fasst nicht nochmals zusammen
        t.send("drei").await;
        assert_eq!(t.ask.requests().len(), 4);
    }

    #[tokio::test]
    async fn zusammenfassen_scheitert_der_chat_bleibt_wie_er_war() {
        let too_long = || fail(status("maximum context length is 8192 tokens", 400));
        let t = make(Fake::new(vec![text("Antwort eins"), too_long(), too_long(), fail(status("ASK: kaputt", 500))]), |_| {});
        t.send("eins").await;
        t.send("zwei").await;
        let errors = t.events_of("error");
        assert_eq!(errors.len(), 1);
        assert!(js_string(&errors[0]["text"]).contains("zu lang für das Modell"), "{errors:?}");
        assert!(summary_index(&t.agent.messages()).is_none());
        // Auf Befehl: kurzer Chat hat nichts zusammenzufassen
        let short = make(Fake::new(vec![text("Hallo")]), |_| {});
        short.send("eins").await;
        let r = short.agent.compact_now().await;
        assert_eq!(r["ok"], false);
        assert!(js_string(&r["reason"]).contains("noch kurz"));
        assert_eq!(short.ask.requests().len(), 1, "keine Anfrage an ASK");
    }

    #[tokio::test]
    async fn auf_befehl_zusammenfassen_und_weitere_zusammenfassung_baut_auf_der_alten_auf() {
        let t = make(Fake::new(vec![text("A1"), text("A2"), text(SUMMARY), text("A3"), text("Zweite Zusammenfassung mit allem Wichtigen.")]), |_| {});
        t.send("eins").await;
        t.send("zwei").await;
        assert_eq!(t.agent.compact_now().await["ok"], true);
        let m = t.agent.messages();
        assert_eq!(summary_index(&m), Some(2), "die letzte Eingabe bleibt wörtlich");
        assert!(t.events_of("notice").iter().any(|e| js_string(&e["text"]).contains("Zusammenfassung")));
        t.send("drei").await;
        assert_eq!(t.agent.compact_now().await["ok"], true);
        let requests = t.ask.requests();
        let again = js_string(&requests[4].messages[1]["content"]);
        assert!(again.contains("Frühere Zusammenfassung") && again.contains(SUMMARY) && again.contains("Person: zwei") && !again.contains("Person: eins"), "{again}");
        assert_eq!(summary_index(&t.agent.messages()), Some(4));
    }

    #[test]
    fn schnitt_nur_an_einer_eingabe_der_person_und_nur_mit_etwas_davor() {
        let user = |t: &str| json!({ "role": "user", "content": t, "time": 1 });
        let ai = |t: &str| json!({ "role": "assistant", "content": t });
        assert_eq!(compact_range(&[user("a")], 1), None, "ein einziger Auftrag: nichts davor");
        assert_eq!(compact_range(&[user("a"), ai("b"), user("c")], 1), Some((0, 2)));
        assert_eq!(compact_range(&[user("a"), ai("b"), user("c")], 2), None, "beide bleiben: nichts zusammenzufassen");
        // Anstösse im Ziel-Modus und Bilder einer Ansicht sind kein Anfang eines Auftrags
        let auto = json!({ "role": "user", "content": "weiter", "auto": true });
        let view = json!({ "role": "user", "content": "Ansicht", "note": "view" });
        let list = [user("a"), ai("b"), auto, ai("c"), view, user("d"), ai("e"), user("f")];
        assert_eq!(compact_range(&list, 1), Some((0, 7)));
        assert_eq!(compact_range(&list, 2), Some((0, 5)));
        // Nach einer Zusammenfassung zählt nur, was danach kommt
        let mut marked = list.to_vec();
        marked[5]["summaryBefore"] = json!({ "content": "kurz", "time": 2 });
        assert_eq!(compact_range(&marked, 1), Some((5, 7)));
        assert_eq!(compact_range(&marked, 2), None);
        let sent = api_messages("sys", &marked, false, None, &NORMAL);
        let texts: Vec<String> = sent.iter().map(|m| js_string(&m["content"])).collect();
        assert_eq!(texts.len(), 5, "{texts:?}");
        assert!(texts[1].contains("kurz") && ohne_zeit(&texts[2]) == "d" && ohne_zeit(&texts[4]) == "f");
    }

    /// Text einer Eingabe ohne den Zeitstempel davor (E85)
    fn ohne_zeit(text: &str) -> String {
        match text.strip_prefix('[').and_then(|r| r.split_once("] ")) {
            Some((_, rest)) => rest.to_string(),
            None => text.to_string(),
        }
    }

    #[test]
    fn zeitstempel_nur_vor_eingaben_der_person_und_datum_im_systemprompt() {
        use chrono::TimeZone;
        let t = chrono::Local.with_ymd_and_hms(2026, 11, 28, 14, 5, 0).single().unwrap();
        let ms = t.timestamp_millis();
        assert_eq!(time_stamp(ms), "[Sa 28.11.2026 14:05]");
        assert_eq!(time_stamp(0), "");
        assert_eq!(date_prompt(t), "\n\n# Datum\nHeute ist Samstag, 28. November 2026. Vor jeder Nachricht der Person steht in eckigen Klammern, wann sie sie geschickt hat (Ortszeit). Nutze das für «heute», «morgen», «letzte Woche» und Fristen; schreib diese Zeitangaben nicht selbst in deine Antworten.");
        let list = [
            json!({ "role": "user", "content": "Bis wann muss das fertig sein?", "time": ms }),
            json!({ "role": "user", "content": "weiter", "auto": true, "time": ms }),
            json!({ "role": "user", "content": "Rückgängig gemacht", "note": "undo", "time": ms }),
            json!({ "role": "user", "content": "Teilfrage", "helper": true, "time": ms }),
            json!({ "role": "user", "content": "ohne Zeit" }),
        ];
        let sent: Vec<String> = api_messages("sys", &list, false, None, &NORMAL).iter().map(|m| js_string(&m["content"])).collect();
        assert_eq!(sent[1], "[Sa 28.11.2026 14:05] Bis wann muss das fertig sein?");
        assert_eq!(sent[2], "weiter", "Anstoss im Ziel-Modus ohne Zeit");
        assert!(!sent[3].starts_with('['), "Hinweis der App ohne Zeit: {}", sent[3]);
        assert_eq!(sent[4], "Teilfrage", "Auftrag an einen Hilfsagenten ohne Zeit");
        assert_eq!(sent[5], "ohne Zeit");
        // Im gespeicherten Chat steht der Zeitstempel nicht im Text
        assert_eq!(list[0]["content"], "Bis wann muss das fertig sein?");
    }

    #[tokio::test]
    async fn auto_modus_alle_werkzeuge_kein_planwerkzeug() {
        let t = make(Fake::new(vec![text("ok")]), auto);
        t.send("Hallo").await;
        let names = tool_names(&t.ask.requests()[0]);
        assert!(names.contains(&"write_file".to_string()));
        assert!(!names.contains(&"plan_vorlegen".to_string()));
    }

    #[tokio::test]
    async fn verbindung_weg_wartet_und_versucht_es_nochmals_mit_hinweis() {
        let ask = Fake::new(vec![fail(anyhow::anyhow!("net::ERR_INTERNET_DISCONNECTED")), fail(status("ASK: Bad Gateway", 502)), text("Geschafft.")]);
        let t = make(ask, |o| o.retry_waits = Some(vec![0.01, 0.01, 0.01]));
        t.send("Hallo").await;
        let retries = t.events_of("retry");
        assert_eq!(retries.len(), 2);
        assert!(js_string(&retries[0]["text"]).contains("Neuer Versuch"));
        assert!(t.events_of("error").is_empty());
        assert_eq!(t.agent.messages().last().unwrap()["content"], "Geschafft.");
    }

    #[tokio::test]
    async fn wiederholen_gibt_nach_den_versuchen_auf_fehler_vom_modell_werden_nicht_wiederholt() {
        let net = || fail(anyhow::anyhow!("fetch failed"));
        let t = make(Fake::new(vec![net(), net(), net()]), |o| o.retry_waits = Some(vec![0.01, 0.01]));
        t.send("Hallo").await;
        assert_eq!(t.events_of("retry").len(), 2);
        assert!(t.events_of("error").iter().any(|e| js_string(&e["text"]).contains("fetch failed")));

        let b = make(Fake::new(vec![fail(status("ASK: model not found", 404))]), |o| o.retry_waits = Some(vec![0.01]));
        b.send("Hallo").await;
        assert_eq!(b.events_of("retry").len(), 0);
        assert_eq!(b.ask.requests().len(), 1);
    }

    #[tokio::test]
    async fn stoppen_waehrend_des_wartens_bricht_ab() {
        let t = make(Fake::new(vec![fail(anyhow::anyhow!("socket hang up"))]), |o| o.retry_waits = Some(vec![30.0]));
        let agent = t.agent.clone();
        t.agent.events.on("event", move |ev| {
            if ev["type"] == "retry" {
                agent.stop();
            }
        });
        let start = std::time::Instant::now();
        t.send("Hallo").await;
        assert!(start.elapsed() < Duration::from_secs(5), "wartet nicht die ganzen 30 s");
        assert!(t.events_of("notice").iter().any(|e| e["text"] == "Gestoppt."));
    }

    #[tokio::test]
    async fn at_datei_nur_vorhandene_pfade_auch_in_anfuehrungszeichen_gehen_als_hinweis_ans_modell() {
        let files = ["src/app.js", "Mein Bericht.docx"];
        let exists = |r: &str| files.contains(&r);
        assert_eq!(mentions_in("Schau @src/app.js und @\"Mein Bericht.docx\", nicht @fehlt.txt oder mail@x.ch", &exists), ["src/app.js", "Mein Bericht.docx"]);
        let t = make(Fake::new(vec![text("ok")]), |o| {
            o.file_exists = Arc::new(|r| r == "src/app.js" || r == "Mein Bericht.docx");
            auto(o);
        });
        t.send("Erklär @src/app.js.").await;
        assert_eq!(t.agent.messages()[0]["mentions"], json!(["src/app.js"]));
        let out = api_messages("sys", &t.agent.messages(), false, None, &NORMAL);
        let user = out.iter().find(|m| m["role"] == "user").unwrap();
        assert!(Regex::new(r"Mit @ erwähnt.*src/app\.js").unwrap().is_match(user["content"].as_str().unwrap()));
    }

    #[tokio::test]
    async fn rueckgaengig_bei_einem_frueheren_auftrag_markiert_auch_die_spaeteren() {
        let slot: SnapshotSlot = Arc::new(Mutex::new(None));
        let s2 = slot.clone();
        let t = make(Fake::new(vec![text("eins"), text("zwei"), text("drei")]), |o| {
            o.get_snapshots = Arc::new(move || s2.lock().clone());
            auto(o);
        });
        *slot.lock() = Some(Arc::new(Snapshots::new(t.folder.path(), None)));
        t.send("Erster").await;
        t.send("Zweiter").await;
        t.send("Dritter").await;
        std::fs::write(t.folder.path().join("a.txt"), "neu").unwrap();
        let ids: Vec<String> = t.agent.messages().iter().filter(|m| m["role"] == "user" && truthy(&m["snapshot"])).map(|m| js_string(&m["snapshot"])).collect();
        assert_eq!(ids.len(), 3);
        let res = t.agent.undo(&ids[1]).await;
        assert_eq!(res["ok"], true, "{res}");
        let undone = || -> Vec<bool> { t.agent.messages().iter().filter(|m| m["role"] == "user" && truthy(&m["snapshot"])).map(|m| truthy(&m["undone"])).collect() };
        assert_eq!(undone(), [false, true, true]);
        assert!(!t.folder.path().join("a.txt").exists(), "vor dem zweiten Auftrag gab es a.txt nicht");
        // Wiederherstellen: a.txt ist zurück, die Aufträge gelten wieder; ein zweites Mal geht nicht
        let note = t.agent.messages().iter().find(|m| m["note"] == "undo").cloned().unwrap();
        let redo = js_string(&note["redo"]);
        assert!(!redo.is_empty(), "{note}");
        let res = t.agent.redo(&redo).await;
        assert_eq!(res["ok"], true, "{res}");
        assert_eq!(std::fs::read_to_string(t.folder.path().join("a.txt")).unwrap(), "neu");
        assert_eq!(undone(), [false, false, false]);
        assert!(t.events_of("redo").iter().any(|e| e["snapshots"].as_array().is_some_and(|a| a.len() == 2)));
        assert_eq!(t.agent.redo(&redo).await["ok"], false);
        // der Verlauf für die Oberfläche kennt redo und redone
        let shown = t.agent.messages().iter().any(|m| m["note"] == "undo" && truthy(&m["redone"]));
        assert!(shown);
    }

    // ---------- Prüfung im Modus «Auto» (review.rs) ----------

    fn review_agent(ask: Arc<Fake>, review: ReviewFn, mode: &'static str) -> T {
        let t = make(ask, |o| {
            o.get_approval_mode = Arc::new(|| "alle".into());
            o.get_mode = Arc::new(move || mode.into());
            o.review = Some(review);
        });
        answer_approvals(&t, &["no", "no", "no", "no"]);
        t
    }

    fn cmd(id: &str, command: &str) -> ToolCall {
        call(id, "run_command", json!({ "command": command }))
    }

    #[tokio::test]
    async fn auto_harmloser_befehl_laeuft_ohne_rueckfrage_derselbe_wird_nicht_nochmals_geprueft() {
        let seen: Arc<Mutex<Vec<ReviewRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let s2 = seen.clone();
        let review: ReviewFn = Arc::new(move |r| {
            s2.lock().push(r);
            Box::pin(async { Ok(Some(crate::review::Verdict { dangerous: false, reason: String::new() })) })
        });
        let ask = Fake::new(vec![calls(vec![cmd("a", "python test.py")]), calls(vec![cmd("b", "python test.py")]), text("ok")]);
        let t = review_agent(ask, review, "auto");
        t.send("Teste mein Skript").await;
        let seen = seen.lock();
        assert_eq!(seen.len(), 1, "zweites Mal aus dem Gedächtnis des Chats");
        assert_eq!(seen[0].task, "Teste mein Skript");
        assert_eq!(seen[0].args["command"], "python test.py");
        assert!(t.events_of("approval").is_empty());
        assert!(!t.events_of("review").is_empty());
    }

    #[tokio::test]
    async fn auto_gefaehrlicher_befehl_wird_zur_rueckfrage_mit_dem_grund_des_pruefers() {
        let review: ReviewFn = Arc::new(|_| Box::pin(async { Ok(Some(crate::review::Verdict { dangerous: true, reason: "Der Befehl ändert die Registry.".into() })) }));
        let ask = Fake::new(vec![calls(vec![cmd("a", "reg add HKCU\\Software\\X /v Y /d 1")]), text("ok")]);
        let t = review_agent(ask, review, "auto");
        t.send("Mach was").await;
        let approval = &t.events_of("approval")[0];
        assert_eq!(approval["reason"], "Der Befehl ändert die Registry.");
        assert_eq!(approval["reviewed"], true);
        assert_eq!(t.events_of("tool-end")[0]["summary"], "abgelehnt");
    }

    #[tokio::test]
    async fn auto_lesen_ohne_pruefung_pruefung_kaputt_feste_regeln() {
        let count = Arc::new(AtomicUsize::new(0));
        let c2 = count.clone();
        let review: ReviewFn = Arc::new(move |_| {
            c2.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Err(anyhow::anyhow!("ASK weg")) })
        });
        let ask = Fake::new(vec![
            calls(vec![call("r", "read_file", json!({ "path": "a.txt" }))]),
            calls(vec![cmd("d", "del alt.txt")]),
            calls(vec![cmd("p", "python x.py")]),
            text("ok"),
        ]);
        let t = review_agent(ask, review, "auto");
        t.send("Räum auf").await;
        assert_eq!(count.load(Ordering::SeqCst), 2, "nur die zwei Befehle geprüft");
        let approvals = t.events_of("approval");
        assert_eq!(approvals.len(), 1, "nur das Löschen fragt nach (Regel)");
        assert!(js_string(&approvals[0]["reason"]).contains("Prüfung durch ASK war nicht möglich"));
    }

    #[tokio::test]
    async fn nachfragen_fragt_vor_jedem_befehl_ohne_pruefer() {
        let count = Arc::new(AtomicUsize::new(0));
        let c2 = count.clone();
        let review: ReviewFn = Arc::new(move |_| {
            c2.fetch_add(1, Ordering::SeqCst);
            Box::pin(async { Ok(Some(crate::review::Verdict { dangerous: false, reason: String::new() })) })
        });
        let t = review_agent(Fake::new(vec![calls(vec![cmd("a", "dir")]), text("ok")]), review, "nachfragen");
        t.send("Zeig").await;
        assert_eq!(count.load(Ordering::SeqCst), 0);
        assert_eq!(t.events_of("approval").len(), 1);
    }

    // ---------- helpers.test.js (Agent) ----------

    /// ASK-Ersatz für die Hilfsagenten-Tests: Hilfsagenten erkennt man am Systemprompt
    struct HelperFake {
        main: Mutex<VecDeque<Completion>>,
        requests: Mutex<Vec<ChatRequest>>,
        running: AtomicUsize,
        most: AtomicUsize,
    }

    fn is_helper(req: &ChatRequest) -> bool {
        js_string(&req.messages[0]["content"]).starts_with("Du bist ein Hilfsagent")
    }

    #[async_trait]
    impl ChatApi for HelperFake {
        async fn chat(&self, req: ChatRequest) -> Result<Completion> {
            self.requests.lock().push(req.clone());
            if !is_helper(&req) {
                return Ok(self.main.lock().pop_front().unwrap());
            }
            let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
            self.most.fetch_max(now, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.running.fetch_sub(1, Ordering::SeqCst);
            Ok(completion(&format!("Antwort zu {}", js_string(&req.messages.last().unwrap()["content"])), vec![]))
        }
    }

    fn helper_fake(main: Vec<Completion>) -> Arc<HelperFake> {
        Arc::new(HelperFake { main: Mutex::new(main.into()), requests: Mutex::new(Vec::new()), running: AtomicUsize::new(0), most: AtomicUsize::new(0) })
    }

    fn helper_agent(fake: Arc<HelperFake>, tweak: impl FnOnce(&mut AgentOptions)) -> (Arc<Agent>, tempfile::TempDir, Arc<Mutex<Vec<Value>>>) {
        let folder = tempfile::tempdir().unwrap();
        let path = folder.path().to_string_lossy().to_string();
        let mut o = AgentOptions::new(fake, move || path.clone(), || "gross".to_string());
        o.terminal = Some(Arc::new(EndpointSnapshot { state: "gestoppt".into(), port: 0, key: String::new() }));
        o.get_system_prompt = Arc::new(|_| "sys".into());
        o.get_approval_mode = Arc::new(|| "heikel".into());
        tweak(&mut o);
        let agent = Agent::new(o);
        let events = Arc::new(Mutex::new(Vec::new()));
        let ev = events.clone();
        agent.events.on("event", move |e| ev.lock().push(e.clone()));
        (agent, folder, events)
    }

    #[tokio::test]
    async fn agent_vier_hilfsagenten_in_einem_schritt_drei_gleichzeitig_eigenes_modell_ergebnisse_in_reihenfolge() {
        let four: Vec<ToolCall> = (1..=4).map(|n| call(&format!("h{n}"), "hilfsagent", json!({ "titel": format!("Teil {n}"), "auftrag": format!("Teil {n} klären") }))).collect();
        let fake = helper_fake(vec![completion("", four), completion("Alles zusammengetragen.", vec![])]);
        let (agent, _folder, events) = helper_agent(fake.clone(), |o| o.get_helper_model = Arc::new(|| "klein".into()));
        agent.run_turn("Finde alles heraus", vec![], json!({})).await;
        assert_eq!(fake.most.load(Ordering::SeqCst), 3, "höchstens drei gleichzeitig");
        let requests = fake.requests.lock().clone();
        assert!(requests.iter().filter(|r| is_helper(r)).all(|r| r.model == "klein"));
        assert!(requests.iter().filter(|r| !is_helper(r)).all(|r| r.model == "gross"));
        assert!(tool_names(&requests[0]).contains(&"hilfsagent".to_string()), "Werkzeug angeboten");
        let last = requests.iter().rfind(|r| !is_helper(r)).unwrap();
        let results: Vec<&Value> = last.messages.iter().filter(|m| m["role"] == "tool").collect();
        assert_eq!(results.iter().map(|m| js_string(&m["tool_call_id"])).collect::<Vec<_>>(), ["h1", "h2", "h3", "h4"]);
        assert!(js_string(&results[2]["content"]).contains("Antwort zu Teil 3 klären"));
        let events = events.lock();
        assert_eq!(events.iter().filter(|e| e["type"] == "tool-start").count(), 4, "jeder Hilfsagent einmal gestartet");
        assert!(events.iter().any(|e| e["type"] == "tool-progress" && e["id"] == "h1"));
        assert!(!events.iter().any(|e| e["type"] == "approval"), "keine Rückfrage");
    }

    #[tokio::test]
    async fn abgelehnter_hilfsagent_startet_gar_nicht_die_anderen_laufen() {
        // Mod-Regel «web»: vor jedem Hilfsagenten eine Rückfrage; den zweiten lehnt die Person ab
        let three: Vec<ToolCall> = (1..=3).map(|n| call(&format!("h{n}"), "hilfsagent", json!({ "titel": format!("Teil {n}"), "auftrag": format!("Teil {n} klären") }))).collect();
        let fake = helper_fake(vec![completion("", three), completion("Fertig.", vec![])]);
        let (agent, _folder, events) = helper_agent(fake.clone(), |o| o.get_ask_before = Arc::new(|| vec!["web".to_string()]));
        let answers: Arc<Mutex<VecDeque<&str>>> = Arc::new(Mutex::new(VecDeque::from(["yes", "no", "yes"])));
        let a2 = agent.clone();
        agent.events.on("event", move |ev| {
            if ev["type"] == "approval" {
                let a = answers.lock().pop_front().unwrap_or("no");
                a2.answer_approval(&js_string(&ev["id"]), a);
            }
        });
        agent.run_turn("Finde alles heraus", vec![], json!({})).await;
        let helpers: Vec<String> = fake.requests.lock().iter().filter(|r| is_helper(r)).map(|r| js_string(&r.messages.last().unwrap()["content"])).collect();
        assert_eq!(helpers.len(), 2, "der abgelehnte lief nicht: {helpers:?}");
        assert!(helpers.iter().all(|h| !h.contains("Teil 2")), "{helpers:?}");
        let last = fake.requests.lock().iter().rfind(|r| !is_helper(r)).cloned().unwrap();
        let results: Vec<&Value> = last.messages.iter().filter(|m| m["role"] == "tool").collect();
        assert_eq!(results.iter().map(|m| js_string(&m["tool_call_id"])).collect::<Vec<_>>(), ["h1", "h2", "h3"]);
        assert!(js_string(&results[0]["content"]).contains("Antwort zu Teil 1 klären"));
        assert!(js_string(&results[1]["content"]).contains("abgelehnt"), "{}", results[1]["content"]);
        assert!(js_string(&results[2]["content"]).contains("Antwort zu Teil 3 klären"), "Ergebnis richtig zugeordnet");
        assert_eq!(events.lock().iter().filter(|e| e["type"] == "tool-start").count(), 3);
    }

    #[tokio::test]
    async fn agent_hilfsagenten_auch_im_planmodus_ohne_eigenes_modell_das_modell_des_chats() {
        let fake = helper_fake(vec![completion("", vec![call("h", "hilfsagent", json!({ "titel": "t", "auftrag": "a" }))]), completion("Ok.", vec![])]);
        let (agent, _folder, _events) = helper_agent(fake.clone(), |o| o.get_mode = Arc::new(|| "plan".into()));
        agent.run_turn("Plane das", vec![], json!({})).await;
        let requests = fake.requests.lock().clone();
        assert!(tool_names(&requests[0]).contains(&"hilfsagent".to_string()));
        assert_eq!(requests.iter().find(|r| is_helper(r)).unwrap().model, "gross");
        let result = requests.last().unwrap().messages.iter().find(|m| m["role"] == "tool").unwrap();
        assert!(js_string(&result["content"]).contains("Antwort zu a"), "{result}");
    }

    // ---------- askapi.test.js: Verlauf an das Modell ----------

    #[test]
    fn verlauf_ohne_denktext_alte_ergebnisse_gekuerzt_textmodus_als_text() {
        let big = "x".repeat(5000);
        let mut messages = vec![json!({ "role": "user", "content": "los" })];
        for i in 0..10 {
            messages.push(json!({ "role": "assistant", "content": "", "reasoning": "geheim", "tool_calls": [{ "id": format!("c{i}"), "type": "function", "function": { "name": "read_file", "arguments": "{}" } }] }));
            messages.push(json!({ "role": "tool", "tool_call_id": format!("c{i}"), "content": big }));
        }
        messages.push(json!({ "role": "assistant", "content": "a", "textMode": true, "raw": "a<tool_call>{}</tool_call>" }));
        messages.push(json!({ "role": "tool", "tool_call_id": "t", "content": "ok", "textMode": true }));
        let out = api_messages("SYS", &messages, false, None, &NORMAL);
        assert_eq!(out[0]["content"], "SYS");
        assert!(out.iter().all(|m| m.get("reasoning").is_none()));
        let tools: Vec<&Value> = out.iter().filter(|m| m["role"] == "tool").collect();
        assert!(tools[0]["content"].as_str().unwrap().chars().count() < 2000, "älteste Ergebnisse gekürzt");
        assert_eq!(tools.last().unwrap()["content"].as_str().unwrap().len(), 5000, "neueste Ergebnisse ganz");
        assert_eq!(out[out.len() - 2..], [json!({ "role": "assistant", "content": "a<tool_call>{}</tool_call>" }), json!({ "role": "user", "content": "<tool_response>\nok\n</tool_response>" })]);
    }

    #[test]
    fn textmodus_wandelt_auch_fruehere_werkzeugaufrufe_in_text_um() {
        let out = api_messages("SYS", &[
            json!({ "role": "user", "content": "los" }),
            json!({ "role": "assistant", "content": "Ich schaue.", "tool_calls": [{ "id": "c1", "type": "function", "function": { "name": "list_files", "arguments": "{\"path\":\".\"}" } }] }),
            json!({ "role": "tool", "tool_call_id": "c1", "content": "a.txt" }),
        ], true, None, &NORMAL);
        assert_eq!(out.iter().map(|m| js_string(&m["role"])).collect::<Vec<_>>(), ["system", "user", "assistant", "user"]);
        assert_eq!(out[2]["content"], "Ich schaue.\n<tool_call>\n{\"name\": \"list_files\", \"arguments\": {\"path\":\".\"}}\n</tool_call>");
        assert!(out[2].get("tool_calls").is_none());
        assert_eq!(out[3]["content"], "<tool_response>\na.txt\n</tool_response>");
    }

    #[test]
    fn verlauf_lange_argumente_frueherer_aufrufe_werden_gekuerzt_hinweise_gehen_nicht_an_das_modell() {
        let big = "x".repeat(5000);
        let mut messages = vec![json!({ "role": "user", "content": "los" })];
        for n in 0..12 {
            messages.push(json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": format!("c{n}"), "type": "function", "function": { "name": "write_file", "arguments": json!({ "path": format!("d{n}.txt"), "content": big }).to_string() } }] }));
            messages.push(json!({ "role": "note", "kind": "notice", "content": "Hinweis für die Person" }));
            messages.push(json!({ "role": "tool", "tool_call_id": format!("c{n}"), "content": format!("Geschrieben {big}") }));
        }
        let out = api_messages("SYS", &messages, false, None, &NORMAL);
        assert!(out.iter().all(|m| ["system", "user", "assistant", "tool"].contains(&m["role"].as_str().unwrap())));
        let args: Vec<String> = out.iter().filter(|m| m.get("tool_calls").is_some()).map(|m| js_string(&m["tool_calls"][0]["function"]["arguments"])).collect();
        assert_eq!(args.len(), 12);
        let first: Value = serde_json::from_str(&args[0]).unwrap();
        assert!(args[0].len() < 1500 && first["path"] == "d0.txt", "alter Aufruf gekürzt, bleibt gültiges JSON");
        assert!(first["content"].as_str().unwrap().contains("gekürzt"));
        assert_eq!(serde_json::from_str::<Value>(&args[11]).unwrap()["content"], big.as_str(), "neuer Aufruf unverändert");
        assert!(messages[1]["tool_calls"][0]["function"]["arguments"].as_str().unwrap().len() > 5000, "der gespeicherte Chat bleibt vollständig");
        let size = |l: &[Value]| serde_json::to_string(l).unwrap().len();
        assert!(size(&api_messages("SYS", &messages, false, None, &TIGHT)) < size(&out) / 2);
        // Textmodus: frühere Aufrufe ebenfalls gekürzt
        let text = api_messages("SYS", &messages, true, None, &NORMAL);
        assert!(text[1]["role"] == "user" && text[2]["content"].as_str().unwrap().contains("<tool_call>") && text[2]["content"].as_str().unwrap().len() < 1500);
        assert_eq!(short_args(&"kein json ".repeat(500), 100).len(), 5000);
        assert_eq!(short_args("{\"a\":\"kurz\"}", 100), "{\"a\":\"kurz\"}");
    }

    #[test]
    fn meldungen_verlauf_zu_lang_werden_erkannt_andere_nicht() {
        for m in ["This model's maximum context length is 32768 tokens. However, your request has 40211 input tokens.", "context_length_exceeded",
            "prompt is too long: 210000 tokens > 200000 maximum", "Please reduce the length of the messages."] {
            assert!(TOO_LONG.is_match(m), "{m}");
        }
        for m in ["tools is not supported", "model does not support image input", "Model not found"] {
            assert!(!TOO_LONG.is_match(m), "{m}");
        }
    }

    // ---------- attachments.test.js, cleanup.test.js, visual.test.js, memory.test.js ----------

    #[test]
    fn verlauf_pfade_im_text_bilder_nur_aus_den_letzten_zwei_eingaben() {
        let att = |n: &str, image: bool| json!({ "path": format!("eingang/{n}"), "name": n, "image": image });
        let messages = vec![
            json!({ "role": "user", "content": "alt", "attachments": [att("1.png", true)] }),
            json!({ "role": "assistant", "content": "ok" }),
            json!({ "role": "user", "content": "", "attachments": [att("2.png", true), att("liste.xlsx", false)] }),
            json!({ "role": "assistant", "content": "ok" }),
            json!({ "role": "user", "content": "neu", "attachments": [att("3.png", true)] }),
        ];
        let url = |a: &Value| Some(format!("data:image/png;base64,{}", js_string(&a["name"])));
        let out = api_messages("sys", &messages, false, Some(&url), &NORMAL);
        assert_eq!(out[1]["content"], "alt\n\n[Angehängt, liegt im Arbeitsordner: eingang/1.png]");
        assert_eq!(out[3]["content"], json!([
            { "type": "text", "text": "Siehe Anhang.\n\n[Angehängt, liegt im Arbeitsordner: eingang/2.png, eingang/liste.xlsx]" },
            { "type": "image_url", "image_url": { "url": "data:image/png;base64,2.png" } }]));
        assert_eq!(out[5]["content"][1]["image_url"]["url"], "data:image/png;base64,3.png");
        // ohne Bildfunktion (Modell versteht keine Bilder): nur Text
        assert!(api_messages("sys", &messages, false, None, &NORMAL).iter().all(|m| m["content"].is_string()));
        // Bild nicht mehr lesbar: nur Text
        let none = |_: &Value| None;
        assert!(api_messages("sys", &messages, false, Some(&none), &NORMAL)[5]["content"].is_string());
    }

    #[test]
    fn schleifenschutz_dreimal_derselbe_aufruf_oder_viele_fehler_hintereinander() {
        let c = |name: &str, args: Value| json!({ "role": "assistant", "content": "", "tool_calls": [{ "function": { "name": name, "arguments": args.to_string() } }] });
        let ok = json!({ "role": "tool", "ok": true, "content": "" });
        let bad = json!({ "role": "tool", "ok": false, "content": "Fehler" });
        let same = vec![json!({ "role": "user", "content": "los" }), c("run_command", json!({ "command": "ping" })), ok.clone(), c("run_command", json!({ "command": "ping" })), ok.clone()];
        assert_eq!(loop_reason(&same), "");
        let with = |extra: Vec<Value>| [same.clone(), extra].concat();
        assert!(loop_reason(&with(vec![c("run_command", json!({ "command": "ping" })), ok.clone()])).contains("3-mal hintereinander genau denselben Schritt"));
        assert_eq!(loop_reason(&with(vec![c("run_command", json!({ "command": "dir" })), ok.clone()])), "");
        let mut fails = vec![json!({ "role": "user", "content": "los" })];
        for i in 0..8 {
            fails.push(c("read_file", json!({ "path": format!("d{i}.txt") })));
            fails.push(bad.clone());
        }
        assert!(loop_reason(&fails).contains("8 Werkzeugaufrufe hintereinander"));
        fails.push(json!({ "role": "user", "content": "neu" }));
        assert_eq!(loop_reason(&fails), "", "zählt nur seit der letzten Eingabe");
    }

    #[test]
    fn ansichten_bilder_als_eigene_nachricht_nur_die_letzten_zwei_schleifenschutz_zaehlt_sie_nicht_als_eingabe() {
        let view = |n: &str| json!({ "role": "user", "auto": true, "note": "view", "content": format!("[Ansicht] Bilder zu view_file({n}.docx)"),
            "attachments": [{ "path": format!(".ask/arbeit/c/ansicht/{n}.png"), "name": format!("{n}.png"), "image": true }] });
        let c = |id: &str| json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": id, "type": "function", "function": { "name": "view_file", "arguments": "{\"path\":\"a.docx\"}" } }] });
        let r = |id: &str| json!({ "role": "tool", "tool_call_id": id, "content": "Seiten 1 von 1" });
        let messages = vec![json!({ "role": "user", "content": "mach den Bericht", "attachments": [{ "path": ".ask/anhaenge/c/foto.png", "name": "foto.png", "image": true }] }),
            c("1"), r("1"), view("a"), c("2"), r("2"), view("b"), c("3"), r("3"), view("c")];
        let url = |a: &Value| Some(format!("data:image/png;base64,{}", js_string(&a["name"])));
        let out = api_messages("sys", &messages, false, Some(&url), &NORMAL);
        let views: Vec<&Value> = out.iter().filter(|m| m["role"] == "user").skip(1).collect();
        assert_eq!(views[0]["content"], "[Ansicht] Bilder zu view_file(a.docx): nicht mehr mitgeschickt.");
        assert_eq!(views[1]["content"][1]["image_url"]["url"], "data:image/png;base64,b.png");
        assert_eq!(views[2]["content"][0]["text"], "[Ansicht] Bilder zu view_file(c.docx)");
        assert!(out[1]["content"].is_array(), "Anhang der Person bleibt mit Bild");
        // drei gleiche Aufrufe seit der Eingabe der Person, obwohl Ansichten dazwischen liegen
        assert!(loop_reason(&messages).contains("3-mal"));
    }

    #[test]
    fn das_modell_erfaehrt_dass_die_app_einen_sichtbaren_browser_hat() {
        let spec = tool_specs("").into_iter().find(|s| s["function"]["name"] == "browser").unwrap()["function"]["description"].as_str().unwrap().to_string();
        assert!(spec.contains("sichtbar im Browser-Bereich"));
        assert!(!spec.contains("unsichtbar"), "{spec}");
        let prompt = std::fs::read_to_string(crate::paths::repo_root().join("resources").join("anweisungen").join("systemprompt.md")).unwrap();
        assert!(prompt.contains("Die App hat einen eingebauten Browser"));
    }

    #[test]
    fn skill_mit_name_anleitung_geht_an_das_modell_im_chat_steht_nur_die_eingabe() {
        let msgs = api_messages("System", &[json!({ "role": "user", "content": "/bericht Woche 47", "skill": { "name": "bericht", "text": "Schreib eine Seite." } })], false, None, &NORMAL);
        assert_eq!(msgs[1]["content"], "[Die Person ruft ihren Skill «bericht» auf. Anleitung des Skills:]\nSchreib eine Seite.\n[Ende der Anleitung]\n\n/bericht Woche 47");
    }

    #[tokio::test]
    async fn arbeitsschleife_merken_als_werkzeug_skill_nur_an_das_modell_done_meldet_stopp_und_fehler() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Arc::new(Memory::new(dir.path().join("gedaechtnis.json")));
        let on = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let on2 = on.clone();
        let ask = Fake::new(vec![
            calls(vec![call("c1", "merken", json!({ "text": "Mag Tabellen.", "bereich": "konto" }))]),
            text("Gemerkt."),
            fail(status("ASK: kaputt", 500)),
            text("Fertig."),
        ]);
        let m2 = memory.clone();
        let t = make(ask, |o| o.knowledge = Some(KnowledgeCtx { memory: Some(m2), memory_on: Some(Arc::new(move || on2.load(Ordering::SeqCst))), ..Default::default() }));
        t.send("Merk dir, dass ich Tabellen mag.").await;
        let items: Vec<(String, String)> = memory.list().iter().map(|i| (i.text.clone(), i.source.clone())).collect();
        assert_eq!(items, [("Mag Tabellen.".to_string(), "merken".to_string())]);
        assert!(tool_names(&t.ask.requests()[0]).contains(&"merken".to_string()));
        t.send("Noch etwas").await;
        let done: Vec<(Value, Value)> = t.events_of("done").iter().map(|d| (d["stopped"].clone(), d["failed"].clone())).collect();
        assert_eq!(done, [(json!(false), json!(false)), (json!(false), json!(true))]);
        on.store(false, Ordering::SeqCst);
        t.agent.run_turn("/bericht KW 47", vec![], json!({ "skill": { "name": "bericht", "text": "Eine Seite." } })).await;
        let requests = t.ask.requests();
        let last = requests.last().unwrap();
        let names = tool_names(last);
        assert!(!names.iter().any(|n| n == "merken" || n == "vergessen"), "ausgeschaltet: keine Werkzeuge fürs Gedächtnis");
        let content = js_string(&last.messages.last().unwrap()["content"]);
        assert!(Regex::new(r"Anleitung des Skills:\]\nEine Seite\.[\s\S]*/bericht KW 47$").unwrap().is_match(&content), "{content}");
        let items = t.agent.transcript();
        assert_eq!(items.as_array().unwrap().iter().rfind(|i| i["role"] == "user").unwrap()["content"], "/bericht KW 47");
    }

    // ---------- weitere Fälle der Arbeitsschleife ----------

    #[tokio::test]
    async fn ziel_modus_stoesst_an_und_goal_done_beendet() {
        let ask = Fake::new(vec![text("Ich überlege."), calls(vec![call("g", "goal_done", json!({ "zusammenfassung": "Alles fertig" }))]), text("Fertig.")]);
        let t = make(ask, |_| {});
        t.agent.set_goal("Bau etwas");
        t.send("Los").await;
        let goal = t.agent.state()["goal"].clone();
        assert_eq!(goal["done"], true);
        assert_eq!(goal["summary"], "Alles fertig");
        assert_eq!(t.ask.requests().len(), 3);
        assert!(t.agent.messages().iter().any(|m| m["role"] == "user" && m["auto"] == true && m["content"] == GOAL_NUDGE));
    }

    #[tokio::test]
    async fn automatisch_das_modell_waehlt_den_denkaufwand_selbst() {
        let ask = Fake::new(vec![
            calls(vec![call("e", "denkaufwand", json!({ "stufe": "maximal", "grund": "kniffliger Fehler" }))]),
            text("Gefunden."),
        ]);
        let t = make(ask, |o| o.get_thinking = Arc::new(|| "standard".to_string()));
        t.send("Such den Fehler").await;
        let reqs = t.ask.requests();
        let names = |r: &ChatRequest| r.tools.clone().unwrap_or_default().iter().map(|s| js_string(&s["function"]["name"])).collect::<Vec<_>>();
        assert!(names(&reqs[0]).contains(&"denkaufwand".to_string()), "bei «Automatisch» angeboten");
        assert_eq!(reqs.iter().map(|r| r.thinking.clone()).collect::<Vec<_>>(), vec!["mittel", "maximal"], "beginnt bei mittel, dann die Wahl");
        // nächster Auftrag beginnt wieder bei mittel
        t.send("Und jetzt?").await;
        assert_eq!(t.ask.requests().last().unwrap().thinking, "mittel");

        // fest eingestellt: kein Werkzeug, die Einstellung gilt
        let fixed = make(Fake::new(vec![text("Ok")]), |o| o.get_thinking = Arc::new(|| "tief".to_string()));
        fixed.send("Hallo").await;
        let r = fixed.ask.requests();
        assert!(!names(&r[0]).contains(&"denkaufwand".to_string()));
        assert_eq!(r[0].thinking, "tief");
    }

    #[tokio::test]
    async fn send_kehrt_sofort_zurueck_und_der_auftrag_laeuft_weiter() {
        let t = make(Fake::new(vec![text("Hallo")]), |_| {});
        t.agent.send("Los", vec![], json!({}));
        assert!(t.agent.running());
        for _ in 0..200 {
            if !t.agent.running() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(!t.agent.running());
        assert_eq!(t.roles(), ["user", "assistant"]);
        assert_eq!(t.events_of("done").len(), 1);
    }

    #[tokio::test]
    async fn chat_wird_im_selben_format_gespeichert_und_wieder_geoeffnet() {
        let t = make(Fake::new(vec![text("Antwort")]), |_| {});
        t.send("Frage").await;
        let saved = t.saved();
        let keys: Vec<&String> = saved.as_object().unwrap().keys().collect();
        assert_eq!(keys, ["id", "title", "created", "updated", "messages", "titleAuto"]);
        assert_eq!(saved["messages"][1]["stats"]["promptTokens"], 0);
        let id = t.agent.chat_id();
        t.agent.new_chat();
        assert!(t.agent.open(&id));
        assert_eq!(t.agent.messages().len(), 2);
        assert!(!t.agent.open("../x"));
        let folder = t.folder();
        assert!(t.agent.remove(&folder, &id));
        assert_eq!(t.agent.messages().len(), 0);
        t.agent.open_latest(Some(NEW));
    }
}
