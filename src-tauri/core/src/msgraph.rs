//! Microsoft 365 (OneDrive, Teams) für geteilte Docs per Link (ENTSCHEIDE E83). Die App meldet die Person einmal im
//! Browser bei Microsoft an (OAuth mit PKCE, Rückmeldung an http://localhost, wie Desktop-Apps von Microsoft) und
//! spricht danach Microsoft Graph an: Ordner `ASK Docs/<name>.askdoc` im eigenen OneDrive anlegen, einen Link zum
//! Bearbeiten für die Schule erzeugen (den man in Teams schickt) und über einen solchen Link geteilte Ordner anderer öffnen.
//!
//! Voraussetzung ist eine App-Registrierung in Microsoft Entra ID (Anleitung in docs/MICROSOFT.md). Ihre Kennung steht in
//! `resources/app-config.json` unter `microsoft.clientId` (dazu `tenant`, Standard «organizations»). Ohne Kennung gibt es
//! das Teilen per Link nicht, das Teilen über einen Ordner bleibt (E82).
//!
//! Das Refresh-Token liegt verschlüsselt mit dem Windows-Konto in den Einstellungen (`microsoft`), wie das GitHub-Token;
//! das Modell sieht es nie. Berechtigungen: `Files.ReadWrite.All` (auch Ordner, die andere geteilt haben),
//! `User.Read` (Name), `offline_access`.
//!
//! Öffentliche Schnittstelle: `config`, `configured`, `status`, `login(open)`, `logout`, `access_token`, `GraphStore`
//! (Ablage für docsync.rs), `create_shared`, `open_link`, `share_link_id`.

use crate::docsync::{Store, MAX_IMAGE};
use crate::settings;
use crate::util::{js_string, now_ms, str_of, take_chars};
use anyhow::{anyhow, bail, Result};
use base64::Engine;
use parking_lot::Mutex;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

const SCOPES: &str = "Files.ReadWrite.All User.Read offline_access";
/// Ordner im eigenen OneDrive für geteilte Docs
pub const ROOT: &str = "ASK Docs";
const SETTING: &str = "microsoft";

/// Einstellungen der App-Registrierung und Adressen (Tests: eigene Adressen über die Umgebung)
#[derive(Clone, Debug)]
pub struct MsConfig {
    pub client_id: String,
    pub tenant: String,
    pub login: String,
    pub graph: String,
}

pub fn config() -> MsConfig {
    let c = &settings::app_config()["microsoft"];
    let env = |k: &str, d: &str| std::env::var(k).ok().filter(|v| !v.is_empty()).unwrap_or_else(|| d.to_string());
    MsConfig {
        client_id: env("ASK_TEST_MS_CLIENT", &str_of(c, "clientId")),
        tenant: { let t = str_of(c, "tenant"); if t.is_empty() { "organizations".into() } else { t } },
        login: env("ASK_TEST_MS_LOGIN", "https://login.microsoftonline.com"),
        graph: env("ASK_TEST_GRAPH", "https://graph.microsoft.com/v1.0"),
    }
}

/// Ist eine App-Registrierung eingetragen?
pub fn configured() -> bool {
    !config().client_id.trim().is_empty()
}

struct Access {
    token: String,
    until: i64,
}

fn cache() -> &'static Mutex<Option<Access>> {
    static C: std::sync::OnceLock<Mutex<Option<Access>>> = std::sync::OnceLock::new();
    C.get_or_init(|| Mutex::new(None))
}

/// { eingerichtet, angemeldet, name }
pub fn status() -> Value {
    let s = settings::get(SETTING);
    json!({ "eingerichtet": configured(), "angemeldet": !str_of(&s, "refresh").is_empty(), "name": str_of(&s, "name") })
}

pub fn logout() {
    settings::set(json!({ SETTING: Value::Null }));
    *cache().lock() = None;
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut s = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(60)).build().expect("HTTP-Client")
}

async fn token_request(client: &reqwest::Client, pairs: &[(&str, &str)]) -> Result<Value> {
    let c = config();
    let res = client.post(format!("{}/{}/oauth2/v2.0/token", c.login, c.tenant))
        .header("Content-Type", "application/x-www-form-urlencoded").body(form(pairs)).send().await
        .map_err(|_| anyhow!("Microsoft ist gerade nicht erreichbar. Bist du online?"))?;
    let status = res.status().as_u16();
    let v: Value = res.json().await.unwrap_or(Value::Null);
    if status >= 400 || str_of(&v, "access_token").is_empty() {
        let msg = str_of(&v, "error_description");
        bail!("Microsoft hat die Anmeldung abgelehnt{}", if msg.is_empty() { String::new() } else { format!(": {}", take_chars(msg.lines().next().unwrap_or(""), 300)) });
    }
    Ok(v)
}

fn keep(v: &Value) {
    let expires = v["expires_in"].as_i64().unwrap_or(3600);
    *cache().lock() = Some(Access { token: str_of(v, "access_token"), until: now_ms() + (expires - 120).max(60) * 1000 });
    let refresh = str_of(v, "refresh_token");
    if !refresh.is_empty() {
        let mut s = settings::get(SETTING);
        if !s.is_object() {
            s = json!({});
        }
        s["refresh"] = json!(crate::secrets::protect(&refresh));
        settings::set(json!({ SETTING: s }));
    }
}

/// Gültiges Zugriffstoken (aus dem Zwischenspeicher oder mit dem Refresh-Token erneuert)
pub async fn access_token(client: &reqwest::Client) -> Result<String> {
    if let Some(a) = cache().lock().as_ref().filter(|a| a.until > now_ms()) {
        return Ok(a.token.clone());
    }
    let refresh = crate::secrets::unprotect(&str_of(&settings::get(SETTING), "refresh"));
    if refresh.is_empty() {
        bail!("Nicht bei Microsoft angemeldet. Im Doc bei «Teilen» auf «Mit Microsoft anmelden» klicken.");
    }
    let c = config();
    // Nur eine Ablehnung heisst «neu anmelden»; offline bleibt die Anmeldung gültig
    let v = token_request(client, &[("client_id", &c.client_id), ("grant_type", "refresh_token"), ("refresh_token", &refresh), ("scope", SCOPES)]).await
        .map_err(|e| if e.to_string().contains("abgelehnt") { anyhow!("{e} Bitte im Doc bei «Teilen» neu bei Microsoft anmelden.") } else { e })?;
    keep(&v);
    Ok(str_of(&v, "access_token"))
}

fn query_param(target: &str, key: &str) -> String {
    url::Url::parse(&format!("http://localhost{target}")).ok()
        .and_then(|u| u.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.to_string())).unwrap_or_default()
}

/// Anmeldung im Browser: `open(adresse)` öffnet die Seite von Microsoft; danach meldet der Browser den Code an
/// http://localhost:<port>. Liefert { name }.
pub async fn login(open: impl Fn(&str)) -> Result<Value> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let c = config();
    if c.client_id.trim().is_empty() {
        bail!("Teilen per Link ist noch nicht eingerichtet: Es fehlt die App-Registrierung bei Microsoft (siehe docs/MICROSOFT.md).");
    }
    let verifier = b64url(&rand::random::<[u8; 48]>());
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    let state = hex::encode(rand::random::<[u8; 12]>());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let redirect = format!("http://localhost:{}", listener.local_addr()?.port());
    let mut url = url::Url::parse(&format!("{}/{}/oauth2/v2.0/authorize", c.login, c.tenant))?;
    url.query_pairs_mut().append_pair("client_id", &c.client_id).append_pair("response_type", "code").append_pair("redirect_uri", &redirect)
        .append_pair("response_mode", "query").append_pair("scope", SCOPES).append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256").append_pair("state", &state).append_pair("prompt", "select_account");
    open(url.as_str());
    let code = tokio::time::timeout(Duration::from_secs(300), async {
        loop {
            let (mut sock, _) = listener.accept().await?;
            // Browser öffnen auf Vorrat leere Verbindungen: nicht ewig auf eine davon warten, sonst bliebe die echte Antwort liegen
            let mut buf = vec![0u8; 8192];
            let n = match tokio::time::timeout(Duration::from_secs(3), sock.read(&mut buf)).await {
                Ok(Ok(n)) => n,
                _ => continue,
            };
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let target = req.split_whitespace().nth(1).unwrap_or("/").to_string();
            let (code, got_state) = (query_param(&target, "code"), query_param(&target, "state"));
            let error = [query_param(&target, "error_description"), query_param(&target, "error")].into_iter().find(|e| !e.is_empty()).unwrap_or_default();
            if (code.is_empty() && error.is_empty()) || got_state != state {
                // z. B. favicon, oder eine Anfrage, die nicht zu dieser Anmeldung gehört: weiter warten
                let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                continue;
            }
            let ok = !code.is_empty();
            let page = if ok { "Angemeldet. Du kannst dieses Fenster schliessen und zu ASK zurückkehren." } else { "Die Anmeldung hat nicht geklappt. Schliess dieses Fenster und versuch es in ASK nochmals." };
            let html = format!("<!doctype html><meta charset=utf-8><title>ASK</title><body style=\"font:16px Segoe UI,sans-serif;padding:40px\"><h2>ASK</h2><p>{page}</p>");
            let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{html}", html.len()).as_bytes()).await;
            if !ok {
                bail!("Anmeldung abgebrochen oder abgelehnt (Microsoft: {}).", take_chars(error.lines().next().unwrap_or(""), 200));
            }
            return Ok::<String, anyhow::Error>(code);
        }
    }).await.map_err(|_| anyhow!("Keine Anmeldung innerhalb von 5 Minuten."))??;
    let http = client();
    let v = token_request(&http, &[("client_id", &c.client_id), ("grant_type", "authorization_code"), ("code", &code), ("redirect_uri", &redirect),
        ("code_verifier", &verifier), ("scope", SCOPES)]).await?;
    keep(&v);
    let me: Value = http.get(format!("{}/me", c.graph)).bearer_auth(str_of(&v, "access_token")).send().await?.json().await.unwrap_or(Value::Null);
    let name = [str_of(&me, "displayName"), str_of(&me, "mail"), str_of(&me, "userPrincipalName")].into_iter().find(|s| !s.is_empty()).unwrap_or_else(|| "Microsoft-Konto".into());
    let mut s = settings::get(SETTING);
    s["name"] = json!(take_chars(&name, 80));
    settings::set(json!({ SETTING: s }));
    Ok(json!({ "name": name }))
}

// ---------- Ablage in OneDrive für docsync.rs ----------

/// Zugriffstoken holen (in der App über `access_token`, in Tests fest)
pub type TokenFn = Arc<dyn Fn() -> Result<String> + Send + Sync>;

/// Token mit dem Zwischenspeicher und Refresh-Token, aus einem Thread ohne laufende async-Aufgabe
pub fn app_token(rt: tokio::runtime::Handle) -> TokenFn {
    Arc::new(move || rt.block_on(access_token(&client())))
}

fn enc(seg: &str) -> String {
    percent_encoding::utf8_percent_encode(seg, percent_encoding::NON_ALPHANUMERIC).to_string()
}

fn enc_path(path: &str) -> String {
    path.split('/').map(enc).collect::<Vec<_>>().join("/")
}

/// Ordner `….askdoc` in OneDrive oder SharePoint (auch von anderen geteilt), angesprochen über Laufwerk und Kennung
pub struct GraphStore {
    rt: tokio::runtime::Handle,
    http: reqwest::Client,
    graph: String,
    token: TokenFn,
    pub drive: String,
    pub folder: String,
    pub link: String,
    me: String,
    own: String,
    own_loaded: bool,
    /// Eigene Einträge, die noch nicht im geladenen Protokoll stehen (append geht nie übers Netz)
    pending: String,
    dirty: bool,
    image_sizes: HashMap<String, u64>,
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl GraphStore {
    pub fn new(rt: tokio::runtime::Handle, token: TokenFn, drive: &str, folder: &str, link: &str, me: &str) -> GraphStore {
        GraphStore { rt, http: client(), graph: config().graph, token, drive: drive.into(), folder: folder.into(), link: link.into(), me: me.into(),
            own: String::new(), own_loaded: false, pending: String::new(), dirty: false, image_sizes: HashMap::new() }
    }

    fn call(&self, method: reqwest::Method, url: &str, body: Option<Vec<u8>>, range: Option<u64>) -> Result<Reply> {
        let token = (self.token)()?;
        self.rt.block_on(async {
            let mut req = self.http.request(method, url).bearer_auth(token);
            if let Some(from) = range {
                req = req.header("Range", format!("bytes={from}-"));
            }
            if let Some(b) = body {
                req = req.header("Content-Type", "application/octet-stream").body(b);
            }
            let res = req.send().await.map_err(|e| anyhow!("OneDrive nicht erreichbar: {e}"))?;
            let status = res.status().as_u16();
            let body = res.bytes().await?.to_vec();
            if status == 401 {
                *cache().lock() = None; // Zugriffstoken abgelaufen oder widerrufen: beim nächsten Mal neu holen
                bail!("OneDrive hat die Anmeldung nicht angenommen. ASK versucht es gleich nochmals; sonst im Doc bei «Teilen» neu anmelden.");
            }
            if status == 403 {
                bail!("Kein Zugriff mehr auf das geteilte Doc in OneDrive. Vielleicht hat die Person, die geteilt hat, den Link entfernt.");
            }
            if status == 429 || status >= 500 {
                bail!("OneDrive ist gerade überlastet. ASK versucht es gleich nochmals.");
            }
            Ok(Reply { status, body })
        })
    }

    fn item(&self, rel: &str) -> String {
        format!("{}/drives/{}/items/{}:/{}:", self.graph, enc(&self.drive), enc(&self.folder), enc_path(rel))
    }

    fn children(&self, rel: &str) -> Result<Vec<Value>> {
        let mut url = format!("{}/children?$select=name,size&$top=999", self.item(rel));
        let mut out = Vec::new();
        // Mehr Einträge als auf eine Seite passen: den Links «@odata.nextLink» folgen (nur zu Graph)
        for _ in 0..20 {
            let r = self.call(reqwest::Method::GET, &url, None, None)?;
            if r.status == 404 {
                return Ok(out);
            }
            if r.status >= 300 {
                bail!("OneDrive antwortet mit Status {}.", r.status);
            }
            let v: Value = serde_json::from_slice(&r.body).unwrap_or(Value::Null);
            out.extend(v["value"].as_array().cloned().unwrap_or_default());
            match v["@odata.nextLink"].as_str() {
                Some(next) if next.starts_with(&self.graph) => url = next.to_string(),
                _ => break,
            }
        }
        Ok(out)
    }

    fn get(&self, rel: &str, from: Option<u64>) -> Result<Option<Vec<u8>>> {
        let r = self.call(reqwest::Method::GET, &format!("{}/content", self.item(rel)), None, from.filter(|f| *f > 0))?;
        match r.status {
            404 => Ok(None),
            416 => Ok(Some(vec![])),
            200 => Ok(Some(match from { Some(f) if f > 0 => r.body.get(f as usize..).map(<[u8]>::to_vec).unwrap_or_default(), _ => r.body })),
            206 => Ok(Some(r.body)),
            s => bail!("OneDrive antwortet mit Status {s}."),
        }
    }

    fn put(&self, rel: &str, bytes: Vec<u8>) -> Result<Value> {
        self.put_to(&format!("{}/content", self.item(rel)), bytes)
    }

    fn put_to(&self, url: &str, bytes: Vec<u8>) -> Result<Value> {
        let r = self.call(reqwest::Method::PUT, url, Some(bytes), None)?;
        if r.status >= 300 {
            bail!("Speichern in OneDrive fehlgeschlagen (Status {}).", r.status);
        }
        Ok(serde_json::from_slice(&r.body).unwrap_or(Value::Null))
    }

    /// Titel aus doc.json
    pub fn title(&self) -> Result<String> {
        let meta = self.get("doc.json", None)?.ok_or_else(|| anyhow!("Unter diesem Link liegt kein geteiltes Doc von ASK (doc.json fehlt)."))?;
        Ok(str_of(&serde_json::from_slice::<Value>(&meta).unwrap_or(Value::Null), "titel"))
    }

    fn load_own(&mut self) -> Result<()> {
        if !self.own_loaded {
            self.own = self.get(&format!("teilnehmer/{}.jsonl", self.me), None)?.map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
            self.own_loaded = true;
        }
        Ok(())
    }
}

impl Store for GraphStore {
    fn describe(&self) -> String {
        "OneDrive (Link)".into()
    }
    fn link(&self) -> Option<String> {
        Some(self.link.clone())
    }
    fn check(&mut self) -> Result<()> {
        self.title().map(|_| ())
    }
    fn logs(&mut self) -> Result<Vec<(String, u64)>> {
        Ok(self.children("teilnehmer")?.iter().filter_map(|e| {
            let name = str_of(e, "name");
            name.strip_suffix(".jsonl").map(|w| (w.to_string(), e["size"].as_u64().unwrap_or(0)))
        }).collect())
    }
    fn read_log(&mut self, who: &str, from: u64) -> Result<Vec<u8>> {
        Ok(self.get(&format!("teilnehmer/{who}.jsonl"), Some(from))?.unwrap_or_default())
    }
    fn append(&mut self, text: &str) -> Result<()> {
        // Ohne Netz: Speichern aus dem Editor oder von ASK wartet nie auf OneDrive; hochgeladen wird beim Abgleich
        self.pending.push_str(text);
        self.dirty = true;
        Ok(())
    }
    fn flush(&mut self) -> Result<()> {
        if self.dirty {
            self.load_own()?;
            if !self.pending.is_empty() {
                let pending = std::mem::take(&mut self.pending);
                self.own.push_str(&pending);
            }
            // Die eigene Datei schreibt nur diese App: ganz hochladen, nie ein Konflikt mit anderen
            self.put(&format!("teilnehmer/{}.jsonl", self.me), self.own.clone().into_bytes())?;
            self.dirty = false;
        }
        Ok(())
    }
    fn images(&mut self) -> Result<Vec<String>> {
        let list = self.children("bilder")?;
        self.image_sizes = list.iter().map(|e| (str_of(e, "name"), e["size"].as_u64().unwrap_or(0))).collect();
        Ok(self.image_sizes.keys().cloned().collect())
    }
    fn get_image(&mut self, name: &str) -> Result<Vec<u8>> {
        // Zu grosse Bilder gar nicht erst laden
        if self.image_sizes.get(name).is_some_and(|n| *n > MAX_IMAGE) {
            bail!("Bild zu gross");
        }
        self.get(&format!("bilder/{name}"), None)?.ok_or_else(|| anyhow!("Bild fehlt"))
    }
    fn put_image(&mut self, name: &str, bytes: Vec<u8>) -> Result<()> {
        // Nie ein Bild überschreiben, das jemand anderes inzwischen mit demselben Namen hochgeladen hat
        match self.put_to(&format!("{}/content?@microsoft.graph.conflictBehavior=fail", self.item(&format!("bilder/{name}"))), bytes) {
            Err(e) if e.to_string().contains("Status 409") => Ok(()),
            r => r.map(|_| ()),
        }
    }
}

/// Neues geteiltes Doc im eigenen OneDrive: Ordner `ASK Docs/<name>.askdoc` mit doc.json, dazu ein Link zum Bearbeiten
/// für die Schule. Blockiert (aus einem Thread ohne laufende async-Aufgabe aufrufen).
pub fn create_shared(rt: tokio::runtime::Handle, token: TokenFn, slug: &str, title: &str, me: &str) -> Result<GraphStore> {
    let graph = config().graph;
    let http = client();
    let call = |method: reqwest::Method, url: String, body: Option<Vec<u8>>, json_body: bool| -> Result<(u16, Value)> {
        let t = token()?;
        rt.block_on(async {
            let mut req = http.request(method, &url).bearer_auth(t);
            if let Some(b) = body {
                req = req.header("Content-Type", if json_body { "application/json" } else { "application/octet-stream" }).body(b);
            }
            let res = req.send().await.map_err(|e| anyhow!("OneDrive nicht erreichbar: {e}"))?;
            let status = res.status().as_u16();
            Ok((status, res.json().await.unwrap_or(Value::Null)))
        })
    };
    // Freier Name im Ordner «ASK Docs»: nur 404 heisst frei; ein Fehler (offline, Anmeldung) bricht ab
    let mut name = None;
    for n in 1..100 {
        let candidate = if n == 1 { format!("{slug}.askdoc") } else { format!("{slug}-{n}.askdoc") };
        let (status, _) = call(reqwest::Method::GET, format!("{graph}/me/drive/root:/{}/{}", enc(ROOT), enc(&candidate)), None, false)?;
        match status {
            404 => { name = Some(candidate); break; }
            200..=299 => continue,
            401 | 403 => bail!("OneDrive hat den Zugriff abgelehnt. Melde dich im Doc bei «Teilen» neu bei Microsoft an."),
            s => bail!("OneDrive antwortet mit Status {s}. Versuch es gleich nochmals."),
        }
    }
    let name = name.ok_or_else(|| anyhow!("Kein freier Name in OneDrive unter «{ROOT}»."))?;
    let meta = serde_json::to_vec_pretty(&json!({ "format": 1, "titel": title, "erstellt": now_ms(), "app": "ASK" }))?;
    let (status, item) = call(reqwest::Method::PUT, format!("{graph}/me/drive/root:/{}/{}/doc.json:/content", enc(ROOT), enc(&name)), Some(meta), false)?;
    if status >= 300 {
        bail!("Das Doc liess sich nicht in OneDrive anlegen (Status {status}).");
    }
    let drive = js_string(&item["parentReference"]["driveId"]);
    let folder = js_string(&item["parentReference"]["id"]);
    if drive.is_empty() || folder.is_empty() {
        bail!("OneDrive hat keine Ordnerkennung geliefert.");
    }
    let (status, link) = call(reqwest::Method::POST, format!("{graph}/drives/{}/items/{}/createLink", enc(&drive), enc(&folder)),
        Some(json!({ "type": "edit", "scope": "organization" }).to_string().into_bytes()), true)?;
    let url = str_of(&link["link"], "webUrl");
    if status >= 300 || url.is_empty() {
        bail!("OneDrive hat keinen Link erstellt (Status {status}). Vielleicht erlaubt die Schule keine Links für alle in der Organisation.");
    }
    Ok(GraphStore::new(rt, token, &drive, &folder, &url, me))
}

/// Kennung für die Graph-Schnittstelle `shares/{id}` aus einem Link (u! + Base64url ohne =)
pub fn share_link_id(link: &str) -> String {
    format!("u!{}", b64url(link.trim().as_bytes()))
}

/// Geteiltes Doc über einen Link öffnen (Link aus Teams, OneDrive oder SharePoint). Blockiert.
pub fn open_link(rt: tokio::runtime::Handle, token: TokenFn, link: &str, me: &str) -> Result<GraphStore> {
    let link = link.trim();
    if !link.starts_with("https://") {
        bail!("Das ist kein Link. Kopiere den ganzen Link aus Teams (beginnt mit https://).");
    }
    let graph = config().graph;
    let t = token()?;
    let http = client();
    let (status, item) = rt.block_on(async {
        let res = http.get(format!("{graph}/shares/{}/driveItem", share_link_id(link))).bearer_auth(t).header("Prefer", "redeemSharingLinkIfNecessary").send().await
            .map_err(|e| anyhow!("OneDrive nicht erreichbar: {e}"))?;
        let s = res.status().as_u16();
        Ok::<_, anyhow::Error>((s, res.json::<Value>().await.unwrap_or(Value::Null)))
    })?;
    if status == 403 || status == 404 {
        bail!("Kein Zugriff über diesen Link (Status {status}). Bitte die Person, die geteilt hat, um einen neuen Link für die Schule.");
    }
    if status >= 300 {
        bail!("OneDrive antwortet mit Status {status}.");
    }
    let drive = js_string(&item["parentReference"]["driveId"]);
    let folder = str_of(&item, "id");
    if !str_of(&item, "name").ends_with(".askdoc") || item.get("folder").is_none() && item.get("file").is_some() {
        bail!("Der Link zeigt nicht auf ein geteiltes Doc von ASK (Ordner «….askdoc»).");
    }
    let mut store = GraphStore::new(rt, token, &drive, &folder, link, me);
    store.check()?;
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kennung_eines_links_wie_graph_sie_will() {
        // Beispiel aus der Dokumentation von Microsoft: u! + Base64url ohne = am Ende
        assert_eq!(share_link_id("https://onedrive.live.com/redir?resid=1231244193912!12&authKey=1201919!12921!1"),
            "u!aHR0cHM6Ly9vbmVkcml2ZS5saXZlLmNvbS9yZWRpcj9yZXNpZD0xMjMxMjQ0MTkzOTEyITEyJmF1dGhLZXk9MTIwMTkxOSExMjkyMSEx");
        assert_eq!(query_param("/?code=abc%20d&state=x", "code"), "abc d");
        assert_eq!(query_param("/favicon.ico", "code"), "");
        assert_eq!(enc_path("teilnehmer/a b.jsonl"), "teilnehmer/a%20b%2Ejsonl");
        assert_eq!(form(&[("a", "x y"), ("b", "ä&")]), "a=x+y&b=%C3%A4%26");
    }

    #[tokio::test]
    async fn ohne_app_registrierung_keine_anmeldung() {
        if configured() {
            return; // eine Kennung ist eingetragen (app-config.json): dann gilt dieser Fall nicht
        }
        let err = login(|_| panic!("öffnet keinen Browser")).await.unwrap_err().to_string();
        assert!(err.contains("noch nicht eingerichtet"), "{err}");
    }
}
