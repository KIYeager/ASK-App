//! HTML-Bausteine in Antworten (ENTSCHEIDE E80): Die Oberfläche legt das HTML eines ```ask-html-Blocks hier ab und
//! zeigt es in einem abgeschotteten iframe über das Protokoll askui (`askui://ansicht/<id>` bzw.
//! `http://askui.localhost/ansicht/<id>`). Nur im Speicher, begrenzt; die Kennung ist der Fingerabdruck des Inhalts.

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::collections::VecDeque;

/// Höchstens so viel HTML pro Baustein
pub const MAX_HTML: usize = 200 * 1024;
/// Höchstens so viele Bausteine im Speicher (ältere fallen weg und werden beim nächsten Anzeigen neu abgelegt)
const MAX_ENTRIES: usize = 64;

/// Vorspann vor dem HTML des Modells: Farben, Schrift, window.ask
pub const PRELUDE: &str = include_str!("../../../resources/ask-ui/vorspann.html");

/// Kein Netz, keine Formulare nach aussen, keine fremden Fenster: nur eigenes Inline-CSS und -JS und data:-Bilder
pub const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; \
font-src data:; media-src data: blob:; form-action 'none'; base-uri 'none'; frame-src 'none'; worker-src 'none'";

static STORE: Mutex<VecDeque<(String, String)>> = Mutex::new(VecDeque::new());

/// Legt HTML ab und gibt die Kennung zurück; `None`, wenn es zu gross oder leer ist
pub fn put(html: &str) -> Option<String> {
    if html.trim().is_empty() || html.len() > MAX_HTML {
        return None;
    }
    let id = hex::encode(&Sha256::digest(html.as_bytes())[..12]);
    let mut store = STORE.lock();
    if let Some(pos) = store.iter().position(|(k, _)| *k == id) {
        let entry = store.remove(pos).expect("vorhanden");
        store.push_back(entry);
    } else {
        store.push_back((id.clone(), html.to_string()));
        while store.len() > MAX_ENTRIES {
            store.pop_front();
        }
    }
    Some(id)
}

/// Ganze Seite für das iframe: Vorspann und HTML des Modells
pub fn page(id: &str) -> Option<String> {
    let store = STORE.lock();
    let html = store.iter().find(|(k, _)| k == id).map(|(_, v)| v.as_str())?;
    Some(format!("{PRELUDE}\n{html}"))
}

/// Kennung aus der Adresse: Host «ansicht» (askui://ansicht/<id>) oder Pfad /ansicht/<id> (Windows)
pub fn id_from(host: &str, path: &str) -> Option<String> {
    let rest = if host == "ansicht" { path.trim_start_matches('/') } else { path.strip_prefix("/ansicht/")? };
    let id = rest.split(['/', '?', '#']).next().unwrap_or("");
    (id.len() == 24 && id.bytes().all(|b| b.is_ascii_hexdigit())).then(|| id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ablegen_holen_und_begrenzen() {
        let id = put("<p>Hallo</p>").unwrap();
        assert_eq!(put("<p>Hallo</p>").unwrap(), id, "gleicher Inhalt, gleiche Kennung");
        let full = page(&id).unwrap();
        assert!(full.starts_with("<!doctype html>"));
        assert!(full.contains("window.ask") && full.ends_with("<p>Hallo</p>"));
        assert!(put("   ").is_none());
        assert!(put(&"x".repeat(MAX_HTML + 1)).is_none());
        assert!(page("000000000000000000000000").is_none());
        // Begrenzt (im selben Test, weil der Speicher für alle Tests derselbe ist)
        let first = put("<p>erster</p>").unwrap();
        for i in 0..MAX_ENTRIES {
            put(&format!("<p>{i}</p>"));
        }
        assert!(page(&first).is_none(), "der älteste fällt weg");
        assert!(STORE.lock().len() <= MAX_ENTRIES);
    }

    #[test]
    fn adressen() {
        let id = "0123456789abcdef01234567";
        assert_eq!(id_from("ansicht", &format!("/{id}")).as_deref(), Some(id));
        assert_eq!(id_from("askui.localhost", &format!("/ansicht/{id}")).as_deref(), Some(id));
        assert_eq!(id_from("askui.localhost", &format!("/ansicht/{id}/x")).as_deref(), Some(id));
        assert!(id_from("askui.localhost", "/ansicht/../datei").is_none());
        assert!(id_from("askui.localhost", &format!("/anders/{id}")).is_none());
    }

    #[test]
    fn kein_netz() {
        assert!(CSP.starts_with("default-src 'none'"));
        assert!(!CSP.contains("connect-src") && !CSP.contains("http"), "keine Verbindungen nach aussen");
    }
}
