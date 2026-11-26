//! Visual de cada avatar a partir do `cfg`.
//!
//! Tudo vem do `cfg`, gerado por `sky-avatars` como no navegador: rosto (textura da cabeça, que troca com a boca),
//! cabelo ou chapéu, tronco, braços, pernas e pés (bermuda e bota), as cores brilhantes e os contornos escuros
//! (casco invertido de 0,026 m). Do avatar de referência exportado ficam só a hierarquia de juntas, a cabeça e as
//! orelhas (com a cor da pele trocada).

use bevy::asset::RenderAssetUsages;
use bevy::gltf::GltfExtras;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use sky_avatars::hair::{HairShape, paint};
use sky_avatars::{Cfg, Palette, Palettes, body, head};
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
                app.insert_resource(crate::editor::MyLook::load(&kit));
                app.insert_resource(kit).add_systems(Update, (apply_looks, mouths.after(apply_looks)));
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

/// Peças geradas para o visual (corpo, cabelo, chapéu, contornos), refeitas quando ele muda.
#[derive(Component, Debug)]
struct LookPart;

/// Os três rostos do visual (boca normal, alta e grito) e as malhas da cabeça que os usam.
#[derive(Component, Debug)]
struct HeadFaces {
    meshes: Vec<Entity>,
    mats: [Handle<ToonMaterial>; 3],
    cur: u8,
}

/// Largura do contorno do avatar (`OLB`: `w = 0,026`, no espaço da câmera).
const OUTLINE_W: f32 = 0.026;
/// Força do brilho (`EMIG` no corpo e chapéus, `EMIGH` no cabelo).
const GLOSS: f32 = 0.9;
const GLOSS_HAIR: f32 = 0.35;

/// Casco do contorno: cada vértice sai `w` metros ao longo da normal no espaço do mundo. `s` é a escala acumulada do
/// nó (o original desloca no espaço da câmera, sem escala; aqui a malha fica no nó, então o deslocamento é desfeito
/// da escala: `w · S⁻¹ · normalize(S⁻¹ n)`).
fn outline_mesh(pos: &[f32], nrm: &[f32], s: Vec3, w: f32) -> Mesh {
    let mut out = Vec::with_capacity(pos.len());
    for (p, n) in pos.chunks_exact(3).zip(nrm.chunks_exact(3)) {
        let d = (Vec3::new(n[0], n[1], n[2]) / s).normalize_or_zero() / s * w;
        out.extend_from_slice(&[p[0] + d.x, p[1] + d.y, p[2] + d.z]);
    }
    tri_mesh(&out, nrm, None)
}

/// `SphereGeometry(1, w, h)` com os triângulos expandidos (posições e normais).
fn sphere_tris(w: usize, h: usize) -> (Vec<f32>, Vec<f32>) {
    let (p, n, idx) = body::sphere(w, h);
    let (mut pos, mut nrm) = (Vec::with_capacity(idx.len() * 3), Vec::with_capacity(idx.len() * 3));
    for i in idx {
        let i = i as usize * 3;
        pos.extend_from_slice(&p[i..i + 3]);
        nrm.extend_from_slice(&n[i..i + 3]);
    }
    (pos, nrm)
}

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

/// Junta mais próxima acima de uma malha do avatar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Joint {
    Torso,
    Arm,
    Thigh,
    Shin,
    Foot,
    Head,
    Other,
}

fn joint_of(name: &str) -> Option<Joint> {
    Some(match name {
        "fig" => Joint::Torso,
        "armL" | "armR" => Joint::Arm,
        "lgL" | "lgR" => Joint::Thigh,
        "knL" | "knR" => Joint::Shin,
        "ftL" | "ftR" => Joint::Foot,
        "hd" => Joint::Head,
        "hold" => Joint::Other,
        _ => return None,
    })
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn apply_looks(
    mut commands: Commands,
    kit: Res<AvatarKit>,
    looks: Query<(Entity, &AvatarLook, Option<&LookApplied>)>,
    children: Query<&Children>,
    extras: Query<&GltfExtras>,
    mesh_q: Query<(&Mesh3d, Option<&MeshMaterial3d<ToonMaterial>>, Option<&OriginalMat>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ToonMaterial>>,
    mut images: ResMut<Assets<Image>>,
    parents: Query<&ChildOf>,
    old_parts: Query<Entity, With<LookPart>>,
    names: Query<&Name>,
    transforms: Query<&Transform>,
    mut rigs: Query<&mut crate::anim::Rig>,
    world: Option<Res<CurrentWorld>>,
) {
    let Some(world) = world else { return };
    for (e, look, applied) in &looks {
        if applied.is_some_and(|a| a.0 == look.0) {
            continue;
        }
        let all: Vec<Entity> = children.iter_descendants(e).collect();
        // espera a cena do avatar existir e os materiais toon serem aplicados
        let ready = all.iter().any(|c| mesh_q.get(*c).is_ok_and(|(_, m, _)| m.is_some()));
        if !ready {
            continue;
        }
        let cfg = &look.0;
        let map = color_map(&kit, cfg);
        let b = body::body(&kit.palettes, cfg);
        let joint = |mut x: Entity| -> Option<Joint> {
            while x != e {
                x = parents.get(x).ok()?.parent();
                if let Some(j) = names.get(x).ok().and_then(|n| joint_of(n.as_str())) {
                    return Some(j);
                }
            }
            None
        };
        let named = |n: &str| -> Vec<Entity> {
            all.iter().copied().filter(|c| names.get(*c).is_ok_and(|x| x.as_str() == n)).collect()
        };
        let scale_of = |x: Entity| transforms.get(x).map_or(Vec3::ONE, |t| t.scale);
        let ol_mat = mats.add(ToonMaterial::new(&desc(Kind::Outline, false, false, false), &world.lighting));
        let spawn_outline = |cmd: &mut Commands, meshes: &mut Assets<Mesh>, at: Entity, mesh: Mesh, t: Transform| {
            let m = meshes.add(mesh);
            cmd.entity(at).with_children(|p| {
                p.spawn((LookPart, Mesh3d(m), MeshMaterial3d(ol_mat.clone()), t, bevy::light::NotShadowCaster));
            });
        };

        // peças geradas do visual anterior saem
        for c in &all {
            if old_parts.contains(*c) {
                commands.entity(*c).despawn();
            }
        }

        // cabeça: o nó com `head: 1` nos extras (com a escala do elipsoide); a malha é ele ou um filho
        let head_nodes: Vec<Entity> =
            all.iter().copied().filter(|c| extras.get(*c).is_ok_and(|x| x.value.contains("\"head\":1"))).collect();
        let mut head_meshes = Vec::new();
        for h in &head_nodes {
            head_meshes.push(*h);
            head_meshes.extend(children.iter_descendants(*h));
        }
        head_meshes.retain(|c| mesh_q.contains(*c));
        let head_base = head_meshes
            .iter()
            .find_map(|c| {
                mesh_q.get(*c).ok().and_then(|(_, cur, orig)| orig.map(|o| o.0.clone()).or(cur.map(|m| m.0.clone())))
            })
            .and_then(|h| mats.get(&h).cloned());
        let faces: Option<[Handle<ToonMaterial>; 3]> = head_base.map(|base| {
            [0usize, 1, 2].map(|v| {
                // o canvas do original sobe com `flipY` (linha 0 em v = 1, o topo da cabeça) e as UVs da esfera
                // exportada seguem essa convenção; no wgpu a linha 0 fica em v = 0, então as linhas vão invertidas
                let px: Vec<u8> = head::head_texture(&kit.palettes, cfg, v)
                    .chunks_exact(head::FW * 4)
                    .rev()
                    .flatten()
                    .copied()
                    .collect();
                let img = images.add(Image::new(
                    Extent3d { width: head::FW as u32, height: head::FH as u32, depth_or_array_layers: 1 },
                    TextureDimension::D2,
                    px,
                    TextureFormat::Rgba8UnormSrgb,
                    RenderAssetUsages::default(),
                ));
                let mut m = base.clone().avatar();
                if b.head_gloss {
                    m = m.glossy(GLOSS);
                }
                m.base_texture = Some(img);
                m.params.flags |= sky_rendering::flags::TEXTURE;
                mats.add(m)
            })
        });

        for c in &all {
            let Ok((_, cur_mat, orig_mat)) = mesh_q.get(*c) else { continue };
            // sempre a partir do material original (o visual pode mudar várias vezes)
            let mat = orig_mat.map(|o| o.0.clone()).or_else(|| cur_mat.map(|m| m.0.clone()));
            if let (Some(m), None) = (&mat, orig_mat) {
                commands.entity(*c).insert(OriginalMat(m.clone()));
            }
            let j = joint(*c);
            // corpo do avatar de referência: dá lugar às peças geradas abaixo
            if matches!(j, Some(Joint::Torso | Joint::Arm | Joint::Thigh | Joint::Shin | Joint::Foot)) {
                commands.entity(*c).insert(Visibility::Hidden);
                continue;
            }
            if head_meshes.contains(c) {
                if let Some(f) = &faces {
                    commands.entity(*c).insert(MeshMaterial3d(f[0].clone()));
                }
                continue;
            }
            // orelhas (e o resto): cor de material, trocada pela do visual
            let Some(mat) = mat else { continue };
            let Some(base) = mats.get(&mat) else { continue };
            let rgb = [base.params.color.x, base.params.color.y, base.params.color.z];
            let mut m = base.clone();
            if let Some((_, to)) = map.iter().find(|(from, _)| close(*from, rgb)) {
                m.params.color = Vec4::new(to[0], to[1], to[2], base.params.color.w);
            }
            if j == Some(Joint::Head) {
                m = m.avatar();
                if b.ear_gloss {
                    m = m.glossy(GLOSS);
                }
            }
            commands.entity(*c).insert(MeshMaterial3d(mats.add(m)));
        }

        // contorno da cabeça e das orelhas (SphereGeometry(1, 12, 8), na escala do nó e do grupo da cabeça)
        let hd = named("hd").first().copied();
        let hd_s = hd.map_or(Vec3::ONE, scale_of);
        let (sgs_p, sgs_n) = sphere_tris(12, 8);
        let ears: Vec<Entity> = all
            .iter()
            .copied()
            .filter(|c| extras.get(*c).is_ok_and(|x| x.value.contains("\"big\":1")) && joint(*c) == Some(Joint::Head))
            .collect();
        for n in head_nodes.iter().chain(&ears) {
            let s = scale_of(*n) * hd_s;
            spawn_outline(
                &mut commands,
                &mut meshes,
                *n,
                outline_mesh(&sgs_p, &sgs_n, s, OUTLINE_W),
                Transform::IDENTITY,
            );
        }

        // corpo: malha normal e brilhante de cada junta, com o contorno feito da geometria de longe
        let body_mat =
            mats.add(ToonMaterial::new(&desc(Kind::Toon(Ramp::Scene), false, false, true), &world.lighting).avatar());
        let body_gloss = mats.get(&body_mat).cloned().map(|m| mats.add(m.glossy(GLOSS)));
        let parts = [
            ("fig", &b.torso),
            ("armL", &b.arm),
            ("armR", &b.arm),
            ("lgL", &b.thigh),
            ("lgR", &b.thigh),
            ("knL", &b.shin),
            ("knR", &b.shin),
        ];
        for (name, limb) in parts {
            for at in named(name) {
                for f in 0..2 {
                    let hi = &limb.hi[f];
                    if hi.is_empty() {
                        continue;
                    }
                    let lo = if limb.lo[f].is_empty() { hi } else { &limb.lo[f] };
                    let m = meshes.add(tri_mesh(&hi.positions, &hi.normals, Some(&hi.colors)));
                    let mat = if f == 1 { body_gloss.clone().unwrap_or(body_mat.clone()) } else { body_mat.clone() };
                    commands.entity(at).with_children(|p| {
                        p.spawn((LookPart, Mesh3d(m), MeshMaterial3d(mat), Transform::IDENTITY));
                    });
                    spawn_outline(
                        &mut commands,
                        &mut meshes,
                        at,
                        outline_mesh(&lo.positions, &lo.normals, Vec3::ONE, OUTLINE_W),
                        Transform::IDENTITY,
                    );
                }
            }
        }
        // pés: elipsoide SphereGeometry(1, 10, 7) na cor do calçado; contorno com SphereGeometry(1, 8, 6)
        let mut fd = desc(Kind::Toon(Ramp::Avatar), false, false, false);
        let fc = b.foot.color;
        fd.color = LinearRgba::rgb(fc[0], fc[1], fc[2]);
        let mut foot_mat = ToonMaterial::new(&fd, &world.lighting).avatar();
        if b.foot.gloss {
            foot_mat = foot_mat.glossy(GLOSS);
        }
        let foot_mat = mats.add(foot_mat);
        let (fp, fnrm) = sphere_tris(10, 7);
        let foot_mesh = meshes.add(tri_mesh(&fp, &fnrm, None));
        let (lp, ln) = sphere_tris(8, 6);
        let ft = Transform::from_translation(Vec3::from(b.foot.center)).with_scale(Vec3::from(b.foot.radii));
        for at in [named("ftL"), named("ftR")].concat() {
            commands.entity(at).with_children(|p| {
                p.spawn((LookPart, Mesh3d(foot_mesh.clone()), MeshMaterial3d(foot_mat.clone()), ft));
            });
            spawn_outline(
                &mut commands,
                &mut meshes,
                at,
                outline_mesh(&lp, &ln, Vec3::from(b.foot.radii), OUTLINE_W),
                ft,
            );
        }
        if let Ok(mut rig) = rigs.get_mut(e) {
            rig.fy = b.fy;
        }

        // cabelo ou chapéu do visual, no grupo da cabeça, no lugar dos do avatar de referência
        for c in &all {
            if extras.get(*c).is_ok_and(|x| x.value.contains("\"part\":\"hair") || x.value.contains("\"part\":\"hat\""))
            {
                commands.entity(*c).insert(Visibility::Hidden);
            }
        }
        if let Some(hd) = hd {
            let hat = cfg.get("hat");
            let (name, w, flat, double, gloss) = if hat > 0 {
                (format!("hat-{hat}"), 0.011, false, false, cfg.flag("hb").then_some(GLOSS))
            } else {
                (format!("hs-{:02}", cfg.get("hs")), 0.034, true, true, cfg.flag("rb").then_some(GLOSS_HAIR))
            };
            if let Some(shape) = kit.shape(&name) {
                let colors = paint(&shape, &kit.palettes, cfg);
                let mesh = meshes.add(tri_mesh(&shape.positions, &shape.normals, Some(&colors)));
                let mut m =
                    ToonMaterial::new(&desc(Kind::Toon(Ramp::Avatar), flat, double, true), &world.lighting).avatar();
                if let Some(k) = gloss {
                    m = m.glossy(k);
                }
                let mat = mats.add(m);
                let ol_pos: Vec<f32> =
                    shape.lo_positions.iter().zip(&shape.lo_offsets).map(|(p, o)| p + o * w as f32).collect();
                let ol_mesh = meshes.add(tri_mesh(&ol_pos, &shape.lo_offsets, None));
                commands.entity(hd).with_children(|p| {
                    p.spawn((LookPart, Mesh3d(mesh), MeshMaterial3d(mat), Transform::IDENTITY));
                    p.spawn((
                        LookPart,
                        Mesh3d(ol_mesh),
                        MeshMaterial3d(ol_mat.clone()),
                        Transform::IDENTITY,
                        bevy::light::NotShadowCaster,
                    ));
                });
            }
        }
        let mut ec = commands.entity(e);
        ec.insert(LookApplied(cfg.clone()));
        if let Some(f) = faces {
            ec.insert(HeadFaces { meshes: head_meshes, mats: f, cur: 0 });
        }
    }
}

/// Boca do quadro: troca o rosto da cabeça (`mo.scale` → `headMat(c, v)` no original).
fn mouths(mut commands: Commands, mut q: Query<(&crate::anim::Rig, &mut HeadFaces)>) {
    for (rig, mut hf) in &mut q {
        let v = rig.mouth.min(2);
        if hf.cur == v {
            continue;
        }
        hf.cur = v;
        for m in &hf.meshes {
            if let Ok(mut ec) = commands.get_entity(*m) {
                ec.insert(MeshMaterial3d(hf.mats[usize::from(v)].clone()));
            }
        }
    }
}
