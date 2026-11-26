//! 🩺 Medicina lúdica (`assets/app-health.js`): remédios de fantasia feitos com a despensa da fazenda e oferecidos a um
//! amigo cansado na mesma sala (até 3 m). O servidor decide tudo (`lib/health.js`); quem está cansado aparece de cabeça
//! baixa e com o ícone da indisposição no nome.

use crate::friends::Friends;
use crate::player::Me;
use crate::state::{ActorName, ChatLog, Emote, Fid, NetId, Screen, SendRaw, ServerOther, Session};
use bevy::prelude::*;
use bevy::text::EditableText;
use serde::Deserialize;
use serde_json::{Value, json};
use sky_ui::{Activate, Theme, UiFonts, button, panel, text_input};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug)]
pub struct MedicinePlugin;

impl Plugin for MedicinePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Med>()
            .add_systems(OnEnter(Screen::Playing), spawn_panel)
            .add_systems(OnExit(Screen::Playing), (crate::menu::despawn::<MedPanel>, reset))
            .add_systems(Update, (boot, receive, search, refresh, rebuild).chain().run_if(in_state(Screen::Playing)));
    }
}

/// Distância para cuidar (o servidor aceita 3 m; aqui um pouco menos, pela demora da rede).
const NEAR: f32 = 2.7;

#[derive(Debug, Clone, Deserialize)]
pub struct Medicine {
    pub id: String,
    pub n: String,
    pub ic: String,
    pub g: Vec<String>,
    pub kind: String,
    #[serde(default)]
    pub info: String,
    /// Nota MIDI da melodia do remédio.
    #[serde(default)]
    pub note: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Route {
    pub n: String,
    pub action: String,
    pub effect: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Illness {
    pub n: String,
    pub ic: String,
}

#[derive(Resource, Debug, Clone, Default, Deserialize)]
pub struct MedCatalog {
    #[serde(rename = "MEDICINES")]
    pub meds: Vec<Medicine>,
    #[serde(rename = "MED_ROUTES")]
    pub routes: BTreeMap<String, Route>,
    #[serde(rename = "ILLNESSES")]
    pub ill: BTreeMap<String, Illness>,
}

impl MedCatalog {
    pub fn from_catalog_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        #[derive(Deserialize)]
        struct C {
            #[serde(rename = "MEDICINE")]
            m: MedCatalog,
        }
        Ok(serde_json::from_slice::<C>(bytes)?.m)
    }
    pub fn med(&self, id: &str) -> Option<&Medicine> {
        self.meds.iter().find(|m| m.id == id)
    }
    /// Situação em texto (`status`).
    pub fn status(&self, h: &Health) -> String {
        if h.tired {
            let i = self.ill.get(&h.ill);
            format!("{} {}", i.map_or("😴", |i| i.ic.as_str()), i.map_or("Cansaço", |i| i.n.as_str()))
        } else {
            "💚 Bem-disposto".into()
        }
    }
}

/// Saúde de um avatar (`health` no `pub` do servidor e `healthstate`).
#[derive(Component, Debug, Clone, Default, PartialEq)]
pub struct Health {
    pub ill: String,
    pub tired: bool,
}

impl Health {
    pub fn from_value(v: &Value) -> Self {
        Self {
            ill: v.get("ill").and_then(Value::as_str).unwrap_or_default().to_owned(),
            tired: v.get("tired").and_then(Value::as_bool).unwrap_or(false),
        }
    }
}

#[derive(Resource, Debug, Default)]
pub struct Med {
    lv: u64,
    helped: u64,
    meds: BTreeMap<String, u64>,
    me: Health,
    patient: Option<sky_shared::ActorId>,
    kind: String,
    q: String,
    busy: Option<(String, f64)>,
    msg: String,
    open: HashSet<String>,
    asked: bool,
    dirty: bool,
}

fn reset(mut m: ResMut<Med>) {
    *m = Med::default();
}

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

const WHY: [(&str, &str); 12] = [
    ("calma", "Aguarde um instante antes de repetir esta ação."),
    ("autocura", "Você só pode receber cura de outro jogador. Peça ajuda a um amigo."),
    ("despensa", "Faltam ingredientes: plante e colha na Fazenda primeiro."),
    ("remedio", "Prepare esse remédio antes de usar."),
    ("saudavel", "Essa pessoa já está bem. Seu remédio foi preservado."),
    ("longe", "Chegue mais perto do amigo (até 3 m)."),
    ("amigo", "Vocês precisam ser amigos para receber cuidados."),
    ("alvo", "Esse amigo precisa estar na mesma sala."),
    ("cheio", "Você já tem 99 unidades desse remédio."),
    ("receita", "Receita desconhecida."),
    ("full", "Não há espaço para novos registros agora."),
    ("descanso", "Descanse um pouco antes de preparar mais."),
];

fn boot(s: Res<Session>, mut m: ResMut<Med>, mut send: MessageWriter<SendRaw>) {
    if s.online && !m.asked {
        m.asked = true;
        send.write(SendRaw(json!({"t": "healthget"})));
    }
}

#[allow(clippy::too_many_arguments)]
fn receive(
    mut commands: Commands,
    mut msgs: MessageReader<ServerOther>,
    mut med: ResMut<Med>,
    cat: Res<MedCatalog>,
    mut log: ResMut<ChatLog>,
    me: Query<(Entity, &NetId), With<Me>>,
    actors: Query<(Entity, &NetId, &ActorName)>,
    mut health: Query<&mut Health>,
    snd: Res<crate::sound::Sound>,
    tf: Query<&GlobalTransform>,
) {
    let find = |id: &str| -> Option<Entity> {
        let id: sky_shared::ActorId = id.parse().ok()?;
        me.iter()
            .find(|(_, n)| n.0 == id)
            .map(|(e, _)| e)
            .or_else(|| actors.iter().find(|(_, n, _)| n.0 == id).map(|(e, _, _)| e))
    };
    for m in msgs.read() {
        let v = &m.raw;
        let set =
            |commands: &mut Commands, health: &mut Query<&mut Health>, e: Entity, h: Health| match health.get_mut(e) {
                Ok(mut x) => {
                    if *x != h {
                        *x = h;
                    }
                }
                Err(_) => {
                    commands.entity(e).insert(h);
                }
            };
        match m.t.as_str() {
            "health" => {
                med.lv = v.get("lv").and_then(Value::as_u64).unwrap_or(1);
                med.helped = v.get("helped").and_then(Value::as_u64).unwrap_or(0);
                med.meds = v
                    .get("meds")
                    .and_then(Value::as_object)
                    .map(|o| o.iter().filter_map(|(k, n)| n.as_u64().map(|n| (k.clone(), n))).collect())
                    .unwrap_or_default();
                med.me = Health::from_value(v);
                if let Ok((e, _)) = me.single() {
                    set(&mut commands, &mut health, e, med.me.clone());
                }
                med.dirty = true;
            }
            "healthstate" => {
                if let Some(e) = v.get("id").and_then(Value::as_str).and_then(find) {
                    set(&mut commands, &mut health, e, Health::from_value(v));
                }
                med.dirty = true;
            }
            "medr" => {
                let rid = v.get("rid").and_then(Value::as_str).unwrap_or_default();
                if med.busy.as_ref().is_none_or(|(r, _)| r != rid) {
                    continue;
                }
                med.busy = None;
                med.dirty = true;
                if let Some(st) = v.get("state") {
                    med.meds = st
                        .get("meds")
                        .and_then(Value::as_object)
                        .map(|o| o.iter().filter_map(|(k, n)| n.as_u64().map(|n| (k.clone(), n))).collect())
                        .unwrap_or_default();
                    med.lv = st.get("lv").and_then(Value::as_u64).unwrap_or(med.lv);
                    med.helped = st.get("helped").and_then(Value::as_u64).unwrap_or(med.helped);
                }
                let ok = v.get("ok").and_then(Value::as_bool).unwrap_or(false);
                let i = v.get("i").and_then(Value::as_str).unwrap_or_default();
                med.msg = if ok {
                    if v.get("k").and_then(Value::as_str) == Some("craft") {
                        let n = cat.med(i).map_or(i, |m| m.n.as_str());
                        if let (Ok((e, _)), Some(r)) = (me.single(), cat.med(i)) {
                            commands.entity(e).insert(Emote { ic: r.ic.clone(), left: 2.0 });
                            snd.med(r.note, &r.kind, true, 0.8);
                        }
                        format!("✔ {n} pronto. Escolha quem vai receber o cuidado.")
                    } else {
                        "💚 Cuidado concluído!".into()
                    }
                } else {
                    let r = v.get("reason").and_then(Value::as_str).unwrap_or_default();
                    WHY.iter()
                        .find(|(k, _)| *k == r)
                        .map_or("Aguarde um instante e tente novamente.", |(_, t)| t)
                        .to_owned()
                };
            }
            "medfx" => {
                // alguém cuidou de alguém: o paciente fica bem e o efeito aparece sobre ele
                let Some(r) = v.get("i").and_then(Value::as_str).and_then(|i| cat.med(i)) else { continue };
                let Some(to) = v.get("to").and_then(Value::as_str).and_then(find) else { continue };
                set(&mut commands, &mut health, to, Health::default());
                let here =
                    me.single().ok().and_then(|(e, _)| tf.get(e).ok()).map_or(Vec3::ZERO, GlobalTransform::translation);
                let there = tf.get(to).map_or(here, GlobalTransform::translation);
                let vol = crate::sound::dvol(Vec2::new(there.x - here.x, there.z - here.z).length());
                if vol > 0.03 {
                    snd.med(r.note, &r.kind, false, vol);
                }
                let injection = r.kind == "injection";
                commands.entity(to).insert(Emote {
                    ic: if injection { "😖".into() } else { r.ic.clone() },
                    left: if injection { 1.2 } else { 2.4 },
                });
                let healer = v.get("id").and_then(Value::as_str).and_then(find);
                if me.single().is_ok_and(|(e, _)| e == to)
                    && let Some(h) = healer.filter(|h| *h != to)
                {
                    let name = actors.get(h).map_or("Um amigo".to_owned(), |(_, _, n)| n.0.clone());
                    log.push("", format!("💚 {name} cuidou de você. Energia renovada!"), true);
                    med.me = Health::default();
                }
                med.dirty = true;
            }
            _ => {}
        }
    }
}

// ---------- interface ----------

#[derive(Component, Debug)]
pub struct MedPanel;
#[derive(Component, Debug)]
struct MedBody;
#[derive(Component, Debug)]
struct MedSearch;

pub fn on_toggle(
    _: On<Activate>,
    mut q: Query<&mut Node, With<MedPanel>>,
    mut m: ResMut<Med>,
    mut send: MessageWriter<SendRaw>,
    s: Res<Session>,
) {
    for mut n in &mut q {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
        // aberto: pede o estado e a despensa de novo
        if n.display == Display::Flex && s.online {
            send.write(SendRaw(json!({"t": "healthget"})));
            send.write(SendRaw(json!({"t": "farmget"})));
        }
    }
    m.dirty = true;
}

fn spawn_panel(mut commands: Commands, t: Res<Theme>, f: Res<UiFonts>, mut med: ResMut<Med>) {
    let root = commands
        .spawn(panel(&t))
        .insert((
            MedPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                width: Val::Px(500.0),
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
    let head = commands.spawn(Node { column_gap: Val::Px(8.0), align_items: AlignItems::Center, ..default() }).id();
    let title = commands.spawn((Text::new("🩺 Medicina"), f.font(15.0, 700), TextColor(t.strong))).id();
    let q = commands.spawn((text_input(&t, &f, "", Val::Px(160.0)), MedSearch)).id();
    commands.entity(head).add_children(&[title, q]);
    let body = commands
        .spawn((MedBody, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(5.0), ..default() }))
        .id();
    commands.entity(root).add_children(&[head, body]);
    med.dirty = true;
}

fn search(q: Query<&EditableText, (With<MedSearch>, Changed<EditableText>)>, mut m: ResMut<Med>) {
    for e in &q {
        let v = crate::backpack::norm(sky_ui::input_value(e).trim());
        if v != m.q {
            m.q = v;
            m.dirty = true;
        }
    }
}

/// A cada segundo, com o painel aberto: a distância até o paciente muda o que dá para fazer.
fn refresh(time: Res<Time>, mut acc: Local<f32>, mut m: ResMut<Med>, panel: Query<&Node, With<MedPanel>>) {
    *acc += time.delta_secs();
    if *acc < 1.0 {
        return;
    }
    *acc = 0.0;
    if m.busy.as_ref().is_some_and(|(_, t)| now_ms() - t > 6000.0) {
        m.busy = None;
        m.msg = "Sem resposta. Confira a conexão antes de tentar novamente.".into();
        m.dirty = true;
    }
    if panel.iter().any(|n| n.display != Display::None) {
        m.dirty = true;
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn rebuild(
    mut commands: Commands,
    mut med: ResMut<Med>,
    cat: Res<MedCatalog>,
    farm: Res<crate::farm::Farm>,
    ings: Res<crate::cook::CookCatalog>,
    friends: Res<Friends>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    me: Query<&Transform, With<Me>>,
    actors: Query<(&NetId, &ActorName, &Fid, &Transform, Option<&Health>), Without<Me>>,
    body: Query<(Entity, Option<&Children>), With<MedBody>>,
) {
    if !med.dirty && !farm.is_changed() {
        return;
    }
    med.dirty = false;
    let Ok((root, kids)) = body.single() else { return };
    if let Some(kids) = kids {
        for c in kids {
            commands.entity(*c).despawn();
        }
    }
    let pantry = farm.pantry();
    let qty = |g: &str| pantry.get(g).copied().unwrap_or(0);
    let mut out = Vec::new();
    let txt = |commands: &mut Commands, s: String, size: f32, w: u16| {
        commands.spawn((Text::new(s), f.font(size, w), TextColor(t.fg))).id()
    };
    out.push(txt(
        &mut commands,
        format!(
            "{}{} · Medicina · Nível {} · {} cuidados com amigos",
            cat.status(&med.me),
            if med.me.tired { " · Peça um remédio a um amigo" } else { "" },
            med.lv.max(1),
            med.helped
        ),
        14.0,
        600,
    ));
    // pacientes: amigos nesta sala
    let here = me.single().map_or(Vec2::ZERO, |t| Vec2::new(t.translation.x, t.translation.z));
    let friends_here: Vec<_> =
        actors.iter().filter(|(id, _, fid, _, _)| !id.0.bot && friends.friend_of_handle(&fid.0).is_some()).collect();
    let row = commands
        .spawn(Node {
            column_gap: Val::Px(6.0),
            flex_wrap: FlexWrap::Wrap,
            align_items: AlignItems::Center,
            ..default()
        })
        .id();
    let lbl = txt(&mut commands, "Paciente:".into(), 14.0, 600);
    commands.entity(row).add_child(lbl);
    if friends_here.is_empty() {
        let e = txt(&mut commands, "Convide um amigo para esta sala (👥 › Convidar).".into(), 14.0, 500);
        commands.entity(row).add_child(e);
        med.patient = None;
    }
    let mut patient: Option<(sky_shared::ActorId, bool, bool)> = None;
    for (id, name, _, tr, h) in &friends_here {
        let h = h.cloned().unwrap_or_default();
        let near = Vec2::new(tr.translation.x, tr.translation.z).distance(here) <= NEAR;
        let sel = med.patient == Some(id.0);
        if sel {
            patient = Some((id.0, h.tired, near));
        }
        let b = button(&mut commands, &t, &f, &format!("{} · {}", name.0, cat.status(&h)), sel);
        let aid = id.0;
        commands.entity(b).observe(move |_: On<Activate>, mut m: ResMut<Med>| {
            m.patient = Some(aid);
            m.dirty = true;
        });
        commands.entity(row).add_child(b);
    }
    out.push(row);
    if let Some((_, _, near)) = patient
        && !near
    {
        out.push(txt(&mut commands, "Aproxime-se para cuidar (até 3 m).".into(), 13.0, 500));
    }
    // filtro por forma de aplicação
    let kinds = commands
        .spawn(Node { column_gap: Val::Px(4.0), flex_wrap: FlexWrap::Wrap, row_gap: Val::Px(4.0), ..default() })
        .id();
    for (k, label) in std::iter::once(("", "Todas")).chain(cat.routes.iter().map(|(k, r)| (k.as_str(), r.n.as_str()))) {
        let b = button(&mut commands, &t, &f, label, med.kind == k);
        let k = k.to_owned();
        commands.entity(b).observe(move |_: On<Activate>, mut m: ResMut<Med>| {
            m.kind.clone_from(&k);
            m.dirty = true;
        });
        commands.entity(kinds).add_child(b);
    }
    out.push(kinds);
    if !med.msg.is_empty() {
        out.push(txt(&mut commands, med.msg.clone(), 14.0, 600));
    }
    // receitas
    let q = med.q.clone();
    let list: Vec<&Medicine> = cat
        .meds
        .iter()
        .filter(|r| med.kind.is_empty() || r.kind == med.kind)
        .filter(|r| {
            q.is_empty()
                || crate::backpack::norm(&r.n).contains(&q)
                || r.g.iter().any(|g| crate::backpack::norm(g).contains(&q))
        })
        .collect();
    out.push(txt(
        &mut commands,
        format!("{} de {} receitas · Abra uma para preparar", list.len(), cat.meds.len()),
        13.0,
        500,
    ));
    for r in list {
        let have = med.meds.get(&r.id).copied().unwrap_or(0);
        let open = med.open.contains(&r.id);
        let hb =
            button(&mut commands, &t, &f, &format!("{} {} {} ×{have}", if open { "▾" } else { "▸" }, r.ic, r.n), false);
        let id = r.id.clone();
        commands.entity(hb).observe(move |_: On<Activate>, mut m: ResMut<Med>| {
            if !m.open.remove(&id) {
                m.open.insert(id.clone());
            }
            m.dirty = true;
        });
        out.push(hb);
        if !open {
            continue;
        }
        let route = cat.routes.get(&r.kind);
        out.push(txt(
            &mut commands,
            format!("{} · {}\n{}", route.map_or("", |x| x.n.as_str()), route.map_or("", |x| x.effect.as_str()), r.info),
            13.0,
            500,
        ));
        let list: Vec<String> = r
            .g
            .iter()
            .map(|g| {
                let i = ings.ing(g);
                format!("{} {} {}/1", i.map_or("", |i| i.ic.as_str()), i.map_or(g.as_str(), |i| i.n.as_str()), qty(g))
            })
            .collect();
        out.push(txt(&mut commands, list.join(" · "), 13.0, 500));
        let acts = commands.spawn(Node { column_gap: Val::Px(6.0), ..default() }).id();
        let can_craft = med.busy.is_none() && have < 99 && r.g.iter().all(|g| qty(g) > 0);
        let craft = button(&mut commands, &t, &f, "Preparar", false);
        if can_craft {
            let id = r.id.clone();
            commands.entity(craft).observe(
                move |_: On<Activate>, mut m: ResMut<Med>, mut send: MessageWriter<SendRaw>, s: Res<Session>| {
                    act(&mut m, &mut send, &s, json!({"t": "medcraft", "i": id}));
                },
            );
        }
        let can_use = med.busy.is_none() && have > 0 && patient.is_some_and(|(_, tired, near)| tired && near);
        let use_b = button(&mut commands, &t, &f, route.map_or("Usar", |x| x.action.as_str()), can_use);
        if can_use && let Some((to, _, _)) = patient {
            let id = r.id.clone();
            commands.entity(use_b).observe(
                move |_: On<Activate>, mut m: ResMut<Med>, mut send: MessageWriter<SendRaw>, s: Res<Session>| {
                    act(&mut m, &mut send, &s, json!({"t": "medheal", "i": id, "to": to.to_string()}));
                },
            );
        }
        commands.entity(acts).add_children(&[craft, use_b]);
        out.push(acts);
    }
    commands.entity(root).add_children(&out);
}

fn act(m: &mut Med, send: &mut MessageWriter<SendRaw>, s: &Session, mut v: Value) {
    if m.busy.is_some() {
        return;
    }
    if !s.online {
        m.msg = "Sem conexão: a medicina fica no servidor.".into();
        m.dirty = true;
        return;
    }
    let rid = format!("med{}", crate::state::rid());
    v["rid"] = Value::String(rid.clone());
    m.msg = if v["t"] == "medcraft" { "Preparando…".into() } else { "Cuidando…".into() };
    m.busy = Some((rid, now_ms()));
    m.dirty = true;
    send.write(SendRaw(v));
}

/// Cansado: cabeça baixa e braços caídos (`healthPose`), fora de gestos.
pub fn tired_pose(p: &mut crate::anim::Pose) {
    p.hd.x += 0.16;
    p.arm_l.x += 0.08;
    p.arm_r.x += 0.08;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_and_status() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogs/catalog.json");
        let cat = MedCatalog::from_catalog_json(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(cat.meds.len(), 18);
        assert!(cat.meds.iter().all(|m| cat.routes.contains_key(&m.kind)));
        assert_eq!(cat.status(&Health::default()), "💚 Bem-disposto");
        let h = Health { ill: "frio".into(), tired: true };
        assert_eq!(cat.status(&h), "🥶 Frio estelar");
        // o id do pedido passa na regra do servidor (^med[A-Za-z0-9_-]{6,21}$)
        let rid = format!("med{}", crate::state::rid());
        assert!(rid.len() >= 9 && rid.len() <= 24);
    }
}
