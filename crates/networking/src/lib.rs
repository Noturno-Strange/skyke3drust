//! Cliente de rede compatível com o servidor Node.js do Skyke3D.
//!
//! Sequência (a mesma do navegador, ver `assets/app-net.js` e `assets/identity.js`):
//! 1. `GET /ticket` e prova de trabalho ([`sky_security::pow::solve_ticket`]); 404 = servidor sem tickets.
//! 2. WebSocket em `ws(s)://servidor/?ticket=…`, com `Origin` igual ao endereço do servidor.
//! 3. Identidade: registra na primeira vez (`idreg`, com prova de trabalho se pedida) e prova a chave (`auth` → `proof`).
//! 4. `accountget` com o token da sessão e, por fim, `create` ou `join`, esperando `joined`.
//!
//! Depois disso, uma tarefa Tokio lê e escreve no socket; o jogo conversa com ela por canais ([`NetHandle`]),
//! sem bloquear o quadro.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod identity_store;

pub use identity_store::StoredIdentity;

use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use sky_identity::KeyPair;
use sky_protocol::{ActorState, AvatarCfg, ClientMsg, Joined, ServerMsg, binary};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use url::Url;

/// Tempo máximo de cada etapa do aperto de mão.
const STEP_TIMEOUT: Duration = Duration::from_secs(12);
/// Mensagem maior que isso é descartada (o servidor original aceita até 32 KiB do cliente).
const MAX_IN: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("endereço do servidor inválido: {0}")]
    BadUrl(String),
    #[error("servidor indisponível: {0}")]
    Http(String),
    #[error("resposta inválida do servidor")]
    BadResponse,
    #[error("prova de trabalho: {0}")]
    Pow(#[from] sky_security::pow::PowError),
    #[error("conexão: {0}")]
    Ws(String),
    #[error("tempo esgotado esperando {0}")]
    Timeout(&'static str),
    #[error("conexão encerrada pelo servidor")]
    Closed,
    #[error("identidade: {0}")]
    Identity(String),
    #[error("{0}")]
    Server(String),
}

/// Entrar numa sala nova ou numa existente.
#[derive(Debug, Clone)]
pub enum Enter {
    Create { world: sky_shared::World, bots: u32 },
    Join { code: String },
}

#[derive(Debug, Clone)]
pub struct ConnectConfig {
    /// Endereço HTTP(S) do servidor, ex.: `http://localhost:3000` ou `https://skyke3d.com.br`.
    pub server: String,
    /// Arquivo da identidade deste aparelho (criado na primeira vez).
    pub identity_path: PathBuf,
    pub name: String,
    pub cfg: AvatarCfg,
    pub enter: Enter,
}

/// Eventos que chegam do servidor depois de entrar na sala.
#[derive(Debug, Clone)]
pub enum NetEvent {
    Server(ServerMsg),
    State(Vec<ActorState>),
    /// Quadro de voz bruto (o crate `sky-voice` decodifica).
    Voice(Vec<u8>),
    Closed(String),
}

/// Ponte entre o jogo e a tarefa de rede.
#[derive(Debug)]
pub struct NetHandle {
    pub joined: Box<Joined>,
    out: mpsc::UnboundedSender<Message>,
    pub events: mpsc::UnboundedReceiver<NetEvent>,
}

impl NetHandle {
    pub fn send(&self, m: &ClientMsg) {
        let _ = self.out.send(Message::text(m.to_json()));
    }
    pub fn send_binary(&self, b: Vec<u8>) {
        let _ = self.out.send(Message::binary(b));
    }
    /// Recebe sem bloquear (para o laço do jogo).
    pub fn try_recv(&mut self) -> Option<NetEvent> {
        self.events.try_recv().ok()
    }
}

#[derive(Deserialize)]
struct TicketJson {
    v: u32,
    c: String,
    e: u64,
    d: u32,
    k: String,
}

fn endpoints(server: &str) -> Result<(Url, Url, String), NetError> {
    let base = Url::parse(server).map_err(|e| NetError::BadUrl(e.to_string()))?;
    let ws_scheme = match base.scheme() {
        "http" => "ws",
        "https" => "wss",
        s => return Err(NetError::BadUrl(format!("esquema {s} (use http ou https)"))),
    };
    let host = base.host_str().ok_or_else(|| NetError::BadUrl("sem host".into()))?;
    let origin = match base.port() {
        Some(p) => format!("{}://{host}:{p}", base.scheme()),
        None => format!("{}://{host}", base.scheme()),
    };
    let mut ticket = base.clone();
    ticket.set_path("/ticket");
    ticket.set_query(None);
    let mut ws = base;
    ws.set_scheme(ws_scheme).map_err(|()| NetError::BadUrl("esquema".into()))?;
    ws.set_path("/");
    ws.set_query(None);
    Ok((ticket, ws, origin))
}

async fn get_ticket(url: &Url) -> Result<Option<String>, NetError> {
    let client = reqwest::Client::builder()
        .timeout(STEP_TIMEOUT)
        .user_agent(concat!("skyke3d-nativo/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(|e| NetError::Http(e.to_string()))?;
    let r = client.get(url.clone()).send().await.map_err(|e| NetError::Http(e.to_string()))?;
    if r.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !r.status().is_success() {
        return Err(NetError::Http(if r.status().as_u16() == 429 {
            "muitas tentativas; aguarde um instante".into()
        } else {
            r.status().to_string()
        }));
    }
    let t: TicketJson = r.json().await.map_err(|_| NetError::BadResponse)?;
    if t.v != 1 {
        return Err(NetError::BadResponse);
    }
    let param = tokio::task::spawn_blocking(move || {
        sky_security::pow::solve_ticket(&sky_security::pow::Ticket { c: t.c, e: t.e, d: t.d, k: t.k })
    })
    .await
    .map_err(|_| NetError::BadResponse)??;
    Ok(Some(param))
}

type Ws = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Conn {
    ws: Ws,
}

impl Conn {
    async fn send(&mut self, m: &ClientMsg) -> Result<(), NetError> {
        self.ws.send(Message::text(m.to_json())).await.map_err(|e| NetError::Ws(e.to_string()))
    }

    /// Lê mensagens de texto até `want` aceitar uma (ou o tempo acabar). Binárias são ignoradas nesta fase.
    async fn wait<T>(
        &mut self,
        what: &'static str,
        mut want: impl FnMut(ServerMsg) -> Option<T>,
    ) -> Result<T, NetError> {
        let fut = async {
            while let Some(m) = self.ws.next().await {
                let m = m.map_err(|e| NetError::Ws(e.to_string()))?;
                match m {
                    Message::Text(t) if t.len() <= MAX_IN => {
                        if let Ok(sm) = ServerMsg::parse(&t)
                            && let Some(v) = want(sm)
                        {
                            return Ok(v);
                        }
                    }
                    Message::Close(_) => return Err(NetError::Closed),
                    _ => {}
                }
            }
            Err(NetError::Closed)
        };
        tokio::time::timeout(STEP_TIMEOUT, fut).await.map_err(|_| NetError::Timeout(what))?
    }
}

fn server_err(scope_prefix: &[&str], m: &ServerMsg) -> Option<NetError> {
    if let ServerMsg::Err { msg, scope, code, .. } = m
        && (scope_prefix.is_empty() || scope_prefix.contains(&scope.as_str()))
    {
        let text = if msg.is_empty() { code.clone() } else { msg.clone() };
        return Some(NetError::Server(text));
    }
    None
}

/// Registra a identidade deste aparelho na primeira vez.
async fn enroll(c: &mut Conn) -> Result<StoredIdentity, NetError> {
    let k = sky_identity::new_device_key();
    let primary = KeyPair::generate();
    let recovery = KeyPair::generate();
    let pk = primary.public_b64().map_err(|e| NetError::Identity(e.to_string()))?;
    let rpk = recovery.public_b64().map_err(|e| NetError::Identity(e.to_string()))?;
    let mut pow = String::new();
    for _ in 0..4 {
        c.send(&ClientMsg::Idreg { k: k.clone(), pk: pk.clone(), rpk: rpk.clone(), pow: pow.clone() }).await?;
        let r = c
            .wait("registro", |m| match &m {
                ServerMsg::IdOk { op, .. } if op == "register" => Some(Ok(m)),
                ServerMsg::Err { scope, .. } if scope == "idreg" => Some(Err(m)),
                _ => None,
            })
            .await?;
        match r {
            Ok(ServerMsg::IdOk { raw, .. }) => {
                let id = raw.get("id").and_then(|v| v.as_str()).unwrap_or_default().to_owned();
                if !sky_identity::valid_id(&id) {
                    return Err(NetError::BadResponse);
                }
                let primary_b64 =
                    primary.to_pkcs8().map(|d| base64_url(&d)).map_err(|e| NetError::Identity(e.to_string()))?;
                // A chave de recuperação NÃO é gravada. Enquanto o código de recuperação não é exibido pela interface
                // nativa, ela se perde ao fechar o jogo; ver docs/estado-da-migracao.md.
                drop(recovery);
                return Ok(StoredIdentity {
                    v: 1,
                    k,
                    id,
                    pk,
                    rpk,
                    generation: raw.get("gen").and_then(|v| v.as_u64()).unwrap_or(0),
                    epoch: raw.get("epoch").and_then(|v| v.as_u64()).unwrap_or(0),
                    primary: primary_b64,
                });
            }
            Err(ServerMsg::Err { code, raw, msg, .. }) if code == "pow" => {
                let bits = raw.get("bits").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                if bits == 0 {
                    return Err(NetError::Server(msg));
                }
                let (k2, pk2, rpk2) = (k.clone(), pk.clone(), rpk.clone());
                pow = tokio::task::spawn_blocking(move || sky_security::pow::solve_idreg(&k2, &pk2, &rpk2, bits))
                    .await
                    .map_err(|_| NetError::BadResponse)??;
            }
            Err(m) => return Err(server_err(&[], &m).unwrap_or(NetError::BadResponse)),
            Ok(_) => return Err(NetError::BadResponse),
        }
    }
    Err(NetError::Server("o servidor continua pedindo prova de trabalho".into()))
}

fn base64_url(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Prova a chave e devolve o token de sessão.
async fn login(c: &mut Conn, id: &mut StoredIdentity) -> Result<String, NetError> {
    let key = id.primary_key().ok_or_else(|| NetError::Identity("chave local ilegível".into()))?;
    c.send(&ClientMsg::Auth { id: id.id.clone(), action: "join".into() }).await?;
    let nonce = c
        .wait("desafio", |m| match m {
            ServerMsg::Challenge { nonce, .. } => Some(Ok(nonce)),
            ref e @ ServerMsg::Err { ref scope, .. } if scope == "auth" => Some(Err(server_err(&[], e))),
            _ => None,
        })
        .await?
        .map_err(|e| e.unwrap_or(NetError::BadResponse))?;
    let sig = key.sign_b64(&sky_identity::payload::auth(&id.id, &nonce, "join"));
    c.send(&ClientMsg::Proof { id: id.id.clone(), action: "join".into(), nonce, sig }).await?;
    let (token, generation, epoch) = c
        .wait("sessão", |m| match m {
            ServerMsg::Session { token, generation, epoch, .. } => Some(Ok((token, generation, epoch))),
            ref e @ ServerMsg::Err { ref scope, .. } if scope == "proof" => Some(Err(server_err(&[], e))),
            _ => None,
        })
        .await?
        .map_err(|e| e.unwrap_or(NetError::BadResponse))?;
    id.generation = generation;
    id.epoch = epoch;
    Ok(token)
}

/// Conecta, autentica e entra na sala.
pub async fn connect(cfg: ConnectConfig) -> Result<NetHandle, NetError> {
    let (ticket_url, mut ws_url, origin) = endpoints(&cfg.server)?;
    if let Some(t) = get_ticket(&ticket_url).await? {
        ws_url.query_pairs_mut().append_pair("ticket", &t);
    }
    let mut req = ws_url.as_str().into_client_request().map_err(|e| NetError::Ws(e.to_string()))?;
    req.headers_mut().insert("Origin", HeaderValue::from_str(&origin).map_err(|e| NetError::BadUrl(e.to_string()))?);
    let (ws, _) = tokio::time::timeout(STEP_TIMEOUT, tokio_tungstenite::connect_async(req))
        .await
        .map_err(|_| NetError::Timeout("conexão"))?
        .map_err(|e| NetError::Ws(e.to_string()))?;
    let mut c = Conn { ws };

    let mut ident = match StoredIdentity::load(&cfg.identity_path) {
        Some(i) => i,
        None => {
            let i = enroll(&mut c).await?;
            i.save(&cfg.identity_path).map_err(|e| NetError::Identity(e.to_string()))?;
            tracing::info!("identidade nova registrada: {}", i.id);
            i
        }
    };
    let token = login(&mut c, &mut ident).await?;
    let _ = ident.save(&cfg.identity_path);

    c.send(&ClientMsg::Accountget { token: Some(token.clone()) }).await?;
    let _account = c.wait("conta", |m| matches!(m, ServerMsg::Account(_)).then_some(m)).await?;

    let name = sky_security::clean_chat(&cfg.name, sky_shared::limits::NAME);
    let msg = match &cfg.enter {
        Enter::Create { world, bots } => ClientMsg::Create {
            style: world.id().into(),
            bots: (*bots).min(sky_shared::limits::BOTS),
            name,
            cfg: cfg.cfg.clone(),
            token: Some(token),
            k: Some(ident.k.clone()),
        },
        Enter::Join { code } => ClientMsg::Join {
            code: sky_shared::normalize_room_code(code)
                .ok_or_else(|| NetError::Server("código de sala inválido".into()))?,
            name,
            cfg: cfg.cfg.clone(),
            token: Some(token),
            k: Some(ident.k.clone()),
        },
    };
    c.send(&msg).await?;
    let joined = c
        .wait("entrada na sala", |m| match m {
            ServerMsg::Joined(j) => Some(Ok(j)),
            ref e @ ServerMsg::Err { .. } => Some(Err(server_err(&[], e))),
            _ => None,
        })
        .await?
        .map_err(|e| e.unwrap_or(NetError::BadResponse))?;

    let (out_tx, out_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    tokio::spawn(pump(c.ws, out_rx, ev_tx));
    Ok(NetHandle { joined, out: out_tx, events: ev_rx })
}

async fn pump(ws: Ws, mut out: mpsc::UnboundedReceiver<Message>, ev: mpsc::UnboundedSender<NetEvent>) {
    let (mut sink, mut stream) = ws.split();
    let reason = loop {
        tokio::select! {
            m = out.recv() => match m {
                Some(m) => if let Err(e) = sink.send(m).await { break e.to_string() },
                None => { let _ = sink.send(Message::Close(None)).await; break "saída".into() }
            },
            m = stream.next() => match m {
                Some(Ok(Message::Text(t))) if t.len() <= MAX_IN => {
                    if let Ok(sm) = ServerMsg::parse(&t) && ev.send(NetEvent::Server(sm)).is_err() { break "jogo fechado".into() }
                }
                Some(Ok(Message::Binary(b))) if b.len() <= MAX_IN => {
                    let e = match binary::kind(&b) {
                        Some(binary::KIND_STATE) => binary::decode_state(&b).map(NetEvent::State),
                        Some(binary::KIND_VOICE) => Some(NetEvent::Voice(b.to_vec())),
                        _ => None,
                    };
                    if let Some(e) = e && ev.send(e).is_err() { break "jogo fechado".into() }
                }
                Some(Ok(Message::Close(f))) => break f.map(|f| f.reason.to_string()).unwrap_or_default(),
                Some(Ok(_)) => {}
                Some(Err(e)) => break e.to_string(),
                None => break String::new(),
            }
        }
    };
    let _ = ev.send(NetEvent::Closed(reason));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_derivation() {
        let (t, w, o) = endpoints("https://skyke3d.com.br").unwrap();
        assert_eq!(t.as_str(), "https://skyke3d.com.br/ticket");
        assert_eq!(w.as_str(), "wss://skyke3d.com.br/");
        assert_eq!(o, "https://skyke3d.com.br");
        let (_, w, o) = endpoints("http://localhost:3000/qualquer?x=1").unwrap();
        assert_eq!(w.as_str(), "ws://localhost:3000/");
        assert_eq!(o, "http://localhost:3000");
        assert!(endpoints("ftp://x").is_err());
    }

    #[test]
    fn b64url() {
        assert_eq!(base64_url(b"\xfb\xff"), "-_8");
        assert_eq!(base64_url(b"abc"), "YWJj");
    }
}
