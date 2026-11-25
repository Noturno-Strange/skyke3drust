//! `world.json`: metadados exportados de cada mundo pelo `tools/asset-exporter`.

use bevy::color::Srgba;
use bevy::math::Vec3;
use serde::Deserialize;
use sky_rendering::ToonLighting;

#[derive(Debug, Clone, Deserialize)]
pub struct Circle {
    pub x: f32,
    pub z: f32,
    pub r: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Toon {
    pub scene: [[u8; 3]; 3],
    pub avatar: [[u8; 3]; 3],
    pub outline: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hemi {
    pub sky: String,
    pub ground: String,
    pub intensity: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DirLight {
    pub color: String,
    pub intensity: f32,
    pub position: [f32; 3],
    #[serde(default)]
    pub target: [f32; 3],
}

#[derive(Debug, Clone, Deserialize)]
pub struct PointLight {
    pub color: String,
    pub intensity: f32,
    pub distance: f32,
    pub decay: f32,
    pub position: [f32; 3],
}

#[derive(Debug, Clone, Deserialize)]
pub struct Lights {
    pub hemi: Hemi,
    pub key: DirLight,
    pub fill: DirLight,
    pub fire: Option<PointLight>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fog {
    /// Valores que o three.js usa no shader (já no espaço de trabalho linear).
    pub linear: [f32; 3],
    pub near: f32,
    pub far: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Shot {
    pub pos: [f32; 3],
    pub look: [f32; 3],
    pub fov: f32,
}

/// Assento (tronco, toco...). Os campos variam por tipo; os comuns são posição e giro.
#[derive(Debug, Clone, Deserialize)]
pub struct Seat {
    #[serde(default)]
    pub x: f32,
    #[serde(default)]
    pub z: f32,
    #[serde(default)]
    pub y: f32,
    #[serde(default)]
    pub r: f32,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Sprite do cliente web (quad sempre virado para a câmera).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpriteMeta {
    pub root: String,
    pub pos: [f32; 3],
    pub scale: [f32; 2],
    #[serde(default = "half")]
    pub center: [f32; 2],
    #[serde(default)]
    pub rotation: f32,
    pub color: String,
    pub opacity: f32,
    #[serde(default)]
    pub additive: bool,
    #[serde(default = "yes")]
    pub depth_test: bool,
    #[serde(default)]
    pub fog: bool,
    /// Índice de `sprite-N.png` (-1 = sem textura).
    pub tex: i32,
}

fn half() -> [f32; 2] {
    [0.5, 0.5]
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorldMeta {
    pub v: u32,
    pub world: String,
    pub walk: Circle,
    pub fire: Option<Circle>,
    #[serde(default)]
    pub seats: Vec<Seat>,
    /// Áreas que a fauna evita (pés de árvores, pedras...).
    #[serde(default)]
    pub foot: Vec<serde_json::Value>,
    pub toon: Toon,
    pub lights: Lights,
    pub fog: Option<Fog>,
    #[serde(default)]
    pub shots: std::collections::BTreeMap<String, Shot>,
    #[serde(default)]
    pub sprites: Vec<SpriteMeta>,
}

#[derive(Debug, thiserror::Error)]
pub enum MetaError {
    #[error("world.json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("world.json: versão {0} não suportada")]
    Version(u32),
}

/// Cor do `world.json`. O jogo original desliga o `THREE.ColorManagement`: cores definidas por hex entram cruas no
/// shader (sem conversão sRGB → linear) e `getHexString()` devolve esses mesmos valores crus. Por isso o hex é lido
/// direto como valor linear, sem conversão.
pub(crate) fn lin(hex: &str) -> Vec3 {
    let c = Srgba::hex(hex).unwrap_or(Srgba::WHITE);
    Vec3::new(c.red, c.green, c.blue)
}

fn ramp(r: &[[u8; 3]; 3]) -> [Vec3; 3] {
    r.map(|c| Vec3::new(f32::from(c[0]), f32::from(c[1]), f32::from(c[2])) / 255.0)
}

fn dir(l: &DirLight) -> Vec3 {
    (Vec3::from(l.position) - Vec3::from(l.target)).normalize_or(Vec3::Y)
}

impl WorldMeta {
    pub fn parse(bytes: &[u8]) -> Result<Self, MetaError> {
        let m: WorldMeta = serde_json::from_slice(bytes)?;
        if m.v != 1 {
            return Err(MetaError::Version(m.v));
        }
        Ok(m)
    }

    /// Iluminação no formato do material toon.
    pub fn lighting(&self) -> ToonLighting {
        let l = &self.lights;
        let (fire_pos, fire_distance, fire_color, fire_decay) = match &l.fire {
            Some(f) => (Vec3::from(f.position), f.distance, lin(&f.color) * f.intensity, f.decay),
            None => (Vec3::ZERO, 0.0, Vec3::ZERO, 2.0),
        };
        ToonLighting {
            ramp_scene: ramp(&self.toon.scene),
            ramp_avatar: ramp(&self.toon.avatar),
            hemi_sky: lin(&l.hemi.sky) * l.hemi.intensity,
            hemi_ground: lin(&l.hemi.ground) * l.hemi.intensity,
            key_dir: dir(&l.key),
            key_color: lin(&l.key.color) * l.key.intensity,
            fill_dir: dir(&l.fill),
            fill_color: lin(&l.fill.color) * l.fill.intensity,
            fire_pos,
            fire_distance,
            fire_color,
            fire_decay,
            fog: self.fog.as_ref().map(|f| (Vec3::from(f.linear), f.near, f.far)),
            outline: lin(&self.toon.outline),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_exported_world() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/worlds");
        for w in sky_shared::World::ALL {
            let p = root.join(w.id()).join("world.json");
            let m = WorldMeta::parse(&std::fs::read(&p).unwrap()).unwrap();
            assert_eq!(m.world, w.id());
            assert!(!m.seats.is_empty(), "{w}: sem assentos");
            assert!((m.walk.r - 10.2).abs() < 1e-6);
            let l = m.lighting();
            assert!(l.key_dir.is_normalized());
            assert!(m.shots.contains_key("jogo"));
        }
    }
}
