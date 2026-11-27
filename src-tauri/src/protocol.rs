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
fn read_with_retry(rel: &str) -> Option<(std::path::PathBuf, Vec<u8>)> {
    for attempt in 0..8 {
        if let Some(full) = crate::shell().core().resolve_file(rel) {
            if let Ok(data) = std::fs::read(&full) {
                return Some((full, data));
            }
        }
        if attempt < 7 {
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
    }
    None
}

fn respond(uri: &tauri::http::Uri, range: Option<&str>) -> Response<Vec<u8>> {
    let Some(rel) = relative_path(uri.host().unwrap_or(""), uri.path()) else { return not_found() };
    let Some((full, data)) = read_with_retry(&rel) else { return not_found() };
    let mime = mime_of(&full);
    let total = data.len();
    let builder = Response::builder()
        .header(header::CONTENT_TYPE, mime)
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CACHE_CONTROL, "no-cache");
    if let Some((start, end)) = range.and_then(|r| parse_range(r, total)) {
        return builder
            .status(StatusCode::PARTIAL_CONTENT)
            .header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{total}"))
            .body(data[start..=end].to_vec())
            .expect("Antwort");
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

// ---- Protokoll askmod (ENTSCHEIDE E72): Dateien eingeschalteter Mods für die Oberfläche ----------------------
// Adresse: askmod://mod/<id>/<fingerabdruck>/<pfad> bzw. http://askmod.localhost/mod/<id>/<fingerabdruck>/<pfad>.
// Geliefert wird nur, was mods.rs für einen eingeschalteten, unveränderten Mod freigibt.

pub fn handle_mod(_ctx: UriSchemeContext<'_, Wry>, request: Request<Vec<u8>>, responder: UriSchemeResponder) {
    let uri = request.uri().clone();
    tauri::async_runtime::spawn_blocking(move || {
        responder.respond(respond_mod(&uri));
    });
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
