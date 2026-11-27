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
    /// Mods (mods.rs): Werkzeug mod_erstellen
    pub mods: Option<Arc<crate::mods::Mods>>,
    /// Kataloge (catalog.rs): Werkzeuge katalog_suchen und katalog_hinzufuegen
    pub catalog: Option<Arc<crate::catalog::Catalog>>,
    /// Repositories der Kataloge (mitgelieferte und eigene)
    pub catalog_sources: Option<Arc<dyn Fn() -> Vec<String> + Send + Sync>>,
    /// Ist das Gedächtnis in den Einstellungen eingeschaltet? Fehlt die Funktion, gilt es als eingeschaltet.
    pub memory_on: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Wird nach jeder Änderung aufgerufen (die Oberfläche lädt dann neu)
    pub changed: Option<Arc<dyn Fn() + Send + Sync>>,
    /// Nach Ein- oder Ausschalten eines Mods (die Oberfläche lädt die Mods neu, Zustand modsCode)
    pub mods_changed: Option<Arc<dyn Fn() + Send + Sync>>,
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
            label: "Skill benutzt",
            description: "Liest die Anleitung eines Skills (Liste unter «Skills»). Lies den Skill, bevor du einen passenden Auftrag ausführst.",
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
        KnowledgeTool {
            name: "mod_liste",
            label: "Mods",
            description: "Listet die Mods der Person (Name, Kennung, an oder aus, was sie ändern) und die mitgelieferten Beispiele. Benutze es, bevor du einen Mod änderst oder wenn die Person nach ihren Mods fragt.",
            parameters: json!({ "type": "object", "properties": {} }),
        },
        KnowledgeTool {
            name: "mod_lesen",
            label: "Mod lesen",
            description: "Liest einen Mod der Person: mod.json und den Inhalt seiner Textdateien (CSS, JavaScript …), oder nur eine Datei. Benutze es, um einen Mod zu erklären, Fehler darin zu finden oder ihn zu ändern.",
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Kennung des Mods aus mod_liste, z. B. «robo-helfer»" },
                    "datei": { "type": "string", "description": "Optional: nur diese Datei, z. B. «mod.js»" },
                },
                "required": ["id"],
            }),
        },
        KnowledgeTool {
            name: "katalog_suchen",
            label: "Katalog",
            description: concat!(
                "Sucht Skills und Mods in den Katalogen auf GitHub (die der Schule, anthropics/skills und eigene der Person) oder in einem genannten öffentlichen Repository. ",
                "Benutze es, wenn die Person einen fertigen Skill oder Mod sucht oder etwas will, das ein Katalog schon hat. Danach mit katalog_hinzufuegen anbieten.",
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "suche": { "type": "string", "description": "Suchwörter (alle müssen vorkommen), leer für alles" },
                    "art": { "type": "string", "enum": ["skill", "mod", "alle"], "description": "Standard: alle" },
                    "repo": { "type": "string", "description": "Optional: nur dieses Repository oder dieser Link, z. B. «anthropics/skills»" },
                },
            }),
        },
        KnowledgeTool {
            name: "katalog_hinzufuegen",
            label: "Aus Katalog hinzufügen",
            description: concat!(
                "Fügt einen Skill oder Mod aus einem Katalog hinzu (Angaben aus katalog_suchen). Nur, wenn die Person es will. ",
                "Die Person sieht vorher Beschreibung und Inhalt und bestätigt. Ein Mod ist danach aus, bis sie ihn unter Einstellungen → Mods einschaltet; teste ihn vorher mit mod_testen.",
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "repo": { "type": "string", "description": "Repository aus katalog_suchen, z. B. «KIYeager/ASK-Erweiterungen»" },
                    "art": { "type": "string", "enum": ["skill", "mod"] },
                    "pfad": { "type": "string", "description": "Ordner im Repository aus katalog_suchen, z. B. «mods/neon»" },
                },
                "required": ["repo", "art", "pfad"],
            }),
        },
        KnowledgeTool {
            name: "mod_testen",
            label: "Mod testen",
            description: concat!(
                "Probiert einen Mod aus, auch wenn er aus ist: lädt ihn in einer unsichtbaren Testansicht von ASK mit Beispieldaten, ",
                "führt Aktionen aus und liefert ein Bildschirmfoto der Oberfläche und einen Bericht (geladen, Fehler in der Konsole, angelegte Elemente, Bereiche, Knöpfe, Befehle). ",
                "Benutze es nach mod_erstellen, bevor du der Person sagst, dass der Mod fertig ist, und um Fehler zu finden.",
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Kennung des Mods" },
                    "farbschema": { "type": "string", "enum": ["dunkel", "hell"], "description": "Standard: dunkel" },
                    "aktionen": {
                        "type": "array",
                        "description": "Optional, der Reihe nach vor dem Bildschirmfoto: {art:'ereignis', name:'fertig'|'gesendet'|'antwort'|'fehler'|'frage'|'chat'|'theme', daten:{…}} · {art:'klick', selektor:'.robo-helfer-figur'} · {art:'panel', titel:'Timer'} (Bereich des Mods öffnen) · {art:'eingabe', text:'/vokabeln Tiere'} · {art:'senden', text:'Hallo'} (Beispielauftrag mit Antwort) · {art:'warten', ms:1000}",
                        "items": { "type": "object" },
                    },
                },
                "required": ["id"],
            }),
        },
        KnowledgeTool {
            name: "mod_einschalten",
            label: "Mod einschalten",
            description: concat!(
                "Schaltet einen Mod ein oder aus. Einschalten nur, wenn die Person es will, und erst nach mod_testen ohne Fehler; ",
                "die Person bestätigt es in einem Dialog (bei Code mit Warnung). Ausschalten geht ohne Rückfrage. Danach lädt die Oberfläche neu, sobald der Auftrag fertig ist.",
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Kennung des Mods" },
                    "an": { "type": "boolean", "description": "true: einschalten (Standard), false: ausschalten" },
                },
                "required": ["id"],
            }),
        },
        KnowledgeTool {
            name: "mod_erstellen",
            label: "Mod erstellen",
            description: concat!(
                "Legt einen Mod für die Oberfläche von ASK an oder ändert einen vorhandenen (Aussehen, /Befehle, Bereiche, Figuren, Regeln). ",
                "Lies vorher den Skill «mod-erstellen» (Format und JavaScript-Schnittstelle), zum Ändern zuerst mod_lesen. Nur, wenn die Person es wünscht. ",
                "Beim Ändern genügen die geänderten Dateien (mit «id»); ein leerer Inhalt löscht eine Datei. ",
                "Die Person bestätigt das Speichern; der Mod ist danach aus, bis sie ihn unter Einstellungen → Mods nach einer Vorschau einschaltet.",
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Kennung eines vorhandenen Mods, der geändert wird (weglassen bei einem neuen Mod)" },
                    "dateien": { "type": "object", "description": "Dateiname → Inhalt (Text). Neuer Mod: mindestens «mod.json», dazu z. B. «style.css», «mod.js», «bild.svg». Ändern: nur die geänderten Dateien, \"\" löscht eine.", "additionalProperties": { "type": "string" } },
                },
                "required": ["dateien"],
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
        "mod_erstellen" => a.get("dateien").and_then(|d| d.get("mod.json")).map(js_string)
            .and_then(|t| crate::mods::parse_manifest(&t).ok()).map(|m| m.name).unwrap_or_else(|| field(a, "id")),
        "mod_testen" => field(a, "id"),
        "mod_einschalten" => format!("{} {}", field(a, "id"), if a.get("an") == Some(&Value::Bool(false)) { "aus" } else { "an" }),
        "katalog_suchen" => [field(a, "suche"), field(a, "repo")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "),
        "katalog_hinzufuegen" => format!("{} {}", field(a, "repo"), field(a, "pfad")),
        "mod_lesen" => [field(a, "id"), field(a, "datei")].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · "),
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
        "mod_erstellen" => mod_erstellen(a, ctx).await,
        "mod_liste" => mod_liste(ctx),
        "mod_einschalten" => mod_einschalten(a, ctx).await,
        "katalog_suchen" => katalog_suchen(a, ctx).await,
        "katalog_hinzufuegen" => katalog_hinzufuegen(a, ctx).await,
        "mod_lesen" => mod_lesen(a, ctx),
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

fn need_catalog(ctx: &MemoryToolCtx) -> Result<Arc<crate::catalog::Catalog>> {
    ctx.knowledge.as_ref().and_then(|k| k.catalog.clone()).ok_or_else(|| anyhow::anyhow!("Kataloge sind hier nicht verfügbar."))
}

/// Höchstens so viele Treffer zeigen
const CATALOG_HITS: usize = 30;

async fn katalog_suchen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let catalog = need_catalog(ctx)?;
    let k = ctx.knowledge.as_ref().expect("Wissen");
    let repos: Vec<String> = match field(a, "repo") {
        r if !r.is_empty() => vec![r],
        _ => k.catalog_sources.as_ref().map(|f| f()).unwrap_or_default(),
    };
    if repos.is_empty() {
        return Ok(ToolOutput::new(false, "Keine Kataloge eingerichtet.".into(), "keine Kataloge"));
    }
    let art = field(a, "art");
    let query = field(a, "suche");
    let own_skills: Vec<String> = k.skills.as_ref().map(|s| s.list_own().into_iter().map(|x| x.slug).collect()).unwrap_or_default();
    let own_mods: Vec<String> = k.mods.as_ref().map(|m| m.list().into_iter().map(|x| x.manifest.id).collect()).unwrap_or_default();
    let (mut lines, mut problems, mut total) = (Vec::new(), Vec::new(), 0usize);
    for repo in &repos {
        let src = match crate::catalog::parse_source(repo) {
            Ok(s) => s,
            Err(e) => { problems.push(format!("{repo}: {e}")); continue; }
        };
        let entries = match catalog.list(&src).await {
            Ok(e) => e,
            Err(e) => { problems.push(format!("{}: {e}", src.repo_name())); continue; }
        };
        for e in entries.iter().filter(|e| (art.is_empty() || art == "alle" || e.kind == art) && crate::catalog::matches(e, &query)) {
            total += 1;
            if lines.len() >= CATALOG_HITS {
                continue;
            }
            let folder = e.path.rsplit('/').next().unwrap_or("").to_string();
            let have = if e.kind == "skill" { own_skills.contains(&e.name) } else { own_mods.contains(&folder) };
            let name = if e.kind == "skill" { format!("/{}", e.name) } else { e.name.clone() };
            lines.push(format!("- [{}] {name}: {} (repo {}, pfad {}){}", if e.kind == "mod" { "Mod" } else { "Skill" },
                clip_line(&e.description, 200), src.repo_name(), e.path, if have { " – schon hinzugefügt" } else { "" }));
        }
    }
    let mut out = if lines.is_empty() {
        format!("Nichts gefunden{} in: {}.", if query.is_empty() { String::new() } else { format!(" für «{query}»") }, repos.join(", "))
    } else {
        format!("{total} Treffer{}:\n{}", if total > lines.len() { format!(", die ersten {}", lines.len()) } else { String::new() }, lines.join("\n"))
    };
    if !problems.is_empty() {
        out.push_str(&format!("\nNicht erreichbar: {}", problems.join(" · ")));
    }
    out.push_str("\nHinweis: Skills aus anthropics/skills sind auf Englisch; ASK antwortet trotzdem auf Deutsch. Dort beschriebene Skripte neben der SKILL.md kommen nicht mit.");
    Ok(ToolOutput::new(!lines.is_empty(), out, format!("{total} Treffer")))
}

async fn katalog_hinzufuegen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let catalog = need_catalog(ctx)?;
    let k = ctx.knowledge.as_ref().expect("Wissen");
    let (Some(skills), Some(mods)) = (k.skills.clone(), k.mods.clone()) else { bail!("Skills und Mods sind hier nicht verfügbar.") };
    let art = field(a, "art");
    if art != "skill" && art != "mod" {
        bail!("art muss «skill» oder «mod» sein.");
    }
    let src = crate::catalog::parse_source(&field(a, "repo"))?;
    let prepared = crate::catalog::prepare(&catalog, &src, &art, field(a, "pfad").trim_matches('/')).await?;
    let Some(confirm) = &ctx.confirm else {
        return Ok(ToolOutput::new(false, "Die Person hat das Hinzufügen abgelehnt.".into(), "abgelehnt"));
    };
    let r = crate::catalog::install(prepared, &skills, &mods, confirm, true, &src.repo_name()).await?;
    if r["ok"] != true {
        return Ok(ToolOutput::new(false, "Die Person hat das Hinzufügen abgelehnt.".into(), "abgelehnt"));
    }
    k.changed();
    Ok(if art == "skill" {
        let slug = js_string(&r["skill"]["slug"]);
        ToolOutput::new(true, format!("Skill «/{slug}» hinzugefügt. Die Person kann ihn mit /{slug} aufrufen; du liest ihn mit skill_lesen."), format!("/{slug} hinzugefügt"))
    } else {
        let (id, name) = (js_string(&r["mod"]["id"]), js_string(&r["mod"]["name"]));
        ToolOutput::new(true, format!("Mod «{name}» (id {id}) hinzugefügt, aber aus. Teste ihn mit mod_testen und sag der Person: Einstellungen → Mods → «{name}» → Vorschau ansehen und einschalten."), format!("{id} hinzugefügt"))
    })
}

async fn mod_einschalten(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let mods = need_mods(ctx)?;
    let id = crate::mods::slug(&field(a, "id"));
    let on = a.get("an") != Some(&Value::Bool(false));
    let Some(info) = mods.info(&id) else {
        return Ok(ToolOutput::new(false, format!("Keinen Mod «{id}» gefunden."), "nicht gefunden"));
    };
    let name = info.manifest.name.clone();
    let notify = |k: &KnowledgeCtx| if let Some(f) = &k.mods_changed { f() };
    if !on {
        if !mods.enable(&id, false)? {
            return Ok(ToolOutput::new(true, format!("Mod «{name}» war schon aus."), "schon aus"));
        }
        if let Some(k) = &ctx.knowledge { notify(k); }
        return Ok(ToolOutput::new(true, format!("Mod «{name}» ausgeschaltet. Die Oberfläche lädt neu, wenn der Auftrag fertig ist."), format!("{id} aus")));
    }
    if !info.fehler.is_empty() {
        return Ok(ToolOutput::new(false, format!("Nicht eingeschaltet: {}", info.fehler), "fehlerhaft"));
    }
    if info.aktiv {
        return Ok(ToolOutput::new(true, format!("Mod «{name}» ist schon an."), "schon an"));
    }
    let code = !info.manifest.script.is_empty();
    let ok = match &ctx.confirm {
        Some(confirm) => {
            let mut detail = format!("{}\n\nÄndert: {}.", info.manifest.beschreibung, mod_parts(&info.manifest));
            if code {
                detail.push_str(&format!("\n\nAchtung: Der Mod enthält Code ({}), der in ASK läuft und alles in der Oberfläche sehen und bedienen kann, auch deine Chats. Schalte ihn nur ein, wenn du ihm vertraust; den Code siehst du unter Einstellungen → Mods → Vorschau. Solange er an ist, bestätigst du heikle Schritte in einem Fenster von Windows.", info.manifest.script));
            }
            detail.push_str("\n\nAusschalten: Einstellungen → Mods oder Ctrl+Shift+M.");
            confirm(format!("Mod «{name}» einschalten?"), detail, ConfirmOptions { danger: code, ..Default::default() }).await.ok
        }
        None => false,
    };
    if !ok {
        return Ok(ToolOutput::new(false, "Die Person hat das Einschalten abgelehnt.".into(), "abgelehnt"));
    }
    mods.enable(&id, true)?;
    if let Some(k) = &ctx.knowledge { notify(k); }
    Ok(ToolOutput::new(true, format!("Mod «{name}» eingeschaltet. Die Oberfläche lädt neu, sobald dieser Auftrag fertig ist; erst dann wirkt er."), format!("{id} an")))
}

fn need_mods(ctx: &MemoryToolCtx) -> Result<Arc<crate::mods::Mods>> {
    ctx.knowledge.as_ref().and_then(|k| k.mods.clone()).ok_or_else(|| anyhow::anyhow!("Mods sind hier nicht verfügbar."))
}

/// Teile eines Mods in Worten («Aussehen, Code, 2 Befehle …»)
fn mod_parts(m: &crate::mods::Manifest) -> String {
    let mut parts = Vec::new();
    if !m.style.is_empty() { parts.push("Aussehen".to_string()); }
    if !m.script.is_empty() { parts.push("Code".to_string()); }
    if !m.befehle.is_empty() { parts.push(format!("Befehle {}", m.befehle.iter().map(|b| format!("/{}", b.name)).collect::<Vec<_>>().join(" "))); }
    if !m.anweisungen.is_empty() { parts.push("Anweisungen".to_string()); }
    if !m.nachfragen.is_empty() { parts.push(format!("fragt vor: {}", m.nachfragen.join(", "))); }
    parts.join(", ")
}

fn mod_liste(ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let mods = need_mods(ctx)?;
    let list = mods.list();
    let mut out = String::new();
    if list.is_empty() {
        out.push_str("Die Person hat noch keine Mods.");
    } else {
        out.push_str("Mods der Person:");
        for m in &list {
            let state = if !m.fehler.is_empty() { format!("fehlerhaft: {}", m.fehler) } else if m.aktiv { "an".into() } else if m.geaendert { "geändert, wieder einschalten nötig".into() } else { "aus".into() };
            out.push_str(&format!("\n- {} (id {}, {}, Version {}): {} [{}; {} Dateien]", m.manifest.name, m.manifest.id, state,
                if m.manifest.version.is_empty() { "–" } else { &m.manifest.version }, m.manifest.beschreibung, mod_parts(&m.manifest), m.dateien));
        }
    }
    let examples: Vec<String> = mods.examples().iter().filter(|e| e["installiert"] != true)
        .map(|e| format!("{} (id {})", js_string(&e["name"]), js_string(&e["id"]))).collect();
    if !examples.is_empty() {
        out.push_str(&format!("\nNicht hinzugefügte Beispiele (Einstellungen → Mods → Beispiele): {}", examples.join(", ")));
    }
    Ok(ToolOutput::new(true, out, format!("{} Mods", list.len())))
}

/// Höchstens so viel Text aus den Dateien eines Mods auf einmal
const MOD_READ_MAX: usize = 30_000;

fn mod_lesen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let mods = need_mods(ctx)?;
    let id = crate::mods::slug(&field(a, "id"));
    let Some(v) = mods.get(&id) else {
        let ids: Vec<String> = mods.list().into_iter().map(|m| m.manifest.id).collect();
        return Ok(ToolOutput::new(false, format!("Keinen Mod «{id}» gefunden. Vorhanden: {}", if ids.is_empty() { "keine".into() } else { ids.join(", ") }), "nicht gefunden"));
    };
    let only = field(a, "datei").replace('\\', "/");
    let files = v["files"].as_array().cloned().unwrap_or_default();
    let state = if v["aktiv"] == true { "an" } else if v["geaendert"] == true { "geändert seit dem Einschalten (aus)" } else { "aus" };
    let mut out = format!("Mod «{}» (id {}, {state}). Dateien: {}", js_string(&v["name"]), id,
        files.iter().map(|f| format!("{} ({} Bytes)", js_string(&f["path"]), f["size"])).collect::<Vec<_>>().join(", "));
    if truthy(&v["fehler"]) {
        out.push_str(&format!("\nFehler: {}", js_string(&v["fehler"])));
    }
    let mut shown = 0;
    for f in &files {
        let path = js_string(&f["path"]);
        if !only.is_empty() && path != only {
            continue;
        }
        let text = js_string(&f["text"]);
        if text.is_empty() {
            out.push_str(&format!("\n\n--- {path} ---\n(keine Textdatei)"));
            continue;
        }
        if char_len(&out) + char_len(&text) > MOD_READ_MAX {
            out.push_str(&format!("\n\n--- {path} ---\n(zu lang für dieses Mal; einzeln lesen mit datei: «{path}»)"));
            continue;
        }
        out.push_str(&format!("\n\n--- {path} ---\n{text}"));
        shown += 1;
    }
    if !only.is_empty() && shown == 0 && !files.iter().any(|f| js_string(&f["path"]) == only) {
        return Ok(ToolOutput::new(false, format!("Im Mod «{id}» gibt es keine Datei «{only}»."), "nicht gefunden"));
    }
    Ok(ToolOutput::new(true, out, if only.is_empty() { format!("{id} gelesen") } else { format!("{id}/{only}") }))
}

async fn mod_erstellen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let mods = need_mods(ctx)?;
    let given = crate::mods::files_from_json(a.get("dateien").unwrap_or(&Value::Null))?;
    // Kennung: aus der neuen mod.json, sonst aus «id» (Ändern einzelner Dateien)
    let new_manifest = given.iter().find(|(p, _)| p == "mod.json").and_then(|(_, d)| crate::mods::parse_manifest(&String::from_utf8_lossy(d)).ok());
    let id = new_manifest.as_ref().map(|m| m.id.clone()).unwrap_or_else(|| crate::mods::slug(&field(a, "id")));
    let exists = !id.is_empty() && mods.info(&id).is_some();
    let mut merged: std::collections::BTreeMap<String, Vec<u8>> = if exists { mods.files(&id)?.into_iter().collect() } else { Default::default() };
    let (mut added, mut changed, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    for (p, d) in given {
        let path = match crate::mods::check_path(&p) {
            Ok(path) => path,
            Err(e) => return Ok(ToolOutput::new(false, format!("Nicht gespeichert: {e}"), "ungültig")),
        };
        if d.is_empty() {
            if merged.remove(&path).is_some() { removed.push(path); }
        } else if let Some(old) = merged.insert(path.clone(), d.clone()) {
            if old != d { changed.push(path); }
        } else {
            added.push(path);
        }
    }
    let files: Vec<(String, Vec<u8>)> = merged.into_iter().collect();
    let m = match crate::mods::Mods::check_files(&files) {
        Ok(m) => m,
        Err(e) => return Ok(ToolOutput::new(false, format!("Nicht gespeichert: {e}"), "ungültig")),
    };
    if added.is_empty() && changed.is_empty() && removed.is_empty() {
        return Ok(ToolOutput::new(false, "Nichts geändert: Die Dateien sind schon so.".into(), "unverändert"));
    }
    let what = [("Neu", &added), ("Geändert", &changed), ("Gelöscht", &removed)].iter().filter(|(_, l)| !l.is_empty())
        .map(|(t, l)| format!("{t}: {}", l.join(", "))).collect::<Vec<_>>().join(" · ");
    let ok = match &ctx.confirm {
        Some(confirm) => {
            let message = if exists { format!("Mod «{}» ändern?", m.name) } else { format!("Mod «{}» speichern?", m.name) };
            let detail = format!("{}\n\n{what}\n\nDer Mod ist danach aus. Einschalten unter Einstellungen → Mods, nach einer Vorschau des Codes.", clip_line(&m.beschreibung, 200));
            confirm(message, detail, ConfirmOptions::default()).await.ok
        }
        None => false,
    };
    if !ok {
        return Ok(ToolOutput::new(false, "Die Person hat das Speichern abgelehnt.".into(), "abgelehnt"));
    }
    let was_on = exists && mods.info(&id).is_some_and(|i| i.aktiv);
    let info = mods.save(&files, true)?;
    if let Some(k) = &ctx.knowledge {
        k.changed();
        if was_on { if let Some(f) = &k.mods_changed { f(); } }
    }
    let state = if was_on { "Er war eingeschaltet und ist jetzt aus, weil er geändert wurde" } else { "Er ist aus" };
    Ok(ToolOutput::new(true, format!(
        "Mod «{}» gespeichert ({what}). {state}: Noch ist in ASK nichts davon zu sehen. Prüfe ihn jetzt mit mod_testen (Bildschirmfoto, Fehler, CSS ohne Treffer) und verbessere ihn, bis die Wirkung stimmt. \
Sag der Person erst dann, dass er fertig ist, und biete an, ihn mit mod_einschalten einzuschalten.", info.manifest.name),
        format!("{} gespeichert", info.manifest.id)))
}

async fn skill_erstellen(a: &Value, ctx: &MemoryToolCtx) -> Result<ToolOutput> {
    let skills = need_skills(ctx)?;
    let slug = slugify(&field(a, "name"));
    let problem = check_name(&slug);
    if !problem.is_empty() {
        bail!("{problem}");
    }
    let exists = skills.own(&slug).is_some();
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
                mods: None,
                catalog: None,
                catalog_sources: None,
                mods_changed: None,
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
        assert_eq!(names, ["merken", "vergessen", "skill_lesen", "skill_erstellen", "mod_liste", "mod_lesen", "katalog_suchen", "katalog_hinzufuegen", "mod_testen", "mod_einschalten", "mod_erstellen"]);
        assert_eq!(specs[0]["type"], "function");
        assert_eq!(specs[0]["function"]["parameters"]["required"], json!(["text", "bereich"]));
        let keys: Vec<&String> = specs[0]["function"]["parameters"]["properties"].as_object().unwrap().keys().collect();
        assert_eq!(keys, ["text", "bereich", "tags"], "Reihenfolge wie im Original");
    }

    #[tokio::test]
    async fn werkzeug_mod_erstellen_fragt_und_speichert_aus() {
        let t = tempfile::tempdir().unwrap();
        let mods = Arc::new(crate::mods::Mods::new(t.path().join("mods"), None));
        let answer = Arc::new(AtomicBool::new(false));
        let answer2 = answer.clone();
        let confirm: ConfirmFn = Arc::new(move |_m, _d, _o| {
            let ok = answer2.load(Ordering::SeqCst);
            Box::pin(async move { Confirmed { ok, checked: false } })
        });
        let ctx = MemoryToolCtx {
            knowledge: Some(KnowledgeCtx { mods: Some(mods.clone()), ..Default::default() }),
            confirm: Some(confirm),
            ..Default::default()
        };
        let args = json!({ "dateien": { "mod.json": "{\"id\":\"neon\",\"name\":\"Neon\",\"style\":\"style.css\"}", "style.css": ":root{--accent:#0ff}" } });
        let r = run_tool("mod_erstellen", args.clone(), &ctx).await;
        assert!(!r.ok, "abgelehnt");
        assert!(mods.list().is_empty());
        answer.store(true, Ordering::SeqCst);
        let r = run_tool("mod_erstellen", args, &ctx).await;
        assert!(r.ok, "{}", r.output);
        let m = mods.info("neon").unwrap();
        assert!(!m.aktiv, "neu ist aus");
        let r = run_tool("mod_erstellen", json!({ "dateien": { "mod.json": "{}" } }), &ctx).await;
        assert!(!r.ok && r.output.contains("Nicht gespeichert"), "{}", r.output);
        // Ansehen
        let r = run_tool("mod_liste", json!({}), &ctx).await;
        assert!(r.ok && r.output.contains("Neon (id neon, aus") && r.output.contains("Aussehen"), "{}", r.output);
        let r = run_tool("mod_lesen", json!({ "id": "neon" }), &ctx).await;
        assert!(r.ok && r.output.contains("--- style.css ---\n:root{--accent:#0ff}"), "{}", r.output);
        let r = run_tool("mod_lesen", json!({ "id": "neon", "datei": "fehlt.js" }), &ctx).await;
        assert!(!r.ok);
        assert!(!run_tool("mod_lesen", json!({ "id": "gibts-nicht" }), &ctx).await.ok);
        // Ändern: nur die geänderte Datei, eine neue dazu, die mod.json bleibt
        let r = run_tool("mod_erstellen", json!({ "id": "neon", "dateien": { "style.css": ":root{--acc:#f0f}", "bild.svg": "<svg/>" } }), &ctx).await;
        assert!(r.ok && r.output.contains("Neu: bild.svg") && r.output.contains("Geändert: style.css"), "{}", r.output);
        let r = run_tool("mod_lesen", json!({ "id": "neon", "datei": "style.css" }), &ctx).await;
        assert!(r.output.contains("--acc:#f0f") && r.output.contains("mod.json"), "{}", r.output);
        // Löschen mit leerem Inhalt; die nötige style.css lässt sich nicht löschen
        assert!(run_tool("mod_erstellen", json!({ "id": "neon", "dateien": { "bild.svg": "" } }), &ctx).await.ok);
        let r = run_tool("mod_erstellen", json!({ "id": "neon", "dateien": { "style.css": "" } }), &ctx).await;
        assert!(!r.ok && r.output.contains("style.css"), "{}", r.output);
        let r = run_tool("mod_erstellen", json!({ "id": "neon", "dateien": { "style.css": ":root{--acc:#f0f}" } }), &ctx).await;
        assert!(!r.ok && r.output.contains("Nichts geändert"), "{}", r.output);
        // Einschalten nur mit Bestätigung, ausschalten ohne
        answer.store(false, Ordering::SeqCst);
        let r = run_tool("mod_einschalten", json!({ "id": "neon" }), &ctx).await;
        assert!(!r.ok && !mods.info("neon").unwrap().aktiv, "abgelehnt: {}", r.output);
        answer.store(true, Ordering::SeqCst);
        let r = run_tool("mod_einschalten", json!({ "id": "neon" }), &ctx).await;
        assert!(r.ok && mods.info("neon").unwrap().aktiv, "{}", r.output);
        assert!(run_tool("mod_einschalten", json!({ "id": "neon" }), &ctx).await.output.contains("schon an"));
        answer.store(false, Ordering::SeqCst);
        let r = run_tool("mod_einschalten", json!({ "id": "neon", "an": false }), &ctx).await;
        assert!(r.ok && !mods.info("neon").unwrap().aktiv, "ohne Rückfrage aus: {}", r.output);
        assert!(!run_tool("mod_einschalten", json!({ "id": "fehlt" }), &ctx).await.ok);
    }
}
