//! Rede no cliente: conecta ao servidor (Node.js atual) numa thread Tokio e traz os eventos para o laço do Bevy.

use crate::gestures::{Gesture, GestureCatalog};
use crate::look::{AvatarKit, AvatarLook};
use crate::player::{Me, Remote, avatar_scene};
use crate::state::{ActorName, ChatLog, ConnectError, ConnectRequest, Screen, SendCfg, SendChat, SendGesture, Session};
use bevy::prelude::*;
use sky_networking::{ConnectConfig, Enter, NetEvent, NetHandle};
use sky_protocol::{ClientMsg, PlayerPub, ServerMsg};
use sky_shared::{ActorId, World};
use sky_world::LoadWorld;
use std::collections::HashMap;
use std::sync::{Mutex, mpsc};

#[derive(Debug)]
pub struct NetPlugin;

impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (start, finish_connect, receive, send_moves, send_chat, send_gesture, send_cfg))
            .add_systems(OnExit(Screen::Playing), leave);
    }
}

#[derive(Resource)]
struct Runtime(tokio::runtime::Runtime);

#[derive(Resource)]
struct Pending(Mutex<mpsc::Receiver<Result<NetHandle, String>>>);

#[derive(Resource)]
struct Net {
    h: Mutex<NetHandle>,
    actors: HashMap<ActorId, Entity>,
    me: ActorId,
    last_sent: Option<(String, f64)>,
    send_t: f64,
}

fn identity_path() -> std::path::PathBuf {
    crate::state::data_dir().join("online/identidade.json")
}

fn start(
    mut commands: Commands,
    mut reqs: MessageReader<ConnectRequest>,
    rt: Option<Res<Runtime>>,
    mut s: ResMut<Session>,
    kit: Option<Res<AvatarKit>>,
    my: Option<Res<crate::editor::MyLook>>,
) {
    let Some(r) = reqs.read().last().cloned() else { return };
    let runtime = match rt {
        Some(rt) => rt.0.handle().clone(),
        None => match tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build() {
            Ok(rt) => {
                let h = rt.handle().clone();
                commands.insert_resource(Runtime(rt));
                h
            }
            Err(e) => {
                error!("rede: {e}");
                return;
            }
        },
    };
    s.my_name.clone_from(&r.name);
    let cfg = ConnectConfig {
        server: r.server,
        identity_path: identity_path(),
        name: r.name,
        // o visual deste avatar (o mesmo que aparece aqui), para os outros verem igual
        cfg: my.map(|m| m.cfg.0.clone()).or_else(|| kit.map(|k| k.reference_map())).unwrap_or_default(),
        enter: match r.join {
            Some(code) => Enter::Join { code },
            None => Enter::Create { world: r.world, bots: r.bots },
        },
    };
    let (tx, rx) = mpsc::channel();
    runtime.spawn(async move {
        let r = sky_networking::connect(cfg).await.map_err(|e| e.to_string());
        let _ = tx.send(r);
    });
    commands.insert_resource(Pending(Mutex::new(rx)));
}

fn spawn_actor(commands: &mut Commands, assets: &AssetServer, kit: Option<&AvatarKit>, p: &PlayerPub) -> Entity {
    let mut r = Remote::default();
    r.push(0.0, p.x, p.z, p.ry, p.y);
    let mut e = commands.spawn((
        r,
        avatar_scene(assets),
        Transform::from_xyz(p.x, p.y, p.z),
        ActorName(p.name.clone()),
        Name::new(p.name.clone()),
    ));
    // o cfg vem de outro jogador: validado contra as faixas do catálogo antes de usar
    if let Some(cfg) = kit.and_then(|k| k.palettes.clean(&p.cfg)) {
        e.insert(AvatarLook(cfg));
    }
    e.id()
}

#[allow(clippy::too_many_arguments)]
fn finish_connect(
    mut commands: Commands,
    pending: Option<Res<Pending>>,
    assets: Res<AssetServer>,
    mut load: MessageWriter<LoadWorld>,
    mut me: Query<(Entity, &mut Transform), With<Me>>,
    mut session: ResMut<Session>,
    mut err: ResMut<ConnectError>,
    mut next: ResMut<NextState<Screen>>,
    kit: Option<Res<AvatarKit>>,
    my: Option<Res<crate::editor::MyLook>>,
) {
    let Some(p) = pending else { return };
    let r = p.0.lock().ok().and_then(|rx| rx.try_recv().ok());
    let Some(r) = r else { return };
    commands.remove_resource::<Pending>();
    let h = match r {
        Ok(h) => h,
        Err(e) => {
            error!("não foi possível entrar: {e}");
            err.0 = Some(e);
            next.set(Screen::Menu);
            return;
        }
    };
    let j = &h.joined;
    info!("sala {} ({}), você é {}", sky_shared::format_room_code(&j.code), j.style, j.id);
    load.write(LoadWorld(j.style.parse().unwrap_or(World::Amazora)));
    if let Ok((e, mut t)) = me.single_mut() {
        t.translation = Vec3::new(j.me.x, j.me.y, j.me.z);
        commands.entity(e).insert(ActorName(j.me.name.clone()));
    }
    let mut actors = HashMap::new();
    for p in j.players.iter().filter(|p| p.id != j.id) {
        actors.insert(p.id, spawn_actor(&mut commands, &assets, kit.as_deref(), p));
    }
    *session = Session {
        online: true,
        code: j.code.clone(),
        people: j.players.iter().filter(|p| !p.bot).count() as u32,
        max: j.max,
        my_name: j.me.name.clone(),
    };
    // conta criada agora: o código de recuperação aparece para o jogador guardar (só na memória)
    if let (Some(acc), Some(my)) = (&h.new_account, &my) {
        match crate::recovery::code_for(acc, &j.me.name, &my.cfg) {
            Some(c) => commands.insert_resource(c),
            None => error!("não foi possível montar o código de recuperação"),
        }
    }
    let me_id = j.id;
    commands.insert_resource(Net { h: Mutex::new(h), actors, me: me_id, last_sent: None, send_t: 0.0 });
    next.set(Screen::Playing);
}

#[allow(clippy::too_many_arguments)]
fn receive(
    mut commands: Commands,
    net: Option<ResMut<Net>>,
    assets: Res<AssetServer>,
    time: Res<Time<Real>>,
    mut remotes: Query<&mut Remote>,
    mut log: ResMut<ChatLog>,
    mut session: ResMut<Session>,
    mut next: ResMut<NextState<Screen>>,
    mut err: ResMut<ConnectError>,
    kit: Option<Res<AvatarKit>>,
    mut looks: Query<&mut AvatarLook>,
    gcat: Res<GestureCatalog>,
    gests: Query<&Gesture>,
) {
    let Some(mut net) = net else { return };
    let now = time.elapsed_secs_f64() * 1000.0;
    let mut events = Vec::new();
    if let Ok(mut h) = net.h.lock() {
        while let Some(e) = h.try_recv() {
            events.push(e);
            if events.len() > 512 {
                break;
            }
        }
    }
    for e in events {
        match e {
            NetEvent::State(states) => {
                for s in states {
                    if let Some(&ent) = net.actors.get(&s.id)
                        && let Ok(mut r) = remotes.get_mut(ent)
                    {
                        r.push(now, s.x, s.z, s.ry, s.y);
                        r.sitting = s.sitting;
                    }
                }
            }
            NetEvent::Server(ServerMsg::PlayerJoined(p)) if p.id != net.me => {
                let ent = spawn_actor(&mut commands, &assets, kit.as_deref(), &p);
                if let Some(old) = net.actors.insert(p.id, ent) {
                    commands.entity(old).despawn();
                }
                if !p.bot {
                    session.people += 1;
                }
                log.push("", format!("{} entrou na sala.", p.name), true);
            }
            NetEvent::Server(ServerMsg::PlayerLeft { id }) => {
                if let Some(ent) = net.actors.remove(&id) {
                    commands.entity(ent).despawn();
                    if !id.bot {
                        session.people = session.people.saturating_sub(1);
                    }
                }
            }
            NetEvent::Server(ServerMsg::Cap { max }) => session.max = max,
            NetEvent::Server(ServerMsg::Gest { id, g }) => {
                if let Some(&ent) = net.actors.get(&id) {
                    // como startGest: "stop" para; outro gesto só depois de 1 s do anterior
                    if g == "stop" {
                        commands.entity(ent).remove::<Gesture>();
                    } else if let Some(dur) = gcat.duration(&g)
                        && gests.get(ent).map_or(true, |cur| cur.e > 1.0)
                    {
                        commands.entity(ent).insert(Gesture { id: g, e: 0.0, dur });
                    }
                }
            }
            NetEvent::Server(ServerMsg::Cfg { id, cfg }) => {
                if let (Some(&ent), Some(kit)) = (net.actors.get(&id), kit.as_deref())
                    && let Some(c) = kit.palettes.clean(&cfg)
                {
                    match looks.get_mut(ent) {
                        Ok(mut l) => l.0 = c,
                        Err(_) => {
                            commands.entity(ent).insert(AvatarLook(c));
                        }
                    }
                }
            }
            NetEvent::Server(ServerMsg::Chat { name, text, .. }) => {
                log.push(name, sky_security::clean_chat(&text, sky_shared::limits::CHAT), false);
            }
            NetEvent::Server(ServerMsg::Sys { text }) => log.push("", text, true),
            NetEvent::Server(ServerMsg::Err { msg, .. }) => log.push("", format!("⚠ {msg}"), true),
            NetEvent::Closed(r) => {
                warn!("conexão encerrada {r}");
                for (_, e) in net.actors.drain() {
                    commands.entity(e).despawn();
                }
                commands.remove_resource::<Net>();
                err.0 = Some("A conexão com o servidor caiu.".into());
                next.set(Screen::Menu);
                return;
            }
            _ => {}
        }
    }
}

/// Envia a posição a cada 100 ms quando muda (ou a cada 2 s), como o cliente web.
fn send_moves(time: Res<Time<Real>>, net: Option<ResMut<Net>>, me: Query<(&Transform, &Me)>) {
    let (Some(mut net), Ok((t, me))) = (net, me.single()) else { return };
    let sit = u8::from(me.seat.is_some());
    let now = time.elapsed_secs_f64();
    if now - net.send_t < 0.1 {
        return;
    }
    net.send_t = now;
    let ry = t.rotation.to_euler(EulerRot::YXZ).0;
    let key = format!("{:.2},{:.2},{:.2},{:.2}{sit}", t.translation.x, t.translation.z, ry, t.translation.y);
    let due = match &net.last_sent {
        Some((k, at)) => *k != key || now - at > 2.0,
        None => true,
    };
    if due {
        if let Ok(h) = net.h.lock() {
            h.send(&ClientMsg::Move { x: t.translation.x, z: t.translation.z, ry, y: t.translation.y, s: sit });
        }
        net.last_sent = Some((key, now));
    }
}

fn send_chat(mut msgs: MessageReader<SendChat>, net: Option<Res<Net>>, mut log: ResMut<ChatLog>) {
    for SendChat(text) in msgs.read() {
        match &net {
            Some(net) => {
                if let Ok(h) = net.h.lock() {
                    h.send(&ClientMsg::Chat { text: text.clone() });
                }
            }
            None => log.push("", "Sem conexão: entre numa sala para conversar.", true),
        }
    }
}

/// Ao sair da sala (voltar ao menu), avisa o servidor e fecha a conexão.
fn leave(mut commands: Commands, net: Option<Res<Net>>, mut s: ResMut<Session>) {
    if let Some(net) = net {
        if let Ok(h) = net.h.lock() {
            h.send(&ClientMsg::Leave);
        }
        for e in net.actors.values() {
            commands.entity(*e).despawn();
        }
        commands.remove_resource::<Net>();
    }
    *s = Session::default();
}

/// Gesto do jogador local: aplica aqui (o servidor só repassa para os outros) e envia.
fn send_gesture(
    mut commands: Commands,
    mut msgs: MessageReader<SendGesture>,
    net: Option<Res<Net>>,
    me: Query<Entity, With<Me>>,
    gcat: Res<GestureCatalog>,
) {
    for SendGesture(g) in msgs.read() {
        let Ok(e) = me.single() else { continue };
        if g == "stop" {
            commands.entity(e).remove::<Gesture>();
        } else if let Some(dur) = gcat.duration(g) {
            commands.entity(e).insert(Gesture { id: g.clone(), e: 0.0, dur });
        } else {
            continue;
        }
        if let Some(net) = &net
            && let Ok(h) = net.h.lock()
        {
            h.send(&ClientMsg::Gest { g: g.clone() });
        }
    }
}

/// Visual novo do jogador (o editor já aplicou aqui e espera 250 ms entre mudanças).
fn send_cfg(mut msgs: MessageReader<SendCfg>, net: Option<Res<Net>>) {
    for SendCfg(cfg) in msgs.read() {
        if let Some(net) = &net
            && let Ok(h) = net.h.lock()
        {
            h.send(&ClientMsg::Cfg { cfg: cfg.clone() });
        }
    }
}
