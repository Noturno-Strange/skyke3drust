//! Jogador local, atores remotos e câmera em terceira pessoa (assets/app-ui.js do cliente web).

use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use std::collections::VecDeque;

/// Velocidade de caminhada do cliente web (m/s).
pub const WALK_SPEED: f32 = 3.2;
/// Atraso de interpolação dos outros jogadores (ms), igual a `INTERP` em app-net.js.
pub const INTERP_MS: f64 = 230.0;

#[derive(Debug)]
pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Controls>().add_systems(Startup, spawn_me).add_systems(
            Update,
            (walk.run_if(in_state(crate::state::Screen::Playing)), follow_camera, interpolate).chain(),
        );
    }
}

/// Jogador deste computador.
#[derive(Component, Debug, Default)]
pub struct Me {
    pub moving: bool,
}

/// Outro jogador ou bot, posicionado pelos pacotes de estado.
#[derive(Component, Debug, Default)]
pub struct Remote {
    pub snaps: VecDeque<Snap>,
    pub sitting: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Snap {
    pub t: f64,
    pub x: f32,
    pub z: f32,
    pub r: f32,
    pub y: f32,
}

#[derive(Resource, Debug, Default)]
pub struct Controls {
    pub cam_yaw: f32,
    pub cam_yaw_smooth: f32,
}

pub fn avatar_scene(assets: &AssetServer) -> WorldAssetRoot {
    // Modelo exportado do cliente web (visual sorteado no menu) até a personalização ser migrada.
    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("avatars/default.glb")))
}

fn spawn_me(mut commands: Commands, assets: Res<AssetServer>) {
    commands.spawn((Me::default(), avatar_scene(&assets), Transform::from_xyz(0.0, 0.0, 5.0), Name::new("eu")));
}

/// Mantém dentro da área caminhável (círculo de raio 10,2 m em volta de (0, 1,2)).
pub fn clamp_walk(p: Vec2) -> Vec2 {
    let c = Vec2::new(0.0, 1.2);
    let d = p - c;
    if d.length() > 10.2 { c + d.normalize() * 10.2 } else { p }
}

fn walk(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    ctl: Res<Controls>,
    mut me: Query<(&mut Transform, &mut Me)>,
    focus: Res<bevy::input_focus::InputFocus>,
    inputs: Query<(), With<bevy::text::EditableText>>,
) {
    let Ok((mut t, mut me)) = me.single_mut() else { return };
    // digitando no chat ou num campo: as teclas não movem o avatar
    if crate::hud::typing(&focus, &inputs) {
        me.moving = false;
        return;
    }
    let k = |a: KeyCode, b: KeyCode| keys.pressed(a) || keys.pressed(b);
    let mut mv = Vec2::new(
        f32::from(u8::from(k(KeyCode::KeyD, KeyCode::ArrowRight)))
            - f32::from(u8::from(k(KeyCode::KeyA, KeyCode::ArrowLeft))),
        f32::from(u8::from(k(KeyCode::KeyS, KeyCode::ArrowDown)))
            - f32::from(u8::from(k(KeyCode::KeyW, KeyCode::ArrowUp))),
    );
    if ctl.cam_yaw != 0.0 {
        let (s, c) = ctl.cam_yaw.sin_cos();
        mv = Vec2::new(mv.x * c + mv.y * s, -mv.x * s + mv.y * c);
    }
    me.moving = false;
    if mv.length_squared() > 0.0 {
        let d = mv.normalize();
        let before = Vec2::new(t.translation.x, t.translation.z);
        let p = clamp_walk(before + d * WALK_SPEED * time.delta_secs());
        t.translation.x = p.x;
        t.translation.z = p.y;
        t.rotation = Quat::from_rotation_y(d.x.atan2(d.y));
        me.moving = p.distance(before) > time.delta_secs() * 0.8;
    }
}

/// Câmera do jogo: 9,2 m atrás e 4,8 m acima, olhando 3 m à frente do avatar, com suavização exponencial.
fn follow_camera(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut ctl: ResMut<Controls>,
    me: Query<&Transform, (With<Me>, Without<Camera3d>)>,
    mut cam: Query<&mut Transform, With<Camera3d>>,
    shot: Option<Res<crate::shot::ShotActive>>,
) {
    if shot.is_some() {
        return;
    }
    let (Ok(me), Ok(mut cam)) = (me.single(), cam.single_mut()) else { return };
    let dt = time.delta_secs();
    if keys.pressed(KeyCode::KeyQ) {
        ctl.cam_yaw += 1.8 * dt;
    }
    if keys.pressed(KeyCode::KeyE) {
        ctl.cam_yaw -= 1.8 * dt;
    }
    let diff = (ctl.cam_yaw - ctl.cam_yaw_smooth + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    ctl.cam_yaw_smooth += diff * (dt * 10.0).min(1.0);
    let (sy, cy) = ctl.cam_yaw_smooth.sin_cos();
    let w = (1.0 - cy).min(1.0);
    let px = me.translation.x * (0.8 + 0.2 * w);
    let pz = me.translation.z;
    let want = Vec3::new(px + sy * 9.2, 4.8, pz + cy * 9.2);
    let k = 1.0 - (-5.0 * dt).exp();
    cam.translation = cam.translation.lerp(want, k);
    cam.look_at(Vec3::new(px - sy * 3.0, 1.2, pz - cy * 3.0), Vec3::Y);
}

/// Desenha cada ator ~230 ms no passado, entre dois pacotes (como `snapPos` no cliente web).
fn interpolate(time: Res<Time<Real>>, mut q: Query<(&mut Transform, &Remote)>) {
    let rt = time.elapsed_secs_f64() * 1000.0 - INTERP_MS;
    for (mut t, r) in &mut q {
        let s = &r.snaps;
        let Some(first) = s.front() else { continue };
        let p = if rt <= first.t {
            *first
        } else {
            let mut out = *s.back().unwrap_or(first);
            for i in (1..s.len()).rev() {
                let (a, b) = (s[i - 1], s[i]);
                if rt >= a.t {
                    out = if rt >= b.t {
                        b
                    } else {
                        let f = ((rt - a.t) / (b.t - a.t).max(1.0)) as f32;
                        let dr = (b.r - a.r).sin().atan2((b.r - a.r).cos());
                        Snap {
                            t: rt,
                            x: a.x + (b.x - a.x) * f,
                            z: a.z + (b.z - a.z) * f,
                            y: a.y + (b.y - a.y) * f,
                            r: a.r + dr * f,
                        }
                    };
                    break;
                }
            }
            out
        };
        t.translation = Vec3::new(p.x, p.y, p.z);
        t.rotation = Quat::from_rotation_y(p.r);
    }
}

impl Remote {
    /// Guarda um pacote; depois de uma pausa longa repete a posição antiga logo antes da nova (sem "escorregar").
    pub fn push(&mut self, t: f64, x: f32, z: f32, r: f32, y: f32) {
        if let Some(l) = self.snaps.back().copied()
            && t - l.t > 350.0
        {
            self.snaps.push_back(Snap { t: t - 200.0, ..l });
        }
        self.snaps.push_back(Snap { t, x, z, r, y });
        while self.snaps.len() > 6 {
            self.snaps.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_clamp() {
        assert_eq!(clamp_walk(Vec2::new(0.0, 5.0)), Vec2::new(0.0, 5.0));
        let p = clamp_walk(Vec2::new(0.0, 30.0));
        assert!((p.distance(Vec2::new(0.0, 1.2)) - 10.2).abs() < 1e-4);
    }

    #[test]
    fn snapshot_buffer() {
        let mut r = Remote::default();
        for i in 0..10 {
            r.push(f64::from(i) * 200.0, i as f32, 0.0, 0.0, 0.0);
        }
        assert_eq!(r.snaps.len(), 6);
        let mut r = Remote::default();
        r.push(0.0, 0.0, 0.0, 0.0, 0.0);
        r.push(1000.0, 1.0, 0.0, 0.0, 0.0);
        assert_eq!(r.snaps.len(), 3, "pausa longa repete a posição anterior");
    }
}
