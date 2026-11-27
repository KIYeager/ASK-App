//! Ausweich-DNS für die Anfragen der App an ASK (ENTSCHEIDE E57, E58, E60): Findet die App den Namen von ASK nicht
//! (der Browser aber schon), fragt sie zuerst Quad9 über DNS-over-HTTPS (RFC 8484) und nur bei einem Fehler wieder das
//! DNS von Windows («automatic» wie in Chromium), damit Namen im Netz der Schule weiter gehen. Nur für den Namen der
//! mitgelieferten Adresse der Schule; alle anderen Namen gehen wie immer an Windows.
//!
//! In Electron stellte `app.configureHostResolver` das für die ganze App um. WebView2 lässt das nicht zur Laufzeit zu;
//! hier gilt es für die Anfragen, die die App selbst schickt (Anmeldung prüfen, Modelle, Chats). Lädt die ASK-Seite in
//! der App trotzdem nicht, geht die Anmeldung über das Token aus dem Browser.

use anyhow::{anyhow, bail, Result};
use parking_lot::Mutex;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

pub const DOH_URL: &str = "https://dns.quad9.net/dns-query";
/// Adressen von dns.quad9.net, damit die Abfrage selbst nicht am DNS von Windows hängt
const QUAD9: [&str; 2] = ["9.9.9.9:443", "149.112.112.112:443"];

/// Namensauflösung der App mit Ausweg über Quad9. Klonen über `Arc`.
pub struct FallbackDns {
    enabled: AtomicBool,
    host: Mutex<String>,
    client: reqwest::Client,
}

impl Default for FallbackDns {
    fn default() -> Self {
        Self::new()
    }
}

impl FallbackDns {
    pub fn new() -> Self {
        let addrs: Vec<SocketAddr> = QUAD9.iter().filter_map(|a| a.parse().ok()).collect();
        let client = reqwest::Client::builder()
            .resolve_to_addrs("dns.quad9.net", &addrs)
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_default();
        Self { enabled: AtomicBool::new(false), host: Mutex::new(String::new()), client }
    }

    /// Für welchen Namen der Ausweg gilt (Host der mitgelieferten Adresse)
    pub fn set_host(&self, url: &str) {
        *self.host.lock() = url::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_lowercase)).unwrap_or_default();
    }

    /// Ausweg einschalten; false, wenn er schon an war
    pub fn enable(&self) -> bool {
        !self.enabled.swap(true, Ordering::SeqCst)
    }

    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    fn applies_to(&self, host: &str) -> bool {
        self.enabled() && !host.is_empty() && *self.host.lock() == host.to_lowercase()
    }

    /// Adressen über Quad9 (A und AAAA)
    pub async fn lookup(&self, host: &str) -> Result<Vec<IpAddr>> {
        let mut out = Vec::new();
        let mut last = None;
        for qtype in [1u16, 28] {
            match self.query(host, qtype).await {
                Ok(mut a) => out.append(&mut a),
                Err(e) => last = Some(e),
            }
        }
        if out.is_empty() {
            return Err(last.unwrap_or_else(|| anyhow!("Quad9 kennt {host} nicht")));
        }
        Ok(out)
    }

    async fn query(&self, host: &str, qtype: u16) -> Result<Vec<IpAddr>> {
        let res = self.client.post(DOH_URL)
            .header("content-type", "application/dns-message")
            .header("accept", "application/dns-message")
            .body(build_query(host, qtype)?)
            .send()
            .await?;
        if !res.status().is_success() {
            bail!("Quad9 antwortet mit HTTP {}", res.status().as_u16());
        }
        parse_answers(&res.bytes().await?)
    }
}

impl reqwest::dns::Resolve for FallbackDns {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_string();
        let doh = self.applies_to(&host).then(|| (self.client.clone(), self.host.lock().clone()));
        Box::pin(async move {
            if let Some((client, h)) = doh {
                let me = FallbackDns { enabled: AtomicBool::new(true), host: Mutex::new(h), client };
                if let Ok(ips) = me.lookup(&host).await {
                    let addrs: reqwest::dns::Addrs = Box::new(ips.into_iter().map(|ip| SocketAddr::new(ip, 0)));
                    return Ok(addrs);
                }
            }
            // DNS von Windows (bzw. des Systems), auch wenn Quad9 nicht antwortet
            let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let addrs: reqwest::dns::Addrs = Box::new(addrs.into_iter());
            Ok(addrs)
        })
    }
}

/// DNS-Anfrage im Format von RFC 1035 (Kennung 0 wie in RFC 8484 empfohlen, Rekursion erwünscht)
pub fn build_query(host: &str, qtype: u16) -> Result<Vec<u8>> {
    let mut q = vec![0, 0, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
    for label in host.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 || !label.is_ascii() {
            bail!("Ungültiger Name: {host}");
        }
        q.push(label.len() as u8);
        q.extend_from_slice(label.as_bytes());
    }
    q.push(0);
    q.extend_from_slice(&qtype.to_be_bytes());
    q.extend_from_slice(&[0, 1]); // IN
    Ok(q)
}

fn skip_name(buf: &[u8], mut i: usize) -> Result<usize> {
    loop {
        let len = *buf.get(i).ok_or_else(|| anyhow!("DNS-Antwort zu kurz"))? as usize;
        if len == 0 {
            return Ok(i + 1);
        }
        if len & 0xC0 == 0xC0 {
            return Ok(i + 2); // Verweis auf einen früheren Namen
        }
        i += 1 + len;
    }
}

/// Adressen (A und AAAA) aus einer DNS-Antwort
pub fn parse_answers(buf: &[u8]) -> Result<Vec<IpAddr>> {
    if buf.len() < 12 {
        bail!("DNS-Antwort zu kurz");
    }
    let rcode = buf[3] & 0x0F;
    if rcode != 0 {
        bail!("DNS-Fehler {rcode}");
    }
    let qd = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let an = u16::from_be_bytes([buf[6], buf[7]]) as usize;
    let mut i = 12;
    for _ in 0..qd {
        i = skip_name(buf, i)? + 4;
    }
    let mut out = Vec::new();
    for _ in 0..an {
        i = skip_name(buf, i)?;
        let head = buf.get(i..i + 10).ok_or_else(|| anyhow!("DNS-Antwort zu kurz"))?;
        let rtype = u16::from_be_bytes([head[0], head[1]]);
        let len = u16::from_be_bytes([head[8], head[9]]) as usize;
        let data = buf.get(i + 10..i + 10 + len).ok_or_else(|| anyhow!("DNS-Antwort zu kurz"))?;
        match (rtype, len) {
            (1, 4) => out.push(IpAddr::V4(Ipv4Addr::new(data[0], data[1], data[2], data[3]))),
            (28, 16) => {
                let mut a = [0u8; 16];
                a.copy_from_slice(data);
                out.push(IpAddr::V6(Ipv6Addr::from(a)));
            }
            _ => {} // CNAME und anderes: die Adressen folgen in weiteren Einträgen
        }
        i += 10 + len;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anfrage_und_antwort() {
        let q = build_query("ask.ict-tfbern.ch", 1).unwrap();
        assert_eq!(&q[..4], &[0, 0, 1, 0]);
        assert_eq!(q[12], 3);
        assert_eq!(&q[13..16], b"ask");
        assert!(build_query("a..b", 1).is_err());

        // Antwort: Frage, CNAME, dann A und AAAA mit Verweisen auf den Namen
        let mut r = q.clone();
        r[2] = 0x81;
        r[3] = 0x80;
        r[7] = 3; // drei Antworten
        let mut rr = |rtype: u16, data: &[u8]| {
            r.extend_from_slice(&[0xC0, 12]);
            r.extend_from_slice(&rtype.to_be_bytes());
            r.extend_from_slice(&[0, 1, 0, 0, 1, 0]);
            r.extend_from_slice(&(data.len() as u16).to_be_bytes());
            r.extend_from_slice(data);
        };
        rr(5, &[3, b'w', b'w', b'w', 0xC0, 12]);
        rr(1, &[192, 0, 2, 7]);
        rr(28, &[0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
        let ips = parse_answers(&r).unwrap();
        assert_eq!(ips, vec!["192.0.2.7".parse::<IpAddr>().unwrap(), "2001:db8::1".parse::<IpAddr>().unwrap()]);

        let mut nx = q.clone();
        nx[3] = 0x83; // NXDOMAIN
        assert!(parse_answers(&nx).is_err());
        assert!(parse_answers(&[0, 1]).is_err());
    }

    #[test]
    fn nur_fuer_die_adresse_der_schule_und_erst_nach_dem_einschalten() {
        let dns = FallbackDns::new();
        dns.set_host("https://ASK.ict-tfbern.ch/");
        assert!(!dns.applies_to("ask.ict-tfbern.ch"));
        assert!(dns.enable());
        assert!(!dns.enable(), "schon an");
        assert!(dns.applies_to("ask.ict-tfbern.ch"));
        assert!(!dns.applies_to("github.com"));
    }
}
