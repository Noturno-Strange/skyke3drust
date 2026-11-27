//! Editor de visual (`mkEditor` em `assets/app-net.js`): abas 👤 Corpo, 💇 Cabelo, 👕 Roupas e ✨ Nome; estilos
//! andam com ◀ ▶ e cada cor abre a paleta inteira em grade logo abaixo da linha (uma por vez); "✨ Cores brilhantes"
//! liga ou desliga juntas as chaves de brilho de todas as partes (`BRK`).
//!
//! O visual do jogador ([`MyLook`]) fica salvo em `skyke3d/online/visual.json` (preferência local, como o
//! `localStorage` do original) e vai ao servidor 250 ms depois da última mudança (`sendCfg`).

use crate::look::{AvatarKit, AvatarLook};
use crate::menu::despawn;
use crate::player::Me;
use crate::state::{Screen, SendCfg};
use bevy::prelude::*;
use bevy::ui_widgets::Button;
use sky_avatars::{Cfg, Palette};
use sky_ui::{Activate, Theme, UiFonts, button, panel};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct EditorPlugin;

impl Plugin for EditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorState>()
            .add_systems(OnEnter(Screen::Playing), spawn_editor)
            .add_systems(OnExit(Screen::Playing), despawn::<EditorPanel>)
            .add_systems(
                Update,
                (rebuild_rows, apply_my_look)
                    .run_if(|s: Res<State<Screen>>| matches!(s.get(), Screen::Playing | Screen::Menu)),
            );
    }
}

/// Visual do jogador local.
#[derive(Resource, Debug, Clone)]
pub struct MyLook {
    pub cfg: Cfg,
    /// Quando o visual deve ir ao servidor (tempo real, s).
    send_at: Option<f64>,
}

impl MyLook {
    fn path() -> std::path::PathBuf {
        crate::state::data_dir().join("online/visual.json")
    }

    /// Visual salvo (validado como um `cfg` de fora) ou o do avatar de referência.
    pub fn load(kit: &AvatarKit) -> Self {
        let saved: Option<BTreeMap<String, u32>> =
            sky_persistence::load_json(&Self::path(), 16 * 1024).map_err(|e| warn!("visual salvo: {e}")).ok().flatten();
        let cfg = saved.and_then(|m| kit.palettes.clean(&m)).unwrap_or_else(|| kit.reference.clone());
        Self { cfg, send_at: None }
    }

    pub fn set(&mut self, k: &str, v: u32) {
        self.cfg.0.insert(k.to_owned(), v);
    }
}

/// Aba aberta, paleta aberta e se as linhas precisam ser refeitas.
#[derive(Resource, Debug, Default)]
struct EditorState {
    tab: usize,
    open: Option<&'static str>,
    dirty: bool,
}

/// Painel do editor (aberto e fechado pelo botão 👕 da barra).
#[derive(Component, Debug)]
pub struct EditorPanel;
/// Lista de linhas do editor (no painel 👕 do jogo ou na seção "3 · Seu avatar" da tela inicial), com o tema dela.
#[derive(Component, Debug)]
struct EditorRows(Theme);
#[derive(Component, Debug)]
struct EditorTab(usize);

/// Abas (`EG`).
const TABS: [&str; 4] = ["👤 Corpo", "💇 Cabelo", "👕 Roupas", "✨ Nome"];

/// Campo do visual (`EDS`): estilo (com nome e a lista de nomes do catálogo) ou cor (paleta e chave de brilho).
#[derive(Debug, Clone, Copy)]
enum Field {
    Step { k: &'static str, n: &'static str, list: &'static str },
    Color { k: &'static str, pal: Pal, gloss: Option<&'static str> },
}

#[derive(Debug, Clone, Copy)]
enum Pal {
    Avatar(Palette),
    /// Cores do nome (`NCOL`).
    Name,
}

impl Field {
    fn key(self) -> &'static str {
        match self {
            Field::Step { k, .. } | Field::Color { k, .. } => k,
        }
    }
}

const fn step(k: &'static str, n: &'static str, list: &'static str) -> Field {
    Field::Step { k, n, list }
}
const fn color(k: &'static str, p: Palette, g: &'static str) -> Field {
    Field::Color { k, pal: Pal::Avatar(p), gloss: Some(g) }
}

/// Linhas de cada aba (`EROWS`): cada estilo fica na mesma linha da sua cor.
const ROWS: [(usize, &str, &[Field]); 10] = [
    (0, "Rosto", &[step("fc", "Rosto", "FACES")]),
    (0, "Pele", &[color("skin", Palette::Skin, "tb")]),
    (0, "Olhos", &[color("eye", Palette::Eye, "eb")]),
    (1, "Cabelo", &[step("hs", "Cabelo", "HS"), color("hair", Palette::Hair, "rb")]),
    (1, "Chapéu", &[step("hat", "Chapéu", "HATS"), color("hc", Palette::Pal, "hb")]),
    (2, "Camisa", &[color("shirt", Palette::Pal, "sb")]),
    (2, "Calça", &[step("bt", "Calça", "BT"), color("pc", Palette::Pal, "pb")]),
    (2, "Calçado", &[step("shoes", "Calçado", "SHOES"), color("sc", Palette::Pal, "cb")]),
    (3, "Cor", &[Field::Color { k: "nc", pal: Pal::Name, gloss: None }]),
    (3, "Efeito", &[step("nf", "Efeito do nome", "NFX")]),
];

/// Chaves de brilho de cada parte (`BRK`).
const GLOSS_KEYS: [&str; 7] = ["tb", "eb", "rb", "hb", "sb", "pb", "cb"];

/// Nomes dos estilos e cores do nome, lidos do catálogo.
#[derive(Resource, Debug, Default, Clone)]
pub struct EditorCatalog {
    lists: BTreeMap<String, Vec<String>>,
}

impl EditorCatalog {
    pub fn from_catalog_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let v: BTreeMap<String, serde_json::Value> = serde_json::from_slice(bytes)?;
        let mut lists = BTreeMap::new();
        for k in ["FACES", "HS", "HATS", "BT", "SHOES", "NFX", "NCOL", "EMO"] {
            if let Some(a) = v.get(k).and_then(|x| x.as_array()) {
                lists.insert(k.to_owned(), a.iter().filter_map(|s| s.as_str().map(str::to_owned)).collect());
            }
        }
        Ok(Self { lists })
    }

    fn list(&self, k: &str) -> &[String] {
        self.lists.get(k).map_or(&[], Vec::as_slice)
    }

    /// Emojis da barra (`EMO`).
    pub fn emojis(&self) -> &[String] {
        self.list("EMO")
    }

    pub fn emoji(&self, i: u32) -> Option<&str> {
        self.emojis().get(i as usize).map(String::as_str)
    }

    /// Cor do nome (`NCOL`); a 0 é a do tema.
    pub fn name_color(&self, i: u32) -> Option<Color> {
        (i > 0).then(|| self.list("NCOL").get(i as usize).and_then(|h| Srgba::hex(h).ok()).map(Color::from))?
    }
}

fn hex_color(h: &str) -> Color {
    Srgba::hex(h).map_or(Color::BLACK, Color::from)
}

fn spawn_editor(
    mut commands: Commands,
    t: Res<Theme>,
    f: Res<UiFonts>,
    mut st: ResMut<EditorState>,
    kit: Option<Res<AvatarKit>>,
) {
    if kit.is_none() {
        return;
    }
    let root = commands
        .spawn(panel(&t))
        .insert((
            EditorPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                width: Val::Px(440.0),
                max_height: Val::Percent(85.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                overflow: Overflow::scroll_y(),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    let title = commands.spawn((Text::new("👕 Visual"), f.font(15.0, 700), TextColor(t.strong))).id();
    let (tabs, rows) = editor_body(&mut commands, &t, &f);
    commands.entity(root).add_children(&[title, tabs, rows]);
    st.dirty = true;
}

/// Abas e linhas do editor (`mkEditor`), para pôr num painel qualquer. As linhas são feitas por `rebuild_rows`.
pub fn editor_body(commands: &mut Commands, t: &Theme, f: &UiFonts) -> (Entity, Entity) {
    let tabs = commands
        .spawn(Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::flex(4, 1.0),
            column_gap: Val::Px(6.0),
            ..default()
        })
        .id();
    for (i, name) in TABS.iter().enumerate() {
        let b = button(commands, t, f, name, false);
        commands.entity(b).insert(EditorTab(i)).observe(move |_: On<Activate>, mut st: ResMut<EditorState>| {
            st.tab = i;
            st.open = None;
            st.dirty = true;
        });
        commands.entity(tabs).add_child(b);
    }
    let rows = commands
        .spawn((EditorRows(*t), Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default() }))
        .id();
    (tabs, rows)
}

/// Fundo suave das linhas (`--soft`).
fn soft(t: &Theme) -> Color {
    match t.kind {
        sky_ui::ThemeKind::Light => Color::srgba(0.0, 0.0, 0.0, 0.043),
        sky_ui::ThemeKind::HighContrast => Color::NONE,
        sky_ui::ThemeKind::Dark => Color::srgba(1.0, 1.0, 1.0, 0.05),
    }
}

/// Visual aleatório (`randomLook`): sem chapéu, nome e brilho como estavam.
pub fn random_look(my: &mut MyLook, kit: &AvatarKit) {
    let mut seed =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
    let mut ri = |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % u64::from(n.max(1))) as u32
    };
    for k in ["skin", "eye", "shirt", "hair", "hs", "hc", "bt", "pc", "sc", "shoes", "fc"] {
        let n = kit.palettes.cr.get(k).copied().unwrap_or(1);
        let v = ri(n);
        my.set(k, v);
    }
    my.set("hat", 0);
}

/// Abre ou fecha o painel (botão 👕).
pub fn toggle(q: &mut Query<&mut Node, With<EditorPanel>>) {
    for mut n in q.iter_mut() {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
    }
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn rebuild_rows(
    mut commands: Commands,
    mut st: ResMut<EditorState>,
    my: Option<Res<MyLook>>,
    kit: Option<Res<AvatarKit>>,
    cat: Res<EditorCatalog>,
    f: Res<UiFonts>,
    rows: Query<(Entity, &EditorRows, Option<&Children>)>,
    added: Query<(), Added<EditorRows>>,
    mut tabs: Query<(&EditorTab, &mut BackgroundColor, &mut BorderColor, &mut Node)>,
) {
    let (Some(my), Some(kit)) = (my, kit) else { return };
    if !st.dirty && !my.is_changed() && added.is_empty() {
        return;
    }
    st.dirty = false;
    for (root, rows, kids) in &rows {
        let t = &rows.0;
        if let Some(kids) = kids {
            for c in kids {
                commands.entity(*c).despawn();
            }
        }
        let cfg = &my.cfg;
        let mut out = Vec::new();
        for (tab, label, fields) in ROWS {
            if tab != st.tab {
                continue;
            }
            let row = commands
                .spawn((
                    Node {
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        column_gap: Val::Px(8.0),
                        min_height: Val::Px(46.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                        border_radius: BorderRadius::all(Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(soft(t)),
                ))
                .id();
            let l = commands.spawn((Text::new(label), f.font(16.0, 600), TextColor(t.fg))).id();
            let ctl =
                commands.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..default() }).id();
            commands.entity(row).add_children(&[l, ctl]);
            let mut grids = Vec::new();
            for field in fields.iter().copied() {
                let k = field.key();
                let n = kit.palettes.cr.get(k).copied().unwrap_or(1).max(1);
                let x = cfg.get(k).min(n - 1);
                match field {
                    Field::Step { n: name, list, .. } => {
                        let prev = arrow(&mut commands, t, &f, "◀");
                        let next = arrow(&mut commands, t, &f, "▶");
                        for (b, d) in [(prev, n - 1), (next, 1)] {
                            commands.entity(b).observe(move |_: On<Activate>, mut my: ResMut<MyLook>| {
                                let v = (my.cfg.get(k) + d) % n;
                                my.set(k, v);
                            });
                        }
                        let val =
                            cat.list(list).get(x as usize).cloned().unwrap_or_else(|| format!("{name} {}", x + 1));
                        let v = commands
                            .spawn((
                                Node { width: Val::Px(112.0), justify_content: JustifyContent::Center, ..default() },
                                children![(
                                    Text::new(val),
                                    f.font(15.0, 500),
                                    TextColor(t.fg),
                                    TextLayout::justify(Justify::Center)
                                )],
                            ))
                            .id();
                        commands.entity(ctl).add_children(&[prev, v, next]);
                    }
                    Field::Color { pal, gloss, .. } => {
                        let glossy = gloss.is_some_and(|g| cfg.flag(g));
                        let hexes: Vec<String> = match pal {
                            Pal::Avatar(p) => (0..n).map(|i| kit.palettes.color(p, i, glossy).to_owned()).collect(),
                            Pal::Name => cat.list("NCOL").to_vec(),
                        };
                        let cur = hexes.get(x as usize).map_or(Color::BLACK, |h| hex_color(h));
                        let open = st.open == Some(k);
                        let sw = commands
                            .spawn((
                                Button,
                                bevy::input_focus::tab_navigation::TabIndex(0),
                                Node {
                                    width: Val::Px(46.0),
                                    height: Val::Px(36.0),
                                    border: UiRect::all(Val::Px(1.0)),
                                    border_radius: BorderRadius::all(Val::Px(12.0)),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                                BorderColor::all(if open { t.accent } else { t.border }),
                                children![(
                                    Node {
                                        width: Val::Px(24.0),
                                        height: Val::Px(24.0),
                                        border: UiRect::all(Val::Px(2.0)),
                                        border_radius: BorderRadius::all(Val::Percent(50.0)),
                                        ..default()
                                    },
                                    BackgroundColor(cur),
                                    BorderColor::all(Color::WHITE.with_alpha(0.85)),
                                    Pickable::IGNORE,
                                )],
                            ))
                            .observe(move |_: On<Activate>, mut st: ResMut<EditorState>| {
                                st.open = if st.open == Some(k) { None } else { Some(k) };
                                st.dirty = true;
                            })
                            .id();
                        commands.entity(ctl).add_child(sw);
                        if open {
                            let grid = commands
                                .spawn((
                                    Node {
                                        display: Display::Grid,
                                        grid_template_columns: RepeatedGridTrack::px(10, 30.0),
                                        column_gap: Val::Px(6.0),
                                        row_gap: Val::Px(6.0),
                                        padding: UiRect::all(Val::Px(10.0)),
                                        justify_content: JustifyContent::Center,
                                        border_radius: BorderRadius::all(Val::Px(12.0)),
                                        ..default()
                                    },
                                    BackgroundColor(soft(t)),
                                ))
                                .id();
                            for (i, h) in hexes.iter().enumerate() {
                                let i = i as u32;
                                let b = commands
                                    .spawn((
                                        Button,
                                        Node {
                                            width: Val::Px(30.0),
                                            height: Val::Px(30.0),
                                            border: UiRect::all(Val::Px(if i == x { 3.0 } else { 1.0 })),
                                            border_radius: BorderRadius::all(Val::Percent(50.0)),
                                            ..default()
                                        },
                                        BackgroundColor(hex_color(h)),
                                        BorderColor::all(if i == x { t.fg } else { t.border }),
                                    ))
                                    .observe(move |_: On<Activate>, mut my: ResMut<MyLook>| my.set(k, i))
                                    .id();
                                commands.entity(grid).add_child(b);
                            }
                            grids.push(grid);
                        }
                    }
                }
            }
            out.push(row);
            out.extend(grids);
        }
        // cores brilhantes: uma caixa só para o avatar todo
        let on = GLOSS_KEYS.iter().any(|k| cfg.flag(k));
        let g = commands
            .spawn((
                Button,
                bevy::input_focus::tab_navigation::TabIndex(0),
                Node {
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(10.0),
                    min_height: Val::Px(46.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(5.0)),
                    border_radius: BorderRadius::all(Val::Px(12.0)),
                    ..default()
                },
                BackgroundColor(soft(t)),
                children![
                    (
                        Node {
                            width: Val::Px(20.0),
                            height: Val::Px(20.0),
                            border: UiRect::all(Val::Px(2.0)),
                            border_radius: BorderRadius::all(Val::Px(5.0)),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(if on { t.accent } else { Color::NONE }),
                        BorderColor::all(if on { t.accent } else { t.border }),
                        Pickable::IGNORE,
                        children![(
                            Text::new(if on { "✔" } else { "" }),
                            f.font(13.0, 800),
                            TextColor(t.on_accent),
                            Pickable::IGNORE
                        )],
                    ),
                    (Text::new("✨ Cores brilhantes"), f.font(16.0, 600), TextColor(t.fg), Pickable::IGNORE),
                ],
            ))
            .observe(move |_: On<Activate>, mut my: ResMut<MyLook>| {
                for k in GLOSS_KEYS {
                    my.set(k, u32::from(!on));
                }
            })
            .id();
        out.push(g);
        commands.entity(root).add_children(&out);
        // abas: a aberta ganha a borda de destaque
        for (tab, mut bg, mut bc, mut node) in &mut tabs {
            let sel = tab.0 == st.tab;
            bg.0 = if sel { soft(t) } else { Color::NONE };
            *bc = BorderColor::all(if sel { t.accent } else { t.border });
            node.border = UiRect::all(Val::Px(if sel { 2.0 } else { 1.0 }));
            node.padding = UiRect::axes(Val::Px(4.0), Val::Px(if sel { 9.0 } else { 10.0 }));
            node.justify_content = JustifyContent::Center;
        }
    }
}

/// Botão quadrado ◀ / ▶ (`.estp button`).
fn arrow(commands: &mut Commands, t: &Theme, f: &UiFonts, s: &str) -> Entity {
    let b = button(commands, t, f, s, false);
    commands.entity(b).insert(Node {
        width: Val::Px(36.0),
        height: Val::Px(36.0),
        border: UiRect::all(Val::Px(1.0)),
        border_radius: BorderRadius::all(Val::Px(10.0)),
        justify_content: JustifyContent::Center,
        align_items: AlignItems::Center,
        ..default()
    });
    b
}

/// Visual mudou: aplica no avatar local, grava e agenda o envio; 250 ms depois da última mudança, envia.
fn apply_my_look(
    time: Res<Time<Real>>,
    my: Option<ResMut<MyLook>>,
    mut me: Query<&mut AvatarLook, With<Me>>,
    mut send: MessageWriter<SendCfg>,
) {
    let Some(mut my) = my else { return };
    let now = time.elapsed_secs_f64();
    if my.is_changed() && !my.is_added() {
        for mut l in &mut me {
            if l.0 != my.cfg {
                l.0 = my.cfg.clone();
            }
        }
        if let Err(e) = sky_persistence::save_json(&MyLook::path(), &my.cfg.0, false) {
            warn!("visual: {e}");
        }
        my.bypass_change_detection().send_at = Some(now + 0.25);
    }
    if my.send_at.is_some_and(|t| now >= t) {
        let m = my.bypass_change_detection();
        m.send_at = None;
        send.write(SendCfg(m.cfg.0.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_lists_match_ranges() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogs/catalog.json");
        let b = std::fs::read(p).unwrap();
        let cat = EditorCatalog::from_catalog_json(&b).unwrap();
        let pal = sky_avatars::Palettes::from_catalog_json(&b).unwrap();
        // cada estilo do editor tem um nome por opção válida do cfg
        for (_, _, fields) in ROWS {
            for f in fields {
                if let Field::Step { k, list, .. } = f {
                    assert_eq!(cat.list(list).len() as u32, pal.cr[*k], "{k}");
                }
            }
        }
        assert_eq!(cat.list("NCOL").len() as u32, pal.cr["nc"]);
        assert!(cat.name_color(0).is_none() && cat.name_color(1).is_some());
        // as chaves de brilho do editor são as do catálogo (BRK)
        let v: serde_json::Value = serde_json::from_slice(&b).unwrap();
        let mut brk: Vec<&str> = v["BRK"].as_object().unwrap().values().filter_map(|x| x.as_str()).collect();
        brk.sort_unstable();
        let mut mine = GLOSS_KEYS.to_vec();
        mine.sort_unstable();
        assert_eq!(brk, mine);
    }
}
