//! HTTP-Server des Terminals (Ersatz für FastAPI/uvicorn aus open-terminal 0.14.2).
//!
//! Nur die Endpunkte, die die App braucht, mit denselben Anfragen, Antworten und Statuscodes:
//! `GET /health` (ohne Anmeldung), `GET /ask/info`, `POST /execute`, `GET /execute/{id}/status`,
//! `DELETE /execute/{id}`, `GET /files/list|read|glob|grep`, `POST /files/write|replace`. Dazu, nur in ASK, die Quellen
//! (ENTSCHEIDE E84): `GET /quellen/search|read|list`.
//! Anmeldung: `Authorization: Bearer <Key>` (konstante Zeit). Fehler als `{"detail": …}`.
//!
//! Öffentliche Schnittstelle:
//! - `serve(config) -> ServerHandle`: bindet 127.0.0.1 und beantwortet Anfragen im Hintergrund
//! - `ServerHandle::addr()`, `shutdown()`

use std::collections::HashMap;
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{HeaderValue, AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};

use crate::files::{ApiError, FileApi, ReadResult, Replacement};
use crate::runner::Registry;
use crate::sources::Sources;
use crate::Config;

/// Grösse einer Anfrage (Dateiinhalt zum Schreiben)
const MAX_BODY: usize = 256 * 1024 * 1024;

struct State {
    key: Vec<u8>,
    port: u16,
    origin: String,
    execute_timeout: f64,
    files: Arc<FileApi>,
    sources: Arc<Sources>,
    registry: Arc<Registry>,
    integrity: String,
    low_integrity: bool,
}

/// Der laufende Server
pub struct ServerHandle {
    addr: SocketAddr,
    registry: Arc<Registry>,
    stop: tokio::sync::watch::Sender<bool>,
}

impl ServerHandle {
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Hört auf zu antworten und beendet alle Befehle samt Prozessbäumen
    pub fn shutdown(&self) {
        let _ = self.stop.send(true);
        self.registry.kill_all();
    }

    pub fn registry(&self) -> &Arc<Registry> {
        &self.registry
    }
}

/// Bindet 127.0.0.1:port (nie eine andere Adresse) und startet den Server
pub async fn serve(config: Config) -> std::io::Result<ServerHandle> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", config.port)).await?;
    let addr = listener.local_addr()?;
    let (integrity, low_integrity) = integrity_of(&config);
    let registry = Registry::new();
    let state = Arc::new(State {
        key: config.key.clone().into_bytes(),
        port: addr.port(),
        origin: config.origin.clone(),
        execute_timeout: config.execute_timeout,
        files: Arc::new(FileApi::new(std::path::Path::new(&config.workdir)).with_read_only(&config.read_only)),
        sources: Arc::new(Sources::new()),
        registry: registry.clone(),
        integrity,
        low_integrity,
    });
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    // Fertige Befehle (mit bis zu 50 MB Ausgabe) auch dann vergessen, wenn danach lange kein Aufruf mehr kommt
    {
        let (registry, mut tick_stop) = (registry.clone(), stop_rx.clone());
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tokio::select! {
                    _ = tick.tick() => registry.cleanup_expired(),
                    _ = tick_stop.changed() => break,
                }
            }
        });
    }
    tokio::spawn(async move {
        let mut accept_stop = stop_rx.clone();
        loop {
            let accepted = tokio::select! {
                r = listener.accept() => r,
                _ = accept_stop.changed() => break,
            };
            let Ok((stream, peer)) = accepted else { continue };
            if !peer.ip().is_loopback() {
                continue; // nie von aussen
            }
            let state = state.clone();
            let mut conn_stop = stop_rx.clone();
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| handle(state.clone(), req));
                let conn = hyper::server::conn::http1::Builder::new().keep_alive(true).serve_connection(TokioIo::new(stream), service);
                tokio::select! {
                    _ = conn => {}
                    _ = conn_stop.changed() => {}
                }
            });
        }
    });
    Ok(ServerHandle { addr, registry, stop })
}

/// Integritätsstufe des Servers und ob sie «Low» ist. Unter Windows kommt sie vom echten Token; sonst
/// (Entwicklung und Tests ohne Integritätsstufen) gilt die Angabe `low_integrity` der Einstellungen.
pub fn integrity_of(config: &Config) -> (String, bool) {
    #[cfg(windows)]
    {
        let _ = config;
        match crate::winproc::current_integrity_sid() {
            Ok(sid) => {
                let low = sid == crate::winproc::LOW_SID;
                (sid, low)
            }
            Err(_) => ("unbekannt".to_string(), false),
        }
    }
    #[cfg(not(windows))]
    {
        ("nicht-windows".to_string(), config.low_integrity)
    }
}

// ---------- Antworten ----------

fn json_response(status: u16, value: &Value) -> Response<Full<Bytes>> {
    let body = serde_json::to_vec(value).unwrap_or_default();
    let mut res = Response::new(Full::new(Bytes::from(body)));
    *res.status_mut() = StatusCode::from_u16(status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    res
}

fn error_response(e: &ApiError) -> Response<Full<Bytes>> {
    json_response(e.status, &json!({ "detail": e.detail }))
}

fn validation_error(location: &str, name: &str, kind: &str, msg: &str, input: Value) -> ApiError {
    ApiError { status: 422, detail: json!([{ "type": kind, "loc": [location, name], "msg": msg, "input": input }]) }
}

fn missing(location: &str, name: &str) -> ApiError {
    validation_error(location, name, "missing", "Field required", Value::Null)
}

/// Vergleich in konstanter Zeit (wie `hmac.compare_digest`)
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    let mut diff = (a.len() ^ b.len()) as u32;
    for i in 0..a.len().max(b.len()) {
        diff |= (a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0)) as u32;
    }
    diff == 0
}

// ---------- Anfragen ----------

struct Query(Vec<(String, String)>);

impl Query {
    fn parse(raw: Option<&str>) -> Query {
        // wie NormalizeNullQueryMiddleware: Parameter mit dem Wert «null» gelten als nicht gesetzt
        let pairs = url::form_urlencoded::parse(raw.unwrap_or("").as_bytes())
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .filter(|(_, v)| !v.eq_ignore_ascii_case("null"))
            .collect();
        Query(pairs)
    }

    fn get(&self, name: &str) -> Option<&str> {
        self.0.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn all(&self, name: &str) -> Vec<String> {
        self.0.iter().filter(|(k, _)| k == name).map(|(_, v)| v.clone()).collect()
    }

    fn required(&self, name: &str) -> Result<&str, ApiError> {
        self.get(name).ok_or_else(|| missing("query", name))
    }

    fn string(&self, name: &str, default: &str) -> String {
        self.get(name).unwrap_or(default).to_string()
    }

    fn int(&self, name: &str, min: Option<i64>, max: Option<i64>) -> Result<Option<i64>, ApiError> {
        let Some(raw) = self.get(name) else { return Ok(None) };
        let value: i64 = raw.trim().parse().map_err(|_| {
            validation_error("query", name, "int_parsing", "Input should be a valid integer, unable to parse string as an integer", json!(raw))
        })?;
        if let Some(min) = min {
            if value < min {
                return Err(validation_error("query", name, "greater_than_equal", &format!("Input should be greater than or equal to {min}"), json!(raw)));
            }
        }
        if let Some(max) = max {
            if value > max {
                return Err(validation_error("query", name, "less_than_equal", &format!("Input should be less than or equal to {max}"), json!(raw)));
            }
        }
        Ok(Some(value))
    }

    fn float(&self, name: &str, min: f64, max: f64) -> Result<Option<f64>, ApiError> {
        let Some(raw) = self.get(name) else { return Ok(None) };
        let value: f64 = raw.trim().parse().ok().filter(|v: &f64| v.is_finite()).ok_or_else(|| {
            validation_error("query", name, "float_parsing", "Input should be a valid number, unable to parse string as a number", json!(raw))
        })?;
        if value < min {
            return Err(validation_error("query", name, "greater_than_equal", &format!("Input should be greater than or equal to {min}"), json!(raw)));
        }
        if value > max {
            return Err(validation_error("query", name, "less_than_equal", &format!("Input should be less than or equal to {max}"), json!(raw)));
        }
        Ok(Some(value))
    }

    fn bool(&self, name: &str, default: bool) -> Result<bool, ApiError> {
        let Some(raw) = self.get(name) else { return Ok(default) };
        match raw.trim().to_lowercase().as_str() {
            "1" | "true" | "t" | "yes" | "y" | "on" => Ok(true),
            "0" | "false" | "f" | "no" | "n" | "off" => Ok(false),
            _ => Err(validation_error("query", name, "bool_parsing", "Input should be a valid boolean, unable to interpret input", json!(raw))),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Route {
    Health,
    Info,
    Execute,
    Status,
    Kill,
    List,
    Read,
    Write,
    Replace,
    Glob,
    Grep,
    SourcesSearch,
    SourcesRead,
    SourcesList,
}

/// Route zu Methode und Pfad; `Err(true)`: Pfad bekannt, Methode nicht (405), `Err(false)`: unbekannt (404)
fn route(method: &Method, path: &str) -> Result<(Route, Option<String>), bool> {
    let table: [(&str, Method, Route); 12] = [
        ("/ask/info", Method::GET, Route::Info),
        ("/execute", Method::POST, Route::Execute),
        ("/files/list", Method::GET, Route::List),
        ("/files/read", Method::GET, Route::Read),
        ("/files/write", Method::POST, Route::Write),
        ("/files/replace", Method::POST, Route::Replace),
        ("/files/glob", Method::GET, Route::Glob),
        ("/files/grep", Method::GET, Route::Grep),
        ("/quellen/search", Method::GET, Route::SourcesSearch),
        ("/quellen/read", Method::GET, Route::SourcesRead),
        ("/quellen/list", Method::GET, Route::SourcesList),
        ("/health", Method::GET, Route::Health),
    ];
    if let Some(rest) = path.strip_prefix("/execute/") {
        let decode = |s: &str| percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned();
        if let Some(id) = rest.strip_suffix("/status") {
            if !id.is_empty() && !id.contains('/') {
                return if method == Method::GET { Ok((Route::Status, Some(decode(id)))) } else { Err(true) };
            }
        } else if !rest.is_empty() && !rest.contains('/') {
            return if method == Method::DELETE { Ok((Route::Kill, Some(decode(rest)))) } else { Err(true) };
        }
        return Err(false);
    }
    for (p, m, r) in table.iter() {
        if *p == path {
            return if m == method { Ok((*r, None)) } else { Err(true) };
        }
    }
    Err(false)
}

async fn handle(state: Arc<State>, req: Request<Incoming>) -> Result<Response<Full<Bytes>>, Infallible> {
    let origin = req.headers().get(ORIGIN).and_then(|v| v.to_str().ok()).map(str::to_string);
    let allowed_origin = origin.as_deref().filter(|o| !state.origin.is_empty() && *o == state.origin).map(str::to_string);
    let mut res = dispatch(&state, req, allowed_origin.is_some()).await;
    if let Some(o) = allowed_origin {
        let h = res.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&o) {
            h.insert("access-control-allow-origin", v);
        }
        h.insert("access-control-allow-credentials", HeaderValue::from_static("true"));
        h.append("vary", HeaderValue::from_static("Origin"));
    }
    Ok(res)
}

fn host_allowed(host: &str, port: u16) -> bool {
    let (name, given) = match host.rsplit_once(':') {
        Some((n, p)) if !n.ends_with(']') || n.starts_with('[') => (n, p.parse::<u16>().ok()),
        _ => (host, None),
    };
    // Schutz vor DNS-Rebinding: nur Adressen dieses Rechners
    matches!(name, "127.0.0.1" | "localhost" | "[::1]") && given.is_none_or(|p| p == port)
}

async fn dispatch(state: &Arc<State>, req: Request<Incoming>, origin_ok: bool) -> Response<Full<Bytes>> {
    let (parts, body) = req.into_parts();
    if let Some(host) = parts.headers.get(HOST).and_then(|v| v.to_str().ok()) {
        if !host_allowed(host, state.port) {
            return error_response(&ApiError::new(400, "Invalid host header"));
        }
    }
    // CORS-Vorabfrage (Preflight): nur für den ASK-Origin, mit erlaubtem Zugriff auf das lokale Netz
    if parts.method == Method::OPTIONS && parts.headers.contains_key("access-control-request-method") && parts.headers.contains_key(ORIGIN) {
        if !origin_ok {
            let mut res = Response::new(Full::new(Bytes::from_static(b"Disallowed CORS origin")));
            *res.status_mut() = StatusCode::BAD_REQUEST;
            return res;
        }
        let mut res = Response::new(Full::new(Bytes::from_static(b"OK")));
        let h = res.headers_mut();
        h.insert("access-control-allow-methods", HeaderValue::from_static("DELETE, GET, HEAD, OPTIONS, PATCH, POST, PUT"));
        h.insert("access-control-max-age", HeaderValue::from_static("600"));
        if let Some(requested) = parts.headers.get("access-control-request-headers") {
            h.insert("access-control-allow-headers", requested.clone());
        }
        if parts.headers.get("access-control-request-private-network").is_some_and(|v| v == "true") {
            h.insert("access-control-allow-private-network", HeaderValue::from_static("true"));
        }
        return res;
    }
    let (route, id) = match route(&parts.method, parts.uri.path()) {
        Ok(r) => r,
        Err(method_not_allowed) => {
            return if method_not_allowed { error_response(&ApiError::new(405, "Method Not Allowed")) } else { error_response(&ApiError::new(404, "Not Found")) };
        }
    };
    if route == Route::Health {
        return json_response(200, &json!({ "status": "ok" }));
    }
    // Anmeldung vor allem anderen (auch vor der Prüfung der Eingaben, wie bei FastAPI)
    let token = parts.headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| {
        let (scheme, param) = v.split_once(' ').unwrap_or((v, ""));
        scheme.eq_ignore_ascii_case("bearer").then(|| param.trim().to_string()).filter(|p| !p.is_empty())
    });
    if !token.is_some_and(|t| constant_time_eq(t.as_bytes(), &state.key)) {
        return error_response(&ApiError::new(401, "Invalid API key"));
    }
    let query = Query::parse(parts.uri.query());
    let body = match Limited::new(body, MAX_BODY).collect().await {
        Ok(b) => b.to_bytes(),
        Err(_) => return error_response(&ApiError::new(413, "Request Entity Too Large")),
    };
    match run(state, route, id, query, body).await {
        Ok(res) => res,
        Err(e) => error_response(&e),
    }
}

fn json_body(body: &[u8]) -> Result<serde_json::Map<String, Value>, ApiError> {
    if body.is_empty() {
        return Err(ApiError { status: 422, detail: json!([{ "type": "missing", "loc": ["body"], "msg": "Field required", "input": null }]) });
    }
    match serde_json::from_slice::<Value>(body) {
        Ok(Value::Object(m)) => Ok(m),
        Ok(other) => Err(ApiError { status: 422, detail: json!([{ "type": "model_attributes_type", "loc": ["body"], "msg": "Input should be a valid dictionary or object to extract fields from", "input": other }]) }),
        Err(e) => Err(ApiError { status: 422, detail: json!([{ "type": "json_invalid", "loc": ["body"], "msg": "JSON decode error", "input": {}, "ctx": { "error": e.to_string() } }]) }),
    }
}

fn body_string(m: &serde_json::Map<String, Value>, name: &str) -> Result<String, ApiError> {
    match m.get(name) {
        Some(Value::String(s)) => Ok(s.clone()),
        Some(other) => Err(validation_error("body", name, "string_type", "Input should be a valid string", other.clone())),
        None => Err(missing("body", name)),
    }
}

fn body_opt_string(m: &serde_json::Map<String, Value>, name: &str) -> Result<Option<String>, ApiError> {
    match m.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(other) => Err(validation_error("body", name, "string_type", "Input should be a valid string", other.clone())),
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, ApiError> + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f).await.map_err(|_| ApiError::new(500, "Internal Server Error"))?
}

async fn run(state: &Arc<State>, route: Route, id: Option<String>, q: Query, body: Bytes) -> Result<Response<Full<Bytes>>, ApiError> {
    let files = state.files.clone();
    match route {
        Route::Health => Ok(json_response(200, &json!({ "status": "ok" }))),
        Route::Info => Ok(json_response(
            200,
            &json!({
                "version": env!("CARGO_PKG_VERSION"),
                "workdir": files.home().to_string_lossy(),
                "low_integrity": state.low_integrity,
                "integrity_sid": state.integrity,
                "pid": std::process::id(),
            }),
        )),
        Route::List => {
            let dir = q.string("directory", ".");
            Ok(json_response(200, &blocking(move || files.list(&dir)).await?))
        }
        Route::Read => {
            let path = q.required("path")?.to_string();
            let start = q.int("start_line", Some(1), None)?.map(|v| v as u32);
            let end = q.int("end_line", Some(1), None)?.map(|v| v as u32);
            match blocking(move || files.read(&path, start, end)).await? {
                ReadResult::Json(v) => Ok(json_response(200, &v)),
                ReadResult::Raw { bytes, mime } => {
                    let mut res = Response::new(Full::new(Bytes::from(bytes)));
                    res.headers_mut().insert(CONTENT_TYPE, HeaderValue::from_str(&mime).unwrap_or(HeaderValue::from_static("application/octet-stream")));
                    Ok(res)
                }
            }
        }
        Route::Write => {
            let m = json_body(&body)?;
            let (path, content) = (body_string(&m, "path")?, body_string(&m, "content")?);
            Ok(json_response(200, &blocking(move || files.write(&path, &content)).await?))
        }
        Route::Replace => {
            let m = json_body(&body)?;
            let path = body_string(&m, "path")?;
            let list = match m.get("replacements") {
                Some(Value::Array(a)) => a.clone(),
                Some(other) => return Err(validation_error("body", "replacements", "list_type", "Input should be a valid list", other.clone())),
                None => return Err(missing("body", "replacements")),
            };
            let mut replacements = Vec::new();
            for item in list {
                let Value::Object(c) = item else { return Err(validation_error("body", "replacements", "model_type", "Input should be a valid dictionary or object", item)) };
                let line = |name: &str| -> Result<Option<u32>, ApiError> {
                    match c.get(name) {
                        None | Some(Value::Null) => Ok(None),
                        Some(Value::Number(n)) if n.as_u64().is_some_and(|v| v >= 1) => Ok(n.as_u64().map(|v| v as u32)),
                        Some(other) => Err(validation_error("body", name, "greater_than_equal", "Input should be a valid integer greater than or equal to 1", other.clone())),
                    }
                };
                replacements.push(Replacement {
                    target: body_string(&c, "target")?,
                    replacement: body_string(&c, "replacement")?,
                    start_line: line("start_line")?,
                    end_line: line("end_line")?,
                    allow_multiple: c.get("allow_multiple").and_then(Value::as_bool).unwrap_or(false),
                });
            }
            Ok(json_response(200, &blocking(move || files.replace(&path, &replacements)).await?))
        }
        Route::Glob => {
            let pattern = q.required("pattern")?.to_string();
            let path = q.string("path", ".");
            let exclude = q.all("exclude");
            let kind = q.string("type", "any");
            if !matches!(kind.as_str(), "file" | "directory" | "any") {
                return Err(validation_error("query", "type", "string_pattern_mismatch", "String should match pattern '^(file|directory|any)$'", json!(kind)));
            }
            let max = q.int("max_results", Some(1), Some(500))?.unwrap_or(50) as usize;
            Ok(json_response(200, &blocking(move || files.glob(&pattern, &path, &exclude, &kind, max)).await?))
        }
        Route::Grep => {
            let query = q.required("query")?.to_string();
            let path = q.string("path", ".");
            let regex = q.bool("regex", true)?;
            let ci = q.bool("case_insensitive", false)?;
            let include = q.all("include");
            let per_line = q.bool("match_per_line", true)?;
            let max = q.int("max_results", Some(1), Some(500))?.unwrap_or(50) as usize;
            Ok(json_response(200, &blocking(move || files.grep(&query, &path, regex, ci, &include, per_line, max)).await?))
        }
        Route::SourcesSearch => {
            let query = q.required("query")?.to_string();
            let limit = q.int("limit", Some(1), Some(20))?.unwrap_or(8) as usize;
            let sources = state.sources.clone();
            Ok(json_response(200, &blocking(move || Ok(sources.search(&files, &query, limit))).await?))
        }
        Route::SourcesRead => {
            let path = q.required("path")?.to_string();
            let seite = q.int("seite", Some(1), None)?.map(|v| v as u32);
            let ort = q.string("ort", "");
            let sources = state.sources.clone();
            Ok(json_response(200, &blocking(move || sources.read(&files, &path, seite, Some(ort.as_str()))).await?))
        }
        Route::SourcesList => {
            let sources = state.sources.clone();
            Ok(json_response(200, &blocking(move || Ok(sources.list(&files))).await?))
        }
        Route::Execute => execute(state, q, &body).await,
        Route::Status => {
            state.registry.cleanup_expired();
            let process = state.registry.get(&id.unwrap_or_default()).ok_or_else(|| ApiError::new(404, "Process not found"))?;
            let wait = q.float("wait", 0.0, 300.0)?;
            let offset = q.int("offset", Some(0), None)?.unwrap_or(0) as usize;
            let tail = q.int("tail", Some(1), None)?.map(|v| v as usize);
            let wait = wait.or((state.execute_timeout > 0.0).then_some(state.execute_timeout));
            if let (Some(w), true) = (wait, process.is_running()) {
                process.wait_done(Duration::from_secs_f64(w)).await;
            }
            Ok(json_response(200, &process.response(false, offset, tail)))
        }
        Route::Kill => {
            state.registry.cleanup_expired();
            let process = state.registry.get(&id.unwrap_or_default()).ok_or_else(|| ApiError::new(404, "Process not found"))?;
            let force = q.bool("force", false)?;
            process.kill_and_wait(force).await;
            Ok(json_response(200, &json!({ "status": "killed" })))
        }
    }
}

async fn execute(state: &Arc<State>, q: Query, body: &[u8]) -> Result<Response<Full<Bytes>>, ApiError> {
    let wait = q.float("wait", 0.0, 300.0)?;
    let tail = q.int("tail", Some(1), None)?.map(|v| v as usize);
    let m = json_body(body)?;
    let command = body_string(&m, "command")?;
    let cwd = body_opt_string(&m, "cwd")?;
    let env: Option<HashMap<String, String>> = match m.get("env") {
        None | Some(Value::Null) => None,
        Some(Value::Object(o)) => {
            let mut map = HashMap::new();
            for (k, v) in o {
                match v {
                    Value::String(s) => map.insert(k.clone(), s.clone()),
                    other => return Err(validation_error("body", "env", "string_type", "Input should be a valid string", other.clone())),
                };
            }
            Some(map).filter(|m| !m.is_empty())
        }
        Some(other) => return Err(validation_error("body", "env", "dict_type", "Input should be a valid dictionary", other.clone())),
    };
    state.registry.cleanup_expired();
    let target = match cwd.filter(|c| !c.is_empty()) {
        Some(c) => state.files.resolve(&c)?,
        None => state.files.home().to_path_buf(),
    };
    let process = state
        .registry
        .spawn(&command, &target, env.as_ref())
        .await
        .map_err(|e| ApiError::new(400, crate::files::os_error_text(&e, &target)))?;
    let wait = wait.or((state.execute_timeout > 0.0).then_some(state.execute_timeout));
    if let Some(w) = wait {
        process.wait_done(Duration::from_secs_f64(w)).await;
    }
    Ok(json_response(200, &process.response(true, 0, tail)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schluesselvergleich() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn hostpruefung() {
        assert!(host_allowed("127.0.0.1:2000", 2000));
        assert!(host_allowed("localhost", 2000));
        assert!(!host_allowed("127.0.0.1:2001", 2000));
        assert!(!host_allowed("evil.example:2000", 2000));
    }

    #[test]
    fn null_parameter_gelten_als_nicht_gesetzt() {
        let q = Query::parse(Some("path=a.txt&start_line=null&end_line=NULL"));
        assert_eq!(q.get("path"), Some("a.txt"));
        assert_eq!(q.int("start_line", Some(1), None).unwrap(), None);
    }

    #[test]
    fn routen() {
        assert_eq!(route(&Method::GET, "/files/read").unwrap().0, Route::Read);
        assert!(route(&Method::POST, "/files/read").unwrap_err());
        assert!(!route(&Method::GET, "/nichts").unwrap_err());
        let (r, id) = route(&Method::DELETE, "/execute/2024-ab%20c").unwrap();
        assert_eq!((r, id.as_deref()), (Route::Kill, Some("2024-ab c")));
        assert_eq!(route(&Method::GET, "/execute/x/status").unwrap().0, Route::Status);
        assert_eq!(route(&Method::GET, "/quellen/search").unwrap().0, Route::SourcesSearch);
        assert!(route(&Method::POST, "/quellen/read").unwrap_err());
    }
}
