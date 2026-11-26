//! Servidor do Skyke3D em Rust: o mesmo protocolo do servidor Node.js (`server.js` e `lib/`), para os jogadores do
//! navegador e do cliente nativo, num executável só, no estilo de um servidor de Minecraft (`server.properties`,
//! console com comandos, pasta de dados e registros).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod admission;
pub mod auth;
pub mod badges;
pub mod banmgr;
pub mod bots;
pub mod capacity;
pub mod catalog;
pub mod config;
pub mod cook;
pub mod farm;
pub mod game;
pub mod gifts;
pub mod guard;
pub mod health;
pub mod http;
pub mod identity;
pub mod identity_ws;
pub mod levels;
pub mod limits;
pub mod listener;
pub mod media;
pub mod metrics;
pub mod moderation;
pub mod names;
pub mod netutil;
pub mod persist;
pub mod pet_genes;
pub mod pets;
pub mod playerdata;
pub mod profanity;
pub mod pubprofile;
pub mod radio;
pub mod seclog;
pub mod social;
pub mod validate;
pub mod voice;
pub mod web;
pub mod webauthn;

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

/// Id aleatório de 12 letras base32 (`newId`).
pub fn new_id() -> String {
    sky_identity::b32_60(&random_bytes::<12>())
}
