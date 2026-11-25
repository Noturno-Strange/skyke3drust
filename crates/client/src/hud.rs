//! Interface dentro da sala: código da sala e lotação (canto superior), chat da sala (canto inferior esquerdo, 380×210
//! como `#chat` no original) e o nome sobre cada avatar (`.lbl`).

use crate::gestures::{Gesture, GestureCatalog};
use crate::menu::despawn;
use crate::player::Me;
use crate::state::{ActorName, ChatLog, Screen, SendChat, SendGesture, Session};
use bevy::input_focus::{FocusCause, InputFocus};
use bevy::prelude::*;
use bevy::text::EditableText;
use sky_ui::{Activate, Theme, UiFonts, button, input_value, panel, text_input};

#[derive(Debug)]
pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Screen::Playing), spawn_hud)
            .add_systems(OnExit(Screen::Playing), (despawn::<HudRoot>, despawn::<NameTag>))
            .add_systems(
                Update,
                (chat_keys, refresh_log, refresh_room, spawn_tags, place_tags, stop_gesture_on_walk)
                    .run_if(in_state(Screen::Playing)),
            );
    }
}

#[derive(Component)]
struct HudRoot;
/// Painel de gestos (aberto e fechado pelo botão 🎭 da barra).
#[derive(Component)]
struct GesturePanel;
#[derive(Component)]
struct ChatLogText;
#[derive(Component)]
struct ChatInput;
#[derive(Component)]
struct RoomText;
/// Rótulo de nome que segue um ator.
#[derive(Component)]
struct NameTag(Entity);
#[derive(Component)]
struct HasTag;

fn spawn_hud(
    mut commands: Commands,
    t: Res<Theme>,
    f: Res<UiFonts>,
    s: Res<Session>,
    mut log: ResMut<ChatLog>,
    gcat: Res<GestureCatalog>,
) {
    let root = commands
        .spawn((
            HudRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .id();
    // sala e lotação
    let room = commands
        .spawn(panel(&t))
        .insert(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            top: Val::Px(12.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(12.0)),
            ..default()
        })
        .id();
    let rt = commands.spawn((Text::new(""), f.font(15.0, 700), TextColor(t.fg), RoomText)).id();
    commands.entity(room).add_child(rt);

    // chat
    let chat = commands
        .spawn(panel(&t))
        .insert(Node {
            position_type: PositionType::Absolute,
            left: Val::Px(12.0),
            bottom: Val::Px(80.0),
            width: Val::Px(380.0),
            height: Val::Px(210.0),
            padding: UiRect::all(Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(12.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        })
        .id();
    let title = commands.spawn((Text::new("💬 Sala"), f.font(14.0, 700), TextColor(t.strong))).id();
    let logbox = commands
        .spawn(Node {
            flex_grow: 1.0,
            min_height: Val::Px(0.0),
            overflow: Overflow::clip(),
            flex_direction: FlexDirection::ColumnReverse,
            ..default()
        })
        .id();
    let logtext = commands.spawn((Text::new(""), f.font(16.0, 500), TextColor(t.fg), ChatLogText)).id();
    commands.entity(logbox).add_child(logtext);
    let input = commands.spawn((text_input(&t, &f, "", Val::Percent(100.0)), ChatInput)).id();
    commands.entity(chat).add_children(&[title, logbox, input]);
    commands.entity(root).add_children(&[room, chat]);

    // barra de baixo (`#bar`): por enquanto o botão de gestos
    let bar = commands
        .spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(12.0),
            left: Val::Percent(50.0),
            margin: UiRect::left(Val::Px(-30.0)),
            column_gap: Val::Px(8.0),
            ..default()
        })
        .id();
    let gb = button(&mut commands, &t, &f, "🎭", false);
    commands.entity(gb).observe(|_: On<Activate>, mut q: Query<&mut Node, With<GesturePanel>>| {
        for mut n in &mut q {
            n.display = if n.display == Display::None { Display::Flex } else { Display::None };
        }
    });
    commands.entity(bar).add_child(gb);
    // painel de gestos: os 30 do catálogo, com ícone e nome
    let gp = commands
        .spawn(panel(&t))
        .insert((
            GesturePanel,
            Node {
                position_type: PositionType::Absolute,
                bottom: Val::Px(64.0),
                left: Val::Percent(50.0),
                margin: UiRect::left(Val::Px(-260.0)),
                width: Val::Px(520.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(6.0),
                row_gap: Val::Px(6.0),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    for g in &gcat.list {
        let b = button(&mut commands, &t, &f, &format!("{} {}", g.ic, g.n), false);
        let id = g.id.clone();
        commands.entity(b).observe(
            move |_: On<Activate>,
                  mut send: MessageWriter<SendGesture>,
                  mut q: Query<&mut Node, With<GesturePanel>>| {
                send.write(SendGesture(id.clone()));
                for mut n in &mut q {
                    n.display = Display::None;
                }
            },
        );
        commands.entity(gp).add_child(b);
    }
    commands.entity(root).add_children(&[bar, gp]);

    if s.online {
        log.push("", "Você entrou na sala. Enter abre o chat; Esc volta para o jogo.", true);
    } else {
        log.push("", "Explorando sem conexão: o chat e os outros jogadores aparecem ao entrar numa sala.", true);
    }
}

/// Enter abre o chat (ou envia), Esc devolve o teclado para o jogo. Mensagens a cada 800 ms no máximo, como no original.
fn chat_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut focus: ResMut<InputFocus>,
    mut input: Query<(Entity, &mut EditableText), With<ChatInput>>,
    mut send: MessageWriter<SendChat>,
    mut log: ResMut<ChatLog>,
    time: Res<Time<Real>>,
    mut last: Local<f64>,
) {
    let Ok((e, mut text)) = input.single_mut() else { return };
    let focused = focus.get() == Some(e);
    if keys.just_pressed(KeyCode::Escape) && focused {
        focus.clear();
        return;
    }
    if !keys.just_pressed(KeyCode::Enter) && !keys.just_pressed(KeyCode::NumpadEnter) {
        return;
    }
    if !focused {
        focus.set(e, FocusCause::Navigated);
        return;
    }
    let msg = sky_security::clean_chat(&input_value(&text), sky_shared::limits::CHAT);
    if msg.is_empty() {
        focus.clear();
        return;
    }
    let now = time.elapsed_secs_f64();
    if now - *last < 0.8 {
        log.push("", "Calma! Aguarde um instante entre mensagens.", true);
        return;
    }
    *last = now;
    text.clear();
    send.write(SendChat(msg));
}

/// O teclado está com um campo de texto? (o movimento ignora as teclas nesse caso)
pub fn typing(focus: &InputFocus, inputs: &Query<(), With<EditableText>>) -> bool {
    focus.get().is_some_and(|e| inputs.contains(e))
}

fn refresh_log(log: Res<ChatLog>, mut q: Query<&mut Text, With<ChatLogText>>, mut seen: Local<u64>) {
    if log.version == *seen {
        return;
    }
    *seen = log.version;
    let start = log.lines.len().saturating_sub(40);
    let s = log.lines[start..]
        .iter()
        .map(|l| if l.system || l.name.is_empty() { l.text.clone() } else { format!("{}: {}", l.name, l.text) })
        .collect::<Vec<_>>()
        .join("\n");
    for mut t in &mut q {
        t.0.clone_from(&s);
    }
}

fn refresh_room(s: Res<Session>, mut q: Query<&mut Text, With<RoomText>>) {
    if !s.is_changed() {
        return;
    }
    let txt = if s.online {
        format!("🔗 Sala {} · {}/{}", sky_shared::format_room_code(&s.code), s.people, s.max.max(1))
    } else {
        "🧭 Explorando sem conexão".into()
    };
    for mut t in &mut q {
        t.0.clone_from(&txt);
    }
}

fn spawn_tags(
    mut commands: Commands,
    t: Res<Theme>,
    f: Res<UiFonts>,
    actors: Query<(Entity, &ActorName), Without<HasTag>>,
) {
    for (e, n) in &actors {
        commands.entity(e).insert(HasTag);
        commands
            .spawn((
                NameTag(e),
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(Val::Px(8.0), Val::Px(1.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    ..default()
                },
                BackgroundColor(t.panel),
                BorderColor::all(t.border),
                Pickable::IGNORE,
            ))
            .with_children(|c| {
                c.spawn((Text::new(n.0.clone()), f.font(13.0, 600), TextColor(t.fg)));
            });
    }
}

/// Posiciona cada rótulo 2,25 m acima dos pés do avatar; some quando o avatar sai da tela ou do mundo.
fn place_tags(
    mut commands: Commands,
    cam: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    actors: Query<&GlobalTransform, With<ActorName>>,
    mut tags: Query<(Entity, &NameTag, &mut Node, &ComputedNode, &mut Visibility)>,
) {
    let Ok((camera, cam_t)) = cam.single() else { return };
    for (e, tag, mut node, computed, mut vis) in &mut tags {
        let Ok(at) = actors.get(tag.0) else {
            commands.entity(e).despawn();
            continue;
        };
        let p = at.translation() + Vec3::Y * 2.25;
        match camera.world_to_viewport(cam_t, p) {
            Ok(v) => {
                let size = computed.size() * computed.inverse_scale_factor();
                node.left = Val::Px(v.x - size.x / 2.0);
                node.top = Val::Px(v.y - size.y);
                *vis = Visibility::Inherited;
            }
            Err(_) => *vis = Visibility::Hidden,
        }
    }
}

/// Andar para o gesto (como no original, que envia `stop`).
fn stop_gesture_on_walk(me: Query<(&Me, Has<Gesture>)>, mut send: MessageWriter<SendGesture>) {
    if let Ok((me, has)) = me.single()
        && me.moving
        && has
    {
        send.write(SendGesture("stop".into()));
    }
}
