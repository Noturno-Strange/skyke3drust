//! Servidor do Skyke3D em Rust: o mesmo protocolo do servidor Node.js (`server.js` e `lib/`), para os jogadores do
//! navegador e do cliente nativo, num executável só, no estilo de um servidor de Minecraft (`server.properties`,
//! console com comandos, pasta de dados e registros).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod admission;
pub mod auth;
pub mod banmgr;
pub mod capacity;
pub mod catalog;
pub mod config;
pub mod identity;
pub mod limits;
pub mod names;
pub mod netutil;
pub mod persist;
pub mod profanity;
pub mod seclog;
pub mod validate;

/// Agora em milissegundos desde 1970 (`Date.now()`).
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Bytes aleatórios do sistema.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    let _ = getrandom::fill(&mut b);
    b
}

/// Bytes em hexadecimal minúsculo.
pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}
