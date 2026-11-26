//! Passkeys (`lib/webauthn.js`): confere o cadastro e a entrada feitos pelo navegador (origem, desafio, hash do rpId,
//! presença e verificação do usuário, contador contra clonagem) e a assinatura com ES256, Ed25519 ou RSA ≥ 2048.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const FLAG_UP: u8 = 0x01;
const FLAG_UV: u8 = 0x04;
const FLAG_AT: u8 = 0x40;

fn b64(s: &str, max: usize) -> Option<Vec<u8>> {
    if s.is_empty() || s.len() > max || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return None;
    }
    URL_SAFE_NO_PAD.decode(s.trim_end_matches('=')).ok().filter(|b| !b.is_empty())
}

/// Chave da credencial válida para o algoritmo?
fn key_ok(spki: &[u8], alg: i64) -> bool {
    match alg {
        -7 => {
            use p256::pkcs8::DecodePublicKey;
            p256::ecdsa::VerifyingKey::from_public_key_der(spki).is_ok()
        }
        -8 => {
            use ed25519_dalek::pkcs8::DecodePublicKey;
            ed25519_dalek::VerifyingKey::from_public_key_der(spki).is_ok()
        }
        -257 => {
            use rsa::pkcs8::DecodePublicKey;
            use rsa::traits::PublicKeyParts;
            rsa::RsaPublicKey::from_public_key_der(spki).is_ok_and(|k| k.n().bits() >= 2048)
        }
        _ => false,
    }
}

fn verify_sig(spki: &[u8], alg: i64, data: &[u8], sig: &[u8]) -> bool {
    match alg {
        -7 => {
            use p256::ecdsa::signature::Verifier;
            use p256::pkcs8::DecodePublicKey;
            let (Ok(k), Ok(s)) =
                (p256::ecdsa::VerifyingKey::from_public_key_der(spki), p256::ecdsa::DerSignature::from_bytes(sig))
            else {
                return false;
            };
            k.verify(data, &s).is_ok()
        }
        -8 => {
            use ed25519_dalek::Verifier;
            use ed25519_dalek::pkcs8::DecodePublicKey;
            let (Ok(k), Ok(s)) =
                (ed25519_dalek::VerifyingKey::from_public_key_der(spki), ed25519_dalek::Signature::from_slice(sig))
            else {
                return false;
            };
            k.verify(data, &s).is_ok()
        }
        -257 => {
            use rsa::pkcs8::DecodePublicKey;
            use rsa::signature::Verifier;
            let Ok(k) = rsa::RsaPublicKey::from_public_key_der(spki) else { return false };
            let (vk, Ok(s)) =
                (rsa::pkcs1v15::VerifyingKey::<rsa::sha2::Sha256>::new(k), rsa::pkcs1v15::Signature::try_from(sig))
            else {
                return false;
            };
            vk.verify(data, &s).is_ok()
        }
        _ => false,
    }
}

struct AuthData<'a> {
    rp: &'a [u8],
    flags: u8,
    sc: u32,
    cred: Option<&'a [u8]>,
}

fn parse_auth_data(ad: &[u8]) -> Option<AuthData<'_>> {
    if ad.len() < 37 {
        return None;
    }
    let flags = ad[32];
    let mut cred = None;
    if flags & FLAG_AT != 0 {
        if ad.len() < 55 {
            return None;
        }
        let n = u16::from_be_bytes([ad[53], ad[54]]) as usize;
        if !(1..=1023).contains(&n) || ad.len() < 55 + n {
            return None;
        }
        cred = Some(&ad[55..55 + n]);
    }
    Some(AuthData { rp: &ad[..32], flags, sc: u32::from_be_bytes([ad[33], ad[34], ad[35], ad[36]]), cred })
}

fn check_client_data(cd: &[u8], typ: &str, challenge: &str, origin: &str) -> &'static str {
    let Ok(j) = serde_json::from_slice::<Value>(cd) else { return "client-data" };
    if !j.is_object() {
        return "client-data";
    }
    if j["type"] != typ {
        return "type";
    }
    if !j["challenge"].as_str().is_some_and(|c| crate::admission::safe_eq(c, challenge)) {
        return "challenge";
    }
    if j["origin"] != origin || j["crossOrigin"] == true {
        return "origin";
    }
    ""
}

/// rpId das passkeys a partir da origem da página (https, ou http em localhost).
pub fn rp_id_of(origin: &str) -> String {
    let Ok(u) = url::Url::parse(origin) else { return String::new() };
    let h = u.host_str().unwrap_or("").to_lowercase();
    let local = h == "localhost" || h.ends_with(".localhost");
    if u.scheme() != "https" && !(u.scheme() == "http" && local) {
        return String::new();
    }
    if h.is_empty() || h.bytes().all(|b| b.is_ascii_digit() || b == b'.') || h.contains(':') || h.starts_with('[') {
        return String::new();
    }
    h
}

/// Identificador do usuário dado ao navegador (não revela o id de identidade).
pub fn user_handle(id: &str) -> String {
    URL_SAFE_NO_PAD.encode(&Sha256::digest(format!("skyke3d/passkey-user|{id}"))[..16])
}

fn rp_hash(rp: &str) -> [u8; 32] {
    Sha256::digest(rp.as_bytes()).into()
}

/// Cadastro: devolve a credencial para guardar (`cid`, `pk`, `alg`, `sc`) ou o motivo.
#[allow(clippy::too_many_arguments)]
pub fn verify_registration(
    origin: &str,
    rp_id: &str,
    challenge: &str,
    cd: &str,
    ad: &str,
    spki: &str,
    alg: i64,
    cid: &str,
) -> Result<Value, &'static str> {
    let (Some(cdb), Some(adb)) = (b64(cd, 2048), b64(ad, 2048)) else { return Err("invalid") };
    if !(16..=1366).contains(&cid.len()) || !cid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
        return Err("invalid");
    }
    let why = check_client_data(&cdb, "webauthn.create", challenge, origin);
    if !why.is_empty() {
        return Err(why);
    }
    let a = parse_auth_data(&adb).ok_or("invalid")?;
    let cred = a.cred.ok_or("invalid")?;
    if a.rp != rp_hash(rp_id) {
        return Err("rp");
    }
    if a.flags & FLAG_UP == 0 || a.flags & FLAG_UV == 0 {
        return Err("uv");
    }
    if URL_SAFE_NO_PAD.encode(cred) != cid {
        return Err("invalid");
    }
    if ![-7, -8, -257].contains(&alg) || !b64(spki, 1200).is_some_and(|k| key_ok(&k, alg)) {
        return Err("alg");
    }
    Ok(json!({"cid": cid, "pk": spki, "alg": alg, "sc": a.sc}))
}

/// Entrada: confere a assinatura com a chave guardada; devolve o contador novo.
#[allow(clippy::too_many_arguments)]
pub fn verify_assertion(
    origin: &str,
    rp_id: &str,
    challenge: &str,
    cd: &str,
    ad: &str,
    sig: &str,
    pk: &str,
    alg: i64,
    sc: i64,
) -> Result<i64, &'static str> {
    let (Some(cdb), Some(adb), Some(sg)) = (b64(cd, 2048), b64(ad, 2048), b64(sig, 1024)) else {
        return Err("invalid");
    };
    let why = check_client_data(&cdb, "webauthn.get", challenge, origin);
    if !why.is_empty() {
        return Err(why);
    }
    let a = parse_auth_data(&adb).ok_or("invalid")?;
    if a.rp != rp_hash(rp_id) {
        return Err("rp");
    }
    if a.flags & FLAG_UP == 0 || a.flags & FLAG_UV == 0 {
        return Err("uv");
    }
    let key = b64(pk, 1200).filter(|k| key_ok(k, alg)).ok_or("alg")?;
    let mut data = adb.clone();
    data.extend_from_slice(&Sha256::digest(&cdb));
    if !verify_sig(&key, alg, &data, &sg) {
        return Err("bad-signature");
    }
    let n = i64::from(a.sc);
    if (n != 0 || sc != 0) && n <= sc {
        return Err("clone");
    }
    Ok(n)
}
