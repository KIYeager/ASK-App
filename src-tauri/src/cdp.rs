//! Webansichten fernsteuern: JavaScript mit Ergebnis ausführen, Bildschirmfoto, Maus und Tastatur.
//! In Electron gab es dafür `webContents.executeJavaScript`, `capturePage` und `sendInputEvent`.
//! Unter Windows geht das über das DevTools-Protokoll von WebView2 (`CallDevToolsProtocolMethod`),
//! ohne offenen Debug-Port: Runtime.evaluate, Page.captureScreenshot, Input.dispatch*.
//! Unter Linux (nur Entwicklung) gibt es kein DevTools-Protokoll; dort meldet jeder Aufruf einen Fehler.

use anyhow::{anyhow, bail, Result};
use serde_json::{json, Value};
use std::time::Duration;
use tauri::{Runtime, Webview};

/// Höchstens so lange auf eine Antwort von WebView2 warten
const CALL_TIMEOUT: Duration = Duration::from_secs(20);

/// Methode des DevTools-Protokolls aufrufen; Ergebnis als JSON
#[cfg(windows)]
pub async fn call<R: Runtime>(webview: &Webview<R>, method: &str, params: Value) -> Result<Value> {
    use webview2_com::CallDevToolsProtocolMethodCompletedHandler;
    use windows::core::HSTRING;

    let (tx, rx) = tokio::sync::oneshot::channel::<Result<Value>>();
    let method_name = method.to_string();
    let params_text = params.to_string();
    webview
        .with_webview(move |pw| {
            let tx = std::sync::Mutex::new(Some(tx));
            let send = move |r: Result<Value>| {
                if let Some(t) = tx.lock().ok().and_then(|mut g| g.take()) {
                    let _ = t.send(r);
                }
            };
            let send = std::sync::Arc::new(send);
            let send2 = send.clone();
            let run = || -> windows::core::Result<()> {
                let core = unsafe { pw.controller().CoreWebView2()? };
                let handler = CallDevToolsProtocolMethodCompletedHandler::create(Box::new(move |res, json: String| {
                    send(match res {
                        Ok(()) => serde_json::from_str(&json).map_err(|e| anyhow!("Antwort von WebView2 unlesbar: {e}")),
                        Err(e) => Err(anyhow!("WebView2: {e}")),
                    });
                    Ok(())
                }));
                unsafe { core.CallDevToolsProtocolMethod(&HSTRING::from(method_name.as_str()), &HSTRING::from(params_text.as_str()), &handler) }
            };
            if let Err(e) = run() {
                send2(Err(anyhow!("WebView2: {e}")));
            }
        })
        .map_err(|e| anyhow!("Webansicht nicht erreichbar: {e}"))?;
    match tokio::time::timeout(CALL_TIMEOUT, rx).await {
        Ok(Ok(r)) => r,
        Ok(Err(_)) => bail!("Webansicht geschlossen"),
        Err(_) => bail!("Zeitüberschreitung bei {method}"),
    }
}

#[cfg(not(windows))]
pub async fn call<R: Runtime>(_webview: &Webview<R>, method: &str, _params: Value) -> Result<Value> {
    bail!("{method} gibt es nur unter Windows (WebView2)")
}

/// JavaScript ausführen und das Ergebnis als JSON zurückgeben (wie `executeJavaScript(code, true)`):
/// wartet auf Promises, Ausnahmen werden zu Fehlern.
pub async fn eval<R: Runtime>(webview: &Webview<R>, js: &str) -> Result<Value> {
    let res = call(webview, "Runtime.evaluate", json!({
        "expression": js,
        "returnByValue": true,
        "awaitPromise": true,
        "userGesture": true,
    })).await?;
    if let Some(ex) = res.get("exceptionDetails") {
        let text = ex.pointer("/exception/description").and_then(Value::as_str)
            .or_else(|| ex.get("text").and_then(Value::as_str))
            .unwrap_or("Fehler im Skript");
        bail!("{text}");
    }
    Ok(res.pointer("/result/value").cloned().unwrap_or(Value::Null))
}

/// Bildschirmfoto der sichtbaren Seite als PNG
pub async fn screenshot<R: Runtime>(webview: &Webview<R>) -> Result<Vec<u8>> {
    use base64::Engine;
    let res = call(webview, "Page.captureScreenshot", json!({ "format": "png", "fromSurface": true })).await?;
    let data = res.get("data").and_then(Value::as_str).ok_or_else(|| anyhow!("Kein Bild erhalten"))?;
    Ok(base64::engine::general_purpose::STANDARD.decode(data)?)
}

/// Mausereignis: kind = mousePressed | mouseReleased | mouseMoved | mouseWheel
pub async fn mouse<R: Runtime>(webview: &Webview<R>, kind: &str, x: f64, y: f64, button: &str, clicks: u32, delta: (f64, f64)) -> Result<()> {
    let mut params = json!({ "type": kind, "x": x, "y": y, "button": button, "clickCount": clicks });
    if kind == "mouseWheel" {
        params["deltaX"] = json!(delta.0);
        params["deltaY"] = json!(delta.1);
    }
    call(webview, "Input.dispatchMouseEvent", params).await.map(|_| ())
}

/// Tastaturereignis: kind = keyDown | keyUp | rawKeyDown | char
pub async fn key<R: Runtime>(webview: &Webview<R>, kind: &str, key: &str, code: &str, key_code: i64, text: &str, modifiers: i64) -> Result<()> {
    let mut params = json!({ "type": kind, "key": key, "code": code, "windowsVirtualKeyCode": key_code, "modifiers": modifiers });
    if !text.is_empty() {
        params["text"] = json!(text);
        params["unmodifiedText"] = json!(text);
    }
    call(webview, "Input.dispatchKeyEvent", params).await.map(|_| ())
}

/// Text einfügen, als hätte jemand ihn getippt
pub async fn insert_text<R: Runtime>(webview: &Webview<R>, text: &str) -> Result<()> {
    call(webview, "Input.insertText", json!({ "text": text })).await.map(|_| ())
}

/// Grösse der Seite für Bildschirmfotos festlegen (unsichtbares Fenster); 0 setzt zurück
pub async fn set_viewport<R: Runtime>(webview: &Webview<R>, width: u32, height: u32) -> Result<()> {
    if width == 0 {
        return call(webview, "Emulation.clearDeviceMetricsOverride", json!({})).await.map(|_| ());
    }
    call(webview, "Emulation.setDeviceMetricsOverride", json!({
        "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false,
    })).await.map(|_| ())
}
