//! Renderização do Skyke3D: material toon em WGSL/WESL (`assets/shaders/toon.wesl`) que reproduz o `MeshToonMaterial` do
//! cliente web, materiais básicos (sem luz) e contornos em casco invertido.
//!
//! A iluminação de cada mundo (rampas toon tingidas, hemisfério, luz principal, preenchimento, fogueira e neblina) vem do
//! `world.json` exportado do jogo original e é copiada para cada material do mundo ([`ToonLighting`]).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use bevy::asset::Asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{
    AsBindGroup, Face, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

pub mod mips;

pub use mips::MipQueue;

pub const SHADER_PATH: &str = "shaders/toon.wesl";

#[derive(Debug)]
pub struct ToonPlugin;

impl Plugin for ToonPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<ToonMaterial>::default())
            .init_resource::<MipQueue>()
            .add_systems(PostUpdate, mips::generate_mips);
    }
}

/// Bits de [`ToonParams::flags`] (iguais às constantes do shader).
pub mod flags {
    pub const TOON: u32 = 1;
    pub const FLAT: u32 = 2;
    pub const TEXTURE: u32 = 4;
    pub const FOG: u32 = 8;
    pub const VERTEX_COLOR: u32 = 16;
    pub const MASK: u32 = 32;
    pub const DOUBLE: u32 = 64;
    pub const SKY: u32 = 128;
    pub const GROUND: u32 = 256;
    pub const ADDITIVE: u32 = 512;
    /// Material de avatar (`EMI` do original): soma 0,42 × a cor difusa como emissivo.
    pub const AVATAR: u32 = 1024;
    /// Cor brilhante (`EMIGK`): dois pontos de luz refletidos; a força fica em `fog_range.w`.
    pub const GLOSS: u32 = 2048;
}

/// Luz e neblina de um mundo, já em espaço linear e com intensidades aplicadas (unidades do three.js).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct ToonLighting {
    pub ramp_scene: [Vec3; 3],
    pub ramp_avatar: [Vec3; 3],
    pub hemi_sky: Vec3,
    pub hemi_ground: Vec3,
    /// Direção que aponta PARA a luz (posição − alvo, normalizada).
    pub key_dir: Vec3,
    pub key_color: Vec3,
    pub fill_dir: Vec3,
    pub fill_color: Vec3,
    pub fire_pos: Vec3,
    pub fire_distance: f32,
    pub fire_color: Vec3,
    pub fire_decay: f32,
    pub fog: Option<(Vec3, f32, f32)>,
    pub outline: Vec3,
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
pub struct ToonParams {
    pub color: Vec4,
    pub emissive: Vec4,
    pub uv: Vec4,
    pub ramp0: Vec4,
    pub ramp1: Vec4,
    pub ramp2: Vec4,
    pub hemi_sky: Vec4,
    pub hemi_ground: Vec4,
    pub key_dir: Vec4,
    pub key_color: Vec4,
    pub fill_dir: Vec4,
    pub fill_color: Vec4,
    pub fire_pos: Vec4,
    pub fire_color: Vec4,
    pub fog_color: Vec4,
    pub fog_range: Vec4,
    pub ground_avg: Vec4,
    pub flags: u32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub _pad2: u32,
}

/// Qual rampa toon o material usa (o jogo tem uma para o cenário e outra, mais clara, para avatares).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Ramp {
    #[default]
    Scene,
    Avatar,
}

/// Como desenhar um material exportado (vem de `extras.sky.kind`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Toon(Ramp),
    Basic,
    Outline,
    /// Domo do céu: degradê horizonte, meio e alto (cores sRGB, como no shader original).
    SkyDome {
        hor: Vec3,
        mid: Vec3,
        top: Vec3,
    },
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(ToonKey)]
pub struct ToonMaterial {
    #[uniform(0)]
    pub params: ToonParams,
    #[texture(1)]
    #[sampler(2)]
    pub base_texture: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub cull: Option<Face>,
    pub depth_write: bool,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToonKey {
    cull: u8,
    depth_write: bool,
}

impl From<&ToonMaterial> for ToonKey {
    fn from(m: &ToonMaterial) -> Self {
        Self {
            cull: match m.cull {
                None => 0,
                Some(Face::Back) => 1,
                Some(Face::Front) => 2,
            },
            depth_write: m.depth_write,
        }
    }
}

impl Material for ToonMaterial {
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = match key.bind_group_data.cull {
            1 => Some(Face::Back),
            2 => Some(Face::Front),
            _ => None,
        };
        if !key.bind_group_data.depth_write
            && let Some(ds) = descriptor.depth_stencil.as_mut()
        {
            ds.depth_write_enabled = Some(false);
        }
        Ok(())
    }
}

/// Descrição de um material exportado, independente de engine.
#[derive(Debug, Clone)]
pub struct MaterialDesc {
    pub kind: Kind,
    /// Cor base linear (com alfa = opacidade).
    pub color: LinearRgba,
    pub emissive: LinearRgba,
    pub texture: Option<Handle<Image>>,
    pub uv_scale: Vec2,
    pub uv_offset: Vec2,
    pub flat: bool,
    pub double_sided: bool,
    pub back_side: bool,
    pub transparent: bool,
    pub alpha_cutoff: Option<f32>,
    pub depth_write: bool,
    pub fog: bool,
    pub vertex_colors: bool,
    /// Cor média do chão (linear), quando o material é o chão do mundo.
    pub ground_avg: Option<Vec3>,
    /// Mistura aditiva (`THREE.AdditiveBlending`).
    pub additive: bool,
}

fn v4(v: Vec3, w: f32) -> Vec4 {
    v.extend(w)
}

impl ToonMaterial {
    pub fn new(d: &MaterialDesc, l: &ToonLighting) -> Self {
        let mut f = 0;
        let ramp = match d.kind {
            Kind::Toon(Ramp::Avatar) => {
                f |= flags::TOON;
                l.ramp_avatar
            }
            Kind::Toon(Ramp::Scene) => {
                f |= flags::TOON;
                l.ramp_scene
            }
            _ => l.ramp_scene,
        };
        let color = if d.kind == Kind::Outline { l.outline.extend(1.0) } else { Vec4::from(d.color.to_f32_array()) };
        if d.flat {
            f |= flags::FLAT;
        }
        if d.texture.is_some() {
            f |= flags::TEXTURE;
        }
        if d.fog && l.fog.is_some() {
            f |= flags::FOG;
        }
        if d.vertex_colors {
            f |= flags::VERTEX_COLOR;
        }
        if d.alpha_cutoff.is_some() {
            f |= flags::MASK;
        }
        if d.double_sided {
            f |= flags::DOUBLE;
        }
        if d.ground_avg.is_some() {
            f |= flags::GROUND;
        }
        if d.additive {
            f |= flags::ADDITIVE;
        }
        let ramp = match d.kind {
            Kind::SkyDome { hor, mid, top } => {
                f = flags::SKY;
                [hor, mid, top]
            }
            _ => ramp,
        };
        let (fog_color, near, far) = l.fog.unwrap_or((Vec3::ZERO, 0.0, 0.0));
        let params = ToonParams {
            color,
            emissive: Vec4::from(d.emissive.to_f32_array()),
            uv: Vec4::new(d.uv_scale.x, d.uv_scale.y, d.uv_offset.x, d.uv_offset.y),
            ramp0: v4(ramp[0], 1.0),
            ramp1: v4(ramp[1], 1.0),
            ramp2: v4(ramp[2], 1.0),
            hemi_sky: v4(l.hemi_sky, 1.0),
            hemi_ground: v4(l.hemi_ground, 1.0),
            key_dir: v4(l.key_dir, 0.0),
            key_color: v4(l.key_color, 0.0),
            fill_dir: v4(l.fill_dir, 0.0),
            fill_color: v4(l.fill_color, 0.0),
            fire_pos: v4(l.fire_pos, l.fire_distance),
            fire_color: v4(l.fire_color, l.fire_decay),
            fog_color: v4(fog_color, 1.0),
            fog_range: Vec4::new(near, far, d.alpha_cutoff.unwrap_or(0.0), 0.0),
            ground_avg: d.ground_avg.unwrap_or(Vec3::ZERO).extend(1.0),
            flags: f,
            ..default()
        };
        let cull = match d.kind {
            // casco invertido: só as faces de trás aparecem (THREE.BackSide)
            Kind::Outline | Kind::SkyDome { .. } => Some(Face::Front),
            _ if d.double_sided => None,
            _ if d.back_side => Some(Face::Front),
            _ => Some(Face::Back),
        };
        let alpha_mode = if d.additive {
            AlphaMode::Add
        } else if d.transparent {
            AlphaMode::Blend
        } else if let Some(c) = d.alpha_cutoff {
            AlphaMode::Mask(c)
        } else {
            AlphaMode::Opaque
        };
        Self { params, base_texture: d.texture.clone(), alpha_mode, cull, depth_write: d.depth_write }
    }

    /// Material de avatar: todos os do original (corpo, cabeça, orelhas, pés, cabelo, chapéus) somam o `EMI`.
    pub fn avatar(mut self) -> Self {
        self.params.flags |= flags::AVATAR;
        self
    }

    /// Versão "cor brilhante" do material (`glossyOf`): `k` = 0,9 no corpo, 0,35 no cabelo.
    pub fn glossy(mut self, k: f32) -> Self {
        self.params.flags |= flags::GLOSS;
        self.params.fog_range.w = k;
        self
    }
}
