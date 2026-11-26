// Noch nicht portiert (src/main/visual.js). Platzhalter für die Funktion, die die Hülle schon braucht.
pub fn allowed_navigation(url: &str, _file_root: &str) -> bool {
    url.starts_with("http://") || url.starts_with("https://") || url == "about:blank"
}
