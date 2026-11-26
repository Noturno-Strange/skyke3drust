//! 🍳 Culinária (`assets/app-cook.js`): despensa, livro de receitas e preparo. O servidor decide tudo (`lib/cook.js`): a
//! receita sai da combinação (método + ingredientes) e só ele soma experiência e desbloqueios. Aqui se escolhem o método e
//! os ingredientes (até 6 espaços), perto da fogueira (5,5 m; o servidor aceita 6,5 m). Lavar e descascar é só local, como
//! no original. O prato pronto vai para a mão 4 s depois.

use crate::farm::{Farm, FarmCatalog, coin};
use crate::player::Me;
use crate::state::{Screen, SendRaw, ServerOther, Session};
use bevy::prelude::*;
use bevy::text::EditableText;
use serde::Deserialize;
use serde_json::{Value, json};
use sky_ui::{Activate, Theme, UiFonts, button, panel, text_input};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug)]
pub struct CookPlugin;

impl Plugin for CookPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Cook>()
            .add_systems(OnEnter(Screen::Playing), spawn_panel)
            .add_systems(OnExit(Screen::Playing), (crate::menu::despawn::<CookPanel>, reset))
            .add_systems(
                Update,
                (boot, receive, search, work, fire, rebuild).chain().run_if(in_state(Screen::Playing)),
            );
    }
}

/// Fogueira (centro da área de caminhada, `WALK`) e o alcance para cozinhar.
const FIRE: Vec2 = Vec2::new(0.0, 1.2);
const REACH: f32 = 5.5;
/// O prato pronto fica sobre a fogueira por 4 s e depois vai para a mão.
const DISH_S: f32 = 4.0;

// ---------- catálogo ----------

#[derive(Debug, Clone, Deserialize)]
pub struct Ing {
    pub id: String,
    pub n: String,
    pub c: String,
    pub ic: String,
    /// Preparo: `l` lavar, `d` descascar.
    #[serde(default)]
    pub p: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub n: String,
    /// Método (`cozinhar`, `assar`).
    pub m: String,
    pub g: Vec<String>,
    pub ic: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Method {
    pub n: String,
}

#[derive(Resource, Debug, Clone, Default, Deserialize)]
pub struct CookCatalog {
    #[serde(rename = "ING")]
    pub ing: Vec<Ing>,
    #[serde(rename = "RECIPES")]
    pub recipes: Vec<Recipe>,
    #[serde(rename = "COOK_METHODS")]
    pub methods: BTreeMap<String, Method>,
    #[serde(rename = "COOK_SLOTS")]
    pub slots: usize,
    #[serde(rename = "SUPPLY_PRICE")]
    pub supply_price: u64,
}

impl CookCatalog {
    pub fn from_catalog_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
    pub fn ing(&self, id: &str) -> Option<&Ing> {
        self.ing.iter().find(|i| i.id == id)
    }
    pub fn recipe(&self, id: &str) -> Option<&Recipe> {
        self.recipes.iter().find(|r| r.id == id)
    }
}

/// Lavar e descascar: ícone e nome.
fn step_name(c: char) -> (&'static str, &'static str) {
    if c == 'l' { ("💧", "Lavar") } else { ("🔪", "Descascar") }
}

// ---------- estado ----------

#[derive(Resource, Debug, Default)]
pub struct Cook {
    lv: u32,
    xp: u64,
    lo: u64,
    hi: u64,
    max: bool,
    rank: u32,
    done: BTreeMap<String, u64>,
    badges: Vec<(String, bool)>,
    has_state: bool,
    slots: Vec<String>,
    tab: usize,
    /// Preparo feito aqui (letras de `p` já feitas).
    prep: HashMap<String, String>,
    sel_recipe: String,
    sel_prep: String,
    /// Preparando: ingrediente, passo e quanto falta (0,65 s).
    working: Option<(String, char, f32)>,
    busy: Option<(String, f64)>,
    msg: String,
    q: String,
    closed: HashSet<String>,
    asked: bool,
    dirty: bool,
    near: Option<bool>,
    /// Prato pronto: vai para a mão quando o tempo acaba.
    dish: Option<(String, f32)>,
}

impl Cook {
    fn have(&self, id: &str) -> &str {
        self.prep.get(id).map_or("", String::as_str)
    }
    fn ready(&self, cat: &CookCatalog, id: &str) -> bool {
        cat.ing(id).is_none_or(|i| i.p.chars().all(|c| self.have(id).contains(c)))
    }
    fn missing(&self, cat: &CookCatalog, id: &str) -> String {
        cat.ing(id).map_or(String::new(), |i| {
            i.p.chars()
                .filter(|c| !self.have(id).contains(*c))
                .map(|c| step_name(c).1.to_lowercase())
                .collect::<Vec<_>>()
                .join(" e ")
        })
    }
    fn set_state(&mut self, v: &Value) {
        let u = |k| v.get(k).and_then(Value::as_u64).unwrap_or(0);
        self.lv = u("lv").max(1) as u32;
        self.xp = u("xp");
        self.lo = u("lo");
        self.hi = u("hi");
        self.max = v.get("max").and_then(Value::as_bool).unwrap_or(false);
        self.rank = u("rank") as u32;
        self.done = v
            .get("done")
            .and_then(Value::as_object)
            .map(|o| o.iter().filter_map(|(k, n)| n.as_u64().map(|n| (k.clone(), n))).collect())
            .unwrap_or_default();
        self.badges = v
            .get("badges")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|b| {
                        (
                            b.get("i").and_then(Value::as_str).unwrap_or_default().to_owned(),
                            b.get("on").and_then(Value::as_bool).unwrap_or(false),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.has_state = true;
        self.dirty = true;
    }
}

fn reset(mut c: ResMut<Cook>) {
    *c = Cook::default();
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

/// De onde vem um ingrediente que falta (`source`).
fn source(cat: &CookCatalog, farm: &FarmCatalog, id: &str) -> String {
    let c = farm.sources(id);
    if c.is_empty() {
        format!("Trocar por 🪙 {} no armazém (moedas das colheitas)", cat.supply_price)
    } else {
        format!("Plante {} na Fazenda", c.iter().map(|c| c.n.as_str()).collect::<Vec<_>>().join(" ou "))
    }
}

const WHY: [(&str, &str); 9] = [
    ("calma", "Aguarde um instante antes de repetir esta ação."),
    ("despensa", "Faltam ingredientes. Plante e colha na Fazenda; troque mantimentos no armazém da Despensa."),
    ("nada", "Essa combinação não deu em nada. Veja o Livro de receitas."),
    ("longe", "Chegue mais perto da fogueira."),
    ("ingredientes", "Combinação inválida."),
    ("metodo", "Método inválido."),
    ("full", "Não há vaga para novos cozinheiros agora."),
    ("pid", "Entre com sua identidade para cozinhar."),
    ("novos", "Muitas cozinhas novas saíram desta rede hoje. Tente amanhã."),
];

fn why(m: &Value) -> String {
    let r = m.get("reason").and_then(Value::as_str).unwrap_or_default();
    if r == "descanso" {
        let w = m.get("wait").and_then(Value::as_u64).unwrap_or(60).div_ceil(60);
        return format!("⏸ Descanso de {w} min: muitas ações seguidas, no mesmo ritmo. Seu progresso está guardado.");
    }
    WHY.iter().find(|(k, _)| *k == r).map_or("Não foi possível cozinhar agora.", |(_, t)| t).to_owned()
}

fn boot(s: Res<Session>, mut c: ResMut<Cook>, mut send: MessageWriter<SendRaw>) {
    if s.online && !c.asked {
        c.asked = true;
        send.write(SendRaw(json!({"t": "cookget"})));
    }
}

#[allow(clippy::too_many_arguments)]
fn receive(
    mut msgs: MessageReader<ServerOther>,
    mut cook: ResMut<Cook>,
    mut farm: ResMut<Farm>,
    cat: Res<CookCatalog>,
    mut cooked: ResMut<crate::backpack::CookedRecipes>,
    mut log: ResMut<crate::state::ChatLog>,
    snd: Res<crate::sound::Sound>,
) {
    for m in msgs.read() {
        let v = &m.raw;
        match m.t.as_str() {
            "cook" => {
                cook.set_state(v);
                if let Some(p) = v.get("pantry") {
                    farm.set_pantry(p);
                }
            }
            "cookr" => {
                let rid = v.get("rid").and_then(Value::as_str).unwrap_or_default();
                if cook.busy.as_ref().is_none_or(|(r, _)| r != rid) {
                    continue;
                }
                cook.busy = None;
                cook.dirty = true;
                if !v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    snd.play("erro", 0.9);
                    cook.msg = if v.get("renew").is_some() && v.get("reason").and_then(Value::as_str) == Some("nivel") {
                        "A estrela só vem no nível 999.".into()
                    } else {
                        why(v)
                    };
                    continue;
                }
                if let Some(st) = v.get("state") {
                    cook.set_state(st);
                }
                if v.get("renew").is_some() {
                    cook.msg = format!(
                        "★ Estrela vermelha {}! Você voltou ao nível 1 e ganha +10% de experiência e +5% de moedas a cada estrela.",
                        v.get("rank").and_then(Value::as_u64).unwrap_or(0)
                    );
                    continue;
                }
                if let Some(p) = v.get("pantry") {
                    farm.set_pantry(p);
                }
                if let Some(w) = v.get("wallet").and_then(Value::as_u64) {
                    farm.set_coins(w);
                }
                let id = v.get("recipe").and_then(Value::as_str).unwrap_or_default().to_owned();
                let Some(r) = cat.recipe(&id) else { continue };
                let u = |k| v.get(k).and_then(Value::as_u64).unwrap_or(0);
                let mut s = format!("{} {} pronto! +{} XP", r.ic, r.n, u("xp"));
                if u("coins") > 0 {
                    s += &format!(" · +{}", coin(u("coins")));
                }
                if v.get("first").and_then(Value::as_bool).unwrap_or(false) {
                    s += " · novo na mochila!";
                }
                if v.get("lvUp").and_then(Value::as_bool).unwrap_or(false) {
                    s += &format!(" · ⬆ Nível {}", u("lv"));
                }
                if u("home") > 0 {
                    s += &format!(" · 🌱 +{}% de moedas da sua horta", u("home") * 10);
                }
                if u("cut") > 0 {
                    s += &format!(" · ⚖ ganho −{}%", u("cut"));
                }
                for b in v.get("badges").and_then(Value::as_array).into_iter().flatten() {
                    let t = |k| b.get(k).and_then(Value::as_str).unwrap_or_default();
                    s += &format!(" · {} {}", t("i"), t("n"));
                }
                cook.msg = s;
                snd.play("cook_done", 1.0);
                if v.get("lvUp").and_then(Value::as_bool).unwrap_or(false)
                    || v.get("badges").and_then(Value::as_array).is_some_and(|b| !b.is_empty())
                {
                    snd.play("join", 0.9);
                }
                cook.slots.clear();
                cook.dish = Some((id, DISH_S));
            }
            "supplyr" => {
                cook.dirty = true;
                if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    if let Some(st) = v.get("state") {
                        farm.set_state(st);
                    }
                    cook.msg = "✔ Mantimento adicionado à despensa. Clique nele para usar.".into();
                    snd.play("pickup", 0.8);
                } else {
                    cook.msg = if v.get("reason").and_then(Value::as_str) == Some("moedas") {
                        "Você precisa de moedas: plante e colha na Fazenda primeiro.".into()
                    } else {
                        "Não foi possível trocar esse ingrediente.".into()
                    };
                }
            }
            "cookfx" => {
                if let Some(st) = v.get("st").and_then(Value::as_u64) {
                    log.push("", format!("⭐ Um jogador ganhou a {st}ª estrela de cozinheiro!"), true);
                }
            }
            _ => {}
        }
    }
    // receitas já feitas liberam os pratos na mochila
    if cook.is_changed() {
        let want: HashSet<String> = cook.done.iter().filter(|(_, n)| **n > 0).map(|(k, _)| k.clone()).collect();
        if cooked.0 != want {
            cooked.0 = want;
        }
    }
}

/// Preparo (0,65 s) e o prato que vai para a mão.
fn work(time: Res<Time>, mut cook: ResMut<Cook>, mut send: MessageWriter<SendRaw>, s: Res<Session>) {
    let dt = time.delta_secs();
    if let Some((id, c, t)) = cook.working.as_mut() {
        *t -= dt;
        if *t <= 0.0 {
            let (id, c) = (id.clone(), *c);
            cook.working = None;
            cook.prep.entry(id).or_default().push(c);
            cook.dirty = true;
        }
    }
    if let Some((id, t)) = cook.dish.as_mut() {
        *t -= dt;
        if *t <= 0.0 {
            let id = id.clone();
            cook.dish = None;
            if s.online {
                send.write(SendRaw(json!({"t": "item", "item": id})));
            }
        }
    }
}

/// Fogueira ao alcance? (conferida duas vezes por segundo, como no original)
fn fire(time: Res<Time>, mut acc: Local<f32>, me: Query<&Transform, With<Me>>, mut cook: ResMut<Cook>) {
    *acc += time.delta_secs();
    if *acc < 0.5 {
        return;
    }
    *acc = 0.0;
    let Ok(t) = me.single() else { return };
    let near = Vec2::new(t.translation.x, t.translation.z).distance(FIRE) <= REACH;
    if cook.near != Some(near) {
        cook.near = Some(near);
        if cook.tab == 0 {
            cook.dirty = true;
        }
    }
}

fn dist_to_fire(me: &Query<&Transform, With<Me>>) -> f32 {
    me.single().map_or(99.0, |t| Vec2::new(t.translation.x, t.translation.z).distance(FIRE))
}

// ---------- interface ----------

#[derive(Component, Debug)]
pub struct CookPanel;
#[derive(Component, Debug)]
struct CookBody;
#[derive(Component, Debug)]
struct CookHead;
#[derive(Component, Debug)]
struct CookSearch;
#[derive(Component, Debug)]
struct CookMsg;

pub fn on_toggle(_: On<Activate>, mut q: Query<&mut Node, With<CookPanel>>, mut c: ResMut<Cook>) {
    for mut n in &mut q {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
    }
    c.dirty = true;
}

fn spawn_panel(mut commands: Commands, t: Res<Theme>, f: Res<UiFonts>, mut cook: ResMut<Cook>) {
    let root = commands
        .spawn(panel(&t))
        .insert((
            CookPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                width: Val::Px(520.0),
                max_height: Val::Percent(82.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(12.0)),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                overflow: Overflow::scroll_y(),
                display: Display::None,
                ..default()
            },
        ))
        .id();
    let head = commands.spawn((Text::new("🍳 Culinária"), f.font(15.0, 700), TextColor(t.strong), CookHead)).id();
    let tabs = commands.spawn(Node { column_gap: Val::Px(6.0), align_items: AlignItems::Center, ..default() }).id();
    for (i, name) in ["🧺 Despensa", "📖 Receitas", "🔪 Preparo"].iter().enumerate() {
        let b = button(&mut commands, &t, &f, name, false);
        commands.entity(b).observe(move |_: On<Activate>, mut c: ResMut<Cook>| {
            c.tab = i;
            c.dirty = true;
        });
        commands.entity(tabs).add_child(b);
    }
    let q = commands.spawn((text_input(&t, &f, "", Val::Px(120.0)), CookSearch)).id();
    commands.entity(tabs).add_child(q);
    let body = commands
        .spawn((CookBody, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), ..default() }))
        .id();
    let msg = commands
        .spawn((
            Text::new(""),
            f.font(14.0, 500),
            TextColor(t.fg),
            CookMsg,
            Node { min_height: Val::Px(20.0), flex_shrink: 0.0, ..default() },
        ))
        .id();
    commands.entity(root).add_children(&[head, tabs, msg, body]);
    cook.dirty = true;
}

fn search(q: Query<&EditableText, (With<CookSearch>, Changed<EditableText>)>, mut c: ResMut<Cook>) {
    for e in &q {
        let v = crate::backpack::norm(sky_ui::input_value(e).trim());
        if v != c.q {
            c.q = v;
            c.dirty = true;
        }
    }
}

/// Coloca um ingrediente num espaço (`addSlot`), ou troca por moedas no armazém se não houver na despensa.
fn add_slot(
    cook: &mut Cook,
    farm: &Farm,
    cat: &CookCatalog,
    fcat: &FarmCatalog,
    s: &Session,
    send: &mut MessageWriter<SendRaw>,
    id: &str,
) {
    let qty = farm.pantry().get(id).copied().unwrap_or(0);
    let name = cat.ing(id).map_or(id, |i| i.n.as_str());
    cook.dirty = true;
    if qty == 0 {
        if !fcat.sources(id).is_empty() {
            cook.msg = format!("{}. Depois colha para abastecer sua despensa.", source(cat, fcat, id));
            return;
        }
        if !s.online {
            cook.msg = "Sem conexão: a despensa fica no servidor.".into();
            return;
        }
        cook.msg = format!("Trocando 🪙 {} por {name}…", cat.supply_price);
        send.write(SendRaw(json!({"t": "supply", "i": id, "rid": crate::state::rid()})));
        return;
    }
    if cook.slots.iter().any(|x| x == id) {
        cook.msg = "Esse ingrediente já está num espaço.".into();
        return;
    }
    if cook.slots.len() >= cat.slots {
        cook.msg = format!("Os {} espaços já estão cheios.", cat.slots);
        return;
    }
    if !cook.ready(cat, id) {
        cook.sel_prep = id.to_owned();
        cook.tab = 2;
        cook.msg = format!("Falta {} {}. Prepare aqui e volte.", cook.missing(cat, id), name.to_lowercase());
        return;
    }
    cook.slots.push(id.to_owned());
    cook.msg.clear();
}

/// Cozinhar ou assar o que está nos espaços.
fn go(cook: &mut Cook, s: &Session, send: &mut MessageWriter<SendRaw>, near: bool, m: &str) {
    if cook.busy.is_some() || cook.slots.len() < 2 {
        return;
    }
    cook.dirty = true;
    if !near {
        cook.msg = "Chegue perto da fogueira para cozinhar.".into();
        return;
    }
    if !s.online {
        cook.msg = "Sem conexão: as receitas são conferidas pelo servidor.".into();
        return;
    }
    let rid = crate::state::rid();
    cook.busy = Some((rid.clone(), now_ms()));
    cook.msg = "Cozinhando…".into();
    send.write(SendRaw(json!({"t": "cookdo", "m": m, "g": cook.slots.clone(), "rid": rid})));
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut cook: ResMut<Cook>,
    farm: Res<Farm>,
    cat: Res<CookCatalog>,
    fcat: Res<FarmCatalog>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    me: Query<&Transform, With<Me>>,
    body: Query<(Entity, Option<&Children>), With<CookBody>>,
    mut head: Query<&mut Text, (With<CookHead>, Without<CookMsg>)>,
    mut msg: Query<&mut Text, (With<CookMsg>, Without<CookHead>)>,
) {
    if cook.busy.as_ref().is_some_and(|(_, t)| now_ms() - t > 5000.0) {
        cook.busy = None;
        cook.msg = "Sem resposta do servidor. Tente de novo.".into();
        cook.dirty = true;
    }
    if !cook.dirty && !farm.is_changed() {
        return;
    }
    cook.dirty = false;
    let pantry = farm.pantry();
    let qty = |id: &str| pantry.get(id).copied().unwrap_or(0);
    for mut h in &mut head {
        h.0 = if cook.has_state {
            let xp = if cook.max {
                format!("{} XP", cook.xp)
            } else {
                format!("{} / {} XP", cook.xp - cook.lo, cook.hi.saturating_sub(cook.lo))
            };
            let badges: String = cook.badges.iter().filter(|b| b.1).map(|b| b.0.as_str()).collect();
            format!(
                "🍳 Culinária · Nível {}{}{} · {xp} · {} {badges}",
                cook.lv,
                if cook.max { " (máx.)" } else { "" },
                if cook.rank > 0 { format!(" {}", "⭐".repeat(cook.rank.min(5) as usize)) } else { String::new() },
                coin(farm.coins())
            )
        } else {
            "🍳 Culinária".into()
        };
    }
    for mut m in &mut msg {
        m.0.clone_from(&cook.msg);
    }
    let Ok((root, kids)) = body.single() else { return };
    if let Some(kids) = kids {
        for c in kids {
            commands.entity(*c).despawn();
        }
    }
    let mut out = Vec::new();
    let txt = |commands: &mut Commands, s: String, size: f32| {
        commands.spawn((Text::new(s), f.font(size, 500), TextColor(t.fg))).id()
    };
    let q = cook.q.clone();
    // grupo recolhível com a lista dentro
    let group =
        |commands: &mut Commands, out: &mut Vec<Entity>, cook: &Cook, key: String, label: String, n: usize| -> bool {
            let closed = q.is_empty() && cook.closed.contains(&key);
            let b = button(commands, &t, &f, &format!("{} {label} ({n})", if closed { "▸" } else { "▾" }), false);
            commands.entity(b).observe(move |_: On<Activate>, mut c: ResMut<Cook>| {
                if !c.closed.remove(&key) {
                    c.closed.insert(key.clone());
                }
                c.dirty = true;
            });
            out.push(b);
            !closed
        };
    match cook.tab {
        0 => {
            // espaços de combinação, botões e a distância da fogueira
            let d = dist_to_fire(&me);
            out.push(txt(
                &mut commands,
                if d <= REACH {
                    "🔥 Fogueira ao alcance".into()
                } else {
                    format!("Chegue perto da fogueira para cozinhar (a {} m)", (d - REACH).ceil())
                },
                14.0,
            ));
            let row =
                commands.spawn(Node { column_gap: Val::Px(4.0), align_items: AlignItems::Center, ..default() }).id();
            for i in 0..cat.slots {
                let label = cook.slots.get(i).and_then(|id| cat.ing(id)).map_or((i + 1).to_string(), |g| g.ic.clone());
                let b = button(&mut commands, &t, &f, &label, false);
                commands.entity(b).insert(Node {
                    width: Val::Px(46.0),
                    height: Val::Px(42.0),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                });
                commands.entity(b).observe(move |_: On<Activate>, mut c: ResMut<Cook>| {
                    if i < c.slots.len() {
                        c.slots.remove(i);
                        c.dirty = true;
                    }
                });
                commands.entity(row).add_child(b);
            }
            out.push(row);
            let ok = cook.slots.len() >= 2 && cook.slots.iter().all(|id| qty(id) > 0) && cook.busy.is_none();
            let acts = commands.spawn(Node { column_gap: Val::Px(6.0), ..default() }).id();
            for (m, label) in [("cozinhar", "🍲 Cozinhar"), ("assar", "🔥 Assar")] {
                let b = button(&mut commands, &t, &f, label, ok);
                commands.entity(b).observe(
                    move |_: On<Activate>,
                          mut c: ResMut<Cook>,
                          s: Res<Session>,
                          mut send: MessageWriter<SendRaw>,
                          me: Query<&Transform, With<Me>>| {
                        let near = dist_to_fire(&me) <= REACH;
                        go(&mut c, &s, &mut send, near, m);
                    },
                );
                commands.entity(acts).add_child(b);
            }
            let clear = button(&mut commands, &t, &f, "Limpar", false);
            commands.entity(clear).observe(|_: On<Activate>, mut c: ResMut<Cook>| {
                c.slots.clear();
                c.msg.clear();
                c.dirty = true;
            });
            commands.entity(acts).add_child(clear);
            out.push(acts);
            // despensa por categoria
            let mut order: Vec<String> = Vec::new();
            let mut by: BTreeMap<usize, Vec<&Ing>> = BTreeMap::new();
            for i in &cat.ing {
                if !q.is_empty() && !crate::backpack::norm(&i.n).contains(&q) {
                    continue;
                }
                let gi = order.iter().position(|g| *g == i.c).unwrap_or_else(|| {
                    order.push(i.c.clone());
                    order.len() - 1
                });
                by.entry(gi).or_default().push(i);
            }
            for (gi, list) in by {
                if !group(&mut commands, &mut out, &cook, format!("d:{}", order[gi]), order[gi].clone(), list.len()) {
                    continue;
                }
                let grid = commands
                    .spawn(Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(4.0),
                        row_gap: Val::Px(4.0),
                        ..default()
                    })
                    .id();
                for i in list {
                    let n = qty(&i.id);
                    let on = cook.slots.contains(&i.id);
                    let b = button(&mut commands, &t, &f, &format!("{} {} ×{n}", i.ic, i.n), on);
                    let id = i.id.clone();
                    commands.entity(b).observe(
                        move |_: On<Activate>,
                              mut c: ResMut<Cook>,
                              farm: Res<Farm>,
                              cat: Res<CookCatalog>,
                              fcat: Res<FarmCatalog>,
                              s: Res<Session>,
                              mut send: MessageWriter<SendRaw>| {
                            add_slot(&mut c, &farm, &cat, &fcat, &s, &mut send, &id);
                        },
                    );
                    commands.entity(grid).add_child(b);
                }
                out.push(grid);
            }
        }
        1 => {
            // detalhe da receita escolhida
            if let Some(r) = cat.recipe(&cook.sel_recipe) {
                let n = cook.done.get(&r.id).copied().unwrap_or(0);
                let mname = cat.methods.get(&r.m).map_or(r.m.as_str(), |m| m.n.as_str());
                out.push(txt(
                    &mut commands,
                    format!(
                        "{} {} · {mname} · {}",
                        r.ic,
                        r.n,
                        if n > 0 {
                            format!("feita {n} {}", if n == 1 { "vez" } else { "vezes" })
                        } else {
                            "ainda não feita".into()
                        }
                    ),
                    15.0,
                ));
                let chips: Vec<String> =
                    r.g.iter()
                        .filter_map(|g| cat.ing(g).map(|i| (g, i)))
                        .map(|(g, i)| {
                            let mut s = format!("{} {} ×{}", i.ic, i.n, qty(g));
                            if qty(g) == 0 {
                                s += &format!(" · {}", source(&cat, &fcat, g));
                            }
                            if !cook.ready(&cat, g) {
                                s += &format!(" ({})", cook.missing(&cat, g));
                            }
                            s
                        })
                        .collect();
                out.push(txt(&mut commands, chips.join("\n"), 13.0));
                let id = r.id.clone();
                let b = button(&mut commands, &t, &f, "Usar nos espaços", true);
                commands.entity(b).observe(
                    move |_: On<Activate>,
                          mut c: ResMut<Cook>,
                          farm: Res<Farm>,
                          cat: Res<CookCatalog>,
                          fcat: Res<FarmCatalog>| {
                        let Some(r) = cat.recipe(&id) else { return };
                        let pantry = farm.pantry();
                        let q = |g: &str| pantry.get(g).copied().unwrap_or(0);
                        c.slots = r.g.iter().filter(|g| c.ready(&cat, g) && q(g) > 0).cloned().collect();
                        c.tab = 0;
                        let none: Vec<String> =
                            r.g.iter()
                                .filter(|g| q(g) == 0)
                                .map(|g| {
                                    format!(
                                        "{} ({})",
                                        cat.ing(g).map_or(g.as_str(), |i| i.n.as_str()),
                                        source(&cat, &fcat, g)
                                    )
                                })
                                .collect();
                        let unprepared: Vec<&str> =
                            r.g.iter()
                                .filter(|g| !c.ready(&cat, g))
                                .filter_map(|g| cat.ing(g).map(|i| i.n.as_str()))
                                .collect();
                        c.msg = if !none.is_empty() {
                            format!("Faltam na despensa: {}", none.join("; "))
                        } else if !unprepared.is_empty() {
                            format!("Faltam preparar: {}. Vá em Preparo.", unprepared.join(", "))
                        } else {
                            let m = cat.methods.get(&r.m).map_or("", |m| m.n.as_str());
                            format!("{}: tudo pronto. Clique em {m}.", r.n)
                        };
                        c.dirty = true;
                    },
                );
                out.push(b);
            } else {
                out.push(txt(&mut commands, "Clique numa receita para ver os ingredientes.".into(), 14.0));
            }
            for (mk, method) in &cat.methods {
                let list: Vec<&Recipe> = cat
                    .recipes
                    .iter()
                    .filter(|r| &r.m == mk)
                    .filter(|r| {
                        q.is_empty()
                            || crate::backpack::norm(&r.n).contains(&q)
                            || r.g.iter().any(|g| cat.ing(g).is_some_and(|i| crate::backpack::norm(&i.n).contains(&q)))
                    })
                    .collect();
                if list.is_empty()
                    || !group(&mut commands, &mut out, &cook, format!("r:{mk}"), method.n.clone(), list.len())
                {
                    continue;
                }
                for r in list {
                    let n = cook.done.get(&r.id).copied().unwrap_or(0);
                    let ings: String =
                        r.g.iter().filter_map(|g| cat.ing(g).map(|i| i.ic.as_str())).collect::<Vec<_>>().join(" ");
                    let tag = if n > 0 { format!("✔ ×{n}") } else { format!("{} ingr.", r.g.len()) };
                    let b = button(
                        &mut commands,
                        &t,
                        &f,
                        &format!("{} {} · {ings} · {tag}", r.ic, r.n),
                        cook.sel_recipe == r.id,
                    );
                    let id = r.id.clone();
                    commands.entity(b).observe(move |_: On<Activate>, mut c: ResMut<Cook>| {
                        c.sel_recipe = id.clone();
                        c.dirty = true;
                    });
                    out.push(b);
                }
            }
        }
        _ => {
            // preparo: lavar e descascar
            if let Some(i) = cat.ing(&cook.sel_prep) {
                let state = if cook.working.as_ref().is_some_and(|w| w.0 == i.id) {
                    " · preparando…".to_owned()
                } else if cook.ready(&cat, &i.id) {
                    " · pronto".to_owned()
                } else {
                    format!(" · falta {}", cook.missing(&cat, &i.id))
                };
                out.push(txt(&mut commands, format!("{} {}{state}", i.ic, i.n), 15.0));
                let row = commands.spawn(Node { column_gap: Val::Px(6.0), ..default() }).id();
                for c in i.p.chars() {
                    let (ic, n) = step_name(c);
                    let done = cook.have(&i.id).contains(c);
                    let b = button(&mut commands, &t, &f, &format!("{ic} {n}{}", if done { " ✔" } else { "" }), !done);
                    if !done {
                        let id = i.id.clone();
                        commands.entity(b).observe(
                            move |_: On<Activate>, mut ck: ResMut<Cook>, snd: Res<crate::sound::Sound>| {
                                if ck.working.is_none() {
                                    snd.play(if c == 'l' { "wash" } else { "peel" }, 0.9);
                                    ck.working = Some((id.clone(), c, 0.65));
                                    ck.dirty = true;
                                }
                            },
                        );
                    }
                    commands.entity(row).add_child(b);
                }
                out.push(row);
            } else {
                out.push(txt(
                    &mut commands,
                    "Clique num ingrediente para lavar ou descascar. Quem não aparece aqui já vem pronto.".into(),
                    14.0,
                ));
            }
            let grid = commands
                .spawn(Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(4.0), row_gap: Val::Px(4.0), ..default() })
                .id();
            for i in cat.ing.iter().filter(|i| !i.p.is_empty()) {
                if !q.is_empty() && !crate::backpack::norm(&i.n).contains(&q) {
                    continue;
                }
                let steps: String =
                    i.p.chars()
                        .map(|c| format!("{}{}", step_name(c).0, if cook.have(&i.id).contains(c) { "✔" } else { "" }))
                        .collect::<Vec<_>>()
                        .join(" ");
                let b = button(&mut commands, &t, &f, &format!("{} {} {steps}", i.ic, i.n), cook.sel_prep == i.id);
                let id = i.id.clone();
                commands.entity(b).observe(move |_: On<Activate>, mut c: ResMut<Cook>| {
                    c.sel_prep = id.clone();
                    c.dirty = true;
                });
                commands.entity(grid).add_child(b);
            }
            out.push(grid);
        }
    }
    commands.entity(root).add_children(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_and_prep() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogs/catalog.json");
        let b = std::fs::read(p).unwrap();
        let cat = CookCatalog::from_catalog_json(&b).unwrap();
        let fcat = FarmCatalog::from_catalog_json(&b).unwrap();
        assert_eq!((cat.ing.len(), cat.recipes.len(), cat.slots), (88, 182, 6));
        assert!(cat.recipes.iter().all(|r| cat.methods.contains_key(&r.m) && r.g.iter().all(|g| cat.ing(g).is_some())));
        let mut c = Cook::default();
        assert!(!c.ready(&cat, "cenoura"));
        assert_eq!(c.missing(&cat, "cenoura"), "lavar e descascar");
        c.prep.insert("cenoura".into(), "ld".into());
        assert!(c.ready(&cat, "cenoura"));
        assert!(source(&cat, &fcat, "tomate").starts_with("Plante"));
        assert!(source(&cat, &fcat, "sal").contains("armazém"));
    }
}
