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
    /// Endereço do servidor (para o link de convite).
    pub server: String,
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

/// Gesto que o jogador quer fazer (`stop` para).
#[derive(Message, Debug, Clone)]
pub struct SendGesture(pub String);

/// Qualquer mensagem para o servidor (emoji, chat global, denúncia...); sem conexão, é descartada.
#[derive(Message, Debug, Clone)]
pub struct SendMsg(pub sky_protocol::ClientMsg);

/// Id do ator na sala (nos avatares dos outros jogadores).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetId(pub sky_shared::ActorId);

/// Jogadores silenciados nesta sessão: o chat e os emojis deles não aparecem.
#[derive(Resource, Debug, Default)]
pub struct Muted(pub std::collections::HashSet<sky_shared::ActorId>);

/// Emoji sobre a cabeça (`showEmo`: some em 4 s).
#[derive(Component, Debug, Clone)]
pub struct Emote {
    pub ic: String,
    pub left: f32,
}

/// Chat global: linhas, pessoas online e se a assinatura já foi pedida.
#[derive(Resource, Debug, Default)]
pub struct GlobalChat {
    pub log: ChatLog,
    pub people: u64,
    pub on: bool,
}

/// Aba do chat em uso.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ChatTab {
    #[default]
    Room,
    Global,
}

/// Visual novo do jogador para enviar ao servidor (`{t:'cfg'}`).
#[derive(Message, Debug, Clone)]
pub struct SendCfg(pub std::collections::BTreeMap<String, u32>);

/// Pasta de dados do jogo (`$XDG_DATA_HOME/skyke3d`, `%APPDATA%\skyke3d` ou `~/.local/share/skyke3d`).
pub fn data_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("APPDATA").map(std::path::PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir)
        .join("skyke3d")
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
