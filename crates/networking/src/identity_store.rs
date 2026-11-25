//! Identidade guardada no aparelho para o cliente nativo.
//!
//! Guarda a chave do aparelho, o id e a chave **principal** (PKCS#8, arquivo só do dono). A chave de recuperação nunca
//! é gravada: igual ao navegador, ela só existe no código de recuperação que o jogador guarda.

use serde::{Deserialize, Serialize};
use sky_identity::KeyPair;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredIdentity {
    pub v: u32,
    /// Chave do aparelho (43 caracteres).
    pub k: String,
    pub id: String,
    pub pk: String,
    pub rpk: String,
    #[serde(default)]
    pub generation: u64,
    #[serde(default)]
    pub epoch: u64,
    /// Chave principal PKCS#8 em base64url.
    pub primary: String,
}

impl StoredIdentity {
    pub fn primary_key(&self) -> Option<KeyPair> {
        KeyPair::from_pkcs8_b64(&self.primary).ok()
    }

    /// Confere formato e coerência (dado lido do disco é não confiável).
    pub fn is_valid(&self) -> bool {
        self.v == 1
            && sky_identity::valid_device_key(&self.k)
            && sky_identity::valid_id(&self.id)
            && self.primary_key().is_some_and(|kp| kp.public_b64().ok().as_deref() == Some(self.pk.as_str()))
    }

    pub fn load(path: &Path) -> Option<Self> {
        match sky_persistence::load_json::<StoredIdentity>(path, 16 * 1024) {
            Ok(Some(s)) if s.is_valid() => Some(s),
            Ok(Some(_)) => {
                tracing::warn!("identidade em {} inválida; ignorada", path.display());
                None
            }
            Ok(None) => None,
            Err(e) => {
                tracing::warn!("identidade: {e}");
                None
            }
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), sky_persistence::PersistError> {
        sky_persistence::save_json(path, self, true)
    }
}
