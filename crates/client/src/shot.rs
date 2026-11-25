//! Captura de referência: posiciona a câmera como no exportador (`world.json` → `shots`), espera o mundo carregar,
//! salva um PNG e fecha. O comparador visual compara com `ref-<nome>.png` exportado do cliente web.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use sky_world::{CurrentWorld, WorldPart};
use std::path::PathBuf;

#[derive(Debug)]
pub struct ShotPlugin {
    pub name: String,
    pub out: PathBuf,
}

/// Presente enquanto a captura controla a câmera.
#[derive(Resource, Debug)]
pub struct ShotActive {
    name: String,
    out: PathBuf,
    frames: u32,
    taken: bool,
}

impl Plugin for ShotPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ShotActive { name: self.name.clone(), out: self.out.clone(), frames: 0, taken: false })
            .add_systems(Update, run);
    }
}

#[allow(clippy::too_many_arguments)]
fn run(
    mut commands: Commands,
    mut s: ResMut<ShotActive>,
    world: Option<Res<CurrentWorld>>,
    assets: Res<AssetServer>,
    parts: Query<&bevy::world_serialization::WorldAssetRoot, With<WorldPart>>,
    mut cam: Query<(&mut Transform, &mut Projection), With<Camera3d>>,
    mut me: Query<&mut Visibility, With<crate::player::Me>>,
    mut exit: MessageWriter<AppExit>,
) {
    let Some(world) = world else { return };
    let Some(shot) = world.meta.shots.get(&s.name) else {
        error!("captura {} não existe em world.json", s.name);
        exit.write(AppExit::error());
        return;
    };
    // só o cenário (os avatares têm comparação própria)
    for mut v in &mut me {
        *v = Visibility::Hidden;
    }
    if let Ok((mut t, mut p)) = cam.single_mut() {
        *t = Transform::from_translation(Vec3::from(shot.pos)).looking_at(Vec3::from(shot.look), Vec3::Y);
        if let Projection::Perspective(pp) = p.as_mut() {
            pp.fov = shot.fov.to_radians();
        }
    }
    let loaded = !parts.is_empty() && parts.iter().all(|r| assets.is_loaded_with_dependencies(&r.0));
    if !loaded {
        return;
    }
    s.frames += 1;
    // alguns quadros para os materiais serem trocados e os pipelines compilarem
    if s.frames == 90 && !s.taken {
        s.taken = true;
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(s.out.clone()));
        info!("captura {} → {}", s.name, s.out.display());
    }
    if s.frames == 150 {
        exit.write(AppExit::Success);
    }
}

/// Captura da janela inteira (interface incluída) depois de alguns segundos, para conferir telas.
#[derive(Debug)]
pub struct CapturePlugin {
    pub after: f32,
    pub out: PathBuf,
}

#[derive(Resource, Debug)]
struct Capture {
    after: f32,
    out: PathBuf,
    taken_at: Option<f32>,
}

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Capture { after: self.after, out: self.out.clone(), taken_at: None })
            .add_systems(Update, capture);
    }
}

fn capture(mut commands: Commands, mut c: ResMut<Capture>, time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    let t = time.elapsed_secs();
    match c.taken_at {
        None if t >= c.after => {
            c.taken_at = Some(t);
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(c.out.clone()));
        }
        Some(at) if t - at > 1.5 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
}
