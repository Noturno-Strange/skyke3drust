//! 🎁 Presentes (`assets/app-gift.js`): a caixa de presentes é uma coleção para mostrar (não vai para a mochila nem vira
//! moeda). Dá para enviar o que você mesmo colheu ou cozinhou a um amigo, inclusive offline, até o limite do dia. O
//! servidor (`lib/gifts.js`) confere tudo; o amigo é achado pelo código do perfil público (`pp`).

use crate::friends::Friends;
use crate::state::{ChatLog, Screen, SendRaw, ServerOther, Session};
use bevy::prelude::*;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sky_ui::{Activate, Theme, UiFonts, button, panel};

#[derive(Debug)]
pub struct GiftPlugin;

impl Plugin for GiftPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Gifts>()
            .add_systems(OnEnter(Screen::Playing), spawn_panel)
            .add_systems(OnExit(Screen::Playing), (crate::menu::despawn::<GiftPanel>, reset))
            .add_systems(Update, (boot, receive, rebuild).chain().run_if(in_state(Screen::Playing)));
    }
}

/// Código do perfil público de um amigo (`ppOf`): 20 primeiros dígitos hex do SHA-256 de `skyke3d-profile:<id>`.
pub fn pp_of(id: &str) -> String {
    Sha256::digest(format!("skyke3d-profile:{id}")).iter().take(10).map(|b| format!("{b:02x}")).collect()
}

/// Presente numa caixa.
#[derive(Debug, Clone, PartialEq)]
pub struct Gift {
    pub id: u64,
    /// `c` colheita, `r` receita.
    pub k: String,
    pub i: String,
    /// Quem enviou.
    pub n: String,
    pub at: f64,
}

fn gifts_of(v: &Value) -> Vec<Gift> {
    v.get("list")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(300)
                .map(|g| Gift {
                    id: g.get("id").and_then(Value::as_u64).unwrap_or(0),
                    k: g.get("k").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    i: g.get("i").and_then(Value::as_str).unwrap_or_default().to_owned(),
                    n: g.get("n").and_then(Value::as_str).unwrap_or_default().chars().take(16).collect(),
                    at: g.get("at").and_then(Value::as_f64).unwrap_or(0.0),
                })
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Resource, Debug, Default)]
pub struct Gifts {
    tab: usize,
    mine: Vec<Gift>,
    total: u64,
    max: u64,
    unseen: u64,
    left: Option<u64>,
    per_day: u64,
    /// Destinatário: código do perfil e nome; a caixa dele.
    to: Option<(String, String)>,
    other: Option<Option<Vec<Gift>>>,
    sel: Option<(String, String)>,
    confirm: bool,
    pick: Option<u64>,
    busy: Option<String>,
    msg: String,
    asked: bool,
    dirty: bool,
}

fn reset(mut g: ResMut<Gifts>) {
    *g = Gifts::default();
}

fn boot(s: Res<Session>, mut g: ResMut<Gifts>, mut send: MessageWriter<SendRaw>) {
    if s.online && !g.asked {
        g.asked = true;
        send.write(SendRaw(json!({"t": "giftget"})));
    }
}

const WHY: [(&str, &str); 10] = [
    ("pid", "Entre com sua identidade para enviar presentes."),
    ("self", "Não dá para presentear você mesmo."),
    ("item", "Item inválido."),
    ("nao_tem", "Você ainda não colheu ou preparou esse item."),
    ("dia", "Você já enviou os presentes de hoje. Tente de novo amanhã."),
    ("par", "Você já enviou um presente para essa pessoa hoje. Amanhã pode de novo."),
    ("cheia", "A caixa dessa pessoa está cheia."),
    ("full", "Não há espaço para novas caixas de presentes agora."),
    ("sem_perfil", "Essa pessoa não tem o perfil público ligado, então não recebe presentes."),
    ("calma", "Calma! Um presente de cada vez."),
];

/// Ícone e nome de um presente (colheita ou receita do catálogo).
fn item(
    farm: &crate::farm::FarmCatalog,
    cook: &crate::cook::CookCatalog,
    k: &str,
    i: &str,
) -> Option<(String, String)> {
    match k {
        "c" => farm.crop(i).map(|c| (c.ic.clone(), c.n.clone())),
        "r" => cook.recipe(i).map(|r| (r.ic.clone(), r.n.clone())),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn receive(
    mut msgs: MessageReader<ServerOther>,
    mut g: ResMut<Gifts>,
    mut log: ResMut<ChatLog>,
    mut send: MessageWriter<SendRaw>,
    farm: Res<crate::farm::FarmCatalog>,
    cook: Res<crate::cook::CookCatalog>,
    snd: Res<crate::sound::Sound>,
    mut show: MessageWriter<crate::show::LifeShow>,
) {
    for m in msgs.read() {
        let v = &m.raw;
        let u = |k| v.get(k).and_then(Value::as_u64);
        match m.t.as_str() {
            "gifts" => {
                g.mine = gifts_of(v);
                g.total = u("total").unwrap_or(0);
                g.max = u("max").unwrap_or(0);
                g.unseen = u("unseen").unwrap_or(0);
                if let Some(l) = u("left") {
                    g.left = Some(l);
                }
                g.per_day = u("perDay").unwrap_or(5);
                g.dirty = true;
            }
            "giftbox" => {
                let to = v.get("to").and_then(Value::as_str).unwrap_or_default();
                if g.to.as_ref().is_some_and(|(pp, _)| pp == to) {
                    let none = v.get("none").is_some_and(|x| x.as_bool().unwrap_or(x.as_u64() == Some(1)));
                    g.other = Some(if none { None } else { Some(gifts_of(v)) });
                    g.dirty = true;
                }
            }
            "giftr" => {
                let rid = v.get("rid").and_then(Value::as_str).unwrap_or_default();
                if g.busy.as_deref() != Some(rid) {
                    continue;
                }
                g.busy = None;
                if let Some(l) = u("left") {
                    g.left = Some(l);
                }
                g.dirty = true;
                if v.get("ok").and_then(Value::as_bool).unwrap_or(false) {
                    let gift = v.get("gift").cloned().unwrap_or_default();
                    let it = item(&farm, &cook, gift["k"].as_str().unwrap_or(""), gift["i"].as_str().unwrap_or(""));
                    let to = g.to.as_ref().map_or(String::new(), |(_, n)| format!(" para {n}"));
                    g.msg = format!(
                        "🎁 Presente enviado{}{to}!",
                        it.map_or(String::new(), |(ic, n)| format!(": {ic} {n}"))
                    );
                    snd.play("confirm", 0.9);
                    show.write(crate::show::LifeShow::new("🎁", crate::show::At::Me, 2.4, 2.5, 1.0));
                    g.sel = None;
                    if let Some((pp, _)) = &g.to {
                        send.write(SendRaw(json!({"t": "giftsee", "to": pp})));
                    }
                } else {
                    let r = v.get("reason").and_then(Value::as_str).unwrap_or_default();
                    g.msg = format!(
                        "⚠ {}",
                        WHY.iter().find(|(k, _)| *k == r).map_or("Não foi possível enviar agora.", |(_, t)| t)
                    );
                }
            }
            "giftin" => {
                let k = v.get("k").and_then(Value::as_str).unwrap_or_default();
                let i = v.get("i").and_then(Value::as_str).unwrap_or_default();
                let Some((ic, n)) = item(&farm, &cook, k, i) else { continue };
                let from: String = v.get("n").and_then(Value::as_str).unwrap_or_default().chars().take(16).collect();
                log.push("", format!("🎁 {from} enviou {ic} {n} para você. Veja em 🎁."), true);
                snd.play("notif", 0.9);
                g.unseen = u("unseen").unwrap_or(g.unseen + 1);
                send.write(SendRaw(json!({"t": "giftget"})));
                g.dirty = true;
            }
            _ => {}
        }
    }
}

#[derive(Component, Debug)]
pub struct GiftPanel;
#[derive(Component, Debug)]
struct GiftBody;

pub fn on_toggle(
    _: On<Activate>,
    mut q: Query<&mut Node, With<GiftPanel>>,
    mut g: ResMut<Gifts>,
    mut send: MessageWriter<SendRaw>,
    s: Res<Session>,
) {
    for mut n in &mut q {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
        if n.display == Display::Flex && s.online {
            send.write(SendRaw(json!({"t": "giftget", "seen": g.tab == 0})));
        }
    }
    g.dirty = true;
}

fn spawn_panel(mut commands: Commands, t: Res<Theme>, f: Res<UiFonts>, mut g: ResMut<Gifts>) {
    let root = commands
        .spawn(panel(&t))
        .insert((
            GiftPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                width: Val::Px(480.0),
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
    let title = commands.spawn((Text::new("🎁 Presentes"), f.font(15.0, 700), TextColor(t.strong))).id();
    let body = commands
        .spawn((GiftBody, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(5.0), ..default() }))
        .id();
    commands.entity(root).add_children(&[title, body]);
    g.dirty = true;
}

fn day(ms: f64) -> String {
    // data no fuso de Brasília (UTC−3), como o jogo mostra no navegador da maioria dos jogadores
    let days = ((ms / 1000.0 - 3.0 * 3600.0) / 86400.0).floor() as i64;
    let (y, m, d) = civil(days);
    format!("{d:02}/{m:02}/{y}")
}

/// Dias desde 1970-01-01 → (ano, mês, dia), algoritmo de Howard Hinnant.
fn civil(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut g: ResMut<Gifts>,
    friends: Res<Friends>,
    farm_state: Res<crate::farm::Farm>,
    cooked: Res<crate::backpack::CookedRecipes>,
    farm: Res<crate::farm::FarmCatalog>,
    cook: Res<crate::cook::CookCatalog>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    body: Query<(Entity, Option<&Children>), With<GiftBody>>,
) {
    if !g.dirty {
        return;
    }
    g.dirty = false;
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
    // abas
    let tabs = commands.spawn(Node { column_gap: Val::Px(6.0), ..default() }).id();
    let mut names = vec!["📦 Minha caixa".to_owned(), "🎁 Enviar".to_owned()];
    if let Some((_, n)) = &g.to {
        names.push(format!("👀 Caixa de {n}"));
    }
    for (i, name) in names.iter().enumerate() {
        let b = button(&mut commands, &t, &f, name, g.tab == i);
        commands.entity(b).observe(move |_: On<Activate>, mut g: ResMut<Gifts>| {
            g.tab = i;
            g.dirty = true;
        });
        commands.entity(tabs).add_child(b);
    }
    out.push(tabs);
    if !g.msg.is_empty() {
        out.push(txt(&mut commands, g.msg.clone(), 14.0));
    }
    let grid = |commands: &mut Commands, list: &[Gift], own: bool, pick: Option<u64>| {
        let e = commands
            .spawn(Node { flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(4.0), row_gap: Val::Px(4.0), ..default() })
            .id();
        for gi in list {
            let Some((ic, _)) = item(&farm, &cook, &gi.k, &gi.i) else { continue };
            let b = button(commands, &t, &f, &format!("{ic}\n{}", gi.n), own && pick == Some(gi.id));
            if own {
                let id = gi.id;
                commands.entity(b).observe(move |_: On<Activate>, mut g: ResMut<Gifts>| {
                    g.pick = Some(id);
                    g.confirm = false;
                    g.dirty = true;
                });
            }
            commands.entity(e).add_child(b);
        }
        e
    };
    match g.tab {
        0 => {
            out.push(txt(
                &mut commands,
                format!(
                    "{} {} de {}{}. Ficam aqui para mostrar: não vão para a mochila.",
                    g.total,
                    if g.total == 1 { "presente" } else { "presentes" },
                    g.max,
                    if g.unseen > 0 {
                        format!(" · {} {}", g.unseen, if g.unseen == 1 { "novo" } else { "novos" })
                    } else {
                        String::new()
                    }
                ),
                13.0,
            ));
            if g.mine.is_empty() {
                out.push(txt(
                    &mut commands,
                    "Sua caixa está vazia. Quando alguém enviar um presente, ele aparece aqui.".into(),
                    14.0,
                ));
            } else {
                let mine = g.mine.clone();
                out.push(grid(&mut commands, &mine, true, g.pick));
            }
            if let Some(p) = g.pick.and_then(|id| g.mine.iter().find(|x| x.id == id).cloned())
                && let Some((ic, n)) = item(&farm, &cook, &p.k, &p.i)
            {
                out.push(txt(
                    &mut commands,
                    format!(
                        "{ic} {n} · {} · De {}, em {}.",
                        if p.k == "c" { "da fazenda" } else { "da culinária" },
                        p.n,
                        day(p.at)
                    ),
                    14.0,
                ));
                let rm = button(
                    &mut commands,
                    &t,
                    &f,
                    if g.confirm { "Clique de novo: some para sempre" } else { "🗑 Tirar da caixa" },
                    false,
                );
                let id = p.id;
                commands.entity(rm).observe(
                    move |_: On<Activate>, mut g: ResMut<Gifts>, mut send: MessageWriter<SendRaw>| {
                        if g.confirm {
                            g.confirm = false;
                            g.pick = None;
                            send.write(SendRaw(json!({"t": "giftdel", "id": id})));
                        } else {
                            g.confirm = true;
                        }
                        g.dirty = true;
                    },
                );
                out.push(rm);
            }
        }
        1 => {
            // para quem
            let row = commands
                .spawn(Node {
                    column_gap: Val::Px(4.0),
                    flex_wrap: FlexWrap::Wrap,
                    row_gap: Val::Px(4.0),
                    align_items: AlignItems::Center,
                    ..default()
                })
                .id();
            let l = txt(&mut commands, "Para:".into(), 14.0);
            commands.entity(row).add_child(l);
            let mut list: Vec<_> = friends.fr.iter().filter(|x| x.id != friends.me).collect();
            list.sort_by_key(|x| (!friends.on.contains(&x.id), x.n.to_lowercase()));
            if list.is_empty() {
                let e = txt(&mut commands, "Seus amigos aparecem aqui. Adicione alguém em 👥.".into(), 13.0);
                commands.entity(row).add_child(e);
            }
            for fr in list {
                let pp = pp_of(&fr.id);
                let sel = g.to.as_ref().is_some_and(|(p, _)| *p == pp);
                let b = button(
                    &mut commands,
                    &t,
                    &f,
                    &format!("{}{}", fr.n, if friends.on.contains(&fr.id) { " · online" } else { "" }),
                    sel,
                );
                let name = fr.n.clone();
                commands.entity(b).observe(
                    move |_: On<Activate>, mut g: ResMut<Gifts>, mut send: MessageWriter<SendRaw>, s: Res<Session>| {
                        g.to = Some((pp.clone(), name.clone()));
                        g.other = None;
                        g.confirm = false;
                        g.msg.clear();
                        if s.online {
                            send.write(SendRaw(json!({"t": "giftsee", "to": pp})));
                        }
                        g.dirty = true;
                    },
                );
                commands.entity(row).add_child(b);
            }
            out.push(row);
            if let Some(l) = g.left {
                out.push(txt(
                    &mut commands,
                    if l == 0 {
                        "Limite de hoje atingido.".into()
                    } else {
                        format!(
                            "{} de {} hoje",
                            if l == 1 { "Resta 1 presente".into() } else { format!("Restam {l} presentes") },
                            g.per_day
                        )
                    },
                    13.0,
                ));
            }
            // o que eu já produzi
            let done = farm_state.harvested();
            let mut any = false;
            for (k, title) in [("c", "🌱 Colheitas da fazenda"), ("r", "🍳 Receitas da culinária")] {
                let items: Vec<(String, String, String)> = if k == "c" {
                    farm.crops
                        .iter()
                        .filter(|c| done.contains(&c.id))
                        .map(|c| (c.id.clone(), c.ic.clone(), c.n.clone()))
                        .collect()
                } else {
                    cook.recipes
                        .iter()
                        .filter(|r| cooked.0.contains(&r.id))
                        .map(|r| (r.id.clone(), r.ic.clone(), r.n.clone()))
                        .collect()
                };
                if items.is_empty() {
                    continue;
                }
                any = true;
                out.push(txt(&mut commands, format!("{title} ({})", items.len()), 14.0));
                let wrap = commands
                    .spawn(Node {
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(4.0),
                        row_gap: Val::Px(4.0),
                        ..default()
                    })
                    .id();
                for (id, ic, n) in items {
                    let sel = g.sel.as_ref().is_some_and(|(sk, si)| sk == k && *si == id);
                    let b = button(&mut commands, &t, &f, &format!("{ic} {n}"), sel);
                    let key = (k.to_owned(), id);
                    commands.entity(b).observe(move |_: On<Activate>, mut g: ResMut<Gifts>| {
                        g.sel = Some(key.clone());
                        g.confirm = false;
                        g.dirty = true;
                    });
                    commands.entity(wrap).add_child(b);
                }
                out.push(wrap);
            }
            if !any {
                out.push(txt(&mut commands, "Você ainda não tem o que presentear: colha uma planta na 🌱 Fazenda ou prepare uma receita na 🍳 Culinária.".into(), 14.0));
            }
            if let Some((k, i)) = g.sel.clone()
                && let Some((ic, n)) = item(&farm, &cook, &k, &i)
            {
                let ok = g.to.is_some() && g.busy.is_none() && g.left != Some(0);
                let label = if g.confirm {
                    "Clique de novo para confirmar".to_owned()
                } else {
                    format!("🎁 Enviar {ic} {n}")
                };
                let b = button(&mut commands, &t, &f, &label, ok);
                if ok {
                    commands.entity(b).observe(
                        move |_: On<Activate>,
                              mut g: ResMut<Gifts>,
                              mut send: MessageWriter<SendRaw>,
                              s: Res<Session>| {
                            if !g.confirm {
                                g.confirm = true;
                                g.dirty = true;
                                return;
                            }
                            g.confirm = false;
                            let Some((pp, _)) = g.to.clone() else { return };
                            if !s.online {
                                g.msg = "Sem conexão: entre numa sala para enviar.".into();
                                g.dirty = true;
                                return;
                            }
                            let rid = crate::state::rid();
                            g.busy = Some(rid.clone());
                            g.msg = "Enviando…".into();
                            send.write(SendRaw(json!({"t": "giftsend", "to": pp, "k": k, "i": i, "rid": rid})));
                            g.dirty = true;
                        },
                    );
                }
                out.push(b);
                if g.to.is_none() {
                    out.push(txt(&mut commands, "Falta escolher para quem.".into(), 13.0));
                }
            }
        }
        _ => {
            let name = g.to.as_ref().map_or(String::new(), |(_, n)| n.clone());
            match g.other.clone() {
                None => out.push(txt(&mut commands, format!("Carregando a caixa de {name}…"), 14.0)),
                Some(None) => out.push(txt(
                    &mut commands,
                    "Este jogador não tem o perfil público ligado: a caixa dele não aparece e não dá para enviar presentes.".into(),
                    14.0,
                )),
                Some(Some(list)) => {
                    out.push(txt(&mut commands, format!("Caixa de {name}: {} {}.", list.len(), if list.len() == 1 { "presente" } else { "presentes" }), 14.0));
                    out.push(grid(&mut commands, &list, false, None));
                }
            }
        }
    }
    commands.entity(root).add_children(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_code_and_dates() {
        // o mesmo que o navegador calcula (crypto.subtle SHA-256, 20 primeiros dígitos)
        let pp = pp_of("abcdefgh2345");
        assert_eq!(pp, "e876ec2fdffd338fd966");
        assert!(pp.bytes().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(civil(0), (1970, 1, 1));
        assert_eq!(civil(20_000), (2024, 10, 4));
        assert_eq!(day(1_700_000_000_000.0), "14/11/2023");
    }
}
