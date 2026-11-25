//! Tela inicial (o `#start` do original): nome, mundo, bots, servidor, criar sala, entrar com código ou explorar
//! sem conexão. O mundo escolhido aparece ao fundo, como no jogo web.

use crate::args::Args;
use crate::state::{ConnectError, ConnectRequest, Screen, Session};
use bevy::prelude::*;
use bevy::text::EditableText;
use sky_shared::World;
use sky_ui::{Activate, Theme, UiFonts, button, input_value, panel, text_input};
use sky_world::LoadWorld;

#[derive(Debug)]
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(Screen::Menu), spawn_menu)
            .add_systems(OnExit(Screen::Menu), despawn::<MenuRoot>)
            .add_systems(OnEnter(Screen::Connecting), spawn_connecting)
            .add_systems(OnExit(Screen::Connecting), despawn::<ConnectingRoot>)
            .add_systems(Update, update_world_label.run_if(in_state(Screen::Menu)));
    }
}

#[derive(Component)]
struct MenuRoot;
#[derive(Component)]
struct ConnectingRoot;
#[derive(Component)]
struct NameField;
#[derive(Component)]
struct ServerField;
#[derive(Component)]
struct CodeField;
#[derive(Component)]
struct WorldLabel;
#[derive(Component)]
struct BotsLabel;

/// Escolhas da tela inicial.
#[derive(Resource, Debug, Clone)]
pub struct MenuChoice {
    pub world: World,
    pub bots: u32,
}

pub fn despawn<T: Component>(mut commands: Commands, q: Query<Entity, With<T>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

fn label(f: &UiFonts, t: &Theme, s: &str) -> impl Bundle {
    (Text::new(s), f.font(14.0, 700), TextColor(t.strong))
}

#[allow(clippy::too_many_lines)]
fn spawn_menu(
    mut commands: Commands,
    t: Res<Theme>,
    f: Res<UiFonts>,
    a: Res<Args>,
    err: Res<ConnectError>,
    choice: Option<Res<MenuChoice>>,
    mut load: MessageWriter<LoadWorld>,
) {
    let choice = choice.map(|c| c.clone()).unwrap_or(MenuChoice { world: a.world, bots: a.bots.min(20) });
    load.write(LoadWorld(choice.world));
    commands.insert_resource(choice.clone());
    let start = Theme { panel: t.start_panel, border: t.start_border, accent: t.start_accent, ..*t };
    let server = a.server.clone().unwrap_or_else(|| "https://skyke3d.com.br".into());

    let root = commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(2.5),
                top: Val::Percent(2.5),
                bottom: Val::Percent(2.5),
                width: Val::Px(520.0),
                ..default()
            },
        ))
        .id();
    let p = commands
        .spawn(panel(&start))
        .insert(Node {
            width: Val::Percent(100.0),
            padding: UiRect::all(Val::Px(22.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(18.0)),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(12.0),
            overflow: Overflow::scroll_y(),
            ..default()
        })
        .id();
    commands.entity(root).add_child(p);

    let title = commands.spawn((Text::new("Skyke3D"), f.font(44.0, 800), TextColor(start.accent))).id();
    let sub = commands
        .spawn((
            Text::new("Entre numa sala com os amigos, converse, cante e explore os seis mundos."),
            f.font(15.0, 500),
            TextColor(start.fg),
        ))
        .id();
    commands.entity(p).add_children(&[title, sub]);

    // nome
    let l = commands.spawn(label(&f, &start, "Seu nome")).id();
    let name = commands.spawn((text_input(&start, &f, &a.name_or_empty(), Val::Percent(100.0)), NameField)).id();
    commands.entity(p).add_children(&[l, name]);

    // mundo ◀ ▶
    let l = commands.spawn(label(&f, &start, "Mundo")).id();
    let row = commands.spawn(Node { column_gap: Val::Px(8.0), align_items: AlignItems::Center, ..default() }).id();
    let prev = button(&mut commands, &start, &f, "◀", false);
    let wl = commands.spawn((Text::new(""), f.font(17.0, 700), TextColor(start.fg), WorldLabel)).id();
    let next = button(&mut commands, &start, &f, "▶", false);
    commands.entity(prev).observe(|_: On<Activate>, mut c: ResMut<MenuChoice>, mut load: MessageWriter<LoadWorld>| {
        c.world = step_world(c.world, -1);
        load.write(LoadWorld(c.world));
    });
    commands.entity(next).observe(|_: On<Activate>, mut c: ResMut<MenuChoice>, mut load: MessageWriter<LoadWorld>| {
        c.world = step_world(c.world, 1);
        load.write(LoadWorld(c.world));
    });
    commands.entity(row).add_children(&[prev, wl, next]);
    commands.entity(p).add_children(&[l, row]);

    // bots − n +
    let l = commands.spawn(label(&f, &start, "Bots na sala nova")).id();
    let row = commands.spawn(Node { column_gap: Val::Px(8.0), align_items: AlignItems::Center, ..default() }).id();
    let minus = button(&mut commands, &start, &f, "−", false);
    let bl = commands.spawn((Text::new(""), f.font(17.0, 700), TextColor(start.fg), BotsLabel)).id();
    let plus = button(&mut commands, &start, &f, "+", false);
    commands.entity(minus).observe(|_: On<Activate>, mut c: ResMut<MenuChoice>| c.bots = c.bots.saturating_sub(1));
    commands.entity(plus).observe(|_: On<Activate>, mut c: ResMut<MenuChoice>| c.bots = (c.bots + 1).min(20));
    commands.entity(row).add_children(&[minus, bl, plus]);
    commands.entity(p).add_children(&[l, row]);

    // servidor
    let l = commands.spawn(label(&f, &start, "Servidor")).id();
    let srv = commands.spawn((text_input(&start, &f, &server, Val::Percent(100.0)), ServerField)).id();
    commands.entity(p).add_children(&[l, srv]);

    // criar
    let create = button(&mut commands, &start, &f, "Criar sala", true);
    commands.entity(create).observe(connect_from_menu(false));
    commands.entity(p).add_child(create);

    // entrar com código
    let l = commands.spawn(label(&f, &start, "Ou entre com o código da sala")).id();
    let row = commands.spawn(Node { column_gap: Val::Px(8.0), align_items: AlignItems::Center, ..default() }).id();
    let code =
        commands.spawn((text_input(&start, &f, a.join.as_deref().unwrap_or(""), Val::Px(220.0)), CodeField)).id();
    let join = button(&mut commands, &start, &f, "Entrar", false);
    commands.entity(join).observe(connect_from_menu(true));
    commands.entity(row).add_children(&[code, join]);
    commands.entity(p).add_children(&[l, row]);

    // explorar
    let explore = button(&mut commands, &start, &f, "Explorar sem conexão", false);
    commands.entity(explore).observe(|_: On<Activate>, mut s: ResMut<Session>, mut next: ResMut<NextState<Screen>>| {
        *s = Session::default();
        next.set(Screen::Playing);
    });
    commands.entity(p).add_child(explore);

    if let Some(e) = &err.0 {
        let m = commands
            .spawn((Text::new(format!("⚠ {e}")), f.font(15.0, 600), TextColor(Color::srgb(1.0, 0.55, 0.55))))
            .id();
        commands.entity(p).add_child(m);
    }
    let help = commands
        .spawn((
            Text::new("Teclas: W A S D ou setas para andar · Q e E giram a câmera · Enter abre o chat"),
            f.font(13.0, 500),
            TextColor(start.fg.with_alpha(0.75)),
        ))
        .id();
    commands.entity(p).add_child(help);
}

fn step_world(w: World, d: i32) -> World {
    let i = World::ALL.iter().position(|x| *x == w).unwrap_or(0) as i32;
    World::ALL[(i + d).rem_euclid(World::ALL.len() as i32) as usize]
}

#[allow(clippy::type_complexity)]
fn connect_from_menu(
    join: bool,
) -> impl FnMut(
    On<Activate>,
    Query<&EditableText, With<NameField>>,
    Query<&EditableText, With<ServerField>>,
    Query<&EditableText, With<CodeField>>,
    Res<MenuChoice>,
    ResMut<ConnectError>,
    MessageWriter<ConnectRequest>,
    ResMut<NextState<Screen>>,
) {
    move |_, name, server, code, choice, mut err, mut req, mut next| {
        let name = name.single().map(input_value).unwrap_or_default().trim().to_string();
        let server = server.single().map(input_value).unwrap_or_default().trim().to_string();
        let code = code.single().map(input_value).unwrap_or_default();
        if name.is_empty() || name.eq_ignore_ascii_case("visitante") {
            err.0 = Some("Escolha um nome para entrar.".into());
            next.set(Screen::Menu);
            return;
        }
        let join = if join {
            match sky_shared::normalize_room_code(&code) {
                Some(c) => Some(c),
                None => {
                    err.0 = Some("Digite o código da sala (10 letras e números).".into());
                    next.set(Screen::Menu);
                    return;
                }
            }
        } else {
            None
        };
        err.0 = None;
        req.write(ConnectRequest { server, name, join, world: choice.world, bots: choice.bots });
        next.set(Screen::Connecting);
    }
}

fn update_world_label(
    c: Res<MenuChoice>,
    mut w: Query<&mut Text, (With<WorldLabel>, Without<BotsLabel>)>,
    mut b: Query<&mut Text, (With<BotsLabel>, Without<WorldLabel>)>,
) {
    if !c.is_changed() {
        return;
    }
    for mut t in &mut w {
        t.0 = format!("{} · {}", c.world.display_name(), c.world.biome());
    }
    for mut t in &mut b {
        t.0 = c.bots.to_string();
    }
}

fn spawn_connecting(mut commands: Commands, t: Res<Theme>, f: Res<UiFonts>) {
    let root = commands
        .spawn((
            ConnectingRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .id();
    let p = commands.spawn(panel(&t)).id();
    let m = commands.spawn((Text::new("Conectando ao servidor…"), f.font(18.0, 700), TextColor(t.fg))).id();
    commands.entity(p).add_child(m);
    commands.entity(root).add_child(p);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_stepping_wraps() {
        assert_eq!(step_world(World::Amazora, -1), World::Pamphora);
        assert_eq!(step_world(World::Pamphora, 1), World::Amazora);
        assert_eq!(step_world(World::Cerron, 1), World::Atlanthia);
    }
}
