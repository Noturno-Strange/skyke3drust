//! 🌱 Fazenda (`assets/app-farm.js`): canteiros, sementes e loja. O servidor decide tudo (`lib/farm.js`): aqui só se
//! escolhe a semente e o canteiro e se mostra o progresso. As barras andam no relógio do servidor (diferença medida
//! em cada resposta), uma vez por segundo com o painel aberto.

use crate::state::{Screen, SendRaw, ServerOther, Session};
use bevy::prelude::*;
use bevy::text::EditableText;
use serde::Deserialize;
use serde_json::{Value, json};
use sky_ui::{Activate, Theme, UiFonts, button, panel, text_input};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug)]
pub struct FarmPlugin;

impl Plugin for FarmPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Farm>()
            .add_systems(OnEnter(Screen::Playing), spawn_panel)
            .add_systems(OnExit(Screen::Playing), (crate::menu::despawn::<FarmPanel>, reset))
            .add_systems(
                Update,
                (boot, receive, search, rebuild, tick, timeout).chain().run_if(in_state(Screen::Playing)),
            );
    }
}

// ---------- catálogo ----------

/// Semente (`CROPS`).
#[derive(Debug, Clone, Deserialize)]
pub struct Crop {
    pub id: String,
    pub n: String,
    /// Categoria.
    pub c: String,
    pub ic: String,
    /// Tempo para crescer (s).
    pub t: u64,
    pub lv: u32,
    pub xp: u32,
    /// Colheita mínima e máxima.
    pub y: [u32; 2],
    /// Ingrediente que vai para a despensa.
    #[serde(default)]
    pub g: Option<String>,
}

/// Nível da loja (`FARM_LAND`, `FARM_PROP`, `FARM_HOUSE`).
#[derive(Debug, Clone, Deserialize)]
pub struct Tier {
    pub n: String,
    #[serde(default)]
    pub ic: String,
    /// Canteiros abertos (terreno).
    #[serde(default)]
    pub p: usize,
    #[serde(rename = "$")]
    pub price: u64,
    pub lv: u32,
    /// Bônus (produção ou experiência).
    #[serde(default)]
    pub b: f64,
}

#[derive(Resource, Debug, Clone, Default, Deserialize)]
pub struct FarmCatalog {
    #[serde(rename = "CROPS")]
    pub crops: Vec<Crop>,
    #[serde(rename = "FARM_W")]
    pub w: usize,
    #[serde(rename = "FARM_H")]
    pub h: usize,
    #[serde(rename = "FARM_ORDER")]
    pub order: Vec<usize>,
    #[serde(rename = "FARM_LAND")]
    pub land: Vec<Tier>,
    #[serde(rename = "FARM_PROP")]
    pub prop: Vec<Tier>,
    #[serde(rename = "FARM_HOUSE")]
    pub house: Vec<Tier>,
}

impl FarmCatalog {
    pub fn from_catalog_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }

    pub fn crop(&self, id: &str) -> Option<&Crop> {
        self.crops.iter().find(|c| c.id == id)
    }

    /// Canteiros abertos por um terreno (`farmOpen`).
    pub fn open_set(&self, land: usize) -> HashSet<usize> {
        let i = land.min(self.land.len().saturating_sub(1));
        let n = self.land.get(i).map_or(0, |t| t.p);
        self.order.iter().take(n).copied().collect()
    }

    /// Sementes que dão um ingrediente (`FARM_SOURCES`).
    pub fn sources(&self, ing: &str) -> Vec<&Crop> {
        self.crops.iter().filter(|c| c.g.as_deref() == Some(ing)).collect()
    }
}

// ---------- estado vindo do servidor ----------

#[derive(Debug, Clone, Copy, Deserialize, PartialEq)]
pub struct Plot {
    /// Plantada e pronta em (ms no relógio do servidor).
    pub a: f64,
    pub e: f64,
    /// Regada.
    #[serde(default)]
    pub w: u8,
}

#[derive(Debug, Clone, Default)]
pub struct FarmState {
    pub lv: u32,
    pub xp: u64,
    pub lo: u64,
    pub hi: u64,
    pub max: bool,
    pub rank: u32,
    pub coins: u64,
    pub land: usize,
    pub prop: usize,
    pub house: usize,
    pub plots: Vec<Option<(String, Plot)>>,
    pub done: BTreeMap<String, u64>,
    pub pantry: BTreeMap<String, u64>,
    pub harvests: u64,
    pub units: u64,
    pub badges: Vec<(String, String, bool)>,
}

fn u(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or(0)
}

fn counts(v: Option<&Value>) -> BTreeMap<String, u64> {
    v.and_then(Value::as_object)
        .map(|o| o.iter().filter_map(|(k, n)| n.as_u64().map(|n| (k.clone(), n))).collect())
        .unwrap_or_default()
}

impl FarmState {
    /// Lê o estado (`view` em `lib/farm.js`); campos estranhos viram zero, como dado não confiável.
    pub fn from_value(v: &Value) -> Option<Self> {
        let plots = v.get("plots")?.as_array()?;
        Some(Self {
            lv: u(v, "lv").max(1) as u32,
            xp: u(v, "xp"),
            lo: u(v, "lo"),
            hi: u(v, "hi"),
            max: v.get("max").and_then(Value::as_bool).unwrap_or(false),
            rank: u(v, "rank") as u32,
            coins: u(v, "coins"),
            land: u(v, "land") as usize,
            prop: u(v, "prop") as usize,
            house: u(v, "house") as usize,
            plots: plots
                .iter()
                .map(|p| {
                    let c = p.get("c")?.as_str()?.to_owned();
                    let pl: Plot = serde_json::from_value(p.clone()).ok()?;
                    Some((c, pl))
                })
                .collect(),
            done: counts(v.get("done")),
            pantry: counts(v.get("pantry")),
            harvests: v.get("rec").map_or(0, |r| u(r, "harvests")),
            units: v.get("rec").map_or(0, |r| u(r, "units")),
            badges: v
                .get("badges")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|b| {
                            let s = |k| b.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
                            (s("i"), s("n"), b.get("on").and_then(Value::as_bool).unwrap_or(false))
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

/// Fração crescida e se está pronta, no relógio do servidor.
pub fn growth(p: &Plot, now: f64) -> (f64, bool) {
    let f = if p.e <= p.a { 1.0 } else { ((now - p.a) / (p.e - p.a)).clamp(0.0, 1.0) };
    (f, now >= p.e)
}

/// Desenho do canteiro: semente, broto, crescendo ou o fruto pronto.
pub fn stage(crop_ic: &str, f: f64, ready: bool) -> &str {
    if ready {
        crop_ic
    } else if f < 0.34 {
        "🟤"
    } else if f < 0.67 {
        "🌱"
    } else {
        "🌿"
    }
}

/// Tempo curto: 45 s, 12 min, 1 h 20 min (`dur`).
pub fn dur(ms: f64) -> String {
    let s = (ms / 1000.0).ceil().max(0.0) as u64;
    if s < 60 {
        return format!("{s} s");
    }
    let m = s.div_ceil(60);
    if m < 60 {
        return format!("{m} min");
    }
    let (h, mm) = (m / 60, m % 60);
    if mm > 0 { format!("{h} h {mm} min") } else { format!("{h} h") }
}

/// Moedas em texto (1.234).
pub fn coin(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push('.');
        }
        out.push(ch);
    }
    format!("🪙 {out}")
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

/// Fazenda do jogador: estado, relógio do servidor e o que o painel mostra.
#[derive(Resource, Debug, Default)]
pub struct Farm {
    pub st: Option<FarmState>,
    /// Diferença entre o relógio do servidor e o daqui (ms).
    off: f64,
    tab: usize,
    seed: String,
    q: String,
    dig: bool,
    busy: Option<(String, f64)>,
    msg: String,
    closed: HashSet<String>,
    asked: bool,
    dirty: bool,
}

impl Farm {
    fn now(&self) -> f64 {
        now_ms() + self.off
    }
    fn lv(&self) -> u32 {
        self.st.as_ref().map_or(1, |s| s.lv)
    }
    /// Moedas da carteira (a culinária paga na mesma).
    pub fn coins(&self) -> u64 {
        self.st.as_ref().map_or(0, |s| s.coins)
    }
    pub fn set_coins(&mut self, n: u64) {
        if let Some(s) = self.st.as_mut() {
            s.coins = n;
            self.dirty = true;
        }
    }
    /// Despensa (ingredientes das colheitas e do armazém).
    pub fn pantry(&self) -> BTreeMap<String, u64> {
        self.st.as_ref().map(|s| s.pantry.clone()).unwrap_or_default()
    }
    /// Plantas já colhidas (só elas podem ser presenteadas).
    pub fn harvested(&self) -> HashSet<String> {
        self.st
            .as_ref()
            .map(|s| s.done.iter().filter(|(_, n)| **n > 0).map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }

    /// Despensa nova (resposta da culinária).
    pub fn set_pantry(&mut self, v: &Value) {
        if let Some(s) = self.st.as_mut() {
            s.pantry = counts(Some(v));
            self.dirty = true;
        }
    }
    /// Recebe um estado novo (de `farm`, `farmr` ou `supply`).
    pub fn set_state(&mut self, v: &Value) {
        if let Some(st) = FarmState::from_value(v) {
            if let Some(n) = v.get("now").and_then(Value::as_f64) {
                self.off = n - now_ms();
            }
            self.st = Some(st);
            self.dirty = true;
        }
    }
}

fn reset(mut f: ResMut<Farm>) {
    *f = Farm::default();
}

// ---------- interface ----------

#[derive(Component, Debug)]
pub struct FarmPanel;
#[derive(Component, Debug)]
struct FarmBody;
#[derive(Component, Debug)]
struct FarmHead;
#[derive(Component, Debug)]
struct FarmSearch;
#[derive(Component, Debug)]
struct FarmMsg;
/// Canteiro `i`: ícone, barra e tempo atualizados a cada segundo.
#[derive(Component, Debug)]
struct CellIcon(usize);
#[derive(Component, Debug)]
struct CellFill(usize);
#[derive(Component, Debug)]
struct CellTrack(usize);
#[derive(Component, Debug)]
struct CellTime(usize);

pub fn on_toggle(_: On<Activate>, mut q: Query<&mut Node, With<FarmPanel>>, mut f: ResMut<Farm>) {
    for mut n in &mut q {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
    }
    f.dirty = true;
}

/// Pede o estado ao entrar na sala (`farmBoot`): a culinária também usa a despensa e as moedas.
fn boot(s: Res<Session>, mut f: ResMut<Farm>, mut send: MessageWriter<SendRaw>) {
    if s.online && !f.asked {
        f.asked = true;
        send.write(SendRaw(json!({"t": "farmget"})));
    }
}

const WHY: [(&str, &str); 17] = [
    ("pid", "Entre com sua identidade para plantar."),
    ("canteiro", "Canteiro inválido."),
    ("semente", "Semente inválida."),
    ("nivel", "Ainda não tem o nível para isso."),
    ("fechado", "Esse canteiro ainda está fechado."),
    ("ocupado", "Esse canteiro já tem planta."),
    ("cheio", "Não há canteiro vazio."),
    ("vazio", "Não há planta aí."),
    ("pronta", "Já está pronta: colha!"),
    ("regada", "Já foi regada."),
    ("crescendo", "Ainda está crescendo."),
    ("full", "Não há vaga para novas fazendas agora."),
    ("calma", "Calma! Um canteiro de cada vez."),
    ("moedas", "Moedas insuficientes. Colha mais um pouco!"),
    ("maximo", "Já está no máximo."),
    ("loja", "Item inválido."),
    ("novos", "Muitas fazendas novas saíram desta rede hoje. Tente amanhã."),
];

fn why(m: &Value) -> String {
    let r = m.get("reason").and_then(Value::as_str).unwrap_or_default();
    if r == "descanso" {
        let w = m.get("wait").and_then(Value::as_u64).unwrap_or(60).div_ceil(60);
        return format!("⏸ Descanso de {w} min: muitas ações seguidas, no mesmo ritmo. Seu progresso está guardado.");
    }
    WHY.iter().find(|(k, _)| *k == r).map_or("Não foi possível agora.", |(_, t)| t).to_owned()
}

fn receive(
    mut msgs: MessageReader<ServerOther>,
    mut f: ResMut<Farm>,
    cat: Res<FarmCatalog>,
    mut log: ResMut<crate::state::ChatLog>,
) {
    for m in msgs.read() {
        match m.t.as_str() {
            "farm" => f.set_state(&m.raw),
            "farmr" => {
                let rid = m.raw.get("rid").and_then(Value::as_str).unwrap_or_default();
                if f.busy.as_ref().is_none_or(|(r, _)| r != rid) {
                    continue;
                }
                f.busy = None;
                f.dirty = true;
                if !m.raw.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    f.msg = why(&m.raw);
                    continue;
                }
                if let Some(st) = m.raw.get("state") {
                    f.set_state(st);
                }
                let n = u(&m.raw, "n");
                let k = m.raw.get("k").and_then(Value::as_str).unwrap_or_default();
                f.msg = match k {
                    "plant" => {
                        let s = cat.crop(&f.seed);
                        format!(
                            "{}{}",
                            if n > 1 { format!("{n} canteiros plantados") } else { "Plantado".into() },
                            s.map_or(".".into(), |s| format!(
                                ": {} {}, pronta em {}.",
                                s.ic,
                                s.n,
                                dur(s.t as f64 * 1000.0)
                            ))
                        )
                    }
                    "water" => format!(
                        "💧 {}: cresce mais rápido e rende +1.",
                        if n > 1 { format!("{n} plantas regadas") } else { "Regada".into() }
                    ),
                    "dig" => "⛏ Planta arrancada.".into(),
                    "buy" => format!("🎉 Comprado: {}!", m.raw.get("name").and_then(Value::as_str).unwrap_or_default()),
                    "renew" => format!(
                        "★ Safra {}: você voltou ao nível 1 e ganha +10% de experiência e +5% nas moedas a cada safra.",
                        u(&m.raw, "rank")
                    ),
                    _ => {
                        // colheita: o que veio, a experiência e o que subiu
                        let mut sum: Vec<(String, u64)> = Vec::new();
                        for g in m.raw.get("got").and_then(Value::as_array).into_iter().flatten() {
                            let c = g.get("c").and_then(Value::as_str).unwrap_or_default().to_owned();
                            let q = u(g, "n");
                            match sum.iter_mut().find(|(x, _)| *x == c) {
                                Some((_, t)) => *t += q,
                                None => sum.push((c, q)),
                            }
                        }
                        let got: Vec<String> = sum
                            .iter()
                            .filter_map(|(c, n)| cat.crop(c).map(|cr| format!("{} {n} {}", cr.ic, cr.n)))
                            .collect();
                        let mut s =
                            format!("🧺 {} · +{} XP · +{}", got.join(", "), u(&m.raw, "xp"), coin(u(&m.raw, "coins")));
                        if m.raw.get("lvUp").and_then(Value::as_bool).unwrap_or(false) {
                            s += &format!(" · ⬆ Nível {}", u(&m.raw, "lv"));
                        }
                        if u(&m.raw, "cut") > 0 {
                            s += &format!(" · ⚖ ganho −{}%", u(&m.raw, "cut"));
                        }
                        for b in m.raw.get("badges").and_then(Value::as_array).into_iter().flatten() {
                            let t = |k| b.get(k).and_then(Value::as_str).unwrap_or_default();
                            s += &format!(" · {} {}", t("i"), t("n"));
                        }
                        s
                    }
                };
            }
            "farmfx" => {
                if let Some(sf) = m.raw.get("sf").and_then(Value::as_u64) {
                    log.push("", format!("🌾 Um jogador começou a {sf}ª safra!"), true);
                }
            }
            _ => {}
        }
    }
}

/// Uma ação por vez (o servidor responde antes da próxima); 5 s sem resposta liberam de novo.
fn act(f: &mut Farm, send: &mut MessageWriter<SendRaw>, s: &Session, mut m: Value) {
    if f.busy.is_some() {
        return;
    }
    if !s.online {
        f.msg = "Sem conexão: a fazenda fica no servidor. Entre numa sala para plantar.".into();
        f.dirty = true;
        return;
    }
    let rid = crate::state::rid();
    m["rid"] = Value::String(rid.clone());
    f.busy = Some((rid, now_ms()));
    send.write(SendRaw(m));
}

fn timeout(mut f: ResMut<Farm>) {
    if f.busy.as_ref().is_some_and(|(_, t)| now_ms() - t > 5000.0) {
        f.busy = None;
        f.msg = "Sem resposta do servidor. Tente de novo.".into();
        f.dirty = true;
    }
}

fn spawn_panel(mut commands: Commands, t: Res<Theme>, f: Res<UiFonts>, mut farm: ResMut<Farm>) {
    let root = commands
        .spawn(panel(&t))
        .insert((
            FarmPanel,
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
    let head = commands.spawn((Text::new("🌱 Fazenda"), f.font(15.0, 700), TextColor(t.strong), FarmHead)).id();
    let tabs = commands.spawn(Node { column_gap: Val::Px(6.0), align_items: AlignItems::Center, ..default() }).id();
    for (i, name) in ["🌱 Canteiros", "🌾 Sementes", "🏪 Loja"].iter().enumerate() {
        let b = button(&mut commands, &t, &f, name, false);
        commands.entity(b).observe(move |_: On<Activate>, mut farm: ResMut<Farm>| {
            farm.tab = i;
            farm.dirty = true;
        });
        commands.entity(tabs).add_child(b);
    }
    let q = commands.spawn((text_input(&t, &f, "", Val::Px(120.0)), FarmSearch)).id();
    commands.entity(tabs).add_child(q);
    let body = commands
        .spawn((FarmBody, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default() }))
        .id();
    let msg = commands
        .spawn((
            Text::new(""),
            f.font(14.0, 500),
            TextColor(t.fg),
            FarmMsg,
            Node { min_height: Val::Px(20.0), flex_shrink: 0.0, ..default() },
        ))
        .id();
    commands.entity(root).add_children(&[head, tabs, body, msg]);
    farm.dirty = true;
}

fn search(q: Query<&EditableText, (With<FarmSearch>, Changed<EditableText>)>, mut f: ResMut<Farm>) {
    for e in &q {
        let v = crate::backpack::norm(sky_ui::input_value(e).trim());
        if v != f.q {
            f.q = v;
            if f.tab == 1 {
                f.dirty = true;
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut farm: ResMut<Farm>,
    cat: Res<FarmCatalog>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    body: Query<(Entity, Option<&Children>), With<FarmBody>>,
    mut head: Query<&mut Text, (With<FarmHead>, Without<FarmMsg>)>,
    mut msg: Query<&mut Text, (With<FarmMsg>, Without<FarmHead>)>,
) {
    if !farm.dirty {
        return;
    }
    farm.dirty = false;
    let lv = farm.lv();
    for mut h in &mut head {
        h.0 = match &farm.st {
            Some(s) => {
                let xp = if s.max {
                    format!("{} XP", s.xp)
                } else {
                    format!("{} / {} XP", s.xp - s.lo, s.hi.saturating_sub(s.lo))
                };
                let rank =
                    if s.rank > 0 { format!(" {}", "⭐".repeat(s.rank.min(5) as usize)) } else { String::new() };
                format!(
                    "🌱 Fazenda · Nível {}{}{rank} · {xp} · {}",
                    s.lv,
                    if s.max { " (máx.)" } else { "" },
                    coin(s.coins)
                )
            }
            None => "🌱 Fazenda".into(),
        };
    }
    for mut m in &mut msg {
        m.0.clone_from(&farm.msg);
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
    match farm.tab {
        0 => {
            let seed = cat.crop(&farm.seed);
            out.push(txt(
                &mut commands,
                seed.map_or("Escolha o que deseja plantar na aba Sementes.".into(), |s| {
                    format!("Em uso: {} {} · {}", s.ic, s.n, dur(s.t as f64 * 1000.0))
                }),
                14.0,
            ));
            let acts = commands.spawn(Node { column_gap: Val::Px(6.0), flex_wrap: FlexWrap::Wrap, ..default() }).id();
            let plant_all = button(&mut commands, &t, &f, "🌱 Plantar tudo", true);
            commands.entity(plant_all).observe(
                |_: On<Activate>, mut farm: ResMut<Farm>, mut send: MessageWriter<SendRaw>, s: Res<Session>| {
                    if farm.seed.is_empty() {
                        farm.msg = "Escolha uma semente primeiro.".into();
                        farm.tab = 1;
                        farm.dirty = true;
                        return;
                    }
                    let c = farm.seed.clone();
                    act(&mut farm, &mut send, &s, json!({"t": "farmdo", "k": "plant", "i": -1, "c": c}));
                },
            );
            let water_all = button(&mut commands, &t, &f, "💧 Regar tudo", true);
            commands.entity(water_all).observe(
                |_: On<Activate>, mut farm: ResMut<Farm>, mut send: MessageWriter<SendRaw>, s: Res<Session>| {
                    act(&mut farm, &mut send, &s, json!({"t": "farmdo", "k": "water", "i": -1}));
                },
            );
            let harvest_all = button(&mut commands, &t, &f, "🧺 Colher tudo", true);
            commands.entity(harvest_all).observe(
                |_: On<Activate>, mut farm: ResMut<Farm>, mut send: MessageWriter<SendRaw>, s: Res<Session>| {
                    act(&mut farm, &mut send, &s, json!({"t": "farmdo", "k": "harvest", "i": -1}));
                },
            );
            let dig = button(&mut commands, &t, &f, if farm.dig { "⛏ Arrancando" } else { "⛏" }, farm.dig);
            commands.entity(dig).observe(|_: On<Activate>, mut farm: ResMut<Farm>| {
                farm.dig = !farm.dig;
                farm.msg = if farm.dig {
                    "⛏ Modo arrancar: clique num canteiro para tirar a planta (sem colheita). Clique de novo em ⛏ para sair."
                        .into()
                } else {
                    String::new()
                };
                farm.dirty = true;
            });
            commands.entity(acts).add_children(&[plant_all, water_all, harvest_all, dig]);
            out.push(acts);
            // terreno aberto e mais uma linha e uma coluna fechadas (o que a próxima compra abre)
            let open = cat.open_set(farm.st.as_ref().map_or(0, |s| s.land));
            let (mut mr, mut mc) = (0, 0);
            for i in &open {
                mr = mr.max(i / cat.w.max(1));
                mc = mc.max(i % cat.w.max(1));
            }
            let (vr, vc) = ((mr + 1).min(cat.h.saturating_sub(1)), (mc + 1).min(cat.w.saturating_sub(1)));
            for r in 0..=vr {
                let row = commands.spawn(Node { column_gap: Val::Px(4.0), ..default() }).id();
                for c in 0..=vc {
                    let i = r * cat.w + c;
                    let locked = !open.contains(&i);
                    let b = commands
                        .spawn((bevy::ui_widgets::Button, BackgroundColor(t.panel), BorderColor::all(t.border)))
                        .id();
                    commands.entity(b).insert((Node {
                        width: Val::Px(56.0),
                        height: Val::Px(56.0),
                        row_gap: Val::Px(2.0),
                        padding: UiRect::all(Val::Px(2.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        flex_direction: FlexDirection::Column,
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },));
                    commands.entity(b).with_children(|p| {
                        p.spawn((
                            Text::new(if locked { "🔒" } else { "" }),
                            f.font(17.0, 500),
                            TextColor(t.fg),
                            CellIcon(i),
                        ));
                        if !locked {
                            p.spawn((
                                Node {
                                    width: Val::Px(40.0),
                                    height: Val::Px(4.0),
                                    flex_shrink: 0.0,
                                    border_radius: BorderRadius::all(Val::Px(2.0)),
                                    ..default()
                                },
                                BackgroundColor(t.border),
                                CellTrack(i),
                                Visibility::Hidden,
                            ))
                            .with_children(|b| {
                                b.spawn((
                                    Node {
                                        width: Val::Percent(0.0),
                                        height: Val::Percent(100.0),
                                        border_radius: BorderRadius::all(Val::Px(2.0)),
                                        ..default()
                                    },
                                    BackgroundColor(t.accent),
                                    CellFill(i),
                                ));
                            });
                            p.spawn((Text::new(""), f.font(10.0, 600), TextColor(t.fg), CellTime(i)));
                        }
                    });
                    if !locked {
                        commands.entity(b).observe(
                            move |_: On<Activate>,
                                  mut farm: ResMut<Farm>,
                                  cat: Res<FarmCatalog>,
                                  mut send: MessageWriter<SendRaw>,
                                  s: Res<Session>| {
                                let now = farm.now();
                                let p = farm.st.as_ref().and_then(|s| s.plots.get(i).cloned().flatten());
                                if farm.dig {
                                    match p {
                                        Some(_) => act(&mut farm, &mut send, &s, json!({"t":"farmdo","k":"dig","i":i})),
                                        None => farm.msg = "Esse canteiro já está vazio.".into(),
                                    }
                                    farm.dirty = true;
                                    return;
                                }
                                match p {
                                    None => {
                                        let Some(seed) = cat.crop(&farm.seed) else {
                                            farm.msg = "Escolha uma semente primeiro.".into();
                                            farm.tab = 1;
                                            farm.dirty = true;
                                            return;
                                        };
                                        if seed.lv > farm.lv() {
                                            farm.msg = format!("🔒 Essa semente libera no nível {}.", seed.lv);
                                            farm.dirty = true;
                                            return;
                                        }
                                        let c = seed.id.clone();
                                        act(&mut farm, &mut send, &s, json!({"t":"farmdo","k":"plant","i":i,"c":c}));
                                    }
                                    Some((c, pl)) => {
                                        let (_, ready) = growth(&pl, now);
                                        if ready {
                                            act(&mut farm, &mut send, &s, json!({"t":"farmdo","k":"harvest","i":i}));
                                        } else if pl.w == 0 {
                                            act(&mut farm, &mut send, &s, json!({"t":"farmdo","k":"water","i":i}));
                                        } else {
                                            let n = cat.crop(&c).map_or(String::new(), |c| format!("{} {}", c.ic, c.n));
                                            farm.msg = format!("{n} já foi regada. Falta {}.", dur(pl.e - now));
                                            farm.dirty = true;
                                        }
                                    }
                                }
                            },
                        );
                    }
                    commands.entity(row).add_child(b);
                }
                out.push(row);
            }
            if let Some(s) = &farm.st {
                let badges: String = s.badges.iter().filter(|b| b.2).map(|b| b.0.as_str()).collect();
                out.push(txt(
                    &mut commands,
                    format!(
                        "Colheitas: {} · Total colhido: {} · Canteiros: {} de {}{}",
                        s.harvests,
                        s.units,
                        open.len(),
                        cat.w * cat.h,
                        if badges.is_empty() { String::new() } else { format!(" · {badges}") }
                    ),
                    13.0,
                ));
            }
        }
        1 => {
            let q = farm.q.clone();
            let mut order: Vec<String> = Vec::new();
            let mut by: BTreeMap<usize, Vec<&Crop>> = BTreeMap::new();
            for c in &cat.crops {
                if !q.is_empty() && !crate::backpack::norm(&c.n).contains(&q) {
                    continue;
                }
                let gi = order.iter().position(|g| *g == c.c).unwrap_or_else(|| {
                    order.push(c.c.clone());
                    order.len() - 1
                });
                by.entry(gi).or_default().push(c);
            }
            for (gi, list) in by {
                let g = order[gi].clone();
                let closed = q.is_empty() && farm.closed.contains(&g);
                let hb = button(
                    &mut commands,
                    &t,
                    &f,
                    &format!("{} {g} ({})", if closed { "▸" } else { "▾" }, list.len()),
                    false,
                );
                let gname = g.clone();
                commands.entity(hb).observe(move |_: On<Activate>, mut farm: ResMut<Farm>| {
                    if !farm.closed.remove(&gname) {
                        farm.closed.insert(gname.clone());
                    }
                    farm.dirty = true;
                });
                out.push(hb);
                if closed {
                    continue;
                }
                for c in list {
                    let lock = c.lv > lv;
                    let n = farm.st.as_ref().and_then(|s| s.done.get(&c.id).copied()).unwrap_or(0);
                    let y = if c.y[1] > c.y[0] { format!("{}–{}", c.y[0], c.y[1]) } else { c.y[0].to_string() };
                    let tag = if lock {
                        format!("nível {}", c.lv)
                    } else if n > 0 {
                        format!("✔ ×{n}")
                    } else {
                        "nova".into()
                    };
                    let label = format!(
                        "{} {} · ⏱ {} · ✨ {} XP · 🧺 {y} · {tag}",
                        if lock { "🔒" } else { &c.ic },
                        c.n,
                        dur(c.t as f64 * 1000.0),
                        c.xp
                    );
                    let b = button(&mut commands, &t, &f, &label, farm.seed == c.id);
                    let id = c.id.clone();
                    commands.entity(b).observe(
                        move |_: On<Activate>, mut farm: ResMut<Farm>, cat: Res<FarmCatalog>| {
                            let Some(s) = cat.crop(&id) else { return };
                            if s.lv > farm.lv() {
                                farm.msg = format!("🔒 {} libera no nível {} de fazendeiro.", s.n, s.lv);
                            } else {
                                farm.seed = id.clone();
                                farm.msg =
                                    format!("{} {} escolhida. Clique num canteiro vazio para plantar.", s.ic, s.n);
                                farm.tab = 0;
                            }
                            farm.dirty = true;
                        },
                    );
                    out.push(b);
                }
            }
        }
        _ => {
            let s = farm.st.clone().unwrap_or_default();
            out.push(txt(
                &mut commands,
                format!(
                    "Uma única propriedade, que evolui com você. Saldo: {}. Colheitas abastecem a despensa e rendem moedas.",
                    coin(s.coins)
                ),
                14.0,
            ));
            for (k, title, list, cur) in [
                ("land", "🧱 Terreno", &cat.land, s.land),
                ("prop", "🌾 Produção da propriedade", &cat.prop, s.prop),
                ("house", "🏠 Casa", &cat.house, s.house),
            ] {
                let what = |t: &Tier| match k {
                    "land" => format!("{} canteiros", t.p),
                    "prop" if t.b > 0.0 => format!("+{}% na colheita", (t.b * 100.0).round()),
                    "prop" => "colheita normal".into(),
                    _ if t.b > 0.0 => format!("+{}% de experiência", (t.b * 100.0).round()),
                    _ => "experiência normal".into(),
                };
                let Some(tc) = list.get(cur) else { continue };
                out.push(
                    commands
                        .spawn((
                            Text::new(format!("{title}: {} {} · {}", tc.ic, tc.n, what(tc))),
                            f.font(14.0, 700),
                            TextColor(t.fg),
                        ))
                        .id(),
                );
                match list.get(cur + 1) {
                    None => out.push(txt(&mut commands, "No máximo. 🏆".into(), 13.0)),
                    Some(nx) => {
                        let lk = nx.lv > lv;
                        let poor = s.coins < nx.price;
                        let label =
                            if lk { format!("🔒 Nível {}", nx.lv) } else { format!("Melhorar {}", coin(nx.price)) };
                        let row = commands
                            .spawn(Node { column_gap: Val::Px(8.0), align_items: AlignItems::Center, ..default() })
                            .id();
                        let note = txt(
                            &mut commands,
                            format!(
                                "Próximo: {} {} · {}{}",
                                nx.ic,
                                nx.n,
                                what(nx),
                                if poor && !lk {
                                    format!(" · faltam {}", coin(nx.price - s.coins))
                                } else {
                                    String::new()
                                }
                            ),
                            13.0,
                        );
                        let b = button(&mut commands, &t, &f, &label, !lk && !poor);
                        if !lk && !poor {
                            commands.entity(b).observe(
                                move |_: On<Activate>,
                                      mut farm: ResMut<Farm>,
                                      mut send: MessageWriter<SendRaw>,
                                      s: Res<Session>| {
                                    act(&mut farm, &mut send, &s, json!({"t": "farmdo", "k": "buy", "b": k}));
                                },
                            );
                        }
                        commands.entity(row).add_children(&[b, note]);
                        out.push(row);
                    }
                }
            }
            if s.max {
                let b = button(&mut commands, &t, &f, "🌾 Nova safra (volta ao nível 1 com +1 safra)", true);
                commands.entity(b).observe(
                    |_: On<Activate>, mut farm: ResMut<Farm>, mut send: MessageWriter<SendRaw>, s: Res<Session>| {
                        act(&mut farm, &mut send, &s, json!({"t": "farmdo", "k": "renew"}));
                    },
                );
                out.push(b);
            }
        }
    }
    commands.entity(root).add_children(&out);
}

/// Uma vez por segundo: ícone, barra e tempo de cada canteiro, no relógio do servidor.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn tick(
    time: Res<Time>,
    mut acc: Local<f32>,
    farm: Res<Farm>,
    cat: Res<FarmCatalog>,
    mut icons: Query<(&CellIcon, &mut Text), Without<CellTime>>,
    mut times: Query<(&CellTime, &mut Text), Without<CellIcon>>,
    mut fills: Query<(&CellFill, &mut Node)>,
    mut tracks: Query<(&CellTrack, &mut Visibility)>,
) {
    *acc += time.delta_secs();
    if *acc < 1.0 && !farm.is_changed() {
        return;
    }
    *acc = 0.0;
    let now = farm.now();
    let plot = |i: usize| farm.st.as_ref().and_then(|s| s.plots.get(i).cloned().flatten());
    for (c, mut t) in &mut icons {
        if t.0 == "🔒" {
            continue;
        }
        let want = plot(c.0).map_or(String::new(), |(id, p)| {
            let ic = cat.crop(&id).map_or("?", |c| c.ic.as_str());
            let (f, ready) = growth(&p, now);
            format!("{}{}", stage(ic, f, ready), if p.w > 0 && !ready { "💧" } else { "" })
        });
        if t.0 != want {
            t.0 = want;
        }
    }
    for (c, mut t) in &mut times {
        let want =
            plot(c.0).map_or(String::new(), |(_, p)| if growth(&p, now).1 { "pronta".into() } else { dur(p.e - now) });
        if t.0 != want {
            t.0 = want;
        }
    }
    for (c, mut v) in &mut tracks {
        let want = if plot(c.0).is_some() { Visibility::Inherited } else { Visibility::Hidden };
        if *v != want {
            *v = want;
        }
    }
    for (c, mut n) in &mut fills {
        let w = plot(c.0).map_or(0.0, |(_, p)| growth(&p, now).0 as f32 * 100.0);
        if n.width != Val::Percent(w) {
            n.width = Val::Percent(w);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_state_and_helpers() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogs/catalog.json");
        let cat = FarmCatalog::from_catalog_json(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!((cat.w, cat.h, cat.crops.len()), (8, 8, 100));
        assert_eq!(cat.open_set(0).len(), cat.land[0].p);
        assert!(!cat.sources("tomate").is_empty());
        let v = json!({"lv":3,"xp":120,"lo":100,"hi":200,"coins":1234,"land":0,"prop":0,"house":0,"now":1000.0,
            "plots":[{"c":"tomate","a":0.0,"e":2000.0,"w":1}, null], "done":{"tomate":2}, "pantry":{"tomate":5},
            "rec":{"harvests":4,"units":9}, "badges":[{"i":"🌱","n":"Primeira","on":true}]});
        let s = FarmState::from_value(&v).unwrap();
        assert_eq!((s.lv, s.coins, s.pantry["tomate"], s.harvests), (3, 1234, 5, 4));
        let (c, pl) = s.plots[0].clone().unwrap();
        assert_eq!(c, "tomate");
        assert_eq!(growth(&pl, 1000.0), (0.5, false));
        assert!(growth(&pl, 2000.0).1);
        assert!(s.plots[1].is_none());
        assert_eq!(dur(45_000.0), "45 s");
        assert_eq!(dur(80.0 * 60_000.0), "1 h 20 min");
        assert_eq!(coin(1234567), "🪙 1.234.567");
        assert_eq!(stage("🍅", 0.5, false), "🌱");
        assert_eq!(stage("🍅", 0.5, true), "🍅");
    }
}
