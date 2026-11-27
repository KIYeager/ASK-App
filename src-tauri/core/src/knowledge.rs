//! Gedächtnis (memory.rs, ENTSCHEIDE E50) und eigene Skills (skills.rs, E51) in der App: Ablage in userData,
//! Abschnitte im Systemprompt, Merken nach dem Auftrag, Aufruf mit /name und die Schnittstelle zur Oberfläche.
//!
//! Öffentliche Schnittstelle (Port von src/main/knowledge.js):
//! - `Knowledge::new(KnowledgeConfig)`; Felder `memory` und `skills` sind öffentlich
//! - `memory_on()`, `for_agent() -> KnowledgeCtx`, `system_prompt(folder)`, `skill_for(text) -> Option<SkillCall>`
//! - `after_turn(folder, chat_id, chat_title, messages, ev)`: braucht eine tokio-Laufzeit (startet eine Aufgabe)
//! - `wait_idle()`: wartet, bis keine Abfrage zum Merken mehr läuft (in JS `await k.learning`)
//! - `handle(channel, args, host, confirm)`: die Kanäle `memory:*` und `skills:*` (in JS `setupIpc`);
//!   `None`, wenn der Kanal nicht zu diesem Modul gehört. `Err(text)` entspricht einem geworfenen Fehler.
//!
//! Abweichungen vom JavaScript, die aus den Typen folgen: `settings` ist ein Trait (`KnowledgeSettings`, Standard
//! `GlobalSettings` über settings.rs), `after_turn` bekommt statt des Agenten die Daten des Chats als Parameter
//! (`agent.chat.id`, `agent.chat.title`, `agent.chat.messages`), und `projects`/`onSettings` aus `setupIpc` sind
//! Felder von `KnowledgeConfig`.

use crate::askapi::{ChatApi, ChatRequest};
use crate::host::{ConfirmFn, ConfirmOptions, Filters, Host, SendFn};
use crate::memory::{self, clean_tags, ChatRef, CompleteFn, ItemPatch, LearnParams, Memory, NewItem};
use crate::memorytools::KnowledgeCtx;
use crate::mods::Mods;
use crate::skills::{invocation, slugify, SaveOptions, SkillInput, Skills};
use crate::util::{js_string, str_of, take_chars, truthy};
use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// So viel der Anleitung geht mit /name an das Modell
pub const SKILL_TEXT_MAX: usize = 12000;

/// Zeitlimit einer Abfrage zum Merken
const LEARN_TIMEOUT: Duration = Duration::from_secs(90);

/// Die Einstellungen, die Knowledge braucht (in JS `settings.get` und `settings.set`)
pub trait KnowledgeSettings: Send + Sync {
    fn get(&self, key: &str) -> Value;
    fn set(&self, values: Value);
}

/// Die Einstellungen der App (settings.rs)
pub struct GlobalSettings;

impl KnowledgeSettings for GlobalSettings {
    fn get(&self, key: &str) -> Value {
        crate::settings::get(key)
    }
    fn set(&self, values: Value) {
        crate::settings::set(values);
    }
}

pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;

/// Angaben für `Knowledge::new` (die Optionen des Konstruktors in JS plus die von `setupIpc`)
#[derive(Clone)]
pub struct KnowledgeConfig {
    /// userData: darin `gedaechtnis.json` und `skills/`
    pub dir: PathBuf,
    pub settings: Arc<dyn KnowledgeSettings>,
    /// ASK für das Merken; `None`: es wird nichts gemerkt
    pub ask: Option<Arc<dyn ChatApi>>,
    pub get_model: Arc<dyn Fn() -> String + Send + Sync>,
    /// `send(kanal, daten)` an die Oberfläche
    pub send: SendFn,
    pub log: LogFn,
    /// Projekte für `memory:list` (in JS `projects()` aus `setupIpc`)
    pub projects: Arc<dyn Fn() -> Value + Send + Sync>,
    /// Nach `memory:enable` (in JS `onSettings`)
    pub on_settings: Arc<dyn Fn() + Send + Sync>,
    /// Mitgelieferte Skills (resources/standard-skills, ENTSCHEIDE E70); `None`: keine
    pub standard_skills: Option<PathBuf>,
    /// Kataloge auf GitHub (catalog.rs, E71) für die Werkzeuge katalog_suchen und katalog_hinzufuegen
    pub catalog: Option<Arc<crate::catalog::Catalog>>,
    pub catalog_sources: Arc<dyn Fn() -> Vec<String> + Send + Sync>,
}

impl KnowledgeConfig {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        KnowledgeConfig {
            dir: dir.into(),
            settings: Arc::new(GlobalSettings),
            ask: None,
            get_model: Arc::new(String::new),
            send: Arc::new(|_, _| {}),
            log: Arc::new(|_| {}),
            projects: Arc::new(|| json!([])),
            on_settings: Arc::new(|| {}),
            standard_skills: None,
            catalog: None,
            catalog_sources: Arc::new(Vec::new),
        }
    }
}

/// Namen der ausgeschalteten mitgelieferten Skills (Einstellung `skillsOff`)
fn skills_off(settings: &Arc<dyn KnowledgeSettings>) -> Vec<String> {
    settings.get("skillsOff").as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// «/name …» in der Eingabe: der Skill mit gekürzter Anleitung (`{ name, text }`)
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SkillCall {
    pub name: String,
    pub text: String,
}

struct Job {
    folder: String,
    chat_id: String,
    chat_title: String,
    user: String,
    answer: String,
}

#[derive(Default)]
struct LearnState {
    learning: bool, // laufende Abfrage (höchstens eine)
    next: Option<Job>, // wartender Auftrag, nur der neueste
    done: HashSet<String>, // schon ausgewertete Eingaben (Chat und Zeit)
    order: VecDeque<String>, // Reihenfolge der Einträge in done
}

pub struct Knowledge {
    pub memory: Arc<Memory>,
    pub skills: Arc<Skills>,
    /// Mods in userData/mods (mods.rs, E72)
    pub mods: Arc<Mods>,
    cfg: KnowledgeConfig,
    state: Arc<Mutex<LearnState>>,
}

fn failure(err: impl std::fmt::Display) -> Value {
    json!({ "ok": false, "reason": err.to_string() })
}

/// `safe` in JS: `{ ok: true, ...Ergebnis }` oder `{ ok: false, reason }`
fn safe(result: Result<Value>) -> Result<Value, String> {
    Ok(match result {
        Ok(Value::Object(m)) => {
            let mut out = Map::new();
            out.insert("ok".into(), Value::Bool(true));
            out.extend(m);
            Value::Object(out)
        }
        Ok(_) => json!({ "ok": true }),
        Err(err) => failure(err),
    })
}

fn scope_of(v: Option<&Value>) -> (String, Option<String>) {
    let null = Value::Null;
    let v = v.unwrap_or(&null);
    let scope = v.get("scope").and_then(Value::as_str);
    match v.get("project") {
        Some(p) if scope == Some("projekt") && truthy(p) => ("projekt".to_string(), Some(js_string(p))),
        _ => ("konto".to_string(), None),
    }
}

fn to_value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

impl Knowledge {
    pub fn new(cfg: KnowledgeConfig) -> Self {
        Knowledge {
            memory: Arc::new(Memory::new(cfg.dir.join("gedaechtnis.json"))),
            skills: Arc::new(match &cfg.standard_skills {
                Some(std_dir) => {
                    let settings = cfg.settings.clone();
                    Skills::with_standard(cfg.dir.join("skills"), std_dir.clone(), Arc::new(move || skills_off(&settings)))
                }
                None => Skills::new(cfg.dir.join("skills")),
            }),
            mods: Arc::new(Mods::new(cfg.dir.join("mods"))),
            cfg,
            state: Arc::new(Mutex::new(LearnState::default())),
        }
    }

    /// Schalter in den Einstellungen, Standard an
    pub fn memory_on(&self) -> bool {
        memory_on(&self.cfg)
    }

    /// Für agent.js (Werkzeuge merken, vergessen, skill_lesen, skill_erstellen)
    pub fn for_agent(&self) -> KnowledgeCtx {
        let cfg = self.cfg.clone();
        let send = self.cfg.send.clone();
        KnowledgeCtx {
            memory: Some(self.memory.clone()),
            skills: Some(self.skills.clone()),
            mods: Some(self.mods.clone()),
            catalog: self.cfg.catalog.clone(),
            catalog_sources: Some(self.cfg.catalog_sources.clone()),
            memory_on: Some(Arc::new(move || memory_on(&cfg))),
            changed: Some(Arc::new(move || send("memory:changed", json!({})))),
            mods_changed: Some({
                let (send, on_settings) = (self.cfg.send.clone(), self.cfg.on_settings.clone());
                Arc::new(move || { send("mods:changed", json!({})); on_settings(); })
            }),
        }
    }

    /// Abschnitte für den Systemprompt
    pub fn system_prompt(&self, folder: &str) -> String {
        let mut text = String::new();
        if self.memory_on() {
            text.push_str(&self.memory.prompt(folder)); // Gedächtnis nicht lesbar: leer (das Modul meldet nichts)
        }
        text.push_str(&self.skills.prompt());
        text.push_str(&self.mods.prompt());
        text
    }

    /// «/name …» in der Eingabe: Skill für diesen Auftrag, sonst `None`
    pub fn skill_for(&self, text: &str) -> Option<SkillCall> {
        let inv = invocation(text)?;
        let s = self.skills.get(&inv.slug)?;
        if s.slug != inv.slug {
            return None;
        }
        let body = if s.body.chars().count() > SKILL_TEXT_MAX {
            format!("{}\n… (gekürzt)", take_chars(&s.body, SKILL_TEXT_MAX))
        } else {
            s.body
        };
        Some(SkillCall { name: s.slug, text: body })
    }

    /// Nach einem fertigen Auftrag (Ereignis done von agent.js): im Hintergrund merken, was dauerhaft wichtig ist.
    /// Nie nach Stopp oder Fehler, höchstens einmal pro Eingabe, immer nur eine Abfrage gleichzeitig.
    /// `chat_id`, `chat_title` und `messages` sind `agent.chat.id`, `.title` und `.messages`, `ev` ist das Ereignis
    /// (`Value::Null` wie ein fehlendes Ereignis). Muss in einer tokio-Laufzeit aufgerufen werden.
    pub fn after_turn(&self, folder: &str, chat_id: &str, chat_title: &str, messages: &[Value], ev: &Value) {
        if ev.is_null() || truthy(ev.get("stopped").unwrap_or(&Value::Null)) || truthy(ev.get("failed").unwrap_or(&Value::Null))
            || !self.memory_on() || folder.is_empty()
        {
            return;
        }
        let Some(u) = messages.iter().rposition(|m| str_of(m, "role") == "user" && !truthy(m.get("auto").unwrap_or(&Value::Null))) else { return };
        let Some(a) = messages.iter().rposition(|m| {
            str_of(m, "role") == "assistant" && !truthy(m.get("stopped").unwrap_or(&Value::Null)) && !str_of(m, "content").trim().is_empty()
        }) else { return };
        if a < u {
            return;
        }
        let time = messages[u].get("time").map(js_string).unwrap_or_else(|| "undefined".to_string());
        let key = format!("{chat_id}:{time}");
        let job = Job {
            folder: folder.to_string(),
            chat_id: chat_id.to_string(),
            chat_title: chat_title.to_string(),
            user: str_of(&messages[u], "content"),
            answer: str_of(&messages[a], "content"),
        };
        {
            let mut st = self.state.lock();
            if st.done.contains(&key) {
                return;
            }
            st.done.insert(key.clone());
            st.order.push_back(key);
            if st.done.len() > 500 {
                while st.order.len() > 200 {
                    if let Some(old) = st.order.pop_front() {
                        st.done.remove(&old);
                    }
                }
                st.done = st.order.iter().cloned().collect();
            }
            if st.learning {
                st.next = Some(job);
                return;
            }
            st.learning = true;
        }
        self.run(job);
    }

    fn run(&self, job: Job) {
        let cfg = self.cfg.clone();
        let memory = self.memory.clone();
        let state = self.state.clone();
        tokio::spawn(async move {
            let mut job = job;
            loop {
                run_job(&cfg, &memory, &job).await;
                let next = {
                    let mut st = state.lock();
                    match st.next.take() {
                        Some(n) if memory_on(&cfg) => Some(n),
                        _ => {
                            st.learning = false;
                            None
                        }
                    }
                };
                match next {
                    Some(n) => job = n,
                    None => break,
                }
            }
        });
    }

    /// Wartet, bis keine Abfrage zum Merken mehr läuft (in JS `await k.learning`)
    pub async fn wait_idle(&self) {
        while self.state.lock().learning {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    // ---------- Schnittstelle zur Oberfläche ----------

    /// Die Kanäle `memory:*` und `skills:*` (in JS `setupIpc`). `None`: nicht mein Kanal.
    pub async fn handle(&self, channel: &str, args: &[Value], host: &dyn Host, confirm: &ConfirmFn) -> Option<Result<Value, String>> {
        let null = Value::Null;
        let arg = |i: usize| args.get(i).unwrap_or(&null);
        let changed = || (self.cfg.send)("memory:changed", json!({}));
        let id_of = |v: &Value| js_string(v); // String(id || '')
        let text_arg = |v: &Value| if truthy(v) { js_string(v) } else { String::new() };
        Some(match channel {
            "memory:list" => Ok(json!({ "enabled": self.memory_on(), "items": to_value(&self.memory.list()), "projects": (self.cfg.projects)() })),
            "memory:add" => {
                let v = arg(0);
                let (scope, project) = scope_of(args.first());
                safe(self.memory.add(NewItem {
                    text: str_of(v, "text"),
                    tags: json!(clean_tags(v.get("tags").unwrap_or(&null))),
                    scope,
                    project,
                    source: "hand".into(),
                    ..Default::default()
                }).map(|item| json!({ "item": item })))
            }
            "memory:update" => {
                let v = arg(1);
                let mut patch = ItemPatch {
                    text: v.get("text").map(js_string),
                    tags: v.get("tags").cloned(),
                    ..Default::default()
                };
                if truthy(v.get("scope").unwrap_or(&null)) {
                    let (scope, project) = scope_of(args.get(1));
                    patch.scope = Some(scope);
                    patch.project = project;
                }
                safe(match self.memory.update(&text_arg(arg(0)), patch) {
                    Ok(Some(item)) => Ok(json!({ "item": item })),
                    Ok(None) => Err(anyhow!("Den Eintrag gibt es nicht mehr.")),
                    Err(e) => Err(e),
                })
            }
            "memory:remove" => self.memory.remove(&text_arg(arg(0))).map(Value::Bool).map_err(|e| e.to_string()),
            "memory:clear" => {
                let n = self.memory.list().len();
                if n == 0 {
                    return Some(Ok(Value::Bool(false)));
                }
                let message = "Alles vergessen?".to_string();
                let detail = format!("ASK vergisst alle {n} Einträge im Gedächtnis, über dich und alle Projekte. Das lässt sich nicht rückgängig machen.");
                if !confirm(message, detail, ConfirmOptions { danger: true, ..Default::default() }).await.ok {
                    return Some(Ok(Value::Bool(false)));
                }
                match self.memory.clear() {
                    Ok(()) => {
                        changed();
                        Ok(Value::Bool(true))
                    }
                    Err(e) => Err(e.to_string()),
                }
            }
            "memory:enable" => {
                self.cfg.settings.set(json!({ "memory": truthy(arg(0)) }));
                (self.cfg.on_settings)();
                Ok(Value::Null)
            }
            "skills:list" => Ok(to_value(&self.skills.list())),
            // arg(1) «standard»: den mitgelieferten zeigen, auch wenn er aus oder durch einen eigenen ersetzt ist
            "skills:get" => {
                let name = text_arg(arg(0));
                let found = if js_string(arg(1)) == "standard" { self.skills.standard(&name) } else { self.skills.own(&name).or_else(|| self.skills.get(&name)) };
                Ok(found.map(|s| to_value(&s)).unwrap_or(Value::Null))
            }
            // Mitgelieferten Skill ein- oder ausschalten (E70)
            "skills:enable" => {
                let name = slugify(&text_arg(arg(0)));
                if self.skills.standard(&name).is_none() {
                    return Some(Ok(Value::Bool(false)));
                }
                let mut off: Vec<String> = skills_off(&self.cfg.settings).into_iter().filter(|n| *n != name).collect();
                if !truthy(arg(1)) {
                    off.push(name);
                }
                self.cfg.settings.set(json!({ "skillsOff": off }));
                Ok(Value::Bool(true))
            }
            "skills:save" => {
                let v = arg(0);
                safe(self.skills.save(
                    &SkillInput { name: str_of(v, "name"), description: str_of(v, "description"), body: str_of(v, "body") },
                    &SaveOptions { previous: text_arg(arg(1)), overwrite: false },
                ).map(|skill| json!({ "skill": skill })))
            }
            "skills:remove" => {
                let Some(s) = self.skills.own(&text_arg(arg(0))) else { return Some(Ok(Value::Bool(false))) };
                let message = format!("Skill «{}» löschen?", s.slug);
                let detail = "Die Anleitung wird von diesem Laptop entfernt. Exportiere sie vorher, wenn du sie behalten willst.".to_string();
                if !confirm(message, detail, ConfirmOptions { danger: true, ..Default::default() }).await.ok {
                    return Some(Ok(Value::Bool(false)));
                }
                self.skills.remove(&s.slug).map(Value::Bool).map_err(|e| e.to_string())
            }
            "skills:export" => safe(self.export_skill(&id_of(arg(0)), host).await),
            "skills:import" => safe(self.import_skill(host, confirm).await),
            _ => return None,
        })
    }

    async fn export_skill(&self, name: &str, host: &dyn Host) -> Result<Value> {
        let text = self.skills.export_text(name).filter(|t| !t.is_empty()).ok_or_else(|| anyhow!("Den Skill gibt es nicht mehr."))?;
        let filters: Filters = vec![("Skill (Markdown)".into(), vec!["md".into()])];
        let Some(path) = host.pick_save_file("Skill exportieren", &format!("{}.md", slugify(name)), filters).await.filter(|p| !p.is_empty()) else {
            return Ok(json!({ "canceled": true }));
        };
        std::fs::write(&path, text)?;
        Ok(json!({ "path": path }))
    }

    async fn import_skill(&self, host: &dyn Host, confirm: &ConfirmFn) -> Result<Value> {
        let filters: Filters = vec![("Skill (Markdown)".into(), vec!["md".into(), "txt".into()])];
        let Some(file) = host.pick_open_file("Skill importieren", filters).await.filter(|p| !p.is_empty()) else {
            return Ok(json!({ "canceled": true }));
        };
        if std::fs::metadata(&file)?.len() > 100 * 1024 {
            return Err(anyhow!("Die Datei ist grösser als 100 KB."));
        }
        let data = Skills::read_import(&std::fs::read(&file)?)?;
        let exists = self.skills.own(&data.name).is_some();
        if exists {
            let message = format!("Skill «{}» ersetzen?", data.name);
            let detail = "Es gibt schon einen Skill mit diesem Namen. Die importierte Datei ersetzt ihn.".to_string();
            if !confirm(message, detail, ConfirmOptions::default()).await.ok {
                return Ok(json!({ "canceled": true }));
            }
        }
        let skill = self.skills.save(
            &SkillInput { name: data.name, description: data.description, body: data.body },
            &SaveOptions { previous: String::new(), overwrite: true },
        )?;
        Ok(json!({ "skill": skill }))
    }
}

fn memory_on(cfg: &KnowledgeConfig) -> bool {
    cfg.settings.get("memory") != Value::Bool(false)
}

/// Eine Abfrage zum Merken; Fehler werden still ins Protokoll geschrieben (kein Fehler im Chat)
async fn run_job(cfg: &KnowledgeConfig, memory: &Arc<Memory>, job: &Job) {
    let Some(ask) = cfg.ask.clone() else { return }; // ohne ASK gibt `learn` null zurück
    let model = (cfg.get_model)();
    let complete: CompleteFn = Arc::new(move |model, messages| {
        let ask = ask.clone();
        Box::pin(async move {
            let completion = ask
                .chat(ChatRequest { model, messages, tools: None, web_search: false, thinking: "tief".into(), ..Default::default() })
                .await?;
            Ok(completion.content)
        })
    });
    let learned = tokio::time::timeout(
        LEARN_TIMEOUT,
        memory::learn(LearnParams {
            memory,
            complete: &complete,
            model: &model,
            folder: &job.folder,
            chat: Some(ChatRef { id: job.chat_id.clone(), title: job.chat_title.clone() }),
            user: &job.user,
            answer: &job.answer,
        }),
    )
    .await;
    match learned {
        Ok(Ok(Some(res))) if !res.added.is_empty() || !res.removed.is_empty() => {
            (cfg.send)(
                "memory:changed",
                json!({
                    "chatId": job.chat_id,
                    "folder": job.folder,
                    "added": res.added.iter().map(|i| i.text.clone()).collect::<Vec<_>>(),
                    "removed": res.removed.len(),
                }),
            );
        }
        Ok(Ok(_)) => {}
        Ok(Err(err)) => (cfg.log)(&format!("Gedächtnis: {err}")),
        Err(_) => (cfg.log)("Gedächtnis: Zeitüberschreitung nach 90 Sekunden"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::askapi::Completion;
    use crate::host::{fixed_confirm, Confirmed, NullHost};
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MapSettings(Mutex<Map<String, Value>>);

    impl KnowledgeSettings for MapSettings {
        fn get(&self, key: &str) -> Value {
            self.0.lock().get(key).cloned().unwrap_or(Value::Null)
        }
        fn set(&self, values: Value) {
            if let Value::Object(m) = values {
                self.0.lock().extend(m);
            }
        }
    }

    struct FakeAsk(Arc<AtomicUsize>);

    #[async_trait]
    impl ChatApi for FakeAsk {
        async fn chat(&self, _req: ChatRequest) -> Result<Completion> {
            self.0.fetch_add(1, Ordering::SeqCst);
            let mut c = Completion::new();
            c.content = r#"{"neu":[{"text":"Lernt für die Prüfung im Januar.","bereich":"konto"}]}"#.into();
            Ok(c)
        }
    }

    fn proj() -> String {
        std::env::temp_dir().join("ASK-Projekte").join("Budget").to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn nach_dem_auftrag_hoechstens_einmal_nie_nach_stopp_fehler_oder_ausgeschaltet() {
        let dir = tempfile::tempdir().unwrap();
        let settings = Arc::new(MapSettings(Mutex::new(Map::new())));
        settings.set(json!({ "memory": true }));
        let calls = Arc::new(AtomicUsize::new(0));
        let sent: Arc<Mutex<Vec<(String, Value)>>> = Arc::new(Mutex::new(Vec::new()));
        let s2 = sent.clone();
        let mut cfg = KnowledgeConfig::new(dir.path());
        cfg.settings = settings.clone();
        cfg.ask = Some(Arc::new(FakeAsk(calls.clone())));
        cfg.get_model = Arc::new(|| "qwen".to_string());
        cfg.send = Arc::new(move |ch, d| s2.lock().push((ch.to_string(), d)));
        let k = Knowledge::new(cfg);
        let mut messages = vec![
            json!({ "role": "user", "content": "Ich lerne für die Prüfung im Januar, hilf mir beim Plan.", "time": 5 }),
            json!({ "role": "assistant", "content": "", "tool_calls": [{}] }),
            json!({ "role": "tool", "content": "x" }),
            json!({ "role": "assistant", "content": "Hier ist dein Lernplan bis Januar." }),
        ];
        let p = proj();
        k.after_turn(&p, "c1", "Prüfung", &messages, &json!({ "type": "done", "stopped": true }));
        k.after_turn(&p, "c1", "Prüfung", &messages, &json!({ "type": "done", "failed": true }));
        k.after_turn("", "c1", "Prüfung", &messages, &json!({ "type": "done" }));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        k.after_turn(&p, "c1", "Prüfung", &messages, &json!({ "type": "done" }));
        k.after_turn(&p, "c1", "Prüfung", &messages, &json!({ "type": "done" })); // dieselbe Eingabe: kein zweites Mal
        k.wait_idle().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            sent.lock().last().cloned().unwrap(),
            ("memory:changed".to_string(), json!({ "chatId": "c1", "folder": p, "added": ["Lernt für die Prüfung im Januar."], "removed": 0 }))
        );
        assert!(k.system_prompt(&p).contains("Lernt für die Prüfung"));
        settings.set(json!({ "memory": false }));
        assert!(!k.system_prompt(&p).contains("Gedächtnis"), "ausgeschaltet: nichts im Systemprompt");
        messages.push(json!({ "role": "user", "content": "Noch etwas Neues für später merken bitte.", "time": 9 }));
        messages.push(json!({ "role": "assistant", "content": "Gut, mache ich so." }));
        k.after_turn(&p, "c1", "Prüfung", &messages, &json!({ "type": "done" }));
        k.wait_idle().await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(dir.path().join("gedaechtnis.json").exists(), "nur in userData");
    }

    fn plain(dir: &std::path::Path) -> Knowledge {
        let mut cfg = KnowledgeConfig::new(dir);
        cfg.settings = Arc::new(MapSettings(Mutex::new(Map::new())));
        Knowledge::new(cfg)
    }

    #[test]
    fn aufruf_mit_name_nur_vorhandene_skills_anleitung_fuer_diesen_auftrag() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(invocation("/bericht Woche 47").unwrap().rest, "Woche 47");
        assert_eq!(invocation("  /Bericht").unwrap().slug, "bericht");
        assert!(invocation("bitte /bericht").is_none());
        assert!(invocation("/").is_none());
        let k = plain(dir.path());
        k.skills.save(&SkillInput { name: "bericht".into(), description: "Bericht".into(), body: "Schreib eine Seite.".into() }, &SaveOptions::default()).unwrap();
        assert_eq!(k.skill_for("/bericht Woche 47"), Some(SkillCall { name: "bericht".into(), text: "Schreib eine Seite.".into() }));
        assert_eq!(k.skill_for("/unbekannt etwas"), None);
        assert_eq!(k.skill_for("Hallo"), None);
        assert!(k.system_prompt("").contains("- bericht: Bericht"));
    }

    #[test]
    fn lange_anleitung_wird_gekuerzt() {
        let dir = tempfile::tempdir().unwrap();
        let k = plain(dir.path());
        k.skills.save(&SkillInput { name: "lang".into(), description: "x".into(), body: "a".repeat(20000) }, &SaveOptions::default()).unwrap();
        let s = k.skill_for("/lang").unwrap();
        assert!(s.text.ends_with("\n… (gekürzt)"));
        assert_eq!(s.text.chars().count(), SKILL_TEXT_MAX + "\n… (gekürzt)".chars().count());
    }

    #[tokio::test]
    async fn schnittstelle_gedaechtnis_und_skills() {
        let dir = tempfile::tempdir().unwrap();
        let k = plain(dir.path());
        let host = NullHost::default();
        let yes = fixed_confirm(Confirmed { ok: true, checked: false });
        let no = fixed_confirm(Confirmed { ok: false, checked: false });
        assert!(k.handle("foo:bar", &[], &host, &yes).await.is_none());
        let r = k.handle("memory:add", &[json!({ "text": "Mag kurze Antworten.", "tags": "stil" })], &host, &yes).await.unwrap().unwrap();
        assert_eq!(r["ok"], true);
        let id = r["item"]["id"].as_str().unwrap().to_string();
        assert_eq!(r["item"]["scope"], "konto");
        let r = k.handle("memory:add", &[json!({ "text": "x" })], &host, &yes).await.unwrap().unwrap();
        assert_eq!(r["ok"], false);
        assert!(r["reason"].as_str().unwrap().contains("zu kurz"));
        let r = k.handle("memory:update", &[json!(id), json!({ "text": "Mag lange Antworten." })], &host, &yes).await.unwrap().unwrap();
        assert_eq!(r["item"]["text"], "Mag lange Antworten.");
        let r = k.handle("memory:update", &[json!("m0"), json!({ "text": "Mag lange Antworten." })], &host, &yes).await.unwrap().unwrap();
        assert_eq!(r["reason"], "Den Eintrag gibt es nicht mehr.");
        let l = k.handle("memory:list", &[], &host, &yes).await.unwrap().unwrap();
        assert_eq!(l["enabled"], true);
        assert_eq!(l["items"].as_array().unwrap().len(), 1);
        // Alles vergessen nur nach Rückfrage
        assert_eq!(k.handle("memory:clear", &[], &host, &no).await.unwrap().unwrap(), json!(false));
        assert_eq!(k.memory.list().len(), 1);
        assert_eq!(k.handle("memory:clear", &[], &host, &yes).await.unwrap().unwrap(), json!(true));
        assert!(k.memory.list().is_empty());
        // Schalter
        k.handle("memory:enable", &[json!(false)], &host, &yes).await.unwrap().unwrap();
        assert!(!k.memory_on());
        // Skills
        let skill = json!({ "name": "Mein Bericht", "description": "Bericht", "body": "Schreib." });
        let r = k.handle("skills:save", &[skill, json!("")], &host, &yes).await.unwrap().unwrap();
        assert_eq!(r["skill"]["slug"], "mein-bericht");
        let r = k.handle("skills:list", &[], &host, &yes).await.unwrap().unwrap();
        assert_eq!(r.as_array().unwrap().len(), 1);
        assert_eq!(k.handle("skills:remove", &[json!("mein-bericht")], &host, &no).await.unwrap().unwrap(), json!(false));
        assert_eq!(k.handle("skills:remove", &[json!("mein-bericht")], &host, &yes).await.unwrap().unwrap(), json!(true));
        assert_eq!(k.handle("skills:remove", &[json!("mein-bericht")], &host, &yes).await.unwrap().unwrap(), json!(false));
        let r = k.handle("skills:export", &[json!("gibt-es-nicht")], &host, &yes).await.unwrap().unwrap();
        assert_eq!(r["reason"], "Den Skill gibt es nicht mehr.");
    }
}
