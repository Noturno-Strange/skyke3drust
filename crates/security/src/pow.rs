//! Provas de trabalho do protocolo original.
//!
//! * Ticket de admissão (`lib/admission.js`): `GET /ticket` devolve `{v:1,c,e,d,k}`; o cliente acha um `n` decimal tal que
//!   `SHA-256("c:n")` tenha `d` bits zero à esquerda e conecta com `?ticket=c.e.d.k.n`.
//! * Registro de identidade (`lib/idpow.js` / `assets/identity.js`): `SHA-256("skyke3d/idreg|k|pk|rpk|n")` com `bits` zeros.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

/// Dificuldade máxima aceita (o servidor limita a 24 bits).
pub const MAX_BITS: u32 = 24;

/// Tem pelo menos `bits` bits zero à esquerda?
pub fn zero_bits(hash: &[u8], bits: u32) -> bool {
    let mut have = 0u32;
    for &b in hash {
        if have >= bits {
            return true;
        }
        if b == 0 {
            have += 8;
            continue;
        }
        return have + b.leading_zeros() >= bits;
    }
    have >= bits
}

/// Ticket recebido de `/ticket`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ticket {
    pub c: String,
    pub e: u64,
    pub d: u32,
    pub k: String,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PowError {
    #[error("dificuldade fora do limite: {0} bits")]
    TooHard(u32),
    #[error("ticket com formato inválido")]
    Format,
}

fn valid_b64(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// Resolve um ticket e devolve o parâmetro `ticket` pronto para a URL do WebSocket.
pub fn solve_ticket(t: &Ticket) -> Result<String, PowError> {
    if t.d > MAX_BITS {
        return Err(PowError::TooHard(t.d));
    }
    if !valid_b64(&t.c, 16, 64) || !valid_b64(&t.k, 20, 64) {
        return Err(PowError::Format);
    }
    let base = format!("{}:", t.c);
    let n = search(t.d, |n, h| {
        h.update(base.as_bytes());
        h.update(n.as_bytes());
    });
    Ok(format!("{}.{}.{}.{}.{}", t.c, t.e, t.d, t.k, n))
}

/// Resolve a prova de trabalho do registro de identidade.
pub fn solve_idreg(k: &str, pk: &str, rpk: &str, bits: u32) -> Result<String, PowError> {
    if bits > MAX_BITS {
        return Err(PowError::TooHard(bits));
    }
    let head = format!("skyke3d/idreg|{k}|{pk}|{rpk}|");
    Ok(search(bits, |n, h| {
        h.update(head.as_bytes());
        h.update(n.as_bytes());
    }))
}

/// Confere a prova do registro (lado servidor).
pub fn idreg_ok(k: &str, pk: &str, rpk: &str, pow: &str, bits: u32) -> bool {
    if bits == 0 {
        return true;
    }
    if pow.is_empty() || pow.len() > 12 || !pow.bytes().all(|c| c.is_ascii_digit()) {
        return false;
    }
    let h = Sha256::digest(format!("skyke3d/idreg|{k}|{pk}|{rpk}|{pow}"));
    zero_bits(&h, bits)
}

fn search(bits: u32, feed: impl Fn(&str, &mut Sha256)) -> String {
    let mut buf = itoa_buf();
    for n in 0u64.. {
        let s = fmt_u64(n, &mut buf);
        let mut h = Sha256::new();
        feed(s, &mut h);
        if zero_bits(&h.finalize(), bits) {
            return s.to_owned();
        }
    }
    unreachable!("o espaço de u64 não se esgota antes de achar uma prova de até 24 bits")
}

fn itoa_buf() -> [u8; 20] {
    [0; 20]
}
fn fmt_u64(mut n: u64, buf: &mut [u8; 20]) -> &str {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    // só dígitos ASCII
    std::str::from_utf8(&buf[i..]).unwrap_or("0")
}

type HmacSha256 = Hmac<Sha256>;

/// Emite um ticket (lado servidor). `ip_key` é o IPv4 ou o prefixo IPv6 /64 do cliente.
pub fn issue_ticket(secret: &[u8], ip_key: &str, bits: u32, now_ms: u64, ttl_ms: u64, challenge: &[u8; 18]) -> Ticket {
    let c = URL_SAFE_NO_PAD.encode(challenge);
    let e = (now_ms + ttl_ms) / 1000;
    let d = bits.min(MAX_BITS);
    let k = ticket_sig(secret, &c, e, d, ip_key);
    Ticket { c, e, d, k }
}

fn ticket_sig(secret: &[u8], c: &str, e: u64, d: u32, ip_key: &str) -> String {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else { return String::new() };
    mac.update(format!("v1|{c}|{e}|{d}|{ip_key}").as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

#[derive(Debug, PartialEq, Eq)]
pub enum TicketReject {
    Format,
    Expired,
    Signature,
    Pow,
}

/// Confere o parâmetro `ticket` (lado servidor). Retorna o desafio para o controle de reuso.
pub fn verify_ticket(
    secret: &[u8],
    param: &str,
    ip_key: &str,
    now_ms: u64,
    skew_ms: u64,
) -> Result<String, TicketReject> {
    let p: Vec<&str> = param.split('.').collect();
    let [c, e, d, k, n] = p.as_slice() else { return Err(TicketReject::Format) };
    let digits = |s: &str, max: usize| !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_digit());
    if !valid_b64(c, 16, 64) || !digits(e, 12) || !digits(d, 2) || !valid_b64(k, 20, 64) {
        return Err(TicketReject::Format);
    }
    let e: u64 = e.parse().map_err(|_| TicketReject::Format)?;
    let d: u32 = d.parse().map_err(|_| TicketReject::Format)?;
    if d > MAX_BITS {
        return Err(TicketReject::Format);
    }
    if e * 1000 + skew_ms < now_ms {
        return Err(TicketReject::Expired);
    }
    let want = ticket_sig(secret, c, e, d, ip_key);
    if !constant_eq(want.as_bytes(), k.as_bytes()) {
        return Err(TicketReject::Signature);
    }
    if d > 0 {
        if !digits(n, 10) {
            return Err(TicketReject::Pow);
        }
        if !zero_bits(&Sha256::digest(format!("{c}:{n}")), d) {
            return Err(TicketReject::Pow);
        }
    }
    Ok((*c).to_owned())
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_bit_counting() {
        assert!(zero_bits(&[0, 0, 0x0f], 20));
        assert!(!zero_bits(&[0, 0, 0x1f], 20));
        assert!(zero_bits(&[0xff], 0));
        assert!(zero_bits(&[0x00, 0x80], 8));
        assert!(!zero_bits(&[0x00, 0x80], 9));
    }

    #[test]
    fn ticket_issue_solve_verify() {
        let secret = b"segredo-de-teste";
        let t = issue_ticket(secret, "203.0.113.9", 10, 1_000_000, 60_000, &[7; 18]);
        let param = solve_ticket(&t).unwrap();
        assert!(verify_ticket(secret, &param, "203.0.113.9", 1_000_000, 120_000).is_ok());
        assert_eq!(verify_ticket(secret, &param, "203.0.113.10", 1_000_000, 120_000), Err(TicketReject::Signature));
        assert_eq!(verify_ticket(secret, &param, "203.0.113.9", 10_000_000, 0), Err(TicketReject::Expired));
        let head = param.rsplit_once('.').unwrap().0;
        let wrong = (0u64..).find(|n| !zero_bits(&Sha256::digest(format!("{}:{n}", t.c)), t.d)).unwrap();
        let bad = format!("{head}.{wrong}");
        assert_eq!(verify_ticket(secret, &bad, "203.0.113.9", 1_000_000, 120_000), Err(TicketReject::Pow));
    }

    #[test]
    fn idreg_roundtrip() {
        let n = solve_idreg("k", "pk", "rpk", 8).unwrap();
        assert!(idreg_ok("k", "pk", "rpk", &n, 8));
        assert!(!idreg_ok("k", "pk", "outra", &n, 16));
        assert_eq!(solve_idreg("k", "pk", "rpk", 25), Err(PowError::TooHard(25)));
    }
}
