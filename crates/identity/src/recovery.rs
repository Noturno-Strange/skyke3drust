//! Código de recuperação v2 (`encodeV2`/`decodeV2` em `assets/recovery.js`), byte a byte igual ao do navegador: um
//! código criado aqui abre no navegador e vice-versa.
//!
//! Formato, em bytes:
//! `[cabeçalho: versão 2 (4 bits) · cores brilhantes · amigos · chave de amigo · 0]`
//! `[12 campos do visual × 7 bits + 4 bits zerados = 11 bytes]` `[id da identidade: 12 letras × 5 bits = 8 bytes]`
//! `[tamanho (1 byte) e a chave privada de recuperação em PKCS#8 DER]` `[chave de amigo: 32 bytes, se o bit]`
//! `[amigos: quantidade (1 byte) e ids de 12 letras × 5 bits, se o bit]` `[nome em UTF-8, até 64 bytes]`
//! `[conferência: 5 primeiros bytes do SHA-256 de "skyke3d/rec/v2" + tudo o que veio antes]`,
//! em Base32 Crockford (sem I, L, O e U; O vira 0, I e L viram 1) em grupos de 5 separados por hífen.
//!
//! O código leva a chave de recuperação: é segredo do jogador e nunca é gravado em disco por este crate. O que sai de
//! [`decode_v2`] é dado não confiável: o visual ainda precisa ser validado contra o catálogo (faixas e opções retiradas).

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

/// Ordem dos campos do visual no código (`F`).
pub const FIELDS: [&str; 12] = ["fc", "skin", "eye", "hs", "hair", "hat", "hc", "shirt", "bt", "pc", "shoes", "sc"];
const FB: u32 = 7;
const VER2: u8 = 2;
const HEAD: usize = 12;
const CK: usize = 5;
const NAMEB: usize = 64;
const KEYB: usize = 32;
const MAXF: usize = 40;
const IDB: usize = 8;
const MAXB2: usize = HEAD + IDB + 1 + 256 + KEYB + 1 + (MAXF * 60).div_ceil(8) + NAMEB + CK;
const TAG2: &[u8] = b"skyke3d/rec/v2";
const A: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const FID: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Conteúdo de um código de recuperação.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RecoveryCode {
    /// Id da identidade (12 letras `[a-z2-7]`).
    pub id: String,
    /// Chave privada de recuperação (PKCS#8 DER).
    pub rec: Vec<u8>,
    /// Chave de amigo / do aparelho (43 letras base64url); vazia quando não vai no código.
    pub fk: String,
    pub name: String,
    /// Os 12 campos de [`FIELDS`] (valores crus, ainda sem validar contra o catálogo).
    pub look: BTreeMap<String, u32>,
    pub gloss: bool,
    pub friends: Vec<String>,
}

/// Por que um código não abriu (`err` do original).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CodeError {
    #[error("Código não reconhecido. Confira se copiou tudo.")]
    Format,
    #[error("O código tem um erro de digitação ou está incompleto.")]
    Check,
    #[error("Este código é de outra versão.")]
    Version,
}

fn check2(body: &[u8]) -> [u8; CK] {
    let mut h = Sha256::new();
    h.update(TAG2);
    h.update(body);
    let d = h.finalize();
    let mut out = [0u8; CK];
    out.copy_from_slice(&d[..CK]);
    out
}

fn b32(u8s: &[u8]) -> String {
    let (mut s, mut acc, mut bits) = (String::new(), 0u32, 0u32);
    for &x in u8s {
        acc = (acc << 8) | u32::from(x);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            s.push(A[((acc >> bits) & 31) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    if bits > 0 {
        s.push(A[((acc << (5 - bits)) & 31) as usize] as char);
    }
    s
}

fn b32_val(c: char) -> Option<u32> {
    match c {
        'O' => Some(0),
        'I' | 'L' => Some(1),
        _ => A.iter().position(|&a| a as char == c).map(|i| i as u32),
    }
}

fn unb32(s: &str) -> Option<Vec<u8>> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for ch in s.chars() {
        acc = (acc << 5) | b32_val(ch)?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 255) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    (acc == 0 && bits < 5).then_some(out)
}

fn b64_bytes(s: &str, exact: Option<usize>) -> Option<Vec<u8>> {
    if s.is_empty() || exact.is_some_and(|n| s.len() != n) {
        return None;
    }
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for ch in s.bytes() {
        acc = (acc << 6) | B64.iter().position(|&b| b == ch)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((acc >> bits) & 255) as u8);
        }
        acc &= (1 << bits) - 1;
    }
    Some(out)
}

/// `keyText`: 32 bytes → 43 letras (sempre fecha com o resto, como no original).
fn key_text(b: &[u8]) -> String {
    let (mut s, mut acc, mut bits) = (String::new(), 0u32, 0u32);
    for &x in b {
        acc = (acc << 8) | u32::from(x);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            s.push(B64[((acc >> bits) & 63) as usize] as char);
        }
        acc &= (1 << bits) - 1;
    }
    s.push(B64[((acc << (6 - bits)) & 63) as usize] as char);
    s
}

fn valid_fid(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|c| FID.contains(&c))
}

fn pack_ids(ids: &[&str]) -> Vec<u8> {
    let (mut out, mut acc, mut bits) = (Vec::new(), 0u32, 0u32);
    for id in ids {
        for ch in id.bytes() {
            acc = (acc << 5) | FID.iter().position(|&f| f == ch).unwrap_or(0) as u32;
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                out.push(((acc >> bits) & 255) as u8);
            }
            acc &= (1 << bits) - 1;
        }
    }
    if bits > 0 {
        out.push(((acc << (8 - bits)) & 255) as u8);
    }
    out
}

fn unpack_ids(b: &[u8], n: usize) -> Option<Vec<String>> {
    let (mut ids, mut acc, mut bits, mut j, mut id) = (Vec::new(), 0u32, 0u32, 0usize, String::new());
    while ids.len() < n {
        while bits < 5 {
            acc = (acc << 8) | u32::from(*b.get(j)?);
            j += 1;
            bits += 8;
        }
        bits -= 5;
        id.push(FID[((acc >> bits) & 31) as usize] as char);
        acc &= (1 << bits) - 1;
        if id.len() == 12 {
            ids.push(std::mem::take(&mut id));
        }
    }
    Some(ids)
}

/// `encodeV2`: `None` se o id ou a chave não servem.
pub fn encode_v2(c: &RecoveryCode) -> Option<String> {
    if !valid_fid(&c.id) || c.rec.is_empty() || c.rec.len() > 255 {
        return None;
    }
    let lb = b64_bytes(&c.fk, Some(43));
    let mut p = [0u8; HEAD];
    p[0] = (VER2 << 4)
        | if c.gloss { 8 } else { 0 }
        | if c.friends.is_empty() { 0 } else { 4 }
        | if lb.is_some() { 2 } else { 0 };
    let (mut acc, mut bits, mut j) = (0u32, 0u32, 1usize);
    for k in FIELDS {
        let v = c.look.get(k).copied().filter(|v| *v < 128).unwrap_or(0);
        acc = (acc << FB) | v;
        bits += FB;
        while bits >= 8 {
            bits -= 8;
            p[j] = ((acc >> bits) & 255) as u8;
            j += 1;
        }
        acc &= (1 << bits) - 1;
    }
    p[j] = ((acc << (8 - bits)) & 255) as u8;
    let mut body = p.to_vec();
    body.extend(pack_ids(&[&c.id]));
    body.push(c.rec.len() as u8);
    body.extend_from_slice(&c.rec);
    if let Some(lb) = lb {
        body.extend(lb);
    }
    if !c.friends.is_empty() {
        // sem repetidos (na ordem em que aparecem), só ids válidos, até MAXF
        let mut ids: Vec<&str> = Vec::new();
        for f in &c.friends {
            if valid_fid(f) && !ids.contains(&f.as_str()) {
                ids.push(f);
            }
        }
        ids.truncate(MAXF);
        body.push(ids.len() as u8);
        body.extend(pack_ids(&ids));
    }
    let nm = c.name.as_bytes();
    body.extend_from_slice(&nm[..nm.len().min(NAMEB)]);
    let ck = check2(&body);
    body.extend_from_slice(&ck);
    let s = b32(&body);
    let groups: Vec<&str> = s.as_bytes().chunks(5).filter_map(|g| std::str::from_utf8(g).ok()).collect();
    Some(groups.join("-"))
}

/// `decodeV2`: abre um código colado pelo jogador (maiúsculas ou minúsculas, com ou sem hífens e espaços).
pub fn decode_v2(s: &str) -> Result<RecoveryCode, CodeError> {
    let limit = ((MAXB2 * 8).div_ceil(5) as f64 * 1.3) as usize;
    if s.encode_utf16().count() > limit {
        return Err(CodeError::Format);
    }
    let t: String = s.to_uppercase().chars().filter(|c| !c.is_whitespace() && *c != '-').collect();
    let min = (HEAD + IDB + 2 + CK) * 8;
    let u = if t.chars().count() >= min.div_ceil(5) { unb32(&t) } else { None };
    let u = u.filter(|u| u.len() >= HEAD + IDB + 2 + CK && u.len() <= MAXB2).ok_or(CodeError::Format)?;
    let (body, ck) = u.split_at(u.len() - CK);
    if check2(body) != ck {
        return Err(CodeError::Check);
    }
    if body[0] >> 4 != VER2 {
        return Err(CodeError::Version);
    }
    if body[0] & 1 != 0 || body[HEAD - 1] & 15 != 0 {
        return Err(CodeError::Format);
    }
    let mut look = BTreeMap::new();
    let (mut acc, mut bits, mut j) = (0u32, 0u32, 1usize);
    for k in FIELDS {
        while bits < FB {
            acc = (acc << 8) | u32::from(body[j]);
            j += 1;
            bits += 8;
        }
        bits -= FB;
        look.insert(k.to_owned(), (acc >> bits) & 127);
        acc &= (1 << bits) - 1;
    }
    let id = unpack_ids(&body[HEAD..HEAD + IDB], 1).and_then(|v| v.into_iter().next()).unwrap_or_default();
    if !valid_fid(&id) {
        return Err(CodeError::Format);
    }
    let mut off = HEAD + IDB;
    let rl = usize::from(body[off]);
    off += 1;
    if rl == 0 || off + rl > body.len() {
        return Err(CodeError::Format);
    }
    let rec = body[off..off + rl].to_vec();
    off += rl;
    let mut fk = String::new();
    if body[0] & 2 != 0 {
        if off + KEYB > body.len() {
            return Err(CodeError::Format);
        }
        fk = key_text(&body[off..off + KEYB]);
        off += KEYB;
    }
    let mut friends = Vec::new();
    if body[0] & 4 != 0 {
        let n = usize::from(*body.get(off).ok_or(CodeError::Format)?);
        let nb = (n * 60).div_ceil(8);
        if n > MAXF || body.len() < off + 1 + nb {
            return Err(CodeError::Format);
        }
        friends = unpack_ids(&body[off + 1..off + 1 + nb], n).ok_or(CodeError::Format)?;
        off += 1 + nb;
    }
    if body.len() - off > NAMEB {
        return Err(CodeError::Format);
    }
    let name = String::from_utf8(body[off..].to_vec()).map_err(|_| CodeError::Format)?;
    Ok(RecoveryCode { id, rec, fk, name, look, gloss: body[0] & 8 != 0, friends })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> RecoveryCode {
        let look = FIELDS.iter().enumerate().map(|(i, k)| ((*k).to_owned(), (i as u32 * 3) % 13)).collect();
        RecoveryCode {
            id: "abcdefgh2345".into(),
            rec: (0..138u32).map(|i| (i * 7 + 3) as u8).collect(),
            fk: "AbCdEfGhIjKlMnOpQrStUvWxYz0123456789-_AbCdE".into(),
            name: "Ana Clara ✨".into(),
            look,
            gloss: true,
            friends: vec!["zzzzzzzzzzzz".into(), "a2a2a2a2a2a2".into(), "zzzzzzzzzzzz".into()],
        }
    }

    /// Vetores gerados com o `recovery.js` do jogo (`SKYREC.encodeV2`) sobre as mesmas entradas de [`sample`].
    #[test]
    fn matches_browser_encoder() {
        let c = sample();
        let code = encode_v2(&c).unwrap();
        assert_eq!(code, include_str!("../tests/recovery-v2.txt").trim());
        let mut d = decode_v2(&code).unwrap();
        // o decode tira os repetidos (como o encode já fez)
        assert_eq!(d.friends, vec!["zzzzzzzzzzzz", "a2a2a2a2a2a2"]);
        d.friends = c.friends.clone();
        assert_eq!(d, c);
        let plain = RecoveryCode { fk: String::new(), friends: vec![], name: String::new(), gloss: false, ..c };
        assert_eq!(encode_v2(&plain).unwrap(), include_str!("../tests/recovery-v2-plain.txt").trim());
    }

    #[test]
    fn tolerant_input_and_errors() {
        let code = encode_v2(&sample()).unwrap();
        // minúsculas, sem hífens, com espaços, O no lugar de 0
        let loose = code.to_lowercase().replace('-', " ");
        assert!(decode_v2(&loose).is_ok());
        let zero_as_o = code.replace('0', "O");
        assert!(decode_v2(&zero_as_o).is_ok());
        // um caractere trocado: a conferência recusa
        let mut b = code.clone().into_bytes();
        let i = b.iter().position(|c| *c != b'-' && *c != b'A').unwrap();
        b[i] = b'A';
        assert_eq!(decode_v2(&String::from_utf8(b).unwrap()), Err(CodeError::Check));
        assert_eq!(decode_v2("ABC"), Err(CodeError::Format));
        assert_eq!(decode_v2(&"A".repeat(5000)), Err(CodeError::Format));
        assert!(encode_v2(&RecoveryCode { id: "curto".into(), ..sample() }).is_none());
    }
}
