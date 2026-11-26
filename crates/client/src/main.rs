//! Skyke3D nativo.
//!
//! ```text
//! skyke3d                                              # tela inicial
//! skyke3d --world cerron --explore                     # explorar um mundo sem rede
//! skyke3d --server http://localhost:3000 --name Ana [--world cerron --bots 3 | --join ABCDE-FGHJK]
//! skyke3d --world cerron --shot jogo --out cerron.png  # captura de referência para o comparador visual
//! ```

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod anim;
mod args;
mod backpack;
mod cook;
mod editor;
mod farm;
mod gestures;
mod hud;
mod look;
mod menu;
mod net;
mod player;
mod recovery;
mod settings;
mod shot;
mod social;
mod state;

use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::prelude::*;
use bevy::window::WindowResolution;
use sky_world::{LoadWorld, WorldPlugin};
use state::{ChatLog, ConnectError, ConnectRequest, Screen, SendCfg, SendChat, SendGesture, Session};

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
    .init_state::<Screen>()
    .init_resource::<Session>()
    .init_resource::<ChatLog>()
    .init_resource::<ConnectError>()
    .add_message::<ConnectRequest>()
    .add_message::<SendChat>()
    .add_message::<SendGesture>()
    .add_message::<SendCfg>()
    .add_message::<state::SendMsg>()
    .add_message::<state::SendRaw>()
    .add_message::<state::ServerOther>()
    .add_plugins((WorldPlugin { assets_dir: assets.clone() }, sky_ui::SkyUiPlugin))
    .add_plugins((
        look::LookPlugin { assets_dir: assets.clone() },
        player::PlayerPlugin,
        net::NetPlugin,
        hud::HudPlugin,
    ))
    .add_plugins((
        anim::AnimPlugin,
        gestures::GesturePlugin,
        editor::EditorPlugin,
        recovery::RecoveryPlugin,
        social::SocialPlugin,
        settings::SettingsPlugin,
        backpack::BackpackPlugin,
        farm::FarmPlugin,
        cook::CookPlugin,
    ))
    .insert_resource(load_gestures(&assets))
    .insert_resource(load_editor_catalog(&assets))
    .insert_resource(load_items(&assets))
    .insert_resource(load_catalog::<farm::FarmCatalog>(&assets, "fazenda", farm::FarmCatalog::from_catalog_json))
    .insert_resource(load_catalog::<cook::CookCatalog>(&assets, "culinária", cook::CookCatalog::from_catalog_json))
    .insert_resource(a.clone())
    .add_systems(Startup, setup)
    .add_systems(OnEnter(Screen::Playing), start_gesture);
    if let Some(shot) = &a.shot {
        app.add_plugins(shot::ShotPlugin { name: shot.clone(), out: a.out.clone() });
    } else {
        app.add_plugins(menu::MenuPlugin);
    }
    if let Some(after) = a.capture {
        app.add_plugins(shot::CapturePlugin { after, out: a.out.clone() });
    }
    app.run();
}

fn setup(
    mut commands: Commands,
    a: Res<args::Args>,
    mut load: MessageWriter<LoadWorld>,
    mut req: MessageWriter<ConnectRequest>,
    mut next: ResMut<NextState<Screen>>,
) {
    // Mesma câmera do cliente web: perspectiva de 60° na vertical, de 0,1 a 600 m, sem tone mapping e saída sRGB.
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: 60f32.to_radians(), near: 0.1, far: 600.0, ..default() }),
        Tonemapping::None,
        DebandDither::Disabled,
        Msaa::Sample4,
        Transform::from_xyz(0.0, 4.8, 14.2).looking_at(Vec3::new(0.0, 1.2, 2.0), Vec3::Y),
    ));
    if a.shot.is_some() {
        load.write(LoadWorld(a.world));
    } else if let Some(server) = &a.server {
        // linha de comando com servidor: entra direto, sem a tela inicial
        req.write(ConnectRequest {
            server: server.clone(),
            name: a.name.clone(),
            join: a.join.clone(),
            world: a.world,
            bots: a.bots,
        });
        next.set(Screen::Connecting);
    } else if a.explore {
        load.write(LoadWorld(a.world));
        next.set(Screen::Playing);
    }
}

fn load_catalog<T: Default>(
    assets: &std::path::Path,
    what: &str,
    parse: fn(&[u8]) -> Result<T, serde_json::Error>,
) -> T {
    let p = assets.join("../data/catalogs/catalog.json");
    match std::fs::read(&p).map_err(|e| e.to_string()).and_then(|b| parse(&b).map_err(|e| e.to_string())) {
        Ok(c) => c,
        Err(e) => {
            error!("{what}: catálogo em {}: {e}", p.display());
            T::default()
        }
    }
}

fn load_items(assets: &std::path::Path) -> backpack::ItemCatalog {
    let p = assets.join("../data/catalogs/catalog.json");
    std::fs::read(&p).ok().and_then(|b| backpack::ItemCatalog::from_catalog_json(&b).ok()).unwrap_or_else(|| {
        error!("mochila: catálogo não encontrado em {}", p.display());
        backpack::ItemCatalog::default()
    })
}

fn load_editor_catalog(assets: &std::path::Path) -> editor::EditorCatalog {
    let p = assets.join("../data/catalogs/catalog.json");
    std::fs::read(&p).ok().and_then(|b| editor::EditorCatalog::from_catalog_json(&b).ok()).unwrap_or_else(|| {
        error!("editor de visual: catálogo não encontrado em {}", p.display());
        editor::EditorCatalog::default()
    })
}

fn load_gestures(assets: &std::path::Path) -> gestures::GestureCatalog {
    let p = assets.join("../data/catalogs/catalog.json");
    std::fs::read(&p).ok().and_then(|b| gestures::GestureCatalog::from_catalog_json(&b).ok()).unwrap_or_else(|| {
        error!("gestos: catálogo não encontrado em {}", p.display());
        gestures::GestureCatalog::default()
    })
}

fn start_gesture(a: Res<args::Args>, mut send: MessageWriter<SendGesture>) {
    if let Some(g) = &a.gesture {
        send.write(SendGesture(g.clone()));
    }
}
