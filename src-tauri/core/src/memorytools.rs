//! Werkzeuge für Gedächtnis (memory.rs, ENTSCHEIDE E50) und eigene Skills (skills.rs, E51). Sie laufen
//! im Hauptprozess, nicht im Terminal: Gedächtnis und Skills liegen in den Daten der App, die das
//! Terminal nicht beschreiben kann.
//!
//! Öffentliche Schnittstelle (Port von src/main/memorytools.js):
//! - `MEMORY_TOOLS`: Namen der Werkzeuge, die vom Schalter «Gedächtnis» abhängen
//! - `knowledge_tools() -> Vec<KnowledgeTool>`: Name, Titel, Beschreibung und Parameter (`KnowledgeTool::spec`
//!   liefert den Eintrag im OpenAI-Format, byte-gleich mit der JavaScript-Fassung)
//! - `describe(name, args) -> String`: Kurztext zum Aufruf (`describe` der JavaScript-Werkzeuge)
//! - `run(name, args, ctx) -> Result<ToolOutput>`: führt ein Werkzeug aus. Ein `Err` entspricht einem
//!   geworfenen Fehler in JavaScript; `tools.rs` macht daraus `{ ok: false, output: "Fehler: …" }`.
//!
//! `KnowledgeCtx` entspricht `ctx.knowledge` (von main.js über agent.js), `MemoryToolCtx` den übrigen
//! Feldern des Werkzeug-Kontexts, die diese Werkzeuge brauchen.

use crate::host::{ConfirmFn, ConfirmOptions};
use crate::memory::{is_sensitive, Memory, NewItem};
use crate::skills::{check_name, slugify, SaveOptions, SkillInput, Skills};
use crate::util::{char_len, js_string, str_of, take_chars, truthy};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::sync::Arc;

pub const MEMORY_TOOLS: [&str; 2] = ["merken", "vergessen"];

/// So lang darf eine Anleitung sein, die das Modell anlegt (ganz in der Rückfrage)
pub const SKILL_SHOWN: usize = 3000;

/// `ctx.knowledge = { memory, skills, memoryOn(), changed() }`
#[derive(Clone, Default)]
pub struct KnowledgeCtx {
    pub memory: Option<Arc<Memory>>,
    pub skills: Option<Arc<Skills>>,
    /// Ist das Gedächtnis in den Einstellungen eingeschaltet? Fehlt die Funktion, gilt es als eingeschaltet.
    pub memory_on: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Wird nach jeder Änderung aufgerufen (die Oberfläche lädt dann neu)
    pub changed: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl KnowledgeCtx {
    fn changed(&self) {
        if let Some(f) = &self.changed {
            f();
        }
    }
}

/// Teile des Werkzeug-Kontexts, die Gedächtnis und Skills brauchen
#[derive(Clone, Default)]
pub struct MemoryToolCtx {
    pub folder: String,
    pub chat: String,
    pub chat_title: String,
    pub knowledge: Option<KnowledgeCtx>,
    /// Rückfrage an die Person (`ctx.confirm`); ohne sie gilt jede Rückfrage als abgelehnt
    pub confirm: Option<ConfirmFn>,
}

/// Ergebnis eines Werkzeugs: `{ ok, output, summary, display? }`
#[derive(Clone, Debug, PartialEq)]
pub struct ToolOutput {
    pub ok: bool,
    pub output: String,
    pub summary: String,
    pub display: Option<String>,
}

impl ToolOutput {
    fn new(ok: bool, output: String, summary: impl Into<String>) -> Self {
        ToolOutput { ok, output, summary: summary.into(), display: None }
    }
}

/// Beschreibung eines Werkzeugs
#[derive(Clone, Debug)]
pub struct KnowledgeTool {
    pub name: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

impl KnowledgeTool {
    /// Eintrag im OpenAI-Format (`toolSpecs` in tools.js)
    pub fn spec(&self) -> Value {
        json!({ "type": "function", "function": { "name": self.name, "description": self.description, "parameters": self.parameters } })
    }
}

enum Part {
    Memory,
    Skills,
}

type Parts = (Option<Arc<Memory>>, Option<Arc<Skills>>);

fn need(ctx: &MemoryToolCtx, part: Part) -> Result<Parts> {
    let missing = || match part {
        Part::Memory => anyhow::anyhow!("Das Gedächtnis ist hier nicht verfügbar."),
        Part::Skills => anyhow::anyhow!("Skills sind hier nicht verfügbar."),
    };
    let Some(k) = &ctx.knowledge else { return Err(missing()) };
    match part {
        Part::Memory => {
            let Some(m) = &k.memory else { return Err(missing()) };
            if k.memory_on.as_ref().is_some_and(|f| !f()) {
                bail!("Das Gedächtnis ist in den Einstellungen ausgeschaltet.");
            }
            Ok((Some(m.clone()), None))
        }
        Part::Skills => {
            let Some(s) = &k.skills else { return Err(missing()) };
            Ok((None, Some(s.clone())))
        }
    }
}

fn need_memory(ctx: &MemoryToolCtx) -> Result<Arc<Memory>> {
    need(ctx, Part::Memory).map(|(m, _)| m.expect("Gedächtnis"))
}

fn need_skills(ctx: &MemoryToolCtx) -> Result<Arc<Skills>> {
    need(ctx, Part::Skills).map(|(_, s)| s.expect("Skills"))
}

fn clip_line(t: &str, n: usize) -> String {
    let s = crate::memory::collapse_ws(t);
    if char_len(&s) > n { format!("{}…", take_chars(&s, n - 1)) } else { s }
}

fn field(a: &Value, key: &str) -> String {
    str_of(a, key)
}

pub fn knowledge_tools() -> Vec<KnowledgeTool> {
    vec![
        KnowledgeTool {
            name: "merken",
            label: "Merken",
            description: concat!(
                "Merkt sich eine dauerhafte Angabe für spätere Chats, nur auf diesem Laptop. Benutze es, wenn die Person «merk dir …» sagt ",
                "oder eine Vorliebe nennt, die auch später gilt. bereich «konto»: über die Person, gilt in allen Projekten; «projekt»: nur für dieses Projekt. ",
                "Ein kurzer Satz. Nie Passwörter, Tokens, Gesundheit oder Angaben über andere Personen.",
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "text": { "type": "string", "description": "Ein kurzer Satz, z. B. «Mag Antworten in Stichworten.»" },
                    "bereich": { "type": "string", "enum": ["konto", "projekt"], "description": "konto (über die Person) oder projekt (nur dieses Projekt)" },
                    "tags": { "type": "array", "items": { "type": "string" }, "description": "1 bis 3 kurze Schlagworte" },
                },
                "required": ["text", "bereich"],
            }),
        },
        KnowledgeTool {
            name: "vergessen",
            label: "Vergessen",
            description: concat!(
                "Löscht einen Eintrag aus dem Gedächtnis, mit der id aus dem Abschnitt «Gedächtnis» oder mit seinem Text. ",
                "Benutze es, wenn die Person sagt, etwas stimme nicht mehr oder ASK solle es vergessen.",
            ),
            parameters: json!({
                "type": "object",
                "properties": { "id": { "type": "string", "description": "id des Eintrags, z. B. m1a2b3c4" }, "text": { "type": "string", "description": "oder der Text des Eintrags" } },
            }),
        },
        KnowledgeTool {
            name: "skill_lesen",
            label: "Skill",
            description: "Liest die Anleitung eines eigenen Skills der Person (Liste unter «Eigene Skills»). Lies den Skill, bevor du einen passenden Auftrag ausführst.",
            parameters: json!({ "type": "object", "properties": { "name": { "type": "string", "description": "Name des Skills aus der Liste" } }, "required": ["name"] }),
        },
        KnowledgeTool {
            name: "skill_erstellen",
            label: "Skill erstellen",
            description: concat!(
                "Legt einen eigenen Skill an (oder ersetzt einen gleichnamigen): eine Anleitung, die ASK später bei passenden Aufträgen liest und die die Person mit /name aufrufen kann. ",
                "Nur, wenn die Person es wünscht. Die Person bestätigt das Speichern.",
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "kurzer Name, klein mit Bindestrichen, z. B. «lernjournal»" },
                    "beschreibung": { "type": "string", "description": "Ein Satz: wofür und wann der Skill gilt" },
                    "anleitung": { "type": "string", "description": "Die Anleitung in Markdown: Schritte, Vorgaben, Beispiele" },
                },
                "required": ["name", "beschreibung", "anleitung"],
            }),
        },
    ]
}

/// Kurztext zum Aufruf, wie `describe` der Werkzeuge in JavaScript
pub fn describe(name: &str, a: &Value) -> String {
    match name {
        "merken" => field(a, "text"),
        "vergessen" => {
            if a.get("id").is_some_and(truthy) { field(a, "id") } else { field(a, "text") }
        }
        "skill_lesen" | "skill_erstellen" => field(a, "name"),
        _ => String::new(),
    }
}

/// Führt das Werkzeug `name` aus. Unbekannte Namen: `None`.
pub async fn run(name: &str, a: &Value, ctx: &MemoryToolCtx) -> Option<Result<ToolOutput>> {
    Some(match name {
        "merken" => merken(a, ctx),
        "vergessen" => vergessen(a, ctx),
        "skill_lesen" => skill_lesen(a, ctx),
        "skill_erstellen" => skill_erstellen(a, ctx).await,
        _ => return None,
    })
}

fn merken(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let memory = need_memory(ctx)?;
    let text = field(a, "text");
    if is_sensitive(&text) {
        return Ok(ToolOutput::new(false, "Nicht gemerkt: Das sieht nach heiklen Angaben aus (Zugangsdaten, Gesundheit, Noten). So etwas gehört nicht ins Gedächtnis.".into(), "nicht gemerkt"));
    }
    let scope = if field(a, "bereich") == "projekt" { "projekt" } else { "konto" };
    if scope == "projekt" && ctx.folder.is_empty() {
        bail!("Ohne Projekt geht nur bereich «konto».");
    }
    let item = memory.add(NewItem {
        text,
        tags: a.get("tags").cloned().unwrap_or(Value::Null),
        scope: scope.to_string(),
        project: if scope == "projekt" { Some(ctx.folder.clone()) } else { None },
        chat: Some(ctx.chat.clone()),
        chat_title: ctx.chat_title.clone(),
        chat_folder: ctx.folder.clone(),
        source: "merken".into(),
    })?;
    if let Some(k) = &ctx.knowledge {
        k.changed();
    }
    let where_ = if scope == "projekt" { "dieses Projekt" } else { "alle Projekte" };
    Ok(ToolOutput::new(true, format!("Gemerkt (id {}, {where_}): {}", item.id, item.text), clip_line(&item.text, 80)))
}

fn vergessen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let memory = need_memory(ctx)?;
    let visible: std::collections::HashSet<String> = memory.visible(&ctx.folder).into_iter().map(|i| i.id).collect();
    let items = if a.get("id").is_some_and(truthy) {
        let id = field(a, "id");
        if visible.contains(&id) { memory.get(&id).into_iter().collect() } else { Vec::new() }
    } else if a.get("text").is_some_and(truthy) {
        memory.find_by_text(&field(a, "text"), &ctx.folder)
    } else {
        bail!("id oder text angeben.");
    };
    if items.is_empty() {
        return Ok(ToolOutput::new(false, "Kein passender Eintrag im Gedächtnis.".into(), "nicht gefunden"));
    }
    if items.len() > 3 {
        let list: Vec<String> = items.iter().take(10).map(|i| format!("- [{}] {}", i.id, i.text)).collect();
        return Ok(ToolOutput::new(false, format!("{} Einträge passen. Gib die id an:\n{}", items.len(), list.join("\n")), "nicht eindeutig"));
    }
    for i in &items {
        memory.remove(&i.id)?;
    }
    if let Some(k) = &ctx.knowledge {
        k.changed();
    }
    let texts: Vec<&str> = items.iter().map(|i| i.text.as_str()).collect();
    Ok(ToolOutput::new(true, format!("Vergessen: {}", texts.join(" | ")), clip_line(&texts.join(", "), 80)))
}

fn skill_lesen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let skills = need_skills(ctx)?;
    let name = field(a, "name");
    let Some(s) = skills.get(&name) else {
        let names: Vec<String> = skills.list().into_iter().map(|x| x.slug).collect();
        let tail = if names.is_empty() { " Es gibt noch keine eigenen Skills.".to_string() } else { format!(" Vorhanden: {}", names.join(", ")) };
        return Ok(ToolOutput::new(false, format!("Keinen Skill «{name}» gefunden.{tail}"), "nicht gefunden"));
    };
    let mut out = ToolOutput::new(true, format!("Skill «{}»: {}\n\n{}", s.slug, s.description, s.body), format!("{} gelesen", s.slug));
    out.display = Some(format!("{}\n\n{}", s.description, take_chars(&s.body, 3000)));
    Ok(out)
}

async fn skill_erstellen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let skills = need_skills(ctx)?;
    let slug = slugify(&field(a, "name"));
    let problem = check_name(&slug);
    if !problem.is_empty() {
        bail!("{problem}");
    }
    let exists = skills.get(&slug).is_some();
    // Die Person sieht vor dem Speichern die ganze Anleitung: Was danach in jedem passenden Auftrag gilt,
    // darf nicht hinter einem «…» stehen. Längere Skills legt die Person selbst in den Einstellungen an.
    let anleitung = field(a, "anleitung");
    if char_len(&anleitung) > SKILL_SHOWN {
        return Ok(ToolOutput::new(
            false,
            format!(
                "Die Anleitung hat {} Zeichen; höchstens {SKILL_SHOWN} gehen, damit die Person sie vor dem Speichern ganz lesen kann. \
Fasse sie kürzer, oder die Person legt den Skill in den Einstellungen unter «Skills» selbst an.",
                char_len(&anleitung)
            ),
            "zu lang",
        ));
    }
    let ok = match &ctx.confirm {
        Some(confirm) => {
            let message = if exists { format!("Skill «{slug}» ersetzen?") } else { format!("Skill «{slug}» speichern?") };
            let detail = format!(
                "{}\n\n{anleitung}\n\nAufrufen mit /{slug}. Ändern und löschen in den Einstellungen unter «Skills».",
                clip_line(&field(a, "beschreibung"), 200)
            );
            confirm(message, detail, ConfirmOptions::default()).await.ok
        }
        None => false,
    };
    if !ok {
        return Ok(ToolOutput::new(false, "Die Person hat das Speichern abgelehnt.".into(), "abgelehnt"));
    }
    let s = skills.save(
        &SkillInput { name: slug, description: field(a, "beschreibung"), body: js_string(a.get("anleitung").unwrap_or(&Value::Null)) },
        &SaveOptions { previous: String::new(), overwrite: true },
    )?;
    if let Some(k) = &ctx.knowledge {
        k.changed();
    }
    Ok(ToolOutput::new(true, format!("Skill «{}» gespeichert. Die Person ruft ihn mit /{} auf.", s.slug, s.slug), format!("/{}", s.slug)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::Confirmed;
    use parking_lot::Mutex;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn proj() -> String {
        std::env::temp_dir().join("ASK-Projekte").join("Budget").to_string_lossy().to_string()
    }

    /// Wie `runTool` in tools.js: Fehler werden zu `{ ok: false, output: "Fehler: …" }`
    async fn run_tool(name: &str, args: Value, ctx: &MemoryToolCtx) -> ToolOutput {
        match run(name, &args, ctx).await.expect("bekanntes Werkzeug") {
            Ok(o) => o,
            Err(e) => ToolOutput::new(false, format!("Fehler: {e}"), e.to_string()),
        }
    }

    #[tokio::test]
    async fn werkzeuge_merken_und_vergessen_ausgeschaltet_nicht_angeboten() {
        let dir = tempfile::tempdir().unwrap();
        let memory = Arc::new(Memory::new(dir.path().join("gedaechtnis.json")));
        let on = Arc::new(AtomicBool::new(true));
        let changed = Arc::new(AtomicUsize::new(0));
        let (on2, ch2) = (on.clone(), changed.clone());
        let ctx = MemoryToolCtx {
            folder: proj(),
            chat: "c1".into(),
            chat_title: "Test".into(),
            knowledge: Some(KnowledgeCtx {
                memory: Some(memory.clone()),
                skills: None,
                memory_on: Some(Arc::new(move || on2.load(Ordering::SeqCst))),
                changed: Some(Arc::new(move || { ch2.fetch_add(1, Ordering::SeqCst); })),
            }),
            confirm: None,
        };
        let mut r = run_tool("merken", json!({ "text": "Mag Tabellen statt Fliesstext.", "bereich": "konto", "tags": ["stil"] }), &ctx).await;
        assert!(r.ok, "{}", r.output);
        assert!(regex::Regex::new(r"Gemerkt \(id m[0-9a-f]+, alle Projekte\)").unwrap().is_match(&r.output));
        run_tool("merken", json!({ "text": "Excel-Dateien liegen in daten/.", "bereich": "projekt" }), &ctx).await;
        assert_eq!(memory.for_project(&proj()).len(), 1);
        r = run_tool("merken", json!({ "text": "Mein Passwort ist geheim123", "bereich": "konto" }), &ctx).await;
        assert!(!r.ok);
        r = run_tool("vergessen", json!({ "text": "tabellen statt" }), &ctx).await;
        assert!(r.ok, "{}", r.output);
        r = run_tool("vergessen", json!({ "id": "m00000000" }), &ctx).await;
        assert!(!r.ok);
        assert_eq!(changed.load(Ordering::SeqCst), 3);
        on.store(false, Ordering::SeqCst);
        r = run_tool("merken", json!({ "text": "Noch etwas Neues.", "bereich": "konto" }), &ctx).await;
        assert!(!r.ok);
        assert!(r.output.contains("ausgeschaltet"));
        assert!(knowledge_tools().iter().any(|t| t.spec()["function"]["name"] == "merken"));
    }

    #[tokio::test]
    async fn werkzeuge_skill_lesen_und_skill_erstellen_mit_rueckfrage() {
        let dir = tempfile::tempdir().unwrap();
        let skills = Arc::new(Skills::new(dir.path()));
        let asked: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let answer = Arc::new(AtomicBool::new(false));
        let (asked2, answer2) = (asked.clone(), answer.clone());
        let confirm: ConfirmFn = Arc::new(move |m, d, _o| {
            asked2.lock().push((m, d));
            let ok = answer2.load(Ordering::SeqCst);
            Box::pin(async move { Confirmed { ok, checked: false } })
        });
        let ctx = MemoryToolCtx {
            knowledge: Some(KnowledgeCtx { skills: Some(skills.clone()), ..Default::default() }),
            confirm: Some(confirm),
            ..Default::default()
        };
        let mut r = run_tool("skill_lesen", json!({ "name": "bericht" }), &ctx).await;
        assert!(!r.ok);
        assert!(r.output.contains("noch keine"));
        let args = json!({ "name": "Mein Bericht", "beschreibung": "Bericht nach Vorlage", "anleitung": "1. Lesen\n2. Schreiben" });
        r = run_tool("skill_erstellen", args.clone(), &ctx).await;
        assert!(!r.ok, "abgelehnt");
        assert!(asked.lock()[0].0.contains("Skill «mein-bericht» speichern?"));
        assert!(skills.list().is_empty());
        answer.store(true, Ordering::SeqCst);
        r = run_tool("skill_erstellen", args.clone(), &ctx).await;
        assert!(r.ok, "{}", r.output);
        assert!(r.output.contains("/mein-bericht"));
        run_tool("skill_erstellen", args.clone(), &ctx).await;
        assert!(asked.lock()[2].0.contains("ersetzen?"));
        r = run_tool("skill_lesen", json!({ "name": "Mein Bericht" }), &ctx).await;
        assert!(r.ok);
        assert!(r.output.contains("Bericht nach Vorlage\n\n1. Lesen\n2. Schreiben"));
        r = run_tool("skill_erstellen", json!({ "name": "github", "beschreibung": "x", "anleitung": "y" }), &ctx).await;
        assert!(!r.ok);
        assert!(r.output.contains("Befehl der App"));
        // Die Rückfrage zeigt die ganze Anleitung; was zu lang dafür ist, legt das Modell nicht an
        let long = format!("Anfang\n{}\nENDE", "x".repeat(2000));
        r = run_tool("skill_erstellen", json!({ "name": "lang", "beschreibung": "x", "anleitung": long }), &ctx).await;
        assert!(r.ok);
        assert!(asked.lock().last().unwrap().1.contains("ENDE"), "ganze Anleitung in der Rückfrage");
        let count = asked.lock().len();
        r = run_tool("skill_erstellen", json!({ "name": "zulang", "beschreibung": "x", "anleitung": "y".repeat(3001) }), &ctx).await;
        assert!(!r.ok);
        assert_eq!(asked.lock().len(), count, "gar nicht erst gefragt");
    }

    #[test]
    fn werkzeugbeschreibungen_im_openai_format() {
        let specs: Vec<Value> = knowledge_tools().iter().map(KnowledgeTool::spec).collect();
        let names: Vec<&str> = specs.iter().map(|s| s["function"]["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["merken", "vergessen", "skill_lesen", "skill_erstellen"]);
        assert_eq!(specs[0]["type"], "function");
        assert_eq!(specs[0]["function"]["parameters"]["required"], json!(["text", "bereich"]));
        let keys: Vec<&String> = specs[0]["function"]["parameters"]["properties"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["text", "bereich", "tags"], "Reihenfolge wie im Original");
    }
}
