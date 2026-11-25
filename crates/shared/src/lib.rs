//! Tipos e constantes compartilhados por cliente, servidor e ferramentas.
//!
//! Os valores daqui espelham o Skyke3D original (`assets/catalog.js`, `lib/validate.js` e `server.js`).
//! Mudar um deles quebra a compatibilidade com o servidor Node.js enquanto os dois coexistirem.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Limites herdados de `assets/catalog.js` (`LIMITS`) e `lib/validate.js`.
pub mod limits {
    /// Tamanho máximo do nome do jogador, em caracteres.
    pub const NAME: usize = 16;
    /// Bots por sala.
    pub const BOTS: u32 = 20;
    /// Mensagem de chat da sala.
    pub const CHAT: usize = 200;
    /// Mensagem do chat global.
    pub const GLOBAL_CHAT: usize = 140;
    /// Amigos por conta.
    pub const FRIENDS: usize = 100;
    /// Código de sala (sem o hífen exibido ao jogador).
    pub const ROOM_CODE: usize = 10;
    /// Token de sessão de sala (base64url de 32 bytes).
    pub const ROOM_TOKEN: usize = 43;
    /// Tamanho máximo de uma mensagem WebSocket aceita pelo servidor original.
    pub const WS_MAX_PAYLOAD: usize = 32 * 1024;
    /// Pessoas por sala no padrão (o painel do servidor sobe até 40).
    pub const ROOM_PEOPLE_DEFAULT: u32 = 16;
    pub const ROOM_PEOPLE_MAX: u32 = 40;
    /// Altura máxima aceita para o avatar (`move.y`).
    pub const MOVE_MAX_Y: f32 = 1.6;
}

/// Fogueira central, igual em todos os mundos (`server.js`: `FIRE_X`, `FIRE_Z`, `FIRE_REACH`).
pub mod fire {
    pub const X: f32 = 0.0;
    pub const Z: f32 = 1.2;
    pub const REACH: f32 = 6.5;
}

/// Os seis mundos (biomas brasileiros) do Skyke3D.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum World {
    #[default]
    Amazora,
    Cerron,
    Atlanthia,
    Caatara,
    Panthara,
    Pamphora,
}

impl World {
    pub const ALL: [World; 6] =
        [World::Amazora, World::Cerron, World::Atlanthia, World::Caatara, World::Panthara, World::Pamphora];

    /// Id usado no protocolo e nos arquivos (`amazora`, `cerron`...).
    pub const fn id(self) -> &'static str {
        match self {
            World::Amazora => "amazora",
            World::Cerron => "cerron",
            World::Atlanthia => "atlanthia",
            World::Caatara => "caatara",
            World::Panthara => "panthara",
            World::Pamphora => "pamphora",
        }
    }

    /// Nome mostrado ao jogador.
    pub const fn display_name(self) -> &'static str {
        match self {
            World::Amazora => "Amazora",
            World::Cerron => "Cerron",
            World::Atlanthia => "Atlanthia",
            World::Caatara => "Caatara",
            World::Panthara => "Panthara",
            World::Pamphora => "Pamphora",
        }
    }

    /// Bioma que inspira o mundo.
    pub const fn biome(self) -> &'static str {
        match self {
            World::Amazora => "Amazônia",
            World::Cerron => "Cerrado",
            World::Atlanthia => "Mata Atlântica",
            World::Caatara => "Caatinga",
            World::Panthara => "Pantanal",
            World::Pamphora => "Pampa",
        }
    }
}

impl fmt::Display for World {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("mundo desconhecido: {0}")]
pub struct UnknownWorld(pub String);

impl FromStr for World {
    type Err = UnknownWorld;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        World::ALL.into_iter().find(|w| w.id() == s).ok_or_else(|| UnknownWorld(s.to_owned()))
    }
}

/// Id de um ator na sala: `p<n>` para pessoas, `b<n>` para bots (como no servidor original).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ActorId {
    pub bot: bool,
    pub n: u32,
}

impl ActorId {
    /// Monta a partir do inteiro do pacote binário: bit 31 = bot, 31 bits baixos = número.
    pub const fn from_wire(u: u32) -> Self {
        Self { bot: u & 0x8000_0000 != 0, n: u & 0x7fff_ffff }
    }
    pub const fn to_wire(self) -> u32 {
        (self.n & 0x7fff_ffff) | if self.bot { 0x8000_0000 } else { 0 }
    }
}

impl fmt::Display for ActorId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", if self.bot { 'b' } else { 'p' }, self.n)
    }
}

impl FromStr for ActorId {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        let (bot, rest) = match s.as_bytes().first() {
            Some(b'p') => (false, &s[1..]),
            Some(b'b') => (true, &s[1..]),
            _ => return Err(()),
        };
        if rest.is_empty() || rest.len() > 10 || !rest.bytes().all(|c| c.is_ascii_digit()) {
            return Err(());
        }
        let n: u32 = rest.parse().map_err(|_| ())?;
        if n > 0x7fff_ffff {
            return Err(());
        }
        Ok(Self { bot, n })
    }
}

impl Serialize for ActorId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ActorId {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(|_| serde::de::Error::custom("id de ator inválido"))
    }
}

/// Alfabeto dos códigos de sala (sem I, O, 0 e 1, que se confundem), igual a `lib/validate.js`.
pub const ROOM_CODE_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Normaliza um código de sala como o servidor (`normCode`): maiúsculas, só letras e números,
/// corta em 10 e exige exatamente 10 caracteres do alfabeto.
pub fn normalize_room_code(input: &str) -> Option<String> {
    let c: String = input.to_uppercase().chars().filter(char::is_ascii_alphanumeric).take(limits::ROOM_CODE).collect();
    (c.len() == limits::ROOM_CODE && c.chars().all(|ch| ROOM_CODE_ALPHABET.contains(ch))).then_some(c)
}

/// Formata o código para exibição (`ABCDE-FGHIJ`).
pub fn format_room_code(code: &str) -> String {
    if code.len() == limits::ROOM_CODE && code.is_ascii() {
        format!("{}-{}", &code[..5], &code[5..])
    } else {
        code.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_roundtrip() {
        for w in World::ALL {
            assert_eq!(w.id().parse::<World>().unwrap(), w);
        }
        assert!("nyx".parse::<World>().is_err());
    }

    #[test]
    fn actor_id_wire_and_text() {
        let a = ActorId { bot: true, n: 7 };
        assert_eq!(ActorId::from_wire(a.to_wire()), a);
        assert_eq!(a.to_string(), "b7");
        assert_eq!("p12".parse::<ActorId>().unwrap(), ActorId { bot: false, n: 12 });
        assert!("x1".parse::<ActorId>().is_err());
        assert!("p".parse::<ActorId>().is_err());
        assert!("p-1".parse::<ActorId>().is_err());
    }

    #[test]
    fn room_codes() {
        assert_eq!(normalize_room_code("abcde-fghjk").as_deref(), Some("ABCDEFGHJK"));
        assert_eq!(normalize_room_code("abc"), None);
        // I e O não existem no alfabeto
        assert_eq!(normalize_room_code("ABCDEFGHIJ"), None);
        assert_eq!(format_room_code("ABCDEFGHJK"), "ABCDE-FGHJK");
    }
}
