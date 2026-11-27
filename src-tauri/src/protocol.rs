//! Protokoll askfile: für die Vorschau in der Seitenleiste, nur Dateien aus dem Arbeitsordner.
//! Unter Windows kommt es als http://askfile.localhost/datei/<Pfad>, sonst als askfile://datei/<Pfad>.
//! Mit Range-Anfragen (Videos, grosse PDF).

use tauri::http::{header, Request, Response, StatusCode};
use tauri::{UriSchemeContext, UriSchemeResponder, Wry};

pub fn handle(_ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let uri = request.uri().clone();
    let range = request.headers().get(header::RANGE).and_then(|v| v.to_str().ok()).map(str::to_string);
    tauri::async_runtime::spawn_blocking(move || {
        responder.respond(respond(&uri, range.as_deref()));
    });
}

fn not_found() -> Response<Vec<u8>> {
    Response::builder().status(StatusCode::NOT_FOUND).body(b"Nicht gefunden".to_vec()).expect("Antwort")
}

/// Relativer Pfad aus der Adresse: Host «datei» (askfile://datei/a/b) oder Pfad /datei/a/b (Windows)
pub fn relative_path(host: &str, path: &str) -> Option<String> {
    let encoded = if host == "datei" { path.trim_start_matches('/') } else { path.strip_prefix("/datei/")? };
    let decoded = percent_encoding::percent_decode_str(encoded).decode_utf8().ok()?.to_string();
    Some(decoded.replace('/', std::path::MAIN_SEPARATOR_STR))
}

/// Eine eben geschriebene Datei (Bild vom Modell) kann unter Windows kurz gesperrt sein, etwa während der
/// Virenschutz sie prüft: dann bis gut eine Sekunde lang nochmals versuchen statt gleich «nicht gefunden»
fn open_with_retry(rel: &str) -> Option<(std::path::PathBuf, std::fs::File, usize)> {
    for attempt in 0..8 {
        if let Some(full) = crate::shell().core().resolve_file(rel) {
            if let Ok(file) = std::fs::File::open(&full) {
                if let Ok(meta) = file.metadata() {
                    return Some((full, file, meta.len() as usize));
                }
            }
        }
        if attempt < 7 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
    None
}

/// Höchstens so viel auf eine Teilanfrage (Range): Ein Video fragt in vielen Stücken, und jedes Stück läge sonst
/// ganz im Speicher (früher sogar die ganze Datei pro Anfrage).
const RANGE_MAX: usize = 8 * 1024 * 1024;

/// Bytes `start..=end` einer Datei lesen
fn read_slice(file: &mut std::fs::File, start: usize, end: usize) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(start as u64))?;
    let mut buf = vec![0u8; end - start + 1];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

fn respond(uri: &tauri::http::Uri, range: Option<&str>) -> Response<Vec<u8>> {
    let Some(rel) = relative_path(uri.host().unwrap_or(""), uri.path()) else { return not_found() };
    let Some((full, mut file, total)) = open_with_retry(&rel) else { return not_found() };
    let mime = mime_of(&full);
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "no-cache");
    if let Some((start, end)) = range.and_then(|r| parse_range(r, total)) {
        // Nur den verlangten Teil lesen, und höchstens RANGE_MAX: Den Rest holt die Ansicht mit der nächsten Anfrage
        let end = end.min(start + RANGE_MAX - 1);
        let Ok(part) = read_slice(&mut file, start, end) else { return not_found() };
        return builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
            .body(part)
            .expect("Antwort");
    }
    let mut data = Vec::with_capacity(total);
    if std::io::Read::read_to_end(&mut file, &mut data).is_err() {
        return not_found();
    }
    builder.status(StatusCode::OK).body(data).expect("Antwort")
}

/// «bytes=a-b», «bytes=a-», «bytes=-n» → (Anfang, Ende) einschliesslich
fn parse_range(r: &str, total: usize) -> Option<(usize, usize)> {
    if total == 0 {
        return None;
    }
    let spec = r.strip_prefix("bytes=")?.split(',').next()?.trim();
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: usize = b.parse().ok()?;
        (total.saturating_sub(n), total - 1)
    } else {
        let s: usize = a.parse().ok()?;
        let e = if b.is_empty() { total - 1 } else { b.parse::<usize>().ok()?.min(total - 1) };
        (s, e)
    };
    (start <= end && start < total).then_some((start, end))
}

fn mime_of(path: &std::path::Path) -> &'static str {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "html" | "htm" => "text/plain; charset=utf-8", // nie als Seite ausführen
        "txt" | "md" | "csv" | "json" | "xml" | "log" | "yml" | "yaml" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

// ---- Protokoll askui (ENTSCHEIDE E80): HTML-Bausteine aus Antworten, abgeschottet ---------------------------------
// Adresse: askui://ansicht/<id> bzw. http://askui.localhost/ansicht/<id>. Die Seite bekommt eine eigene CSP ohne Netz;
// das iframe in der Oberfläche hat sandbox ohne allow-same-origin, also einen undurchsichtigen Ursprung.

pub fn handle_ui(_ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let uri = request.uri().clone();
    let page = ask_core::uiblocks::id_from(uri.host().unwrap_or(""), uri.path()).and_then(|id| ask_core::uiblocks::page(&id));
    let response = match page {
        Some(html) => Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_SECURITY_POLICY, ask_core::uiblocks::CSP)
            .header(header::CACHE_CONTROL, "no-store")
            .header("X-Content-Type-Options", "nosniff")
            .header("Referrer-Policy", "no-referrer")
            .body(html.into_bytes())
            .expect("Antwort"),
        None => not_found(),
    };
    responder.respond(response);
}

// ---- Protokoll askmod (ENTSCHEIDE E72): Dateien eingeschalteter Mods für die Oberfläche ----------------------
// Adresse: askmod://mod/<id>/<fingerabdruck>/<pfad> bzw. http://askmod.localhost/mod/<id>/<fingerabdruck>/<pfad>.
// Geliefert wird nur, was mods.rs für einen eingeschalteten, unveränderten Mod freigibt.

pub fn handle_mod(ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let uri = request.uri().clone();
    // Testansicht (mod_testen, E75): Dateien auch ausgeschalteter Mods, aber nur an die unsichtbaren Testfenster
    let test = crate::pages::is_mod_test(ctx.webview_label());
    let origin = request.headers().get(header::ORIGIN).and_then(|v| v.to_str().ok());
    let dev = if cfg!(debug_assertions) { ctx.app_handle().config().build.dev_url.as_ref().map(|u| u.origin().ascii_serialization()) } else { None };
    let allow = allowed_origin(origin, dev.as_deref());
    tauri::async_runtime::spawn_blocking(move || {
        let response = if test { respond_mod_test(&uri) } else { respond_mod(&uri) };
        responder.respond(with_origin(response, allow.as_deref()));
    });
}

/// Ursprung der gebauten Oberfläche, wenn eine Anfrage keinen Origin-Header mitschickt
#[cfg(windows)]
const APP_ORIGIN: &str = "http://tauri.localhost"; // WebView2
#[cfg(not(windows))]
const APP_ORIGIN: &str = "tauri://localhost"; // WKWebView, WebKitGTK

/// Ursprünge der gebauten Oberfläche (https nur, falls useHttpsScheme einmal eingeschaltet wird)
const APP_ORIGINS: &[&str] = &["tauri://localhost", "http://tauri.localhost", "https://tauri.localhost"];

/// Wert für Access-Control-Allow-Origin: CSS, Code und Schriften der Mods kommen von askmod, also von einem anderen
/// Ursprung als die Oberfläche. Schriften (@font-face), `<link crossorigin>` und `<script crossorigin>` (mods.js)
/// lädt der Browser nur mit CORS-Freigabe. Freigegeben wird nur die eigene Oberfläche (in der Entwicklung auch der
/// Vite-Server `dev`), nicht `*`: Andere Seiten (z. B. im Browser-Bereich) können die Dateien nicht auslesen.
pub fn allowed_origin(origin: Option<&str>, dev: Option<&str>) -> Option<String> {
    match origin {
        Some(o) if APP_ORIGINS.contains(&o) || dev == Some(o) => Some(o.to_string()),
        Some(_) => None,
        None => Some(dev.unwrap_or(APP_ORIGIN).to_string()),
    }
}

fn with_origin(mut response: Response<Vec<u8>>, allow: Option<&str>) -> Response<Vec<u8>> {
    let headers = response.headers_mut();
    headers.insert(header::VARY, header::HeaderValue::from_static("Origin"));
    if let Some(value) = allow.and_then(|a| header::HeaderValue::from_str(a).ok()) {
        // Auch bei «nicht gefunden», sonst meldet der Browser einen CORS-Fehler statt 404
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, value);
    }
    response
}

fn respond_mod_test(uri: &tauri::http::Uri) -> Response<Vec<u8>> {
    let (host, path) = (uri.host().unwrap_or(""), uri.path());
    let rest = if host == "test" { Some(path.to_string()) } else { path.strip_prefix("/test").map(str::to_string) };
    let Some((id, hash, file)) = rest.and_then(|r| mod_path("mod", &r)) else { return not_found() };
    let Some(data) = crate::shell().core().knowledge.mods.test_file(&id, &hash, &file) else { return not_found() };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mod_mime(&file))
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .body(data)
        .expect("Antwort")
}

/// (Kennung, Fingerabdruck, Pfad) aus der Adresse
pub fn mod_path(host: &str, path: &str) -> Option<(String, String, String)> {
    let rest = if host == "mod" { path.trim_start_matches('/') } else { path.strip_prefix("/mod/")? };
    let decoded = percent_encoding::percent_decode_str(rest).decode_utf8().ok()?.to_string();
    let mut parts = decoded.splitn(3, '/');
    let (id, hash, file) = (parts.next()?, parts.next()?, parts.next()?);
    if id.is_empty() || hash.is_empty() || file.is_empty() {
        return None;
    }
    Some((id.to_string(), hash.to_string(), file.to_string()))
}

fn mod_mime(file: &str) -> &'static str {
    match file.rsplit('.').next().unwrap_or("").to_lowercase().as_str() {
        "js" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "woff2" => "font/woff2",
        "woff" => "font/woff",
        "ttf" => "font/ttf",
        _ => mime_of(std::path::Path::new(file)),
    }
}

fn respond_mod(uri: &tauri::http::Uri) -> Response<Vec<u8>> {
    let Some((id, hash, file)) = mod_path(uri.host().unwrap_or(""), uri.path()) else { return not_found() };
    let core = crate::shell().core();
    let mods = &core.knowledge.mods;
    if mods.info(&id).is_none_or(|m| m.hash != hash) {
        return not_found();
    }
    let Some(data) = mods.file(&id, &file) else { return not_found() };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mod_mime(&file))
        .header(header::CACHE_CONTROL, "no-cache")
        .header("X-Content-Type-Options", "nosniff")
        .body(data)
        .expect("Antwort")
}

#[cfg(test)]
mod tests {
    #[test]
    fn adressen_der_mods() {
        use super::mod_path;
        assert_eq!(mod_path("mod", "/robo/abc/bilder/a%20b.png"), Some(("robo".into(), "abc".into(), "bilder/a b.png".into())));
        assert_eq!(mod_path("askmod.localhost", "/mod/robo/abc/mod.js"), Some(("robo".into(), "abc".into(), "mod.js".into())));
        assert_eq!(mod_path("askmod.localhost", "/x/robo/abc/mod.js"), None);
        assert_eq!(mod_path("mod", "/robo/abc"), None);
    }

    #[test]
    fn schriften_und_css_nur_fuer_die_eigene_oberflaeche() {
        use super::{allowed_origin, APP_ORIGIN};
        assert_eq!(allowed_origin(Some("http://tauri.localhost"), None).as_deref(), Some("http://tauri.localhost"));
        assert_eq!(allowed_origin(Some("tauri://localhost"), None).as_deref(), Some("tauri://localhost"));
        assert_eq!(allowed_origin(None, None).as_deref(), Some(APP_ORIGIN));
        // Fremde Seiten bekommen keine Freigabe, auch nicht localhost mit anderem Port
        assert_eq!(allowed_origin(Some("https://example.com"), None), None);
        assert_eq!(allowed_origin(Some("http://localhost:8080"), None), None);
        assert_eq!(allowed_origin(Some("null"), None), None);
        // Entwicklung: Vite-Server
        let dev = Some("http://localhost:5173");
        assert_eq!(allowed_origin(Some("http://localhost:5173"), dev).as_deref(), Some("http://localhost:5173"));
        assert_eq!(allowed_origin(None, dev).as_deref(), dev);
        assert_eq!(allowed_origin(Some("http://localhost:8080"), dev), None);
    }

    #[test]
    fn freigabe_auch_bei_nicht_gefunden() {
        use super::{not_found, with_origin};
        let r = with_origin(not_found(), Some("tauri://localhost"));
        assert_eq!(r.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN], "tauri://localhost");
        assert_eq!(r.headers()[header::VARY], "Origin");
        let r = with_origin(not_found(), None);
        assert!(r.headers().get(header::ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
    }

    use super::*;

    #[test]
    fn paths_and_ranges() {
        let sep = std::path::MAIN_SEPARATOR_STR;
        assert_eq!(relative_path("datei", "/a/b%20c.png").unwrap(), format!("a{sep}b c.png"));
        assert_eq!(relative_path("askfile.localhost", "/datei/x.pdf").unwrap(), "x.pdf");
        assert!(relative_path("askfile.localhost", "/anders/x.pdf").is_none());
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 9)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=200-", 100), None);
    }
}
