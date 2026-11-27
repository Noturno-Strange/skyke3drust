//! Registro de eventos de segurança (`lib/seclog.js`): sem segredos e sem IP (id e IP entram só como HMAC).

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use indexmap::IndexMap;
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;

const EVENTS: [&str; 16] = [
    "identity.create",
    "identity.remove",
    "auth.fail",
    "key.rotate",
    "recovery-code.rotate",
    "recovery.grant",
    "keypass.grant",
    "recovery.request",
    "recovery.apply",
    "recovery.cancel",
    "sessions.revoke",
    "passkey.add",
    "passkey.remove",
    "passkey.login",
    "passkey.fail",
    "passkey.clone",
];

/// Campos opcionais de um evento.
#[derive(Debug, Default, Clone)]
pub struct Ev<'a> {
    pub id: &'a str,
    pub ip: &'a str,
    pub ok: Option<bool>,
    pub reason: &'a str,
    pub cred: &'a str,
    pub n: Option<i64>,
}

#[derive(Debug)]
pub struct SecLog {
    file: Option<PathBuf>,
    max_bytes: u64,
    key: Vec<u8>,
    keep: usize,
    recent: VecDeque<Value>,
    counts: IndexMap<String, u64>,
    bytes: u64,
}

/// Data e hora ISO 8601 em UTC (`toISOString`).
pub fn iso(ms: u64) -> String {
    let secs = ms / 1000;
    let (h, mi, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let (y, mo, d) = civil(secs / 86400);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{:03}Z", ms % 1000)
}

/// Dias desde 1970 → (ano, mês, dia) no calendário gregoriano.
pub fn civil(days: u64) -> (i64, u32, u32) {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

impl SecLog {
    pub fn new(file: &str) -> Self {
        let file = (!file.is_empty()).then(|| PathBuf::from(file));
        let key = load_secret(file.as_ref());
        let bytes = file.as_ref().and_then(|f| std::fs::metadata(f).ok()).map_or(0, |m| m.len());
        Self {
            file,
            max_bytes: 2 * 1024 * 1024,
            key,
            keep: 200,
            recent: VecDeque::new(),
            counts: IndexMap::new(),
            bytes,
        }
    }

    fn mac(&self, kind: &str, v: &str, len: usize) -> String {
        let Ok(mut m) = Hmac::<Sha256>::new_from_slice(&self.key) else { return String::new() };
        m.update(format!("{kind}|{v}").as_bytes());
        URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())[..len].to_owned()
    }

    pub fn who_of(&self, id: &str) -> String {
        self.mac("id", id, 16)
    }

    pub fn log(&mut self, ev: &str, e: Ev<'_>) {
        if !EVENTS.contains(&ev) {
            return;
        }
        let t = crate::now();
        let at = iso(t);
        let day = &at[..10];
        let ok = e.ok.unwrap_or(true);
        let mut o = json!({"at": at, "ev": ev, "ok": ok});
        if !e.id.is_empty() {
            o["who"] = json!(self.mac("id", e.id, 16));
        }
        if !e.ip.is_empty() {
            o["from"] = json!(self.mac("ip", &format!("{day}|{}", e.ip), 12));
        }
        if !e.cred.is_empty() {
            o["cred"] = json!(self.mac("cred", e.cred, 12));
        }
        if (1..=24).contains(&e.reason.len()) && e.reason.bytes().all(|b| b.is_ascii_lowercase() || b == b'-') {
            o["reason"] = json!(e.reason);
        }
        if let Some(n) = e.n.filter(|n| *n >= 0) {
            o["n"] = json!(n);
        }
        let k = format!("{ev}{}", if ok { "" } else { ":fail" });
        *self.counts.entry(k).or_default() += 1;
        let line = format!("{o}\n");
        self.recent.push_back(o);
        while self.recent.len() > self.keep {
            self.recent.pop_front();
        }
        let Some(f) = &self.file else { return };
        if self.bytes + line.len() as u64 > self.max_bytes {
            let mut old = f.as_os_str().to_owned();
            old.push(".1");
            let _ = std::fs::rename(f, PathBuf::from(old));
            self.bytes = 0;
        }
        let mut o = std::fs::OpenOptions::new();
        o.append(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        match o.open(f).and_then(|mut h| h.write_all(line.as_bytes())) {
            Ok(()) => self.bytes += line.len() as u64,
            Err(x) => tracing::error!("seclog: {x}"),
        }
    }

    pub fn stats(&self) -> Value {
        let recent: Vec<&Value> = self.recent.iter().rev().take(50).collect();
        json!({"counts": self.counts, "recent": recent, "bytes": self.bytes, "file": self.file.is_some()})
    }
}

fn load_secret(file: Option<&PathBuf>) -> Vec<u8> {
    let fresh = crate::random_bytes::<32>().to_vec();
    let Some(f) = file else { return fresh };
    let kf = f.parent().unwrap_or(std::path::Path::new(".")).join("security.key");
    if let Ok(s) = std::fs::read(&kf)
        && s.len() >= 32
    {
        return s;
    }
    if let Some(d) = kf.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    match o.open(&kf).and_then(|mut h| h.write_all(&fresh)) {
        Ok(()) => fresh,
        Err(_) => std::fs::read(&kf).ok().filter(|s| s.len() >= 32).unwrap_or(fresh),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_795_651_200_123), "2026-11-26T00:00:00.123Z");
    }
}
