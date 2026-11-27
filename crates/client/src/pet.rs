//! 🐾 Mascote (`assets/app-pet.js`): adotar, cuidar (alimentar, brincar, descansar, limpar, carinho, aventura), roupinha,
//! conquistas e soltar. O servidor decide tudo (genes, humor, necessidades, esperas); aqui se pede a ação e se mostra o
//! resultado. A criatura é desenhada por `pet_art` numa imagem que só anima com o painel aberto. Também dá para
//! cumprimentar o mascote de um amigo (`petsocial`).

use crate::canvas::Canvas;
use crate::friends::Friends;
use crate::pet_art::{Animator, Genes};
use crate::state::{ChatLog, Screen, SendRaw, ServerOther, Session};
use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::text::EditableText;
use serde_json::{Value, json};
use sky_ui::{Activate, Theme, UiFonts, button, panel, text_input};

#[derive(Debug)]
pub struct PetPlugin;

impl Plugin for PetPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Pet>()
            .add_systems(OnEnter(Screen::Playing), spawn_panel)
            .add_systems(OnExit(Screen::Playing), (crate::menu::despawn::<PetPanel>, reset))
            .add_systems(Update, (boot, receive, rebuild, animate).chain().run_if(in_state(Screen::Playing)));
    }
}

/// Tamanho da imagem do mascote (o desenho é 200×180).
const W: u32 = 300;
const H: u32 = 270;

const ACT: [(&str, &str, &str); 6] = [
    ("feed", "🍎", "Alimentar"),
    ("play", "🎾", "Brincar"),
    ("rest", "😴", "Descansar"),
    ("clean", "🛁", "Limpar"),
    ("pet", "🤗", "Carinho"),
    ("special", "⭐", "Aventura"),
];
const NEED: [(&str, &str, &str); 6] = [
    ("food", "🍖", "Fome"),
    ("energy", "⚡", "Energia"),
    ("joy", "😊", "Felicidade"),
    ("clean", "🫧", "Higiene"),
    ("health", "💚", "Saúde"),
    ("fun", "🎈", "Diversão"),
];
const COS: [&str; 5] = ["Nenhum", "Laço", "Cachecol", "Coroa estelar", "Óculos redondos"];

fn flash_of(a: &str) -> &'static str {
    match a {
        "feed" => "eat",
        "rest" => "sleep",
        "play" | "special" => "play",
        _ => "happy",
    }
}

const WHY: [(&str, &str); 8] = [
    ("cooldown", "Ele ainda está se recuperando."),
    ("full", "Ele já está de barriga cheia."),
    ("tired", "Está cansado demais para isso."),
    ("rested", "Ele já descansou bastante."),
    ("clean", "Já está limpinho."),
    ("rid", "Tente de novo."),
    ("action", "Essa ação não existe."),
    ("none", "Você ainda não tem um mascote."),
];
const ADOPT_ERR: [(&str, &str); 5] = [
    ("exists", "Você já tem um mascote."),
    ("name", "Use um nome simples, sem links."),
    ("full", "Não há vaga para novos mascotes agora."),
    ("busy", "Não foi possível agora. Tente de novo."),
    ("invalid", "Não foi possível adotar agora."),
];

fn now_ms() -> f64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
}

fn mins(ms: f64) -> String {
    if ms >= 3_600_000.0 {
        format!("{} h", (ms / 3_600_000.0).ceil())
    } else {
        format!("{} min", (ms / 60_000.0).ceil().max(1.0))
    }
}

#[derive(Resource, Default)]
pub struct Pet {
    pet: Option<Value>,
    /// Esperas: ação → quando libera (ms, relógio daqui).
    cd: Vec<(String, f64)>,
    anim: Option<Animator>,
    canvas: Option<Canvas>,
    image: Option<Handle<Image>>,
    busy: f64,
    msg: String,
    adopt_msg: String,
    release_at: f64,
    asked: bool,
    dirty: bool,
    open: bool,
}

impl std::fmt::Debug for Pet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pet").field("has", &self.pet.is_some()).finish_non_exhaustive()
    }
}

impl Pet {
    fn set(&mut self, p: Value) {
        let genes = Genes::from_value(&p["genes"]);
        let stage = sky_protocol::json::js_u64(&p["stage"]).unwrap_or(0) as usize;
        let cos = sky_protocol::json::js_i64(&p["cos"]).unwrap_or(0);
        let mood = p["mood"]["state"].as_str().unwrap_or("idle");
        match self.anim.as_mut() {
            Some(a) if a.genes == genes => {
                a.stage = stage;
                a.cos = cos;
                a.set_state(mood);
            }
            _ => self.anim = Some(Animator::new(genes, stage, cos, mood)),
        }
        let n = now_ms();
        self.cd = p["cd"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), n + v.as_f64().unwrap_or(0.0))).collect())
            .unwrap_or_default();
        self.pet = Some(p);
        self.dirty = true;
    }
}

fn reset(mut p: ResMut<Pet>) {
    let image = p.image.take();
    *p = Pet { image, ..Pet::default() };
}

fn boot(s: Res<Session>, mut p: ResMut<Pet>, mut send: MessageWriter<SendRaw>) {
    if s.online && !p.asked {
        p.asked = true;
        send.write(SendRaw(json!({"t": "petget"})));
    }
}

fn receive(
    mut msgs: MessageReader<ServerOther>,
    mut p: ResMut<Pet>,
    mut log: ResMut<ChatLog>,
    snd: Res<crate::sound::Sound>,
) {
    for m in msgs.read() {
        let v = &m.raw;
        match m.t.as_str() {
            "pet" => {
                p.busy = 0.0;
                if let Some(e) = v.get("err").and_then(Value::as_str) {
                    p.adopt_msg = if e == "locked" {
                        format!(
                            "Você soltou um mascote há pouco. Poderá adotar outro em {} dia(s).",
                            (v["wait"].as_f64().unwrap_or(0.0) / 86_400_000.0).ceil()
                        )
                    } else {
                        ADOPT_ERR.iter().find(|(k, _)| *k == e).map_or(ADOPT_ERR[4].1, |(_, t)| t).to_owned()
                    };
                    p.dirty = true;
                    continue;
                }
                if v.get("released").is_some_and(|x| x.as_bool().unwrap_or(false)) {
                    log.push("", "🐾 Você soltou seu mascote. Poderá adotar outro daqui a 30 dias.", true);
                }
                match v.get("pet").filter(|x| x.is_object()) {
                    Some(pet) => {
                        if v.get("fresh").is_some_and(|x| x.as_bool().unwrap_or(false)) {
                            log.push(
                                "",
                                format!("🐾 {} chegou! Cuide bem dele.", pet["name"].as_str().unwrap_or("Seu mascote")),
                                true,
                            );
                        }
                        p.set(pet.clone());
                    }
                    None => {
                        p.pet = None;
                        p.anim = None;
                        p.dirty = true;
                    }
                }
            }
            "petres" => {
                p.busy = 0.0;
                if let Some(pet) = v.get("pet").filter(|x| x.is_object()) {
                    p.set(pet.clone());
                }
                snd.play(if v["ok"].as_bool().unwrap_or(false) { "confirm" } else { "erro" }, 0.8);
                if v["ok"].as_bool().unwrap_or(false) {
                    let a = v["action"].as_str().unwrap_or_default();
                    if !v["dup"].as_bool().unwrap_or(false)
                        && let Some(an) = p.anim.as_mut()
                    {
                        an.flash(flash_of(a), if a == "rest" { 3.0 } else { 2.2 });
                    }
                    let fresh: Vec<&str> =
                        v["fresh"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
                    let names: Vec<String> = p
                        .pet
                        .as_ref()
                        .and_then(|x| x["ach"].as_array())
                        .into_iter()
                        .flatten()
                        .filter(|a| a["id"].as_str().is_some_and(|id| fresh.contains(&id)))
                        .filter_map(|a| a["n"].as_str().map(str::to_owned))
                        .collect();
                    p.msg = if !names.is_empty() {
                        format!("🏅 {}", names.join(" · "))
                    } else if v["gain"].as_f64().unwrap_or(0.0) > 0.0 {
                        "Ele gostou! O vínculo cresceu.".into()
                    } else {
                        "Ele gostou!".into()
                    };
                } else if let Some(r) = v["reason"].as_str() {
                    p.msg = if r == "cooldown" && v["wait"].as_f64().is_some() {
                        format!("Espere {}.", mins(v["wait"].as_f64().unwrap_or(0.0)))
                    } else {
                        WHY.iter().find(|(k, _)| *k == r).map_or("Não foi possível agora.", |(_, t)| t).to_owned()
                    };
                }
                p.dirty = true;
            }
            "petsocial" => {
                let ok = v["ok"].as_bool().unwrap_or(false);
                let k = v["k"].as_str().unwrap_or("wave");
                log.push(
                    "",
                    if ok {
                        match k {
                            "hug" => "🤗 Você fez carinho no mascote do seu amigo.".to_owned(),
                            "gift" => "🎁 Você deixou um presentinho para o mascote do seu amigo.".to_owned(),
                            _ => "👋 Você cumprimentou o mascote do seu amigo.".to_owned(),
                        }
                    } else {
                        match v["reason"].as_str().unwrap_or("") {
                            "cooldown" => "Você já fez isso há pouco. Tente de novo mais tarde.",
                            "none" => "Esse jogador não tem mascote público.",
                            "busy" => "Esse mascote já recebeu muitas visitas hoje.",
                            _ => "Não foi possível agora.",
                        }
                        .to_owned()
                    },
                    true,
                );
            }
            _ => {}
        }
    }
}

#[derive(Component, Debug)]
pub struct PetPanel;
#[derive(Component, Debug)]
struct PetBody;
#[derive(Component, Debug)]
struct PetName;

pub fn on_toggle(
    _: On<Activate>,
    mut q: Query<&mut Node, With<PetPanel>>,
    mut p: ResMut<Pet>,
    mut send: MessageWriter<SendRaw>,
    s: Res<Session>,
) {
    for mut n in &mut q {
        n.display = if n.display == Display::None { Display::Flex } else { Display::None };
        p.open = n.display == Display::Flex;
        if p.open && s.online {
            send.write(SendRaw(json!({"t": "petget"})));
        }
    }
    p.dirty = true;
}

fn spawn_panel(
    mut commands: Commands,
    t: Res<Theme>,
    f: Res<UiFonts>,
    mut p: ResMut<Pet>,
    mut images: ResMut<Assets<Image>>,
) {
    let img = p
        .image
        .get_or_insert_with(|| {
            images.add(Image::new(
                Extent3d { width: W, height: H, depth_or_array_layers: 1 },
                TextureDimension::D2,
                vec![0; (W * H * 4) as usize],
                TextureFormat::Rgba8UnormSrgb,
                RenderAssetUsages::default(),
            ))
        })
        .clone();
    let root = commands
        .spawn(panel(&t))
        .insert((
            PetPanel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(12.0),
                top: Val::Px(12.0),
                width: Val::Px(440.0),
                max_height: Val::Percent(84.0),
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
    let title = commands.spawn((Text::new("🐾 Mascote"), f.font(15.0, 700), TextColor(t.strong))).id();
    let pic = commands
        .spawn((
            ImageNode::new(img),
            Node {
                width: Val::Px(W as f32),
                height: Val::Px(H as f32),
                align_self: AlignSelf::Center,
                flex_shrink: 0.0,
                ..default()
            },
        ))
        .id();
    let body = commands
        .spawn((PetBody, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(5.0), ..default() }))
        .id();
    commands.entity(root).add_children(&[title, pic, body]);
    p.canvas = Canvas::new(W, H);
    p.dirty = true;
}

/// Desenha o mascote com o painel aberto (o animador escolhe quantos quadros por segundo).
fn animate(time: Res<Time>, mut p: ResMut<Pet>, mut images: ResMut<Assets<Image>>, mut acc: Local<f32>) {
    let dt = time.delta_secs();
    let p = &mut *p;
    // as esperas do botões contam a cada segundo
    *acc += dt;
    if *acc >= 1.0 {
        *acc = 0.0;
        if p.open && p.cd.iter().any(|(_, t)| *t > now_ms() - 1000.0) {
            p.dirty = true;
        }
    }
    if !p.open {
        return;
    }
    let (Some(an), Some(c), Some(h)) = (p.anim.as_mut(), p.canvas.as_mut(), p.image.as_ref()) else { return };
    if an.tick(dt, c)
        && let Some(mut img) = images.get_mut(h)
    {
        img.data = Some(c.rgba());
    }
}

#[allow(clippy::too_many_arguments)]
fn rebuild(
    mut commands: Commands,
    mut p: ResMut<Pet>,
    friends: Res<Friends>,
    t: Res<Theme>,
    f: Res<UiFonts>,
    body: Query<(Entity, Option<&Children>), With<PetBody>>,
    names: Query<&EditableText, With<PetName>>,
) {
    if !p.dirty {
        return;
    }
    p.dirty = false;
    let typed: Option<String> = names.iter().next().map(sky_ui::input_value);
    let Ok((root, kids)) = body.single() else { return };
    if let Some(kids) = kids {
        for c in kids {
            commands.entity(*c).despawn();
        }
    }
    let mut out = Vec::new();
    let txt = |commands: &mut Commands, s: String, size: f32, w: u16| {
        commands.spawn((Text::new(s), f.font(size, w), TextColor(t.fg))).id()
    };
    match p.pet.clone() {
        None => {
            out.push(txt(
                &mut commands,
                "Você ainda não tem um mascote. Dê um nome e adote uma criatura única, sorteada para você.".into(),
                14.0,
                500,
            ));
            let row =
                commands.spawn(Node { column_gap: Val::Px(6.0), align_items: AlignItems::Center, ..default() }).id();
            let input =
                commands.spawn((text_input(&t, &f, typed.as_deref().unwrap_or(""), Val::Px(200.0)), PetName)).id();
            let b = button(&mut commands, &t, &f, "🐾 Adotar", true);
            commands.entity(b).observe(
                |_: On<Activate>,
                 mut p: ResMut<Pet>,
                 names: Query<&EditableText, With<PetName>>,
                 mut send: MessageWriter<SendRaw>,
                 s: Res<Session>| {
                    let n: String = names
                        .iter()
                        .next()
                        .map(sky_ui::input_value)
                        .unwrap_or_default()
                        .chars()
                        .filter(|c| !"<>&\"'`".contains(*c))
                        .take(16)
                        .collect::<String>()
                        .trim()
                        .to_owned();
                    p.adopt_msg = if n.is_empty() {
                        "Escolha um nome para o mascote.".into()
                    } else if !s.online {
                        "Entre numa sala para adotar.".into()
                    } else {
                        send.write(SendRaw(json!({"t": "petadopt", "name": n})));
                        "Procurando seu mascote…".into()
                    };
                    p.dirty = true;
                },
            );
            commands.entity(row).add_children(&[input, b]);
            out.push(row);
            if !p.adopt_msg.is_empty() {
                out.push(txt(&mut commands, p.adopt_msg.clone(), 14.0, 600));
            }
        }
        Some(pet) => {
            let s = |k: &str| pet[k].as_str().unwrap_or_default().to_owned();
            let age = sky_protocol::json::js_u64(&pet["age"]).unwrap_or(0);
            out.push(txt(
                &mut commands,
                format!(
                    "{} · {} · {age} {} · {}",
                    s("name"),
                    s("stageName"),
                    if age == 1 { "dia" } else { "dias" },
                    s("pers")
                ),
                15.0,
                700,
            ));
            let special = s("special");
            out.push(txt(
                &mut commands,
                format!(
                    "{} · {}{}",
                    pet["mood"]["text"].as_str().unwrap_or_default(),
                    s("rarity"),
                    if special.is_empty() || special == "Nenhuma" { String::new() } else { format!(" · {special}") }
                ),
                14.0,
                500,
            ));
            let bond = pet["a"]["bond"].as_f64().unwrap_or(0.0);
            let full = ((bond / 20.0).floor() as usize).min(5);
            out.push(txt(&mut commands, format!("Vínculo: {}{}", "♥".repeat(full), "♡".repeat(5 - full)), 14.0, 600));
            // necessidades em barras
            for (k, ic, n) in NEED {
                let v = pet["a"][k].as_f64().unwrap_or(0.0).clamp(0.0, 100.0) as f32;
                let row = commands
                    .spawn(Node { column_gap: Val::Px(6.0), align_items: AlignItems::Center, ..default() })
                    .id();
                let l = commands
                    .spawn((
                        Text::new(format!("{ic} {n}")),
                        f.font(13.0, 500),
                        TextColor(t.fg),
                        Node { width: Val::Px(110.0), ..default() },
                    ))
                    .id();
                let track = commands
                    .spawn((
                        Node {
                            width: Val::Px(220.0),
                            height: Val::Px(8.0),
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(t.border),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Node {
                                width: Val::Percent(v),
                                height: Val::Percent(100.0),
                                border_radius: BorderRadius::all(Val::Px(4.0)),
                                ..default()
                            },
                            BackgroundColor(if v < 25.0 { Color::srgb(0.9, 0.35, 0.3) } else { t.accent }),
                        ));
                    })
                    .id();
                let pct = commands.spawn((Text::new(format!("{v:.0}%")), f.font(12.0, 500), TextColor(t.fg))).id();
                commands.entity(row).add_children(&[l, track, pct]);
                out.push(row);
            }
            // ações com espera
            let acts = commands
                .spawn(Node { column_gap: Val::Px(4.0), row_gap: Val::Px(4.0), flex_wrap: FlexWrap::Wrap, ..default() })
                .id();
            let now = now_ms();
            for (a, ic, n) in ACT {
                let left = p.cd.iter().find(|(k, _)| k == a).map_or(0.0, |(_, t)| t - now);
                let wait = left > 0.0;
                let label = if wait { format!("{ic} {n} · {}", mins(left)) } else { format!("{ic} {n}") };
                let b = button(&mut commands, &t, &f, &label, false);
                if !wait && now - p.busy > 2500.0 {
                    commands.entity(b).observe(
                        move |_: On<Activate>,
                              mut p: ResMut<Pet>,
                              mut send: MessageWriter<SendRaw>,
                              s: Res<Session>| {
                            if !s.online || now_ms() - p.busy < 2500.0 {
                                return;
                            }
                            p.busy = now_ms();
                            p.msg.clear();
                            send.write(SendRaw(json!({"t": "petact", "a": a, "rid": crate::state::rid()})));
                            p.dirty = true;
                        },
                    );
                }
                commands.entity(acts).add_child(b);
            }
            out.push(acts);
            if !p.msg.is_empty() {
                out.push(txt(&mut commands, p.msg.clone(), 14.0, 600));
            }
            // roupinha
            let cos_row = commands
                .spawn(Node {
                    column_gap: Val::Px(4.0),
                    row_gap: Val::Px(4.0),
                    flex_wrap: FlexWrap::Wrap,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .id();
            let l = txt(&mut commands, "Visual:".into(), 13.0, 600);
            commands.entity(cos_row).add_child(l);
            let cur = sky_protocol::json::js_i64(&pet["cos"]).unwrap_or(0);
            for i in pet["cosOk"].as_array().into_iter().flatten().filter_map(Value::as_i64) {
                let b = button(&mut commands, &t, &f, COS.get(i as usize).copied().unwrap_or("?"), i == cur);
                commands.entity(b).observe(move |_: On<Activate>, mut send: MessageWriter<SendRaw>| {
                    send.write(SendRaw(json!({"t": "petdress", "c": i})));
                });
                commands.entity(cos_row).add_child(b);
            }
            out.push(cos_row);
            // números, conquistas e soltar
            let st = &pet["stats"];
            let pl = |n: u64, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
            let u = |k: &str| sky_protocol::json::js_u64(&st[k]).unwrap_or(0);
            out.push(txt(
                &mut commands,
                [
                    pl(u("total"), "cuidado", "cuidados"),
                    pl(u("feed"), "refeição", "refeições"),
                    pl(u("play"), "brincadeira", "brincadeiras"),
                    pl(u("clean"), "banho", "banhos"),
                    pl(u("rest"), "descanso", "descansos"),
                ]
                .join(" · "),
                13.0,
                500,
            ));
            let ach: Vec<String> = pet["ach"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|a| a["n"].as_str().map(|n| format!("🏅 {n}")))
                .collect();
            if !ach.is_empty() {
                out.push(txt(&mut commands, ach.join("  "), 13.0, 500));
            }
            let sure = now - p.release_at < 4000.0;
            let rel = button(
                &mut commands,
                &t,
                &f,
                if sure { "Tem certeza? Clique de novo para soltar" } else { "Soltar mascote" },
                false,
            );
            commands.entity(rel).observe(|_: On<Activate>, mut p: ResMut<Pet>, mut send: MessageWriter<SendRaw>| {
                if now_ms() - p.release_at < 4000.0 {
                    p.release_at = 0.0;
                    send.write(SendRaw(json!({"t": "petrel", "sure": true})));
                } else {
                    p.release_at = now_ms();
                    p.msg = "Ele vai embora para sempre e você só poderá adotar outro daqui a 30 dias.".into();
                }
                p.dirty = true;
            });
            out.push(rel);
        }
    }
    // visitar o mascote de um amigo online
    let online: Vec<_> = friends.fr.iter().filter(|x| friends.on.contains(&x.id)).collect();
    if !online.is_empty() {
        out.push(txt(&mut commands, "Mascotes dos amigos online:".into(), 13.0, 700));
        for fr in online {
            let row =
                commands.spawn(Node { column_gap: Val::Px(4.0), align_items: AlignItems::Center, ..default() }).id();
            let n = txt(&mut commands, format!("{}  ", fr.n), 13.0, 600);
            commands.entity(row).add_child(n);
            let pp = crate::gift::pp_of(&fr.id);
            for (k, ic) in [("wave", "👋"), ("hug", "🤗"), ("gift", "🎁")] {
                let b = button(&mut commands, &t, &f, ic, false);
                let pp = pp.clone();
                commands.entity(b).observe(move |_: On<Activate>, mut send: MessageWriter<SendRaw>| {
                    send.write(SendRaw(json!({"t": "petsocial", "to": pp, "k": k})));
                });
                commands.entity(row).add_child(b);
            }
            out.push(row);
        }
    }
    commands.entity(root).add_children(&out);
}
