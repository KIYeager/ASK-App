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

fn respond(uri: &tauri::http::Uri, range: Option<&str>) -> Response<Vec<u8>> {
    let Some(rel) = relative_path(uri.host().unwrap_or(""), uri.path()) else { return not_found() };
    let Some(full) = crate::shell().core().resolve_file(&rel) else { return not_found() };
    let Ok(data) = std::fs::read(&full) else { return not_found() };
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

#[cfg(test)]
mod tests {
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
