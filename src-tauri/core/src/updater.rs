//! Updates aus dem öffentlichen Release-Repository (ENTSCHEIDE E37, E40, E57). Die installierte App fragt
//! einmal vor dem Herunterladen (Dialog in der App, E56), zeigt den Download als Karte unten links und
//! startet danach mit 10 s Countdown von selbst neu («Jetzt» oder «Später» in der Karte); die einzelne EXE
//! kann sich nicht selbst ersetzen und zeigt nur einen Hinweis mit Link zur neuen Version. Mit
//! «Beta-Versionen erhalten» kommen auch Vorabversionen (v0.3.0-beta.1). Während das Modell arbeitet,
//! fragt die App nie.
//!
//! Prüfen, Laden und Installieren übernimmt ein `UpdateBackend` (Tauri-Hülle: tauri-plugin-updater mit
//! signierten Paketen; Tests: Attrappe). Der Ablauf und die Texte sind dieselben wie mit electron-updater.
//!
//! Schnittstelle: `Updater::new(UpdaterOptions)`, `start()`, `stop()`, `snapshot()`, `check(manual)`,
//! `action()`, `busy_changed()`, `idle()`, `postpone()`, `tick()` (Countdown, jede Sekunde),
//! Ereignis «changed» über `events`.

use anyhow::Result;
use async_trait::async_trait;
use futures::future::BoxFuture;
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub const FIRST_CHECK_MS: u64 = 20 * 1000;
pub const CHECK_EVERY_MS: u64 = 6 * 60 * 60 * 1000;
pub const NOTES_MAX: usize = 700;
/// Countdown nach dem Download, bevor ASK für das Update neu startet
pub const RESTART_SECONDS: i64 = 10;

/// Neue Version laut Release-Repository
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub notes: String,
}

#[async_trait]
pub trait UpdateBackend: Send + Sync {
    /// Neuere Version als die laufende? beta: auch Vorabversionen
    async fn check(&self, beta: bool) -> Result<Option<UpdateInfo>>;
    /// Paket der zuletzt gefundenen Version laden; progress(Prozent)
    async fn download(&self, progress: Box<dyn Fn(u32) + Send + Sync>) -> Result<()>;
    /// Geladenes Paket installieren und ASK neu starten (kehrt normalerweise nicht zurück)
    async fn install(&self) -> Result<()>;
}

pub type AskFn = Arc<dyn Fn(Value) -> BoxFuture<'static, i64> + Send + Sync>;
pub type FlagFn = Arc<dyn Fn() -> bool + Send + Sync>;
pub type BeforeInstallFn = Arc<dyn Fn() -> BoxFuture<'static, ()> + Send + Sync>;

pub struct UpdaterOptions {
    pub backend: Arc<dyn UpdateBackend>,
    /// { owner, repo } aus app-config.json; None: Updates aus
    pub config: Option<(String, String)>,
    /// Installierte App oder einzelne EXE (in der Entwicklung aus)
    pub packaged: bool,
    pub portable: bool,
    pub current: String,
    pub beta: FlagFn,
    pub busy: FlagFn,
    /// Dialog in der App; gibt den Index des Knopfs (Schliessen: cancelId, ohne Fenster: -1)
    pub ask: AskFn,
    pub open_external: Arc<dyn Fn(&str) + Send + Sync>,
    /// Terminal und Agenten sauber beenden
    pub before_install: BeforeInstallFn,
    pub log: Arc<dyn Fn(&str) + Send + Sync>,
}

pub fn release_url(owner: &str, repo: &str, version: &str) -> String {
    format!("https://github.com/{owner}/{repo}/releases/tag/v{version}")
}

pub fn is_beta(version: &str) -> bool {
    version.contains('-')
}

/// Release-Text für den Dialog: ohne HTML, gekürzt.
pub fn notes_text(notes: &str) -> String {
    let re = |p: &str| regex::Regex::new(p).expect("Regex");
    let mut t = re(r"(?i)</(p|li|h\d)>|<br\s*/?>").replace_all(notes, "\n").to_string();
    t = re(r"(?i)<li[^>]*>").replace_all(&t, "• ").to_string();
    t = re(r"<[^>]+>").replace_all(&t, "").to_string();
    t = t.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'");
    t = re(r"[ \t]+").replace_all(&t, " ").to_string();
    t = re(r"\n\s*\n+").replace_all(&t, "\n").to_string();
    let t = t.trim().to_string();
    if t.chars().count() > NOTES_MAX {
        format!("{} …", crate::util::take_chars(&t, NOTES_MAX).trim_end())
    } else {
        t
    }
}

#[derive(Default)]
struct State {
    state: String,
    info: Option<UpdateInfo>,
    percent: u32,
    error: String,
    /// «Später» für diese Version: bis zum nächsten Start nicht mehr von selbst fragen
    dismissed: String,
    /// 'angebot' oder 'neustart', sobald das Modell fertig ist
    pending: String,
    dialog_open: bool,
    /// Sekunden bis zum Neustart (Zustand «neustart»)
    countdown: i64,
    counting: bool,
    /// «Später» nach dem Download: beim Schliessen installieren
    postponed: bool,
}

pub struct Updater {
    o: UpdaterOptions,
    s: Mutex<State>,
    enabled: bool,
    pub events: crate::util::Emitter,
    timers: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Updater {
    pub fn new(o: UpdaterOptions) -> Arc<Self> {
        let enabled = o.config.is_some() && o.packaged;
        let s = State { state: if enabled { "aktuell".into() } else { "aus".into() }, ..Default::default() };
        Arc::new(Self { o, s: Mutex::new(s), enabled, events: crate::util::Emitter::new(), timers: Mutex::new(Vec::new()) })
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn state(&self) -> String {
        self.s.lock().state.clone()
    }

    pub fn snapshot(&self) -> Value {
        let s = self.s.lock();
        json!({
            "state": s.state,
            "current": self.o.current,
            "version": s.info.as_ref().map(|i| i.version.clone()).unwrap_or_default(),
            "percent": s.percent,
            "error": s.error,
            "portable": self.o.portable,
            "beta": (self.o.beta)(),
            "enabled": self.enabled,
            "countdown": s.countdown,
            "postponed": s.postponed,
        })
    }

    fn changed(&self) {
        self.events.emit("changed", &Value::Null);
    }

    fn set(&self, state: &str, f: impl FnOnce(&mut State)) {
        {
            let mut s = self.s.lock();
            s.state = state.to_string();
            f(&mut s);
        }
        self.changed();
    }

    pub fn start(self: &Arc<Self>) {
        if !self.enabled {
            return;
        }
        let me = self.clone();
        let first = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(FIRST_CHECK_MS)).await;
            me.check(false).await;
        });
        let me = self.clone();
        let every = tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_millis(CHECK_EVERY_MS));
            iv.tick().await;
            loop {
                iv.tick().await;
                me.check(false).await;
            }
        });
        // Countdown: jede Sekunde ein Schritt, nur im Zustand «neustart»
        let me = self.clone();
        let count = tokio::spawn(async move {
            let mut iv = tokio::time::interval(Duration::from_secs(1));
            loop {
                iv.tick().await;
                if me.s.lock().counting {
                    me.tick().await;
                }
            }
        });
        self.timers.lock().extend([first, every, count]);
    }

    pub fn stop(&self) {
        for t in self.timers.lock().drain(..) {
            t.abort();
        }
    }

    /// manual: von der Person ausgelöst (Menü, /update, Klick unten): dann auch «kein Update» und Fehler melden.
    pub async fn check(self: &Arc<Self>, manual: bool) {
        if !self.enabled {
            if manual {
                self.dialog(json!({ "type": "info", "message": "Updates gibt es nur in der installierten App oder der ASK-EXE.", "buttons": ["OK"] })).await;
            }
            return;
        }
        let state = self.state();
        if state == "lädt" || state == "neustart" {
            return;
        }
        if state == "geladen" {
            if manual {
                self.restart_now().await;
            }
            return;
        }
        let had_offer = state == "verfügbar" && self.s.lock().info.is_some();
        self.set("prüft", |s| s.error.clear());
        let result = self.o.backend.check((self.o.beta)()).await;
        match result {
            Err(err) => {
                let msg = err.to_string().lines().next().unwrap_or("").to_string();
                self.set("fehler", |s| s.error = msg.clone());
                if manual {
                    self.dialog(json!({ "type": "warning", "message": "Die Suche nach Updates hat nicht geklappt.",
                        "detail": format!("Ist der Laptop mit dem Internet verbunden?\n\n{msg}"), "buttons": ["OK"] })).await;
                }
            }
            Ok(Some(info)) if !info.version.is_empty() => {
                let version = info.version.clone();
                self.set("verfügbar", |s| s.info = Some(info));
                let dismissed = self.s.lock().dismissed.clone();
                if manual || dismissed != version {
                    self.offer().await;
                }
            }
            Ok(_) => {
                self.set(if had_offer { "verfügbar" } else { "aktuell" }, |_| {});
                if manual {
                    let detail = if (self.o.beta)() { "Beta-Versionen sind eingeschaltet." } else { "" };
                    self.dialog(json!({ "type": "info", "message": format!("ASK {} ist die neueste Version.", self.o.current),
                        "detail": detail, "buttons": ["OK"] })).await;
                }
            }
        }
    }

    /// Klick auf «Update» in der Statuszeile oder «Jetzt neu starten» in der Karte
    pub async fn action(self: &Arc<Self>) {
        match self.state().as_str() {
            "verfügbar" => self.offer().await,
            "geladen" | "neustart" => self.restart_now().await,
            "lädt" => {}
            _ => self.check(true).await,
        }
    }

    /// Das Modell arbeitet wieder: einen laufenden Countdown anhalten, Neustart nach dem Auftrag.
    pub fn busy_changed(&self) {
        if (self.o.busy)() && self.state() == "neustart" {
            self.stop_countdown();
            self.set("geladen", |s| s.pending = "neustart".into());
        }
    }

    /// Das Modell ist fertig: aufgeschobene Frage oder aufgeschobenen Neustart nachholen.
    pub fn idle(self: &Arc<Self>) {
        let pending = std::mem::take(&mut self.s.lock().pending);
        let state = self.state();
        if pending == "angebot" && state == "verfügbar" {
            let me = self.clone();
            tokio::spawn(async move { me.offer().await });
        }
        if pending == "neustart" && state == "geladen" {
            self.schedule_restart();
        }
    }

    async fn offer(self: &Arc<Self>) {
        let Some(info) = self.s.lock().info.clone() else { return };
        if (self.o.busy)() {
            self.s.lock().pending = "angebot".into();
            return;
        }
        let version = info.version.clone();
        let label = if is_beta(&version) { format!("ASK {version} (Beta)") } else { format!("ASK {version}") };
        let notes = notes_text(&info.notes);
        let neu = if notes.is_empty() { String::new() } else { format!("\n\nNeu:\n{notes}") };
        let current = &self.o.current;
        if self.o.portable {
            let intro = format!("Du hast {current}. Diese ASK-EXE aktualisiert sich nicht selbst: Lade die neue EXE herunter und starte sie. Chats und Einstellungen bleiben.");
            let mut opts = json!({ "type": "info", "message": format!("{label} ist da."), "detail": format!("{intro}{neu}"),
                "buttons": ["Download öffnen", "Später"], "defaultId": 0, "cancelId": 1 });
            self.card(&mut opts, &version, &intro, &notes);
            let res = self.dialog(opts).await;
            if res == 0 {
                if let Some((owner, repo)) = &self.o.config {
                    (self.o.open_external)(&release_url(owner, repo, &version));
                }
            } else if res == 1 {
                self.s.lock().dismissed = version;
            }
            return;
        }
        let intro = format!("Du hast {current}. ASK lädt das Update im Hintergrund (Anzeige unten links), du kannst weiterarbeiten. Danach startet ASK von selbst neu, ausser während eines Auftrags. Chats, Ordner und Einstellungen bleiben.");
        let mut opts = json!({ "type": "question", "message": format!("{label} ist da. Jetzt aktualisieren?"), "detail": format!("{intro}{neu}"),
            "buttons": ["Jetzt aktualisieren", "Später"], "defaultId": 0, "cancelId": 1 });
        self.card(&mut opts, &version, &intro, &notes);
        let res = self.dialog(opts).await;
        if res == 1 {
            self.s.lock().dismissed = version;
        }
        if res != 0 {
            return;
        }
        self.download().await;
    }

    async fn download(self: &Arc<Self>) {
        self.set("lädt", |s| { s.percent = 0; s.postponed = false; });
        let me = self.clone();
        let progress = Box::new(move |p: u32| {
            me.s.lock().percent = p.min(100);
            me.changed();
        });
        if let Err(err) = self.o.backend.download(progress).await {
            let msg = err.to_string().lines().next().unwrap_or("").to_string();
            self.set("verfügbar", |s| s.error = msg.clone());
            self.dialog(json!({ "type": "warning", "message": "Das Update konnte nicht heruntergeladen werden.",
                "detail": format!("Später nochmals versuchen (unten auf «Update» klicken).\n\n{msg}"), "buttons": ["OK"] })).await;
            return;
        }
        self.set("geladen", |s| s.percent = 100);
        self.schedule_restart();
    }

    /// Nach dem Download: Countdown, dann Neustart. Während eines Auftrags erst danach.
    fn schedule_restart(&self) {
        if self.s.lock().postponed || self.state() != "geladen" {
            return;
        }
        if (self.o.busy)() {
            self.s.lock().pending = "neustart".into();
            return;
        }
        self.set("neustart", |s| { s.countdown = RESTART_SECONDS; s.counting = true; });
    }

    /// Eine Sekunde des Countdowns (läuft in `start` jede Sekunde; Tests rufen es direkt)
    pub async fn tick(self: &Arc<Self>) {
        if !self.s.lock().counting {
            return;
        }
        if (self.o.busy)() {
            self.busy_changed();
            return;
        }
        let left = {
            let mut s = self.s.lock();
            s.countdown -= 1;
            s.countdown
        };
        if left <= 0 {
            self.restart_now().await;
        } else {
            self.changed();
        }
    }

    fn stop_countdown(&self) {
        let mut s = self.s.lock();
        s.counting = false;
        s.countdown = 0;
    }

    /// «Später» in der Karte: nicht mehr von selbst neu starten; das Update installiert sich beim Schliessen.
    pub fn postpone(&self) {
        let state = self.state();
        if state != "neustart" && state != "geladen" {
            return;
        }
        self.stop_countdown();
        self.set("geladen", |s| { s.pending.clear(); s.postponed = true; });
    }

    /// Beim Beenden mit geladenem, aufgeschobenem Update: jetzt installieren
    pub async fn install_on_quit(&self) -> bool {
        if self.state() != "geladen" {
            return false;
        }
        (self.o.before_install)().await;
        self.o.backend.install().await.is_ok()
    }

    async fn restart_now(self: &Arc<Self>) {
        self.stop_countdown();
        if (self.o.busy)() {
            // ein Auftrag läuft: danach
            self.set("geladen", |s| s.pending = "neustart".into());
            return;
        }
        self.set("installiert", |_| {});
        (self.o.before_install)().await;
        // Nicht still: Der Installer zeigt seinen Fortschritt und startet ASK danach wieder.
        if let Err(err) = self.o.backend.install().await {
            (self.o.log)(&format!("Installieren fehlgeschlagen: {err}"));
            let msg = err.to_string();
            self.set("geladen", |s| s.error = msg);
        }
    }

    /// Zusatzangaben für den eigenen Update-Dialog der App
    fn card(&self, opts: &mut Value, version: &str, intro: &str, notes: &str) {
        opts["kind"] = json!("update");
        opts["version"] = json!(version);
        opts["current"] = json!(self.o.current);
        opts["beta"] = json!(is_beta(version));
        opts["intro"] = json!(intro);
        opts["notes"] = json!(notes);
    }

    /// Nie zwei Dialoge übereinander
    async fn dialog(&self, options: Value) -> i64 {
        {
            let mut s = self.s.lock();
            if s.dialog_open {
                return -1;
            }
            s.dialog_open = true;
        }
        let res = (self.o.ask)(options).await;
        self.s.lock().dialog_open = false;
        res
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct Fake {
        result: Mutex<Option<UpdateInfo>>,
        fail: AtomicBool,
        downloads: AtomicUsize,
        installs: AtomicUsize,
        betas: Mutex<Vec<bool>>,
    }

    #[async_trait]
    impl UpdateBackend for Fake {
        async fn check(&self, beta: bool) -> Result<Option<UpdateInfo>> {
            self.betas.lock().push(beta);
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("net::ERR_INTERNET_DISCONNECTED\nmehr");
            }
            Ok(self.result.lock().clone())
        }
        async fn download(&self, progress: Box<dyn Fn(u32) + Send + Sync>) -> Result<()> {
            progress(50);
            self.downloads.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn install(&self) -> Result<()> {
            self.installs.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Setup {
        u: Arc<Updater>,
        fake: Arc<Fake>,
        dialogs: Arc<Mutex<Vec<Value>>>,
        opened: Arc<Mutex<Vec<String>>>,
        busy: Arc<AtomicBool>,
        before: Arc<AtomicUsize>,
    }

    fn setup(result: Option<UpdateInfo>, answers: Vec<i64>, portable: bool, packaged: bool) -> Setup {
        let fake = Arc::new(Fake::default());
        *fake.result.lock() = result;
        let dialogs = Arc::new(Mutex::new(Vec::new()));
        let answers = Arc::new(Mutex::new(answers));
        let opened = Arc::new(Mutex::new(Vec::new()));
        let busy = Arc::new(AtomicBool::new(false));
        let before = Arc::new(AtomicUsize::new(0));
        let (d, a, o, b, bi) = (dialogs.clone(), answers.clone(), opened.clone(), busy.clone(), before.clone());
        let u = Updater::new(UpdaterOptions {
            backend: fake.clone(),
            config: Some(("KIYeager".into(), "ASK-Releases".into())),
            packaged,
            portable,
            current: "0.1.0".into(),
            beta: Arc::new(|| false),
            busy: Arc::new(move || b.load(Ordering::SeqCst)),
            ask: Arc::new(move |opts| {
                d.lock().push(opts);
                let r = { let mut a = a.lock(); if a.is_empty() { 1 } else { a.remove(0) } };
                Box::pin(async move { r })
            }),
            open_external: Arc::new(move |u| o.lock().push(u.to_string())),
            before_install: Arc::new(move || { bi.fetch_add(1, Ordering::SeqCst); Box::pin(async {}) }),
            log: Arc::new(|_| {}),
        });
        Setup { u, fake, dialogs, opened, busy, before }
    }

    fn info(v: &str, notes: &str) -> Option<UpdateInfo> {
        Some(UpdateInfo { version: v.into(), notes: notes.into() })
    }

    #[test]
    fn notes_without_html() {
        assert_eq!(notes_text("<ul><li>Neu: Updates</li><li>A &amp; B</li></ul>"), "• Neu: Updates\n• A & B");
        assert!(notes_text(&"x".repeat(800)).ends_with(" …"));
        assert!(is_beta("0.3.0-beta.1"));
        assert!(!is_beta("0.3.0"));
        assert_eq!(release_url("o", "r", "1.0.0"), "https://github.com/o/r/releases/tag/v1.0.0");
    }

    #[tokio::test]
    async fn off_without_package() {
        let s = setup(None, vec![], false, false);
        assert_eq!(s.u.snapshot()["state"], "aus");
        s.u.check(false).await;
        assert!(s.fake.betas.lock().is_empty());
    }

    #[tokio::test]
    async fn offer_download_countdown_restart() {
        let s = setup(info("0.2.0", "<ul><li>Neu: Updates</li></ul>"), vec![0], false, true);
        s.u.check(false).await;
        {
            let d = s.dialogs.lock();
            assert_eq!(d.len(), 1);
            assert!(d[0]["message"].as_str().unwrap().contains("ASK 0.2.0 ist da"));
            assert!(d[0]["detail"].as_str().unwrap().contains("• Neu: Updates"));
            assert_eq!(d[0]["kind"], "update");
            assert_eq!(d[0]["notes"], "• Neu: Updates");
        }
        assert_eq!(s.fake.downloads.load(Ordering::SeqCst), 1);
        assert_eq!(s.u.snapshot()["state"], "neustart");
        assert_eq!(s.u.snapshot()["countdown"], RESTART_SECONDS);
        for _ in 0..RESTART_SECONDS {
            s.u.tick().await;
        }
        assert_eq!(s.u.snapshot()["state"], "installiert");
        assert_eq!(s.before.load(Ordering::SeqCst), 1);
        assert_eq!(s.fake.installs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn later_dismisses_until_manual() {
        let s = setup(info("0.2.0", ""), vec![1], false, true);
        s.u.check(false).await;
        s.u.check(false).await;
        assert_eq!(s.dialogs.lock().len(), 1);
        s.u.check(true).await;
        assert_eq!(s.dialogs.lock().len(), 2);
        assert_eq!(s.fake.downloads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn busy_defers_offer_and_restart() {
        let s = setup(info("0.2.0", ""), vec![0], false, true);
        s.busy.store(true, Ordering::SeqCst);
        s.u.check(false).await;
        assert!(s.dialogs.lock().is_empty());
        s.busy.store(false, Ordering::SeqCst);
        s.u.idle();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(s.u.snapshot()["state"], "neustart");
        s.busy.store(true, Ordering::SeqCst);
        s.u.tick().await;
        assert_eq!(s.u.snapshot()["state"], "geladen");
        s.busy.store(false, Ordering::SeqCst);
        s.u.idle();
        assert_eq!(s.u.snapshot()["state"], "neustart");
        s.u.postpone();
        assert_eq!(s.u.snapshot()["postponed"], true);
        assert!(s.u.install_on_quit().await);
    }

    #[tokio::test]
    async fn portable_only_links() {
        let s = setup(info("0.2.0", ""), vec![0], true, true);
        s.u.check(false).await;
        assert_eq!(s.opened.lock()[0], "https://github.com/KIYeager/ASK-Releases/releases/tag/v0.2.0");
        assert_eq!(s.fake.downloads.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn errors_only_reported_when_manual() {
        let s = setup(None, vec![], false, true);
        s.fake.fail.store(true, Ordering::SeqCst);
        s.u.check(false).await;
        assert!(s.dialogs.lock().is_empty());
        assert_eq!(s.u.snapshot()["error"], "net::ERR_INTERNET_DISCONNECTED");
        s.u.check(true).await;
        assert_eq!(s.dialogs.lock().len(), 1);
        s.fake.fail.store(false, Ordering::SeqCst);
        s.u.check(true).await;
        assert!(s.dialogs.lock()[1]["message"].as_str().unwrap().contains("neueste Version"));
    }
}
