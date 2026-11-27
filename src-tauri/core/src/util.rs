//! Kleine Hilfen, die viele Module brauchen: Ereignisse wie EventEmitter in Node, Zeit in
//! Millisekunden und JavaScript-ähnliche Zugriffe auf JSON-Werte (die Daten kommen aus der
//! Oberfläche, aus ASK und aus Dateien, die das Modell ändern kann, und sind nicht typisiert).

use parking_lot::Mutex;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub type Listener = Arc<dyn Fn(&Value) + Send + Sync>;

/// Wie `EventEmitter` in Node: `on(name, f)` und `emit(name, payload)`. Klonen teilt die Liste.
/// Die Rückrufe laufen ohne gehaltene Sperre, dürfen also selbst wieder `on` oder `emit` aufrufen.
#[derive(Clone, Default)]
pub struct Emitter {
    inner: Arc<Mutex<HashMap<String, Vec<Listener>>>>,
}

impl Emitter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn on(&self, event: &str, f: impl Fn(&Value) + Send + Sync + 'static) {
        self.inner.lock().entry(event.to_string()).or_default().push(Arc::new(f));
    }

    pub fn emit(&self, event: &str, payload: &Value) {
        let list: Vec<Listener> = self.inner.lock().get(event).cloned().unwrap_or_default();
        for f in list {
            f(payload);
        }
    }

    pub fn has_listeners(&self, event: &str) -> bool {
        self.inner.lock().get(event).is_some_and(|l| !l.is_empty())
    }
}

/// Millisekunden seit 1970 wie `Date.now()`.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Wahrheitswert wie in JavaScript (`!!x`).
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0 && !f.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `String(x ?? '')`: Text eines Werts; null wird leer, Zahlen und Wahrheitswerte als Text.
pub fn js_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                i.to_string()
            } else if let Some(f) = n.as_f64() {
                if f.fract() == 0.0 && f.abs() < 1e15 { format!("{}", f as i64) } else { f.to_string() }
            } else {
                n.to_string()
            }
        }
        Value::Array(a) => a.iter().map(js_string).collect::<Vec<_>>().join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// Text eines Felds (`String(obj[key] ?? '')`).
pub fn str_of(v: &Value, key: &str) -> String {
    js_string(v.get(key).unwrap_or(&Value::Null))
}

/// Zahl eines Felds (`Number(obj[key]) || 0`).
pub fn num_of(v: &Value, key: &str) -> f64 {
    match v.get(key) {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => s.trim().parse::<f64>().unwrap_or(0.0),
        Some(Value::Bool(true)) => 1.0,
        _ => 0.0,
    }
}

/// Wahrheitswert eines Felds (`!!obj[key]`).
pub fn bool_of(v: &Value, key: &str) -> bool {
    v.get(key).is_some_and(truthy)
}

/// Zeichenweise kürzen (JavaScript zählt UTF-16-Einheiten, hier Unicode-Zeichen; für Grenzen wie
/// «höchstens 80 Zeichen» ist das gleichwertig und schneidet nie mitten in ein Zeichen).
pub fn take_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Länge in Zeichen (für Vergleiche mit Grenzen aus dem JavaScript-Code).
pub fn char_len(s: &str) -> usize {
    s.chars().count()
}

/// Letzter Teil eines Windows- oder POSIX-Pfads (`path.win32.basename`).
pub fn win_basename(p: &str) -> String {
    p.trim_end_matches(['\\', '/']).rsplit(['\\', '/']).next().unwrap_or("").to_string()
}

/// Gleicher Pfad ohne Rücksicht auf Gross- und Kleinschreibung (Windows), nach `path.resolve`.
pub fn same_path(a: &str, b: &str) -> bool {
    !a.is_empty() && !b.is_empty() && path_key(a) == path_key(b)
}

/// Schlüssel eines Ordners für Tabellen: aufgelöst, ohne abschliessenden Trenner, klein geschrieben.
pub fn path_key(p: &str) -> String {
    let resolved = std::path::absolute(p).unwrap_or_else(|_| std::path::PathBuf::from(p));
    let s = resolved.to_string_lossy().to_string();
    let s = if s.len() > 3 { s.trim_end_matches(['\\', '/']).to_string() } else { s };
    s.to_lowercase()
}

/// Binärdaten aus der Oberfläche: `{ "$b64": "…" }` (bridge.js), eine Liste von Bytes (ältere Aufrufe,
/// Tests) oder Text. Alles andere ergibt leere Daten.
pub fn bytes_of(v: &Value) -> Vec<u8> {
    use base64::Engine;
    match v {
        Value::Object(m) => m.get("$b64").and_then(Value::as_str)
            .and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
            .unwrap_or_default(),
        Value::Array(a) => a.iter().map(|b| b.as_u64().unwrap_or(0) as u8).collect(),
        Value::String(s) => s.as_bytes().to_vec(),
        _ => Vec::new(),
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn js_like_access() {
        let v = json!({ "a": "x", "n": 3, "f": 1.5, "b": true, "z": 0, "e": "" });
        assert_eq!(str_of(&v, "a"), "x");
        assert_eq!(str_of(&v, "n"), "3");
        assert_eq!(str_of(&v, "missing"), "");
        assert_eq!(num_of(&v, "f"), 1.5);
        assert!(bool_of(&v, "b"));
        assert!(!bool_of(&v, "z"));
        assert!(!bool_of(&v, "e"));
    }

    #[test]
    fn emitter_calls_listeners() {
        let e = Emitter::new();
        let hits = Arc::new(Mutex::new(Vec::new()));
        let h = hits.clone();
        e.on("x", move |v| h.lock().push(v.clone()));
        e.emit("x", &serde_json::json!(1));
        e.emit("y", &serde_json::json!(2));
        assert_eq!(hits.lock().len(), 1);
    }

    #[test]
    fn basename_windows_and_posix() {
        assert_eq!(win_basename(r"C:\Users\a\Projekt"), "Projekt");
        assert_eq!(win_basename("/home/a/b/"), "b");
    }
}
