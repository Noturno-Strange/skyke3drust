//! 🎒 Mochila (`#pMochila` no original): os itens do catálogo por categoria, com busca. Clicar pega o item (ele aparece
//! na mão, crescendo em 0,25 s); clicar de novo no item da mão, ou Espaço, usa (bebe ou come: o braço sobe por
//! 1,6 s e a comida some da mão); "Soltar" larga. Receitas ficam bloqueadas até serem preparadas na Culinária.

use crate::player::Me;
use crate::state::{Screen, SendMsg, Session};
use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;
use sky_protocol::ClientMsg;
use sky_ui::{Activate, Theme, UiFonts, button, panel, text_input};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug)]
pub struct BackpackPlugin;

impl Plugin for BackpackPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BackpackState>()
            .init_resource::<CookedRecipes>()
            .add_systems(OnEnter(Screen::Playing), spawn_panel)
            .add_systems(
                OnExit(Screen::Playing),
                (crate::menu::despawn::<BackpackPanel>, crate::menu::despawn::<HeldIcon>),
            )
            .add_systems(
                Update,
                (search, rebuild, use_key, tick_held, place_held_icons).run_if(in_state(Screen::Playing)),
            );
    }
}

/// Um item do catálogo (`ITEMS`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct Item {
    pub id: String,
    /// Nome.
    pub n: String,
    /// Categoria.
    pub c: String,
    /// Ícone (emoji).
    pub ic: String,
    #[serde(default)]
    pub info: String,
    #[serde(default)]
    pub food: u8,
    /// Dá o brilho ✨ ao usar.
    #[serde(default)]
    pub mg: u8,
    /// Receita: bloqueada até ser preparada.
    #[serde(default)]
    pub rc: u8,
}

/// Itens do catálogo, na ordem do original.
#[derive(Resource, Debug, Default, Clone)]
pub struct ItemCatalog {
    pub items: Vec<Item>,
}

impl ItemCatalog {
    pub fn from_catalog_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        #[derive(serde::Deserialize)]
        struct C {
            #[serde(rename = "ITEMS")]
            items: Vec<Item>,
        }
        let c: C = serde_json::from_slice(bytes)?;
        Ok(Self { items: c.items })
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
}

/// Ícone desenhado (os `ICONS` SVG do original, em `assets/icons/<NOME>.png`) em vez de emoji.
fn icon_image(ic: &str) -> Option<String> {
    (!ic.is_empty() && ic.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')).then(|| format!("icons/{ic}.png"))
}

/// Receitas já preparadas (a Culinária preenche; sem ela, as receitas ficam bloqueadas).
#[derive(Resource, Debug, Default)]
pub struct CookedRecipes(pub HashSet<String>);

/// Item na mão de um avatar.
#[derive(Component, Debug, Clone)]
pub struct Held {
    pub item: Item,
    /// Tempo desde que apareceu (cresce em 0,25 s).
    pub t: f32,
    /// Comida usada: some da mão quando chega a 0.
    pub eat: Option<f32>,
}

/// Usando o item (`a.drink`): o braço direito sobe por 1,6 s.
#[derive(Component, Debug, Clone, Copy)]
pub struct Drinking(pub f32);

#[derive(Component, Debug)]
pub struct BackpackPanel;
#[derive(Component, Debug)]
struct BackpackRows;
#[derive(Component, Debug)]
struct BackpackSearch;
#[derive(Component, Debug)]
struct ItemInfo;
/// Ícone do item na mão (texto ou imagem da interface na posição da mão): o avatar e o item mostrado.
#[derive(Component, Debug)]
struct HeldIcon(Entity, String);

#[derive(Resource, Debug, Default)]
pub struct BackpackState {
    query: String,
    closed: HashSet<String>,
    dirty: bool,
}

fn toggle(q: &mut Query<&mut Node, With<BackpackPanel>>, st: &mut BackpackState) {
    for mut n in q.iter_mut() {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
    }
    st.dirty = true;
}

/// Abre ou fecha a mochila (botão 🎒).
pub fn on_toggle(_: On<Activate>, mut q: Query<&mut Node, With<BackpackPanel>>, mut st: ResMut<BackpackState>) {
    toggle(&mut q, &mut st);
}

fn spawn_panel(mut commands: Commands, t: Res<Theme>, f: Res<UiFonts>, mut st: ResMut<BackpackState>) {
    let root = commands
        .spawn(panel(&t))
        .insert((
            BackpackPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                width: Val::Px(420.0),
                max_height: Val::Percent(78.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    let head = commands.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..default() }).id();
    let title = commands.spawn((Text::new("🎒 Mochila"), f.font(15.0, 700), TextColor(t.strong))).id();
    let q = commands.spawn((text_input(&t, &f, "", Val::Px(170.0)), BackpackSearch)).id();
    let drop = button(&mut commands, &t, &f, "Soltar", false);
    commands.entity(drop).observe(drop_item);
    commands.entity(head).add_children(&[title, q, drop]);
    let rows = commands
        .spawn((
            BackpackRows,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                overflow: Overflow::scroll_y(),
                min_height: Val::Px(0.0),
                flex_shrink: 1.0,
                ..default()
            },
            ScrollPosition::default(),
        ))
        .id();
    let info = commands
        .spawn((
            Text::new(" "),
            f.font(14.0, 500),
            TextColor(t.fg),
            ItemInfo,
            Node { min_height: Val::Px(20.0), ..default() },
        ))
        .id();
    commands.entity(root).add_children(&[head, rows, info]);
    st.dirty = true;
}

fn search(q: Query<&EditableText, (With<BackpackSearch>, Changed<EditableText>)>, mut st: ResMut<BackpackState>) {
    for e in &q {
        let v = sky_ui::input_value(e).trim().to_lowercase();
        if v != st.query {
            st.query = v;
            st.dirty = true;
        }
    }
}

/// Sem acento e em minúsculas, para a busca (`norm` no original).
pub fn norm(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ã' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'õ' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            c => c,
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut st: ResMut<BackpackState>,
    cat: Res<ItemCatalog>,
    cooked: Res<CookedRecipes>,
    me: Query<Option<&Held>, With<Me>>,
    held_changed: Query<(), (With<Me>, Changed<Held>)>,
    mut removed: RemovedComponents<Held>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    rows: Query<(Entity, Option<&Children>), With<BackpackRows>>,
    assets: Res<AssetServer>,
) {
    let held_now = me.single().ok().flatten().map(|h| h.item.id.clone());
    if !st.dirty && held_changed.is_empty() && removed.read().count() == 0 && !cooked.is_changed() {
        return;
    }
    st.dirty = false;
    let Ok((root, kids)) = rows.single() else { return };
    if let Some(kids) = kids {
        for c in kids {
            commands.entity(*c).despawn();
        }
    }
    let q = norm(&st.query);
    let mut groups: BTreeMap<usize, (String, Vec<&Item>)> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for it in &cat.items {
        if !q.is_empty() && !norm(&it.n).contains(&q) {
            continue;
        }
        let gi = order.iter().position(|g| *g == it.c).unwrap_or_else(|| {
            order.push(it.c.clone());
            order.len() - 1
        });
        groups.entry(gi).or_insert_with(|| (it.c.clone(), Vec::new())).1.push(it);
    }
    let locked = |it: &Item| it.rc == 1 && !cooked.0.contains(&it.id);
    let mut out = Vec::new();
    for (_, (g, list)) in groups {
        let closed = q.is_empty() && st.closed.contains(&g);
        let note = if g == "Receitas" {
            format!("({}/{} feitas)", list.iter().filter(|i| !locked(i)).count(), list.len())
        } else {
            format!("({})", list.len())
        };
        let hb = button(&mut commands, &t, &f, &format!("{} {g} {note}", if closed { "▸" } else { "▾" }), false);
        let gname = g.clone();
        commands.entity(hb).observe(move |_: On<Activate>, mut st: ResMut<BackpackState>| {
            if !st.closed.remove(&gname) {
                st.closed.insert(gname.clone());
            }
            st.dirty = true;
        });
        out.push(hb);
        if closed {
            continue;
        }
        let grid = commands
            .spawn(Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(4.0), row_gap: Val::Px(4.0), ..default() })
            .id();
        for it in list {
            let lk = locked(it);
            let img = icon_image(&it.ic);
            let label = match (lk, &img) {
                (true, Some(_)) => "🔒".to_owned(),
                (true, None) => format!("🔒{}", it.ic),
                (false, Some(_)) => String::new(),
                (false, None) => it.ic.clone(),
            };
            let b = button(&mut commands, &t, &f, &label, held_now.as_deref() == Some(it.id.as_str()));
            commands.entity(b).insert(Node {
                width: Val::Px(54.0),
                height: Val::Px(48.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            });
            if let Some(path) = &img {
                commands.entity(b).with_children(|c| {
                    c.spawn((
                        ImageNode::new(assets.load(path.clone())),
                        Node { width: Val::Px(34.0), height: Val::Px(34.0), ..default() },
                        Pickable::IGNORE,
                    ));
                });
            }
            let info = if lk {
                format!("{} — Bloqueado: prepare esta receita na Culinária 🍳", it.n)
            } else {
                format!("{} — {}", it.n, it.info)
            };
            let id = it.id.clone();
            let info2 = info.clone();
            commands
                .entity(b)
                .observe(move |_: On<bevy::picking::events::PointerOver>, mut q: Query<&mut Text, With<ItemInfo>>| {
                    for mut t in &mut q {
                        t.0.clone_from(&info2);
                    }
                })
                .observe(
                    move |_: On<Activate>,
                          mut q: Query<&mut Text, With<ItemInfo>>,
                          mut commands: Commands,
                          cat: Res<ItemCatalog>,
                          me: Query<MeHeld, With<Me>>,
                          s: Res<Session>,
                          mut send: MessageWriter<SendMsg>| {
                        if lk {
                            for mut t in &mut q {
                                t.0.clone_from(&info);
                            }
                            return;
                        }
                        let Ok((e, held, drinking)) = me.single() else { return };
                        if held.is_some_and(|h| h.item.id == id) {
                            use_item(&mut commands, e, held, drinking, &s, &mut send);
                        } else if let Some(it) = cat.get(&id) {
                            send.write(SendMsg(ClientMsg::Item { item: Some(id.clone()), use_: 0 }));
                            if !s.online {
                                commands.entity(e).insert(Held { item: it.clone(), t: 0.0, eat: None });
                            }
                        }
                    },
                );
            commands.entity(grid).add_child(b);
        }
        out.push(grid);
    }
    if out.is_empty() {
        out.push(commands.spawn((Text::new("Nenhum item encontrado."), f.font(15.0, 500), TextColor(t.fg))).id());
    }
    commands.entity(root).add_children(&out);
}

fn drop_item(
    _: On<Activate>,
    mut commands: Commands,
    me: Query<Entity, With<Me>>,
    s: Res<Session>,
    mut send: MessageWriter<SendMsg>,
) {
    send.write(SendMsg(ClientMsg::Item { item: None, use_: 0 }));
    if !s.online
        && let Ok(e) = me.single()
    {
        commands.entity(e).remove::<Held>();
    }
}

/// Usa o item da mão (`useItem`): vai ao servidor, que devolve para todos (sem conexão, aplica aqui).
fn use_item(
    commands: &mut Commands,
    e: Entity,
    held: Option<&Held>,
    drinking: Option<&Drinking>,
    s: &Session,
    send: &mut MessageWriter<SendMsg>,
) {
    let Some(h) = held else { return };
    if drinking.is_some_and(|d| d.0 > 0.0) {
        return;
    }
    send.write(SendMsg(ClientMsg::Item { item: Some(h.item.id.clone()), use_: 1 }));
    if !s.online {
        apply_item(commands, e, Some(h), Some(h.item.clone()), true);
    }
}

/// Mensagem `item` (do servidor, ou local sem conexão): pega, solta ou usa.
pub fn apply_item(commands: &mut Commands, e: Entity, cur: Option<&Held>, item: Option<Item>, use_: bool) {
    let mut ec = commands.entity(e);
    match (item, use_) {
        (None, _) => {
            ec.remove::<Held>();
        }
        (Some(it), false) => {
            ec.insert(Held { item: it, t: 0.0, eat: None });
        }
        (Some(it), true) => {
            let eat = (it.food == 1).then_some(1.5);
            let t = cur.map_or(0.0, |h| h.t);
            if it.mg == 1 {
                ec.insert(crate::state::Emote { ic: "✨".into(), left: 4.0 });
            }
            ec.insert((Held { item: it, t, eat }, Drinking(1.6)));
        }
    }
}

/// Espaço usa o item da mão (fora de campos de texto).
fn use_key(
    keys: Res<ButtonInput<KeyCode>>,
    focus: Res<InputFocus>,
    inputs: Query<(), With<EditableText>>,
    mut commands: Commands,
    me: Query<MeHeld, With<Me>>,
    s: Res<Session>,
    mut send: MessageWriter<SendMsg>,
) {
    if !keys.just_pressed(KeyCode::Space) || crate::hud::typing(&focus, &inputs) {
        return;
    }
    if let Ok((e, held, drinking)) = me.single() {
        use_item(&mut commands, e, held, drinking, &s, &mut send);
    }
}

fn tick_held(
    mut commands: Commands,
    time: Res<Time>,
    mut held: Query<(Entity, &mut Held)>,
    mut drinking: Query<(Entity, &mut Drinking)>,
) {
    let dt = time.delta_secs();
    for (e, mut h) in &mut held {
        h.t += dt;
        if let Some(t) = h.eat.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                commands.entity(e).remove::<Held>();
            }
        }
    }
    for (e, mut d) in &mut drinking {
        d.0 -= dt;
        if d.0 <= 0.0 {
            commands.entity(e).remove::<Drinking>();
        }
    }
}

/// Entidade, item na mão e uso em andamento do jogador local.
type MeHeld<'a> = (Entity, Option<&'a Held>, Option<&'a Drinking>);

/// Escala do item na mão nos primeiros 0,25 s (`setHeld`).
fn grow(t: f32) -> f32 {
    let k = (t / 0.25).min(1.0);
    let e = 1.0 + 1.70158 * (k - 1.0).powi(3) + 1.70158 * (k - 1.0).powi(2);
    e.max(0.05)
}

/// Ícone na mão: texto da interface no ponto `hold` do braço, com o tamanho de 0,6 m na tela.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn place_held_icons(
    mut commands: Commands,
    f: Res<UiFonts>,
    assets: Res<AssetServer>,
    cam: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    held: Query<(Entity, &Held)>,
    children: Query<&Children>,
    names: Query<(&Name, &GlobalTransform)>,
    mut icons: Query<(
        Entity,
        &HeldIcon,
        &mut Node,
        Option<&mut Text>,
        Option<&mut TextFont>,
        &ComputedNode,
        &mut Visibility,
    )>,
    ui: Res<UiScale>,
) {
    let Ok((camera, cam_t)) = cam.single() else { return };
    let mut have = HashSet::new();
    for (ie, icon, mut node, text, font, computed, mut vis) in &mut icons {
        // item trocado: o ícone é refeito (pode passar de emoji para imagem)
        let Ok((_, h)) =
            held.get(icon.0).map_err(|_| ()).and_then(|x| if x.1.item.id == icon.1 { Ok(x) } else { Err(()) })
        else {
            commands.entity(ie).despawn();
            continue;
        };
        have.insert(icon.0);
        let hold = children
            .iter_descendants(icon.0)
            .find_map(|c| names.get(c).ok().filter(|(n, _)| n.as_str() == "hold").map(|(_, g)| g.translation()));
        let p = hold.unwrap_or_default() + Vec3::Y * 0.12;
        let top = camera.world_to_viewport(cam_t, p + Vec3::Y * 0.3);
        match (hold, camera.world_to_viewport(cam_t, p), top) {
            (Some(_), Ok(v), Ok(vt)) => {
                // `Val::Px` é multiplicado pelo tamanho da interface; a posição na tela não
                let (v, vt) = (v / ui.0, vt / ui.0);
                let px = ((v.y - vt.y).abs() * 2.0 * grow(h.t)).clamp(4.0, 200.0);
                if let (Some(mut text), Some(mut font)) = (text, font) {
                    font.font_size = (px * 0.8).into();
                    if text.0 != h.item.ic {
                        text.0.clone_from(&h.item.ic);
                    }
                } else {
                    node.width = Val::Px(px);
                    node.height = Val::Px(px);
                }
                let size = computed.size() * computed.inverse_scale_factor();
                node.left = Val::Px(v.x - size.x / 2.0);
                node.top = Val::Px(v.y - size.y / 2.0);
                *vis = Visibility::Inherited;
            }
            _ => *vis = Visibility::Hidden,
        }
    }
    for (e, h) in &held {
        if !have.contains(&e) {
            let base = (
                HeldIcon(e, h.item.id.clone()),
                Node { position_type: PositionType::Absolute, ..default() },
                Pickable::IGNORE,
                Visibility::Hidden,
            );
            match icon_image(&h.item.ic) {
                Some(path) => commands.spawn((base, ImageNode::new(assets.load(path)))),
                None => commands.spawn((base, Text::new(h.item.ic.clone()), f.font(20.0, 500))),
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_items_and_grow_curve() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogs/catalog.json");
        let cat = ItemCatalog::from_catalog_json(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(cat.items.len(), 206);
        assert_eq!(cat.get("refri").map(|i| i.ic.as_str()), Some("🥤"));
        assert!(cat.items.iter().filter(|i| i.rc == 1).count() > 100);
        assert!((grow(0.25) - 1.0).abs() < 1e-6);
        // a fórmula do original usa 1,70158 nos dois termos: começa em 1, passa de 1,2 no meio e volta a 1
        assert!((grow(0.0) - 1.0).abs() < 1e-6);
        assert!((grow(0.125) - 1.212_697_5).abs() < 1e-5);
        assert_eq!(norm("Maçã Gratinada"), "maca gratinada");
    }
}
