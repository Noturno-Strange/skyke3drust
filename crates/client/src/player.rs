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
            (walk.run_if(in_state(crate::state::Screen::Playing)), follow_camera, interpolate_system).chain(),
        );
    }
}

/// Jogador deste computador.
#[derive(Component, Debug, Default)]
pub struct Me {
    pub moving: bool,
    /// Sentado neste assento.
    pub seat: Option<SeatPose>,
    /// Altura do pulo e velocidade vertical.
    pub jy: f32,
    pub vy: f32,
}

/// Assento: posição, giro e a pose de quem senta (altura, deslocamento do corpo, dobra do joelho), como `seat()`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeatPose {
    pub x: f32,
    pub z: f32,
    pub r: f32,
    pub h: f32,
    pub fo: f32,
    pub k: f32,
}

/// Assentos do mundo atual.
pub fn seats(world: &sky_world::CurrentWorld) -> Vec<SeatPose> {
    let num = |s: &sky_world::meta::Seat, k: &str, d: f32| {
        s.extra.get(k).and_then(serde_json::Value::as_f64).map_or(d, |v| v as f32)
    };
    world
        .meta
        .seats
        .iter()
        .map(|s| SeatPose { x: s.x, z: s.z, r: s.r, h: num(s, "h", 0.45), fo: num(s, "fo", 0.0), k: num(s, "k", 1.4) })
        .collect()
}

/// Assento mais perto de (x, z) a menos de `max` metros (`seatAt` usa 0,5; sentar procura até 1,5).
pub fn seat_near(seats: &[SeatPose], x: f32, z: f32, max: f32) -> Option<SeatPose> {
    seats
        .iter()
        .map(|s| (s, (s.x - x).hypot(s.z - z)))
        .filter(|(_, d)| *d < max)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(s, _)| *s)
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
    /// Botão direito pressionado: distância arrastada e se já virou giro.
    pub orbit: Option<(f32, bool)>,
}

pub fn avatar_scene(assets: &AssetServer) -> WorldAssetRoot {
    // Modelo exportado do cliente web (visual sorteado no menu) até a personalização ser migrada.
    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("avatars/default.glb")))
}

fn spawn_me(mut commands: Commands, assets: Res<AssetServer>, kit: Option<Res<crate::look::AvatarKit>>) {
    let mut e =
        commands.spawn((Me::default(), avatar_scene(&assets), Transform::from_xyz(0.0, 0.0, 5.0), Name::new("eu")));
    if let Some(kit) = kit {
        e.insert(crate::look::AvatarLook(kit.reference.clone()));
    }
}

/// Mantém dentro da área caminhável (círculo de raio 10,2 m em volta de (0, 1,2)).
pub fn clamp_walk(p: Vec2) -> Vec2 {
    let c = Vec2::new(0.0, 1.2);
    let d = p - c;
    if d.length() > 10.2 { c + d.normalize() * 10.2 } else { p }
}

#[allow(clippy::too_many_arguments)]
fn walk(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    ctl: Res<Controls>,
    mut me: Query<(&mut Transform, &mut Me)>,
    focus: Res<bevy::input_focus::InputFocus>,
    inputs: Query<(), With<bevy::text::EditableText>>,
    world: Option<Res<sky_world::CurrentWorld>>,
    others: Query<(&Transform, &Remote), Without<Me>>,
    mut log: ResMut<crate::state::ChatLog>,
) {
    let Ok((mut t, mut me)) = me.single_mut() else { return };
    // digitando no chat ou num campo: as teclas não movem o avatar
    if crate::hud::typing(&focus, &inputs) {
        me.moving = false;
        return;
    }
    let dt = time.delta_secs();
    // F senta no assento livre mais perto (até 1,5 m) ou levanta, um passo para trás; E pula
    if keys.just_pressed(KeyCode::KeyF) {
        if let Some(seat) = me.seat.take() {
            let p = clamp_walk(Vec2::new(t.translation.x - seat.r.sin() * 0.8, t.translation.z - seat.r.cos() * 0.8));
            t.translation.x = p.x;
            t.translation.z = p.y;
        } else if let Some(world) = &world {
            let here = Vec2::new(t.translation.x, t.translation.z);
            let taken = |s: &SeatPose| {
                others.iter().any(|(o, r)| r.sitting && (o.translation.x - s.x).hypot(o.translation.z - s.z) < 0.4)
            };
            let free: Vec<SeatPose> = seats(world).into_iter().filter(|s| !taken(s)).collect();
            match seat_near(&free, here.x, here.y, 1.5) {
                Some(s) => {
                    t.translation = Vec3::new(s.x, 0.0, s.z);
                    t.rotation = Quat::from_rotation_y(s.r);
                    me.seat = Some(s);
                    me.jy = 0.0;
                    me.vy = 0.0;
                }
                None => log.push("", "Chegue perto de uma cadeira para sentar.", true),
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyE) && me.seat.is_none() && me.jy == 0.0 {
        me.vy = 4.3;
    }
    me.jy += me.vy * dt;
    me.vy -= 12.0 * dt;
    if me.jy <= 0.0 {
        me.jy = 0.0;
        me.vy = 0.0;
    }
    t.translation.y = me.jy;
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
    if mv.length_squared() > 0.0 && me.seat.is_some() {
        // andar levanta
        me.seat = None;
    }
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
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<bevy::input::mouse::AccumulatedMouseMotion>,
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
    // botão direito: arrastar gira a câmera (camYaw −= dx·0,008); clique sem arrastar alterna entre a câmera padrão e a
    // de frente para o avatar, como no cliente web
    if mouse.just_pressed(MouseButton::Right) {
        ctl.orbit = Some((0.0, false));
    }
    if let Some((dist, moved)) = ctl.orbit.as_mut() {
        *dist += motion.delta.length();
        if *dist > 5.0 {
            *moved = true;
        }
        if *moved {
            ctl.cam_yaw -= motion.delta.x * 0.008;
        }
    }
    if mouse.just_released(MouseButton::Right)
        && let Some((_, moved)) = ctl.orbit.take()
        && !moved
    {
        let face = me.rotation.to_euler(EulerRot::YXZ).0;
        let ad =
            |a: f32, b: f32| (a - b + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
        ctl.cam_yaw = if ad(face, ctl.cam_yaw_smooth).abs() < 0.35 {
            ctl.cam_yaw_smooth + ad(0.0, ctl.cam_yaw_smooth)
        } else {
            ctl.cam_yaw_smooth + ad(face, ctl.cam_yaw_smooth)
        };
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
pub fn interpolate_system(time: Res<Time<Real>>, mut q: Query<(&mut Transform, &Remote)>) {
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
