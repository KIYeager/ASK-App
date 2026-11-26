//! Einstellungen der App als JSON im Benutzerordner (%APPDATA%\ASK\einstellungen.json, derselbe
//! Ort wie bei Electron: Ein Update auf die Tauri-Fassung behält sie).
//! Die Werte bleiben untypisiertes JSON wie in der Electron-Fassung, damit ältere und neuere
//! Fassungen dieselbe Datei lesen.

use crate::util::{same_path, win_basename};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::path::PathBuf;

pub const MAX_RECENT: usize = 8;

/// Farben der Projekte in der Seitenleiste (TF Blau zuerst, danach Sekundärfarben des CD)
pub const PROJECT_COLORS: [&str; 7] = ["#0082B4", "#F28855", "#4EB68E", "#EE716D", "#60C5E2", "#A2CE8D", "#A5ACAE"];

fn defaults() -> Map<String, Value> {
    let v = json!({
        "askUrl": "",
        "folder": "",
        "recent": [],
        "port": 0,
        "warningAccepted": false,
        "sidebarOpen": true,
        "chatbarOpen": true,
        "model": "",
        "webSearch": false,
        "thinking": "standard",
        "approvals": "heikel",
        "mode": "auto",
        "theme": "system",
        "betaUpdates": false,
    });
    match v {
        Value::Object(m) => m,
        _ => Map::new(),
    }
}

static DATA: Mutex<Option<Map<String, Value>>> = Mutex::new(None);

pub fn file() -> PathBuf {
    crate::paths::user_data().join("einstellungen.json")
}

pub fn app_config() -> Value {
    std::fs::read_to_string(crate::paths::app_config_file())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}))
}

fn loaded(guard: &mut Option<Map<String, Value>>) -> &mut Map<String, Value> {
    if guard.is_none() {
        let stored: Map<String, Value> = std::fs::read_to_string(file())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| if let Value::Object(m) = v { Some(m) } else { None })
            .unwrap_or_default();
        let mut data = defaults();
        for (k, v) in stored {
            data.insert(k, v);
        }
        // Immer die mitgelieferte Adresse der Schule (ENTSCHEIDE E58), ausser einem nachgebauten ASK für Tests
        let stored = data.get("askUrl").map(crate::util::js_string).unwrap_or_default();
        let built_in = app_config().get("askUrl").map(crate::util::js_string).unwrap_or_default();
        data.insert("askUrl".into(), Value::String(crate::askapi::effective_ask_url(&stored, &built_in)));
        if !data.get("recent").is_some_and(Value::is_array) {
            data.insert("recent".into(), json!([]));
        }
        // Projekte (ein Projekt = ein Arbeitsordner). Ältere Einstellungen kennen nur «recent».
        if !data.get("projects").is_some_and(Value::is_array) {
            let projects: Vec<Value> = data["recent"].as_array().cloned().unwrap_or_default().iter().enumerate()
                .map(|(i, f)| {
                    let folder = crate::util::js_string(f);
                    let base = win_basename(&folder);
                    let name = if base.is_empty() { folder.clone() } else { base };
                    json!({ "folder": folder, "name": name, "color": PROJECT_COLORS[i % PROJECT_COLORS.len()] })
                })
                .collect();
            data.insert("projects".into(), Value::Array(projects));
        }
        *guard = Some(data);
    }
    guard.as_mut().expect("Einstellungen geladen")
}

fn save(data: &Map<String, Value>) {
    let path = file();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let text = serde_json::to_string_pretty(&Value::Object(data.clone())).unwrap_or_else(|_| "{}".into());
    let tmp = path.with_extension("json.tmp");
    let ok = std::fs::write(&tmp, &text).is_ok() && std::fs::rename(&tmp, &path).is_ok();
    if !ok {
        // Umbenennen kann scheitern (Virenscanner, umgeleitete Ordner): dann direkt schreiben.
        let _ = std::fs::remove_file(&tmp);
        let _ = std::fs::write(&path, text);
    }
}

/// Alle Einstellungen (Kopie)
pub fn load() -> Value {
    let mut g = DATA.lock();
    Value::Object(loaded(&mut g).clone())
}

/// Ein Wert; fehlt er: null
pub fn get(key: &str) -> Value {
    let mut g = DATA.lock();
    loaded(&mut g).get(key).cloned().unwrap_or(Value::Null)
}

/// Werte setzen und speichern (`Object.assign`)
pub fn set(values: Value) {
    let mut g = DATA.lock();
    let data = loaded(&mut g);
    if let Value::Object(m) = values {
        for (k, v) in m {
            data.insert(k, v);
        }
    }
    save(data);
}

/// Einen Wert ganz entfernen (in JavaScript `set({ key: undefined })`)
pub fn remove(key: &str) {
    let mut g = DATA.lock();
    let data = loaded(&mut g);
    if data.remove(key).is_some() {
        save(data);
    }
}

/// Nur für Tests: beim nächsten Zugriff neu aus der Datei lesen.
pub fn reload() {
    *DATA.lock() = None;
}

fn find_index(data: &Map<String, Value>, folder: &str) -> Option<usize> {
    data.get("projects")?.as_array()?.iter()
        .position(|p| crate::util::str_of(p, "folder").to_lowercase() == folder.to_lowercase())
}

pub fn find_project(folder: &str) -> Option<Value> {
    let mut g = DATA.lock();
    let data = loaded(&mut g);
    let i = find_index(data, folder)?;
    Some(data["projects"][i].clone())
}

/// Ordner als zuletzt verwendet und als aktuellen Ordner merken; ohne Projekt wird eines angelegt.
pub fn add_recent(folder: &str, name: &str) {
    let mut g = DATA.lock();
    let data = loaded(&mut g);
    let mut recent: Vec<Value> = vec![Value::String(folder.to_string())];
    for f in data["recent"].as_array().cloned().unwrap_or_default() {
        if crate::util::js_string(&f).to_lowercase() != folder.to_lowercase() {
            recent.push(f);
        }
    }
    recent.truncate(MAX_RECENT);
    data.insert("recent".into(), Value::Array(recent));
    data.insert("folder".into(), Value::String(folder.to_string()));
    match find_index(data, folder) {
        None => {
            let projects = data["projects"].as_array().cloned().unwrap_or_default();
            let used: Vec<String> = projects.iter().map(|p| crate::util::str_of(p, "color")).collect();
            let color = PROJECT_COLORS.iter().find(|c| !used.iter().any(|u| u == *c)).copied()
                .unwrap_or(PROJECT_COLORS[projects.len() % PROJECT_COLORS.len()]);
            let base = win_basename(folder);
            let fallback = if base.is_empty() { folder.to_string() } else { base };
            let title = if name.is_empty() { fallback } else { name.to_string() };
            let mut list = vec![json!({ "folder": folder, "name": crate::util::take_chars(&title, 80), "color": color })];
            list.extend(projects);
            data.insert("projects".into(), Value::Array(list));
        }
        Some(i) if !name.is_empty() => {
            data["projects"][i]["name"] = Value::String(crate::util::take_chars(name, 80));
        }
        Some(_) => {}
    }
    save(data);
}

pub fn remove_recent(folder: &str) {
    let mut g = DATA.lock();
    let data = loaded(&mut g);
    let keep = |f: &str| f.to_lowercase() != folder.to_lowercase();
    let recent: Vec<Value> = data["recent"].as_array().cloned().unwrap_or_default().into_iter()
        .filter(|f| keep(&crate::util::js_string(f))).collect();
    let projects: Vec<Value> = data["projects"].as_array().cloned().unwrap_or_default().into_iter()
        .filter(|p| keep(&crate::util::str_of(p, "folder"))).collect();
    data.insert("recent".into(), Value::Array(recent));
    data.insert("projects".into(), Value::Array(projects));
    if crate::util::js_string(&data["folder"]).to_lowercase() == folder.to_lowercase() {
        data.insert("folder".into(), Value::String(String::new()));
    }
    save(data);
}

/// Name oder Farbe eines Projekts ändern
pub fn update_project(folder: &str, patch: &Value) -> bool {
    let mut g = DATA.lock();
    let data = loaded(&mut g);
    let Some(i) = find_index(data, folder) else { return false };
    if let Some(Value::String(name)) = patch.get("name") {
        if !name.trim().is_empty() {
            data["projects"][i]["name"] = Value::String(crate::util::take_chars(name.trim(), 80));
        }
    }
    if let Some(Value::String(color)) = patch.get("color") {
        let ok = color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit());
        if ok {
            data["projects"][i]["color"] = Value::String(color.clone());
        }
    }
    save(data);
    true
}

/// Ordner gleich (für Aufrufer, die zwei Pfade vergleichen)
pub fn same_folder(a: &str, b: &str) -> bool {
    same_path(a, b)
}
