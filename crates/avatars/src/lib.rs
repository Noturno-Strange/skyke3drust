//! Avatares do Skyke3D.
//!
//! O avatar do jogo original é gerado por código (`assets/app-avatar.js`): corpo em tubos e faixas de esfera, cabelo
//! esculpido por campo de distância, chapéus e uma textura de rosto calculada por pixel. Este crate porta essa geração
//! para Rust, parte por parte, e cada parte é conferida contra o que o navegador gera com o mesmo visual (`cfg`).
//!
//! Portado: paletas e validação do `cfg` ([`cfg`]), textura da cabeça ([`head`]), cores do cabelo e dos chapéus
//! sobre a geometria exportada ([`hair`]), pernas e pés ([`body`]).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod body;
pub mod cfg;
pub mod hair;
pub mod head;

pub use cfg::{Cfg, Palette, Palettes};
