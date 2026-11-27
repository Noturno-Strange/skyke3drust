//! Tela inicial (o `#start` do original, `index.html` + `assets/app.css`): um painel à esquerda com o logo e o botão de
//! música, as seções numeradas **1 · Seu nome**, **2 · Estilo do cenário**, **3 · Seu avatar** (o mesmo editor do
//! painel 👕) e **4 · Termos**, e a barra fixa embaixo com **✨ Criar sala**, o código da sala, **Entrar** e os 🤖 bots.
//! O mundo escolhido e o avatar aparecem ao fundo, como no jogo web.
//!
//! Só o cliente nativo tem a seção do servidor (o navegador já está nele) e o modo sem conexão.

use crate::args::Args;
use crate::look::AvatarKit;
use crate::settings::Settings;
use crate::state::{ConnectError, ConnectRequest, Screen, Session};
use bevy::input_focus::tab_navigation::TabIndex;
use bevy::prelude::*;
use bevy::text::{EditableText, EditableTextFilter};
use bevy::ui_widgets::Button;
use sky_shared::World;
use sky_ui::{Activate, FocusRing, Theme, ThemeKind, UiFonts, button, input_value, panel, text_input_with};
use sky_world::LoadWorld;
use std::sync::{Arc, Mutex};

/// Versão dos Termos de Uso e da Política de Privacidade (`TERMS_VER` em `assets/app-net.js`).
pub const TERMS_VER: &str = "2026-11-15.2";

#[derive(Debug)]
pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MenuUi>()
            .add_systems(OnEnter(Screen::Menu), spawn_menu)
            .add_systems(OnExit(Screen::Menu), (despawn::<MenuRoot>, despawn::<TermsModal>))
            .add_systems(OnEnter(Screen::Connecting), spawn_connecting)
            .add_systems(OnExit(Screen::Connecting), despawn::<ConnectingRoot>)
            .add_systems(
                Update,
                (refresh_labels, clear_name_error, menu_name_tag, upper_code, terms_fetch, terms_scroll)
                    .run_if(in_state(Screen::Menu)),
            );
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
struct BotsField;
#[derive(Component)]
struct WorldName;
#[derive(Component)]
struct WorldIdx;
#[derive(Component)]
struct MusicLabel;
#[derive(Component)]
struct AdultBox;
#[derive(Component)]
struct TermsState;
/// Mensagens de erro: do nome (`#nerr`), dos termos (`#terr`) e da barra de baixo (`#serr`).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum ErrLabel {
    Name,
    Terms,
    Start,
}
/// Partes que abrem e fecham (o `<details>` do original).
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Fold {
    Account,
    Server,
}
#[derive(Component)]
struct FoldArrow(Fold);

#[derive(Component)]
struct TermsModal;
#[derive(Component)]
struct TermsBody;
#[derive(Component)]
struct TermsHint;
#[derive(Component)]
struct TermsCheck;
#[derive(Component)]
struct TermsOk;

/// Escolhas da tela inicial.
#[derive(Resource, Debug, Clone)]
pub struct MenuChoice {
    pub world: World,
    pub bots: u32,
}

/// Onde a thread que baixa os termos deixa o resultado.
type FetchSlot = Arc<Mutex<Option<Result<String, String>>>>;
/// Documento dos termos já convertido em blocos (ou o erro).
type TermsDoc = Option<Result<Vec<(Block, String)>, String>>;

/// Estado da tela: erros, partes abertas e o documento dos termos.
#[derive(Resource, Default)]
struct MenuUi {
    name_err: String,
    terms_err: String,
    start_err: String,
    account_open: bool,
    server_open: bool,
    /// O jogador rolou o documento até o fim e marcou "Li todo o documento".
    terms_read: bool,
    terms_checked: bool,
    terms_loaded: bool,
    fetch: Option<FetchSlot>,
}

pub fn despawn<T: Component>(mut commands: Commands, q: Query<Entity, With<T>>) {
    for e in &q {
        commands.entity(e).despawn();
    }
}

/// Cores do `#start` em cada tema (`app.css`).
#[derive(Debug, Clone, Copy)]
struct Pal {
    top: Color,
    bottom: Color,
    fg: Color,
    st: Color,
    ac: Color,
    onac: Color,
    bd: Color,
    slot: Color,
    soft: Color,
    line: Color,
    err: Color,
    ok: Color,
    logo: [Color; 3],
}

fn hex(s: &str) -> Color {
    Srgba::hex(s).map_or(Color::WHITE, Color::from)
}

impl Pal {
    fn new(t: &Theme) -> Self {
        match t.kind {
            ThemeKind::Light => Self {
                top: hex("#fffaf0f7"),
                bottom: hex("#f1e7d8f2"),
                fg: hex("#2b2218"),
                st: hex("#7a4300"),
                ac: hex("#a85a00"),
                onac: hex("#ffffff"),
                bd: hex("#c9b8a0"),
                slot: hex("#0000000f"),
                soft: hex("#0000000b"),
                line: hex("#0000001f"),
                err: hex("#b3261e"),
                ok: hex("#156b38"),
                logo: [hex("#b35400"), hex("#c2185b"), hex("#0b6fa8")],
            },
            ThemeKind::HighContrast => Self {
                top: t.panel,
                bottom: t.panel,
                fg: t.fg,
                st: t.strong,
                ac: t.accent,
                onac: t.on_accent,
                bd: t.border,
                slot: t.panel,
                soft: Color::NONE,
                line: t.border,
                err: hex("#ffdcdc"),
                ok: hex("#7dff7d"),
                logo: [t.accent; 3],
            },
            ThemeKind::Dark => Self {
                top: hex("#2d2340f5"),
                bottom: hex("#17121fef"),
                fg: hex("#f5ede0"),
                st: hex("#ffcf8a"),
                ac: hex("#ffb23e"),
                onac: hex("#1b1306"),
                bd: hex("#5b4b73"),
                slot: hex("#00000040"),
                soft: hex("#ffffff0d"),
                line: hex("#ffffff22"),
                err: hex("#ff8a8a"),
                ok: hex("#7ae6a4"),
                logo: [hex("#ffb23e"), hex("#ff5ea8"), hex("#5bd6ff")],
            },
        }
    }

    /// Tema dos botões e campos do painel.
    fn theme(&self, t: &Theme) -> Theme {
        Theme {
            panel: Color::NONE,
            fg: self.fg,
            accent: self.ac,
            on_accent: self.onac,
            border: self.bd,
            strong: self.st,
            ..*t
        }
    }
}

/// Título de seção (`.st`): maiúsculas, espaçado, na cor `--st`.
fn sec_title(f: &UiFonts, p: &Pal, s: &str) -> impl Bundle {
    (Text::new(s.to_uppercase()), f.font(16.0, 800), TextColor(p.st))
}

fn sec(commands: &mut Commands) -> Entity {
    commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(8.0),
            margin: UiRect::vertical(Val::Px(8.0)),
            ..default()
        })
        .id()
}

fn small(f: &UiFonts, color: Color, s: &str) -> impl Bundle {
    (Text::new(s), f.font(14.0, 600), TextColor(color))
}

/// Botão do `#start` (cantos de 12 px, `padding: 10px 14px`).
fn sbutton(commands: &mut Commands, t: &Theme, f: &UiFonts, s: &str, primary: bool) -> Entity {
    let b = button(commands, t, f, s, primary);
    commands.entity(b).insert(Node {
        padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(12.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    });
    b
}

/// Campo grande (`input.big`).
#[allow(clippy::too_many_arguments)]
fn big_input(
    commands: &mut Commands,
    t: &Theme,
    f: &UiFonts,
    p: &Pal,
    initial: &str,
    ph: &str,
    max: usize,
    size: f32,
) -> Entity {
    commands
        .spawn(text_input_with(t, f, initial, Val::Percent(100.0), ph, Some(max)))
        .insert((
            Node {
                width: Val::Percent(100.0),
                padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                align_items: AlignItems::Center,
                ..default()
            },
            f.font(size, 500),
            BackgroundColor(p.slot),
        ))
        .id()
}

/// Linha que abre e fecha uma parte (`<details><summary>`).
fn fold_row(commands: &mut Commands, f: &UiFonts, p: &Pal, label: &str, which: Fold) -> Entity {
    commands
        .spawn((
            Button,
            TabIndex(0),
            Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(9.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                ..default()
            },
            BackgroundColor(p.soft),
            BorderColor::all(p.line),
            children![
                (Text::new("▶"), f.symbol(12.0, 700), TextColor(p.fg), FoldArrow(which), Pickable::IGNORE),
                (Text::new(label), f.font(16.0, 700), TextColor(p.fg), Pickable::IGNORE),
            ],
        ))
        .observe(move |_: On<Activate>, mut ui: ResMut<MenuUi>| match which {
            Fold::Account => ui.account_open = !ui.account_open,
            Fold::Server => ui.server_open = !ui.server_open,
        })
        .id()
}

/// Caixa de marcar (`label.chk`).
fn check_box(commands: &mut Commands, f: &UiFonts, p: &Pal, on: bool) -> Entity {
    commands
        .spawn((
            Node {
                width: Val::Px(22.0),
                height: Val::Px(22.0),
                flex_shrink: 0.0,
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(5.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(if on { p.ac } else { Color::NONE }),
            BorderColor::all(if on { p.ac } else { p.bd }),
            Pickable::IGNORE,
            children![(Text::new(if on { "✔" } else { "" }), f.font(13.0, 800), TextColor(p.onac), Pickable::IGNORE)],
        ))
        .id()
}

fn set_check(on: bool, p: &Pal, e: Entity, q: &mut CheckQ, texts: &mut Query<&mut Text>) {
    if let Ok((mut bg, mut bc, kids)) = q.get_mut(e) {
        bg.0 = if on { p.ac } else { Color::NONE };
        *bc = BorderColor::all(if on { p.ac } else { p.bd });
        for k in kids {
            if let Ok(mut t) = texts.get_mut(*k) {
                t.0 = if on { "✔".into() } else { String::new() };
            }
        }
    }
}

type CheckQ<'w, 's> = Query<'w, 's, (&'static mut BackgroundColor, &'static mut BorderColor, &'static Children)>;

#[allow(clippy::too_many_lines, clippy::too_many_arguments)]
fn spawn_menu(
    mut commands: Commands,
    t: Res<Theme>,
    f: Res<UiFonts>,
    a: Res<Args>,
    s: Res<Settings>,
    err: Res<ConnectError>,
    choice: Option<Res<MenuChoice>>,
    mut ui: ResMut<MenuUi>,
    mut load: MessageWriter<LoadWorld>,
) {
    let choice = choice.map(|c| c.clone()).unwrap_or(MenuChoice { world: a.world, bots: a.bots.min(20) });
    load.write(LoadWorld(choice.world));
    commands.insert_resource(choice.clone());
    let p = Pal::new(&t);
    let st = p.theme(&t);
    let server = a.server.clone().unwrap_or_else(|| "https://skyke3d.com.br".into());
    // erro da última tentativa de conexão (nome em uso, servidor fora do ar…) vai para a barra de baixo
    ui.start_err = err.0.as_ref().map(|e| format!("⚠ {e}")).unwrap_or_default();
    ui.terms_err.clear();
    ui.name_err.clear();
    ui.server_open = ui.server_open || a.server.is_some() || err.0.is_some();

    let root = commands
        .spawn((
            MenuRoot,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Percent(2.5),
                top: Val::Percent(2.5),
                bottom: Val::Percent(2.5),
                width: Val::Px(660.0),
                max_width: Val::Percent(95.0),
                flex_direction: FlexDirection::Column,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(24.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundGradient::from(LinearGradient::new(
                165f32.to_radians(),
                vec![ColorStop::auto(p.top), ColorStop::auto(p.bottom)],
            )),
            BorderColor::all(p.line),
            BoxShadow::new(Color::BLACK.with_alpha(0.7), Val::Px(0.0), Val::Px(16.0), Val::Px(0.0), Val::Px(50.0)),
            // clique no painel não chega ao mundo
            Pickable::default(),
        ))
        .id();
    let body = commands
        .spawn(Node {
            flex_grow: 1.0,
            min_height: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect { left: Val::Px(24.0), right: Val::Px(24.0), top: Val::Px(24.0), bottom: Val::Px(8.0) },
            overflow: Overflow::scroll_y(),
            ..default()
        })
        .id();
    commands.entity(root).add_child(body);

    // logo e música
    let brand = commands
        .spawn(Node {
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::FlexStart,
            column_gap: Val::Px(10.0),
            margin: UiRect::bottom(Val::Px(6.0)),
            ..default()
        })
        .id();
    let logo = commands.spawn((Text::new("✨ "), f.font(38.0, 900), TextColor(p.logo[0]))).id();
    let word = "Skyke3D";
    let n = word.chars().count() as f32 - 1.0;
    for (i, ch) in word.chars().enumerate() {
        let x = i as f32 / n;
        let c = if x < 0.5 { p.logo[0].mix(&p.logo[1], x * 2.0) } else { p.logo[1].mix(&p.logo[2], x * 2.0 - 1.0) };
        let span = commands.spawn((TextSpan::new(ch.to_string()), f.font(38.0, 900), TextColor(c))).id();
        commands.entity(logo).add_child(span);
    }
    let mus = sbutton(&mut commands, &st, &f, music_label(s.music), false);
    commands.entity(mus).insert(MusicLabel).observe(|_: On<Activate>, mut s: ResMut<Settings>| s.music = !s.music);
    commands.entity(brand).add_children(&[logo, mus]);
    commands.entity(body).add_child(brand);

    // 1 · seu nome
    let s1 = sec(&mut commands);
    let t1 = commands.spawn(sec_title(&f, &p, "1 · Seu nome")).id();
    let initial = if a.name_or_empty().is_empty() { s.name.clone() } else { a.name_or_empty() };
    let name = big_input(&mut commands, &st, &f, &p, &initial, "Digite seu nome", 16, 19.0);
    commands.entity(name).insert(NameField);
    let nerr =
        commands.spawn((small(&f, p.err, ""), ErrLabel::Name, Node { display: Display::None, ..default() })).id();
    let acc = fold_row(&mut commands, &f, &p, "Já jogo · Entrar na minha conta", Fold::Account);
    let acc_box = commands
        .spawn((
            Fold::Account,
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(4.0)),
                ..default()
            },
            children![(
                Text::new(
                    "Use seu código de recuperação ou uma chave de acesso já cadastrada para trazer avatar, amigos e \
                     progresso. No cliente nativo, por enquanto, entre com o código pelo navegador: a conta criada \
                     aqui gera um código que também abre lá."
                ),
                f.font(14.0, 500),
                TextColor(p.fg.with_alpha(0.85)),
            )],
        ))
        .id();
    commands.entity(s1).add_children(&[t1, name, nerr, acc, acc_box]);

    // 2 · estilo do cenário
    let s2 = sec(&mut commands);
    let t2 = commands.spawn(sec_title(&f, &p, "2 · Estilo do cenário")).id();
    let stp = commands
        .spawn((
            Node {
                display: Display::Grid,
                grid_template_columns: vec![
                    GridTrack::px(44.0),
                    GridTrack::minmax(MinTrackSizingFunction::Px(0.0), MaxTrackSizingFunction::Fraction(1.0)),
                    GridTrack::px(44.0),
                ],
                column_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(p.soft),
        ))
        .id();
    let prev = square(&mut commands, &st, &f, "◀");
    let next = square(&mut commands, &st, &f, "▶");
    for (b, d) in [(prev, -1), (next, 1)] {
        commands.entity(b).observe(
            move |_: On<Activate>, mut c: ResMut<MenuChoice>, mut load: MessageWriter<LoadWorld>| {
                c.world = step_world(c.world, d);
                load.write(LoadWorld(c.world));
            },
        );
    }
    let mid = commands
        .spawn((
            Node { flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() },
            children![
                (Text::new(""), f.font(19.0, 700), TextColor(p.fg), WorldName),
                (Text::new(""), f.font(14.0, 500), TextColor(p.fg.with_alpha(0.8)), WorldIdx),
            ],
        ))
        .id();
    commands.entity(stp).add_children(&[prev, mid, next]);
    commands.entity(s2).add_children(&[t2, stp]);

    // 3 · seu avatar
    let s3 = sec(&mut commands);
    let head = commands
        .spawn(Node { justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..default() })
        .id();
    let t3 = commands.spawn(sec_title(&f, &p, "3 · Seu avatar")).id();
    let rnd = sbutton(&mut commands, &st, &f, "🎲 Aleatório", false);
    commands.entity(rnd).observe(
        |_: On<Activate>, my: Option<ResMut<crate::editor::MyLook>>, kit: Option<Res<AvatarKit>>| {
            if let (Some(mut my), Some(kit)) = (my, kit) {
                crate::editor::random_look(&mut my, &kit);
            }
        },
    );
    commands.entity(head).add_children(&[t3, rnd]);
    let (tabs, rows) = crate::editor::editor_body(&mut commands, &Theme { panel: p.soft, ..st }, &f);
    commands.entity(s3).add_children(&[head, tabs, rows]);

    // 4 · termos
    let s4 = sec(&mut commands);
    let t4 = commands.spawn(sec_title(&f, &p, "4 · Termos")).id();
    let adult = commands
        .spawn((
            Button,
            TabIndex(0),
            Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                padding: UiRect::vertical(Val::Px(4.0)),
                ..default()
            },
        ))
        .id();
    let ab = check_box(&mut commands, &f, &p, s.adult);
    commands.entity(ab).insert(AdultBox);
    let at = commands
        .spawn((Text::new("Tenho "), f.font(16.0, 500), TextColor(p.fg), Pickable::IGNORE))
        .with_children(|t| {
            t.spawn((TextSpan::new("18 anos ou mais"), f.font(16.0, 800), TextColor(p.fg)));
            t.spawn((TextSpan::new("."), f.font(16.0, 500), TextColor(p.fg)));
        })
        .id();
    commands.entity(adult).add_children(&[ab, at]).observe(
        |_: On<Activate>, mut s: ResMut<Settings>, mut ui: ResMut<MenuUi>| {
            s.adult = !s.adult;
            ui.terms_err.clear();
        },
    );
    let acc_row = commands
        .spawn(Node {
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            column_gap: Val::Px(10.0),
            row_gap: Val::Px(6.0),
            ..default()
        })
        .id();
    let read = sbutton(&mut commands, &st, &f, "📄 Ler os Termos de Uso e a Política de Privacidade", false);
    commands.entity(read).observe(open_terms_obs);
    let state = commands
        .spawn((
            Button,
            TabIndex(0),
            Node::default(),
            children![(Text::new(""), f.font(14.0, 600), TextColor(p.fg), TermsState, Pickable::IGNORE)],
        ))
        .observe(open_terms_obs)
        .id();
    commands.entity(acc_row).add_children(&[read, state]);
    let terr =
        commands.spawn((small(&f, p.err, ""), ErrLabel::Terms, Node { display: Display::None, ..default() })).id();
    commands.entity(s4).add_children(&[t4, adult, acc_row, terr]);

    // servidor e modo sem conexão (só no nativo)
    let s5 = sec(&mut commands);
    let srv_row = fold_row(&mut commands, &f, &p, "Servidor e modo sem conexão", Fold::Server);
    let srv_box = commands
        .spawn((
            Fold::Server,
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(4.0), Val::Px(4.0)),
                ..default()
            },
        ))
        .id();
    let srv_l = commands.spawn(small(&f, p.fg.with_alpha(0.85), "Endereço do servidor")).id();
    let srv = big_input(&mut commands, &st, &f, &p, &server, "https://skyke3d.com.br", 200, 16.0);
    commands.entity(srv).insert(ServerField);
    let explore = sbutton(&mut commands, &st, &f, "🧭 Explorar sem conexão", false);
    commands.entity(explore).observe(|_: On<Activate>, mut s: ResMut<Session>, mut next: ResMut<NextState<Screen>>| {
        *s = Session::default();
        next.set(Screen::Playing);
    });
    let help = commands
        .spawn((
            Text::new("W A S D ou setas andam · E pula · F senta · botão direito arrastando gira a câmera · Enter abre o chat"),
            f.font(13.0, 500),
            TextColor(p.fg.with_alpha(0.75)),
        ))
        .id();
    commands.entity(srv_box).add_children(&[srv_l, srv, explore, help]);
    commands.entity(s5).add_children(&[srv_row, srv_box]);
    commands.entity(body).add_children(&[s1, s2, s3, s4, s5]);

    // barra de baixo (`.acts`, sempre visível)
    let acts = commands
        .spawn((
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect {
                    left: Val::Px(24.0),
                    right: Val::Px(24.0),
                    top: Val::Px(12.0),
                    bottom: Val::Px(20.0),
                },
                ..default()
            },
            BackgroundColor(p.bottom.with_alpha(0.96)),
        ))
        .id();
    let row = commands.spawn(Node { column_gap: Val::Px(8.0), align_items: AlignItems::Stretch, ..default() }).id();
    let go = sbutton(&mut commands, &st, &f, "✨ Criar sala", true);
    commands.entity(go).observe(connect_from_menu(false));
    let code = big_input(&mut commands, &st, &f, &p, a.join.as_deref().unwrap_or(""), "CÓDIGO", 12, 17.0);
    commands.entity(code).insert((
        CodeField,
        EditableTextFilter::new(|c: char| c.is_ascii_alphanumeric()),
        Node {
            flex_grow: 1.0,
            min_width: Val::Px(90.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(14.0)),
            align_items: AlignItems::Center,
            ..default()
        },
    ));
    let join = sbutton(&mut commands, &st, &f, "Entrar", true);
    commands.entity(join).observe(connect_from_menu(true));
    let bots = commands
        .spawn((
            Node {
                align_items: AlignItems::Center,
                column_gap: Val::Px(4.0),
                padding: UiRect::horizontal(Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(14.0)),
                ..default()
            },
            BackgroundColor(p.slot),
            BorderColor::all(p.line),
        ))
        .id();
    let bi = commands.spawn((Text::new("🤖"), f.font(16.0, 500), TextColor(p.fg))).id();
    let bn = commands
        .spawn(text_input_with(&st, &f, &choice.bots.to_string(), Val::Px(44.0), "", Some(2)))
        .insert((
            Node {
                width: Val::Px(40.0),
                padding: UiRect::axes(Val::Px(2.0), Val::Px(0.0)),
                align_self: AlignSelf::Center,
                ..default()
            },
            BorderColor::all(Color::NONE),
            FocusRing { idle: Color::NONE, focus: Color::NONE },
            BotsField,
            EditableTextFilter::new(|c: char| c.is_ascii_digit()),
        ))
        .id();
    commands.entity(bots).add_children(&[bi, bn]);
    commands.entity(row).add_children(&[go, code, join, bots]);
    let serr =
        commands.spawn((small(&f, p.err, ""), ErrLabel::Start, Node { display: Display::None, ..default() })).id();
    commands.entity(acts).add_children(&[row, serr]);
    commands.entity(root).add_child(acts);
}

fn square(commands: &mut Commands, t: &Theme, f: &UiFonts, s: &str) -> Entity {
    let b = button(commands, t, f, s, false);
    commands.entity(b).insert(Node {
        width: Val::Px(44.0),
        height: Val::Px(40.0),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(12.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    });
    b
}

fn music_label(on: bool) -> &'static str {
    if on { "🔊 Música: ligada" } else { "🔇 Música: desligada" }
}

fn step_world(w: World, d: i32) -> World {
    let i = World::ALL.iter().position(|x| *x == w).unwrap_or(0) as i32;
    World::ALL[(i + d).rem_euclid(World::ALL.len() as i32) as usize]
}

fn terms_ok(s: &Settings) -> bool {
    s.terms == TERMS_VER
}

/// Texto do estado dos termos (`termsView`).
fn terms_view(s: &Settings) -> String {
    if terms_ok(s) {
        format!(
            "✔ Aceitos (versão de {}/{}/{}). Clique para ler de novo.",
            &TERMS_VER[8..10],
            &TERMS_VER[5..7],
            &TERMS_VER[0..4]
        )
    } else {
        "Ainda não aceitos: leia até o fim e confirme.".into()
    }
}

/// Nome que o jogador digitou, limpo (`cleanName`): sem controles, espaços juntos, até 16 letras.
fn clean_name(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(16).collect()
}

#[allow(clippy::type_complexity)]
fn connect_from_menu(
    join: bool,
) -> impl FnMut(
    On<Activate>,
    Query<&EditableText, With<NameField>>,
    Query<&EditableText, With<ServerField>>,
    Query<&EditableText, With<CodeField>>,
    Query<&EditableText, With<BotsField>>,
    ResMut<MenuChoice>,
    ResMut<Settings>,
    ResMut<MenuUi>,
    Commands,
    MessageWriter<ConnectRequest>,
    ResMut<NextState<Screen>>,
) {
    move |_, name, server, code, bots, mut choice, mut s, mut ui, mut commands, mut req, mut next| {
        let name = clean_name(&name.single().map(input_value).unwrap_or_default());
        let server = server.single().map(input_value).unwrap_or_default().trim().to_string();
        let code = code.single().map(input_value).unwrap_or_default();
        choice.bots = bots.single().map(input_value).unwrap_or_default().parse::<u32>().unwrap_or(0).min(20);
        // mesma ordem do original: nome, termos, código
        if name.is_empty() || name.eq_ignore_ascii_case("visitante") {
            ui.name_err = "⚠ Escolha um nome para entrar.".into();
            ui.start_err = "Escolha um nome para entrar.".into();
            return;
        }
        if !s.adult || !terms_ok(&s) {
            ui.terms_err = if s.adult {
                "⚠ Para jogar, leia e aceite os Termos de Uso e a Política de Privacidade.".into()
            } else {
                "⚠ Para jogar, confirme que tem 18 anos ou mais e aceite os Termos de Uso e a Política de Privacidade."
                    .into()
            };
            ui.start_err = "Confira os termos para jogar.".into();
            if s.adult {
                open_terms(&mut commands, &mut ui);
            }
            return;
        }
        let join = if join {
            match sky_shared::normalize_room_code(&code) {
                Some(c) => Some(c),
                None => {
                    ui.start_err = "⚠ Digite o código da sala (10 letras e números).".into();
                    return;
                }
            }
        } else {
            None
        };
        ui.name_err.clear();
        ui.terms_err.clear();
        ui.start_err.clear();
        if s.name != name {
            s.name = name.clone();
        }
        req.write(ConnectRequest { server, name, join, world: choice.world, bots: choice.bots });
        next.set(Screen::Connecting);
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn refresh_labels(
    c: Res<MenuChoice>,
    s: Res<Settings>,
    ui: Res<MenuUi>,
    t: Res<Theme>,
    mut texts: ParamSet<(
        Query<&mut Text, With<WorldName>>,
        Query<&mut Text, With<WorldIdx>>,
        Query<&mut Text, With<TermsState>>,
        Query<(&mut Text, &ErrLabel, &mut Node)>,
        Query<(&mut Text, &FoldArrow)>,
        Query<&mut Text>,
    )>,
    mut colors: Query<&mut TextColor, With<TermsState>>,
    mus: Query<&Children, With<MusicLabel>>,
    adult: Query<Entity, With<AdultBox>>,
    mut folds: Query<(&Fold, &mut Node), Without<ErrLabel>>,
    mut checks: CheckQ,
    mut first: Local<bool>,
) {
    let all = !*first;
    *first = true;
    let p = Pal::new(&t);
    if all || c.is_changed() {
        let i = World::ALL.iter().position(|x| *x == c.world).unwrap_or(0);
        for mut x in &mut texts.p0() {
            x.0 = c.world.display_name().to_string();
        }
        for mut x in &mut texts.p1() {
            x.0 = format!("{} / {}", i + 1, World::ALL.len());
        }
    }
    if all || s.is_changed() {
        for mut x in &mut texts.p2() {
            x.0 = terms_view(&s);
        }
        for mut col in &mut colors {
            col.0 = if terms_ok(&s) { p.ok } else { p.fg.with_alpha(0.85) };
        }
        let labels: Vec<Entity> = mus.iter().flat_map(|k| k.iter()).collect();
        let mut q = texts.p5();
        for e in labels {
            if let Ok(mut x) = q.get_mut(e) {
                x.0 = music_label(s.music).into();
            }
        }
        for e in &adult {
            set_check(s.adult, &p, e, &mut checks, &mut texts.p5());
        }
    }
    if all || ui.is_changed() {
        for (mut x, which, mut n) in &mut texts.p3() {
            let m = match which {
                ErrLabel::Name => &ui.name_err,
                ErrLabel::Terms => &ui.terms_err,
                ErrLabel::Start => &ui.start_err,
            };
            if x.0 != *m {
                x.0.clone_from(m);
            }
            n.display = if m.is_empty() { Display::None } else { Display::Flex };
        }
        for (fold, mut n) in &mut folds {
            let open = match fold {
                Fold::Account => ui.account_open,
                Fold::Server => ui.server_open,
            };
            n.display = if open { Display::Flex } else { Display::None };
        }
        for (mut x, a) in &mut texts.p4() {
            let open = match a.0 {
                Fold::Account => ui.account_open,
                Fold::Server => ui.server_open,
            };
            x.0 = if open { "▼" } else { "▶" }.into();
        }
    }
}

/// O rótulo do meu avatar ao fundo mostra o nome digitado (ou "Visitante", como o `cfg.name` inicial do original).
fn menu_name_tag(
    mut commands: Commands,
    name: Query<&EditableText, With<NameField>>,
    mut me: Query<(Entity, Option<&mut crate::state::ActorName>), With<crate::player::Me>>,
) {
    let Ok(ed) = name.single() else { return };
    let n = clean_name(&input_value(ed));
    let want = if n.is_empty() { "Visitante".to_string() } else { n };
    for (e, cur) in &mut me {
        match cur {
            Some(mut c) => {
                if c.0 != want {
                    c.0.clone_from(&want);
                }
            }
            None => {
                commands.entity(e).insert(crate::state::ActorName(want.clone()));
            }
        }
    }
}

/// O código da sala aparece em maiúsculas (`text-transform: uppercase` no original).
fn upper_code(mut q: Query<&mut EditableText, (With<CodeField>, Changed<EditableText>)>) {
    for mut e in &mut q {
        let v = input_value(&e);
        let up = v.to_uppercase();
        if up != v {
            e.clear();
            e.queue_edit(bevy::text::TextEdit::Insert(up.into()));
        }
    }
}

/// Digitar o nome apaga o aviso do nome (`input` em `#name`).
fn clear_name_error(
    name: Query<&EditableText, (With<NameField>, Changed<EditableText>)>,
    mut ui: ResMut<MenuUi>,
    mut last: Local<String>,
) {
    for e in &name {
        let v = input_value(e);
        if v != *last {
            *last = v;
            if !ui.name_err.is_empty() {
                ui.name_err.clear();
                ui.start_err.clear();
            }
        }
    }
}

fn open_terms_obs(_: On<Activate>, mut commands: Commands, mut ui: ResMut<MenuUi>, open: Query<(), With<TermsModal>>) {
    if open.is_empty() {
        open_terms(&mut commands, &mut ui);
    }
}

/// Abre a janela dos termos (`openTerms`); o documento vem do servidor (`/termos`).
fn open_terms(commands: &mut Commands, ui: &mut MenuUi) {
    ui.terms_read = false;
    ui.terms_checked = false;
    commands.queue(|w: &mut bevy::ecs::world::World| {
        let server = {
            let mut q = w.query_filtered::<&EditableText, With<ServerField>>();
            q.iter(w).next().map(input_value).unwrap_or_default().trim().to_string()
        };
        let t = *w.resource::<Theme>();
        let ok = terms_ok(w.resource::<Settings>());
        let f = w.resource::<UiFonts>().clone();
        let mut ui = w.resource_mut::<MenuUi>();
        if !ui.terms_loaded && ui.fetch.is_none() {
            let slot = Arc::new(Mutex::new(None));
            ui.fetch = Some(slot.clone());
            std::thread::spawn(move || {
                let r = sky_networking::fetch_text_blocking(&server, "/termos").map_err(|e| e.to_string());
                if let Ok(mut s) = slot.lock() {
                    *s = Some(r);
                }
            });
        }
        spawn_terms(&mut w.commands(), &t, &f, ok);
    });
}

fn spawn_terms(commands: &mut Commands, t: &Theme, f: &UiFonts, accepted: bool) {
    let p = Pal::new(t);
    let st = p.theme(t);
    let root = commands
        .spawn((
            TermsModal,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::BLACK.with_alpha(0.55)),
            GlobalZIndex(50),
        ))
        .id();
    let bx = commands
        .spawn(panel(t))
        .insert(Node {
            width: Val::Px(720.0),
            max_width: Val::Percent(94.0),
            height: Val::Percent(86.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            padding: UiRect::all(Val::Px(18.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(18.0)),
            ..default()
        })
        .insert(BackgroundColor(p.bottom.with_alpha(1.0)))
        .id();
    let head = commands
        .spawn(Node { justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..default() })
        .id();
    let h = commands
        .spawn((Text::new("📄 Termos de Uso e Política de Privacidade"), f.font(19.0, 800), TextColor(p.st)))
        .id();
    let x = sbutton(commands, &st, f, "✖", false);
    commands.entity(x).observe(|_: On<Activate>, mut c: Commands, q: Query<Entity, With<TermsModal>>| {
        for e in &q {
            c.entity(e).despawn();
        }
    });
    commands.entity(head).add_children(&[h, x]);
    let body = commands
        .spawn((
            TermsBody,
            Node {
                flex_grow: 1.0,
                min_height: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(12.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(p.slot),
            children![(Text::new("Carregando…"), f.font(15.0, 500), TextColor(p.fg))],
        ))
        .id();
    let hint = commands
        .spawn((
            small(
                f,
                p.fg.with_alpha(0.85),
                if accepted {
                    "✔ Você chegou ao fim do documento."
                } else {
                    "Role até o fim do texto para liberar a confirmação."
                },
            ),
            TermsHint,
        ))
        .id();
    let chk = commands
        .spawn((
            Button,
            TabIndex(0),
            TermsCheck,
            Node { align_items: AlignItems::Center, column_gap: Val::Px(10.0), ..default() },
        ))
        .id();
    let cb = check_box(commands, f, &p, accepted);
    let ct = commands
        .spawn((Text::new("Li todo o documento e "), f.font(15.0, 500), TextColor(p.fg), Pickable::IGNORE))
        .with_children(|t| {
            t.spawn((
                TextSpan::new("aceito os Termos de Uso e a Política de Privacidade"),
                f.font(15.0, 800),
                TextColor(p.fg),
            ));
            t.spawn((TextSpan::new("."), f.font(15.0, 500), TextColor(p.fg)));
        })
        .id();
    commands.entity(chk).add_children(&[cb, ct]).observe(
        move |_: On<Activate>,
              mut ui: ResMut<MenuUi>,
              s: Res<Settings>,
              t: Res<Theme>,
              mut checks: CheckQ,
              mut texts: Query<&mut Text>| {
            if !(ui.terms_read || terms_ok(&s)) {
                return;
            }
            ui.terms_checked = !ui.terms_checked;
            set_check(ui.terms_checked, &Pal::new(&t), cb, &mut checks, &mut texts);
        },
    );
    let row =
        commands.spawn(Node { justify_content: JustifyContent::FlexEnd, column_gap: Val::Px(8.0), ..default() }).id();
    let later = sbutton(commands, &st, f, "Agora não", false);
    commands.entity(later).observe(|_: On<Activate>, mut c: Commands, q: Query<Entity, With<TermsModal>>| {
        for e in &q {
            c.entity(e).despawn();
        }
    });
    let ok = sbutton(commands, &st, f, "Aceitar e continuar", true);
    commands.entity(ok).insert(TermsOk).observe(
        |_: On<Activate>,
         mut c: Commands,
         q: Query<Entity, With<TermsModal>>,
         mut ui: ResMut<MenuUi>,
         mut s: ResMut<Settings>| {
            if !ui.terms_checked {
                return;
            }
            s.terms = TERMS_VER.into();
            ui.terms_err.clear();
            ui.start_err.clear();
            for e in &q {
                c.entity(e).despawn();
            }
        },
    );
    commands.entity(row).add_children(&[later, ok]);
    commands.entity(bx).add_children(&[head, body, hint, chk, row]);
    commands.entity(root).add_child(bx);
    if accepted {
        commands.queue(|w: &mut bevy::ecs::world::World| {
            let mut ui = w.resource_mut::<MenuUi>();
            ui.terms_checked = true;
            ui.terms_read = true;
        });
    }
}

/// O documento chegou do servidor: vira parágrafos na janela.
fn terms_fetch(
    mut commands: Commands,
    mut ui: ResMut<MenuUi>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    body: Query<(Entity, Option<&Children>), With<TermsBody>>,
    added: Query<(), Added<TermsBody>>,
    mut doc: Local<TermsDoc>,
) {
    if let Some(slot) = ui.fetch.clone() {
        let got = slot.lock().ok().and_then(|mut s| s.take());
        if let Some(r) = got {
            ui.fetch = None;
            *doc = Some(
                r.map(|h| html_blocks(&h))
                    .and_then(|b| if b.is_empty() { Err("documento vazio".into()) } else { Ok(b) }),
            );
            ui.terms_loaded = matches!(*doc, Some(Ok(_)));
        } else if added.is_empty() {
            return;
        }
    } else if added.is_empty() && !ui.is_changed() {
        return;
    }
    let Some(d) = doc.as_ref() else { return };
    if !added.is_empty() || ui.is_changed() {
        let p = Pal::new(&t);
        for (e, kids) in &body {
            if kids.is_some_and(|k| k.len() > 1) && added.is_empty() {
                continue;
            }
            commands.entity(e).despawn_related::<Children>();
            match d {
                Ok(blocks) => {
                    for (kind, text) in blocks {
                        let (size, weight, color) = match kind {
                            Block::H2 => (20.0, 800, p.st),
                            Block::H3 => (17.0, 800, p.st),
                            Block::P => (15.0, 500, p.fg),
                        };
                        let c = commands.spawn((Text::new(text.clone()), f.font(size, weight), TextColor(color))).id();
                        commands.entity(e).add_child(c);
                    }
                }
                Err(_) => {
                    let c = commands
                        .spawn((
                            Text::new("Não foi possível carregar o documento. Verifique a conexão e tente de novo."),
                            f.font(15.0, 600),
                            TextColor(p.err),
                        ))
                        .id();
                    commands.entity(e).add_child(c);
                    ui.bypass_change_detection().terms_loaded = false;
                }
            }
        }
    }
}

/// Chegou ao fim do texto: libera a confirmação (`termsSync`).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn terms_scroll(
    mut ui: ResMut<MenuUi>,
    body: Query<(&ComputedNode, &ScrollPosition, Option<&Children>), With<TermsBody>>,
    mut laid: Local<u8>,
    mut hint: Query<&mut Text, With<TermsHint>>,
    ok: Query<&Children, With<TermsOk>>,
    mut colors: Query<&mut TextColor>,
    mut bg: Query<&mut BackgroundColor, With<TermsOk>>,
    t: Res<Theme>,
) {
    let Ok((n, pos, kids)) = body.single() else {
        *laid = 0;
        return;
    };
    // o documento precisa estar montado e medido (dois quadros depois de chegar) antes de ver se chegou ao fim
    if ui.terms_loaded && kids.is_some_and(|k| k.len() > 1) {
        *laid = laid.saturating_add(1);
    } else {
        *laid = 0;
    }
    let s = n.inverse_scale_factor;
    let end = *laid > 2 && pos.y + n.size().y * s >= n.content_size().y * s - 12.0;
    if end && !ui.terms_read {
        ui.terms_read = true;
        for mut h in &mut hint {
            h.0 = "✔ Você chegou ao fim do documento.".into();
        }
    }
    // "Aceitar e continuar" fica apagado até marcar a caixa
    let p = Pal::new(&t);
    let on = ui.terms_checked;
    for mut b in &mut bg {
        let c = if on { p.ac } else { p.ac.with_alpha(0.35) };
        if b.0 != c {
            b.0 = c;
        }
    }
    for kids in &ok {
        for k in kids {
            if let Ok(mut c) = colors.get_mut(*k) {
                let want = if on { p.onac } else { p.onac.with_alpha(0.6) };
                if c.0 != want {
                    c.0 = want;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Block {
    H2,
    H3,
    P,
}

/// Converte o `#doc` da página de termos em blocos de texto: títulos, parágrafos, itens de lista e linhas de tabela.
fn html_blocks(html: &str) -> Vec<(Block, String)> {
    let start = html.find("id=\"doc\"").unwrap_or(0);
    let src = &html[start..];
    let src = &src[src.find('>').map_or(0, |i| i + 1)..];
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut kind = Block::P;
    let mut cells: Vec<String> = Vec::new();
    let mut in_cell = false;
    let mut rest = src;
    let flush = |out: &mut Vec<(Block, String)>, cur: &mut String, kind: Block| {
        let t = decode(cur).split_whitespace().collect::<Vec<_>>().join(" ");
        if !t.is_empty() {
            out.push((kind, t));
        }
        cur.clear();
    };
    while let Some(lt) = rest.find('<') {
        let text = &rest[..lt];
        if in_cell {
            if let Some(c) = cells.last_mut() {
                c.push_str(text);
            }
        } else {
            cur.push_str(text);
        }
        let Some(gt) = rest[lt..].find('>') else { break };
        let tag = rest[lt + 1..lt + gt].trim();
        rest = &rest[lt + gt + 1..];
        let close = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect::<String>()
            .to_lowercase();
        match (name.as_str(), close) {
            ("main" | "body", true) => break,
            ("script" | "style", false) => {
                if let Some(e) = rest.find(&format!("</{name}")) {
                    rest = &rest[e..];
                }
            }
            ("h1" | "h2", false) => {
                flush(&mut out, &mut cur, kind);
                kind = Block::H2;
            }
            ("h3" | "h4", false) => {
                flush(&mut out, &mut cur, kind);
                kind = Block::H3;
            }
            ("li", false) => {
                flush(&mut out, &mut cur, kind);
                kind = Block::P;
                cur.push_str("• ");
            }
            ("p" | "div" | "ul" | "ol" | "table", false) => {
                flush(&mut out, &mut cur, kind);
                kind = Block::P;
            }
            ("h1" | "h2" | "h3" | "h4" | "p" | "li" | "div", true) => {
                flush(&mut out, &mut cur, kind);
                kind = Block::P;
            }
            ("br", _) => cur.push(' '),
            ("tr", false) => {
                flush(&mut out, &mut cur, kind);
                cells.clear();
            }
            ("td" | "th", false) => {
                in_cell = true;
                cells.push(String::new());
            }
            ("td" | "th", true) => in_cell = false,
            ("tr", true) => {
                in_cell = false;
                let row: Vec<String> = cells
                    .drain(..)
                    .map(|c| decode(&c).split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|c| !c.is_empty())
                    .collect();
                if !row.is_empty() {
                    out.push((Block::P, format!("• {}", row.join(" — "))));
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut cur, kind);
    out
}

/// Entidades HTML mais comuns.
fn decode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(j) = tail.find(';').filter(|j| *j <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..j];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        if let Some(c) = ch {
            out.push(c);
            rest = &tail[j + 1..];
        } else {
            out.push('&');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
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

    #[test]
    fn names_are_cleaned_like_the_original() {
        assert_eq!(clean_name("  Ana   Clara \n"), "Ana Clara");
        assert_eq!(clean_name("abcdefghijklmnopqrst"), "abcdefghijklmnop");
    }

    #[test]
    fn terms_document_becomes_blocks() {
        let h = "<html><style>p{}</style><main id=\"doc\"><h2>Termos &amp; Política</h2><p>Olá <b>mundo</b>.</p>\
                 <ul><li>um</li></ul><table><tr><th>A</th><td>b&#233;</td></tr></table></main><p>fora</p>";
        let b = html_blocks(h);
        assert_eq!(
            b,
            vec![
                (Block::H2, "Termos & Política".into()),
                (Block::P, "Olá mundo.".into()),
                (Block::P, "• um".into()),
                (Block::P, "• A — bé".into()),
            ]
        );
    }
}
