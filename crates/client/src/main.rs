//! Skyke3D nativo.
//!
//! ```text
//! skyke3d [--world cerron]                         # explorar um mundo sem rede
//! skyke3d --server http://localhost:3000 --name Ana [--world cerron --bots 3 | --join ABCDE-FGHJK]
//! skyke3d --world cerron --shot jogo --out cerron.png   # captura de referência para o comparador visual
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod args;
mod net;
mod player;
mod shot;

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::window::WindowResolution;
use sky_world::{LoadWorld, WorldPlugin};

fn main() {
    let a = match args::Args::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{}", args::USAGE);
            std::process::exit(2);
        }
    };
    let assets = a.assets_dir();
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(AssetPlugin { file_path: assets.display().to_string(), ..default() }).set(
        WindowPlugin {
            primary_window: Some(Window {
                title: "Skyke3D".into(),
                resolution: WindowResolution::new(1280, 720),
                resizable: a.shot.is_none(),
                ..default()
            }),
            ..default()
        },
    ))
    .insert_resource(ClearColor(Color::BLACK))
    .add_plugins(WorldPlugin { assets_dir: assets.clone() })
    .add_plugins(player::PlayerPlugin)
    .insert_resource(a.clone())
    .add_systems(Startup, setup);
    if let Some(shot) = &a.shot {
        app.add_plugins(shot::ShotPlugin { name: shot.clone(), out: a.out.clone() });
    }
    if a.server.is_some() {
        app.add_plugins(net::NetPlugin);
    }
    app.run();
}

fn setup(mut commands: Commands, a: Res<args::Args>, mut load: MessageWriter<LoadWorld>) {
    // Mesma câmera do cliente web: perspectiva de 60° na vertical, de 0,1 a 600 m, sem tone mapping e saída sRGB.
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.1, far: 600.0, ..default() }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Sample4,
        Transform::from_xyz(0.0, 4.8, 14.2).looking_at(Vec3::new(0.0, 1.2, 2.0), Vec3::Y),
    ));
    if a.server.is_none() {
        load.write(LoadWorld(a.world));
    }
}
