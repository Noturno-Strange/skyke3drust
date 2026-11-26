//! Mensagens JSON (texto) do protocolo original.
//!
//! Toda mensagem é um objeto com o campo `t` (tipo). Aqui estão tipadas as mensagens do núcleo (entrar na sala,
//! identidade, movimento, chat, visual, gestos, itens, voz). As demais continuam passando como [`ServerMsg::Other`],
//! para que o cliente nativo funcione com o servidor Node.js atual sem perder mensagens que ainda não tratamos.
//!
//! Referências: `lib/validate.js` (o que o servidor aceita), `server.js` (o que ele envia), `assets/app-net.js`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sky_shared::ActorId;
use std::collections::BTreeMap;

/// Visual do avatar: índice de cada opção do editor (`skin`, `eye`, `hs`, `hat`...). Validado em `sky-avatars`.
pub type AvatarCfg = BTreeMap<String, u32>;

/// Mensagens que o cliente envia.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "lowercase")]
pub enum ClientMsg {
    // ---- identidade (lib/identity-ws.js) ----
    /// Registra as chaves públicas do aparelho. `pow` só quando o servidor pede prova de trabalho.
    Idreg {
        k: String,
        pk: String,
        rpk: String,
        #[serde(skip_serializing_if = "String::is_empty", default)]
        pow: String,
    },
    /// Pede um desafio para provar a posse da chave.
    Auth {
        id: String,
        action: String,
    },
    /// Assinatura do desafio.
    Proof {
        id: String,
        action: String,
        nonce: String,
        sig: String,
    },
    /// Carrega nome, visual e amigos guardados no servidor.
    Accountget {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        token: Option<String>,
    },

    // ---- salas ----
    Create {
        style: String,
        bots: u32,
        name: String,
        cfg: AvatarCfg,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        token: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        k: Option<String>,
    },
    Join {
        code: String,
        name: String,
        cfg: AvatarCfg,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        token: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        k: Option<String>,
    },
    Resume {
        code: String,
        token: String,
    },
    Leave,

    // ---- na sala ----
    Move {
        x: f32,
        z: f32,
        ry: f32,
        y: f32,
        s: u8,
    },
    Chat {
        text: String,
    },
    Cfg {
        cfg: AvatarCfg,
    },
    /// Gesto/dança (`g` vem de `GESTS` no catálogo) ou `stop`.
    Gest {
        g: String,
    },
    /// Item na mão (`item` vazio solta). `use` consome.
    Item {
        item: String,
        #[serde(rename = "use", skip_serializing_if = "std::ops::Not::not", default)]
        use_: bool,
    },
    /// Indicador de fala.
    Speak {
        v: bool,
    },
    /// Emoji sobre a cabeça (`i` indexa `EMO` no catálogo).
    Emo {
        i: u32,
    },
    /// Liga ou desliga o chat global.
    Gsub {
        on: bool,
    },
    /// Mensagem no chat global.
    Gchat {
        text: String,
    },
    /// Denúncia de um jogador da sala (`reason`: `assedio`, `odio`, `sexual`, `spam`, `golpe`, `trapaca`, `outro`).
    Report {
        id: String,
        reason: String,
        #[serde(skip_serializing_if = "String::is_empty", default)]
        note: String,
    },
}

/// Ator como o servidor descreve (`pub` em `server.js`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerPub {
    pub id: ActorId,
    pub name: String,
    #[serde(default)]
    pub cfg: AvatarCfg,
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub z: f32,
    #[serde(default)]
    pub ry: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub s: u8,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub item: Option<String>,
    #[serde(default)]
    pub talk: bool,
    #[serde(default)]
    pub vl: bool,
    /// Chave para silenciar (não revela o IP).
    #[serde(default)]
    pub mk: String,
    /// Apelido público de amigo.
    #[serde(default)]
    pub fid: String,
    /// Código do perfil público.
    #[serde(default)]
    pub pp: String,
    #[serde(default)]
    pub health: Value,
}

/// Resposta ao entrar (ou voltar) numa sala (`joinPayload` em `server.js`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Joined {
    #[serde(default)]
    pub ver: String,
    pub id: ActorId,
    pub code: String,
    pub style: String,
    #[serde(default)]
    pub host: Option<ActorId>,
    #[serde(default)]
    pub max: u32,
    #[serde(default)]
    pub players: Vec<PlayerPub>,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub rk: String,
    #[serde(rename = "self")]
    pub me: PlayerPub,
    #[serde(default)]
    pub reconnect: Option<u64>,
    #[serde(default)]
    pub vopus: bool,
    #[serde(default)]
    pub resume: Option<u8>,
}

/// Mensagens do servidor que o núcleo do cliente nativo já entende.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    Joined(Box<Joined>),
    PlayerJoined(Box<PlayerPub>),
    PlayerLeft {
        id: ActorId,
    },
    Host {
        id: ActorId,
    },
    Cap {
        max: u32,
    },
    Chat {
        id: Option<ActorId>,
        name: String,
        text: String,
    },
    Cfg {
        id: ActorId,
        cfg: AvatarCfg,
    },
    Gest {
        id: ActorId,
        g: String,
    },
    Item {
        id: ActorId,
        item: Option<String>,
        use_: bool,
    },
    Speak {
        id: ActorId,
        v: bool,
    },
    Sys {
        text: String,
    },
    Emo {
        id: ActorId,
        i: u32,
    },
    /// Chat global: `id` é um apelido opaco do autor (não o id da sala).
    Gchat {
        id: String,
        name: String,
        text: String,
    },
    /// Aviso do chat global.
    Gsys {
        text: String,
    },
    /// Pessoas online no servidor (chat global ligado).
    Gn {
        n: u64,
    },
    Err {
        msg: String,
        scope: String,
        code: String,
        raw: Value,
    },
    Challenge {
        id: String,
        action: String,
        nonce: String,
        exp: u64,
    },
    Session {
        id: String,
        token: String,
        exp: u64,
        generation: u64,
        epoch: u64,
        scope: String,
        raw: Value,
    },
    IdOk {
        op: String,
        raw: Value,
    },
    Account(Value),
    /// Qualquer outra mensagem (fazenda, culinária, mascote, mídia...): preservada sem perda.
    Other {
        t: String,
        raw: Value,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("JSON inválido: {0}")]
    Json(#[from] serde_json::Error),
    #[error("mensagem sem campo t")]
    NoType,
    #[error("campo inválido em {t}: {source}")]
    Field { t: String, source: serde_json::Error },
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned()
}
fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or_default()
}
fn id(v: &Value, k: &str) -> Option<ActorId> {
    v.get(k).and_then(Value::as_str).and_then(|x| x.parse().ok())
}

impl ServerMsg {
    /// Lê uma mensagem de texto do servidor.
    pub fn parse(text: &str) -> Result<Self, ParseError> {
        let v: Value = serde_json::from_str(text)?;
        Self::from_value(v)
    }

    pub fn from_value(v: Value) -> Result<Self, ParseError> {
        let t = v.get("t").and_then(Value::as_str).ok_or(ParseError::NoType)?.to_owned();
        let field = |e| ParseError::Field { t: t.clone(), source: e };
        let need_id = |k: &str| id(&v, k).ok_or_else(|| field(serde::de::Error::custom("id ausente")));
        Ok(match t.as_str() {
            "joined" => ServerMsg::Joined(Box::new(serde_json::from_value(v.clone()).map_err(field)?)),
            "pj" => {
                let p = v.get("p").cloned().unwrap_or(Value::Null);
                ServerMsg::PlayerJoined(Box::new(serde_json::from_value(p).map_err(field)?))
            }
            "pl" => ServerMsg::PlayerLeft { id: need_id("id")? },
            "host" => ServerMsg::Host { id: need_id("id")? },
            "cap" => ServerMsg::Cap { max: u(&v, "max") as u32 },
            "chat" => ServerMsg::Chat { id: id(&v, "id"), name: s(&v, "name"), text: s(&v, "text") },
            "cfg" => ServerMsg::Cfg {
                id: need_id("id")?,
                cfg: serde_json::from_value(v.get("cfg").cloned().unwrap_or_default()).map_err(field)?,
            },
            "gest" => ServerMsg::Gest { id: need_id("id")?, g: s(&v, "g") },
            "item" => ServerMsg::Item {
                id: need_id("id")?,
                item: v.get("item").and_then(Value::as_str).map(str::to_owned),
                use_: v.get("use").is_some_and(|x| x.as_bool().unwrap_or(x.as_u64() == Some(1))),
            },
            "speak" => ServerMsg::Speak { id: need_id("id")?, v: v.get("v").and_then(Value::as_bool).unwrap_or(false) },
            "sys" => ServerMsg::Sys { text: s(&v, "text") },
            "emo" => ServerMsg::Emo { id: need_id("id")?, i: u(&v, "i") as u32 },
            "gchat" => ServerMsg::Gchat {
                id: v.get("id").map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_owned)).unwrap_or_default(),
                name: s(&v, "name"),
                text: s(&v, "text"),
            },
            "gsys" => ServerMsg::Gsys { text: s(&v, "text") },
            "gn" => ServerMsg::Gn { n: u(&v, "n") },
            "err" => ServerMsg::Err { msg: s(&v, "msg"), scope: s(&v, "scope"), code: s(&v, "code"), raw: v },
            "challenge" => ServerMsg::Challenge {
                id: s(&v, "id"),
                action: s(&v, "action"),
                nonce: s(&v, "nonce"),
                exp: u(&v, "exp"),
            },
            "session" => ServerMsg::Session {
                id: s(&v, "id"),
                token: s(&v, "token"),
                exp: u(&v, "exp"),
                generation: u(&v, "gen"),
                epoch: u(&v, "epoch"),
                scope: s(&v, "scope"),
                raw: v,
            },
            "idok" => ServerMsg::IdOk { op: s(&v, "op"), raw: v },
            "account" => ServerMsg::Account(v),
            _ => ServerMsg::Other { t, raw: v },
        })
    }
}

impl ClientMsg {
    pub fn to_json(&self) -> String {
        // Serializar estes tipos não falha (só strings, números e mapas com chave string).
        serde_json::to_string(self).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_messages_match_original_shape() {
        let m = ClientMsg::Move { x: 1.0, z: 2.0, ry: 0.5, y: 0.0, s: 0 };
        assert_eq!(m.to_json(), r#"{"t":"move","x":1.0,"z":2.0,"ry":0.5,"y":0.0,"s":0}"#);
        let i = ClientMsg::Item { item: "cafe".into(), use_: false };
        assert_eq!(i.to_json(), r#"{"t":"item","item":"cafe"}"#);
        let r = ClientMsg::Idreg { k: "k".into(), pk: "p".into(), rpk: "r".into(), pow: String::new() };
        assert_eq!(r.to_json(), r#"{"t":"idreg","k":"k","pk":"p","rpk":"r"}"#);
        assert_eq!(ClientMsg::Leave.to_json(), r#"{"t":"leave"}"#);
        assert_eq!(ClientMsg::Emo { i: 3 }.to_json(), r#"{"t":"emo","i":3}"#);
        assert_eq!(ClientMsg::Gsub { on: true }.to_json(), r#"{"t":"gsub","on":true}"#);
        let r = ClientMsg::Report { id: "p2".into(), reason: "spam".into(), note: String::new() };
        assert_eq!(r.to_json(), r#"{"t":"report","id":"p2","reason":"spam"}"#);
    }

    #[test]
    fn parse_joined_and_unknown() {
        let txt = r#"{"t":"joined","ver":"3.30.1","id":"p1","code":"ABCDEFGHJK","style":"cerron","host":"p1","max":16,
            "players":[{"id":"b1","name":"Bia","cfg":{"skin":2},"x":1,"z":2,"ry":0,"y":0,"s":0,"bot":true,"item":null,"talk":false,"vl":false,"mk":"","fid":"","health":{},"pp":""}],
            "token":"t","rk":"r","self":{"id":"p1","name":"Eu","cfg":{}},"reconnect":30000}"#;
        let ServerMsg::Joined(j) = ServerMsg::parse(txt).unwrap() else { panic!() };
        assert_eq!(j.style, "cerron");
        assert_eq!(j.players[0].id.to_string(), "b1");
        assert_eq!(j.me.name, "Eu");

        let o = ServerMsg::parse(r#"{"t":"farm","x":1}"#).unwrap();
        assert!(matches!(o, ServerMsg::Other { ref t, .. } if t == "farm"));
        assert!(ServerMsg::parse(r#"{"x":1}"#).is_err());
        assert!(ServerMsg::parse(r#"{"t":"pl"}"#).is_err());
        assert_eq!(
            ServerMsg::parse(r#"{"t":"emo","id":"p2","i":4}"#).unwrap(),
            ServerMsg::Emo { id: "p2".parse().unwrap(), i: 4 }
        );
        let g = ServerMsg::parse(r#"{"t":"gchat","id":"h1","name":"Ana","text":"oi","at":1}"#).unwrap();
        assert!(matches!(g, ServerMsg::Gchat { ref name, ref text, .. } if name == "Ana" && text == "oi"));
    }
}
