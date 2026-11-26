//! Botões sociais da barra (`#bar` no original): 😀 emojis sobre a cabeça, 👥 amigos e jogadores (pedidos, convites,
//! silenciar ou denunciar),
//! 🔗 convidar (copia o link da sala) e 🚪 sair da sala.

use crate::editor::EditorCatalog;
use crate::player::Me;
use crate::state::{ActorName, ChatLog, Emote, Muted, NetId, Screen, SendMsg, Session};
use bevy::clipboard::Clipboard;
use bevy::prelude::*;
use sky_protocol::ClientMsg;
use sky_shared::ActorId;
use sky_ui::{Activate, Theme, UiFonts, button, panel};

#[derive(Debug)]
pub struct SocialPlugin;

impl Plugin for SocialPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Muted>()
            .init_resource::<PlayersState>()
            .add_systems(OnExit(Screen::Playing), reset)
            .add_systems(Update, (tick_emotes, rebuild_players).run_if(in_state(Screen::Playing)));
    }
}

/// Painel de emojis (botão 😀).
#[derive(Component, Debug)]
pub struct EmojiPanel;
/// Painel de jogadores (botão 👥).
#[derive(Component, Debug)]
pub struct PlayersPanel;
#[derive(Component, Debug)]
struct PlayersRows;

/// Denúncia aberta (mostra os motivos) e se a lista precisa ser refeita.
#[derive(Resource, Debug, Default)]
pub struct PlayersState {
    reporting: Option<ActorId>,
    /// Amigo com a remoção pedida (o segundo clique confirma).
    removing: Option<String>,
    dirty: bool,
}

/// Motivos de denúncia (`REPORT_REASONS` em `lib/validate.js`).
const REASONS: [(&str, &str); 7] = [
    ("assedio", "Assédio ou ofensa"),
    ("odio", "Discurso de ódio ou discriminação"),
    ("sexual", "Conteúdo sexual ou impróprio"),
    ("spam", "Spam ou flood"),
    ("golpe", "Golpe, link malicioso ou dados pessoais"),
    ("trapaca", "Trapaça ou abuso de falha"),
    ("outro", "Outro motivo"),
];

fn reset(mut muted: ResMut<Muted>, mut st: ResMut<PlayersState>) {
    muted.0.clear();
    *st = PlayersState::default();
}

fn toggle<T: Component>(q: &mut Query<&mut Node, With<T>>) {
    for mut n in q.iter_mut() {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
    }
}

fn side_panel(commands: &mut Commands, t: &Theme, width: f32) -> Entity {
    commands
        .spawn(panel(t))
        .insert(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(64.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-width / 2.0)),
            width: Val::Px(width),
            max_height: Val::Percent(70.0),
            padding: UiRect::all(Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(12.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            overflow: Overflow::scroll_y(),
            display: Display::None,
            ..default()
        })
        .id()
}

/// Cria os botões sociais na barra e os painéis deles (chamado ao montar a interface da sala).
pub fn spawn(commands: &mut Commands, t: &Theme, f: &UiFonts, cat: &EditorCatalog, bar: Entity, root: Entity) {
    // 😀 emojis: a grade do catálogo; o escolhido aparece sobre a cabeça e vai para a sala
    let emo = button(commands, t, f, "😀", false);
    commands.entity(emo).observe(|_: On<Activate>, mut q: Query<&mut Node, With<EmojiPanel>>| toggle(&mut q));
    let ep = side_panel(commands, t, 420.0);
    commands.entity(ep).insert(EmojiPanel);
    let grid = commands
        .spawn(Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(6.0), row_gap: Val::Px(6.0), ..default() })
        .id();
    for (i, ic) in cat.emojis().iter().enumerate() {
        let b = button(commands, t, f, ic, false);
        let (i, ic) = (i as u32, ic.clone());
        commands.entity(b).observe(
            move |_: On<Activate>,
                  mut commands: Commands,
                  mut send: MessageWriter<SendMsg>,
                  me: Query<Entity, With<Me>>,
                  mut q: Query<&mut Node, With<EmojiPanel>>| {
                send.write(SendMsg(ClientMsg::Emo { i }));
                if let Ok(e) = me.single() {
                    commands.entity(e).insert(Emote { ic: ic.clone(), left: 4.0 });
                }
                toggle(&mut q);
            },
        );
        commands.entity(grid).add_child(b);
    }
    commands.entity(ep).add_child(grid);

    // 👥 jogadores da sala: silenciar (só aqui) ou denunciar (vai ao servidor)
    let pl = button(commands, t, f, "👥", false);
    commands.entity(pl).observe(
        |_: On<Activate>, mut q: Query<&mut Node, With<PlayersPanel>>, mut st: ResMut<PlayersState>| {
            toggle(&mut q);
            st.reporting = None;
            st.dirty = true;
        },
    );
    let pp = side_panel(commands, t, 480.0);
    commands.entity(pp).insert(PlayersPanel);
    let title = commands.spawn((Text::new("👥 Amigos e jogadores"), f.font(15.0, 700), TextColor(t.strong))).id();
    let rows = commands
        .spawn((PlayersRows, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default() }))
        .id();
    commands.entity(pp).add_children(&[title, rows]);

    // 🔗 convidar: copia o link da sala (`#sala=CODIGO`, como o cliente web)
    let inv = button(commands, t, f, "🔗", false);
    commands.entity(inv).observe(
        |_: On<Activate>, s: Res<Session>, mut cb: ResMut<Clipboard>, mut log: ResMut<ChatLog>| {
            if !s.online {
                log.push("", "Sem conexão: entre numa sala para convidar.", true);
                return;
            }
            let link = format!("{}/#sala={}", s.server, s.code);
            match cb.set_text(link.clone()) {
                Ok(()) => log.push("", format!("🔗 Link da sala copiado: {link}"), true),
                Err(_) => log.push("", format!("Copie o link da sala: {link}"), true),
            }
        },
    );

    // 🚪 sair da sala: volta para a tela inicial (a saída avisa o servidor)
    let exit = button(commands, t, f, "🚪", false);
    commands.entity(exit).observe(|_: On<Activate>, mut next: ResMut<NextState<Screen>>| next.set(Screen::Menu));

    // ⚙️ ajustes
    let set = button(commands, t, f, "⚙️", false);
    commands.entity(set).observe(|_: On<Activate>, mut q: Query<&mut Node, With<crate::settings::SettingsPanel>>| {
        crate::settings::toggle(&mut q);
    });

    // 🎒 mochila
    let bp = button(commands, t, f, "🎒", false);
    commands.entity(bp).observe(crate::backpack::on_toggle);

    // 🌱 fazenda
    let fm = button(commands, t, f, "🌱", false);
    commands.entity(fm).observe(crate::farm::on_toggle);

    // 🍳 culinária
    let ck = button(commands, t, f, "🍳", false);
    commands.entity(ck).observe(crate::cook::on_toggle);

    commands.entity(bar).add_children(&[emo, bp, ck, fm, pl, inv, set, exit]);
    commands.entity(root).add_children(&[ep, pp]);
}

fn tick_emotes(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Emote)>) {
    for (e, mut em) in &mut q {
        em.left -= time.delta_secs();
        if em.left <= 0.0 {
            commands.entity(e).remove::<Emote>();
        }
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn rebuild_players(
    mut commands: Commands,
    mut st: ResMut<PlayersState>,
    muted: Res<Muted>,
    friends: Res<crate::friends::Friends>,
    s: Res<Session>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    rows: Query<(Entity, Option<&Children>), With<PlayersRows>>,
    panel: Query<&Node, With<PlayersPanel>>,
    actors: Query<(&NetId, &ActorName), Changed<ActorName>>,
    all: Query<(&NetId, &ActorName, Option<&crate::state::Fid>)>,
    mut removed: RemovedComponents<NetId>,
    mut seen: Local<u64>,
) {
    let changed = !actors.is_empty() || removed.read().count() > 0 || friends.version != *seen;
    if !(st.dirty || changed && panel.iter().any(|n| n.display != Display::None)) {
        return;
    }
    *seen = friends.version;
    st.dirty = false;
    let Ok((root, kids)) = rows.single() else { return };
    if let Some(kids) = kids {
        for c in kids {
            commands.entity(*c).despawn();
        }
    }
    let mut out = Vec::new();
    let heading = |commands: &mut Commands, txt: String| {
        commands.spawn((Text::new(txt), f.font(14.0, 700), TextColor(t.strong))).id()
    };
    let line = |commands: &mut Commands, name: String, tag: &str| {
        let row = commands
            .spawn(Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(6.0),
                flex_wrap: FlexWrap::Wrap,
                ..default()
            })
            .id();
        let n = commands
            .spawn((
                Text::new(format!("{name}  ")),
                f.font(15.0, 600),
                TextColor(t.fg),
                Node { flex_grow: 1.0, ..default() },
            ))
            .id();
        let tg = commands.spawn((Text::new(tag.to_owned()), f.font(13.0, 500), TextColor(t.fg))).id();
        commands.entity(row).add_children(&[n, tg]);
        row
    };
    // convite recebido
    if let Some((name, _)) = &friends.invite {
        let row = line(&mut commands, format!("📨 {name} chamou você"), "");
        let b = button(&mut commands, &t, &f, "🚪 Entrar", true);
        commands.entity(b).observe(
            |_: On<Activate>,
             mut fr: ResMut<crate::friends::Friends>,
             s: Res<Session>,
             mut req: MessageWriter<crate::state::ConnectRequest>,
             mut next: ResMut<NextState<Screen>>| {
                crate::friends::join_invite(&mut fr, &s, &mut req, &mut next);
            },
        );
        commands.entity(row).add_child(b);
        out.push(row);
    }
    // pedidos de amizade
    if !friends.req.is_empty() {
        out.push(heading(&mut commands, format!("Pedidos de amizade ({})", friends.req.len())));
        for (id, name) in &friends.req {
            let row = line(&mut commands, name.clone(), "quer ser seu amigo");
            let (id1, id2) = (id.clone(), id.clone());
            let ok = button(&mut commands, &t, &f, "✔ Aceitar", true);
            commands.entity(ok).observe(
                move |_: On<Activate>,
                      mut fr: ResMut<crate::friends::Friends>,
                      mut send: MessageWriter<crate::state::SendRaw>,
                      mut log: ResMut<ChatLog>| fr.accept(&id1, &mut send, &mut log),
            );
            let no = button(&mut commands, &t, &f, "✖ Recusar", false);
            commands
                .entity(no)
                .observe(move |_: On<Activate>, mut fr: ResMut<crate::friends::Friends>| fr.decline(&id2));
            commands.entity(row).add_children(&[ok, no]);
            out.push(row);
        }
    }
    // amigos: online primeiro
    let mut list: Vec<&crate::friends::Friend> = friends.fr.iter().collect();
    list.sort_by_key(|x| (!friends.on.contains(&x.id), x.n.to_lowercase()));
    let online = friends.on.len();
    out.push(heading(
        &mut commands,
        format!("Amigos ({}{})", list.len(), if online > 0 { format!(", {online} online") } else { String::new() }),
    ));
    if list.is_empty() {
        out.push(
            commands
                .spawn((Text::new("Adicione alguém da sala: ➕ ao lado do nome."), f.font(14.0, 500), TextColor(t.fg)))
                .id(),
        );
    }
    for fr in list {
        let live = friends.on.contains(&fr.id);
        let row = line(
            &mut commands,
            format!("{} {}", if live { "🟢" } else { "⚪" }, fr.n),
            if live { "online" } else { "offline" },
        );
        if live && s.online {
            let id = fr.id.clone();
            let b = button(&mut commands, &t, &f, "📨 Convidar", false);
            commands.entity(b).observe(
                move |_: On<Activate>,
                      fr: Res<crate::friends::Friends>,
                      mut send: MessageWriter<crate::state::SendRaw>| fr.invite(&id, &mut send),
            );
            commands.entity(row).add_child(b);
        }
        let id = fr.id.clone();
        let rm = button(
            &mut commands,
            &t,
            &f,
            if st.removing.as_deref() == Some(fr.id.as_str()) { "Confirmar remoção" } else { "✖ Remover" },
            false,
        );
        commands.entity(rm).observe(
            move |_: On<Activate>,
                  mut fr: ResMut<crate::friends::Friends>,
                  mut st: ResMut<PlayersState>,
                  mut send: MessageWriter<crate::state::SendRaw>| {
                if st.removing.as_deref() == Some(id.as_str()) {
                    st.removing = None;
                    fr.remove(&id, &mut send);
                } else {
                    st.removing = Some(id.clone());
                }
                st.dirty = true;
            },
        );
        commands.entity(row).add_child(rm);
        out.push(row);
    }
    // quem está na sala
    let mut room: Vec<(ActorId, String, String)> = all
        .iter()
        .filter(|(id, _, _)| !id.0.bot)
        .map(|(id, n, fid)| (id.0, n.0.clone(), fid.map(|x| x.0.clone()).unwrap_or_default()))
        .collect();
    room.sort_by_key(|a| a.1.to_lowercase());
    out.push(heading(&mut commands, format!("Nesta sala ({})", room.len())));
    if room.is_empty() {
        out.push(
            commands.spawn((Text::new("Ninguém mais na sala por enquanto."), f.font(14.0, 500), TextColor(t.fg))).id(),
        );
    }
    for (id, name, fid) in room {
        let is_friend = friends.friend_of_handle(&fid).is_some();
        let row = line(&mut commands, name.clone(), if is_friend { "amigo" } else { "" });
        if !is_friend && crate::friends::ok_id(&fid) && fid != friends.h {
            if friends.sent.contains(&fid) {
                let b = button(&mut commands, &t, &f, "⏳ Pedido enviado", false);
                commands.entity(row).add_child(b);
            } else {
                let b = button(&mut commands, &t, &f, "➕ Adicionar", true);
                let (fid, name) = (fid.clone(), name.clone());
                commands.entity(b).observe(
                    move |_: On<Activate>,
                          mut fr: ResMut<crate::friends::Friends>,
                          mut send: MessageWriter<crate::state::SendRaw>,
                          mut log: ResMut<ChatLog>| fr.request(&fid, &name, &mut send, &mut log),
                );
                commands.entity(row).add_child(b);
            }
        }
        let is_muted = muted.0.contains(&id);
        let m = button(&mut commands, &t, &f, if is_muted { "🔊" } else { "🔇" }, false);
        commands.entity(m).observe(move |_: On<Activate>, mut muted: ResMut<Muted>, mut st: ResMut<PlayersState>| {
            if !muted.0.remove(&id) {
                muted.0.insert(id);
            }
            st.dirty = true;
        });
        let r = button(&mut commands, &t, &f, "🚩", false);
        commands.entity(r).observe(move |_: On<Activate>, mut st: ResMut<PlayersState>| {
            st.reporting = if st.reporting == Some(id) { None } else { Some(id) };
            st.dirty = true;
        });
        commands.entity(row).add_children(&[m, r]);
        out.push(row);
        if st.reporting == Some(id) {
            let reasons = commands
                .spawn(Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(4.0), row_gap: Val::Px(4.0), ..default() })
                .id();
            for (code, label) in REASONS {
                let b = button(&mut commands, &t, &f, label, false);
                commands.entity(b).observe(
                    move |_: On<Activate>,
                          mut send: MessageWriter<SendMsg>,
                          mut log: ResMut<ChatLog>,
                          mut st: ResMut<PlayersState>| {
                        send.write(SendMsg(ClientMsg::Report {
                            id: id.to_string(),
                            reason: code.into(),
                            note: String::new(),
                        }));
                        log.push("", "✔ Denúncia enviada. Obrigado por ajudar a manter o jogo seguro.", true);
                        st.reporting = None;
                        st.dirty = true;
                    },
                );
                commands.entity(reasons).add_child(b);
            }
            out.push(reasons);
        }
    }
    commands.entity(root).add_children(&out);
}
