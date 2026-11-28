//! Steuerung der App (früher src/main/main.js): eigene Chat-Oberfläche für ASK (Open WebUI der TF Bern) mit
//! lokalem Arbeitsordner. Hält die Projekte und ihre Laufzeiten (Terminal, Beobachtung des Ordners, Server,
//! Agenten), liefert den Zustand für die Oberfläche und beantwortet alle Aufrufe der Oberfläche (`handle`).
//! Fenster und Webansichten liefert die Tauri-Hülle über `Host`, `AskView` (askapi.rs), `PageProvider`
//! (visual.rs) und `UpdateBackend` (updater.rs).
//!
//! Parallele Chats (ENTSCHEIDE E52): Jedes Projekt, in dem gearbeitet wird, hat sein eigenes Terminal und seine
//! eigene Beobachtung des Ordners, jeder laufende Chat seinen eigenen Agenten. `terminal`, `workspace` und
//! `agent` zeigen immer auf das sichtbare Projekt bzw. den sichtbaren Chat; was im Hintergrund läuft, bleibt in
//! `runtimes` stehen, bis es fertig ist.

use crate::agent::{Agent, AgentOptions, NEW};
use crate::askapi::{AskSession, AskView, ChatApi, THINKING_LEVELS};
use crate::attachments::{image_url, native_resize, save_attachments, AttachmentFile};
use crate::bild::Images;
use crate::github::{FnTokenStore, GitHub};
use crate::host::{ConfirmFn, ConfirmOptions, Confirmed, Host};
use crate::knowledge::{Knowledge, KnowledgeConfig};
use crate::review::ReviewFn;
use crate::servers::{Servers, ServersOptions};
use crate::snapshot::Snapshots;
use crate::terminal::{StartOptions, Terminal};
use crate::tools::{run_tool, servers_request_fn, GithubTrust, TerminalEndpoint, ToolCtx};
use crate::updater::{UpdateBackend, Updater, UpdaterOptions};
use crate::util::{bool_of, bytes_of, js_string, now_ms, path_key, same_path, str_of, truthy, win_basename};
use crate::visual::{PageProvider, Visual};
use crate::web::Web;
use crate::workspace::{ImportFile, Workspace};
use crate::{paths, settings};
use futures::future::BoxFuture;
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

/// Was die Hülle beim Start übergibt
/// Interaktive Antworten (ENTSCHEIDE E80): kurzer Hinweis im Systemprompt, die Anleitung steht im Skill
const INTERACTIVE_HINT: &str = "\n\n# Interaktive Antworten\nDie Oberfläche kann Teile deiner Antwort als interaktive Bausteine zeigen: \
einen Block ```ask-ui mit JSON (Diagramm, Quiz, Karteikarten, Rechner, Auswahl, Formular, Schritte, Vergleich, Hinweis) oder ```ask-html \
mit eigenem HTML und JavaScript ohne Internet. Benutze sie von selbst, wo sie mehr helfen als Text: Zahlen vergleichen, üben und \
abfragen, mit Werten spielen, aus Möglichkeiten wählen lassen, Schritte abhaken. Kurze Antworten und reine Erklärungen bleiben Text. \
Nimm zuerst die festen Bausteine (```ask-ui) und baue sie nie in HTML nach; ```ask-html nur, wenn keiner passt. Bevor du einen Baustein schreibst, lies den Skill «interaktive-antworten» mit skill_lesen und halte dich genau an das Format; \
schreib den Block nie in eine Datei, sondern direkt in die Antwort, mit einem Satz Text davor.";

pub struct AppOptions {
    pub host: Arc<dyn Host>,
    pub ask_view: Arc<dyn AskView>,
    pub pages: Arc<dyn PageProvider>,
    pub update_backend: Arc<dyn UpdateBackend>,
    /// Anfang der Adressen für Dateien des Arbeitsordners (Protokoll askfile): unter Windows
    /// «http://askfile.localhost/datei/», sonst «askfile://datei/»
    pub file_base: String,
}

/// Laufzeit eines Projekts: Terminal, Beobachtung des Ordners, Server, Agenten
pub struct Runtime {
    pub folder: String,
    pub terminal: Terminal,
    pub workspace: Arc<Workspace>,
    pub servers: Arc<Servers>,
    pub agents: Mutex<Vec<Arc<Agent>>>,
}

impl Runtime {
    fn agents(&self) -> Vec<Arc<Agent>> {
        self.agents.lock().clone()
    }
    fn has(&self, a: &Arc<Agent>) -> bool {
        self.agents.lock().iter().any(|x| Arc::ptr_eq(x, a))
    }
    fn busy(&self) -> bool {
        self.agents().iter().any(|a| a.running() || a.undoing())
    }
}

struct St {
    runtimes: HashMap<String, Arc<Runtime>>,
    terminal: Terminal,
    workspace: Arc<Workspace>,
    agent: Option<Arc<Agent>>,
    /// Chat, der sichtbar werden soll, sobald das Terminal seines Ordners läuft: (Schlüssel des Ordners, Chat)
    pending_chat: Option<(String, String)>,
    quitting: bool,
}

/// Offene Dialoge und Menüs der Oberfläche (ENTSCHEIDE E56)
#[derive(Default)]
struct Ui {
    seq: u64,
    ready: bool,
    waits: HashMap<u64, (tokio::sync::oneshot::Sender<Value>, Value)>,
}

type Action = Arc<dyn Fn(bool) -> BoxFuture<'static, ()> + Send + Sync>;

/// Eintrag eines Menüs wie bei Menu.buildFromTemplate: label, click, type (separator, radio, checkbox), checked,
/// enabled, submenu; dazu swatch (Farbfeld) und hint (Tastenkürzel)
#[derive(Clone, Default)]
struct MenuItem {
    label: String,
    kind: &'static str,
    checked: bool,
    enabled: bool,
    swatch: Option<String>,
    submenu: Vec<MenuItem>,
    click: Option<Action>,
}

impl MenuItem {
    fn new(label: impl Into<String>, click: Action) -> Self {
        Self { label: label.into(), kind: "normal", enabled: true, click: Some(click), ..Default::default() }
    }
    fn separator() -> Self {
        Self { kind: "separator", ..Default::default() }
    }
    fn sub(label: impl Into<String>, submenu: Vec<MenuItem>) -> Self {
        Self { label: label.into(), kind: "normal", enabled: true, submenu, ..Default::default() }
    }
}

pub const THEMES: [(&str, &str); 3] = [("system", "system"), ("hell", "light"), ("dunkel", "dark")];
/// Modus im Eingabefeld (ENTSCHEIDE E52): auto (ohne Rückfragen), nachfragen, plan
pub const CHAT_MODES: [&str; 3] = ["auto", "nachfragen", "plan"];
const NOTES: &str = ".ask/projekt.md";
/// Seite mit den Neuerungen aller Versionen (Release-Repository aus app-config.json)
fn releases_url() -> String {
    let c = settings::app_config()["updates"].clone();
    match (c.get("owner").and_then(Value::as_str), c.get("repo").and_then(Value::as_str)) {
        (Some(o), Some(r)) if !o.is_empty() && !r.is_empty() => format!("https://github.com/{o}/{r}/releases"),
        _ => String::new(),
    }
}

/// Zusatz in Rückfragen von Windows, solange Code eines Mods läuft (E72)
const MOD_NOTE: &str = "Diese Abfrage kommt von Windows, weil ein Mod mit Code eingeschaltet ist; ein Mod kann sie nicht beantworten. Hast du das nicht selbst ausgelöst, wähle «Nein» und schalte die Mods aus (Ctrl+Shift+M).";
const NOTES_MAX: usize = 6000;
/// Kontextgrösse in Token (Einstellungen → Modelle, ENTSCHEIDE E78): so viel Verlauf geht höchstens an das Modell.
/// Meldet ASK für ein Modell ein kleineres Fenster, gilt das kleinere.
pub const CONTEXT_DEFAULT: i64 = 250_000;
const CONTEXT_RANGE: (i64, i64) = (8_000, 2_000_000);
/// Reserve: Ist der Verlauf bis auf so viele Token voll, fasst ASK die älteren Aufträge zusammen
pub const RESERVE_DEFAULT: i64 = 30_000;
const RESERVE_RANGE: (i64, i64) = (2_000, 500_000);

fn context_size() -> i64 {
    settings::get("contextSize").as_i64().map(|v| v.clamp(CONTEXT_RANGE.0, CONTEXT_RANGE.1)).unwrap_or(CONTEXT_DEFAULT)
}

fn context_reserve() -> i64 {
    settings::get("contextReserve").as_i64().map(|v| v.clamp(RESERVE_RANGE.0, RESERVE_RANGE.1)).unwrap_or(RESERVE_DEFAULT)
}

/// Fenster, mit dem die App für ein Modell rechnet: das eingestellte, höchstens aber, was ASK zum Modell meldet
/// (oder einmal in einer Fehlermeldung genannt hat; 0: nichts bekannt)
pub fn effective_context(reported: i64, learned: i64, setting: i64) -> i64 {
    let known = if reported > 0 { reported } else { learned };
    if known > 0 { known.min(setting) } else { setting }
}
const BUSY: &str = "Das Modell arbeitet noch in diesem Projekt. Zuerst stoppen (Esc).";
const NOTIFY_MIN_MS: i64 = 8000; // kurze Aufträge ohne Meldung

const TEXT_EXT: [&str; 23] = [".txt", ".md", ".csv", ".json", ".py", ".js", ".html", ".css", ".xml", ".log", ".yml",
    ".yaml", ".ini", ".toml", ".bat", ".ps1", ".sql", ".tsv", ".java", ".c", ".cpp", ".h", ".ts"];
const IMAGE_EXT: [&str; 7] = [".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg", ".bmp"];
const OFFICE_EXT: [&str; 9] = [".docx", ".doc", ".xlsx", ".xls", ".pptx", ".ppt", ".odt", ".ods", ".odp"];
/// Dateiarten, die ein Doppelklick ohne Rückfrage im zugehörigen Programm öffnet: Dokumente, Bilder, Medien,
/// Text. Alles andere (exe, bat, cmd, lnk, js, vbs, hta, ps1, py, msi, reg, Makro-Dokumente, Unbekanntes) fragt nach.
const SAFE_OPEN_EXT: [&str; 39] = [".txt", ".md", ".csv", ".tsv", ".json", ".xml", ".yml", ".yaml", ".ini", ".toml", ".log", ".css", ".sql",
    ".html", ".htm", ".svg", ".pdf", ".docx", ".xlsx", ".pptx", ".odt", ".ods", ".odp", ".rtf", ".png", ".jpg", ".jpeg", ".gif", ".webp",
    ".bmp", ".ico", ".tif", ".tiff", ".mp3", ".wav", ".ogg", ".m4a", ".mp4", ".webm"];
const SAFE_OPEN_EXT2: [&str; 2] = [".mov", ".zip"];

fn color_name(c: &str) -> &str {
    match c {
        "#0082B4" => "TF Blau",
        "#F28855" => "Orange",
        "#4EB68E" => "Grün",
        "#EE716D" => "Rot",
        "#60C5E2" => "Hellblau",
        "#A2CE8D" => "Hellgrün",
        "#A5ACAE" => "Grau",
        _ => c,
    }
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default()
}

/// Ergebnis { ok: false, reason }
fn fail(reason: impl Into<String>) -> Value {
    json!({ "ok": false, "reason": reason.into() })
}

fn arg(args: &[Value], i: usize) -> Value {
    args.get(i).cloned().unwrap_or(Value::Null)
}

fn sarg(args: &[Value], i: usize) -> String {
    js_string(&arg(args, i))
}

/// Ordnername für ein neues Projekt
pub fn safe_folder_name(name: &str) -> String {
    let mut clean: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || (c as u32) < 0x20 { ' ' } else { c }).collect();
    clean = clean.trim_end_matches(['.', ' ']).to_string();
    let clean: String = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let clean = crate::util::take_chars(clean.trim(), 60);
    let lower = clean.to_lowercase();
    let device = matches!(lower.as_str(), "con" | "prn" | "aux" | "nul")
        || ((lower.starts_with("com") || lower.starts_with("lpt")) && lower.len() == 4 && lower.as_bytes()[3].is_ascii_digit());
    if device { format!("{clean}-Projekt") } else { clean }
}

/// Token aus dem Browser für ASK (ENTSCHEIDE E57), verschlüsselt mit dem Windows-Konto in den Einstellungen
struct AskTokenStore;

impl crate::askapi::TokenStore for AskTokenStore {
    fn get(&self) -> String {
        crate::secrets::unprotect(settings::get("askToken").as_str().unwrap_or(""))
    }
    fn set(&self, token: &str) {
        settings::set(json!({ "askToken": crate::secrets::protect(token) }));
    }
}

pub struct App {
    pub(crate) host: Arc<dyn Host>,
    pub ask: Arc<AskSession>,
    github: Arc<GitHub>,
    github_trust: Arc<GithubTrust>,
    web: Arc<Web>,
    images: Arc<Images>,
    visual: Arc<Visual>,
    pub knowledge: Arc<Knowledge>,
    /// Kataloge für Skills und Mods auf GitHub (catalog.rs, E71)
    pub(crate) catalog: Arc<crate::catalog::Catalog>,
    pub updater: Arc<Updater>,
    reviewer: ReviewFn,
    /// Ausweich-DNS über Quad9 für die Anfragen der App (ENTSCHEIDE E57, E58)
    pub dns: Arc<crate::doh::FallbackDns>,
    /// Ausweich-DNS eingeschaltet, aber noch nicht bestätigt (ASK hat danach noch nicht geladen)
    dns_pending: std::sync::atomic::AtomicBool,
    ask_view: Arc<dyn AskView>,
    file_base: String,
    test_mode: bool,
    st: Mutex<St>,
    snapshots: Mutex<HashMap<String, Arc<Snapshots>>>,
    ui: Mutex<Ui>,
    /// Die geladene Oberfläche hat Code eines Mods ausgeführt (bis sie neu lädt, E72)
    ui_mod_code: std::sync::atomic::AtomicBool,
    orders: Mutex<HashSet<PathBuf>>,
    me: Weak<App>,
}

/// Laufzeit für Aufgaben: Rückrufe kommen auch aus anderen Threads (Beobachtung des Ordners, Klick auf eine
/// Benachrichtigung), dort gibt es keine aktuelle tokio-Laufzeit.
static RUNTIME: std::sync::OnceLock<tokio::runtime::Handle> = std::sync::OnceLock::new();

fn spawn<F>(f: F) -> tokio::task::JoinHandle<F::Output>
where
    F: std::future::Future + Send + 'static,
    F::Output: Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(h) => h.spawn(f),
        Err(_) => RUNTIME.get().expect("App::new in einer tokio-Laufzeit").spawn(f),
    }
}

fn make_terminal() -> Terminal {
    // Der Terminal-Server braucht kein Python mehr (ENTSCHEIDE E61). Python für die Skripte des Modells entpackt
    // die installierte App im Hintergrund (App::start); Befehle mit Python warten darauf (tools.rs).
    Terminal::native()
}

impl App {
    /// Muss in einer tokio-Laufzeit aufgerufen werden (die Hülle: tauri::async_runtime)
    pub fn new(o: AppOptions) -> Arc<Self> {
        if let Ok(h) = tokio::runtime::Handle::try_current() {
            let _ = RUNTIME.set(h);
        }
        let test_mode = paths::test_mode();
        let env_on = |k: &str| std::env::var(k).map(|v| v == "1").unwrap_or(false);
        let dns = Arc::new(crate::doh::FallbackDns::new());
        dns.set_host(&str_of(&settings::app_config(), "askUrl"));
        let client = Web::client_builder(test_mode).dns_resolver(dns.clone()).build().unwrap_or_default();
        let ask = Arc::new(AskSession::with_client(client.clone()));
        // Token aus dem Browser (ENTSCHEIDE E57), verschlüsselt wie das GitHub-Token
        ask.set_token_store(Arc::new(AskTokenStore));
        ask.attach(o.ask_view.clone());

        // GitHub-Token verschlüsselt mit dem Windows-Konto (DPAPI) in den Einstellungen; nur hier lesbar.
        let store = FnTokenStore {
            get: Box::new(|| crate::secrets::unprotect(settings::get("githubToken").as_str().unwrap_or(""))),
            set: Box::new(|token: &str| settings::set(json!({ "githubToken": crate::secrets::protect(token) }))),
        };
        let mut github = GitHub::new(Arc::new(store)).with_link_file(|root| project_file(&root.to_string_lossy(), "github.json", ".ask/github.json"));
        if test_mode {
            if let Ok(api) = std::env::var("ASK_TEST_GITHUB_API") {
                github = github.with_api(&api);
            }
        }
        let trusted: Vec<String> = settings::get("githubTrusted").as_array().map(|a| a.iter().map(js_string).collect()).unwrap_or_default();
        let github_trust = Arc::new(GithubTrust::with_store(trusted, |list| settings::set(json!({ "githubTrusted": list }))));

        // Eigener Client fürs Web: Weiterleitungen prüft Web::get Schritt für Schritt (auch per DNS)
        let web_client = Web::fetch_client_builder(test_mode).dns_resolver(dns.clone()).build().unwrap_or_else(|_| client.clone());
        let web = Arc::new(Web::new(Some(ask.clone()), web_client, test_mode));

        Arc::new_cyclic(|me: &Weak<App>| {
            let m1 = me.clone();
            let images = Arc::new(Images::new(ask.clone(), Arc::new(move || m1.upgrade().map(|a| a.current_model()).unwrap_or_default()))
                .with_client(client.clone())
                .with_allow_local(test_mode));
            let visual = Arc::new(Visual::new(o.pages.clone(), test_mode));
            let m2 = me.clone();
            let m3 = me.clone();
            let m4 = me.clone();
            let host = o.host.clone();
            let catalog = Arc::new(crate::catalog::Catalog::new(client.clone()));
            let mut kcfg = KnowledgeConfig::new(paths::user_data());
            kcfg.ask = Some(ask.clone() as Arc<dyn ChatApi>);
            kcfg.get_model = Arc::new(move || m2.upgrade().map(|a| a.current_model()).unwrap_or_default());
            kcfg.send = Arc::new(move |ch: &str, v: Value| host.send(ch, v));
            kcfg.log = Arc::new(|m: &str| eprintln!("[gedaechtnis] {m}"));
            kcfg.projects = Arc::new(|| {
                let p = settings::load()["projects"].as_array().cloned().unwrap_or_default();
                Value::Array(p.iter().map(|x| json!({ "folder": x["folder"], "name": x["name"], "color": x["color"] })).collect())
            });
            kcfg.on_settings = Arc::new(move || { if let Some(a) = m3.upgrade() { a.push_state(); } });
            kcfg.standard_skills = Some(paths::standard_skills_dir());
            kcfg.catalog = Some(catalog.clone());
            kcfg.catalog_sources = Arc::new(|| crate::extensions::catalogs().iter().filter_map(|c| c["repo"].as_str().map(str::to_string)).collect());
            let knowledge = Arc::new(Knowledge::new(kcfg));

            // Prüfer für den Modus «Auto»: eigenes Modell aus den Einstellungen, sonst das Modell des Chats
            let ask_r = ask.clone();
            let reviewer = crate::review::make_reviewer(ask.clone() as Arc<dyn ChatApi>, move || {
                let wanted = settings::get("reviewModel").as_str().unwrap_or("").to_string();
                if ask_r.models().iter().any(|mm| mm.id == wanted) { wanted } else { m4.upgrade().map(|a| a.current_model()).unwrap_or_default() }
            }, None).into_fn();

            // Updates (ENTSCHEIDE E37, E60): fragt vor dem Herunterladen und vor dem Neustart, nie während eines Auftrags.
            let cfg = if env_on("ASK_NO_UPDATES") { Value::Null } else { settings::app_config()["updates"].clone() };
            let config = match (cfg.get("owner").and_then(Value::as_str), cfg.get("repo").and_then(Value::as_str)) {
                (Some(o), Some(r)) if !o.is_empty() && !r.is_empty() => Some((o.to_string(), r.to_string())),
                _ => None,
            };
            let portable = crate::portable::is_portable(&paths::config().exe, std::env::var("LOCALAPPDATA").ok().as_deref());
            let (m5, m6, m7) = (me.clone(), me.clone(), me.clone());
            let host2 = o.host.clone();
            let updater = Updater::new(UpdaterOptions {
                backend: o.update_backend.clone(),
                config,
                packaged: paths::is_packaged(),
                portable,
                current: o.host.app_version(),
                beta: Arc::new(|| truthy(&settings::get("betaUpdates"))),
                busy: Arc::new(move || m5.upgrade().is_some_and(|a| a.any_running())),
                ask: Arc::new(move |opts: Value| {
                    let m = m6.clone();
                    Box::pin(async move {
                        let Some(a) = m.upgrade() else { return -1 };
                        if a.test_mode { return -1; }
                        a.show_box(opts).await.get("response").and_then(Value::as_i64).unwrap_or(-1)
                    })
                }),
                open_external: Arc::new(move |url: &str| host2.open_external(url)),
                before_install: Arc::new(move || {
                    let m = m7.clone();
                    Box::pin(async move { if let Some(a) = m.upgrade() { a.stop_everything().await; } })
                }),
                log: Arc::new(|m: &str| eprintln!("[update] {m}")),
            });

            let terminal = make_terminal();
            let workspace = Arc::new(Workspace::new());
            App {
                host: o.host,
                ask,
                github: Arc::new(github),
                github_trust,
                web,
                images,
                visual,
                knowledge,
                catalog,
                updater,
                reviewer,
                dns,
                dns_pending: std::sync::atomic::AtomicBool::new(false),
                ask_view: o.ask_view.clone(),
                file_base: o.file_base,
                test_mode,
                st: Mutex::new(St { runtimes: HashMap::new(), terminal, workspace, agent: None, pending_chat: None, quitting: false }),
                snapshots: Mutex::new(HashMap::new()),
                ui: Mutex::new(Ui::default()),
                ui_mod_code: std::sync::atomic::AtomicBool::new(false),
                orders: Mutex::new(HashSet::new()),
                me: me.clone(),
            }
        })
    }

    fn send(&self, channel: &str, payload: Value) {
        self.host.send(channel, payload);
    }

    fn terminal(&self) -> Terminal {
        self.st.lock().terminal.clone()
    }

    pub(crate) fn workspace(&self) -> Arc<Workspace> {
        self.st.lock().workspace.clone()
    }

    fn agent(&self) -> Arc<Agent> {
        if let Some(a) = self.st.lock().agent.clone() {
            return a;
        }
        // Ohne Projekt: ein Agent ohne Ordner (zeigt den leeren Chat)
        let a = self.make_agent("", None);
        self.st.lock().agent = Some(a.clone());
        a
    }

    // ---- Start und Ende -------------------------------------------------------------------------------------

    /// Wie app.whenReady in main.js
    pub async fn start(self: &Arc<Self>) {
        self.prepare_python();
        // Ausweich-DNS vor dem ersten Laden (ENTSCHEIDE E57). «secureDns» aus 0.3.7 galt auch nach einer vertippten
        // Adresse: verworfen (E58). Gemerkt wird es mit der Adresse und gilt nur, wenn sie noch die der Schule ist.
        settings::remove("secureDns");
        let dns_for = js_string(&settings::get("secureDnsFor"));
        let school = str_of(&settings::app_config(), "askUrl");
        if !dns_for.is_empty() && crate::askapi::same_origin(&dns_for, &school) && crate::askapi::same_origin(&dns_for, &js_string(&settings::get("askUrl"))) {
            self.dns.enable();
        }
        self.note_first_install();
        self.skip_tour_for_existing_users();
        self.apply_theme();
        self.wire_events();
        self.load_ask();
        let s = settings::load();
        let folder = str_of(&s, "folder");
        if str_of(&s, "askUrl").is_empty() {
            let _ = self.agent();
        } else if folder.is_empty() || !bool_of(&self.open_folder(&folder, "").await, "ok") {
            // Noch kein Projekt, oder der Ordner des letzten Projekts fehlt: Chats ohne Projekt (ENTSCHEIDE E78)
            self.open_free().await;
        }
        crate::portable::cleanup_old_versions();
        self.updater.start();
        // Geteilte Docs (ENTSCHEIDE E82) abgleichen; geänderte Docs der Oberfläche melden
        let me = self.me.clone();
        crate::docsync::start(move |project, rel| {
            if let Some(a) = me.upgrade() {
                a.send_ui("docs:sync", json!({ "projekt": project, "pfad": rel }));
            }
        });
    }

    /// Installierte App und einzelne EXE: Python einmal entpacken (ENTSCHEIDE E39), ohne den Start aufzuhalten.
    /// Gleich zu Beginn von `start`, damit es fertig ist, bevor das Modell Python braucht; der Fortschritt
    /// steht unten links (Zustand «python», E68).
    fn prepare_python(self: &Arc<Self>) {
        if !paths::is_packaged() || crate::python::shared().ready() {
            return;
        }
        let me = Arc::downgrade(self);
        spawn(async move {
            let shown = std::sync::atomic::AtomicI32::new(-1);
            let report = |p: u32| {
                let last = shown.load(std::sync::atomic::Ordering::SeqCst);
                if last < 0 || p as i32 >= last + 5 || p >= 100 {
                    shown.store(p as i32, std::sync::atomic::Ordering::SeqCst);
                    PYTHON_PERCENT.store(p as i32, std::sync::atomic::Ordering::SeqCst);
                    if let Some(a) = me.upgrade() { a.push_state(); }
                }
            };
            report(0);
            if let Err(e) = crate::python::shared().ensure(report).await {
                eprintln!("Python nicht eingerichtet: {e}");
            }
            PYTHON_PERCENT.store(-1, std::sync::atomic::Ordering::SeqCst);
            if let Some(a) = me.upgrade() { a.push_state(); }
        });
    }

    /// Beim Beenden den laufenden Auftrag stoppen und abwarten, bis er gespeichert ist (höchstens 5 s)
    pub async fn shutdown(&self) {
        {
            let mut st = self.st.lock();
            if st.quitting {
                return;
            }
            st.quitting = true;
        }
        for a in self.all_agents() {
            a.stop();
        }
        self.updater.stop();
        let end = now_ms() + 5000;
        while self.any_running() && now_ms() < end {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Ein aufgeschobenes Update installiert sich beim Schliessen («Später» in der Karte)
        if self.updater.install_on_quit().await {
            return;
        }
        self.stop_everything().await;
    }

    async fn stop_everything(&self) {
        for a in self.all_agents() {
            a.stop();
        }
        self.visual.close_renderer().await;
        let (terminal, workspace, rts) = {
            let st = self.st.lock();
            (st.terminal.clone(), st.workspace.clone(), st.runtimes.values().cloned().collect::<Vec<_>>())
        };
        workspace.close();
        let mut waits = vec![spawn(terminal.stop())];
        for rt in rts {
            rt.workspace.close();
            waits.push(spawn(async move { stop_runtime(&rt).await }));
        }
        for w in waits {
            let _ = w.await;
        }
    }

    /// «Neu in ASK» (ENTSCHEIDE E65): nach jedem Update einmal der Text dieser Version. Eine frische Installation
    /// merkt sich die Version gleich; wer von der Electron-Fassung kommt (noch keine «lastVersion», Hinweis schon
    /// bestätigt), sieht die Karte auch.
    fn note_first_install(&self) {
        if settings::get("lastVersion").as_str().is_none() && !bool_of(&settings::load(), "warningAccepted") {
            settings::set(json!({ "lastVersion": self.host.app_version() }));
        }
    }

    fn whats_new(&self) -> Value {
        let current = self.host.app_version();
        if settings::get("lastVersion").as_str() == Some(current.as_str()) {
            return Value::Null;
        }
        whats_new_text(&crate::paths::resources(&["neu.json"]), &current)
    }

    fn skip_tour_for_existing_users(&self) {
        // Wer ASK schon benutzt hat, sieht die Einführung nach dem Update nicht von selbst (E49)
        let s = settings::load();
        let stored = s.get("prefs").cloned().unwrap_or(json!({}));
        if bool_of(&s, "warningAccepted") && !stored.get("tourDone").is_some_and(Value::is_boolean) {
            let mut p = if stored.is_object() { stored } else { json!({}) };
            p["tourDone"] = json!(true);
            settings::set(json!({ "prefs": p }));
        }
    }

    fn apply_theme(&self) {
        let t = settings::get("theme");
        let theme = THEMES.iter().find(|(k, _)| Some(*k) == t.as_str()).map(|(_, v)| *v).unwrap_or("system");
        self.host.set_theme(theme);
    }

    fn wire_events(self: &Arc<Self>) {
        let me = Arc::downgrade(self);
        self.updater.events.on("changed", move |_| {
            let Some(a) = me.upgrade() else { return };
            a.push_state();
            // Fortschritt auch in der Taskleiste: Download, danach der Countdown bis zum Neustart
            let u = a.updater.snapshot();
            let state = str_of(&u, "state");
            let bar = match state.as_str() {
                "lädt" => crate::util::num_of(&u, "percent") / 100.0,
                "neustart" | "installiert" => 2.0,
                _ => -1.0,
            };
            a.host.set_progress(bar);
        });
        let me = Arc::downgrade(self);
        self.ask.on("status", move |s| {
            let Some(a) = me.upgrade() else { return };
            match s.get("state").and_then(Value::as_str) {
                Some("angemeldet") => {
                    let images = a.images.clone();
                    spawn(async move { images.discover().await; });
                }
                Some("nicht-angemeldet") => a.images.forget(),
                _ => {}
            }
            // Ausweich-DNS hat geholfen: ASK lädt jetzt, also auch beim nächsten Start gleich so
            let state = str_of(s, "state");
            if a.dns_pending.load(std::sync::atomic::Ordering::SeqCst) && s.get("loadError").is_none()
                && (state == "nicht-angemeldet" || state == "angemeldet") && !a.ask.load_failed()
            {
                a.dns_pending.store(false, std::sync::atomic::Ordering::SeqCst);
                if let Ok(u) = url::Url::parse(&js_string(&settings::get("askUrl"))) {
                    settings::set(json!({ "secureDnsFor": u.origin().ascii_serialization() }));
                }
            }
            a.push_state();
        });
        // Name von ASK nicht gefunden: Ausweich-DNS über Quad9, nur für die Adresse der Schule (ENTSCHEIDE E57, E58)
        let me = Arc::downgrade(self);
        self.ask.on("name-not-resolved", move |failed| {
            let Some(a) = me.upgrade() else { return };
            let school = str_of(&settings::app_config(), "askUrl");
            if !crate::askapi::same_origin(&js_string(failed), &school) || !a.dns.enable() {
                return;
            }
            a.dns_pending.store(true, std::sync::atomic::Ordering::SeqCst);
            a.push_state();
            // gleich nochmals laden, nicht erst nach 15 Sekunden
            let view = a.ask_view.clone();
            let url = js_string(&settings::get("askUrl"));
            spawn(async move {
                tokio::time::sleep(Duration::from_millis(500)).await;
                if !url.is_empty() {
                    view.load_url(&url);
                }
            });
        });
        let me = Arc::downgrade(self);
        self.ask.on("models", move |_| { if let Some(a) = me.upgrade() { a.push_state(); } });
    }

    /// Die ASK-Seite dient nur der Anmeldung; die App liest danach das Token (askapi.rs).
    fn load_ask(&self) {
        let url = str_of(&settings::load(), "askUrl");
        if url.is_empty() {
            return;
        }
        self.ask.set_base(&url);
        let current = self.ask_view.url();
        if current.is_empty() || !crate::askapi::same_origin(&current, &url) {
            self.ask_view.load_url(&url); // Ladefehler meldet die Hülle (ask_load_failed)
        }
    }

    /// Die Hülle meldet: ASK-Seite beginnt zu laden (finished=false) oder ist geladen
    pub fn ask_page_load(&self, finished: bool, url: &str) {
        if finished {
            self.ask.on_view_navigated(url);
            self.ask.on_view_loaded();
        } else {
            self.ask.on_view_navigation_started(true, false);
        }
    }

    /// Die Hülle meldet: ASK-Seite nicht geladen (ENTSCHEIDE E53)
    pub fn ask_load_failed(&self, code: i32, description: &str, url: &str) {
        self.ask.on_view_failed(code as i64, description, url);
    }

    /// Oberfläche lädt neu oder Fenster zu: offene Dialoge gelten als abgebrochen
    pub fn ui_reloading(&self) {
        self.ui.lock().ready = false;
        self.ui_mod_code.store(false, std::sync::atomic::Ordering::SeqCst);
        self.ui_answer(None, None);
    }

    // ---- Zustand für die Oberfläche --------------------------------------------------------------------------

    fn prefs(&self) -> Value {
        // tourDone: Einführung gesehen (ENTSCHEIDE E49); notify: Benachrichtigungen (E52)
        let defaults = json!({ "verbose": false, "todoCollapsed": true, "notify": true, "tourDone": false });
        let stored = settings::get("prefs");
        let mut out = Map::new();
        for (k, v) in defaults.as_object().expect("Objekt") {
            let s = stored.get(k);
            let same_type = s.is_some_and(|s| s.is_boolean() == v.is_boolean());
            out.insert(k.clone(), if same_type { s.cloned().unwrap_or(Value::Null) } else { v.clone() });
        }
        Value::Object(out)
    }

    fn approval_mode(&self) -> String {
        let mode = settings::get("approvals");
        match mode.as_str() {
            Some(m) if crate::approvals::MODES.contains(&m) && m != "aus" => m.to_string(),
            _ => "heikel".into(),
        }
    }

    fn chat_mode(&self) -> String {
        match settings::get("mode").as_str() {
            Some(m) if CHAT_MODES.contains(&m) => m.to_string(),
            _ => "auto".into(),
        }
    }

    /// Denkaufwand aus den Einstellungen; ältere Werte des früheren Schalters werden übernommen.
    fn thinking_level(&self) -> String {
        let value = js_string(&settings::get("thinking"));
        match value.as_str() {
            "normal" => "standard".into(),
            "gruendlich" | "hoch" => "maximal".into(), // «hoch» gibt es nicht mehr (ASK kennt nur drei Stufen)
            "aus" => "tief".into(),
            v if THINKING_LEVELS.contains(&v) => v.to_string(),
            _ => "standard".into(),
        }
    }

    /// Qwen-Schalter für hohen Denkaufwand am Ende des Systemprompts (siehe THINKING_PARAMS in askapi.rs)
    fn thinking_switch(&self) -> &'static str {
        if self.thinking_level() == "maximal" { "\n\n/think" } else { "" }
    }

    /// Kurz, wie ASK gerade eingestellt ist, damit das Modell Fragen zur App beantworten kann (E77). Die Bedienung
    /// steht im mitgelieferten Skill «ask-hilfe».
    fn app_facts(&self) -> String {
        let s = settings::load();
        let theme = match js_string(&s["theme"]).as_str() { "hell" => "Hell", "dunkel" => "Dunkel", _ => "wie Windows (System)" };
        let mode = match self.chat_mode().as_str() { "auto" => "Auto", "plan" => "Plan", _ => "Nachfragen" };
        let model_name = self.ask.models().into_iter().find(|m| m.id == self.current_model()).map(|m| m.name).unwrap_or_else(|| self.current_model());
        let mods: Vec<String> = self.knowledge.mods.active().into_iter().map(|m| m.manifest.name).collect();
        let u = self.updater.snapshot();
        format!(
            "\n\n# Über die App\nDu arbeitest in ASK, der Desktop-App der TF Bern, Version {}. Fragt die Person, wie etwas in ASK geht, wo eine Einstellung ist oder was ein Knopf macht, lies zuerst den Skill «ask-hilfe» und antworte mit dem genauen Weg dorthin. \
Einstellungen wie Farbschema, Modus oder Modell änderst du nicht selbst; du sagst, wo die Person sie findet.\n\
Jetzt eingestellt: Farbschema {theme}; Modus {mode}; Modell {model_name}; Denkaufwand {}; Websuche {}; Gedächtnis {}; frühere Chats {}; Beta-Versionen {}; eingeschaltete Mods: {}.",
            self.host.app_version(), self.thinking_level(),
            if truthy(&s["webSearch"]) { "an" } else { "aus" },
            if self.knowledge.memory_on() { "an" } else { "aus" },
            if truthy(&s["chatMemory"]) { "an" } else { "aus" },
            if truthy(&u["beta"]) { "an" } else { "aus" },
            if mods.is_empty() { "keine".to_string() } else { mods.join(", ") },
        ) + if truthy(&s["interactiveUi"]) { INTERACTIVE_HINT } else { "" }
    }

    pub fn current_model(&self) -> String {
        let wanted = js_string(&settings::get("model"));
        let models = self.ask.models();
        if models.iter().any(|m| m.id == wanted) {
            return wanted;
        }
        models.first().map(|m| m.id.clone()).unwrap_or(wanted)
    }

    fn active_rt(&self) -> Option<Arc<Runtime>> {
        let st = self.st.lock();
        st.runtimes.values().find(|rt| rt.terminal.ptr_eq(&st.terminal)).cloned()
    }

    fn is_active_rt(&self, rt: &Runtime) -> bool {
        rt.terminal.ptr_eq(&self.st.lock().terminal)
    }

    fn all_agents(&self) -> Vec<Arc<Agent>> {
        let st = self.st.lock();
        let mut v: Vec<Arc<Agent>> = st.runtimes.values().flat_map(|rt| rt.agents()).collect();
        if let Some(a) = &st.agent {
            if !v.iter().any(|x| Arc::ptr_eq(x, a)) {
                v.push(a.clone());
            }
        }
        v
    }

    /// Arbeitet noch etwas? Auch ein laufendes «Rückgängig» zählt: Beenden oder ein Update mitten im Zurückkopieren
    /// liesse den Ordner halb zurückgesetzt.
    fn any_running(&self) -> bool {
        self.all_agents().iter().any(|a| a.running() || a.undoing())
    }

    /// Laufende Chats für die Seitenleiste (auch die im Hintergrund)
    fn running_chats(&self) -> Value {
        Value::Array(self.all_agents().iter().filter(|a| a.running())
            .map(|a| json!({ "folder": a.get_folder(), "id": a.chat_id(), "title": a.chat_title() })).collect())
    }

    fn terminal_ports(&self) -> Vec<u16> {
        let st = self.st.lock();
        let mut v = vec![st.terminal.port()];
        v.extend(st.runtimes.values().map(|rt| rt.terminal.port()));
        v.into_iter().filter(|p| *p != 0).collect()
    }

    pub fn state(&self) -> Value {
        let s = settings::load();
        let theme = js_string(&s["theme"]);
        let models = self.ask.models();
        json!({
            "askUrl": s["askUrl"],
            "secureDns": self.dns.enabled(),
            "folder": s["folder"],
            // Sichtbar sind die Chats ohne Projekt (E78): kein Terminal nötig, keine Dateien, kein GitHub
            "free": paths::is_free_dir(&js_string(&s["folder"])),
            "recent": s["recent"],
            "warningAccepted": s["warningAccepted"],
            "sidebarOpen": s["sidebarOpen"],
            "chatbarOpen": s["chatbarOpen"],
            "terminal": self.terminal().snapshot(),
            "ask": self.ask.status(),
            "models": models,
            "model": self.current_model(),
            "webSearch": truthy(&s["webSearch"]),
            "chatMemory": truthy(&s["chatMemory"]),
            "interactiveUi": truthy(&s["interactiveUi"]),
            "memory": self.knowledge.memory_on(),
            "thinking": self.thinking_level(),
            "webSearchAvailable": self.ask.web_search(),
            "chat": self.agent().state(),
            "running": self.running_chats(),
            "servers": self.active_rt().map(|rt| json!(rt.servers.list())).unwrap_or(json!([])),
            "update": self.updater.snapshot(),
            "prefs": self.prefs(),
            "version": self.host.app_version(),
            "whatsNew": self.whats_new(),
            "python": match PYTHON_PERCENT.load(std::sync::atomic::Ordering::SeqCst) { p if p >= 0 => json!({ "percent": p }), _ => Value::Null },
            "approvals": self.approval_mode(),
            "mode": self.chat_mode(),
            "contextSize": context_size(),
            "contextReserve": context_reserve(),
            "reviewModel": s.get("reviewModel").and_then(Value::as_str).unwrap_or(""),
            "helperModel": s.get("helperModel").and_then(Value::as_str).unwrap_or(""),
            "theme": if THEMES.iter().any(|(k, _)| *k == theme) { theme } else { "system".into() },
            "project": settings::find_project(&js_string(&s["folder"])).unwrap_or(Value::Null),
            "modsCode": self.mods_code(),
            "catalogs": crate::extensions::catalogs(),
            "releasesUrl": releases_url(),
        })
    }

    pub fn push_state(&self) {
        self.send("state", self.state());
    }

    // ---- Dialoge und Menüs in der eigenen Oberfläche (ENTSCHEIDE E56) ----------------------------------------

    fn ui_request(&self, channel: &str, mut payload: Value, cancel: Value) -> tokio::sync::oneshot::Receiver<Value> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let id = {
            let mut ui = self.ui.lock();
            ui.seq += 1;
            let id = ui.seq;
            ui.waits.insert(id, (tx, cancel));
            id
        };
        payload["id"] = json!(id);
        self.send(channel, payload);
        if !self.host.window_focused() {
            self.host.flash_frame(true);
        }
        rx
    }

    /// id None: alle offenen (Oberfläche lädt neu, Fenster zu); value None: abgebrochen
    fn ui_answer(&self, id: Option<u64>, value: Option<Value>) {
        let waits: Vec<(tokio::sync::oneshot::Sender<Value>, Value)> = {
            let mut ui = self.ui.lock();
            let keys: Vec<u64> = ui.waits.keys().copied().filter(|k| id.is_none_or(|i| i == *k)).collect();
            keys.into_iter().filter_map(|k| ui.waits.remove(&k)).collect()
        };
        for (tx, cancel) in waits {
            let _ = tx.send(value.clone().unwrap_or(cancel));
        }
    }

    /// Optionen von dialog.showMessageBox (dazu dangerId, kind 'update' usw.); gibt { response, checkboxChecked }
    pub async fn show_box(&self, options: Value) -> Value {
        let n = options.get("buttons").and_then(Value::as_array).map(Vec::len).unwrap_or(1) as i64;
        let cancel_id = options.get("cancelId").and_then(Value::as_i64).unwrap_or(n - 1);
        if !self.ui.lock().ready {
            return self.host.native_box(options).await;
        }
        // Läuft Code eines Mods in der Oberfläche, könnte er dort jeden Knopf drücken: Rückfragen kommen dann von
        // Windows (E72). Nur die Hinweise zu Updates bleiben in der Oberfläche.
        if self.mods_code() && options.get("kind").and_then(Value::as_str) != Some("update") {
            let mut o = options.clone();
            let detail = js_string(&o["detail"]);
            o["detail"] = json!(format!("{detail}{}", if detail.is_empty() { "" } else { "\n\n" }).to_string() + MOD_NOTE);
            o["title"] = json!("ASK – Bestätigung");
            return self.host.native_box(o).await;
        }
        let cancel = json!({ "response": cancel_id, "checkboxChecked": false });
        self.ui_request("ui:ask", options, cancel.clone()).await.unwrap_or(cancel)
    }

    /// Rückfrage vor Aktionen nach aussen. options.checkbox: Häkchen im Dialog; options.danger: «Ja» rot.
    /// In automatischen Tests: ja (Häkchen mit ASK_TEST_REMEMBER).
    pub async fn confirm(&self, message: &str, detail: &str, options: ConfirmOptions) -> Confirmed {
        let env_on = |k: &str| std::env::var(k).map(|v| v == "1").unwrap_or(false);
        if self.test_mode && env_on("ASK_TEST_AUTOCONFIRM") {
            return Confirmed { ok: true, checked: options.checkbox.is_some() && env_on("ASK_TEST_REMEMBER") };
        }
        let mut opts = json!({ "type": if options.danger { "warning" } else { "question" }, "buttons": ["Ja", "Nein"],
            "defaultId": 0, "cancelId": 1, "message": message, "detail": detail });
        if options.danger {
            opts["dangerId"] = json!(0);
        }
        if let Some(c) = &options.checkbox {
            opts["checkboxLabel"] = json!(c);
            opts["checkboxChecked"] = json!(false);
        }
        let res = self.show_box(opts).await;
        let ok = res.get("response").and_then(Value::as_i64) == Some(0);
        Confirmed { ok, checked: ok && bool_of(&res, "checkboxChecked") }
    }

    pub(crate) fn confirm_fn(&self) -> ConfirmFn {
        let me = self.me.clone();
        Arc::new(move |m, d, o| {
            let me = me.clone();
            Box::pin(async move {
                match me.upgrade() {
                    Some(a) => a.confirm(&m, &d, o).await,
                    None => Confirmed::default(),
                }
            })
        })
    }

    /// Menü an der Maus (bzw. unter dem angeklickten Knopf). Die Oberfläche bekommt nur Texte und Schlüssel und
    /// meldet den gewählten Schlüssel; click läuft hier.
    async fn popup_menu(&self, items: Vec<MenuItem>) {
        if !self.ui.lock().ready {
            return;
        }
        let mut clicks: HashMap<String, MenuItem> = HashMap::new();
        fn plain(list: &[MenuItem], prefix: &str, clicks: &mut HashMap<String, MenuItem>) -> Vec<Value> {
            list.iter().enumerate().map(|(i, it)| {
                if it.kind == "separator" {
                    return json!({ "separator": true });
                }
                let key = format!("{prefix}{i}");
                if it.click.is_some() {
                    clicks.insert(key.clone(), it.clone());
                }
                let mut v = json!({ "key": key, "label": it.label, "type": it.kind, "checked": it.checked, "enabled": it.enabled });
                if let Some(s) = &it.swatch {
                    v["swatch"] = json!(s);
                }
                if !it.submenu.is_empty() {
                    v["submenu"] = Value::Array(plain(&it.submenu, &format!("{key}."), clicks));
                }
                v
            }).collect()
        }
        let payload = json!({ "items": plain(&items, "", &mut clicks) });
        let key = self.ui_request("ui:menu", payload, Value::Null).await.unwrap_or(Value::Null);
        if let Some(it) = key.as_str().and_then(|k| clicks.get(k)) {
            if let Some(click) = &it.click {
                click(if it.kind == "checkbox" { !it.checked } else { true }).await;
            }
        }
    }

    fn action(&self, f: impl Fn(Arc<App>, bool) -> BoxFuture<'static, ()> + Send + Sync + 'static) -> Action {
        let me = self.me.clone();
        Arc::new(move |checked| match me.upgrade() {
            Some(a) => f(a, checked),
            None => Box::pin(async {}),
        })
    }

    // ---- Projekte, Laufzeiten, Agenten -----------------------------------------------------------------------

    fn snapshots_for(&self, folder: &str) -> Option<Arc<Snapshots>> {
        // Chats ohne Projekt ändern keine Dateien: keine Sicherung, kein «Rückgängig» (E78)
        if folder.is_empty() || paths::is_free_dir(folder) {
            return None;
        }
        let key = path_key(folder);
        let mut map = self.snapshots.lock();
        if !map.contains_key(&key) {
            let store = project_file(folder, "sicherung", ".ask/sicherung");
            map.insert(key.clone(), Arc::new(Snapshots::new(PathBuf::from(folder), Some(store))));
        }
        map.get(&key).cloned()
    }

    fn make_agent(&self, folder: &str, rt: Option<&Arc<Runtime>>) -> Arc<Agent> {
        let me = self.me.clone();
        let f = folder.to_string();
        let terminal = rt.map(|r| r.terminal.clone()).unwrap_or_else(|| self.terminal());
        let env_on = |k: &str| std::env::var(k).map(|v| v == "1").unwrap_or(false);
        let auto = self.test_mode && env_on("ASK_TEST_AUTOCONFIRM");
        let rt_weak = rt.map(Arc::downgrade);
        let ws = rt.map(|r| r.workspace.clone());
        let free = paths::is_free_dir(folder);
        let mut o = AgentOptions::new(self.ask.clone() as Arc<dyn ChatApi>, { let f = f.clone(); move || f.clone() },
            { let me = me.clone(); move || me.upgrade().map(|a| a.current_model()).unwrap_or_default() });
        o.terminal = Some(Arc::new(terminal) as Arc<dyn TerminalEndpoint>);
        o.get_system_prompt = Arc::new({
            let (me, f) = (me.clone(), f.clone());
            move |chat: &str| {
                let Some(a) = me.upgrade() else { return String::new() };
                let history = if truthy(&settings::get("chatMemory")) { crate::history::history_prompt(Path::new(&f), chat) } else { String::new() };
                // Ohne Projekt (E78): keine Projektnotizen und keine Vorlagen für Dateien
                let (notes, templates) = if free { (String::new(), String::new()) } else { (project_notes(&f), crate::vorlagen::system_prompt()) };
                // Quellen (E84): Unterlagen im Ordner «Quellen», nur in Projekten
                let quellen = if free { String::new() } else { let p = Path::new(&f); crate::sources::prompt(p, crate::sources::strict(p)) };
                // Docs (offenes Doc, Kommentare) und Datum (E85) am Schluss: Sie ändern sich oft, der Anfang davor bleibt gleich
                format!("{}{}{}{}{}{}{}{}{}{}", read_instruction("systemprompt.md", &f, chat), notes, quellen, templates, a.knowledge.system_prompt(&f), a.app_facts(), history, a.thinking_switch(), crate::docs::system_prompt(&f), crate::agent::date_prompt(chrono::Local::now()))
            }
        });
        o.get_command_hint = Arc::new({ let f = f.clone(); move |chat: &str| read_instruction("befehle.md", &f, chat) });
        // «Nur aus Quellen» (E84) schaltet die Websuche von ASK für dieses Projekt aus
        o.get_web_search = Arc::new({ let (me, f) = (me.clone(), f.clone()); move || me.upgrade().is_some_and(|a| a.ask.web_search()) && truthy(&settings::get("webSearch")) && (free || !crate::sources::strict(Path::new(&f))) });
        o.get_sources = Arc::new({ let f = f.clone(); move || if free { (false, false) } else { let p = Path::new(&f); (crate::sources::count(p) > 0, crate::sources::strict(p)) } });
        o.get_thinking = Arc::new({ let me = me.clone(); move || me.upgrade().map(|a| a.thinking_level()).unwrap_or_default() });
        o.github = Some(self.github.clone());
        o.github_trust = Some(self.github_trust.clone());
        o.visual = Some(self.visual.clone() as Arc<dyn crate::tools::VisualTools>);
        o.web = Some(self.web.clone());
        o.images = Some(self.images.clone());
        o.confirm = Some(self.confirm_fn());
        o.get_image_url = Some(Arc::new({ let f = f.clone(); move |att: &Value| image_url(Path::new(&f), &str_of(att, "path"), Some(&native_resize)) }));
        o.get_vision = Arc::new({ let me = me.clone(); move || me.upgrade().and_then(|a| { let m = a.current_model(); a.ask.models().into_iter().find(|x| x.id == m).and_then(|x| x.vision) }) });
        o.get_snapshots = Arc::new({ let (me, f) = (me.clone(), f.clone()); move || me.upgrade().and_then(|a| a.snapshots_for(&f)) });
        o.knowledge = Some(self.knowledge.for_agent());
        o.auto_title = !self.test_mode;
        o.free = free;
        o.get_ask_before = Arc::new({ let mods = self.knowledge.mods.clone(); move || mods.ask_before() });
        o.get_chat_memory = Arc::new(|| truthy(&settings::get("chatMemory")));
        // Hilfsagenten (ENTSCHEIDE E55): eigenes Modell aus den Einstellungen, sonst das Modell des Chats
        o.get_helper_model = Arc::new({ let me = me.clone(); move || {
            let wanted = js_string(&settings::get("helperModel"));
            if me.upgrade().is_some_and(|a| a.ask.models().iter().any(|m| m.id == wanted)) { wanted } else { String::new() }
        } });
        // Andere Chats, die gerade im selben Ordner arbeiten (Warnung, ENTSCHEIDE E46)
        o.get_siblings = Arc::new({ let rt_weak = rt_weak.clone(); let me = me.clone(); move || {
            let Some(rt) = rt_weak.as_ref().and_then(Weak::upgrade) else { return Vec::new() };
            let mine = me.upgrade().map(|a| a.agent().chat_id()).unwrap_or_default();
            rt.agents().iter().filter(|x| x.running() && x.chat_id() != mine).map(|x| { let t = x.chat_title(); if t.is_empty() { "ohne Titel".into() } else { t } }).collect()
        } });
        o.servers = rt.map(|r| r.servers.clone());
        // Modus im Eingabefeld: Auto lässt ASK jeden Befehl prüfen (review.rs), Nachfragen fragt vor jedem
        o.get_approval_mode = Arc::new(|| "alle".to_string());
        o.get_mode = Arc::new({ let me = me.clone(); move || me.upgrade().map(|a| a.chat_mode()).unwrap_or_else(|| "auto".into()) });
        o.review = Some(self.reviewer.clone());
        o.file_exists = Arc::new(move |rel: &str| ws.as_ref().is_some_and(|w| w.resolve(rel).is_some()));
        if auto {
            // In automatischen Tests: Rückfragen erlauben, Fragen mit der ersten Antwort beantworten
            o.auto_approve = Some(Arc::new(|_risk| Box::pin(async { "yes".to_string() })));
            o.auto_answer = Some(Arc::new(|_frage: String, optionen: Vec<String>| Box::pin(async move {
                Some(optionen.first().cloned().unwrap_or_else(|| "ja".into()))
            })));
        }
        // Kontextfenster: die eingestellte Grösse, höchstens aber, was ASK zum Modell angibt oder einmal in einer
        // Fehlermeldung genannt hat (E78)
        o.get_context_limit = Arc::new({ let me = me.clone(); move |model: &str| {
            let reported = me.upgrade().and_then(|a| a.ask.models().into_iter().find(|m| m.id == model)).map(|m| m.context).unwrap_or(0);
            let learned = settings::get("contextLimits").get(model).and_then(Value::as_i64).unwrap_or(0);
            effective_context(reported, learned, context_size())
        } });
        o.get_compact_reserve = Arc::new(context_reserve);
        o.on_context_limit = Arc::new(|model: &str, limit: i64| {
            let mut all = settings::get("contextLimits");
            if !all.is_object() {
                all = json!({});
            }
            all[model] = json!(limit);
            settings::set(json!({ "contextLimits": all }));
        });
        let a = Agent::new(o);
        self.wire_agent(&a, folder);
        if let Some(rt) = rt {
            rt.agents.lock().push(a.clone());
        }
        a
    }

    /// Laufzeit eines Projekts: Terminal und Beobachtung des Ordners (für Aufträge an die App wie PDF-Umwandlung)
    fn runtime(&self, folder: &str) -> Arc<Runtime> {
        let key = path_key(folder);
        if let Some(rt) = self.st.lock().runtimes.get(&key).cloned() {
            return rt;
        }
        migrate_project(folder);
        let terminal = make_terminal();
        let endpoint: Arc<dyn TerminalEndpoint> = Arc::new(terminal.clone());
        let mut so = ServersOptions::new(servers_request_fn(endpoint));
        let t2 = terminal.clone();
        so.available = Arc::new(move || t2.state() == "läuft");
        let me = self.me.clone();
        so.blocked_ports = Arc::new(move || me.upgrade().map(|a| a.terminal_ports()).unwrap_or_default());
        let rt = Arc::new(Runtime {
            folder: folder.to_string(),
            terminal,
            workspace: Arc::new(Workspace::new()),
            servers: Arc::new(Servers::new(so)),
            agents: Mutex::new(Vec::new()),
        });
        self.st.lock().runtimes.insert(key, rt.clone());
        self.wire_runtime(&rt);
        rt
    }

    /// Terminal und Ordner eines Projekts: Zustand nur für das sichtbare Projekt an die Oberfläche
    fn wire_runtime(&self, rt: &Arc<Runtime>) {
        rt.terminal.on("port", |p| settings::set(json!({ "port": p })));
        let started = Arc::new(Mutex::new(false));
        let (me, w) = (self.me.clone(), Arc::downgrade(rt));
        rt.terminal.on("state", move |snap| {
            let (Some(a), Some(rt)) = (me.upgrade(), w.upgrade()) else { return };
            let state = str_of(snap, "state");
            let active = a.is_active_rt(&rt);
            if state == "läuft" && !*started.lock() {
                *started.lock() = true;
                // Chats ohne Projekt sind schon offen (open_folder wartet dort nicht auf das Terminal)
                if active && !paths::is_free_dir(&rt.folder) {
                    a.open_pending_or_latest(&rt);
                }
            }
            if state != "läuft" && state != "startet" {
                *started.lock() = false;
            }
            if active && (state == "läuft" || state == "fehler") {
                a.st.lock().pending_chat = None;
            }
            // Ein beendetes Terminal nimmt seine Prozesse mit
            if state == "gestoppt" || state == "startet" || state == "fehler" {
                rt.servers.terminal_gone();
            }
            if active {
                a.push_state();
            }
        });
        let (me, w) = (self.me.clone(), Arc::downgrade(rt));
        rt.servers.on("changed", move |list| {
            if let (Some(a), Some(rt)) = (me.upgrade(), w.upgrade()) {
                if a.is_active_rt(&rt) {
                    a.send("servers", list.clone());
                }
            }
        });
        let (me, w) = (self.me.clone(), Arc::downgrade(rt));
        rt.workspace.events.on("auftrag", move |file| {
            if let (Some(a), Some(rt)) = (me.upgrade(), w.upgrade()) {
                let file = PathBuf::from(js_string(file));
                spawn(async move { a.handle_order(file, rt.workspace.clone()).await });
            }
        });
        let (me, w) = (self.me.clone(), Arc::downgrade(rt));
        rt.workspace.events.on("files", move |files| {
            if let (Some(a), Some(rt)) = (me.upgrade(), w.upgrade()) {
                if Arc::ptr_eq(&rt.workspace, &a.workspace()) {
                    a.send("files", files.clone());
                }
            }
        });
    }

    /// Ereignisse eines Agenten: an die Oberfläche mit Chat und Ordner, damit sie nur den sichtbaren Chat zeigt
    fn wire_agent(&self, a: &Arc<Agent>, folder: &str) {
        let (me, w, f) = (self.me.clone(), Arc::downgrade(a), folder.to_string());
        a.events.on("event", move |ev| {
            let (Some(app), Some(a)) = (me.upgrade(), w.upgrade()) else { return };
            let kind = str_of(ev, "type");
            // Frage oder Rückfrage: Fenster in der Taskleiste aufblinken lassen, wenn es im Hintergrund ist
            if (kind == "approval" || kind == "question") && !app.host.window_focused() {
                app.host.flash_frame(true);
            }
            if kind == "user" {
                a.turn_started.store(now_ms(), std::sync::atomic::Ordering::SeqCst);
            }
            app.notify(&a, &f, ev);
            let mut payload = ev.clone();
            if let Value::Object(m) = &mut payload {
                m.insert("chatId".into(), json!(a.chat_id()));
                m.insert("folder".into(), json!(f));
            }
            app.send("chat:event", payload);
            if kind == "done" {
                // im Hintergrund merken (ENTSCHEIDE E50)
                app.knowledge.after_turn(&f, &a.chat_id(), &a.chat_title(), &a.messages(), ev);
            }
        });
        let (me, w) = (self.me.clone(), Arc::downgrade(a));
        a.events.on("changed", move |_| {
            let (Some(app), Some(a)) = (me.upgrade(), w.upgrade()) else { return };
            if app.st.lock().agent.as_ref().is_some_and(|x| Arc::ptr_eq(x, &a)) {
                app.send("chat:changed", a.state());
            }
            app.push_state();
            app.send("chat:groups-changed", Value::Null);
            if app.any_running() {
                app.updater.busy_changed();
            } else {
                app.updater.idle();
            }
            if !a.running() {
                let app = app.clone();
                spawn(async move { app.release_idle(); });
            }
        });
    }

    /// Windows-Benachrichtigung (ENTSCHEIDE E52): wenn ASK fertig ist, fragt oder um Erlaubnis bittet und man gerade
    /// nicht hinschaut (Fenster im Hintergrund oder anderer Chat). Klick öffnet den Chat.
    fn notify(&self, a: &Arc<Agent>, folder: &str, ev: &Value) {
        let kind = str_of(ev, "type");
        if !["done", "question", "approval"].contains(&kind.as_str()) || !bool_of(&self.prefs(), "notify") {
            return;
        }
        let visible = self.st.lock().agent.as_ref().is_some_and(|x| Arc::ptr_eq(x, a));
        if self.host.window_focused() && visible {
            return;
        }
        if kind == "done" && (now_ms() - a.turn_started.load(std::sync::atomic::Ordering::SeqCst) < NOTIFY_MIN_MS || a.pending_questions() > 0 || a.pending_approvals() > 0) {
            return;
        }
        let title = { let t = a.chat_title(); if t.is_empty() { "Neuer Chat".to_string() } else { t } };
        let body = match kind.as_str() {
            "question" => format!("Frage: {}", str_of(ev, "frage")),
            "approval" => format!("Wartet auf dich: {}", str_of(ev, "title")),
            _ => "Fertig.".into(),
        };
        let (me, f, id, wa) = (self.me.clone(), folder.to_string(), a.chat_id(), Arc::downgrade(a));
        self.host.notify(&crate::util::take_chars(&format!("ASK · {title}"), 120), &crate::util::take_chars(&body, 240), kind == "done", Box::new(move || {
            let Some(app) = me.upgrade() else { return };
            app.host.focus_window();
            let visible = app.st.lock().agent.as_ref().zip(wa.upgrade()).is_some_and(|(x, a)| Arc::ptr_eq(x, &a));
            if !f.is_empty() && !visible {
                spawn(async move { app.open_chat(&f, &id).await; });
            }
        }));
    }

    /// Projekte ohne laufenden Auftrag, die nicht sichtbar sind, wieder schliessen (Terminal beenden)
    fn release_idle(&self) {
        let (visible, rts) = {
            let st = self.st.lock();
            (st.agent.clone(), st.runtimes.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>())
        };
        for (key, rt) in rts {
            if self.is_active_rt(&rt) {
                // Im sichtbaren Projekt höchstens einen untätigen Agenten behalten: den sichtbaren
                rt.agents.lock().retain(|a| visible.as_ref().is_some_and(|v| Arc::ptr_eq(v, a)) || a.running() || a.undoing());
                continue;
            }
            if rt.busy() {
                continue;
            }
            rt.agents.lock().clear();
            rt.workspace.close();
            self.st.lock().runtimes.remove(&key);
            self.snapshots.lock().remove(&key); // samt Dateiliste; beim nächsten Öffnen neu
            spawn(async move { stop_runtime(&rt).await });
        }
    }

    fn agent_for(rt: &Runtime, id: &str) -> Option<Arc<Agent>> {
        rt.agents().into_iter().find(|a| a.chat_id() == id)
    }

    /// Sichtbaren Chat wechseln: einen laufenden Agenten zeigen oder einen untätigen im Projekt weiterverwenden
    fn show_agent(&self, a: Arc<Agent>) {
        let same = self.st.lock().agent.as_ref().is_some_and(|x| Arc::ptr_eq(x, &a));
        if same {
            return;
        }
        self.st.lock().agent = Some(a.clone());
        self.release_idle();
        self.send("chat:changed", a.state());
        self.push_state();
    }

    fn open_in_runtime(&self, rt: &Arc<Runtime>, id: &str) -> bool {
        if id != NEW {
            if let Some(existing) = Self::agent_for(rt, id) {
                self.show_agent(existing);
                return true;
            }
        }
        // Läuft der sichtbare Agent, bleibt er im Hintergrund; der Chat bekommt einen eigenen
        let current = self.agent();
        let a = if !current.running() && !current.undoing() && rt.has(&current) {
            current
        } else {
            rt.agents().into_iter().find(|x| !x.running() && !x.undoing()).unwrap_or_else(|| self.make_agent(&rt.folder, Some(rt)))
        };
        let ok = if id == NEW { a.new_chat() } else { a.open(id) };
        if ok {
            self.show_agent(a);
        }
        ok
    }

    /// Den vorgemerkten Chat dieses Ordners zeigen, sonst einen, der dort gerade arbeitet, sonst den zuletzt benutzten.
    /// Eine Vormerkung für einen anderen Ordner verfällt (die Person hat inzwischen etwas anderes gewählt), und lässt
    /// sich der vorgemerkte Chat nicht öffnen (gelöscht), gilt die übrige Reihenfolge: Sonst bliebe der Agent des
    /// vorherigen Projekts sichtbar, während Dateien und Terminal schon zum neuen gehören.
    fn open_pending_or_latest(&self, rt: &Arc<Runtime>) {
        let want = match self.st.lock().pending_chat.take() {
            Some((key, id)) if key == path_key(&rt.folder) => Some(id),
            _ => None,
        };
        if want.is_some_and(|id| self.open_in_runtime(rt, &id)) {
            return;
        }
        match rt.agents().into_iter().find(|a| a.running()) {
            Some(r) => self.show_agent(r),
            None => self.open_latest_in(rt, None),
        }
    }

    /// Letzten Chat eines Projekts zeigen (beim Öffnen des Projekts)
    fn open_latest_in(&self, rt: &Arc<Runtime>, wanted: Option<&str>) {
        let a = rt.agents().into_iter().find(|x| !x.running() && !x.undoing()).unwrap_or_else(|| self.make_agent(&rt.folder, Some(rt)));
        a.open_latest(wanted);
        let same = self.st.lock().agent.as_ref().is_some_and(|x| Arc::ptr_eq(x, &a));
        self.show_agent(a.clone());
        if same {
            self.send("chat:changed", a.state());
            self.push_state();
        }
    }

    /// Chat (id oder NEW) in einem Ordner öffnen; bei anderem Ordner zuerst wechseln.
    async fn open_chat(&self, folder: &str, id: &str) -> Value {
        // Chats ohne Projekt (E78) hängen nicht am Terminal: Es liest dort nur Anhänge und darf auch fehlen
        if same_path(folder, &js_string(&settings::get("folder"))) && (self.terminal().state() == "läuft" || paths::is_free_dir(folder)) {
            let rt = self.runtime(folder);
            return if self.open_in_runtime(&rt, id) { json!({ "ok": true }) } else { fail("Diesen Chat gibt es nicht mehr.") };
        }
        self.st.lock().pending_chat = Some((path_key(folder), id.to_string()));
        let res = self.open_folder(folder, "").await;
        if !bool_of(&res, "ok") {
            self.st.lock().pending_chat = None;
        }
        res
    }

    /// Projekt sichtbar machen. Laufende Aufträge in anderen Projekten arbeiten im Hintergrund weiter, jeder mit
    /// seinem eigenen Terminal und Ordner (ENTSCHEIDE E52).
    pub async fn open_folder(&self, folder: &str, name: &str) -> Value {
        // Der Ordner der Chats ohne Projekt liegt in den Daten der App (E78): keine Prüfung wie bei Projekten
        // (AppData ist dort gesperrt), und er kommt nicht in die Projektliste
        let free = paths::is_free_dir(folder);
        let path = if free {
            Self::free_folder()
        } else {
            let result = crate::folders::check_folder(folder, &crate::folders::FolderContext::system());
            let Some(path) = result.path.clone().filter(|_| result.ok) else { return result.to_json() };
            path
        };
        let rt = self.runtime(&path);
        if rt.workspace.root() != path {
            if let Err(err) = rt.workspace.open(&path) {
                if !self.is_active_rt(&rt) && rt.agents.lock().is_empty() {
                    self.st.lock().runtimes.remove(&path_key(&path));
                }
                return fail(format!("Der Ordner kann nicht vorbereitet werden: {err}"));
            }
        }
        if free {
            settings::set(json!({ "folder": path }));
        } else {
            settings::add_recent(&path, name);
        }
        {
            let mut st = self.st.lock();
            st.terminal = rt.terminal.clone();
            st.workspace = rt.workspace.clone();
        }
        self.send("files", rt.workspace.files());
        if rt.terminal.state() == "läuft" || free {
            // schon offen (z. B. weil dort ein Auftrag im Hintergrund läuft): gewünschten oder letzten Chat zeigen.
            // Chats ohne Projekt warten nicht auf das Terminal; es startet nebenher (für read_file bei Anhängen).
            if free && !["läuft", "startet"].contains(&rt.terminal.state().as_str()) {
                self.start_terminal();
            }
            self.open_pending_or_latest(&rt);
        } else {
            if rt.agents.lock().is_empty() {
                let a = self.make_agent(&rt.folder, Some(&rt));
                self.st.lock().agent = Some(a);
            }
            self.start_terminal();
        }
        self.release_idle();
        self.push_state();
        // Alte Sicherungen aufräumen, mit derselben Instanz, die auch sichert, und nicht beim Öffnen bremsen
        let me = self.me.clone();
        let p = path.clone();
        spawn(async move {
            tokio::time::sleep(Duration::from_secs(5)).await;
            if let Some(a) = me.upgrade() {
                // Nicht während ein Auftrag dort gerade sichert: Aufräumen könnte ihm eine Fassung wegnehmen
                if same_path(&js_string(&settings::get("folder")), &p) && !crate::snapshot::busy(Path::new(&p)) {
                    if let Some(s) = a.snapshots_for(&p) {
                        let _ = s.prune_now();
                    }
                }
            }
        });
        json!({ "ok": true, "path": path })
    }

    /// Ordner der Chats ohne Projekt (ENTSCHEIDE E78), bei Bedarf angelegt
    fn free_folder() -> String {
        let dir = paths::free_dir();
        let _ = std::fs::create_dir_all(&dir);
        dir.to_string_lossy().to_string()
    }

    /// Chats ohne Projekt zeigen: wie ein Projekt mit eigenem Ordner, aber der Agent bekommt dort nur Werkzeuge,
    /// die nichts ändern (agent.rs, `FREE_ALLOWED`).
    pub async fn open_free(&self) -> Value {
        self.open_folder(&Self::free_folder(), "").await
    }

    fn project_base() -> PathBuf {
        paths::home().join("ASK-Projekte")
    }

    /// Neues Projekt: Ordner unter ASK-Projekte mit dem Namen des Projekts
    async fn new_project(&self, name: &str) -> Value {
        let base = Self::project_base();
        let title = if name.trim().is_empty() { format!("Projekt {}", chrono::Local::now().format("%Y-%m-%d")) } else { name.trim().to_string() };
        let folder_name = { let n = safe_folder_name(&title); if n.is_empty() { "Projekt".into() } else { n } };
        let mut target = base.join(&folder_name);
        let mut i = 2;
        while target.exists() {
            target = base.join(format!("{folder_name} ({i})"));
            i += 1;
        }
        if let Err(err) = std::fs::create_dir_all(&target) {
            return fail(format!("Der Ordner kann nicht angelegt werden: {err}"));
        }
        // Ordner für Kursunterlagen (E84); fehlt er, geht das Projekt trotzdem
        let _ = std::fs::create_dir_all(target.join(crate::sources::FOLDER));
        self.open_folder(&target.to_string_lossy(), &title).await
    }

    fn start_terminal(&self) {
        let s = settings::load();
        let (folder, ask_url) = (str_of(&s, "folder"), str_of(&s, "askUrl"));
        if folder.is_empty() || ask_url.is_empty() {
            return;
        }
        let origin = url::Url::parse(&ask_url).map(|u| u.origin().ascii_serialization()).ok();
        let port = s.get("port").and_then(Value::as_u64).filter(|p| *p > 0 && *p < 65536).map(|p| p as u16);
        let fut = self.terminal().start(StartOptions {
            folder: folder.clone(),
            preferred_port: port,
            origin,
            system_prompt: Some(read_instruction("systemprompt.md", &folder, "allgemein")),
            execute_description: Some(read_instruction("befehle.md", &folder, "allgemein")),
        });
        spawn(fut);
    }

    /// Auftrag des Modells aus .ask/auftraege/<id>.json: Datei mit Office in PDF umwandeln.
    async fn handle_order(&self, file: PathBuf, ws: Arc<Workspace>) {
        let root = ws.root();
        if root.is_empty() || !self.orders.lock().insert(file.clone()) {
            return;
        }
        let base = file.to_string_lossy().trim_end_matches(".json").to_string();
        let res: Value = match std::fs::read_to_string(&file).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Value>(&t).map_err(|e| e.to_string())) {
            Err(err) => json!({ "fehler": format!("Auftrag nicht lesbar: {err}") }),
            Ok(order) => {
                let source = if str_of(&order, "typ") == "pdf" { ws.resolve(&str_of(&order, "quelle")) } else { None };
                match source {
                    None => json!({ "fehler": "Datei nicht gefunden oder ausserhalb des Arbeitsordners." }),
                    Some(src) if std::fs::metadata(&src).map(|m| m.len()).unwrap_or(0) > 50 * 1024 * 1024 => json!({ "fehler": "Die Datei ist grösser als 50 MB." }),
                    Some(src) => crate::office::convert_to_pdf(Path::new(&root), &src, false).await.to_json(),
                }
            }
        };
        if ws.root() == root {
            if bool_of(&res, "ok") {
                let target = PathBuf::from(str_of(&res, "target"));
                let rel = target.strip_prefix(&root).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| str_of(&res, "target"));
                let _ = std::fs::write(format!("{base}.ok"), rel);
            } else {
                let _ = std::fs::write(format!("{base}.fehler"), str_of(&res, "fehler"));
            }
            let _ = std::fs::remove_file(&file);
        }
        self.orders.lock().remove(&file);
    }

    /// Ordner aus der Liste nehmen. Beim aktuellen Ordner auch Terminal und Beobachtung beenden.
    fn forget_folder(&self, f: &str) -> bool {
        if paths::is_free_dir(f) {
            return false; // kein Projekt: steht nicht in der Liste
        }
        let current = same_path(f, &js_string(&settings::get("folder")));
        let rt = self.st.lock().runtimes.get(&path_key(f)).cloned();
        if rt.as_ref().is_some_and(|rt| rt.agents().iter().any(|a| a.running())) {
            self.send("folder:error", json!(BUSY));
            return false;
        }
        settings::remove_recent(f);
        self.snapshots.lock().remove(&path_key(f));
        // Marke «Low» erst entfernen, wenn kein Terminal des Ordners mehr läuft
        let mut stopped: Option<BoxFuture<'static, ()>> = if rt.is_none() { Some(Box::pin(async {})) } else { None };
        if current {
            let (terminal, workspace) = { let st = self.st.lock(); (st.terminal.clone(), st.workspace.clone()) };
            workspace.close();
            stopped = Some(match &rt {
                Some(rt) => { let rt = rt.clone(); Box::pin(async move { stop_runtime(&rt).await }) }
                None => Box::pin(terminal.stop()),
            });
            {
                let mut st = self.st.lock();
                st.terminal = make_terminal();
                st.workspace = Arc::new(Workspace::new());
                st.agent = None;
                if rt.is_some() {
                    st.runtimes.remove(&path_key(f));
                }
            }
        }
        if let Some(stopped) = stopped {
            let folder = f.to_string();
            spawn(async move {
                stopped.await;
                let others: Vec<String> = settings::load()["projects"].as_array().map(|a| a.iter().map(|p| str_of(p, "folder")).collect()).unwrap_or_default();
                let opts = crate::lowlabel::RestoreOptions { other_folders: others, ..Default::default() };
                if let Err(e) = crate::lowlabel::restore_label(&folder, &opts).await {
                    eprintln!("Marke «Low» nicht entfernt: {e}");
                }
            });
        }
        self.push_state();
        self.send("chat:changed", self.agent().state());
        if current {
            // Das sichtbare Projekt ist weg: weiter in den Chats ohne Projekt, statt ohne Chat dazustehen
            let me = self.me.clone();
            spawn(async move {
                if let Some(a) = me.upgrade() {
                    if js_string(&settings::get("folder")).is_empty() && !str_of(&settings::load(), "askUrl").is_empty() {
                        a.open_free().await;
                    }
                }
            });
        }
        true
    }

    /// Linke Leiste: Projekte (ein Projekt = ein Arbeitsordner) mit ihren Chats. Ein Ordner, den es nicht mehr
    /// gibt, bleibt sichtbar (missing), damit man ihn entfernen kann.
    fn chat_groups(&self) -> Value {
        let s = settings::load();
        let folder = str_of(&s, "folder");
        let agent = self.agent();
        // Zuerst die Chats ohne Projekt (E78): eigene Gruppe, kein Projekt (free)
        let free = paths::free_dir().to_string_lossy().to_string();
        let mut groups = vec![json!({ "folder": free, "name": "Chats", "color": "", "free": true, "current": same_path(&free, &folder), "missing": false,
            "chats": agent.list(&free) })];
        groups.extend(s["projects"].as_array().cloned().unwrap_or_default().iter().filter(|p| !paths::is_free_dir(&str_of(p, "folder"))).map(|p| {
            let pf = str_of(p, "folder");
            let exists = Path::new(&pf).exists();
            json!({ "folder": pf, "name": p["name"], "color": p["color"], "current": same_path(&pf, &folder), "missing": !exists,
                "chats": if exists { agent.list(&pf) } else { json!([]) } })
        }));
        Value::Array(groups)
    }

    /// Kursunterlagen wählen und in den Ordner «Quellen» des Projekts kopieren (ENTSCHEIDE E84)
    async fn add_sources(&self, folder: &str) {
        let filters: crate::host::Filters = vec![
            ("Unterlagen".into(), ["pdf", "docx", "pptx", "xlsx", "odt", "odp", "ods", "rtf", "txt", "md", "csv", "epub", "png", "jpg", "jpeg"].iter().map(|s| s.to_string()).collect()),
            ("Alle Dateien".into(), vec!["*".into()]),
        ];
        let picked = self.host.pick_open_files("Unterlagen als Quellen hinzufügen", filters).await;
        if picked.is_empty() {
            return;
        }
        let skipped = picked.len().saturating_sub(50);
        let more = if skipped > 0 { format!(" {skipped} weitere nicht: höchstens 50 auf einmal.") } else { String::new() };
        // Kopieren kann bei grossen Dateien dauern: nicht im Thread der Oberfläche
        let (dir, list) = (folder.to_string(), picked.clone());
        let res = tokio::task::spawn_blocking(move || crate::sources::add(Path::new(&dir), &list)).await.unwrap_or_else(|e| Err(anyhow::anyhow!("{e}")));
        let text = match res {
            Ok(done) if done.is_empty() => "Nichts kopiert: Die gewählten Dateien liessen sich nicht lesen.".to_string(),
            Ok(done) if done.len() == 1 => format!("{} liegt jetzt im Ordner {}. ASK benutzt die Unterlage in diesem Projekt.", done[0].rsplit('/').next().unwrap_or(""), crate::sources::FOLDER),
            Ok(done) => format!("{} Unterlagen liegen jetzt im Ordner {}. ASK benutzt sie in diesem Projekt.", done.len(), crate::sources::FOLDER),
            Err(err) => format!("Nicht kopiert: {err}"),
        };
        self.send("app:notice", json!(format!("{text}{more}")));
        self.push_state();
    }

    fn open_notes(&self, folder: &str) {
        let file = Path::new(folder).join(".ask").join("projekt.md");
        if !file.exists() {
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&file, "# Projektnotizen\n\nWas das Modell in jedem Chat in diesem Ordner wissen soll (Aufbau, wichtige Dateien, Vorlagen, Wünsche).\n");
        }
        let host = self.host.clone();
        let p = file.to_string_lossy().to_string();
        spawn(async move { host.open_path(&p).await; });
    }

    /// Menü eines Projekts in der Seitenleiste (Rechtsklick oder «…»)
    async fn project_menu(&self, folder: &str) {
        let Some(project) = settings::find_project(folder) else { return };
        let exists = Path::new(folder).exists();
        let f = folder.to_string();
        let mut items = Vec::new();
        if exists {
            let f2 = f.clone();
            items.push(MenuItem::new("Neuer Chat", self.action(move |a, _| { let f = f2.clone(); Box::pin(async move {
                let res = a.open_chat(&f, NEW).await;
                if res.get("ok") == Some(&json!(false)) && res.get("reason").is_some() {
                    a.send("folder:error", res["reason"].clone());
                }
            }) })));
        }
        let f2 = f.clone();
        items.push(MenuItem::new("Umbenennen …", self.action(move |a, _| { let f = f2.clone(); Box::pin(async move { a.send("project:rename", json!(f)); }) })));
        let colors = settings::PROJECT_COLORS.iter().map(|c| {
            let (f3, c2) = (f.clone(), c.to_string());
            MenuItem { kind: "radio", checked: str_of(&project, "color") == *c, swatch: Some(c.to_string()),
                ..MenuItem::new(color_name(c), self.action(move |a, _| { let (f, c) = (f3.clone(), c2.clone()); Box::pin(async move {
                    settings::update_project(&f, &json!({ "color": c }));
                    a.send("chat:groups-changed", Value::Null);
                }) })) }
        }).collect();
        items.push(MenuItem::sub("Farbe", colors));
        items.push(MenuItem::separator());
        if exists {
            let f2 = f.clone();
            items.push(MenuItem::new("Im Explorer öffnen", self.action(move |a, _| { let f = f2.clone(); Box::pin(async move { a.host.open_path(&f).await; }) })));
            let f2 = f.clone();
            items.push(MenuItem::new("Projektnotizen bearbeiten", self.action(move |a, _| { let f = f2.clone(); Box::pin(async move { a.open_notes(&f); }) })));
            let f2 = f.clone();
            items.push(MenuItem::new("Quellen hinzufügen …", self.action(move |a, _| { let f = f2.clone(); Box::pin(async move { a.add_sources(&f).await; }) })));
            if same_path(folder, &js_string(&settings::get("folder"))) {
                items.push(MenuItem::new("GitHub …", self.action(|a, _| Box::pin(async move { a.send("menu:github", Value::Null); }))));
            }
            items.push(MenuItem::separator());
        }
        let (f2, name) = (f.clone(), str_of(&project, "name"));
        items.push(MenuItem::new("Aus der Liste entfernen", self.action(move |a, _| { let (f, name) = (f2.clone(), name.clone()); Box::pin(async move {
            let detail = format!("{name}\n{f}\n\nDer Ordner und seine Dateien bleiben erhalten. Du kannst ihn später wieder öffnen.");
            if a.confirm("Projekt aus der Liste entfernen?", &detail, ConfirmOptions::default()).await.ok {
                a.forget_folder(&f);
                a.send("chat:groups-changed", Value::Null);
            }
        }) })));
        self.popup_menu(items).await;
    }

    /// Ordner-Menü. Ein Ordner, den es nicht mehr gibt, lässt sich nicht öffnen (das Terminal startet nicht):
    /// Er ist markiert, und ein Klick bietet an, ihn aus der Liste zu nehmen.
    fn folder_menu_items(&self) -> Vec<MenuItem> {
        let s = settings::load();
        let folder = str_of(&s, "folder");
        let recent: Vec<String> = s["projects"].as_array().cloned().unwrap_or_default().iter().map(|p| str_of(p, "folder")).collect();
        let free = paths::is_free_dir(&folder);
        let mut items: Vec<MenuItem> = vec![
            MenuItem { kind: "radio", checked: free, ..MenuItem::new("Chats ohne Projekt", self.action(|a, _| Box::pin(async move { a.open_free().await; }))) },
            MenuItem::separator(),
        ];
        items.extend(recent.iter().map(|f| {
            let f2 = f.clone();
            if Path::new(f).exists() {
                MenuItem { kind: "radio", checked: same_path(f, &folder), ..MenuItem::new(f.clone(), self.action(move |a, _| { let f = f2.clone(); Box::pin(async move {
                    let res = a.open_folder(&f, "").await;
                    if !bool_of(&res, "ok") {
                        a.send("folder:error", res.get("reason").cloned().unwrap_or(Value::Null));
                    }
                }) })) }
            } else {
                MenuItem::new(format!("{f} (nicht gefunden)"), self.action(move |a, _| { let f = f2.clone(); Box::pin(async move {
                    if a.confirm("Ordner nicht gefunden", &format!("{f}\n\nDiesen Eintrag aus der Liste entfernen?"), ConfirmOptions::default()).await.ok {
                        a.forget_folder(&f);
                    }
                }) }))
            }
        }));
        if !recent.is_empty() {
            items.push(MenuItem::separator());
        }
        items.push(MenuItem::new("Anderen Ordner wählen …", self.action(|a, _| Box::pin(async move { a.send("menu:choose-folder", Value::Null); }))));
        items.push(MenuItem::new("Neues Projekt …", self.action(|a, _| Box::pin(async move { a.send("menu:new-folder", Value::Null); }))));
        items.push(MenuItem::new("GitHub …", self.action(|a, _| Box::pin(async move { a.send("menu:github", Value::Null); }))));
        let project = !folder.is_empty() && !free;
        if project || !recent.is_empty() {
            items.push(MenuItem::separator());
        }
        if project {
            let f2 = folder.clone();
            items.push(MenuItem::new("Arbeitsordner im Explorer öffnen", self.action(move |a, _| { let f = f2.clone(); Box::pin(async move { a.host.open_path(&f).await; }) })));
            let f2 = folder.clone();
            items.push(MenuItem::new("Projektnotizen bearbeiten", self.action(move |a, _| { let f = f2.clone(); Box::pin(async move { a.open_notes(&f); }) })));
        }
        if !recent.is_empty() {
            let sub = recent.iter().map(|f| { let f2 = f.clone(); MenuItem::new(f.clone(), self.action(move |a, _| { let f = f2.clone(); Box::pin(async move { a.forget_folder(&f); }) })) }).collect();
            items.push(MenuItem::sub("Aus der Liste entfernen", sub));
        }
        items.push(MenuItem::separator());
        items.push(MenuItem::new(format!("Nach Updates suchen (ASK {})", self.host.app_version()), self.action(|a, _| Box::pin(async move { a.updater.check(true).await; }))));
        items.push(MenuItem { kind: "checkbox", checked: truthy(&settings::get("betaUpdates")),
            ..MenuItem::new("Beta-Versionen erhalten", self.action(|a, checked| Box::pin(async move { a.set_beta(checked, false).await; }))) });
        items
    }

    /// tell: mit /beta ausgelöst; das Menü zeigt den Zustand selbst als Häkchen.
    async fn set_beta(&self, on: bool, tell: bool) {
        settings::set(json!({ "betaUpdates": on }));
        self.push_state();
        if tell && !self.test_mode {
            let detail = if on { "ASK bietet dir auch Vorabversionen zum Testen an. Sie können Fehler haben." }
                else { "ASK bietet dir nur noch normale Versionen an. Eine installierte Beta bleibt, bis eine neuere normale Version kommt." };
            self.show_box(json!({ "type": "info", "buttons": ["OK"], "message": if on { "Beta-Versionen an" } else { "Beta-Versionen aus" }, "detail": detail })).await;
        }
        if on {
            // eine vorhandene Beta gleich anbieten
            let u = self.updater.clone();
            spawn(async move { u.check(false).await; });
        }
    }

    // ---- GitHub-Bereich --------------------------------------------------------------------------------------

    async fn gh_read<F, Fut>(&self, f: F) -> Value
    where
        F: FnOnce(PathBuf) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Value>>,
    {
        let folder = js_string(&settings::get("folder"));
        if folder.is_empty() || paths::is_free_dir(&folder) {
            return json!({ "ok": false, "message": "Kein Projekt geöffnet." });
        }
        match f(PathBuf::from(folder)).await {
            Ok(v) => merge_ok(v),
            Err(err) => json!({ "ok": false, "message": err.to_string() }),
        }
    }

    /// Aktionen aus dem Bereich «GitHub» für das sichtbare Projekt. Während das Modell dort arbeitet, nicht: Es
    /// könnte gleichzeitig abgleichen, und beide schrieben dieselbe Verbindung. Eine Aktion zur Zeit pro Ordner.
    async fn gh_action<F, Fut>(&self, f: F) -> Value
    where
        F: FnOnce(PathBuf) -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<Value>>,
    {
        let folder = js_string(&settings::get("folder"));
        if folder.is_empty() || paths::is_free_dir(&folder) {
            return json!({ "ok": false, "message": "Kein Projekt geöffnet." });
        }
        if self.st.lock().runtimes.get(&path_key(&folder)).is_some_and(|rt| rt.busy()) {
            return json!({ "ok": false, "message": "ASK arbeitet gerade in diesem Projekt. Warte, bis der Auftrag fertig ist, oder stoppe ihn (Esc)." });
        }
        let root = PathBuf::from(&folder);
        match self.github.exclusive(&root, || f(root.clone())).await {
            Ok(v) => merge_ok(v),
            Err(err) => {
                let pending = err.downcast_ref::<crate::github::PendingChanges>().map(|p| json!(p.pending));
                let mut v = json!({ "ok": false, "message": err.to_string() });
                if let Some(p) = pending {
                    v["pending"] = p;
                }
                v
            }
        }
    }

    /// Hochladen und Anlegen laufen über die Werkzeuge selbst, damit Rückfrage und «nicht mehr fragen» genau gleich gelten.
    async fn gh_tool(&self, name: &str, args: Value) -> Value {
        let ctx = ToolCtx { github: Some(self.github.clone()), github_trust: Some(self.github_trust.clone()), confirm: Some(self.confirm_fn()),
            folder: js_string(&settings::get("folder")), panel: true, ..Default::default() };
        let name = name.to_string();
        self.gh_action(|_root| async move { Ok(run_tool(None, &name, &args, &ctx).await.to_value()) }).await
    }

    // ---- Dateien -------------------------------------------------------------------------------------------

    /// Vorschau in der Seitenleiste (Protokoll askfile): nur Dateien aus dem Arbeitsordner
    pub fn resolve_file(&self, rel: &str) -> Option<PathBuf> {
        self.workspace().resolve(rel)
    }

    async fn file_preview(&self, rel: &str) -> Value {
        let ws = self.workspace();
        let Some(full) = ws.resolve(rel) else { return json!({ "kind": "missing" }) };
        let ext = ext_of(&full);
        let url = format!("{}{}", self.file_base, rel.split(['/', '\\']).map(|p| percent_encoding::utf8_percent_encode(p, percent_encoding::NON_ALPHANUMERIC).to_string()).collect::<Vec<_>>().join("/"));
        let meta = std::fs::metadata(&full).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta.and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
        let name = full.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if IMAGE_EXT.contains(&ext.as_str()) {
            return json!({ "kind": "image", "url": format!("{url}?v={mtime}") }); // neue Fassung, nicht die zwischengespeicherte
        }
        if ext == ".pdf" {
            return json!({ "kind": "pdf", "url": url });
        }
        // Mit installiertem Office zeigt die Vorschau ein PDF der Datei, ohne Klick auf «Als PDF anzeigen»
        if OFFICE_EXT.contains(&ext.as_str()) {
            return json!({ "kind": "office", "name": name, "auto": crate::office::office_installed(&ext).await });
        }
        if TEXT_EXT.contains(&ext.as_str()) || size < 200_000 {
            // nur den Anfang lesen, auch bei sehr grossen Dateien
            if let Ok(buf) = read_start(&full, 200_000) {
                if !buf.contains(&0) {
                    return json!({ "kind": "text", "text": String::from_utf8_lossy(&buf), "truncated": size > 200_000 });
                }
            }
        }
        json!({ "kind": "other", "name": name })
    }

    /// Öffnen mit dem zugehörigen Programm läuft mit den normalen Rechten der Person, nicht im geschützten Terminal.
    /// Programme und Skripte (auch solche, die das Modell angelegt hat) darum nie ohne Rückfrage.
    async fn file_open(&self, rel: &str) -> Value {
        let Some(full) = self.workspace().resolve(rel) else { return json!("nicht gefunden") };
        let p = full.to_string_lossy().to_string();
        let ext = ext_of(&full);
        if !SAFE_OPEN_EXT.contains(&ext.as_str()) && !SAFE_OPEN_EXT2.contains(&ext.as_str()) {
            let auto = self.test_mode && std::env::var("ASK_TEST_AUTOCONFIRM").map(|v| v == "1").unwrap_or(false);
            let response = if auto { 0 } else {
                let name = full.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                self.show_box(json!({
                    "type": "warning", "buttons": ["Im Explorer zeigen", "Trotzdem öffnen", "Abbrechen"], "defaultId": 0, "cancelId": 2,
                    "message": format!("«{name}» öffnen?"),
                    "detail": "Diese Datei kann ein Programm oder Skript sein. Beim Öffnen läuft sie mit deinen Rechten auf dem ganzen Laptop, nicht nur im Arbeitsordner. Öffne sie nur, wenn du weisst, was sie tut. Hat das Modell sie angelegt, lass sie lieber vom Modell im Chat ausführen.",
                })).await.get("response").and_then(Value::as_i64).unwrap_or(2)
            };
            if response == 0 {
                self.host.show_item_in_folder(&p);
                return json!("");
            }
            if response != 1 {
                return json!("");
            }
        }
        json!(self.host.open_path(&p).await)
    }

    // ---- Aufrufe der Oberfläche ----------------------------------------------------------------------------

    pub(crate) fn send_ui(&self, channel: &str, payload: Value) {
        self.send(channel, payload);
    }

    pub(crate) async fn host_pick_folder(&self, title: &str) -> Option<String> {
        self.host.pick_folder(title, "").await.filter(|p| !p.is_empty())
    }

    pub(crate) async fn host_pick_save(&self, title: &str, name: &str, filters: crate::host::Filters) -> Option<String> {
        self.host.pick_save_file(title, name, filters).await.filter(|p| !p.is_empty())
    }

    pub(crate) fn host_copy(&self, text: &str) {
        self.host.clipboard_write(text);
    }

    pub(crate) async fn host_open(&self, path: &str) {
        let _ = self.host.open_path(path).await;
    }

    /// Läuft Code eines eingeschalteten Mods in der Oberfläche? (E72)
    pub fn mods_code(&self) -> bool {
        self.ui_has_mod_code() || self.knowledge.mods.code_active()
    }

    /// Läuft in der geladenen Oberfläche Code eines Mods? Bleibt wahr, bis sie neu lädt, auch wenn der Mod
    /// inzwischen aus ist (der Code läuft bis dahin weiter). Die Hülle lädt dann neu.
    pub fn ui_has_mod_code(&self) -> bool {
        self.ui_mod_code.load(std::sync::atomic::Ordering::SeqCst)
    }

    pub(crate) fn mark_ui_mod_code(&self) {
        self.ui_mod_code.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    /// Heikler Schritt, ausgelöst in der Oberfläche: Läuft Code eines Mods, bestätigt die Person ihn in einem
    /// Dialog von Windows, den der Code nicht bedienen kann. Ohne Mods: gleich ja (der Klick kam von ihr).
    pub async fn secure(&self, message: &str, detail: &str) -> bool {
        if !self.mods_code() {
            return true;
        }
        if self.test_mode && std::env::var("ASK_TEST_AUTOCONFIRM").map(|v| v == "1").unwrap_or(false) {
            return true;
        }
        let opts = json!({ "type": "warning", "title": "ASK – Sicherheitsabfrage", "buttons": ["Ja, das war ich", "Nein"], "defaultId": 1, "cancelId": 1,
            "message": message, "detail": format!("{}{}{MOD_NOTE}", detail, if detail.is_empty() { "" } else { "\n\n" }), "noLink": true });
        self.host.flash_frame(false);
        self.host.native_box(opts).await.get("response").and_then(Value::as_i64) == Some(0)
    }

    /// Welche Aufrufe der Oberfläche bei laufendem Mod-Code eine Bestätigung von Windows brauchen, mit Text
    fn guard_text(&self, channel: &str, a: &[Value]) -> Option<(String, String)> {
        let on = |i: usize| truthy(&arg(a, i));
        Some(match channel {
            "chat:approve" => {
                let (id, ans) = (sarg(a, 0), sarg(a, 1));
                if ans != "yes" && ans != "always" {
                    return None;
                }
                let (title, detail) = std::iter::once(self.agent()).chain(self.all_agents()).find_map(|x| x.approval_text(&id)).unwrap_or_default();
                (format!("Erlauben: {}", if title.is_empty() { "Schritt von ASK".into() } else { title }), format!("{detail}{}", if ans == "always" { "\n\n(Für den Rest dieses Chats ohne weitere Rückfrage.)" } else { "" }))
            }
            "settings:set" => {
                let v = arg(a, 0);
                if v.get("mode").and_then(Value::as_str) == Some("auto") {
                    ("Modus «Auto» einschalten?".into(), "ASK arbeitet dann ohne Rückfragen, ein Modell prüft die Befehle.".into())
                } else if v.get("approvals").is_some() {
                    ("Stufe der Rückfragen ändern?".into(), String::new())
                } else if v.get("reviewModel").is_some() {
                    ("Modell für die Prüfung ändern?".into(), js_string(&v["reviewModel"]))
                } else {
                    return None;
                }
            }
            "ask:set-url" => ("ASK-Adresse ändern?".into(), sarg(a, 0)),
            "ask:use-token" => ("Mit einem eingefügten Token bei ASK anmelden?".into(), String::new()),
            "github:connect" => ("GitHub-Token speichern?".into(), String::new()),
            "servers:start" => ("Server starten?".into(), sarg(a, 0)),
            "chat:move" => ("Chat verschieben?".into(), if paths::is_free_dir(&sarg(a, 2)) { "Zu den Chats ohne Projekt".into() } else { sarg(a, 2) }),
            "skills:save" => ("Skill speichern?".into(), str_of(&arg(a, 0), "name")),
            "skills:enable" if on(1) => ("Mitgelieferten Skill einschalten?".into(), sarg(a, 0)),
            "memory:import-save" => ("Einträge ins Gedächtnis importieren?".into(), format!("{} Einträge", arg(a, 0).as_array().map(Vec::len).unwrap_or(0))),
            "memory:add" | "memory:update" => ("Eintrag im Gedächtnis speichern?".into(), crate::util::take_chars(&str_of(&arg(a, if channel == "memory:add" { 0 } else { 1 }), "text"), 300)),
            "mods:enable" if on(1) => ("Mod einschalten?".into(), sarg(a, 0)),
            "catalog:install" => (format!("Aus dem Katalog hinzufügen: {}?", sarg(a, 2)), sarg(a, 0)),
            "catalog:link" => ("Von Link hinzufügen?".into(), sarg(a, 0)),
            "catalog:sources" if a.first().is_some_and(|v| v.is_array()) => ("Kataloge ändern?".into(), String::new()),
            _ => return None,
        })
    }

    pub async fn handle(self: &Arc<Self>, channel: &str, args: Vec<Value>) -> Result<Value, String> {
        let a = &args;
        if self.mods_code() {
            if let Some((message, detail)) = self.guard_text(channel, a) {
                if !self.secure(&message, &detail).await {
                    return Ok(json!({ "ok": false, "reason": "Nicht bestätigt." }));
                }
            }
        }
        if let Some(r) = self.handle_extensions(channel, a).await {
            return r;
        }
        if let Some(r) = self.handle_docs(channel, a).await {
            return r;
        }
        let agent = self.agent();
        let v: Value = match channel {
            "update:action" => { self.updater.action().await; Value::Null }
            "update:later" => { self.updater.postpone(); Value::Null }
            "update:beta" => { self.set_beta(truthy(&arg(a, 0)), truthy(&arg(a, 1))).await; Value::Null }
            "file:pdf" => {
                let ws = self.workspace();
                match ws.resolve(&sarg(a, 0)) {
                    None => json!({ "fehler": "Datei nicht gefunden." }),
                    Some(source) => {
                        let root = ws.root();
                        let res = crate::office::convert_to_pdf(Path::new(&root), &source, truthy(&arg(a, 1))).await;
                        if res.ok {
                            let t = res.target.clone().unwrap_or_default();
                            json!({ "ok": true, "path": t.strip_prefix(&root).map(|p| p.to_string_lossy().to_string()).unwrap_or_default() })
                        } else {
                            res.to_json()
                        }
                    }
                }
            }
            "state:get" => { self.ui.lock().ready = true; self.state() }
            "ui:answer" => {
                self.host.flash_frame(false);
                let id = arg(a, 0).as_u64();
                let value = arg(a, 1);
                // Dialog: { response, checkboxChecked }; Menü: Schlüssel des Eintrags oder null
                match &value {
                    Value::Object(_) if value.get("response").and_then(Value::as_i64).is_some() => self.ui_answer(id, Some(json!({
                        "response": value["response"], "checkboxChecked": bool_of(&value, "checkboxChecked") }))),
                    Value::Object(_) => self.ui_answer(id, None),
                    Value::String(s) => self.ui_answer(id, Some(json!(s))),
                    _ => self.ui_answer(id, Some(Value::Null)),
                }
                if id.is_none() { self.ui_answer(None, None); }
                Value::Null
            }
            "folder:choose" => {
                let suggested = Self::project_base();
                let default = if suggested.exists() { suggested } else { paths::home() };
                match self.host.pick_folder("Arbeitsordner wählen", &default.to_string_lossy()).await {
                    None => json!({ "ok": false, "canceled": true }),
                    Some(p) => self.open_folder(&p, "").await,
                }
            }
            "folder:new" => self.new_project(&sarg(a, 0)).await,
            "folder:open" => self.open_folder(&sarg(a, 0), "").await,
            "folder:forget" => json!(self.forget_folder(&sarg(a, 0))),
            "project:menu" => { let me = self.clone(); let f = sarg(a, 0); spawn(async move { me.project_menu(&f).await; }); Value::Null }
            "project:update" => { let ok = settings::update_project(&sarg(a, 0), &arg(a, 1)); self.push_state(); json!(ok) }
            "settings:set" => {
                let v = arg(a, 0);
                let mut patch = Map::new();
                if let Some(m) = v.get("approvals").and_then(Value::as_str).filter(|m| crate::approvals::MODES.contains(m)) { patch.insert("approvals".into(), json!(m)); }
                if let Some(m) = v.get("mode").and_then(Value::as_str).filter(|m| CHAT_MODES.contains(m)) { patch.insert("mode".into(), json!(m)); }
                if let Some(m) = v.get("reviewModel").and_then(Value::as_str) { patch.insert("reviewModel".into(), json!(crate::util::take_chars(m, 200))); }
                if let Some(m) = v.get("helperModel").and_then(Value::as_str) { patch.insert("helperModel".into(), json!(crate::util::take_chars(m, 200))); }
                // Zahlen kommen aus einem Eingabefeld: auch als Text annehmen, immer in den erlaubten Bereich holen
                let number = |key: &str| v.get(key).and_then(|x| x.as_f64().or_else(|| x.as_str().and_then(|t| t.trim().parse::<f64>().ok()))).filter(|n| n.is_finite()).map(|n| n.round() as i64);
                if let Some(n) = number("contextSize") { patch.insert("contextSize".into(), json!(n.clamp(CONTEXT_RANGE.0, CONTEXT_RANGE.1))); }
                if let Some(n) = number("contextReserve") { patch.insert("contextReserve".into(), json!(n.clamp(RESERVE_RANGE.0, RESERVE_RANGE.1))); }
                let theme = v.get("theme").and_then(Value::as_str).filter(|t| THEMES.iter().any(|(k, _)| k == t)).map(str::to_string);
                if let Some(t) = &theme { patch.insert("theme".into(), json!(t)); }
                settings::set(Value::Object(patch));
                if theme.is_some() { self.apply_theme(); }
                self.push_state();
                Value::Null
            }
            // Zuerst der sichtbare Chat: Fragen tragen die Kennung des Werkzeugaufrufs, die bei manchen Modellen in
            // jedem Chat gleich ist (call_0); die Oberfläche beantwortet immer die Frage des sichtbaren Chats.
            "chat:approve" => {
                let (id, ans) = (sarg(a, 0), sarg(a, 1));
                json!(agent.answer_approval(&id, &ans) || self.all_agents().iter().any(|x| x.answer_approval(&id, &ans)))
            }
            "chat:answer" => {
                let (id, text) = (sarg(a, 0), sarg(a, 1));
                json!(agent.answer_question(&id, &text) || self.all_agents().iter().any(|x| x.answer_question(&id, &text)))
            }
            "chat:stop-chat" => {
                let rt = self.st.lock().runtimes.get(&path_key(&sarg(a, 0))).cloned();
                if let Some(x) = rt.and_then(|rt| Self::agent_for(&rt, &sarg(a, 1))) { x.stop(); }
                Value::Null
            }
            "workspace:list" => self.workspace().list(&sarg(a, 0)),
            "workspace:find" => {
                // Dateien des Projekts, dazu Vorlagen der TF Bern (E66), damit sie unter den ersten acht stehen
                let mut out = self.workspace().find(&sarg(a, 0));
                let templates = crate::vorlagen::find(&sarg(a, 0));
                let at = out.len().min(5);
                out.splice(at..at, templates.into_iter().take(4));
                json!(out)
            }
            "workspace:import" => {
                let files: Vec<ImportFile> = arg(a, 1).as_array().cloned().unwrap_or_default().iter()
                    .map(|f| ImportFile { name: str_of(f, "name"), kind: str_of(f, "type"), data: bytes_of(&f["data"]) }).collect();
                match self.workspace().import(&sarg(a, 0), &files) {
                    Ok(p) => json!({ "ok": true, "paths": p }),
                    Err(err) => fail(format!("Nicht kopiert: {err}")),
                }
            }
            "folder:menu" => { let me = self.clone(); spawn(async move { let items = me.folder_menu_items(); me.popup_menu(items).await; }); Value::Null }
            "folder:show" => { let f = js_string(&settings::get("folder")); if !f.is_empty() && !paths::is_free_dir(&f) { self.host.open_path(&f).await; } Value::Null }
            "terminal:restart" => { self.start_terminal(); Value::Null }
            "terminal:logs" => json!(self.terminal().logs().join("\n")),
            "warning:accept" => { settings::set(json!({ "warningAccepted": true })); self.push_state(); Value::Null }
            "ask:set-url" => {
                let raw = sarg(a, 0);
                match url::Url::parse(raw.trim()) {
                    Err(_) => fail("Das ist keine gültige Adresse."),
                    Ok(u) if u.scheme() != "https" => fail("Die Adresse muss mit https:// beginnen."),
                    Ok(u) => {
                        let clean = format!("{}{}", u.origin().ascii_serialization(), u.path().trim_end_matches('/'));
                        settings::set(json!({ "askUrl": clean }));
                        self.load_ask();
                        // Erste Einrichtung: noch kein Projekt, also in die Chats ohne Projekt (E78)
                        if js_string(&settings::get("folder")).is_empty() { self.open_free().await; } else { self.start_terminal(); }
                        self.push_state();
                        json!({ "ok": true })
                    }
                }
            }
            "ask:connect" => self.ask.check(true).await,
            "ask:logout" => { self.ask.logout().await; Value::Null }
            // Anmelden im normalen Browser und Token einfügen (ENTSCHEIDE E57). «ask:login-window» macht die Hülle.
            "ask:open-browser" => {
                let url = js_string(&settings::get("askUrl"));
                if url.to_lowercase().starts_with("https://") { self.host.open_external(&url); }
                Value::Null
            }
            "ask:use-token" => self.ask.use_token(&sarg(a, 0)).await,
            "ask:models" => { let _ = self.ask.load_models().await; self.push_state(); json!(self.ask.models()) }
            "thinking:set" => {
                let l = sarg(a, 0);
                settings::set(json!({ "thinking": if THINKING_LEVELS.contains(&l.as_str()) { l } else { "standard".into() } }));
                self.push_state();
                Value::Null
            }
            "memory:set" => { settings::set(json!({ "chatMemory": truthy(&arg(a, 0)) })); self.push_state(); Value::Null }
            "ui:set" => { settings::set(json!({ "interactiveUi": truthy(&arg(a, 0)) })); self.push_state(); Value::Null }
            // Adresse wie askfile, nur mit askui und «ansicht» (http://askui.localhost/ansicht/<id> bzw. askui://ansicht/<id>)
            "ui:put" => crate::uiblocks::put(&sarg(a, 0))
                .map(|id| json!(format!("{}{id}", self.file_base.replace("askfile", "askui").replace("/datei/", "/ansicht/"))))
                .unwrap_or(Value::Null),
            "web:set" => { settings::set(json!({ "webSearch": truthy(&arg(a, 0)) })); self.push_state(); Value::Null }
            "model:set" => {
                settings::set(json!({ "model": sarg(a, 0) }));
                if !agent.running() { agent.reset_modes(); } // ein anderes Modell kann Werkzeuge und Bilder wieder
                self.push_state();
                Value::Null
            }
            "chat:send" => self.chat_send(&agent, &sarg(a, 0), &arg(a, 1), &arg(a, 2)),
            "chat:undo" => agent.undo(&sarg(a, 0)).await,
            "chat:redo" => agent.redo(&sarg(a, 0)).await,
            "chat:rewind" => agent.rewind().await,
            // Läuft der Chat (auch im Hintergrund), ändert sein eigener Agent ihn: sonst überschriebe dessen nächstes
            // Speichern den neuen Titel wieder
            "chat:rename" => { let o = self.chat_owner(&sarg(a, 0), &sarg(a, 1)); json!(o.rename(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2))) }
            "chat:pin" => { let o = self.chat_owner(&sarg(a, 0), &sarg(a, 1)); json!(o.pin(&sarg(a, 0), &sarg(a, 1), truthy(&arg(a, 2)))) }
            // Suche über den Inhalt aller Chats der Ordner in der Liste: [{ folder, id, title, updated, snippets }]
            "chat:search" => {
                let q = sarg(a, 0);
                let mut hits: Vec<Value> = Vec::new();
                for g in self.chat_groups().as_array().cloned().unwrap_or_default() {
                    let f = str_of(&g, "folder");
                    if let Ok(list) = crate::history::search(Path::new(&f), &q, "") {
                        for h in list {
                            hits.push(json!({ "folder": f, "name": g["name"], "id": h.id, "title": h.title, "updated": h.updated, "score": h.score, "snippets": h.snippets }));
                        }
                    }
                }
                hits.sort_by(|x, y| {
                    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_f64).unwrap_or(0.0);
                    s(y, "score").partial_cmp(&s(x, "score")).unwrap_or(std::cmp::Ordering::Equal)
                        .then(s(y, "updated").partial_cmp(&s(x, "updated")).unwrap_or(std::cmp::Ordering::Equal))
                });
                hits.truncate(30);
                Value::Array(hits)
            }
            // Nutzungsübersicht (ENTSCHEIDE E49): lokal aus den Chats aller Projekte; withAsk=false ohne Anfrage an ASK
            "usage:get" => {
                let mut projects = settings::load()["projects"].as_array().cloned().unwrap_or_default();
                // Chats ohne Projekt zählen mit (E78), sobald es welche gibt
                let free = paths::free_dir();
                if free.join(".ask").join("chats").exists() {
                    projects.insert(0, json!({ "folder": free.to_string_lossy(), "name": "Chats ohne Projekt", "color": "" }));
                }
                let mut result = serde_json::to_value(crate::usage::aggregate(&projects, Default::default())).unwrap_or(json!({}));
                result["ask"] = if arg(a, 0) == json!(false) { Value::Null } else {
                    crate::usage::ask_usage(Some(&self.ask), Default::default()).await.unwrap_or(Value::Null)
                };
                result
            }
            "whatsnew:seen" => {
                settings::set(json!({ "lastVersion": self.host.app_version() }));
                self.push_state();
                Value::Null
            }
            "prefs:set" => {
                let mut next = self.prefs();
                if let Some(m) = arg(a, 0).as_object() {
                    for (k, v) in m {
                        if next.get(k).is_some_and(|d| d.is_boolean() == v.is_boolean()) { next[k] = v.clone(); }
                    }
                }
                settings::set(json!({ "prefs": next }));
                self.push_state();
                Value::Null
            }
            "chat:stop" => { agent.stop(); Value::Null }
            // Nebenfrage mit /btw (E86): läuft neben dem Auftrag
            "chat:btw" => agent.btw(&sarg(a, 0)),
            // /zusammenfassen: ältere Aufträge jetzt zusammenfassen (E78)
            "chat:compact" => { let r = agent.compact_now().await; self.push_state(); r }
            "chat:goal" => { agent.set_goal(&sarg(a, 0)); Value::Null }
            "github:status" => {
                let folder = js_string(&settings::get("folder"));
                let res: anyhow::Result<Value> = if folder.is_empty() || paths::is_free_dir(&folder) {
                    let login = if self.github.connected() { self.github.whoami().await.unwrap_or_default() } else { String::new() };
                    Ok(json!({ "linked": false, "connected": self.github.connected(), "login": login }))
                } else {
                    self.github.status(Path::new(&folder)).await
                };
                match res {
                    Ok(s) => { let mut v = merge_ok(s); v["trusted"] = json!(self.github_trust.list()); v }
                    Err(err) => json!({ "ok": false, "connected": self.github.connected(), "message": err.to_string() }),
                }
            }
            "github:connect" => match self.github.connect(&sarg(a, 0)).await {
                Ok(login) => json!({ "ok": true, "login": login }),
                Err(err) => json!({ "ok": false, "message": err.to_string() }),
            },
            "github:disconnect" => { self.github.disconnect(); Value::Null }
            "github:untrust" => { self.github_trust.remove(&sarg(a, 0)); Value::Null }
            "github:branches" => { let gh = self.github.clone(); self.gh_read(|f| async move { gh.branches(&f).await }).await }
            "github:pulls" => { let gh = self.github.clone(); self.gh_read(|f| async move { Ok(json!({ "pulls": gh.pull_requests(&f).await? })) }).await }
            "github:pull" => { let gh = self.github.clone(); self.gh_action(|f| async move { gh.pull(&f).await }).await }
            "github:clone" => { let (gh, r) = (self.github.clone(), sarg(a, 0)); self.gh_action(|f| async move { gh.clone_repo(&f, &r).await }).await }
            "github:push" => {
                let msg = { let m = sarg(a, 0); if m.trim().is_empty() { "Änderungen aus ASK".to_string() } else { m.trim().to_string() } };
                self.gh_tool("github_push", json!({ "nachricht": msg, "ueberschreiben": arg(a, 1) == json!(true) })).await
            }
            "github:create" => self.gh_tool("github_create", json!({ "name": sarg(a, 0).trim(), "privat": arg(a, 1) != json!(false) })).await,
            "github:switch" => { let (gh, n) = (self.github.clone(), sarg(a, 0)); self.gh_action(|f| async move { gh.switch_branch(&f, &n).await }).await }
            "github:branch-create" => { let (gh, n) = (self.github.clone(), sarg(a, 0)); self.gh_action(|f| async move { gh.create_branch(&f, &n).await }).await }
            "github:pr-create" => {
                let (gh, t, b) = (self.github.clone(), sarg(a, 0), sarg(a, 1));
                self.gh_action(|f| async move { gh.create_pull_request(&f, &t, &b, "").await }).await
            }
            "chat:new" => { let f = { let f = sarg(a, 0); if f.is_empty() { js_string(&settings::get("folder")) } else { f } }; self.open_chat(&f, NEW).await }
            "chat:groups" => self.chat_groups(),
            "chat:open" => self.open_chat(&sarg(a, 0), &sarg(a, 1)).await,
            "chat:delete" => {
                let (folder, id, title) = (sarg(a, 0), sarg(a, 1), sarg(a, 2));
                let res = self.show_box(json!({ "type": "question", "buttons": ["Löschen", "Abbrechen"], "defaultId": 1, "cancelId": 1, "dangerId": 0,
                    "message": "Chat löschen?", "detail": if paths::is_free_dir(&folder) { format!("«{title}» wird gelöscht.") }
                        else { format!("«{title}» wird aus {folder} entfernt. Die Dateien im Ordner bleiben.") } })).await;
                // Während der Rückfrage kann ein anderer Chat sichtbar geworden sein (Klick auf eine Benachrichtigung)
                let agent = self.agent();
                let rt = self.st.lock().runtimes.get(&path_key(&folder)).cloned();
                let busy = rt.as_ref().and_then(|rt| Self::agent_for(rt, &id));
                if busy.as_ref().is_some_and(|b| b.running()) || res.get("response").and_then(Value::as_i64) != Some(0) {
                    json!(false)
                } else {
                    let ok = agent.remove(&folder, &id);
                    if let (Some(b), Some(rt)) = (&busy, &rt) {
                        if !Arc::ptr_eq(b, &agent) { rt.agents.lock().retain(|x| !Arc::ptr_eq(x, b)); }
                    }
                    if busy.as_ref().is_some_and(|b| Arc::ptr_eq(b, &agent)) { agent.new_chat(); }
                    json!(ok)
                }
            }
            // pending: offene Frage oder Rückfrage eines Chats, der schon arbeitet (z. B. nach dem Wechsel aus einem anderen Chat)
            "chat:move" => self.move_chat(&sarg(a, 0), &sarg(a, 1), &sarg(a, 2)).await,
            "chat:export" => self.export_chat(&sarg(a, 0), &sarg(a, 1)).await,
            "chat:transcript" => { let mut s = agent.state(); s["items"] = agent.transcript(); s["pending"] = agent.pending_events(); s }
            "ui:chatbar" => { settings::set(json!({ "chatbarOpen": truthy(&arg(a, 0)) })); self.push_state(); Value::Null }
            "ui:sidebar" => { settings::set(json!({ "sidebarOpen": truthy(&arg(a, 0)) })); self.push_state(); Value::Null }
            // Entwicklungsserver des sichtbaren Projekts (ENTSCHEIDE E48). Die Person startet ohne Rückfrage: Sie tippt den Befehl selbst.
            "servers:list" => self.active_rt().map(|rt| json!(rt.servers.list())).unwrap_or(json!([])),
            "servers:start" => match self.active_rt() {
                Some(rt) if paths::is_free_dir(&rt.folder) => fail("Server gibt es nur in einem Projekt."),
                Some(rt) if rt.terminal.state() == "läuft" => match rt.servers.start(&sarg(a, 0), &sarg(a, 1), "person").await {
                    Ok(s) => json!({ "ok": true, "server": s }),
                    Err(err) => fail(err.to_string()),
                },
                _ => fail("Das Terminal läuft nicht."),
            },
            "servers:stop" => match self.active_rt() {
                None => json!({ "ok": false }),
                Some(rt) => match rt.servers.stop(&sarg(a, 0)).await {
                    Ok(ok) => json!({ "ok": ok }),
                    Err(err) => fail(err.to_string()),
                },
            },
            "servers:remove" => json!(self.active_rt().is_some_and(|rt| rt.servers.remove(&sarg(a, 0)))),
            "servers:output" => self.active_rt().map(|rt| json!(rt.servers.output(&sarg(a, 0), usize::MAX))).unwrap_or(json!([])),
            // Anmeldungen im Browser der App löschen (Cookies, Speicher); nach Rückfrage. Das Löschen selbst macht die Hülle.
            "browser:clear" => json!(self.confirm("Anmeldungen im Browser löschen?",
                "Du wirst im Browser der App überall abgemeldet. ASK und dein Chrome sind davon nicht betroffen.",
                ConfirmOptions { danger: true, ..Default::default() }).await.ok),
            "clipboard:write" => { self.host.clipboard_write(&sarg(a, 0)); Value::Null }
            "link:open" => { self.host.open_external(&sarg(a, 0)); Value::Null }
            "workspace:files" => self.workspace().files(),
            "file:preview" => self.file_preview(&sarg(a, 0)).await,
            // Bild als data:-Adresse, wenn das Protokoll askfile es in der Oberfläche nicht liefert (Bilder im Chat)
            "file:data" => {
                let rel = sarg(a, 0);
                match self.workspace().resolve(&rel) {
                    Some(full) if IMAGE_EXT.contains(&ext_of(&full).as_str()) => {
                        let mime = match ext_of(&full).as_str() { ".jpg" | ".jpeg" => "image/jpeg", ".gif" => "image/gif", ".webp" => "image/webp", ".svg" => "image/svg+xml", ".bmp" => "image/bmp", _ => "image/png" };
                        match std::fs::metadata(&full) {
                            Ok(m) if m.len() <= 25 * 1024 * 1024 => match std::fs::read(&full) {
                                Ok(data) => json!(format!("data:{mime};base64,{}", base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data))),
                                Err(_) => Value::Null,
                            },
                            _ => Value::Null,
                        }
                    }
                    _ => Value::Null,
                }
            }
            "file:open" => self.file_open(&sarg(a, 0)).await,
            "file:show" => { if let Some(full) = self.workspace().resolve(&sarg(a, 0)) { self.host.show_item_in_folder(&full.to_string_lossy()); } Value::Null }
            _ => {
                let confirm = self.confirm_fn();
                return match self.knowledge.handle(channel, a, self.host.as_ref(), &confirm).await {
                    Some(r) => r,
                    None => Err(format!("Unbekannter Aufruf: {channel}")),
                };
            }
        };
        Ok(v)
    }

    /// Ist das ein Ort für Chats: die Chats ohne Projekt oder ein Projekt aus der Liste, dessen Ordner es gibt?
    fn chat_place(folder: &str) -> bool {
        paths::is_free_dir(folder) || (settings::find_project(folder).is_some() && Path::new(folder).is_dir())
    }

    /// Chat in ein anderes Projekt oder zu den Chats ohne Projekt verschieben (ENTSCHEIDE E78). War er sichtbar, geht
    /// es am neuen Ort mit ihm weiter.
    async fn move_chat(&self, from: &str, id: &str, to: &str) -> Value {
        if !Self::chat_place(from) || !Self::chat_place(to) {
            return fail("Das Ziel gibt es nicht (mehr).");
        }
        let to = if paths::is_free_dir(to) { Self::free_folder() } else { to.to_string() };
        let rt = self.st.lock().runtimes.get(&path_key(from)).cloned();
        let holder = rt.as_ref().and_then(|rt| Self::agent_for(rt, id));
        if holder.as_ref().is_some_and(|h| h.running() || h.undoing()) {
            return fail("Der Chat arbeitet noch. Zuerst stoppen, dann verschieben.");
        }
        let visible = self.agent();
        let was_visible = visible.chat_id() == id && same_path(&visible.get_folder(), from);
        if let Err(err) = Agent::move_chat(from, &to, id) {
            return fail(err.to_string());
        }
        // Verweise des Gedächtnisses («aus Chat …») zeigen auf den neuen Ort
        let _ = self.knowledge.memory.chat_moved(id, from, &to);
        // Kein Agent hält den Chat mehr am alten Ort (ein späteres Speichern legte ihn dort wieder an)
        if let (Some(h), Some(rt)) = (&holder, &rt) {
            if Arc::ptr_eq(h, &visible) { h.new_chat(); } else { rt.agents.lock().retain(|x| !Arc::ptr_eq(x, h)); }
        }
        self.send("chat:groups-changed", Value::Null);
        if was_visible {
            let res = self.open_chat(&to, id).await;
            if !bool_of(&res, "ok") {
                return json!({ "ok": true, "note": str_of(&res, "reason") }); // verschoben, aber dort nicht zu öffnen
            }
        }
        json!({ "ok": true })
    }

    /// Chat als Markdown-Datei speichern
    async fn export_chat(&self, folder: &str, id: &str) -> Value {
        if !Self::chat_place(folder) {
            return fail("Diesen Chat gibt es nicht mehr.");
        }
        let place = if paths::is_free_dir(folder) {
            "Chat ohne Projekt".to_string()
        } else {
            format!("Projekt «{}»", settings::find_project(folder).map(|p| str_of(&p, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| win_basename(folder)))
        };
        let Some(text) = Agent::export_markdown(folder, id, &place) else { return fail("Diesen Chat gibt es nicht mehr.") };
        let title = text.lines().next().unwrap_or("").trim_start_matches("# ").to_string();
        let name = { let n = safe_folder_name(&crate::util::take_chars(&title, 60)); if n.is_empty() { "Chat".to_string() } else { n } };
        let filters: crate::host::Filters = vec![("Markdown".into(), vec!["md".into()])];
        let Some(path) = self.host.pick_save_file("Chat exportieren", &format!("{name}.md"), filters).await.filter(|p| !p.is_empty()) else {
            return json!({ "ok": false, "canceled": true });
        };
        match std::fs::write(&path, text) {
            Ok(()) => json!({ "ok": true, "path": path }),
            Err(err) => fail(format!("Die Datei liess sich nicht speichern: {err}")),
        }
    }

    fn chat_owner(&self, folder: &str, id: &str) -> Arc<Agent> {
        let rt = self.st.lock().runtimes.get(&path_key(folder)).cloned();
        rt.and_then(|rt| Self::agent_for(&rt, id)).unwrap_or_else(|| self.agent())
    }

    /// Anhänge zuerst nach .ask/anhaenge/<Chat>/ speichern; scheitert das, geht nichts an das Modell.
    /// kept: Anhänge einer zurückgenommenen Eingabe (chat:rewind), die schon dort liegen.
    fn chat_send(&self, agent: &Arc<Agent>, text: &str, files: &Value, kept: &Value) -> Value {
        if agent.running() {
            return fail("Es läuft schon ein Auftrag.");
        }
        if agent.undoing() {
            return fail("Der Ordner wird gerade zurückgesetzt. Einen Moment.");
        }
        let folder = agent.get_folder();
        let chat = agent.chat_id();
        let mut attachments: Vec<Value> = Vec::new();
        let prefix = format!(".ask/anhaenge/{chat}/");
        for k in kept.as_array().cloned().unwrap_or_default().iter().take(10) {
            let rel = str_of(k, "path");
            // nur Dateien, die wirklich bei den Anhängen dieses Chats liegen
            let Some(name) = rel.strip_prefix(&prefix) else { continue };
            if folder.is_empty() || name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
                continue;
            }
            let full = Path::new(&folder).join(".ask").join("anhaenge").join(&chat).join(name);
            if let Ok(m) = std::fs::metadata(&full) {
                let image = { let l = name.to_lowercase(); [".png", ".jpg", ".jpeg", ".gif", ".webp"].iter().any(|e| l.ends_with(e)) };
                attachments.push(json!({ "path": rel, "name": win_basename(&rel), "size": m.len(), "image": image }));
            }
        }
        if let Some(list) = files.as_array().filter(|l| !l.is_empty()) {
            if folder.is_empty() {
                return fail("Zuerst einen Arbeitsordner wählen.");
            }
            let files: Vec<AttachmentFile> = list.iter().map(|f| AttachmentFile { name: str_of(f, "name"), kind: str_of(f, "type"), data: bytes_of(&f["data"]) }).collect();
            match save_attachments(Path::new(&folder), &files, &chat) {
                Ok(saved) => attachments.extend(saved.iter().map(|s| json!(s))),
                Err(err) => return fail(format!("Anhänge nicht gespeichert: {err}")),
            }
        }
        // «/name …»: eigener Skill (ENTSCHEIDE E51)
        let extra = match self.knowledge.skill_for(text) {
            Some(s) => json!({ "skill": s }),
            None => json!({}),
        };
        agent.send(text, attachments, extra);
        json!({ "ok": true })
    }
}

fn merge_ok(v: Value) -> Value {
    let mut out = json!({ "ok": true });
    if let Value::Object(m) = v {
        for (k, x) in m {
            out[k] = x;
        }
    }
    out
}

/// Projekt schliessen: zuerst seine Server stoppen (höchstens 3 s), dann das Terminal. Dessen Job-Objekt beendet,
/// was danach noch läuft.
async fn stop_runtime(rt: &Runtime) {
    let _ = tokio::time::timeout(Duration::from_secs(3), rt.servers.stop_all()).await;
    rt.terminal.stop().await;
}

fn read_start(file: &Path, bytes: usize) -> std::io::Result<Vec<u8>> {
    use std::io::Read;
    let mut buf = Vec::with_capacity(bytes);
    std::fs::File::open(file)?.take(bytes as u64).read_to_end(&mut buf)?;
    Ok(buf)
}

pub fn read_instruction(name: &str, folder: &str, chat: &str) -> String {
    match std::fs::read_to_string(paths::instructions_dir().join(name)) {
        Ok(text) => text.replace("{ordner}", folder).replace("{ordnername}", &win_basename(folder)).replace("{chat}", chat),
        Err(_) => String::new(),
    }
}

/// Projektnotizen (.ask/projekt.md): was jeder Chat in diesem Ordner wissen soll, wie CLAUDE.md in Claude Code.
/// Das Modell pflegt sie selbst, die Person kann sie im Editor ändern.
pub fn project_notes(folder: &str) -> String {
    if folder.is_empty() {
        return String::new();
    }
    let Ok(text) = std::fs::read_to_string(Path::new(folder).join(NOTES)) else { return String::new() };
    let mut text = text.trim().to_string();
    if text.is_empty() {
        return String::new();
    }
    if text.chars().count() > NOTES_MAX {
        text = format!("{}\n… (gekürzt, halte die Notizen kürzer)", crate::util::take_chars(&text, NOTES_MAX));
    }
    format!("\n\n# Projektnotizen (.ask/projekt.md)\n{text}")
}

/// Datei oder Ordner der App zu einem Arbeitsordner, ausserhalb davon (paths::project_data_dir). Frühere Versionen
/// legten Sicherung und GitHub-Verbindung in .ask ab: einmal übernehmen. Was von dort kommt, prüfen snapshot.rs und
/// github.rs beim Lesen, denn das Modell konnte es ändern. Nur beim ersten Blick der App auf den Ordner (Marke
/// «.uebernommen-<name>»), bevor ein Auftrag darin lief: Sonst könnte das Modell später eine eigene .ask/github.json
/// anlegen und so bestimmen, wohin hochgeladen wird.
pub fn project_file(folder: &str, name: &str, legacy: &str) -> PathBuf {
    let dir = paths::project_data_dir(folder);
    let target = dir.join(name);
    let old = Path::new(folder).join(legacy);
    let marker = dir.join(format!(".uebernommen-{name}"));
    let _ = (|| -> std::io::Result<()> {
        if !marker.exists() {
            if !target.exists() && old.exists() {
                std::fs::create_dir_all(&dir)?;
                copy_all(&old, &target)?;
            }
            std::fs::create_dir_all(&dir)?;
            std::fs::write(&marker, "")?;
        }
        if old.is_dir() {
            std::fs::remove_dir_all(&old)?;
        } else if old.exists() {
            std::fs::remove_file(&old)?;
        }
        Ok(())
    })();
    target
}

fn copy_all(from: &Path, to: &Path) -> std::io::Result<()> {
    if from.is_dir() {
        std::fs::create_dir_all(to)?;
        for e in std::fs::read_dir(from)? {
            let e = e?;
            copy_all(&e.path(), &to.join(e.file_name()))?;
        }
        Ok(())
    } else {
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Alte Ablagen eines Ordners übernehmen, sobald die App ihn öffnet (vor dem ersten Auftrag)
fn migrate_project(folder: &str) {
    project_file(folder, "github.json", ".ask/github.json");
    project_file(folder, "sicherung", ".ask/sicherung");
}

/// Text für «Neu in ASK»: aus resources/neu.json ({ "version", "text" }, beim Bauen aus dem Release-Text
/// geschrieben, scripts/dist.js); passt die Version nicht oder fehlt die Datei, ein allgemeiner Satz.
/// Fortschritt beim Entpacken von Python in Prozent, -1: nichts zu tun (Zustand «python»)
static PYTHON_PERCENT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

pub fn whats_new_text(file: &std::path::Path, current: &str) -> Value {
    let stored: Value = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let text = if js_string(&stored["version"]) == current { js_string(&stored["text"]).trim().to_string() } else { String::new() };
    let text = if text.is_empty() { format!("ASK ist jetzt auf Version {current}.") } else { text };
    json!({ "version": current, "text": text })
}

#[cfg(test)]
mod tests {
    #[test]
    fn neu_in_ask_text_nur_fuer_die_passende_version() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("neu.json");
        assert_eq!(whats_new_text(&f, "0.3.11")["text"], "ASK ist jetzt auf Version 0.3.11.");
        std::fs::write(&f, r#"{ "version": "0.3.11", "text": "Schneller.\nNeu anmelden." }"#).unwrap();
        assert_eq!(whats_new_text(&f, "0.3.11"), serde_json::json!({ "version": "0.3.11", "text": "Schneller.\nNeu anmelden." }));
        assert_eq!(whats_new_text(&f, "0.3.12")["text"], "ASK ist jetzt auf Version 0.3.12.");
    }

    use super::*;

    #[test]
    fn kontextfenster_das_eingestellte_hoechstens_was_ask_meldet() {
        assert_eq!(effective_context(0, 0, 250_000), 250_000, "nichts bekannt: die Einstellung");
        assert_eq!(effective_context(131_072, 0, 250_000), 131_072, "ASK meldet weniger");
        assert_eq!(effective_context(1_000_000, 0, 250_000), 250_000, "ASK meldet mehr: die Einstellung begrenzt");
        assert_eq!(effective_context(0, 32_768, 250_000), 32_768, "aus einer Fehlermeldung gelernt");
        assert_eq!(effective_context(131_072, 8_192, 250_000), 131_072, "die Angabe von ASK geht vor");
    }

    #[test]
    fn folder_names() {
        assert_eq!(safe_folder_name("Mein: Projekt?"), "Mein Projekt");
        assert_eq!(safe_folder_name("con"), "con-Projekt");
        assert_eq!(safe_folder_name("COM1"), "COM1-Projekt");
        assert_eq!(safe_folder_name("Bericht. "), "Bericht");
    }

    #[test]
    fn instructions_and_notes() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().to_string_lossy().to_string();
        assert_eq!(project_notes(&f), "");
        std::fs::create_dir_all(dir.path().join(".ask")).unwrap();
        std::fs::write(dir.path().join(".ask").join("projekt.md"), "Aufbau: A").unwrap();
        assert!(project_notes(&f).contains("# Projektnotizen (.ask/projekt.md)\nAufbau: A"));
    }

    #[test]
    fn legacy_project_files_move_once() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ASK_TEST_USER_DATA", dir.path().join("ud"));
        let work = dir.path().join("ordner");
        std::fs::create_dir_all(work.join(".ask")).unwrap();
        std::fs::write(work.join(".ask").join("github.json"), "{\"repo\":\"a/b\"}").unwrap();
        let f = work.to_string_lossy().to_string();
        let target = project_file(&f, "github.json", ".ask/github.json");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{\"repo\":\"a/b\"}");
        assert!(!work.join(".ask").join("github.json").exists());
        // Später angelegte Datei des Modells wird nicht übernommen, nur entfernt
        std::fs::write(work.join(".ask").join("github.json"), "{\"repo\":\"boese/x\"}").unwrap();
        project_file(&f, "github.json", ".ask/github.json");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "{\"repo\":\"a/b\"}");
        assert!(!work.join(".ask").join("github.json").exists());
    }
}
