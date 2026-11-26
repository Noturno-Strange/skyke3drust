//! Botões sociais da barra (`#bar` no original): 😀 emojis sobre a cabeça, 👥 jogadores (silenciar ou denunciar),
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
struct PlayersState {
    reporting: Option<ActorId>,
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
    let title = commands.spawn((Text::new("👥 Jogadores"), f.font(15.0, 700), TextColor(t.strong))).id();
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

    commands.entity(bar).add_children(&[emo, pl, inv, exit]);
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

#[allow(clippy::too_many_arguments)]
fn rebuild_players(
    mut commands: Commands,
    mut st: ResMut<PlayersState>,
    muted: Res<Muted>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    rows: Query<(Entity, Option<&Children>), With<PlayersRows>>,
    panel: Query<&Node, With<PlayersPanel>>,
    actors: Query<(&NetId, &ActorName), Changed<ActorName>>,
    all: Query<(&NetId, &ActorName)>,
    mut removed: RemovedComponents<NetId>,
) {
    let changed = !actors.is_empty() || removed.read().count() > 0;
    if !(st.dirty || changed && panel.iter().any(|n| n.display != Display::None)) {
        return;
    }
    st.dirty = false;
    let Ok((root, kids)) = rows.single() else { return };
    if let Some(kids) = kids {
        for c in kids {
            commands.entity(*c).despawn();
        }
    }
    let mut list: Vec<(ActorId, String)> =
        all.iter().filter(|(id, _)| !id.0.bot).map(|(id, n)| (id.0, n.0.clone())).collect();
    list.sort_by_key(|a| a.1.to_lowercase());
    let mut out = Vec::new();
    if list.is_empty() {
        out.push(
            commands.spawn((Text::new("Ninguém mais na sala por enquanto."), f.font(15.0, 500), TextColor(t.fg))).id(),
        );
    }
    for (id, name) in list {
        let row = commands.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(6.0), ..default() }).id();
        let n = commands
            .spawn((Text::new(name), f.font(15.0, 600), TextColor(t.fg), Node { flex_grow: 1.0, ..default() }))
            .id();
        let is_muted = muted.0.contains(&id);
        let m = button(&mut commands, &t, &f, if is_muted { "🔊 Ouvir" } else { "🔇 Silenciar" }, false);
        commands.entity(m).observe(move |_: On<Activate>, mut muted: ResMut<Muted>, mut st: ResMut<PlayersState>| {
            if !muted.0.remove(&id) {
                muted.0.insert(id);
            }
            st.dirty = true;
        });
        let r = button(&mut commands, &t, &f, "🚩 Denunciar", false);
        commands.entity(r).observe(move |_: On<Activate>, mut st: ResMut<PlayersState>| {
            st.reporting = if st.reporting == Some(id) { None } else { Some(id) };
            st.dirty = true;
        });
        commands.entity(row).add_children(&[n, m, r]);
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
