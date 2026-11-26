//! Amigos (`assets/app-social.js`, servidor em `lib/social.js`). A lista de amigos fica só neste computador
//! (`skyke3d/online/amigos.json`, como `skyke_friends` no navegador); o servidor só liga ids e apelidos enquanto a pessoa
//! está conectada. Na sala, cada jogador aparece por um apelido da conexão (`fid` do ator); `fpres` diz quais apelidos
//! são de cada amigo online.

use crate::state::{ChatLog, ConnectRequest, Screen, SendRaw, ServerOther, Session};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug)]
pub struct FriendsPlugin;

impl Plugin for FriendsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Friends::load())
            .add_systems(OnExit(Screen::Playing), leave)
            .add_systems(Update, (enter, receive).chain().run_if(in_state(Screen::Playing)));
    }
}

/// Máximo de amigos (`FMAX`) e de pedidos guardados.
const FMAX: usize = 100;
const MAX_REQ: usize = 20;

/// Id de amigo ou apelido: 12 letras `[a-z2-7]`.
pub fn ok_id(x: &str) -> bool {
    x.len() == 12 && x.bytes().all(|c| c.is_ascii_lowercase() || (b'2'..=b'7').contains(&c))
}

fn cut(s: &str) -> String {
    s.chars().take(sky_shared::limits::NAME).collect()
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Friend {
    pub id: String,
    /// Último nome visto.
    pub n: String,
}

/// Amigos, pedidos e presença.
#[derive(Resource, Debug, Default)]
pub struct Friends {
    /// Meu id de amigo e meu apelido nesta conexão.
    pub me: String,
    pub h: String,
    pub fr: Vec<Friend>,
    pub on: HashSet<String>,
    /// Pedidos recebidos: id → nome.
    pub req: BTreeMap<String, String>,
    /// Apelidos a quem já pedi amizade.
    pub sent: HashSet<String>,
    /// Apelido atual → id de amigo.
    pub hm: HashMap<String, String>,
    /// Convite recebido: nome e código da sala.
    pub invite: Option<(String, String)>,
    pub version: u64,
    /// A lista já foi enviada nesta conexão (`socEnter`).
    entered: bool,
}

impl Friends {
    fn path() -> std::path::PathBuf {
        crate::state::data_dir().join("online/amigos.json")
    }

    fn load() -> Self {
        let fr: Vec<Friend> = sky_persistence::load_json(&Self::path(), 64 * 1024).ok().flatten().unwrap_or_default();
        Self { fr: fr.into_iter().filter(|f| ok_id(&f.id)).take(FMAX).collect(), ..default() }
    }

    fn save(&mut self, send: &mut MessageWriter<SendRaw>) {
        if let Err(e) = sky_persistence::save_json(&Self::path(), &self.fr, true) {
            warn!("amigos: {e}");
        }
        self.sync(send);
        self.version += 1;
    }

    /// Lista para o servidor (`friends`): ele avisa a presença deles e a minha para eles.
    fn sync(&self, send: &mut MessageWriter<SendRaw>) {
        if self.entered {
            let ids: Vec<&str> = self.fr.iter().map(|f| f.id.as_str()).collect();
            let names: Vec<&str> = self.fr.iter().map(|f| f.n.as_str()).collect();
            send.write(SendRaw(json!({"t": "friends", "ids": ids, "names": names})));
        }
    }

    pub fn is_friend(&self, id: &str) -> bool {
        self.fr.iter().any(|f| f.id == id)
    }

    /// Id de amigo de quem está na sala com este apelido (vazio se não é amigo online).
    pub fn friend_of_handle(&self, h: &str) -> Option<&str> {
        self.hm.get(h).map(String::as_str)
    }

    fn add(&mut self, id: &str, n: &str, send: &mut MessageWriter<SendRaw>) {
        if !ok_id(id) || id == self.me {
            return;
        }
        if let Some(f) = self.fr.iter_mut().find(|f| f.id == id) {
            if !n.is_empty() {
                f.n = cut(n);
            }
        } else if self.fr.len() < FMAX {
            self.fr.push(Friend { id: id.to_owned(), n: if n.is_empty() { "Amigo".into() } else { cut(n) } });
        }
        self.req.remove(id);
        self.save(send);
    }

    /// Pede amizade a quem está na sala (pelo apelido).
    pub fn request(&mut self, h: &str, name: &str, send: &mut MessageWriter<SendRaw>, log: &mut ChatLog) {
        if !ok_id(h) || h == self.h || self.sent.contains(h) || self.friend_of_handle(h).is_some() {
            return;
        }
        self.sent.insert(h.to_owned());
        send.write(SendRaw(json!({"t": "freq", "to": h})));
        log.push("", format!("👋 Pedido de amizade enviado para {}.", cut(name)), true);
        self.version += 1;
    }

    pub fn accept(&mut self, id: &str, send: &mut MessageWriter<SendRaw>, log: &mut ChatLog) {
        let Some(n) = self.req.get(id).cloned() else { return };
        send.write(SendRaw(json!({"t": "facc", "to": id})));
        self.add(id, &n, send);
        log.push("", format!("🤝 Agora vocês são amigos: {n}."), true);
    }

    pub fn decline(&mut self, id: &str) {
        self.req.remove(id);
        self.version += 1;
    }

    pub fn remove(&mut self, id: &str, send: &mut MessageWriter<SendRaw>) {
        self.fr.retain(|f| f.id != id);
        self.on.remove(id);
        self.save(send);
    }

    pub fn invite(&self, id: &str, send: &mut MessageWriter<SendRaw>) {
        if self.on.contains(id) {
            send.write(SendRaw(json!({"t": "finv", "to": id})));
        }
    }
}

/// Saindo da sala: presença e pedidos valem só na conexão.
fn leave(mut f: ResMut<Friends>) {
    f.entered = false;
    f.me.clear();
    f.h.clear();
    f.on.clear();
    f.sent.clear();
    f.hm.clear();
    f.invite = None;
    f.version += 1;
}

/// Ao entrar na sala (`socEnter`): manda a lista; o servidor responde com `fme` e passa o meu apelido para a sala.
fn enter(s: Res<Session>, mut f: ResMut<Friends>, mut send: MessageWriter<SendRaw>) {
    if s.online && !f.entered {
        f.entered = true;
        f.on.clear();
        f.sync(&mut send);
    }
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned()
}

fn receive(
    mut msgs: MessageReader<ServerOther>,
    mut f: ResMut<Friends>,
    mut send: MessageWriter<SendRaw>,
    mut log: ResMut<ChatLog>,
    mut fids: Query<(&crate::state::NetId, &mut crate::state::Fid)>,
) {
    for m in msgs.read() {
        let v = &m.raw;
        match m.t.as_str() {
            "fme" => {
                let (id, h) = (s(v, "id"), s(v, "h"));
                if ok_id(&id) {
                    f.me = id;
                }
                if ok_id(&h) {
                    f.h = h;
                }
                f.version += 1;
            }
            "pfid" => {
                let fid = s(v, "fid");
                if let Some(id) = v.get("id").and_then(Value::as_str).and_then(|x| x.parse().ok())
                    && ok_id(&fid)
                {
                    for (nid, mut fd) in &mut fids {
                        if nid.0 == id {
                            fd.0.clone_from(&fid);
                        }
                    }
                    f.version += 1;
                }
            }
            "fpres" => {
                let id = s(v, "id");
                if !ok_id(&id) || !f.is_friend(&id) {
                    continue;
                }
                f.hm.retain(|_, x| *x != id);
                let on = v.get("on").and_then(Value::as_u64).unwrap_or(0) == 1;
                if on {
                    for h in v.get("hs").and_then(Value::as_array).into_iter().flatten().take(8) {
                        if let Some(h) = h.as_str().filter(|h| ok_id(h)) {
                            f.hm.insert(h.to_owned(), id.clone());
                        }
                    }
                    f.on.insert(id.clone());
                    let n = s(v, "name");
                    if !n.is_empty()
                        && let Some(fr) = f.fr.iter_mut().find(|x| x.id == id)
                    {
                        fr.n = cut(&n);
                    }
                } else {
                    f.on.remove(&id);
                }
                f.version += 1;
            }
            "freq" => {
                let id = s(v, "id");
                if !ok_id(&id) || f.is_friend(&id) || f.req.contains_key(&id) || f.req.len() >= MAX_REQ {
                    continue;
                }
                let n = cut(&s(v, "name"));
                log.push("", format!("👋 {n} quer ser seu amigo. Abra 👥 para aceitar."), true);
                f.req.insert(id, n);
                f.version += 1;
            }
            "facc" => {
                let id = s(v, "id");
                if ok_id(&id) && !f.is_friend(&id) {
                    let n = s(v, "name");
                    f.add(&id, &n, &mut send);
                    f.sent.clear();
                    log.push("", format!("🤝 {} aceitou seu pedido de amizade.", cut(&n)), true);
                }
            }
            "finvr" => {
                let id = s(v, "id");
                let Some(fr) = f.fr.iter().find(|x| x.id == id) else { continue };
                let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let r = s(v, "reason");
                log.push(
                    "",
                    if ok {
                        format!("📨 Convite enviado para {}.", fr.n)
                    } else if r == "wait" {
                        "Aguarde 30 segundos antes de convidar este amigo novamente.".into()
                    } else if r == "same" {
                        "Vocês já estão na mesma sala.".into()
                    } else {
                        "Não foi possível entregar o convite. Confira se seu amigo está online.".into()
                    },
                    true,
                );
            }
            "finv" => {
                let id = s(v, "id");
                let code = s(v, "code");
                if !ok_id(&id) || !f.is_friend(&id) || sky_shared::normalize_room_code(&code).is_none() {
                    continue;
                }
                let n = s(v, "name");
                let name = f.fr.iter().find(|x| x.id == id).map_or_else(|| cut(&n), |x| x.n.clone());
                log.push("", format!("📨 {name} chamou você para a sala dele(a). Abra 👥 para entrar."), true);
                f.invite = Some((name, code));
                f.version += 1;
            }
            _ => {}
        }
    }
}

/// Aceita o convite: sai desta sala e entra na do amigo.
pub fn join_invite(
    f: &mut Friends,
    s: &Session,
    req: &mut MessageWriter<ConnectRequest>,
    next: &mut NextState<Screen>,
) {
    let Some((_, code)) = f.invite.take() else { return };
    req.write(ConnectRequest {
        server: s.server.clone(),
        name: s.my_name.clone(),
        join: Some(code),
        world: sky_shared::World::Amazora,
        bots: 0,
    });
    next.set(Screen::Connecting);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_names() {
        assert!(ok_id("abcdefgh2345"));
        assert!(!ok_id("ABCDEFGH2345") && !ok_id("abc") && !ok_id("abcdefgh2341"));
        assert_eq!(cut(&"a".repeat(100)).chars().count(), sky_shared::limits::NAME);
    }
}
