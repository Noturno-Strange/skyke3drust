//! Identidade sem conta do Skyke3D, compatível com `assets/identity.js` e `lib/identity.js`.
//!
//! * Cada aparelho tem uma **chave do aparelho** (`k`, 32 bytes aleatórios em base64url, 43 caracteres). Ela só serve para
//!   registrar a identidade: o id é `base32(SHA-256("skyke3d/amigo|" + k))` nos 60 bits iniciais (12 letras).
//! * A identidade tem duas chaves ECDSA P-256: a **principal** (fica no aparelho) e a de **recuperação** (só no código de
//!   recuperação que o jogador guarda). As públicas vão como SPKI DER em base64url.
//! * Para entrar, o cliente pede um desafio (`auth`) e assina `"skyke3d/id/v1\nauth\n<id>\n<nonce>\n<ação>"`; a assinatura
//!   vai no formato cru `r||s` (64 bytes) em base64url, como a Web Crypto produz.
//!
//! A chave privada nunca sai do aparelho e o código de recuperação nunca é gravado em disco por este crate.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use p256::ecdsa::signature::{Signer, Verifier};
use p256::ecdsa::{Signature, SigningKey, VerifyingKey};
use p256::pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey};
use sha2::{Digest, Sha256};

/// Separação de domínio das cargas assinadas.
pub const DOMAIN: &str = "skyke3d/id/v1";
const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("chave em formato inválido")]
    BadKey,
    #[error("assinatura em formato inválido")]
    BadSignature,
    #[error("falha ao exportar a chave")]
    Export,
}

/// Os 60 primeiros bits em base32 minúsculo (12 caracteres), como `b32_60` em `lib/ids.js`.
pub fn b32_60(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(12);
    let (mut acc, mut bits, mut i) = (0u32, 0u32, 0usize);
    while out.len() < 12 {
        if bits < 5 {
            acc = (acc << 8) | u32::from(bytes.get(i).copied().unwrap_or(0));
            i += 1;
            bits += 8;
        }
        out.push(B32[((acc >> (bits - 5)) & 31) as usize] as char);
        bits -= 5;
        acc &= (1 << bits) - 1;
    }
    out
}

/// Id de amigo / id herdado de identidade a partir da chave do aparelho.
pub fn fid_of(device_key: &str) -> String {
    b32_60(&Sha256::digest(format!("skyke3d/amigo|{device_key}")))
}

/// A chave do aparelho tem o formato esperado (43 caracteres base64url)?
pub fn valid_device_key(k: &str) -> bool {
    k.len() == 43 && k.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

/// Id de identidade: 12 caracteres `[a-z2-7]`.
pub fn valid_id(id: &str) -> bool {
    id.len() == 12 && id.bytes().all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c))
}

/// Nova chave do aparelho.
pub fn new_device_key() -> String {
    B64.encode(random32())
}

/// 32 bytes do gerador do sistema operacional. Sem entropia do sistema não há como criar chaves com segurança.
fn random32() -> [u8; 32] {
    let mut b = [0u8; 32];
    #[allow(clippy::expect_used)]
    getrandom::fill(&mut b).expect("gerador aleatório do sistema indisponível");
    b
}

/// Cargas canônicas assinadas (iguais às de `assets/identity.js`).
pub mod payload {
    use super::DOMAIN;

    pub fn auth(id: &str, nonce: &str, action: &str) -> String {
        [DOMAIN, "auth", id, nonce, action].join("\n")
    }
    pub fn register(id: &str, pk: &str, rpk: &str) -> String {
        [DOMAIN, "register", id, pk, rpk].join("\n")
    }
    pub fn rotate(id: &str, generation: u64, new_pk: &str) -> String {
        [DOMAIN, "rotate", id, &generation.to_string(), new_pk].join("\n")
    }
    pub fn rotate_recovery(id: &str, rgen: u64, new_rpk: &str) -> String {
        [DOMAIN, "rotate-recovery", id, &rgen.to_string(), new_rpk].join("\n")
    }
    pub fn revoke(id: &str, epoch: u64) -> String {
        [DOMAIN, "revoke", id, &epoch.to_string()].join("\n")
    }
    pub fn recover(id: &str, generation: u64, new_pk: &str, new_rpk: Option<&str>) -> String {
        let g = generation.to_string();
        let mut v = vec![DOMAIN, "recover", id, &g, new_pk];
        if let Some(r) = new_rpk {
            v.push(r);
        }
        v.join("\n")
    }
    pub fn cancel_recovery(id: &str, generation: u64) -> String {
        [DOMAIN, "cancel-recovery", id, &generation.to_string()].join("\n")
    }
}

/// Par de chaves ECDSA P-256.
pub struct KeyPair {
    sk: SigningKey,
}

impl std::fmt::Debug for KeyPair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // nunca imprime a chave privada
        f.debug_struct("KeyPair").field("public", &self.public_b64().unwrap_or_default()).finish()
    }
}

impl KeyPair {
    pub fn generate() -> Self {
        // escalar aleatório; os raros valores fora da faixa da curva (zero ou ≥ n) são sorteados de novo
        loop {
            if let Ok(sk) = SigningKey::from_slice(&random32()) {
                return Self { sk };
            }
        }
    }

    /// Importa PKCS#8 DER (o formato que o cliente original exporta no código de recuperação).
    pub fn from_pkcs8(der: &[u8]) -> Result<Self, IdentityError> {
        SigningKey::from_pkcs8_der(der).map(|sk| Self { sk }).map_err(|_| IdentityError::BadKey)
    }

    pub fn from_pkcs8_b64(s: &str) -> Result<Self, IdentityError> {
        let der = B64.decode(s).map_err(|_| IdentityError::BadKey)?;
        Self::from_pkcs8(&der)
    }

    /// PKCS#8 DER. Quem chama é responsável por guardar com proteção (ver `sky-persistence`).
    pub fn to_pkcs8(&self) -> Result<Vec<u8>, IdentityError> {
        self.sk.to_pkcs8_der().map(|d| d.as_bytes().to_vec()).map_err(|_| IdentityError::Export)
    }

    /// Chave pública SPKI DER em base64url (o `pk`/`rpk` do protocolo).
    pub fn public_b64(&self) -> Result<String, IdentityError> {
        let der = VerifyingKey::from(&self.sk).to_public_key_der().map_err(|_| IdentityError::Export)?;
        Ok(B64.encode(der.as_bytes()))
    }

    /// Assina `data` com ECDSA/SHA-256 e devolve `r||s` em base64url (86 caracteres).
    pub fn sign_b64(&self, data: &str) -> String {
        let sig: Signature = self.sk.sign(data.as_bytes());
        B64.encode(sig.to_bytes())
    }
}

/// Confere uma assinatura `r||s` base64url com uma chave pública SPKI base64url (lado servidor).
pub fn verify_b64(public_spki_b64: &str, data: &str, sig_b64: &str) -> Result<bool, IdentityError> {
    let der = B64.decode(public_spki_b64).map_err(|_| IdentityError::BadKey)?;
    let vk = VerifyingKey::from_public_key_der(&der).map_err(|_| IdentityError::BadKey)?;
    let raw = B64.decode(sig_b64).map_err(|_| IdentityError::BadSignature)?;
    let sig = Signature::from_slice(&raw).map_err(|_| IdentityError::BadSignature)?;
    Ok(vk.verify(data.as_bytes(), &sig).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Valor conferido com `lib/ids.js` (`fidOf`) do servidor original.
    #[test]
    fn fid_matches_node() {
        assert_eq!(fid_of("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"), FID_REF);
    }
    const FID_REF: &str = include_str!("../tests/fid_ref.txt");

    #[test]
    fn sign_and_verify() {
        let kp = KeyPair::generate();
        let pk = kp.public_b64().unwrap();
        assert!((80..=200).contains(&pk.len()), "o servidor aceita pk de 80 a 200 caracteres");
        let data = payload::auth("abcdefghijk2", "nonce", "join");
        let sig = kp.sign_b64(&data);
        assert_eq!(sig.len(), 86);
        assert!(verify_b64(&pk, &data, &sig).unwrap());
        assert!(!verify_b64(&pk, "outra coisa", &sig).unwrap());
        let back = KeyPair::from_pkcs8(&kp.to_pkcs8().unwrap()).unwrap();
        assert_eq!(back.public_b64().unwrap(), pk);
    }

    #[test]
    fn formats() {
        let k = new_device_key();
        assert!(valid_device_key(&k));
        assert!(valid_id(&fid_of(&k)));
        assert_eq!(payload::recover("i", 2, "p", None), "skyke3d/id/v1\nrecover\ni\n2\np");
    }
}
