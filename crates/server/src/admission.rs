//! Ticket de admissão (`lib/admission.js`): HMAC do servidor + prova de trabalho opcional, verificado no handshake.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use indexmap::IndexMap;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sky_security::pow::zero_bits;

type HmacSha256 = Hmac<Sha256>;

pub fn pow_ok(challenge: &str, bits: u32, nonce: &str) -> bool {
    if bits == 0 {
        return true;
    }
    if nonce.is_empty() || nonce.len() > 10 || !nonce.bytes().all(|b| b.is_ascii_digit()) {
        return false;
    }
    zero_bits(&Sha256::digest(format!("{challenge}:{nonce}")), bits)
}

pub fn sig(secret: &[u8], c: &str, e: &str, d: &str, ip_key: &str) -> String {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else { return String::new() };
    mac.update(format!("v1|{c}|{e}|{d}|{ip_key}").as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

pub fn safe_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// `/ticket`: o objeto que o cliente recebe.
pub fn issue(secret: &[u8], ip_key: &str, bits: u32, ttl_ms: u64, now: u64) -> Value {
    let c = URL_SAFE_NO_PAD.encode(crate::random_bytes::<18>());
    let e = (now + ttl_ms) / 1000;
    let d = bits.min(24);
    let k = sig(secret, &c, &e.to_string(), &d.to_string(), ip_key);
    json!({"v": 1, "c": c, "e": e, "d": d, "k": k})
}

fn b64ok(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
fn digits(s: &str, max: usize) -> bool {
    (1..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_digit())
}

/// Confere o parâmetro do handshake. `Ok(bits)` ou o motivo (`formato`, `dificuldade`, `expirado`, `assinatura`, `pow`, `replay`).
pub fn verify(
    secret: &[u8],
    param: &str,
    ip_key: &str,
    now: u64,
    skew_ms: u64,
    used: Option<&mut Used>,
) -> Result<u32, &'static str> {
    let p: Vec<&str> = param.split('.').collect();
    let [c, e, d, k, n] = p.as_slice() else { return Err("formato") };
    if !b64ok(c, 16, 64) || !digits(e, 12) || !digits(d, 2) || !b64ok(k, 20, 64) {
        return Err("formato");
    }
    let exp = e.parse::<u64>().map_err(|_| "formato")? * 1000;
    let bits: u32 = d.parse().map_err(|_| "formato")?;
    if bits > 24 {
        return Err("dificuldade");
    }
    if exp < now || exp > now + skew_ms {
        return Err("expirado");
    }
    if !safe_eq(&sig(secret, c, e, d, ip_key), k) {
        return Err("assinatura");
    }
    if !pow_ok(c, bits, n) {
        return Err("pow");
    }
    if let Some(u) = used
        && !u.take(c, exp, now)
    {
        return Err("replay");
    }
    Ok(bits)
}

/// Desafios já usados (anti-replay), com teto e expiração.
#[derive(Debug)]
pub struct Used {
    map: IndexMap<String, u64>,
    max: usize,
}

impl Used {
    pub fn new(max: usize) -> Self {
        Self { map: IndexMap::new(), max }
    }

    pub fn take(&mut self, c: &str, exp: u64, now: u64) -> bool {
        if self.map.contains_key(c) {
            return false;
        }
        self.map.insert(c.to_owned(), exp);
        while self.map.first().is_some_and(|(_, &e)| e < now) {
            self.map.shift_remove_index(0);
        }
        while self.map.len() > self.max {
            self.map.shift_remove_index(0);
        }
        true
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let t = issue(b"s", "1.2.3.4", 0, 60_000, 1_000_000);
        let p = format!("{}.{}.{}.{}.0", t["c"].as_str().unwrap(), t["e"], t["d"], t["k"].as_str().unwrap());
        let mut u = Used::new(10);
        assert_eq!(verify(b"s", &p, "1.2.3.4", 1_000_000, 120_000, Some(&mut u)), Ok(0));
        assert_eq!(verify(b"s", &p, "1.2.3.4", 1_000_000, 120_000, Some(&mut u)), Err("replay"));
        assert_eq!(verify(b"s", &p, "1.2.3.5", 1_000_000, 120_000, None), Err("assinatura"));
    }
}
