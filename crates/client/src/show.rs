//! Efeitos sobre a cena: o destaque de um objeto (`lifeShow` do `app-life.js`, regras.md 13.1) e a aura dos remédios
//! (`aura`, `prop` e `healthPose` do `app-health.js`).
//!
//! O destaque é o objeto (emoji ou ícone do jogo) subindo, flutuando e girando no próprio eixo, com um brilho suave e
//! 14 partículas luminosas em volta; some encolhendo e esmaecendo. Como os ícones na mão, é desenhado pela interface
//! na posição projetada do ponto 3D (o tamanho acompanha a distância). A aura é a nuvem de pontos do remédio em volta
//! de quem recebe o cuidado e o frasco (seringa, pomada, comprimido…) em 3D ao lado da mão.

use crate::player::Me;
use crate::state::{ActorName, NetId, Screen};
use bevy::asset::RenderAssetUsages;
use bevy::ecs::system::SystemParam;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sky_rendering::{Kind, MaterialDesc, ToonMaterial};
use sky_ui::UiFonts;
use sky_world::CurrentWorld;
use std::f32::consts::{PI, TAU};

pub struct ShowPlugin;

impl Plugin for ShowPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<LifeShow>()
            .add_message::<MedAura>()
            .add_systems(Startup, spark_texture)
            .add_systems(OnExit(Screen::Playing), (crate::menu::despawn::<Show>, crate::menu::despawn::<Aura>))
            .add_systems(
                Update,
                (spawn_shows, tick_shows, spawn_auras, tick_auras, age_poses).run_if(in_state(Screen::Playing)),
            );
    }
}

/// Onde o destaque aparece: sobre o jogador local, sobre outro avatar ou num ponto do chão (a fogueira).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum At {
    Me,
    Actor(Entity),
    Xz(Vec2),
}

/// `lifeShow(ic, x, y, z, {dur, s})`: o ponto é lido uma vez, quando o destaque nasce (não segue o avatar).
#[derive(Message, Debug, Clone)]
pub struct LifeShow {
    pub ic: String,
    pub at: At,
    pub y: f32,
    pub dur: f32,
    pub s: f32,
}

impl LifeShow {
    pub fn new(ic: impl Into<String>, at: At, y: f32, dur: f32, s: f32) -> Self {
        Self { ic: ic.into(), at, y, dur, s }
    }
}

/// `aura(a, r, craft)`: pontos na cor do remédio e o frasco ao lado da mão, seguindo o avatar.
#[derive(Message, Debug, Clone)]
pub struct MedAura {
    pub on: At,
    pub kind: String,
    pub color: u32,
    pub accent: u32,
    pub craft: bool,
}

/// Pose do cuidado (`a.medPose`): 1,5 s de braços e cabeça, por cima da animação.
#[derive(Component, Debug, Clone)]
pub struct MedPose {
    pub age: f32,
    /// `prepare` (quem preparou), `care` (quem cuidou), `ouch` (injeção) ou o tipo do remédio (quem recebeu).
    pub kind: String,
    /// Tipo do remédio (`r.kind`).
    pub route: String,
}

impl MedPose {
    pub fn new(kind: &str, route: &str) -> Self {
        Self { age: 0.0, kind: kind.into(), route: route.into() }
    }

    /// `healthPose`: aplica sobre a pose do quadro.
    pub fn apply(&self, p: &mut crate::anim::Pose) {
        let age = self.age;
        let k = (PI * (age / 1.5).min(1.0)).sin();
        match self.kind.as_str() {
            "ouch" => {
                // a boca abre bem (`mo.scale.y = 1 + 1,8k`)
                if k > 0.35 {
                    p.mouth = 2;
                }
                p.hd.x -= k * 0.25;
                p.fig_rot.z += (age * 24.0).sin() * 0.08 * k;
                p.arm_l.z -= k * 0.5;
                p.arm_r.z += k * 0.5;
            }
            "prepare" => {
                p.arm_l.x -= k * 0.8;
                p.arm_r.x -= k * 0.9;
                p.arm_r.z += (age * 10.0).sin() * 0.12 * k;
            }
            "care" => {
                p.arm_r.x -= k * 1.4;
                if self.route == "balm" {
                    p.arm_r.z += (age * 9.0).sin() * 0.15 * k;
                }
                if self.route == "ritual" {
                    p.arm_l.x -= k * 1.3;
                }
            }
            "drink" | "tablet" => {
                p.arm_r.x -= k * 1.7;
                p.hd.x -= k * 0.22;
                if k > 0.5 {
                    p.mouth = p.mouth.max(1);
                }
            }
            "mist" => {
                p.hd.x -= k * 0.14;
                p.arm_l.z -= k * 0.18;
                p.arm_r.z += k * 0.18;
            }
            "ritual" => {
                p.arm_l.z -= k * 0.45;
                p.arm_r.z += k * 0.45;
                p.hd.x -= k * 0.2;
            }
            _ => {
                p.hd.x -= k * 0.15;
                p.fig_rot.z += (age * 5.0).sin() * 0.025 * k;
            }
        }
    }
}

fn age_poses(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut MedPose)>) {
    for (e, mut p) in &mut q {
        p.age += time.delta_secs();
        if p.age > 1.5 {
            commands.entity(e).remove::<MedPose>();
        }
    }
}

/// Avatares da sala pelo id do servidor (`actors.get(id)` do original): entidade, nome e distância até o jogador local.
#[derive(SystemParam)]
pub struct Actors<'w, 's> {
    me: Query<'w, 's, (Entity, &'static NetId, &'static GlobalTransform), With<Me>>,
    others: Query<'w, 's, (Entity, &'static NetId, &'static ActorName, &'static GlobalTransform), Without<Me>>,
}

impl Actors<'_, '_> {
    pub fn find(&self, id: &str) -> Option<(Entity, String, f32)> {
        let id: sky_shared::ActorId = id.parse().ok()?;
        let here = self.me.single().map_or(Vec3::ZERO, |(_, _, t)| t.translation());
        let d = |t: &GlobalTransform| Vec2::new(t.translation().x - here.x, t.translation().z - here.z).length();
        if let Ok((e, n, t)) = self.me.single()
            && n.0 == id
        {
            return Some((e, String::new(), d(t)));
        }
        self.others.iter().find(|(_, n, _, _)| n.0 == id).map(|(e, _, name, t)| (e, name.0.clone(), d(t)))
    }
}

// ---------- destaque ----------

/// Textura das faíscas (`spkTex`): degradê radial branco de 32 × 32.
#[derive(Resource, Debug, Clone)]
struct Spark(Handle<Image>);

/// Alfa do degradê de `spkTex` a uma distância `d` do centro (0 a 1): 1 → 0,55 em 0,35 → 0.
fn spark_alpha(d: f32) -> f32 {
    if d <= 0.35 {
        1.0 - 0.45 * d / 0.35
    } else if d < 1.0 {
        0.55 * (1.0 - (d - 0.35) / 0.65)
    } else {
        0.0
    }
}

fn spark_texture(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let mut px = Vec::with_capacity(32 * 32 * 4);
    for y in 0..32 {
        for x in 0..32 {
            let d = Vec2::new(x as f32 + 0.5 - 16.0, y as f32 + 0.5 - 16.0).length() / 16.0;
            px.extend_from_slice(&[255, 255, 255, (spark_alpha(d) * 255.0).round() as u8]);
        }
    }
    let img = Image::new(
        Extent3d { width: 32, height: 32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        px,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    commands.insert_resource(Spark(images.add(img)));
}

const SPARKS: usize = 14;

#[derive(Component, Debug)]
pub struct Show {
    pos: Vec3,
    s: f32,
    t: f32,
    dur: f32,
    text: bool,
}
#[derive(Component, Debug)]
struct ShowCard;
#[derive(Component, Debug)]
struct ShowGlow;
#[derive(Component, Debug)]
struct ShowSpark(usize);

/// Ícone desenhado do jogo (`ICONS`, exportados em `assets/icons/`) em vez de emoji.
fn icon_image(ic: &str) -> Option<String> {
    (!ic.is_empty() && ic.bytes().all(|c| c.is_ascii_uppercase() || c == b'_')).then(|| format!("icons/{ic}.png"))
}

/// Onde o efeito começa, já no mundo.
fn resolve(at: At, me: &Query<&GlobalTransform, With<Me>>, tf: &Query<&GlobalTransform>) -> Option<Vec2> {
    match at {
        At::Me => me.single().ok().map(|t| t.translation().xz()),
        At::Actor(e) => tf.get(e).ok().map(|t| t.translation().xz()),
        At::Xz(p) => Some(p),
    }
}

fn spawn_shows(
    mut commands: Commands,
    mut msgs: MessageReader<LifeShow>,
    f: Res<UiFonts>,
    assets: Res<AssetServer>,
    spark: Option<Res<Spark>>,
    me: Query<&GlobalTransform, With<Me>>,
    tf: Query<&GlobalTransform>,
) {
    let Some(spark) = spark else { return };
    for m in msgs.read() {
        let Some(xz) = resolve(m.at, &me, &tf) else { continue };
        let text = icon_image(&m.ic).is_none();
        let root = commands
            .spawn((
                Show { pos: Vec3::new(xz.x, m.y, xz.y), s: m.s, t: 0.0, dur: m.dur, text },
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                Pickable::IGNORE,
                Visibility::Hidden,
            ))
            .id();
        let abs = || (Node { position_type: PositionType::Absolute, ..default() }, Pickable::IGNORE);
        let glow = commands
            .spawn((ShowGlow, abs(), ImageNode::new(spark.0.clone()).with_color(Color::srgba_u8(255, 210, 122, 0))))
            .id();
        let card = match icon_image(&m.ic) {
            Some(path) => {
                commands.spawn((ShowCard, abs(), UiTransform::IDENTITY, ImageNode::new(assets.load(path)))).id()
            }
            None => {
                // texto comum (o "Ai!" da injeção) sai em preto, como o `fillText` sem cor do original
                let ascii = m.ic.is_ascii();
                let c = if ascii { Color::BLACK } else { Color::WHITE };
                commands
                    .spawn((
                        ShowCard,
                        abs(),
                        UiTransform::IDENTITY,
                        Text::new(m.ic.clone()),
                        f.font(20.0, if ascii { 800 } else { 500 }),
                        TextColor(c.with_alpha(0.0)),
                        TextLayout::no_wrap(),
                    ))
                    .id()
            }
        };
        let mut kids = vec![glow, card];
        for j in 0..SPARKS {
            kids.push(
                commands
                    .spawn((
                        ShowSpark(j),
                        abs(),
                        ImageNode::new(spark.0.clone()).with_color(Color::srgba_u8(255, 242, 176, 0)),
                    ))
                    .id(),
            );
        }
        commands.entity(root).add_children(&kids);
    }
}

/// Opacidade do destaque no tempo `t` (aparece em 0,45 s e some nos últimos 0,7 s) e a subida `a`.
fn show_curve(t: f32, dur: f32) -> (f32, f32, f32) {
    let a = (t / 0.45).min(1.0);
    let out = ((t - (dur - 0.7)) / 0.7).clamp(0.0, 1.0);
    (a, out, a * (1.0 - out))
}

/// Posição local da faísca `j` (`showTick`; o original usa 6,283 em vez de 2π).
#[allow(clippy::approx_constant)]
fn spark_pos(j: usize, t: f32, s: f32) -> Vec3 {
    let jf = j as f32;
    let q = jf / SPARKS as f32 * 6.283 + t * if j % 2 == 1 { 1.3 } else { -1.1 };
    let r = s * (0.5 + 0.12 * (t * 3.0 + jf).sin());
    let h = ((jf * 0.37 + t * 0.32) % 1.0) * 0.8 - 0.35;
    Vec3::new(q.cos() * r, h, q.sin() * r)
}

/// Pixels por metro (lógicos da interface) no ponto `p`, e a tangente de meio campo de visão (tamanho dos pontos).
fn scale_at(camera: &Camera, cam_t: &GlobalTransform, p: Vec3, ui: f32) -> Option<(Vec2, f32)> {
    let v = camera.world_to_viewport(cam_t, p).ok()?;
    let up = cam_t.up().as_vec3();
    let v1 = camera.world_to_viewport(cam_t, p + up).ok()?;
    Some((v / ui, v.distance(v1) / ui))
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn tick_shows(
    mut commands: Commands,
    time: Res<Time>,
    ui: Res<UiScale>,
    cam: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    mut shows: Query<(Entity, &mut Show, &Children, &mut Visibility)>,
    mut cards: Query<
        (
            &mut Node,
            &mut UiTransform,
            Option<&mut ImageNode>,
            Option<&mut TextColor>,
            Option<&mut TextFont>,
            &ComputedNode,
        ),
        (With<ShowCard>, Without<ShowGlow>, Without<ShowSpark>),
    >,
    mut glows: Query<(&mut Node, &mut ImageNode), (With<ShowGlow>, Without<ShowCard>, Without<ShowSpark>)>,
    mut sparks: Query<(&ShowSpark, &mut Node, &mut ImageNode), (Without<ShowCard>, Without<ShowGlow>)>,
) {
    let Ok((camera, cam_t, proj)) = cam.single() else { return };
    let half = match proj {
        Projection::Perspective(p) => (p.fov / 2.0).tan(),
        _ => 0.577,
    };
    let dt = time.delta_secs();
    for (e, mut sh, kids, mut vis) in &mut shows {
        sh.t += dt;
        if sh.t >= sh.dur {
            commands.entity(e).despawn();
            continue;
        }
        let t = sh.t;
        let (a, out, k) = show_curve(t, sh.dur);
        let y = sh.pos.y + 0.45 * (1.0 - (1.0 - a) * (1.0 - a)) + (t * 2.2).sin() * 0.05;
        let c = Vec3::new(sh.pos.x, y, sh.pos.z);
        let Some((v, ppm)) = scale_at(camera, cam_t, c, ui.0) else {
            *vis = Visibility::Hidden;
            continue;
        };
        *vis = Visibility::Inherited;
        let place = |n: &mut Node, at: Vec2, w: f32, h: f32| {
            n.left = Val::Px(at.x - w / 2.0);
            n.top = Val::Px(at.y - h / 2.0);
        };
        for kid in kids.iter() {
            if let Ok((mut n, mut tr, img, color, font, computed)) = cards.get_mut(kid) {
                let px = sh.s * ppm;
                let sc = (0.55 + 0.45 * a) * (1.0 - 0.55 * out);
                // `card.rotation.y = 1,7t` numa placa de dois lados: de lado some, de costas espelha
                tr.scale = Vec2::new((t * 1.7).cos() * sc, sc);
                if sh.text {
                    if let Some(mut font) = font {
                        // a placa de 128 px com a fonte de 96 px
                        font.font_size = (px * 0.75).max(1.0).into();
                    }
                    if let Some(mut c) = color {
                        c.0.set_alpha(k);
                    }
                    let size = computed.size() * computed.inverse_scale_factor();
                    place(&mut n, v, size.x, size.y);
                } else {
                    n.width = Val::Px(px);
                    n.height = Val::Px(px);
                    if let Some(mut img) = img {
                        img.color.set_alpha(k);
                    }
                    place(&mut n, v, px, px);
                }
            } else if let Ok((mut n, mut img)) = glows.get_mut(kid) {
                let px = sh.s * 2.0 * ppm;
                n.width = Val::Px(px);
                n.height = Val::Px(px);
                img.color.set_alpha(0.42 * k);
                place(&mut n, v, px, px);
            } else if let Ok((sp, mut n, mut img)) = sparks.get_mut(kid) {
                let p = c + spark_pos(sp.0, t, sh.s);
                match scale_at(camera, cam_t, p, ui.0) {
                    Some((pv, pppm)) => {
                        // `PointsMaterial` com `sizeAttenuation`: tamanho × altura / 2 / profundidade
                        let px = (0.13 * pppm * half).max(1.0);
                        n.width = Val::Px(px);
                        n.height = Val::Px(px);
                        img.color.set_alpha(0.9 * k);
                        place(&mut n, pv, px, px);
                    }
                    None => img.color.set_alpha(0.0),
                }
            }
        }
    }
}

// ---------- aura dos remédios ----------

#[derive(Component, Debug)]
pub struct Aura {
    on: Entity,
    kind: String,
    craft: bool,
    time: f32,
    seeds: Vec<(f32, f32)>,
    prop: Entity,
}
#[derive(Component, Debug)]
struct AuraDot(usize);

/// Cor de hex como o original (`ColorManagement` desligado: o valor entra cru).
fn raw(c: u32) -> LinearRgba {
    LinearRgba::rgb(((c >> 16) & 255) as f32 / 255.0, ((c >> 8) & 255) as f32 / 255.0, (c & 255) as f32 / 255.0)
}

fn basic(color: LinearRgba, world: &CurrentWorld) -> ToonMaterial {
    let d = MaterialDesc {
        kind: Kind::Basic,
        color,
        emissive: LinearRgba::BLACK,
        texture: None,
        uv_scale: Vec2::ONE,
        uv_offset: Vec2::ZERO,
        flat: false,
        double_sided: false,
        back_side: false,
        transparent: false,
        alpha_cutoff: None,
        depth_write: true,
        fog: true,
        vertex_colors: false,
        ground_avg: None,
        additive: false,
    };
    ToonMaterial::new(&d, &world.lighting)
}

/// Peças do frasco (`prop`): malha, material (0 corpo, 1 cor, 2 detalhe) e transformação local.
fn prop_parts(kind: &str) -> (Vec<(Mesh, usize, Transform)>, f32) {
    let cyl = |r: f32, h: f32| Cylinder::new(r, h).mesh().resolution(8).build();
    let at = |y: f32| Transform::from_xyz(0.0, y, 0.0);
    let mut v = Vec::new();
    let mut rot_z = 0.0;
    match kind {
        "injection" => {
            v.push((cyl(0.065, 0.3), 0, at(0.0)));
            v.push((cyl(0.09, 0.035), 1, at(0.17)));
            v.push((cyl(0.025, 0.12), 2, at(0.24)));
            v.push((cyl(0.009, 0.2), 2, at(-0.25)));
            rot_z = -PI / 3.0;
        }
        "balm" => {
            v.push((cyl(0.13, 0.14), 1, at(0.0)));
            v.push((cyl(0.14, 0.035), 2, at(0.09)));
        }
        "tablet" => {
            v.push((Sphere::new(0.1).mesh().uv(8, 6), 1, at(0.0).with_scale(Vec3::new(1.0, 1.8, 1.0))));
            v.push((cyl(0.1, 0.05), 0, at(0.0)));
        }
        _ => {
            v.push((cyl(0.1, 0.24), 1, at(0.0)));
            v.push((cyl(0.045, 0.1), 0, at(0.16)));
            match kind {
                "mist" => {
                    v.push((Cuboid::new(0.16, 0.05, 0.07).mesh().build(), 2, Transform::from_xyz(0.045, 0.23, 0.0)));
                    v.push((Cuboid::new(0.035, 0.1, 0.035).mesh().build(), 2, Transform::from_xyz(0.06, 0.16, 0.0)));
                }
                // `TorusGeometry(.19, .018, 4, 16)` deitado: o toro do Bevy já nasce no plano XZ
                "ritual" => v.push((
                    Torus::new(0.19 - 0.018, 0.19 + 0.018).mesh().minor_resolution(4).major_resolution(16).build(),
                    2,
                    at(0.0),
                )),
                _ => v.push((cyl(0.05, 0.025), 2, at(0.23))),
            }
        }
    }
    (v, rot_z)
}

#[allow(clippy::too_many_arguments)]
fn spawn_auras(
    mut commands: Commands,
    mut msgs: MessageReader<MedAura>,
    me: Query<Entity, With<Me>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ToonMaterial>>,
    world: Option<Res<CurrentWorld>>,
    auras: Query<(Entity, &Aura)>,
) {
    let mut live: Vec<(Entity, f32)> = auras.iter().map(|(e, a)| (e, a.time)).collect();
    for m in msgs.read() {
        let Some(on) = (match m.on {
            At::Me => me.single().ok(),
            At::Actor(e) => Some(e),
            At::Xz(_) => None,
        }) else {
            continue;
        };
        let Some(world) = world.as_deref() else { continue };
        // no máximo 8 de uma vez: a mais antiga sai
        if live.len() >= 8 {
            live.sort_by(|a, b| b.1.total_cmp(&a.1));
            if let Some((old, _)) = live.first().copied() {
                commands.entity(old).despawn();
                live.remove(0);
            }
        }
        let m3 = [raw(0xe5f7ff), raw(m.color), raw(m.accent)].map(|c| mats.add(basic(c, world)));
        let (parts, rot_z) = prop_parts(&m.kind);
        let prop = commands
            .spawn((
                Transform::from_rotation(Quat::from_rotation_z(rot_z)),
                Visibility::Hidden,
                NotShadowCaster,
                DespawnOnExit(Screen::Playing),
            ))
            .with_children(|p| {
                for (mesh, mat, t) in parts {
                    p.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(m3[mat].clone()), t, NotShadowCaster));
                }
            })
            .id();
        let count = 30;
        let seeds = (0..count).map(|i| (i as f32 / count as f32 * TAU, (i % 10) as f32 / 10.0 * 1.6)).collect();
        let c = raw(m.color);
        let size = if m.kind == "mist" { 0.16 } else { 0.1 };
        let root = commands
            .spawn((
                Aura { on, kind: m.kind.clone(), craft: m.craft, time: 0.0, seeds, prop },
                Node {
                    position_type: PositionType::Absolute,
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .id();
        let dots: Vec<Entity> = (0..count)
            .map(|j| {
                commands
                    .spawn((
                        AuraDot(j),
                        Node { position_type: PositionType::Absolute, ..default() },
                        Pickable::IGNORE,
                        BackgroundColor(Color::srgba(c.red, c.green, c.blue, 0.0)),
                        AuraSize(size),
                    ))
                    .id()
            })
            .collect();
        commands.entity(root).add_children(&dots);
        live.push((root, 0.0));
    }
}

/// Tamanho do ponto em metros (`PointsMaterial.size`).
#[derive(Component, Debug, Clone, Copy)]
struct AuraSize(f32);

/// Posição local do ponto `j` da aura no tempo `t` (`healthTick`).
fn aura_point(kind: &str, seed: (f32, f32), j: usize, t: f32) -> Vec3 {
    let (mut angle, mut radius, mut y) = (seed.0, 0.5, seed.1 + t * 0.4);
    match kind {
        "drink" => {
            angle += t * 2.0;
            radius = 0.3 + t * 0.15;
            y = (seed.1 + t * 0.8) % 2.2;
        }
        "balm" => {
            angle -= t;
            radius = 0.55 + (t * 3.0 + seed.0).sin() * 0.12;
            y = seed.1 + t * 0.25;
        }
        "injection" => {
            angle += t * 3.0 + seed.1 * 3.0;
            radius = 0.38 + t * 0.1;
            y = seed.1 + t * 0.35;
        }
        "mist" => {
            radius = 0.25 + t * 0.45;
            y = 0.9 + seed.0.sin() * 0.5 + t * 0.15;
        }
        "tablet" => {
            radius = 0.2 + t * 0.7;
            y = 1.3 + (seed.0 * 2.0).sin() * (0.2 + t * 0.3);
        }
        "ritual" => {
            angle += t * if j % 2 == 1 { 1.0 } else { -1.0 };
            radius = 0.45 + (j % 3) as f32 * 0.2 + t * 0.08;
            y = 0.12 + (j % 3) as f32 * 0.55;
        }
        _ => {}
    }
    Vec3::new(angle.cos() * radius, y, angle.sin() * radius)
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn tick_auras(
    mut commands: Commands,
    time: Res<Time>,
    ui: Res<UiScale>,
    cam: Query<(&Camera, &GlobalTransform, &Projection), With<Camera3d>>,
    tf: Query<&GlobalTransform>,
    mut auras: Query<(Entity, &mut Aura, &Children)>,
    mut dots: Query<(&AuraDot, &AuraSize, &mut Node, &mut BackgroundColor)>,
    mut props: Query<(&mut Transform, &mut Visibility)>,
) {
    let Ok((camera, cam_t, proj)) = cam.single() else { return };
    let half = match proj {
        Projection::Perspective(p) => (p.fov / 2.0).tan(),
        _ => 0.577,
    };
    for (e, mut au, kids) in &mut auras {
        au.time += time.delta_secs();
        let dur = if au.craft { 1.6 } else { 2.8 };
        let Ok(at) = tf.get(au.on).map(GlobalTransform::translation) else {
            commands.entity(au.prop).despawn();
            commands.entity(e).despawn();
            continue;
        };
        if au.time > dur {
            commands.entity(au.prop).despawn();
            commands.entity(e).despawn();
            continue;
        }
        let t = au.time;
        let kind = if au.craft { "ritual" } else { au.kind.as_str() };
        if let Ok((mut pt, mut pv)) = props.get_mut(au.prop) {
            pt.translation = at + Vec3::new(0.43, 1.3, 0.15);
            *pv = if t < if au.craft { 1.4 } else { 1.1 } { Visibility::Inherited } else { Visibility::Hidden };
            match kind {
                "injection" => pt.translation.x += (t * 0.2).min(0.1),
                "drink" => pt.rotation = Quat::from_rotation_z(-((t.min(1.0)) * PI).sin() * 0.7),
                "balm" => pt.translation.x += (t * 9.0).sin() * 0.06,
                "ritual" => pt.rotation = Quat::from_rotation_y(t * 2.0),
                _ => {}
            }
        }
        let alpha = 0.9 * (1.0 - t / dur);
        for kid in kids.iter() {
            let Ok((d, size, mut n, mut bg)) = dots.get_mut(kid) else { continue };
            let p = at + aura_point(kind, au.seeds[d.0], d.0, t);
            match scale_at(camera, cam_t, p, ui.0) {
                Some((v, ppm)) => {
                    let px = (size.0 * ppm * half).max(1.0);
                    n.width = Val::Px(px);
                    n.height = Val::Px(px);
                    n.left = Val::Px(v.x - px / 2.0);
                    n.top = Val::Px(v.y - px / 2.0);
                    bg.0.set_alpha(alpha);
                }
                None => bg.0.set_alpha(0.0),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_follow_show_tick() {
        // aparece em 0,45 s, fica inteiro e some nos últimos 0,7 s
        let (a, out, k) = show_curve(0.225, 4.0);
        assert!((a - 0.5).abs() < 1e-6 && out == 0.0 && (k - 0.5).abs() < 1e-6);
        assert_eq!(show_curve(2.0, 4.0).2, 1.0);
        assert!((show_curve(3.65, 4.0).2 - 0.5).abs() < 1e-5);
        // faíscas: anel de raio s·(0,5 ± 0,12), altura entre −0,35 e 0,45
        for j in 0..SPARKS {
            let p = spark_pos(j, 1.3, 1.0);
            let r = p.xz().length();
            assert!((0.38 - 1e-5..=0.62 + 1e-5).contains(&r) && (-0.35..0.45).contains(&p.y));
        }
        assert!((spark_alpha(0.0) - 1.0).abs() < 1e-6 && (spark_alpha(0.35) - 0.55).abs() < 1e-6);
        assert_eq!(spark_alpha(1.0), 0.0);
    }

    #[test]
    fn aura_points_and_props() {
        // ritual: três anéis em 0,12, 0,67 e 1,22 m
        let p = aura_point("ritual", (0.0, 0.0), 2, 0.0);
        assert!((p.y - 1.22).abs() < 1e-6 && (p.x - 0.85).abs() < 1e-6);
        // bebida: sobe e volta ao pé depois de 2,2 m
        assert!((aura_point("drink", (0.0, 1.6), 0, 1.0).y - 0.2).abs() < 1e-5);
        assert_eq!(prop_parts("injection").0.len(), 4);
        assert!((prop_parts("injection").1 + PI / 3.0).abs() < 1e-6);
        assert_eq!(prop_parts("ritual").0.len(), 3);
        assert_eq!(prop_parts("mist").0.len(), 4);
        assert_eq!(raw(0xff8000), LinearRgba::rgb(1.0, 128.0 / 255.0, 0.0));
    }

    #[test]
    fn med_pose_shapes() {
        let mut p = crate::anim::Pose::rest(0.25);
        let hd = p.hd.x;
        let mut m = MedPose::new("ouch", "injection");
        m.age = 0.75;
        m.apply(&mut p);
        assert_eq!(p.mouth, 2);
        assert!((p.hd.x - (hd - 0.25)).abs() < 1e-6);
    }
}
