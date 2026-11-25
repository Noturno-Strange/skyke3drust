//! Visual de cada avatar a partir do `cfg`.
//!
//! Hoje: rosto, pele e olhos (textura da cabeça gerada por `sky-avatars`, idêntica à do navegador) e as cores de pele,
//! camisa, calça e calçado (as cores do avatar de referência exportado são trocadas pelas do `cfg`). O formato do
//! cabelo, o chapéu, bermuda e bota ainda vêm do avatar de referência até a geração dessas peças ser portada.

use bevy::asset::RenderAssetUsages;
use bevy::gltf::GltfExtras;
use bevy::mesh::VertexAttributeValues;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sky_avatars::hair::{HairShape, paint};
use sky_avatars::{Cfg, Palette, Palettes, head};
use sky_rendering::{Kind, MaterialDesc, Ramp, ToonMaterial};
use sky_world::CurrentWorld;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

#[derive(Debug)]
pub struct LookPlugin {
    pub assets_dir: std::path::PathBuf,
}

impl Plugin for LookPlugin {
    fn build(&self, app: &mut App) {
        match AvatarKit::load(&self.assets_dir) {
            Ok(kit) => {
                app.insert_resource(kit).add_systems(Update, apply_looks);
            }
            Err(e) => error!("avatares: {e}"),
        }
    }
}

/// Paletas do catálogo e o visual do avatar de referência (`assets/avatars/default.json`).
#[derive(Resource, Debug)]
pub struct AvatarKit {
    pub palettes: Palettes,
    pub reference: Cfg,
    /// Altura do corpo acima dos pés no avatar de referência (`userData.fy`).
    pub fy: f32,
    hair_dir: std::path::PathBuf,
    shapes: std::sync::Mutex<HashMap<String, Option<Arc<HairShape>>>>,
}

impl AvatarKit {
    fn load(assets: &std::path::Path) -> Result<Self, String> {
        let cat = assets.join("../data/catalogs/catalog.json");
        let cat = if cat.exists() { cat } else { assets.join("catalog.json") };
        let palettes =
            Palettes::from_catalog_json(&std::fs::read(&cat).map_err(|e| format!("{}: {e}", cat.display()))?)
                .map_err(|e| e.to_string())?;
        #[derive(serde::Deserialize)]
        struct Def {
            cfg: BTreeMap<String, serde_json::Value>,
            #[serde(default)]
            fy: Option<f32>,
        }
        let p = assets.join("avatars/default.json");
        let d: Def = serde_json::from_slice(&std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?)
            .map_err(|e| e.to_string())?;
        let fy = d.fy.unwrap_or(0.247);
        let raw: BTreeMap<String, u32> =
            d.cfg.into_iter().filter_map(|(k, v)| v.as_u64().map(|n| (k, n as u32))).collect();
        let reference = palettes.clean(&raw).ok_or("visual de referência inválido")?;
        Ok(Self { palettes, reference, fy, hair_dir: assets.join("avatars/hair"), shapes: Default::default() })
    }

    /// Geometria de um estilo de cabelo (`hs-NN`) ou chapéu (`hat-N`), lida do disco uma vez.
    fn shape(&self, name: &str) -> Option<Arc<HairShape>> {
        let mut cache = self.shapes.lock().ok()?;
        cache
            .entry(name.to_owned())
            .or_insert_with(|| {
                let p = self.hair_dir.join(format!("{name}.bin"));
                match std::fs::read(&p)
                    .map_err(|e| e.to_string())
                    .and_then(|b| HairShape::parse(&b).map_err(|e| e.to_string()))
                {
                    Ok(s) => Some(Arc::new(s)),
                    Err(e) => {
                        error!("{}: {e}", p.display());
                        None
                    }
                }
            })
            .clone()
    }

    /// Visual como mapa, para enviar ao servidor.
    pub fn reference_map(&self) -> BTreeMap<String, u32> {
        self.reference.0.clone()
    }
}

/// Visual pedido para um avatar.
#[derive(Component, Debug, Clone)]
pub struct AvatarLook(pub Cfg);

/// Visual já aplicado (para refazer quando o `cfg` muda).
#[derive(Component, Debug, Clone)]
struct LookApplied(Cfg);

/// Malha original (antes de trocar as cores), para refazer a troca a partir dela.
#[derive(Component, Debug, Clone)]
struct OriginalMesh(Handle<Mesh>);

/// Cabelo, chapéu e contorno gerados para o visual (refeitos quando ele muda).
#[derive(Component, Debug)]
struct HairPart;

fn tri_mesh(pos: &[f32], nrm: &[f32], colors: Option<&[f32]>) -> Mesh {
    let v3 = |a: &[f32]| a.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect::<Vec<[f32; 3]>>();
    let n = pos.len() / 3;
    let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, v3(pos))
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, v3(nrm));
    if let Some(c) = colors {
        m.insert_attribute(
            Mesh::ATTRIBUTE_COLOR,
            c.chunks_exact(3).map(|c| [c[0], c[1], c[2], 1.0]).collect::<Vec<[f32; 4]>>(),
        );
    }
    m.insert_indices(Indices::U32((0..n as u32).collect()));
    m
}

fn desc(kind: Kind, flat: bool, double: bool, vertex_colors: bool) -> MaterialDesc {
    MaterialDesc {
        kind,
        color: LinearRgba::WHITE,
        emissive: LinearRgba::BLACK,
        texture: None,
        uv_scale: Vec2::ONE,
        uv_offset: Vec2::ZERO,
        flat,
        double_sided: double,
        back_side: false,
        transparent: false,
        alpha_cutoff: None,
        depth_write: true,
        fog: true,
        vertex_colors,
        ground_avg: None,
        additive: false,
    }
}

/// Material original (antes do rosto ou da cor do visual).
#[derive(Component, Debug, Clone)]
struct OriginalMat(Handle<ToonMaterial>);

/// Cor de vértice do avatar: o `tubeG`/`bandG` do original grava o hex convertido para linear (valores medidos no GLB
/// exportado), diferente das cores de luz, que entram cruas.
fn vertex_color(hex: &str) -> [f32; 3] {
    sky_avatars::cfg::hex_to_linear(hex).map(|v| v as f32)
}

/// Pares (cor do avatar de referência → cor do visual pedido) para pele, camisa, calça e calçado.
fn color_map(kit: &AvatarKit, to: &Cfg) -> Vec<([f32; 3], [f32; 3])> {
    let p = &kit.palettes;
    let from = &kit.reference;
    let part = |pal: Palette, k: &str, g: &str| {
        (vertex_color(p.color(pal, from.get(k), from.flag(g))), vertex_color(p.color(pal, to.get(k), to.flag(g))))
    };
    vec![
        part(Palette::Skin, "skin", "tb"),
        part(Palette::Pal, "shirt", "sb"),
        part(Palette::Pal, "pc", "pb"),
        part(Palette::Pal, "sc", "cb"),
    ]
}

fn close(a: [f32; 3], b: [f32; 3]) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 0.002)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_looks(
    mut commands: Commands,
    kit: Res<AvatarKit>,
    looks: Query<(Entity, &AvatarLook, Option<&LookApplied>)>,
    children: Query<&Children>,
    extras: Query<&GltfExtras>,
    mesh_q: Query<(&Mesh3d, Option<&OriginalMesh>, Option<&MeshMaterial3d<ToonMaterial>>, Option<&OriginalMat>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ToonMaterial>>,
    mut images: ResMut<Assets<Image>>,
    parents: Query<&ChildOf>,
    old_parts: Query<Entity, With<HairPart>>,
    world: Option<Res<CurrentWorld>>,
) {
    let Some(world) = world else { return };
    for (e, look, applied) in &looks {
        if applied.is_some_and(|a| a.0 == look.0) {
            continue;
        }
        let all: Vec<Entity> = children.iter_descendants(e).collect();
        // espera a cena do avatar existir e os materiais toon serem aplicados
        let ready = all.iter().any(|c| mesh_q.get(*c).is_ok_and(|(_, _, m, _)| m.is_some()));
        if !ready {
            continue;
        }
        let map = color_map(&kit, &look.0);
        // cabeça: o nó com `head: 1` nos extras; a malha é ele ou um filho
        let head_nodes: Vec<Entity> =
            all.iter().copied().filter(|c| extras.get(*c).is_ok_and(|x| x.value.contains("\"head\":1"))).collect();
        let mut head_meshes = Vec::new();
        for h in &head_nodes {
            head_meshes.push(*h);
            head_meshes.extend(children.iter_descendants(*h));
        }
        let face = head::head_texture(&kit.palettes, &look.0, 0);
        let img = images.add(Image::new(
            Extent3d { width: head::FW as u32, height: head::FH as u32, depth_or_array_layers: 1 },
            TextureDimension::D2,
            face,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::default(),
        ));
        for c in &all {
            let Ok((m3, orig, cur_mat, orig_mat)) = mesh_q.get(*c) else { continue };
            // sempre a partir do material original (o visual pode mudar várias vezes)
            let mat = orig_mat.map(|o| o.0.clone()).or_else(|| cur_mat.map(|m| m.0.clone()));
            if let (Some(m), None) = (&mat, orig_mat) {
                commands.entity(*c).insert(OriginalMat(m.clone()));
            }
            let mat = mat.as_ref();
            if head_meshes.contains(c)
                && let Some(mat) = mat
                && let Some(base) = mats.get(mat).cloned()
            {
                let mut m = base;
                m.base_texture = Some(img.clone());
                m.params.flags |= sky_rendering::flags::TEXTURE;
                commands.entity(*c).insert(MeshMaterial3d(mats.add(m)));
                continue;
            }
            // orelhas e pés usam cor de material (M(cor) no original), não cor de vértice
            if let Some(mat) = mat
                && let Some(base) = mats.get(mat)
            {
                let rgb = [base.params.color.x, base.params.color.y, base.params.color.z];
                let m = match map.iter().find(|(from, _)| close(*from, rgb)) {
                    Some((_, to)) => {
                        let mut m = base.clone();
                        m.params.color = Vec4::new(to[0], to[1], to[2], base.params.color.w);
                        mats.add(m)
                    }
                    None => mat.clone(),
                };
                commands.entity(*c).insert(MeshMaterial3d(m));
            }
            let src = orig.map_or_else(|| m3.0.clone(), |o| o.0.clone());
            let Some(mesh) = meshes.get(&src) else { continue };
            if !mesh.contains_attribute(Mesh::ATTRIBUTE_COLOR) {
                continue;
            }
            let mut mesh = mesh.clone();
            let swap = |col: &mut [f32]| {
                let rgb = [col[0], col[1], col[2]];
                if let Some((_, to)) = map.iter().find(|(from, _)| close(*from, rgb)) {
                    col[..3].copy_from_slice(to);
                }
            };
            match mesh.attribute_mut(Mesh::ATTRIBUTE_COLOR) {
                Some(VertexAttributeValues::Float32x4(cols)) => cols.iter_mut().for_each(|c| swap(c)),
                Some(VertexAttributeValues::Float32x3(cols)) => cols.iter_mut().for_each(|c| swap(c)),
                _ => {}
            }
            let mut ec = commands.entity(*c);
            if orig.is_none() {
                ec.insert(OriginalMesh(src));
            }
            ec.insert(Mesh3d(meshes.add(mesh)));
        }
        // cabelo ou chapéu do visual, no grupo da cabeça (o pai do nó da cabeça), no lugar dos do avatar de referência
        for c in &all {
            if extras.get(*c).is_ok_and(|x| x.value.contains("\"part\":\"hair") || x.value.contains("\"part\":\"hat\""))
            {
                commands.entity(*c).insert(Visibility::Hidden);
            }
            if old_parts.contains(*c) {
                commands.entity(*c).despawn();
            }
        }
        if let Some(hd) = head_nodes.first().and_then(|h| parents.get(*h).ok()).map(ChildOf::parent) {
            let hat = look.0.get("hat");
            let (name, w, flat, double) = if hat > 0 {
                (format!("hat-{hat}"), 0.011, false, false)
            } else {
                (format!("hs-{:02}", look.0.get("hs")), 0.034, true, true)
            };
            if let Some(shape) = kit.shape(&name) {
                let colors = paint(&shape, &kit.palettes, &look.0);
                let mesh = meshes.add(tri_mesh(&shape.positions, &shape.normals, Some(&colors)));
                let mat =
                    mats.add(ToonMaterial::new(&desc(Kind::Toon(Ramp::Avatar), flat, double, true), &world.lighting));
                let ol_pos: Vec<f32> =
                    shape.lo_positions.iter().zip(&shape.lo_offsets).map(|(p, o)| p + o * w as f32).collect();
                let ol_mesh = meshes.add(tri_mesh(&ol_pos, &shape.lo_offsets, None));
                let ol_mat = mats.add(ToonMaterial::new(&desc(Kind::Outline, false, false, false), &world.lighting));
                commands.entity(hd).with_children(|p| {
                    p.spawn((HairPart, Mesh3d(mesh), MeshMaterial3d(mat), Transform::IDENTITY));
                    p.spawn((
                        HairPart,
                        Mesh3d(ol_mesh),
                        MeshMaterial3d(ol_mat),
                        Transform::IDENTITY,
                        bevy::light::NotShadowCaster,
                    ));
                });
            }
        }
        commands.entity(e).insert(LookApplied(look.0.clone()));
    }
}
