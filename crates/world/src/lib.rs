//! Mundos do Skyke3D no Bevy.
//!
//! Cada mundo vem de `assets/worlds/<id>/` (gerado por `tools/asset-exporter` a partir do jogo original):
//! `scene.glb` (cenário), `sky.glb` (céu, planetas, montanhas) e `world.json` ([`meta::WorldMeta`]).
//! Ao carregar, os materiais glTF são trocados pelo [`ToonMaterial`] conforme `extras.sky` de cada material.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod meta;

use bevy::gltf::{GltfAssetLabel, GltfMaterialExtras};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use serde::Deserialize;
use sky_rendering::{Kind, MaterialDesc, MipQueue, Ramp, ToonLighting, ToonMaterial, ToonPlugin};
use sky_shared::World;
use std::collections::HashMap;
use std::path::PathBuf;

pub use meta::WorldMeta;

#[derive(Debug)]
pub struct WorldPlugin {
    /// Pasta `assets/` (a mesma do `AssetPlugin`), para ler o `world.json` sem esperar o carregador de assets.
    pub assets_dir: PathBuf,
}

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ToonPlugin>() {
            app.add_plugins(ToonPlugin);
        }
        app.insert_resource(AssetsDir(self.assets_dir.clone()))
            .init_resource::<MaterialCache>()
            .add_message::<LoadWorld>()
            .add_systems(Update, (load_world, convert_materials).chain())
            .add_systems(PostUpdate, face_camera.before(bevy::transform::TransformSystems::Propagate));
    }
}

#[derive(Resource, Debug)]
struct AssetsDir(PathBuf);

/// Pede a troca do mundo.
#[derive(Message, Debug, Clone, Copy)]
pub struct LoadWorld(pub World);

/// Mundo atual: metadados e iluminação.
#[derive(Resource, Debug, Clone)]
pub struct CurrentWorld {
    pub world: World,
    pub meta: WorldMeta,
    pub lighting: ToonLighting,
}

/// Marca as entidades raiz de um mundo (cenário e céu), para remover ao trocar.
#[derive(Component, Debug)]
pub struct WorldPart;

#[derive(Resource, Default, Debug)]
struct MaterialCache(HashMap<(AssetId<StandardMaterial>, bool), Handle<ToonMaterial>>);

#[allow(clippy::too_many_arguments)]
fn load_world(
    mut msgs: MessageReader<LoadWorld>,
    mut commands: Commands,
    dir: Res<AssetsDir>,
    assets: Res<AssetServer>,
    old: Query<Entity, With<WorldPart>>,
    mut cache: ResMut<MaterialCache>,
    mut clear: ResMut<ClearColor>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut toon_mats: ResMut<Assets<ToonMaterial>>,
) {
    let Some(LoadWorld(w)) = msgs.read().last().copied() else { return };
    let path = dir.0.join("worlds").join(w.id()).join("world.json");
    let meta = match std::fs::read(&path)
        .map_err(|e| e.to_string())
        .and_then(|b| WorldMeta::parse(&b).map_err(|e| e.to_string()))
    {
        Ok(m) => m,
        Err(e) => {
            error!("mundo {w}: {} ({e})", path.display());
            return;
        }
    };
    for e in &old {
        commands.entity(e).despawn();
    }
    cache.0.clear();
    let lighting = meta.lighting();
    if let Some((c, _, _)) = lighting.fog {
        // a neblina some no horizonte com a mesma cor do fundo (valores já codificados em sRGB, como no three.js)
        clear.0 = Color::srgb(c.x, c.y, c.z);
    }
    for part in ["scene", "sky"] {
        let h = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("worlds/{}/{part}.glb", w.id())));
        commands.spawn((WorldAssetRoot(h), WorldPart, Name::new(format!("{w}/{part}"))));
    }
    spawn_sprites(&mut commands, &assets, &mut meshes, &mut toon_mats, w, &meta, &lighting);
    info!("mundo {w} ({}): {} assentos, {} sprites", w.biome(), meta.seats.len(), meta.sprites.len());
    commands.insert_resource(CurrentWorld { world: w, meta, lighting });
}

#[derive(Deserialize, Default)]
struct Extras {
    #[serde(default)]
    sky: SkyExtras,
}

#[derive(Deserialize)]
struct SkyExtras {
    #[serde(default = "toon")]
    kind: String,
    #[serde(default)]
    ramp: String,
    #[serde(default)]
    flat: bool,
    #[serde(default)]
    side: String,
    #[serde(default)]
    transparent: bool,
    #[serde(default = "yes", rename = "depthWrite")]
    depth_write: bool,
    #[serde(default = "yes")]
    fog: bool,
    #[serde(default, rename = "vertexColors")]
    vertex_colors: bool,
    #[serde(default, rename = "groundAvg")]
    ground_avg: Option<[f32; 3]>,
    #[serde(default)]
    hor: Option<[f32; 3]>,
    #[serde(default)]
    mid: Option<[f32; 3]>,
    #[serde(default)]
    top: Option<[f32; 3]>,
}

fn toon() -> String {
    "toon".into()
}
fn yes() -> bool {
    true
}
impl Default for SkyExtras {
    fn default() -> Self {
        serde_json::from_str("{}").unwrap_or_else(|_| unreachable!())
    }
}

/// Troca StandardMaterial (do carregador glTF) pelo material toon do Skyke3D.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn convert_materials(
    mut commands: Commands,
    world: Option<Res<CurrentWorld>>,
    q: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>, Option<&GltfMaterialExtras>, &Mesh3d),
        Added<MeshMaterial3d<StandardMaterial>>,
    >,
    std_mats: Res<Assets<StandardMaterial>>,
    meshes: Res<Assets<Mesh>>,
    mut toon_mats: ResMut<Assets<ToonMaterial>>,
    mut cache: ResMut<MaterialCache>,
    mut mips: ResMut<MipQueue>,
) {
    let Some(world) = world else { return };
    for (e, sm, extras, mesh) in &q {
        let has_color = meshes.get(&mesh.0).is_some_and(|m| m.contains_attribute(Mesh::ATTRIBUTE_COLOR));
        let key = (sm.0.id(), has_color);
        let handle = if let Some(h) = cache.0.get(&key) {
            h.clone()
        } else {
            let Some(s) = std_mats.get(&sm.0) else { continue };
            let x: Extras = extras.and_then(|x| serde_json::from_str(&x.value).ok()).unwrap_or_default();
            let x = x.sky;
            let kind = match x.kind.as_str() {
                "toon" | "standard" => Kind::Toon(if x.ramp == "avatar" { Ramp::Avatar } else { Ramp::Scene }),
                "outline" => Kind::Outline,
                "sky-dome" => Kind::SkyDome {
                    hor: Vec3::from(x.hor.unwrap_or_default()),
                    mid: Vec3::from(x.mid.unwrap_or_default()),
                    top: Vec3::from(x.top.unwrap_or_default()),
                },
                _ => Kind::Basic,
            };
            let kind = if s.unlit && matches!(kind, Kind::Toon(_)) { Kind::Basic } else { kind };
            let d = MaterialDesc {
                kind,
                color: s.base_color.to_linear(),
                emissive: s.emissive,
                texture: s.base_color_texture.clone(),
                uv_scale: Vec2::new(s.uv_transform.matrix2.x_axis.x, s.uv_transform.matrix2.y_axis.y),
                uv_offset: s.uv_transform.translation,
                flat: x.flat,
                double_sided: x.side == "double" || s.double_sided,
                back_side: x.side == "back",
                transparent: x.transparent || matches!(s.alpha_mode, AlphaMode::Blend),
                alpha_cutoff: if let AlphaMode::Mask(c) = s.alpha_mode { Some(c) } else { None },
                depth_write: x.depth_write,
                fog: x.fog,
                vertex_colors: x.vertex_colors && has_color,
                ground_avg: x.ground_avg.map(Vec3::from),
            };
            if let Some(t) = &d.texture {
                mips.request(t.id());
            }
            let h = toon_mats.add(ToonMaterial::new(&d, &world.lighting));
            cache.0.insert(key, h.clone());
            h
        };
        commands.entity(e).remove::<MeshMaterial3d<StandardMaterial>>().insert(MeshMaterial3d(handle));
    }
}

/// Quad que sempre olha para a câmera, como `THREE.Sprite` (com giro próprio em torno do eixo de visão).
#[derive(Component, Debug)]
pub struct Billboard {
    pub rotation: f32,
}

fn spawn_sprites(
    commands: &mut Commands,
    assets: &AssetServer,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<ToonMaterial>,
    w: World,
    meta: &WorldMeta,
    lighting: &ToonLighting,
) {
    if meta.sprites.is_empty() {
        return;
    }
    let mut cache: HashMap<i32, Handle<Image>> = HashMap::new();
    for s in &meta.sprites {
        let texture = (s.tex >= 0).then(|| {
            cache.entry(s.tex).or_insert_with(|| assets.load(format!("worlds/{}/sprite-{}.png", w.id(), s.tex))).clone()
        });
        let c = meta::lin(&s.color);
        let d = MaterialDesc {
            kind: Kind::Basic,
            color: LinearRgba::new(c.x, c.y, c.z, s.opacity),
            emissive: LinearRgba::BLACK,
            texture,
            uv_scale: Vec2::ONE,
            uv_offset: Vec2::ZERO,
            flat: false,
            double_sided: true,
            back_side: false,
            transparent: true,
            alpha_cutoff: None,
            depth_write: false,
            fog: s.fog,
            vertex_colors: false,
            ground_avg: None,
        };
        let mut m = ToonMaterial::new(&d, lighting);
        if s.additive {
            m.alpha_mode = AlphaMode::Add;
        }
        // o centro do sprite pode não ser o meio do quad
        let off = Vec2::new(0.5 - s.center[0], 0.5 - s.center[1]);
        let mesh = meshes.add(Rectangle::new(1.0, 1.0).mesh().build().translated_by(off.extend(0.0)));
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mats.add(m)),
            Transform::from_translation(Vec3::from(s.pos)).with_scale(Vec3::new(s.scale[0], s.scale[1], 1.0)),
            Billboard { rotation: s.rotation },
            WorldPart,
            Name::new(format!("{w}/sprite")),
        ));
    }
}

fn face_camera(cam: Query<&GlobalTransform, With<Camera3d>>, mut q: Query<(&mut Transform, &Billboard)>) {
    let Ok(cam) = cam.single() else { return };
    let (_, rot, _) = cam.to_scale_rotation_translation();
    for (mut t, b) in &mut q {
        t.rotation = rot * Quat::from_rotation_z(b.rotation);
    }
}
