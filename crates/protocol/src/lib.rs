//! Protocolo de rede do Skyke3D, compatível com o servidor Node.js original.
//!
//! O transporte é um WebSocket único por jogador: mensagens de texto são JSON ([`json`]), mensagens binárias são
//! posições e voz ([`binary`]). A documentação completa está em `docs/auditoria/protocolo.md`.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod binary;
pub mod json;

pub use binary::{ActorState, VoiceCodec};
pub use json::{AvatarCfg, ClientMsg, Joined, PlayerPub, ServerMsg};

/// Versão do protocolo original com que este crate foi conferido.
pub const ORIGINAL_VERSION: &str = "3.30.1";
