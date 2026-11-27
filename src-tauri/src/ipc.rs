//! Der eine Befehl der Oberfläche: `ipc(channel, args)` wie `ipcRenderer.invoke` in Electron.
//! Lage und Sichtbarkeit der Webansichten, Tastenkürzel und die Knöpfe des Browsers erledigt die
//! Hülle selbst; alles andere geht an `ask_core::app::App::handle`.

use crate::{shell, views};
use serde_json::Value;

#[tauri::command]
pub async fn ipc(channel: String, args: Vec<Value>) -> Result<Value, String> {
    let arg = |i: usize| args.get(i).cloned().unwrap_or(Value::Null);
    let s = shell();
    let v = &s.views;
    match channel.as_str() {
        "ui:ask-bounds" => v.set_ask_bounds(views::Bounds::from_json(&arg(0))),
        "ui:ask-visible" => v.set_ask_visible(ask_core::util::truthy(&arg(0))),
        "ui:browser-bounds" => v.set_browser_bounds(views::Bounds::from_json(&arg(0))),
        "ui:browser-visible" => v.set_browser_visible(ask_core::util::truthy(&arg(0))),
        "ui:shortcut" => shortcut(arg(0).as_str().unwrap_or("")),
        "ask:reload" => {
            let url = ask_core::settings::get("askUrl").as_str().unwrap_or("").to_string();
            if let Some(wv) = v.webview(views::ASK) {
                let current = wv.url().map(|u| u.to_string()).unwrap_or_default();
                match url::Url::parse(&url) {
                    Ok(target) if !same_origin(&current, &url) => { let _ = wv.navigate(target); }
                    _ => { let _ = wv.reload(); }
                }
            }
        }
        // Anmelden in einem eigenen Fenster (ENTSCHEIDE E57)
        "ask:login-window" => {
            let url = ask_core::settings::get("askUrl").as_str().unwrap_or("").to_string();
            return Ok(Value::Bool(!url.is_empty() && crate::askview::open_login(&url)));
        }
        "browser:show" => { v.ensure_browser(); }
        "browser:go" => {
            let target = browser_address(&ask_core::util::js_string(&arg(0)));
            if let (Some(wv), Ok(u)) = (v.ensure_browser(), url::Url::parse(&target)) {
                let _ = wv.navigate(u);
            }
        }
        "browser:back" => { if let Some(wv) = v.webview(views::BROWSER) { crate::platform::history(&wv, -1); } }
        "browser:forward" => { if let Some(wv) = v.webview(views::BROWSER) { crate::platform::history(&wv, 1); } }
        "browser:reload" => { if let Some(wv) = v.webview(views::BROWSER) { let _ = wv.reload(); } }
        "browser:close" => {
            if let Some(wv) = v.webview(views::BROWSER) {
                let _ = wv.navigate("about:blank".parse().expect("URL"));
            }
            v.set_browser_visible(false);
        }
        // Rückfrage in der App, Löschen hier: Cookies und Speicher des Browsers der App
        "browser:clear" => {
            let ok = s.core().handle(&channel, args).await?;
            if ok == Value::Bool(true) {
                if let Some(wv) = v.webview(views::BROWSER) {
                    let _ = wv.clear_all_browsing_data();
                    let _ = wv.reload();
                }
            }
            return Ok(ok);
        }
        _ => return s.core().handle(&channel, args).await,
    }
    Ok(Value::Null)
}

fn shortcut(key: &str) {
    let v = &shell().views;
    match key {
        "zoom-in" => v.set_zoom(0.5),
        "zoom-out" => v.set_zoom(-0.5),
        "zoom-reset" => v.set_zoom(0.0),
        "reload-ask" => { if let Some(wv) = v.webview(views::ASK) { let _ = wv.reload(); } }
        "devtools-ask" => { if let Some(wv) = v.webview(views::ASK) { wv.open_devtools(); } }
        "devtools-ui" => { if let Some(wv) = v.webview(views::UI) { wv.open_devtools(); } }
        _ => {}
    }
}

pub fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(x), Ok(y)) => x.origin() == y.origin(),
        _ => false,
    }
}

/// Eingabe in der Adressleiste: Adresse oder Suchbegriffe
pub fn browser_address(text: &str) -> String {
    let t = text.trim();
    let lower = t.to_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return t.to_string();
    }
    let host_like = regex_lite_host(t);
    if !t.contains(' ') && host_like {
        return format!("https://{t}");
    }
    let q: String = percent_encoding::utf8_percent_encode(t, percent_encoding::NON_ALPHANUMERIC).to_string();
    format!("https://duckduckgo.com/?q={q}")
}

// /^[\w-]+(\.[\w-]+)+(:\d+)?(\/.*)?$/
fn regex_lite_host(t: &str) -> bool {
    let (host_port, _path) = match t.find('/') { Some(i) => (&t[..i], &t[i..]), None => (t, "") };
    let (host, port) = match host_port.rfind(':') { Some(i) => (&host_port[..i], Some(&host_port[i + 1..])), None => (host_port, None) };
    if let Some(p) = port {
        if p.is_empty() || !p.chars().all(|c| c.is_ascii_digit()) {
            return false;
        }
    }
    let word = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    let parts: Vec<&str> = host.split('.').collect();
    parts.len() >= 2 && parts.iter().all(|p| word(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn address_or_search() {
        assert_eq!(browser_address("https://a.ch/x"), "https://a.ch/x");
        assert_eq!(browser_address("tfbern.ch"), "https://tfbern.ch");
        assert_eq!(browser_address("localhost.test:8080/a"), "https://localhost.test:8080/a");
        assert_eq!(browser_address("wetter bern"), "https://duckduckgo.com/?q=wetter%20bern");
    }
}
