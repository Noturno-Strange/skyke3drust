//! Animação dos avatares, portada de `animA` (assets/app-ui.js): a cada quadro a pose volta ao repouso e as juntas
//! (`fig`, braços, pernas, joelhos, pés e cabeça) recebem andar, pulo ou sentar pelas mesmas fórmulas do original.
//! Os gestos e danças (`DANCES`) entram por cima da pose ([`crate::gestures`]).

use crate::player::{Me, Remote};
use bevy::prelude::*;

#[derive(Debug)]
pub struct AnimPlugin;

impl Plugin for AnimPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (build_rigs, animate.after(crate::player::interpolate_system)).chain());
    }
}

/// Juntas de um avatar (nós com nome no GLB exportado).
#[derive(Component, Debug, Clone)]
pub struct Rig {
    pub fig: Entity,
    pub arm_l: Entity,
    pub arm_r: Entity,
    pub hd: Entity,
    pub lg_l: Entity,
    pub lg_r: Entity,
    pub kn_l: Entity,
    pub kn_r: Entity,
    pub ft_l: Entity,
    pub ft_r: Entity,
    /// Posições de repouso das pernas (x, z) e da altura do corpo (`fy`).
    pub lg_x: [f32; 2],
    pub fy: f32,
    pub ph: f32,
    pub spd: f32,
    pub last: Option<Vec3>,
}

/// Pose de um quadro, nos mesmos termos do original (ângulos em radianos, ordem de Euler XYZ do three.js).
#[derive(Debug, Clone, Copy, Default)]
pub struct Pose {
    pub fig_pos: Vec3,
    pub fig_rot: Vec3,
    pub fig_scale: Vec3,
    pub arm_l: Vec3,
    pub arm_r: Vec3,
    pub hd: Vec3,
    pub lg_l: Vec3,
    pub lg_r: Vec3,
    pub lg_l_y: f32,
    pub lg_r_y: f32,
    pub kn_l: Vec3,
    pub kn_r: Vec3,
    pub ft_l: Vec3,
    pub ft_r: Vec3,
    /// Abertura da boca (`mo.scale`): 0 normal, 1 alta, 2 grito.
    pub mouth: u8,
}

impl Pose {
    /// Repouso (início de `animA`).
    pub fn rest(fy: f32) -> Self {
        Self {
            fig_pos: Vec3::new(0.0, fy, 0.0),
            fig_scale: Vec3::ONE,
            arm_l: Vec3::new(0.0, 0.0, -0.44),
            arm_r: Vec3::new(0.0, 0.0, 0.44),
            lg_l_y: 0.72,
            lg_r_y: 0.72,
            ..default()
        }
    }

    /// Sentado num assento de altura `h`, com o corpo `fo` para a frente e joelhos dobrados `k`.
    pub fn sit(&mut self, h: f32, fo: f32, k: f32) {
        self.fig_pos = Vec3::new(0.0, h - 0.52, fo);
        self.lg_l.x = -1.4;
        self.lg_r.x = -1.4;
        self.kn_l.x = k;
        self.kn_r.x = k;
    }

    /// Andar (`sp` 0–1) na fase `ph`, ou no ar.
    pub fn walk(&mut self, ph: f32, sp: f32, air: bool) {
        if air {
            self.arm_r.z = 2.3;
            self.arm_l.z = -2.3;
            self.lg_l.z = -0.35;
            self.lg_r.z = 0.35;
            self.fig_scale = Vec3::new(0.94, 1.08, 0.94);
        } else if sp > 0.05 {
            let (s1, s2) = (ph.sin(), (ph * 2.0).sin());
            self.lg_l.x = s1 * 0.95 * sp;
            self.lg_r.x = -s1 * 0.95 * sp;
            self.lg_l_y = 0.72 + s1.max(0.0) * 0.12 * sp;
            self.lg_r_y = 0.72 + (-s1).max(0.0) * 0.12 * sp;
            self.arm_l.x = s1 * 1.15 * sp;
            self.arm_r.x = -s1 * 1.15 * sp;
            self.arm_l.z = -0.44 - s1.abs() * 0.5 * sp;
            self.arm_r.z = 0.44 + s1.abs() * 0.5 * sp;
            self.fig_rot.z = s1 * 0.16 * sp;
            self.fig_rot.x = 0.1 * sp;
            self.fig_pos.y += s1.abs() * 0.1 * sp;
            self.fig_scale = Vec3::new(1.0 - s2 * 0.03 * sp, 1.0 + s2 * 0.05 * sp, 1.0 - s2 * 0.03 * sp);
            self.hd.z = (ph + 1.0).sin() * 0.12 * sp;
            self.hd.x = s2 * 0.06 * sp;
        }
    }
}

fn euler(v: Vec3) -> Quat {
    Quat::from_euler(EulerRot::XYZ, v.x, v.y, v.z)
}

#[allow(clippy::type_complexity)]
fn build_rigs(
    mut commands: Commands,
    actors: Query<Entity, (Or<(With<Me>, With<Remote>)>, Without<Rig>)>,
    children: Query<&Children>,
    names: Query<(&Name, &Transform)>,
    kit: Option<Res<crate::look::AvatarKit>>,
) {
    for e in &actors {
        let mut found = std::collections::HashMap::new();
        for c in children.iter_descendants(e) {
            if let Ok((n, t)) = names.get(c) {
                found.insert(n.as_str().to_owned(), (c, *t));
            }
        }
        let get = |k: &str| found.get(k).map(|(e, _)| *e);
        let (Some(fig), Some(arm_l), Some(arm_r), Some(hd), Some(lg_l), Some(lg_r)) =
            (get("fig"), get("armL"), get("armR"), get("hd"), get("lgL"), get("lgR"))
        else {
            continue;
        };
        let (Some(kn_l), Some(kn_r), Some(ft_l), Some(ft_r)) = (get("knL"), get("knR"), get("ftL"), get("ftR")) else {
            continue;
        };
        let lx = |k: &str| found.get(k).map_or(0.0, |(_, t)| t.translation.x);
        commands.entity(e).insert(Rig {
            fig,
            arm_l,
            arm_r,
            hd,
            lg_l,
            lg_r,
            kn_l,
            kn_r,
            ft_l,
            ft_r,
            lg_x: [lx("lgL"), lx("lgR")],
            fy: kit.as_ref().map_or(0.247, |k| k.fy),
            ph: 0.0,
            spd: 0.0,
            last: None,
        });
    }
}

/// Pose de cada avatar no quadro: repouso, sentado ou andando, e por cima o gesto em andamento.
#[allow(clippy::type_complexity)]
fn animate(
    time: Res<Time>,
    mut actors: Query<(&GlobalTransform, &mut Rig, Option<&Me>, Option<&Remote>, Option<&crate::gestures::Gesture>)>,
    mut joints: Query<&mut Transform, Without<Rig>>,
    world: Option<Res<sky_world::CurrentWorld>>,
) {
    let seats = world.as_ref().map(|w| crate::player::seats(w)).unwrap_or_default();
    let dt = time.delta_secs().max(0.0001);
    for (gt, mut rig, me, remote, gest) in &mut actors {
        let pos = gt.translation();
        let moved = rig.last.map_or(0.0, |l| Vec2::new(pos.x - l.x, pos.z - l.z).length());
        rig.last = Some(pos);
        if let Some(me) = me {
            // a velocidade do jogador local segue a tecla (3,2 m/s andando, 0 parado), suavizada
            let target = if me.moving && pos.y < 0.01 { 3.2 } else { 0.0 };
            rig.spd += (target - rig.spd) * (dt * 12.0).min(1.0);
        } else {
            rig.spd += (moved / dt - rig.spd) * 0.25;
        }
        rig.ph += dt * rig.spd * 3.4;
        // sentado: o jogador local sabe o assento; para os outros, o assento a menos de 0,5 m (`seatAt`)
        let seat = match me {
            Some(me) => me.seat,
            None if remote.is_some_and(|r| r.sitting) => crate::player::seat_near(&seats, pos.x, pos.z, 0.5)
                .or(Some(crate::player::SeatPose { x: pos.x, z: pos.z, r: 0.0, h: 0.45, fo: 0.0, k: 1.4 })),
            None => None,
        };
        let sitting = seat.is_some();
        let mut p = Pose::rest(rig.fy);
        if let Some(s) = seat {
            p.sit(s.h, s.fo, s.k);
        }
        let sp = if sitting { 0.0 } else { (rig.spd / 2.2).min(1.0) };
        let air = !sitting && pos.y > 0.07;
        p.walk(rig.ph, sp, air);
        if let Some(g) = gest {
            crate::gestures::apply(&mut p, g, !sitting);
            if sitting {
                p.lg_l = Vec3::new(-1.4, 0.0, 0.0);
                p.lg_r = Vec3::new(-1.4, 0.0, 0.0);
                let k = seat.map_or(1.4, |s| s.k);
                p.kn_l = Vec3::new(k, 0.0, 0.0);
                p.kn_r = Vec3::new(k, 0.0, 0.0);
                p.lg_l_y = 0.72;
                p.lg_r_y = 0.72;
                p.fig_pos.y += 0.52 * (2.0 - p.fig_rot.z.cos() - p.fig_rot.x.cos()) + 0.52 * (1.0 - p.fig_scale.y);
            } else {
                p.fig_pos.y += p.fig_rot.z.sin().abs() * 0.2 + p.fig_rot.x.sin().abs() * 0.18;
            }
        }
        let mut set = |e: Entity, f: &dyn Fn(&mut Transform)| {
            if let Ok(mut t) = joints.get_mut(e) {
                f(&mut t);
            }
        };
        set(rig.fig, &|t| {
            t.translation = p.fig_pos;
            t.rotation = euler(p.fig_rot);
            t.scale = p.fig_scale;
        });
        set(rig.arm_l, &|t| t.rotation = euler(p.arm_l));
        set(rig.arm_r, &|t| t.rotation = euler(p.arm_r));
        set(rig.hd, &|t| t.rotation = euler(p.hd));
        let lx = rig.lg_x;
        set(rig.lg_l, &|t| {
            t.rotation = euler(p.lg_l);
            t.translation = Vec3::new(lx[0], p.lg_l_y, 0.0);
        });
        set(rig.lg_r, &|t| {
            t.rotation = euler(p.lg_r);
            t.translation = Vec3::new(lx[1], p.lg_r_y, 0.0);
        });
        set(rig.kn_l, &|t| t.rotation = euler(p.kn_l));
        set(rig.kn_r, &|t| t.rotation = euler(p.kn_r));
        set(rig.ft_l, &|t| t.rotation = euler(p.ft_l));
        set(rig.ft_r, &|t| t.rotation = euler(p.ft_r));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rest_and_walk_follow_original_formulas() {
        let p = Pose::rest(0.25);
        assert_eq!(p.arm_r.z, 0.44);
        assert_eq!(p.lg_l_y, 0.72);
        let mut w = Pose::rest(0.25);
        w.walk(std::f32::consts::FRAC_PI_2, 1.0, false);
        assert!((w.lg_l.x - 0.95).abs() < 1e-6);
        assert!((w.lg_r.x + 0.95).abs() < 1e-6);
        assert!((w.lg_l_y - 0.84).abs() < 1e-6);
        let mut s = Pose::rest(0.25);
        s.sit(0.45, 0.0, 1.4);
        assert!((s.fig_pos.y + 0.07).abs() < 1e-6);
    }
}
