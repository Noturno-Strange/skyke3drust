//! Telas do jogo e dados da sessão compartilhados entre rede e interface.

use bevy::prelude::*;
use sky_shared::World;

/// Tela atual.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Screen {
    /// Tela inicial (nome, mundo, criar ou entrar).
    #[default]
    Menu,
    /// Ticket, identidade e entrada na sala em andamento.
    Connecting,
    /// Na sala (online) ou explorando sem conexão.
    Playing,
}

/// Pedido de conexão feito pela tela inicial (ou pela linha de comando).
#[derive(Message, Debug, Clone)]
pub struct ConnectRequest {
    pub server: String,
    pub name: String,
    pub join: Option<String>,
    pub world: World,
    pub bots: u32,
}

/// Sessão na sala.
#[derive(Resource, Debug, Clone, Default)]
pub struct Session {
    pub online: bool,
    pub code: String,
    pub people: u32,
    pub max: u32,
    pub my_name: String,
}

/// Uma linha do chat.
#[derive(Debug, Clone, PartialEq)]
pub struct ChatLine {
    pub name: String,
    pub text: String,
    /// Aviso do jogo (entrou, saiu, erro), mostrado sem nome.
    pub system: bool,
}

/// Histórico do chat da sala (o servidor não guarda chat; aqui só as últimas linhas desta sessão).
#[derive(Resource, Debug, Default)]
pub struct ChatLog {
    pub lines: Vec<ChatLine>,
    pub version: u64,
}

impl ChatLog {
    pub const MAX: usize = 200;

    pub fn push(&mut self, name: impl Into<String>, text: impl Into<String>, system: bool) {
        self.lines.push(ChatLine { name: name.into(), text: text.into(), system });
        if self.lines.len() > Self::MAX {
            let extra = self.lines.len() - Self::MAX;
            self.lines.drain(..extra);
        }
        self.version += 1;
    }
}

/// Mensagem de chat que o jogador quer enviar.
#[derive(Message, Debug, Clone)]
pub struct SendChat(pub String);

/// Último erro de conexão (mostrado na tela inicial).
#[derive(Resource, Debug, Default)]
pub struct ConnectError(pub Option<String>);

/// Nome mostrado sobre o avatar.
#[derive(Component, Debug, Clone)]
pub struct ActorName(pub String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_log_is_bounded() {
        let mut l = ChatLog::default();
        for i in 0..250 {
            l.push("a", i.to_string(), false);
        }
        assert_eq!(l.lines.len(), ChatLog::MAX);
        assert_eq!(l.lines[0].text, "50");
        assert_eq!(l.version, 250);
    }
}
