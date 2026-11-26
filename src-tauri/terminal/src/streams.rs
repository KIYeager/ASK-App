//! Ausgabe eines Befehls zeilenweise lesen, ohne Grenze für die Zeilenlänge, und in Text umwandeln
//! (Port von python/ask_terminal/streams.py und `decode_output` aus server.py).
//!
//! Überlange Zeilen (eine Zeile JSON, Base64, verkleinertes HTML) kommen in Stücken weiter, damit nichts
//! verloren geht und der Befehl nicht abbricht.
//!
//! Öffentliche Schnittstelle:
//! - `read_lines(reader, handle, chunk)`: ruft `handle(zeile)` für jede Zeile (mit Zeilenende) auf
//! - `CHUNK`: Standardgrösse (64 KiB)
//! - `Oem`, `decode_output(zeile, &Oem)`: UTF-8, sonst OEM-Codepage

use tokio::io::{AsyncRead, AsyncReadExt};

pub const CHUNK: usize = 64 * 1024;

/// Ruft `handle(zeile)` für jede Zeile (mit Zeilenende) auf; überlange Zeilen kommen in Stücken.
pub async fn read_lines<R, F>(reader: &mut R, mut handle: F, chunk: usize) -> std::io::Result<()>
where
    R: AsyncRead + Unpin,
    F: FnMut(Vec<u8>),
{
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = vec![0u8; chunk];
    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        pending.extend_from_slice(&buf[..n]);
        // alle vollständigen Zeilen abgeben
        let mut start = 0;
        while let Some(pos) = pending[start..].iter().position(|b| *b == b'\n') {
            let end = start + pos + 1;
            handle(pending[start..end].to_vec());
            start = end;
        }
        pending.drain(..start);
        if pending.len() >= chunk {
            // Kein Zeichen in der Mitte trennen: unvollständige UTF-8-Folge am Ende zurückhalten
            let len = pending.len();
            let mut cut = len;
            while cut > len.saturating_sub(4) && cut > 0 && (pending[cut - 1] & 0xC0) == 0x80 {
                cut -= 1;
            }
            if cut > 0 && pending[cut - 1] >= 0xC0 {
                cut -= 1;
            }
            if cut == 0 {
                cut = len;
            }
            handle(pending[..cut].to_vec());
            pending.drain(..cut);
        }
    }
    if !pending.is_empty() {
        handle(pending);
    }
    Ok(())
}

/// Die OEM-Codepage des Systems. Windows-Programme (ping, ipconfig, Meldungen von cmd) schreiben sie, wenn
/// die Konsole nicht auf UTF-8 steht. Unter Windows kommt die Umwandlung von Windows selbst
/// (`MultiByteToWideChar`), sonst gilt cp850 (nur für Entwicklung und Tests).
#[derive(Clone, Copy, Debug)]
pub struct Oem {
    codepage: u32,
}

impl Oem {
    /// Codepage des Systems (`GetOEMCP`), ausserhalb von Windows 850
    pub fn system() -> Self {
        #[cfg(windows)]
        {
            Oem { codepage: unsafe { windows_sys::Win32::Globalization::GetOEMCP() } }
        }
        #[cfg(not(windows))]
        {
            Oem { codepage: 850 }
        }
    }

    /// Eine feste Codepage (Tests)
    pub fn cp850() -> Self {
        Oem { codepage: 850 }
    }

    pub fn codepage(&self) -> u32 {
        self.codepage
    }

    fn decode(&self, bytes: &[u8]) -> String {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Globalization::MultiByteToWideChar;
            if bytes.is_empty() {
                return String::new();
            }
            unsafe {
                let len = MultiByteToWideChar(self.codepage, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
                if len > 0 {
                    let mut wide = vec![0u16; len as usize];
                    let written = MultiByteToWideChar(self.codepage, 0, bytes.as_ptr(), bytes.len() as i32, wide.as_mut_ptr(), len);
                    if written > 0 {
                        wide.truncate(written as usize);
                        return String::from_utf16_lossy(&wide);
                    }
                }
            }
            String::from_utf8_lossy(bytes).into_owned()
        }
        #[cfg(not(windows))]
        {
            bytes.iter().map(|b| if *b < 0x80 { *b as char } else { CP850_HOCH[(*b - 0x80) as usize] }).collect()
        }
    }
}

/// Python schreibt UTF-8, viele Windows-Programme (ping, ipconfig, Meldungen von cmd) die OEM-Codepage.
pub fn decode_output(line: &[u8], oem: &Oem) -> String {
    match std::str::from_utf8(line) {
        Ok(text) => text.to_string(),
        Err(_) => oem.decode(line),
    }
}

#[cfg(not(windows))]
const CP850_HOCH: [char; 128] = [
    '\u{00C7}', '\u{00FC}', '\u{00E9}', '\u{00E2}', '\u{00E4}', '\u{00E0}', '\u{00E5}', '\u{00E7}',
    '\u{00EA}', '\u{00EB}', '\u{00E8}', '\u{00EF}', '\u{00EE}', '\u{00EC}', '\u{00C4}', '\u{00C5}',
    '\u{00C9}', '\u{00E6}', '\u{00C6}', '\u{00F4}', '\u{00F6}', '\u{00F2}', '\u{00FB}', '\u{00F9}',
    '\u{00FF}', '\u{00D6}', '\u{00DC}', '\u{00F8}', '\u{00A3}', '\u{00D8}', '\u{00D7}', '\u{0192}',
    '\u{00E1}', '\u{00ED}', '\u{00F3}', '\u{00FA}', '\u{00F1}', '\u{00D1}', '\u{00AA}', '\u{00BA}',
    '\u{00BF}', '\u{00AE}', '\u{00AC}', '\u{00BD}', '\u{00BC}', '\u{00A1}', '\u{00AB}', '\u{00BB}',
    '\u{2591}', '\u{2592}', '\u{2593}', '\u{2502}', '\u{2524}', '\u{00C1}', '\u{00C2}', '\u{00C0}',
    '\u{00A9}', '\u{2563}', '\u{2551}', '\u{2557}', '\u{255D}', '\u{00A2}', '\u{00A5}', '\u{2510}',
    '\u{2514}', '\u{2534}', '\u{252C}', '\u{251C}', '\u{2500}', '\u{253C}', '\u{00E3}', '\u{00C3}',
    '\u{255A}', '\u{2554}', '\u{2569}', '\u{2566}', '\u{2560}', '\u{2550}', '\u{256C}', '\u{00A4}',
    '\u{00F0}', '\u{00D0}', '\u{00CA}', '\u{00CB}', '\u{00C8}', '\u{0131}', '\u{00CD}', '\u{00CE}',
    '\u{00CF}', '\u{2518}', '\u{250C}', '\u{2588}', '\u{2584}', '\u{00A6}', '\u{00CC}', '\u{2580}',
    '\u{00D3}', '\u{00DF}', '\u{00D4}', '\u{00D2}', '\u{00F5}', '\u{00D5}', '\u{00B5}', '\u{00FE}',
    '\u{00DE}', '\u{00DA}', '\u{00DB}', '\u{00D9}', '\u{00FD}', '\u{00DD}', '\u{00AF}', '\u{00B4}',
    '\u{00AD}', '\u{00B1}', '\u{2017}', '\u{00BE}', '\u{00B6}', '\u{00A7}', '\u{00F7}', '\u{00B8}',
    '\u{00B0}', '\u{00A8}', '\u{00B7}', '\u{00B9}', '\u{00B3}', '\u{00B2}', '\u{25A0}', '\u{00A0}',
];

#[cfg(test)]
mod tests {
    use super::*;

    // Port von tests/test_streams.py (StreamsTest)

    async fn lies(data: &[u8], chunk: usize) -> Vec<Vec<u8>> {
        let mut reader = std::io::Cursor::new(data.to_vec());
        let mut teile = Vec::new();
        read_lines(&mut reader, |l| teile.push(l), chunk).await.unwrap();
        teile
    }

    #[tokio::test]
    async fn zeilen() {
        assert_eq!(lies(b"eins\nzwei\r\ndrei", CHUNK).await, vec![b"eins\n".to_vec(), b"zwei\r\n".to_vec(), b"drei".to_vec()]);
        assert!(lies(b"", CHUNK).await.is_empty());
        assert_eq!(lies(b"\n\n", CHUNK).await, vec![b"\n".to_vec(), b"\n".to_vec()]);
    }

    #[tokio::test]
    async fn sehr_lange_zeile_geht_nicht_verloren() {
        let lang = vec![b'x'; 1_000_000];
        let mut alles = b"vorher\n".to_vec();
        alles.extend_from_slice(&lang);
        alles.extend_from_slice(b"\nnachher\n");
        let teile = lies(&alles, CHUNK).await;
        assert_eq!(teile[0], b"vorher\n");
        assert_eq!(teile.last().unwrap(), b"nachher\n");
        assert_eq!(teile.concat(), alles);
        assert!(teile.iter().all(|t| t.len() <= 2 * CHUNK + 1));
    }

    #[tokio::test]
    async fn umlaute_werden_nicht_zerschnitten() {
        let text = "äöü€😀".repeat(40000).into_bytes();
        let teile = lies(&text, 1000).await;
        assert_eq!(teile.concat(), text);
        for t in &teile {
            assert!(std::str::from_utf8(t).is_ok(), "jedes Stück für sich gültig");
        }
    }

    #[test]
    fn decode_output_utf8_und_oem() {
        // Port von test_decode_output
        let oem = Oem::cp850();
        assert_eq!(decode_output("ü".as_bytes(), &oem), "ü");
        // «ausgeführt» in cp850: ü = 0x81
        assert_eq!(decode_output(b"ausgef\x81hrt", &oem), "ausgeführt");
        assert_eq!(decode_output(b"Stra\xE1e", &oem), "Straße");
    }
}
