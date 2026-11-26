//! Geheimnisse der App (GitHub-Token) verschlüsselt mit dem Windows-Konto (DPAPI), wie zuvor mit
//! `safeStorage` von Electron. Gespeichert als «dpapi:<Base64>». Die Electron-Fassung schrieb
//! «<Base64 von v10 + Nonce + AES-GCM>»; den Schlüssel dazu hat Chromium mit DPAPI in «Local State»
//! (%APPDATA%\ASK) abgelegt. Solche Werte liest `unprotect` noch, damit nach dem Wechsel zu Tauri
//! niemand GitHub neu verbinden muss (ENTSCHEIDE E60).
//! Unter Linux (nur Entwicklung) gibt es kein DPAPI: dort «plain:<Base64>».

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use std::path::Path;

/// Text verschlüsseln; leer bleibt leer
pub fn protect(text: &str) -> String {
    if text.is_empty() {
        return String::new();
    }
    #[cfg(windows)]
    {
        match dpapi::protect(text.as_bytes()) {
            Some(data) => format!("dpapi:{}", B64.encode(data)),
            None => String::new(),
        }
    }
    #[cfg(not(windows))]
    {
        format!("plain:{}", B64.encode(text.as_bytes()))
    }
}

/// Gespeicherten Wert entschlüsseln; unlesbar oder leer ergibt leer
pub fn unprotect(stored: &str) -> String {
    if stored.is_empty() {
        return String::new();
    }
    if let Some(rest) = stored.strip_prefix("dpapi:") {
        #[cfg(windows)]
        {
            return B64.decode(rest).ok().and_then(|d| dpapi::unprotect(&d)).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
        }
        #[cfg(not(windows))]
        {
            let _ = rest;
            return String::new();
        }
    }
    if let Some(rest) = stored.strip_prefix("plain:") {
        return B64.decode(rest).ok().and_then(|b| String::from_utf8(b).ok()).unwrap_or_default();
    }
    electron_legacy(stored, &crate::paths::user_data()).unwrap_or_default()
}

/// Wert der Electron-Fassung (safeStorage) entschlüsseln
pub fn electron_legacy(stored: &str, user_data: &Path) -> Option<String> {
    let data = B64.decode(stored.trim()).ok()?;
    let key = chromium_key(user_data)?;
    decrypt_v10(&data, &key)
}

/// «v10» + 12 Byte Nonce + AES-256-GCM (Chiffrat mit Tag)
pub fn decrypt_v10(data: &[u8], key: &[u8]) -> Option<String> {
    use aes_gcm::aead::{Aead, KeyInit};
    let body = data.strip_prefix(b"v10")?;
    if body.len() < 12 + 16 || key.len() != 32 {
        return None;
    }
    let (nonce, cipher) = body.split_at(12);
    let aes = aes_gcm::Aes256Gcm::new_from_slice(key).ok()?;
    let plain = aes.decrypt(aes_gcm::Nonce::from_slice(nonce), cipher).ok()?;
    String::from_utf8(plain).ok()
}

/// Schlüssel aus «Local State» (os_crypt.encrypted_key, Base64 von «DPAPI» + DPAPI-Daten)
fn chromium_key(user_data: &Path) -> Option<Vec<u8>> {
    let text = std::fs::read_to_string(user_data.join("Local State")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let enc = B64.decode(v.pointer("/os_crypt/encrypted_key")?.as_str()?).ok()?;
    let blob = enc.strip_prefix(b"DPAPI")?;
    #[cfg(windows)]
    {
        dpapi::unprotect(blob)
    }
    #[cfg(not(windows))]
    {
        let _ = blob;
        None
    }
}

#[cfg(windows)]
mod dpapi {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB};

    fn run(input: &[u8], protect: bool) -> Option<Vec<u8>> {
        let inp = CRYPT_INTEGER_BLOB { cbData: input.len() as u32, pbData: input.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
        let ok = unsafe {
            if protect {
                CryptProtectData(&inp, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
            } else {
                CryptUnprotectData(&inp, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(), 0, &mut out)
            }
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let data = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize) }.to_vec();
        unsafe { LocalFree(out.pbData as _) };
        Some(data)
    }

    pub fn protect(data: &[u8]) -> Option<Vec<u8>> {
        run(data, true)
    }

    pub fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
        run(data, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_empty() {
        assert_eq!(protect(""), "");
        assert_eq!(unprotect(""), "");
        let p = protect("ghp_geheim");
        assert_ne!(p, "ghp_geheim");
        assert_eq!(unprotect(&p), "ghp_geheim");
        assert_eq!(unprotect("unsinn"), "");
    }

    #[test]
    fn electron_v10_format() {
        use aes_gcm::aead::{Aead, KeyInit};
        let key = [7u8; 32];
        let nonce = [3u8; 12];
        let aes = aes_gcm::Aes256Gcm::new_from_slice(&key).unwrap();
        let cipher = aes.encrypt(aes_gcm::Nonce::from_slice(&nonce), b"ghp_alt".as_ref()).unwrap();
        let mut data = b"v10".to_vec();
        data.extend_from_slice(&nonce);
        data.extend_from_slice(&cipher);
        assert_eq!(decrypt_v10(&data, &key).as_deref(), Some("ghp_alt"));
        assert_eq!(decrypt_v10(&data, &[8u8; 32]), None);
        assert_eq!(decrypt_v10(b"v11xxxx", &key), None);
    }
}
