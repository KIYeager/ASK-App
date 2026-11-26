//! Nutzungsübersicht (ENTSCHEIDE E49): zählt aus den Chats in den Projektordnern (.ask/chats/*.json),
//! was die Person mit ASK gemacht hat. Alles lokal; `aggregate()` schickt nichts irgendwohin.
//! Nur `ask_usage()` fragt ASK, und nur nach den eigenen Daten des angemeldeten Kontos.
//!
//! Portiert aus src/main/usage.js. Öffentliche Schnittstelle: `summarize_chat(&Value) -> Summary`,
//! `folder_usage(&Path) -> Summary`, `aggregate(&[Value], AggregateOptions) -> Aggregate`,
//! `ask_usage(Option<&AskSession>, AskUsageOptions) -> Option<Value>`, `usage_numbers(&Value) -> Map`,
//! `day_key(ms) -> String`, `DAYS`. Zahlen sind ganze Zahlen (gebrochene Werte in Chats werden abgeschnitten).

use crate::askapi::{AskSession, RequestOptions, Signal};
use crate::util::{js_string, now_ms, truthy, win_basename};
use chrono::{Local, TimeZone};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, SystemTime};

pub const DAYS: usize = 30;
const DAY_MS: i64 = 86_400_000;

/// Tag als «JJJJ-MM-TT» in der Ortszeit des Laptops
pub fn day_key(ms: i64) -> String {
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => d.format("%Y-%m-%d").to_string(),
        None => String::new(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Files {
    pub added: i64,
    pub changed: i64,
    pub removed: i64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Day {
    pub tasks: i64,
    pub tokens: i64,
}

/// Zahlen eines Chats, eines Projekts oder aller Projekte. `days` (Tageswerte «JJJJ-MM-TT») gehört
/// nur zur Zwischenrechnung und kommt nicht in die Ausgabe an die Oberfläche.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub chats: i64,
    pub tasks: i64,
    pub steps: i64,
    pub tokens: i64,
    pub estimated_tokens: i64,
    pub model_ms: i64,
    pub files: Files,
    pub last: i64,
    #[serde(skip)]
    pub days: BTreeMap<String, Day>,
}

fn add(into: &mut Summary, s: &Summary) {
    into.chats += s.chats;
    into.tasks += s.tasks;
    into.steps += s.steps;
    into.tokens += s.tokens;
    into.estimated_tokens += s.estimated_tokens;
    into.model_ms += s.model_ms;
    into.files.added += s.files.added;
    into.files.changed += s.files.changed;
    into.files.removed += s.files.removed;
    into.last = into.last.max(s.last);
}

/// Positive endliche Zahl, sonst 0 (`Number(v) > 0`). Gebrochenes wird abgeschnitten.
fn n(v: Option<&Value>) -> i64 {
    let f = match v {
        Some(Value::Number(x)) => x.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        Some(Value::Bool(true)) => 1.0,
        _ => 0.0,
    };
    if f.is_finite() && f > 0.0 {
        f as i64
    } else {
        0
    }
}

fn len_of(v: Option<&Value>) -> Option<i64> {
    match v {
        Some(Value::Array(a)) => Some(a.len() as i64),
        Some(Value::String(s)) => Some(s.chars().count() as i64),
        _ => None,
    }
}

/// Ein gespeicherter Chat → Zahlen und Tageswerte { 'JJJJ-MM-TT': { tasks, tokens } }.
/// Aufträge sind Eingaben der Person (ohne automatische Nachrichten wie «Weiter am Ziel» oder Ansichten),
/// Token die erzeugten Token (stats.tokens; geschätzte zählen mit und werden ausgewiesen),
/// Dateien aus «changes» der Eingabe; ein rückgängig gemachter Auftrag zählt dort nicht mehr.
pub fn summarize_chat(chat: &Value) -> Summary {
    let mut s = Summary { chats: 1, ..Default::default() };
    let fallback = {
        let u = n(chat.get("updated"));
        if u != 0 { u } else { n(chat.get("created")) }
    };
    let key_of = |ms: Option<&Value>| {
        let t = n(ms);
        day_key(if t != 0 { t } else { fallback })
    };
    let no_messages = Vec::new();
    for m in chat.get("messages").and_then(Value::as_array).unwrap_or(&no_messages) {
        if !m.is_object() {
            continue;
        }
        s.last = s.last.max(n(m.get("time")));
        let role = m.get("role").and_then(Value::as_str);
        let flag = |k: &str| m.get(k).is_some_and(truthy);
        if role == Some("user") && !flag("auto") {
            s.tasks += 1;
            s.days.entry(key_of(m.get("time"))).or_default().tasks += 1;
            if let Some(c) = m.get("changes").filter(|c| truthy(c)) {
                if !flag("undone") {
                    let counts = c.get("counts").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
                    // `counts[i] ?? c.<liste>?.length`
                    let count = |i: usize, list: &str| match counts.get(i).filter(|v| !v.is_null()) {
                        Some(v) => n(Some(v)),
                        None => n(len_of(c.get(list)).map(|l| json!(l)).as_ref()),
                    };
                    s.files.changed += count(0, "changed");
                    s.files.added += count(1, "added");
                    s.files.removed += count(2, "removed");
                }
            }
        } else if role == Some("assistant") && m.get("stats").is_some_and(truthy) {
            let stats = m.get("stats");
            let t = n(stats.and_then(|x| x.get("tokens")));
            s.tokens += t;
            if stats.and_then(|x| x.get("estimated")).is_some_and(truthy) {
                s.estimated_tokens += t;
            }
            s.model_ms += n(stats.and_then(|x| x.get("totalMs")));
            s.days.entry(key_of(m.get("time"))).or_default().tokens += t;
        } else if role == Some("tool") {
            s.steps += 1;
        }
    }
    if s.last == 0 {
        s.last = fallback;
    }
    s
}

/// Zwischenspeicher pro Datei: unveränderte Chats (gleiche Änderungszeit und Grösse) nicht neu lesen
type Cached = (SystemTime, u64, Summary);
static CACHE: Mutex<Option<HashMap<String, Cached>>> = Mutex::new(None);

/// Zahlen eines Projektordners (alle Chats in .ask/chats)
pub fn folder_usage(folder: &Path) -> Summary {
    let mut total = Summary::default();
    let dir = folder.join(".ask").join("chats");
    let Ok(read) = std::fs::read_dir(&dir) else { return total };
    let mut names: Vec<String> = read.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).filter(|f| f.ends_with(".json")).collect();
    names.sort();
    for name in names {
        let file = dir.join(&name);
        let key = file.to_string_lossy().to_string();
        let summary = (|| {
            let st = std::fs::metadata(&file).ok()?;
            let mtime = st.modified().ok()?;
            let hit = CACHE.lock().as_ref().and_then(|c| c.get(&key)).filter(|h| h.0 == mtime && h.1 == st.len()).map(|h| h.2.clone());
            if let Some(s) = hit {
                return Some(s);
            }
            let text = std::fs::read_to_string(&file).ok()?;
            let chat: Value = serde_json::from_str(&text).ok()?;
            if !chat.get("messages").is_some_and(Value::is_array) {
                return None;
            }
            let summary = summarize_chat(&chat);
            let mut cache = CACHE.lock();
            let map = cache.get_or_insert_with(HashMap::new);
            if map.len() > 5000 {
                map.clear();
            }
            map.insert(key.clone(), (mtime, st.len(), summary.clone()));
            Some(summary)
        })();
        let Some(summary) = summary else { continue }; // halb geschrieben oder nicht lesbar
        add(&mut total, &summary);
        for (k, v) in &summary.days {
            let d = total.days.entry(k.clone()).or_default();
            d.tasks += v.tasks;
            d.tokens += v.tokens;
        }
    }
    total
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectUsage {
    pub folder: String,
    pub name: String,
    pub color: String,
    pub missing: bool,
    #[serde(flatten)]
    pub summary: Summary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DayPoint {
    pub date: String,
    pub tasks: i64,
    pub tokens: i64,
}

/// Ergebnis von `aggregate`: `{ projects, total, days }`
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Aggregate {
    pub projects: Vec<ProjectUsage>,
    pub total: Summary,
    pub days: Vec<DayPoint>,
}

#[derive(Clone, Copy, Debug)]
pub struct AggregateOptions {
    pub now: i64,
    pub days: usize,
}

impl Default for AggregateOptions {
    fn default() -> Self {
        AggregateOptions { now: now_ms(), days: DAYS }
    }
}

/// Alle Projekte: { projects: [...], total, days: [{ date, tasks, tokens }] (letzte 30 Tage, ältester zuerst) }
pub fn aggregate(projects: &[Value], opts: AggregateOptions) -> Aggregate {
    let mut total = Summary::default();
    let mut by_day: BTreeMap<String, Day> = BTreeMap::new();
    let mut list = Vec::new();
    for p in projects {
        let folder = p.get("folder").filter(|f| truthy(f)).map(js_string).unwrap_or_default();
        if folder.is_empty() {
            continue;
        }
        let missing = !Path::new(&folder).exists();
        let mut u = if missing { Summary::default() } else { folder_usage(Path::new(&folder)) };
        let per_day = std::mem::take(&mut u.days);
        let name = p.get("name").filter(|v| truthy(v)).map(js_string).unwrap_or_else(|| win_basename(&folder));
        let color = p.get("color").filter(|v| truthy(v)).map(js_string).unwrap_or_default();
        add(&mut total, &u);
        list.push(ProjectUsage { folder, name, color, missing, summary: u });
        for (k, v) in per_day {
            let d = by_day.entry(k).or_default();
            d.tasks += v.tasks;
            d.tokens += v.tokens;
        }
    }
    let mut series = Vec::new();
    // Mittag: Sommerzeit verschiebt keinen Tag
    let noon = Local
        .timestamp_millis_opt(opts.now)
        .single()
        .and_then(|d| d.date_naive().and_hms_opt(12, 0, 0))
        .and_then(|d| Local.from_local_datetime(&d).earliest())
        .map(|d| d.timestamp_millis())
        .unwrap_or(opts.now);
    for i in (0..opts.days as i64).rev() {
        let date = day_key(noon - i * DAY_MS);
        let d = by_day.get(&date).copied().unwrap_or_default();
        series.push(DayPoint { date, tasks: d.tasks, tokens: d.tokens });
    }
    list.sort_by(|a, b| b.summary.tasks.cmp(&a.summary.tasks).then(b.summary.last.cmp(&a.summary.last)));
    Aggregate { projects: list, total, days: series }
}

// ---------- Angaben von ASK (optional) ----------
// NICHT mit dem echten ASK geprüft. Ob ein normales Konto dort Nutzungszahlen abfragen darf, ist offen.
// Versucht werden bekannte Schnittstellen von Open WebUI; was nicht antwortet oder anders aussieht,
// wird still übergangen. Ohne Ergebnis: None, die Oberfläche zeigt den Abschnitt dann nicht.

const PAGES_MAX: usize = 20;
/// Kandidaten für Nutzungszahlen (Open WebUI je nach Version und Rolle); nur Zahlen mit bekannten Namen zählen
const USAGE_PATHS: [&str; 3] = ["/api/v1/chats/stats/usage", "/api/v1/users/user/usage", "/api/v1/analytics/summary"];
const USAGE_FIELDS: [(&str, [&str; 3]); 4] = [
    ("messages", ["total_messages", "message_count", "messages"]),
    ("tokens", ["total_tokens", "tokens", ""]),
    ("inputTokens", ["prompt_tokens", "input_tokens", ""]),
    ("outputTokens", ["completion_tokens", "output_tokens", ""]),
];

/// Open WebUI zählt in Sekunden
fn seconds(v: Option<&Value>) -> i64 {
    let x = n(v);
    if x > 1_000_000_000_000 { x } else { x * 1000 }
}

fn number_value(f: i64) -> Value {
    json!(f)
}

/// Bekannte Zahlen aus einer Antwort lesen: oben, unter «data» oder als Summe über «items»
pub fn usage_numbers(data: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    if !data.is_object() && !data.is_array() {
        return out;
    }
    let sources: Vec<&Value> = [Some(data), data.get("data")].into_iter().flatten().filter(|x| x.is_object()).collect();
    let items: &[Value] = [data.get("items"), data.get("data"), Some(data)]
        .into_iter()
        .flatten()
        .find_map(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for (key, names) in USAGE_FIELDS {
        let names: Vec<&str> = names.iter().copied().filter(|s| !s.is_empty()).collect();
        for src in &sources {
            if let Some(v) = names.iter().filter_map(|f| src.get(*f)).find(|v| v.is_number()) {
                out.insert(key.to_string(), v.clone());
                break;
            }
        }
        if !out.contains_key(key) && !items.is_empty() {
            if let Some(name) = names.iter().find(|f| items.iter().any(|i| i.get(**f).is_some_and(Value::is_number))) {
                out.insert(key.to_string(), number_value(items.iter().map(|i| n(i.get(*name))).sum()));
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug)]
pub struct AskUsageOptions {
    pub now: i64,
    pub timeout_ms: u64,
}

impl Default for AskUsageOptions {
    fn default() -> Self {
        AskUsageOptions { now: now_ms(), timeout_ms: 5000 }
    }
}

/// Antwort als JSON; nicht erlaubt oder nicht lesbar: None; Verbindungsfehler: Err
async fn get_json(session: &AskSession, api_path: &str, signal: &Signal) -> anyhow::Result<Option<Value>> {
    let work = async {
        let res = session.raw(api_path, RequestOptions::default().with_signal(signal.clone())).await?;
        if !res.status().is_success() {
            return Ok(None);
        }
        Ok(res.json::<Value>().await.ok())
    };
    tokio::select! {
        _ = signal.cancelled() => anyhow::bail!("abgebrochen"),
        r = work => r,
    }
}

/// session: AskSession (askapi). Ergebnis `{ webChats, webChats30, more, ...Zahlen }` oder None.
pub async fn ask_usage(session: Option<&AskSession>, opts: AskUsageOptions) -> Option<Value> {
    let session = session.filter(|s| !s.token().is_empty())?;
    let signal = Signal::new();
    let timer = {
        let signal = signal.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(opts.timeout_ms)).await;
            signal.cancel();
        })
    };
    let mut result = Map::new();
    // Chats auf der ASK-Webseite: eigene Chatliste, seitenweise (ältere Versionen liefern alles auf einmal)
    let chats = async {
        let mut seen: HashSet<String> = HashSet::new();
        let mut recent = 0;
        let mut more = false;
        for page in 1..=PAGES_MAX {
            let list = get_json(session, &format!("/api/v1/chats/?page={page}"), &signal).await?;
            let Some(list) = list.as_ref().and_then(Value::as_array).filter(|l| !l.is_empty()) else { break };
            let fresh: Vec<&Value> = list.iter().filter(|c| c.get("id").is_some_and(truthy) && !seen.contains(&js_string(&c["id"]))).collect();
            if fresh.is_empty() {
                break; // Server kennt «page» nicht und liefert dasselbe
            }
            for c in fresh {
                seen.insert(js_string(&c["id"]));
                let stamp = c.get("updated_at").filter(|v| truthy(v)).or_else(|| c.get("created_at"));
                if opts.now - seconds(stamp) < DAYS as i64 * DAY_MS {
                    recent += 1;
                }
            }
            if page == PAGES_MAX {
                more = true;
            }
        }
        anyhow::Ok((seen.len(), recent, more))
    };
    // nicht erreichbar oder nicht erlaubt: der Abschnitt bleibt leer
    if let Ok((count, recent, more)) = chats.await {
        if more {
            result.insert("more".into(), json!(true));
        }
        if count > 0 {
            result.insert("webChats".into(), json!(count));
            result.insert("webChats30".into(), json!(recent));
        }
    }
    for p in USAGE_PATHS {
        if signal.is_cancelled() {
            break;
        }
        // nächste Schnittstelle
        if let Ok(data) = get_json(session, p, &signal).await {
            let nums = usage_numbers(&data.unwrap_or(Value::Null));
            if !nums.is_empty() {
                result.extend(nums);
                break;
            }
        }
    }
    timer.abort();
    if result.is_empty() { None } else { Some(Value::Object(result)) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::askapi::test_http::{client, serve, Resp};
    use std::fs;
    use std::path::PathBuf;

    const DAY: i64 = DAY_MS;

    fn now() -> i64 {
        Local.with_ymd_and_hms(2026, 11, 25, 15, 0, 0).single().unwrap().timestamp_millis()
    }

    fn stats(tokens: i64, total_ms: i64, estimated: bool) -> Value {
        json!({ "tokens": tokens, "genMs": total_ms - 500, "totalMs": total_ms, "firstMs": 500, "promptTokens": 4000, "estimated": estimated })
    }

    fn chat_a() -> Value {
        let now = now();
        json!({ "id": "a", "title": "Auswertung", "created": now - 3 * DAY, "updated": now - 2 * DAY, "messages": [
            { "role": "user", "content": "Werte aus", "time": now - 3 * DAY, "changes": { "changed": ["x.docx"], "added": ["a.py", "b.xlsx"], "removed": [], "counts": [1, 2, 0] } },
            { "role": "assistant", "content": "", "time": now - 3 * DAY + 1000, "stats": stats(200, 4000, false), "tool_calls": [{}, {}] },
            { "role": "tool", "content": "ok" },
            { "role": "tool", "content": "ok" },
            { "role": "assistant", "content": "Fertig", "time": now - 3 * DAY + 9000, "stats": stats(100, 2000, true) },
            { "role": "user", "auto": true, "content": "Weiter am Ziel", "time": now - 3 * DAY + 10000 },
            { "role": "user", "content": "Nochmals", "time": now - 2 * DAY, "undone": true, "changes": { "changed": [], "added": ["c.txt"], "removed": [], "counts": [0, 1, 0] } },
            { "role": "assistant", "content": "Gut", "time": now - 2 * DAY + 500, "stats": stats(50, 1000, false) },
            { "role": "note", "kind": "notice", "content": "Hinweis" },
        ] })
    }

    fn folder_with(chats: &[Value]) -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join(".ask").join("chats");
        fs::create_dir_all(&dir).unwrap();
        for c in chats {
            fs::write(dir.join(format!("{}.json", c["id"].as_str().unwrap())), c.to_string()).unwrap();
        }
        root
    }

    #[test]
    fn nutzung_ein_chat_ergibt_auftraege_schritte_token_zeit_und_dateien() {
        let now = now();
        let s = summarize_chat(&chat_a());
        assert_eq!(s.chats, 1);
        assert_eq!(s.tasks, 2, "automatische Nachrichten zählen nicht");
        assert_eq!(s.steps, 2);
        assert_eq!(s.tokens, 350);
        assert_eq!(s.estimated_tokens, 100);
        assert_eq!(s.model_ms, 7000);
        assert_eq!(s.files, Files { added: 2, changed: 1, removed: 0 }, "rückgängig gemachter Auftrag zählt nicht");
        assert_eq!(s.days[&day_key(now - 3 * DAY)], Day { tasks: 1, tokens: 300 });
        assert_eq!(s.days[&day_key(now - 2 * DAY)], Day { tasks: 1, tokens: 50 });
    }

    #[test]
    fn nutzung_ohne_counts_zaehlen_die_listen_ohne_zeit_das_datum_des_chats() {
        let now = now();
        let s = summarize_chat(&json!({ "id": "b", "updated": now, "messages": [
            { "role": "user", "content": "x", "changes": { "changed": ["a"], "added": ["b", "c"], "removed": ["d"] } },
            { "role": "assistant", "stats": { "tokens": "kaputt", "totalMs": -5 } },
        ] }));
        assert_eq!(s.files, Files { added: 2, changed: 1, removed: 1 });
        assert_eq!(s.tokens, 0);
        assert_eq!(s.model_ms, 0);
        assert_eq!(s.days[&day_key(now)], Day { tasks: 1, tokens: 0 });
        assert_eq!(s.last, now);
    }

    #[test]
    fn nutzung_ordner_mit_kaputten_dateien_und_zwischenspeicher() {
        let now = now();
        let root = folder_with(&[chat_a(), json!({ "id": "leer", "updated": now, "messages": [] })]);
        let chats = root.path().join(".ask").join("chats");
        fs::write(chats.join("kaputt.json"), "{ halb").unwrap();
        fs::write(chats.join("notiz.txt"), "kein Chat").unwrap();
        let u = folder_usage(root.path());
        assert_eq!(u.chats, 2);
        assert_eq!(u.tasks, 2);
        // Geänderte Datei wird neu gelesen, unveränderte kommt aus dem Zwischenspeicher
        let file = chats.join("leer.json");
        fs::write(&file, json!({ "id": "leer", "updated": now, "messages": [{ "role": "user", "content": "neu", "time": now }] }).to_string()).unwrap();
        let later = SystemTime::now() + Duration::from_secs(5);
        fs::File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
        assert_eq!(folder_usage(root.path()).tasks, 3);
    }

    #[test]
    fn nutzung_summe_ueber_projekte_und_die_letzten_30_tage() {
        let now = now();
        let one = folder_with(&[chat_a()]);
        let two = folder_with(&[json!({ "id": "z", "updated": now, "messages": [
            { "role": "user", "content": "heute", "time": now - 1000 },
            { "role": "assistant", "content": "ok", "time": now - 500, "stats": stats(40, 800, false) },
            { "role": "user", "content": "alt", "time": now - 60 * DAY },
        ] })]);
        let missing: PathBuf = one.path().join("gibt-es-nicht");
        let r = aggregate(
            &[
                json!({ "folder": one.path(), "name": "Eins", "color": "#0082B4" }),
                json!({ "folder": two.path(), "name": "Zwei", "color": "#F28855" }),
                json!({ "folder": missing, "name": "Weg" }),
                Value::Null,
            ],
            AggregateOptions { now, days: DAYS },
        );
        assert_eq!(r.projects.len(), 3);
        // meiste Aufträge zuerst, bei Gleichstand das zuletzt benutzte
        assert_eq!(r.projects.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["Zwei", "Eins", "Weg"]);
        assert!(r.projects[2].missing);
        assert_eq!(r.total.chats, 2);
        assert_eq!(r.total.tasks, 4);
        assert_eq!(r.total.tokens, 390);
        assert_eq!(r.total.files.added, 2);
        assert_eq!(r.days.len(), 30);
        assert_eq!(r.days[29].date, day_key(now));
        assert_eq!(r.days[29], DayPoint { date: day_key(now), tasks: 1, tokens: 40 });
        assert_eq!(r.days[26].tasks, 1);
        assert_eq!(r.days.iter().map(|d| d.tasks).sum::<i64>(), 3, "Auftrag vor 60 Tagen nicht in den 30 Tagen");
        assert!(r.projects[1].summary.days.is_empty(), "Tageswerte nur gesamt");
        let out = serde_json::to_value(&r).unwrap();
        assert!(out["projects"][1].get("days").is_none());
        assert_eq!(out["projects"][1]["name"], "Eins");
        assert_eq!(out["projects"][1]["estimatedTokens"], 100);
    }

    #[test]
    fn nutzung_viele_chats_bleiben_schnell() {
        let chats: Vec<Value> = (0..400)
            .map(|i| {
                let mut c = chat_a();
                c["id"] = json!(format!("c{i}"));
                c
            })
            .collect();
        let root = folder_with(&chats);
        folder_usage(root.path());
        let t = std::time::Instant::now();
        let u = folder_usage(root.path());
        assert_eq!(u.chats, 400);
        assert!(t.elapsed().as_millis() < 500, "zweiter Durchgang aus dem Zwischenspeicher: {} ms", t.elapsed().as_millis());
    }

    #[test]
    fn nutzung_von_ask_bekannte_zahlen_sonst_nichts() {
        let eq = |data: Value, expected: Value| assert_eq!(Value::Object(usage_numbers(&data)), expected);
        eq(json!({ "total_tokens": 1200, "prompt_tokens": 800, "completion_tokens": 400, "x": 5 }), json!({ "tokens": 1200, "inputTokens": 800, "outputTokens": 400 }));
        eq(json!({ "items": [{ "message_count": 3 }, { "message_count": 4 }] }), json!({ "messages": 7 }));
        eq(json!({ "detail": "Not Found" }), json!({}));
        eq(Value::Null, json!({}));
    }

    /// ASK-Attrappe: Pfade mit Antwort, nicht genannte geben 404, `None` schliesst die Verbindung
    async fn fake_session(routes: Vec<(&'static str, Option<Value>)>, token: &str) -> (AskSession, crate::askapi::test_http::TestServer) {
        let server = serve(move |req| match routes.iter().find(|(p, _)| *p == req.path) {
            None => Resp::json(404, &json!({})),
            Some((_, None)) => Resp::Drop,
            Some((_, Some(v))) => Resp::json(200, v),
        })
        .await;
        let ask = AskSession::with_client(client());
        ask.set_base(&server.url);
        ask.set_token(token);
        (ask, server)
    }

    #[tokio::test]
    async fn nutzung_von_ask_chats_der_webseite_seitenweise_fehler_still() {
        let now = now();
        let sec = |ms: i64| ms / 1000;
        let page1: Vec<Value> = (0..3).map(|i| json!({ "id": format!("w{i}"), "updated_at": sec(now - i * DAY) })).collect();
        let page2 = json!([{ "id": "w9", "updated_at": sec(now - 90 * DAY) }]);
        let opts = AskUsageOptions { now, timeout_ms: 5000 };
        let (s, _srv) = fake_session(
            vec![
                ("/api/v1/chats/?page=1", Some(json!(page1))),
                ("/api/v1/chats/?page=2", Some(page2)),
                ("/api/v1/chats/?page=3", Some(json!([]))),
                ("/api/v1/users/user/usage", Some(json!({ "total_tokens": 5000 }))),
            ],
            "t",
        )
        .await;
        assert_eq!(ask_usage(Some(&s), opts).await, Some(json!({ "webChats": 4, "webChats30": 3, "tokens": 5000 })));

        // Server ohne «page»: dieselbe Liste wieder, dann aufhören
        let (same, srv) = fake_session(vec![("/api/v1/chats/?page=1", Some(json!(page1))), ("/api/v1/chats/?page=2", Some(json!(page1)))], "t").await;
        assert_eq!(ask_usage(Some(&same), opts).await, Some(json!({ "webChats": 3, "webChats30": 3 })));
        assert!(!srv.paths().contains(&"/api/v1/chats/?page=3".to_string()));

        // Nichts erlaubt oder nicht erreichbar: None
        let (offline, _srv) = fake_session(vec![("/api/v1/chats/?page=1", None)], "t").await;
        assert_eq!(ask_usage(Some(&offline), opts).await, None);
        let (none, _srv) = fake_session(vec![], "t").await;
        assert_eq!(ask_usage(Some(&none), opts).await, None);
        let (no_token, _srv) = fake_session(vec![], "").await;
        assert_eq!(ask_usage(Some(&no_token), AskUsageOptions::default()).await, None);
        assert_eq!(ask_usage(None, AskUsageOptions::default()).await, None);
    }
}
