//! Teile, die es nur unter Windows gibt (WebView2, Benachrichtigungen mit Klick, AppUserModelID).
//! Unter Linux (Entwicklung) tun sie nichts oder fallen auf das Tauri-Plugin zurück.

use serde_json::json;
use tauri::{AppHandle, Webview, Wry};

pub const APP_ID: &str = "ch.tfbern.ask";

/// Wie `app.setAppUserModelId`: Benachrichtigungen und Taskleiste gehören zu «ASK»
pub fn set_app_user_model_id() {
    #[cfg(windows)]
    unsafe {
        let _ = windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(&windows::core::HSTRING::from(APP_ID));
    }
}

/// Berechtigungen, die ASK bekommen darf (Electron: ALLOWED_PERMISSIONS in window.js). Alles andere wird
/// abgelehnt. WebView2 kennt «local-network-access» nicht als eigene Art; Mikrofon und Kamera entsprechen «media».
#[cfg(windows)]
fn ask_allows(kind: webview2_com::Microsoft::Web::WebView2::Win32::COREWEBVIEW2_PERMISSION_KIND) -> bool {
    use webview2_com::Microsoft::Web::WebView2::Win32::*;
    kind == COREWEBVIEW2_PERMISSION_KIND_CLIPBOARD_READ
        || kind == COREWEBVIEW2_PERMISSION_KIND_NOTIFICATIONS
        || kind == COREWEBVIEW2_PERMISSION_KIND_MICROPHONE
        || kind == COREWEBVIEW2_PERMISSION_KIND_CAMERA
}

/// ASK-Seite: Berechtigungen nur für den Ursprung von ASK, Ladefehler an askapi (ENTSCHEIDE E53)
pub fn setup_ask_view(wv: &Webview<Wry>) {
    #[cfg(windows)]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        use webview2_com::{take_pwstr, NavigationCompletedEventHandler, PermissionRequestedEventHandler};
        let _ = wv.with_webview(|pw| unsafe {
            let Ok(core) = pw.controller().CoreWebView2() else { return };
            let mut token = 0i64;
            let _ = core.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_sender, args| {
                    let Some(args) = args else { return Ok(()) };
                    let mut kind = COREWEBVIEW2_PERMISSION_KIND_UNKNOWN_PERMISSION;
                    args.PermissionKind(&mut kind)?;
                    let mut uri = windows::core::PWSTR::null();
                    args.Uri(&mut uri)?;
                    let origin = url::Url::parse(&take_pwstr(uri)).map(|u| u.origin().ascii_serialization()).unwrap_or_default();
                    let allowed_origin = crate::shell().views.ask_origin.lock().clone();
                    let ok = !allowed_origin.is_empty() && origin == allowed_origin && ask_allows(kind);
                    args.SetState(if ok { COREWEBVIEW2_PERMISSION_STATE_ALLOW } else { COREWEBVIEW2_PERMISSION_STATE_DENY })
                })),
                &mut token,
            );
            let _ = core.add_NavigationCompleted(
                &NavigationCompletedEventHandler::create(Box::new(|sender, args| {
                    let (Some(sender), Some(args)) = (sender, args) else { return Ok(()) };
                    let mut ok = windows::core::BOOL::default();
                    args.IsSuccess(&mut ok)?;
                    if !ok.as_bool() {
                        let mut status = COREWEBVIEW2_WEB_ERROR_STATUS_UNKNOWN;
                        args.WebErrorStatus(&mut status)?;
                        let mut src = windows::core::PWSTR::null();
                        sender.Source(&mut src)?;
                        let url = take_pwstr(src);
                        // Abgebrochen (eine neue Navigation) ist kein Fehler (Electron: Code -3)
                        crate::askview::LOADING.store(false, std::sync::atomic::Ordering::SeqCst);
                        if status != COREWEBVIEW2_WEB_ERROR_STATUS_OPERATION_CANCELED {
                            if let Some(core) = crate::shell().core_opt() {
                                // «Name nicht gefunden» mit dem Code und Text von Chromium, den die Oberfläche und askapi kennen
                                if status == COREWEBVIEW2_WEB_ERROR_STATUS_HOST_NAME_NOT_RESOLVED {
                                    core.ask_load_failed(-105, "ERR_NAME_NOT_RESOLVED", &url);
                                } else {
                                    core.ask_load_failed(status.0, &web_error_text(status.0), &url);
                                }
                            }
                        }
                    }
                    Ok(())
                })),
                &mut token,
            );
        });
    }
    #[cfg(not(windows))]
    let _ = wv;
}

/// Browser der App: keine Berechtigungen ausser Vollbild (das fragt WebView2 nicht nach)
pub fn setup_browser_view(wv: &Webview<Wry>) {
    #[cfg(windows)]
    {
        use webview2_com::Microsoft::Web::WebView2::Win32::*;
        use webview2_com::{DocumentTitleChangedEventHandler, HistoryChangedEventHandler, PermissionRequestedEventHandler};
        let label = wv.label().to_string();
        let _ = wv.with_webview(move |pw| unsafe {
            let Ok(core) = pw.controller().CoreWebView2() else { return };
            let mut token = 0i64;
            let _ = core.add_PermissionRequested(
                &PermissionRequestedEventHandler::create(Box::new(|_s, args| {
                    if let Some(args) = args {
                        args.SetState(COREWEBVIEW2_PERMISSION_STATE_DENY)?;
                    }
                    Ok(())
                })),
                &mut token,
            );
            let l1 = label.clone();
            let _ = core.add_HistoryChanged(
                &HistoryChangedEventHandler::create(Box::new(move |_s, _a| { report_browser(&l1); Ok(()) })),
                &mut token,
            );
            let l2 = label.clone();
            let _ = core.add_DocumentTitleChanged(
                &DocumentTitleChangedEventHandler::create(Box::new(move |_s, _a| { report_browser(&l2); Ok(()) })),
                &mut token,
            );
        });
    }
    #[cfg(not(windows))]
    let _ = wv;
}

#[cfg(windows)]
fn report_browser(label: &str) {
    if let Some(wv) = crate::shell().views.webview(label) {
        crate::shell().browser_state(&wv);
    }
}

/// Zurück und vorwärts im Verlauf, und ob das geht
pub fn history(wv: &Webview<Wry>, step: i32) {
    #[cfg(windows)]
    let _ = wv.with_webview(move |pw| unsafe {
        if let Ok(core) = pw.controller().CoreWebView2() {
            let _ = if step < 0 { core.GoBack() } else { core.GoForward() };
        }
    });
    #[cfg(not(windows))]
    {
        let js = if step < 0 { "history.back()" } else { "history.forward()" };
        let _ = wv.eval(js);
    }
}

/// (zurück möglich, vorwärts möglich)
pub async fn can_go(wv: &Webview<Wry>) -> (bool, bool) {
    #[cfg(windows)]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let _ = wv.with_webview(move |pw| unsafe {
            let mut back = windows::core::BOOL::default();
            let mut fwd = windows::core::BOOL::default();
            if let Ok(core) = pw.controller().CoreWebView2() {
                let _ = core.CanGoBack(&mut back);
                let _ = core.CanGoForward(&mut fwd);
            }
            let _ = tx.send((back.as_bool(), fwd.as_bool()));
        });
        rx.await.unwrap_or((false, false))
    }
    #[cfg(not(windows))]
    {
        let _ = wv;
        (false, false)
    }
}

/// Meldung zu einem Ladefehler von WebView2 (wie errorDescription in Electron)
pub fn web_error_text(status: i32) -> String {
    match status {
        1..=6 => "Das Zertifikat der Seite ist ungültig".into(),
        7 => "Zeitüberschreitung".into(),
        8 | 11 => "Keine Verbindung zum Server".into(),
        9 => "Verbindung abgebrochen".into(),
        10 | 12 => "Verbindung zurückgesetzt".into(),
        13 => "Server nicht gefunden".into(),
        16 => "Kein Netzwerk".into(),
        _ => format!("Seite nicht erreichbar (Fehler {status})"),
    }
}

/// Windows-Benachrichtigung; ein Klick ruft on_click auf (ENTSCHEIDE E52)
pub fn notify(app: &AppHandle, title: &str, body: &str, silent: bool, on_click: Box<dyn FnOnce() + Send>) {
    #[cfg(windows)]
    {
        use tauri_winrt_notification::{Sound, Toast};
        let _ = app;
        let click = std::sync::Mutex::new(Some(on_click));
        let toast = Toast::new(APP_ID)
            .title(title)
            .text1(body)
            .sound(if silent { None } else { Some(Sound::Default) })
            .on_activated(move |_action| {
                if let Some(f) = click.lock().ok().and_then(|mut g| g.take()) {
                    f();
                }
                Ok(())
            });
        let _ = toast.show();
    }
    #[cfg(not(windows))]
    {
        use tauri_plugin_notification::NotificationExt;
        let _ = (silent, on_click);
        let _ = app.notification().builder().title(title).body(body).show();
    }
    let _ = json!(null);
}
