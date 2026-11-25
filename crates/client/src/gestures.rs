//! Gestos e danças, portados de `DANCES` (assets/app-ui.js): 30 funções do tempo `e` que mexem nas juntas por cima
//! da pose de `animA`. A duração de cada gesto vem do catálogo (`GESTS`; `null` = repete até parar).

use crate::anim::Pose;
use bevy::prelude::*;
use std::f32::consts::PI;

/// Gesto em andamento num avatar.
#[derive(Component, Debug, Clone)]
pub struct Gesture {
    pub id: String,
    /// Segundos desde o início.
    pub e: f32,
    /// Duração (infinita nos que repetem).
    pub dur: f32,
}

/// Duração e repetição de cada gesto, do catálogo.
#[derive(Resource, Debug, Clone, Default)]
pub struct GestureCatalog {
    pub list: Vec<GestureInfo>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct GestureInfo {
    pub id: String,
    /// Ícone (emoji) e nome mostrados no painel.
    #[serde(default)]
    pub ic: String,
    #[serde(default)]
    pub n: String,
    #[serde(default)]
    pub dur: Option<f32>,
}

impl GestureCatalog {
    pub fn from_catalog_json(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        #[derive(serde::Deserialize)]
        struct C {
            #[serde(rename = "GESTS")]
            gests: Vec<GestureInfo>,
        }
        Ok(Self { list: serde_json::from_slice::<C>(bytes)?.gests })
    }

    pub fn duration(&self, id: &str) -> Option<f32> {
        self.list.iter().find(|g| g.id == id).map(|g| g.dur.unwrap_or(f32::INFINITY))
    }
}

#[derive(Debug)]
pub struct GesturePlugin;

impl Plugin for GesturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, tick);
    }
}

fn tick(mut commands: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Gesture)>) {
    for (e, mut g) in &mut q {
        g.e += time.delta_secs();
        if g.e > g.dur {
            commands.entity(e).remove::<Gesture>();
        }
    }
}

fn flat(p: &mut Pose) {
    p.ft_l.x = -(p.lg_l.x + p.kn_l.x);
    p.ft_r.x = -(p.lg_r.x + p.kn_r.x);
}
/// Agacha mantendo os pés no chão.
fn crouch(p: &mut Pose, d: f32, st: bool) {
    if !st {
        return;
    }
    p.fig_pos.y -= d;
    let al = (1.0 - d / 0.63).clamp(0.2, 1.0).acos();
    p.lg_l.x = -al;
    p.lg_r.x = -al;
    p.kn_l.x = 2.0 * al;
    p.kn_r.x = 2.0 * al;
    flat(p);
}
/// Afasta as pernas sem tirar os pés do chão.
fn spread(p: &mut Pose, a: f32, st: bool) {
    if !st {
        return;
    }
    p.lg_l.z = -a;
    p.lg_r.z = a;
    p.fig_pos.y -= 0.72 * (1.0 - a.cos());
}
/// Chute: coxa sobe com o joelho dobrado, depois o joelho estica.
fn kick(t: f32) -> [f32; 2] {
    let g = (t * 2.0).min(1.0);
    let a = -(1.06 + 0.54 * g * g * (3.0 - 2.0 * g));
    let sh = if t <= 0.5 { 1.06 } else { 1.06 - (2.0 * t - 1.0) * 2.66 };
    [a, sh - a]
}

/// Aplica o gesto `g` no tempo `g.e`. `st` = em pé (alguns gestos só mexem nas pernas em pé).
/// As constantes são as do original (o foguete gira `t * 6.283`, não exatamente τ).
#[allow(clippy::too_many_lines, clippy::approx_constant)]
pub fn apply(p: &mut Pose, g: &Gesture, st: bool) {
    let e = g.e;
    let (s, c) = (f32::sin, f32::cos);
    let mx = f32::max;
    match g.id.as_str() {
        "arms" => {
            let d = g.dur;
            let k = (e * 2.5).min(1.0) * if e > d - 0.4 { (d - e) / 0.4 } else { 1.0 };
            let z = 0.44 + 2.4 * k + s(e * 10.0) * 0.1 * k;
            p.arm_r.z = z;
            p.arm_l.z = -z;
        }
        "scream" => {
            p.hd.x = -0.35;
            p.mouth = 2;
            p.arm_r.z = 2.5;
            p.arm_l.z = -2.5;
            p.fig_rot.z = s(e * 30.0) * 0.04;
        }
        "fart" => p.fig_rot.x = 0.3 + s(e * 25.0) * 0.05,
        "dance1" => {
            let v = s(e * 6.0);
            p.fig_rot.z = v * 0.12;
            p.fig_pos.y += v.abs() * 0.08;
            p.arm_r.z = 0.9 + (1.0 + v) * 0.9;
            p.arm_l.z = -(0.9 + (1.0 - v) * 0.9);
        }
        "dance2" => {
            p.fig_rot.y = e * 5.0;
            p.fig_pos.y += s(e * 5.0).abs() * 0.06;
            p.arm_r.z = 2.2;
            p.arm_l.z = -2.2;
        }
        "dance3" => {
            p.fig_rot.z = s(e * 4.0) * 0.2;
            p.fig_rot.y = s(e * 2.0) * 0.6;
            p.fig_pos.y += s(e * 8.0).abs() * 0.07;
            p.arm_r.z = 2.6 + s(e * 8.0) * 0.2;
            p.arm_l.z = -0.5;
            p.hd.z = s(e * 8.0) * 0.15;
        }
        "dance4" => {
            let k = ((e * 3.0).floor() as usize) % 4;
            p.arm_r.z = [1.57, 1.57, 0.3, 0.3][k];
            p.arm_l.z = -[0.3, 1.57, 1.57, 0.3][k];
            p.hd.y = [0.6, 0.0, -0.6, 0.0][k];
            p.fig_rot.y = [0.4, 0.0, -0.4, 0.0][k];
            p.lg_l.x = if k == 1 { -0.5 } else { 0.0 };
            p.lg_r.x = if k == 3 { -0.5 } else { 0.0 };
        }
        "dance5" => {
            let v = s(e * 10.0);
            p.fig_rot.z = v * 0.13;
            p.fig_rot.y = s(e * 5.0) * 0.3;
            p.fig_pos.y += v.abs() * 0.05;
            p.lg_l.x = -0.55 * mx(0.0, v);
            p.kn_l.x = 0.6 * mx(0.0, v);
            p.lg_r.x = -0.55 * mx(0.0, -v);
            p.kn_r.x = 0.6 * mx(0.0, -v);
            p.arm_r.z = 1.0 + v * 0.5;
            p.arm_l.z = -(1.0 - v * 0.5);
            p.arm_r.x = -0.5 * v;
            p.arm_l.x = 0.5 * v;
            p.hd.z = -v * 0.12;
        }
        "dance6" => {
            let v = s(e * 8.0);
            crouch(p, 0.16, st);
            p.fig_rot.z = v * 0.14;
            p.fig_rot.y = s(e * 4.0) * 0.4;
            p.arm_r.x = -0.8 + v * 0.4;
            p.arm_l.x = -0.8 - v * 0.4;
            p.arm_r.z = 0.3;
            p.arm_l.z = -0.3;
            p.hd.x = s(e * 8.0) * 0.08;
        }
        "dance7" => {
            let v = s(e * 7.0);
            crouch(p, 0.06, st);
            p.arm_r.z = 0.35 - v * 0.45;
            p.arm_l.z = -0.35 - v * 0.45;
            p.arm_r.x = 0.5;
            p.arm_l.x = 0.5;
            p.fig_rot.z = v * 0.14;
            p.hd.z = -v * 0.1;
            p.fig_rot.y = -v * 0.15;
        }
        "dance8" => {
            let k = ((e * 4.0).floor() as i32) % 5;
            p.fig_rot.y = -0.8 + k as f32 * 0.4;
            p.arm_r.z = 1.5;
            p.arm_l.x = -2.3;
            p.arm_l.z = -0.3;
            p.hd.y = 0.3;
            p.fig_pos.y += if (e * 4.0) % 1.0 < 0.2 { 0.03 } else { 0.0 };
        }
        "dance9" => {
            let b = s(e * 9.0);
            spread(p, 0.25, st);
            p.hd.x = 0.35 + b * 0.45;
            p.fig_rot.x = 0.15 + b * 0.12;
            p.fig_pos.y += b.abs() * 0.03;
            p.arm_r.z = 2.4;
            p.arm_l.z = -0.7;
            p.arm_l.x = -0.5 + b * 0.3;
        }
        "dance10" => {
            let k = s(e * 7.0);
            let (l, r) = (mx(0.0, k), mx(0.0, -k));
            p.lg_l.x = -1.5 * l;
            p.lg_r.x = -1.5 * r;
            p.kn_l.x = 0.25 * l;
            p.kn_r.x = 0.25 * r;
            p.arm_r.z = 0.9;
            p.arm_l.z = -0.9;
            p.fig_pos.y += k.abs() * 0.04;
            p.fig_rot.z = -k * 0.06;
            p.hd.z = k * 0.1;
        }
        "dance11" => {
            let (j, w) = (s(e * 13.0) * 0.08, s(e * 3.0) * 0.4);
            p.arm_r.x = -1.5 + j;
            p.arm_l.x = -1.5 - j;
            p.arm_r.z = 0.15;
            p.arm_l.z = -0.15;
            p.hd.z = 0.3 + s(e * 2.0) * 0.1;
            p.hd.x = 0.25;
            p.fig_rot.x = 0.18 + s(e * 3.0) * 0.04;
            p.fig_rot.z = s(e * 3.0) * 0.08;
            p.mouth = 1;
            p.lg_l.x = w;
            p.lg_r.x = -w;
        }
        "dance12" => {
            let h = e * 4.0;
            crouch(p, 0.05, st);
            p.fig_rot.z = s(h) * 0.16;
            p.fig_rot.x = c(h) * 0.1;
            p.arm_r.z = 1.2 + s(h + 1.0) * 0.45;
            p.arm_l.z = -(1.2 + s(h + 3.0) * 0.45);
            p.arm_r.x = -0.3;
            p.arm_l.x = -0.3;
            p.hd.z = -s(h) * 0.1;
        }
        "dance13" => {
            p.fig_rot.y = e * 8.0;
            p.fig_pos.y += 0.03;
            p.arm_r.z = 2.7;
            p.arm_l.z = -2.7;
            p.lg_r.x = -1.1;
            p.kn_r.x = 1.9;
            p.hd.x = -0.1;
        }
        "dance14" => {
            let v = s(e * 6.0);
            crouch(p, 0.32, st);
            if st {
                let (l, r) = (kick(mx(0.0, v)), kick(mx(0.0, -v)));
                p.lg_l.x = l[0];
                p.kn_l.x = l[1];
                p.lg_r.x = r[0];
                p.kn_r.x = r[1];
                flat(p);
            }
            p.arm_r.x = -1.3;
            p.arm_l.x = -1.3;
            p.arm_r.z = -0.2;
            p.arm_l.z = 0.2;
            p.fig_rot.x = 0.12;
            p.fig_rot.y = v * 0.2;
            p.fig_pos.y += v.abs() * 0.04;
        }
        "dance15" => {
            let (fl, pk) = (s(e * 14.0), mx(0.0, s(e * 3.5)));
            crouch(p, 0.1, st);
            p.arm_r.z = 1.15 + fl * 0.45;
            p.arm_l.z = -(1.15 + fl * 0.45);
            p.hd.x = 0.5 * pk;
            p.fig_rot.x = 0.15 * pk;
        }
        "dance16" => {
            let t = s(e * 7.0);
            crouch(p, 0.1 + s(e * 3.5).abs() * 0.06, st);
            p.fig_rot.y = t * 0.7;
            p.lg_l.y = -t * 0.7;
            p.lg_r.y = -t * 0.7;
            p.arm_r.z = 0.7;
            p.arm_l.z = -0.7;
            p.arm_r.x = -t * 0.5;
            p.arm_l.x = t * 0.5;
        }
        "dance17" => {
            let v = s(e * 5.0);
            p.fig_rot.x = -0.12;
            p.fig_pos.z += s(e * 2.5) * 0.12;
            p.lg_l.x = v * 0.4;
            p.lg_r.x = -v * 0.4;
            p.kn_l.x = 0.6 * mx(0.0, -v);
            p.kn_r.x = 0.6 * mx(0.0, v);
            p.arm_r.z = 0.5;
            p.arm_l.z = -0.5;
            p.arm_r.x = -v * 0.5;
            p.arm_l.x = v * 0.5;
            p.hd.x = 0.08 + s(e * 10.0) * 0.04;
        }
        "dance18" => {
            let v = s(e * 6.0);
            p.fig_pos.x += s(e * 3.0) * 0.14;
            p.fig_rot.z = s(e * 3.0) * 0.06;
            p.fig_rot.y = s(e * 1.5) * 0.25;
            p.fig_pos.y += v.abs() * 0.03;
            p.lg_l.x = -0.35 * mx(0.0, v);
            p.lg_r.x = -0.35 * mx(0.0, -v);
            p.arm_r.x = -1.2;
            p.arm_r.z = 0.5;
            p.arm_l.x = -0.9;
            p.arm_l.z = -0.1;
            p.hd.z = 0.1;
        }
        "dance19" => {
            let j = s(e * 5.0).abs();
            let q = 1.0 - j;
            p.fig_pos.y += j * 0.28;
            p.kn_l.x = 1.3 * j;
            p.kn_r.x = 1.3 * j;
            p.lg_l.x = -0.6 * j;
            p.lg_r.x = -0.6 * j;
            p.arm_r.z = 0.5 + j * 2.1;
            p.arm_l.z = -(0.5 + j * 2.1);
            p.fig_scale = Vec3::new(1.0 + q * 0.06, 1.0 - q * 0.08, 1.0 + q * 0.06);
        }
        "rocket" => {
            p.arm_r.z = 2.95;
            p.arm_l.z = -2.95;
            if e < 0.8 {
                crouch(p, 0.26 * (e / 0.4).min(1.0), st);
                p.fig_rot.z = s(e * 70.0) * 0.025;
            } else if st {
                let t = (e - 0.8) / 1.6;
                p.fig_pos.y += s(t * PI) * 0.9;
                p.fig_rot.y = t * 6.283;
                p.fig_scale = Vec3::new(0.94, 1.08, 0.94);
            }
        }
        "spell" => {
            let a = e * 8.0;
            p.arm_r.x = -1.5 + s(a) * 0.35;
            p.arm_r.z = 0.45 + c(a) * 0.35;
            p.arm_l.z = -0.65;
            p.arm_l.x = 0.25;
            p.hd.z = 0.12;
            p.hd.x = -0.1;
            p.fig_rot.y = -0.25;
            p.fig_rot.x = -0.05;
        }
        "alien" => {
            p.arm_r.z = 2.05;
            p.arm_r.x = -0.45;
            p.hd.x = s(e * 4.0) * 0.12;
            p.hd.z = 0.12;
            p.fig_rot.y = s(e * 2.0) * 0.12;
            p.mouth = 1;
        }
        "scope" => {
            p.arm_r.x = -2.2;
            p.arm_r.z = -0.4;
            p.arm_l.x = -2.0;
            p.arm_l.z = 0.5;
            p.hd.x = -0.45;
            p.fig_rot.x = -0.12;
            p.fig_rot.y = s(e * 1.3) * 0.35;
        }
        "star" => {
            let j = s(e * 3.93).abs();
            if st {
                p.fig_pos.y += j * 0.35;
            }
            p.arm_r.z = 0.44 + j * 2.1;
            p.arm_l.z = -(0.44 + j * 2.1);
            if st {
                p.lg_l.z = -0.5 * j;
                p.lg_r.z = 0.5 * j;
            }
        }
        "dance20" => {
            let a = e * 2.0;
            p.fig_pos.x += c(a) * 0.35;
            p.fig_pos.z += s(a) * 0.35;
            p.fig_rot.y = -a;
            p.fig_pos.y += s(e * 8.0).abs() * 0.04;
            p.arm_r.z = 1.57 + s(e * 4.0) * 0.15;
            p.arm_l.z = -(1.57 - s(e * 4.0) * 0.15);
            p.fig_rot.z = 0.08;
        }
        "dance21" => {
            p.fig_pos.y += 0.35 + s(e * 1.5) * 0.12;
            p.fig_rot.x = s(e * 0.9) * 0.25;
            p.fig_rot.z = s(e * 0.7) * 0.2;
            p.fig_rot.y = e * 0.4;
            p.arm_r.z = 1.3 + s(e * 1.3) * 0.4;
            p.arm_l.z = -(1.3 + s(e * 1.1 + 1.0) * 0.4);
            p.lg_l.x = s(e * 1.2) * 0.3;
            p.lg_r.x = -s(e * 1.2 + 0.5) * 0.3;
            p.kn_l.x = 0.4;
            p.kn_r.x = 0.4;
            p.hd.z = s(e * 0.8) * 0.15;
        }
        "dance22" => {
            let (a, b) = ((e * 1.5) % 1.0, (e * 1.5 + 0.5) % 1.0);
            p.arm_r.z = 2.8 - a * 2.2;
            p.arm_r.x = -0.5 * a;
            p.arm_l.z = -(2.8 - b * 2.2);
            p.arm_l.x = -0.5 * b;
            crouch(p, 0.15 * mx(0.0, (a - 0.7) / 0.3), st);
            p.fig_rot.z = 0.15 * (a - 0.5);
            p.hd.x = 0.3 * a - 0.1;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_gesture_is_implemented() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/catalogs/catalog.json");
        let cat = GestureCatalog::from_catalog_json(&std::fs::read(p).unwrap()).unwrap();
        assert_eq!(cat.list.len(), 30);
        for g in &cat.list {
            let mut a = Pose::rest(0.25);
            let before = format!("{a:?}");
            apply(&mut a, &Gesture { id: g.id.clone(), e: 0.37, dur: 3.0 }, true);
            assert_ne!(before, format!("{a:?}"), "gesto {} não mexe em nada", g.id);
        }
        assert_eq!(cat.duration("arms"), Some(3.0));
        assert_eq!(cat.duration("dance1"), Some(f32::INFINITY));
    }

    #[test]
    fn crouch_keeps_feet_flat() {
        let mut p = Pose::rest(0.25);
        crouch(&mut p, 0.2, true);
        assert!((p.ft_l.x + p.lg_l.x + p.kn_l.x).abs() < 1e-6);
        let k = kick(1.0);
        assert!((k[0] + 1.6).abs() < 1e-6);
    }
}
