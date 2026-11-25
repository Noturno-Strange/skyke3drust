//! Pernas do avatar: coxa e canela em tubos (`tubeG` em `assets/app-avatar.js`) e o pé (elipsoide), que mudam com
//! bermuda (`bt`) e bota (`shoes`).
//!
//! A geometria é calculada como no three.js: posições em `f64` gravadas em `f32`, normais de `computeVertexNormals`
//! (somadas face a face no atributo `f32` e normalizadas no fim) e os triângulos expandidos na ordem dos índices,
//! como `splitGl` faz antes de montar a malha.

use crate::cfg::{Cfg, Palette, Palettes, hex_to_linear};
use std::f64::consts::PI;

type V3 = [f64; 3];

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
/// `Vector3.normalize` do three.js: multiplica por `1 / (comprimento || 1)`.
fn normalize(a: V3) -> V3 {
    let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
    let k = 1.0 / if l == 0.0 { 1.0 } else { l };
    [a[0] * k, a[1] * k, a[2] * k]
}

/// Estação do tubo (`TS`): centro, raio e cor `#rrggbb`.
#[derive(Debug, Clone)]
pub struct Station<'a> {
    pub p: V3,
    pub r: f64,
    pub c: &'a str,
}

/// Malha de triângulos soltos (3 vértices por triângulo), cores lineares.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub colors: Vec<f32>,
}

/// `tubeG(st, rad, capS, capE, capK)`.
pub fn tube(st: &[Station<'_>], rad: usize, cap_s: bool, cap_e: bool, cap_k: usize) -> TriMesh {
    let n = st.len();
    let t: Vec<V3> = (0..n).map(|i| normalize(sub(st[(i + 1).min(n - 1)].p, st[i.saturating_sub(1)].p))).collect();
    let f: Vec<(V3, V3)> = t
        .iter()
        .map(|&t| {
            let axis = if t[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] };
            let nn = normalize(cross(t, axis));
            (nn, cross(t, nn))
        })
        .collect();
    // anéis: centro, base, raio, cor
    let mut rings: Vec<(V3, (V3, V3), f64, &str)> = Vec::new();
    let cap = |rings: &mut Vec<_>, i: usize, sg: f64, rev: bool| {
        let s = &st[i];
        for j in 1..=cap_k {
            let k = if rev { cap_k + 1 - j } else { j };
            let a = k as f64 / cap_k as f64 * PI / 2.0;
            let o = sg * s.r * a.sin();
            let c = [s.p[0] + t[i][0] * o, s.p[1] + t[i][1] * o, s.p[2] + t[i][2] * o];
            rings.push((c, f[i], s.r * a.cos(), s.c));
        }
    };
    if cap_s {
        cap(&mut rings, 0, -1.0, true);
    }
    for (i, s) in st.iter().enumerate() {
        rings.push((s.p, f[i], s.r, s.c));
    }
    if cap_e {
        cap(&mut rings, n - 1, 1.0, false);
    }
    let nv = rings.len() * rad;
    let mut pos = vec![0f32; nv * 3];
    let mut col = vec![0f32; nv * 3];
    for (j, (c, (nn, b), r, h)) in rings.iter().enumerate() {
        let lin = hex_to_linear(h).map(|v| v as f32);
        for k in 0..rad {
            let a = k as f64 / rad as f64 * PI * 2.0;
            let (cs, sn) = (a.cos(), a.sin());
            let o = (j * rad + k) * 3;
            for d in 0..3 {
                pos[o + d] = (c[d] + r * (cs * nn[d] + sn * b[d])) as f32;
            }
            col[o..o + 3].copy_from_slice(&lin);
        }
    }
    let mut idx = Vec::new();
    for j in 0..rings.len().saturating_sub(1) {
        for k in 0..rad {
            let a = j * rad + k;
            let b = j * rad + (k + 1) % rad;
            idx.extend_from_slice(&[a, b, a + rad, b, b + rad, a + rad]);
        }
    }
    expand(&pos, &col, &idx)
}

/// `computeVertexNormals` sobre a malha indexada e expansão dos triângulos na ordem dos índices.
fn expand(pos: &[f32], col: &[f32], idx: &[usize]) -> TriMesh {
    let p = |v: usize| [f64::from(pos[v * 3]), f64::from(pos[v * 3 + 1]), f64::from(pos[v * 3 + 2])];
    let mut nrm = vec![0f32; pos.len()];
    for tri in idx.chunks_exact(3) {
        let (a, b, c) = (tri[0], tri[1], tri[2]);
        let fc = cross(sub(p(c), p(b)), sub(p(a), p(b)));
        for v in [a, b, c] {
            for d in 0..3 {
                nrm[v * 3 + d] = (f64::from(nrm[v * 3 + d]) + fc[d]) as f32;
            }
        }
    }
    for v in nrm.chunks_exact_mut(3) {
        let n = normalize([f64::from(v[0]), f64::from(v[1]), f64::from(v[2])]);
        for d in 0..3 {
            v[d] = n[d] as f32;
        }
    }
    let mut m = TriMesh::default();
    for &v in idx {
        m.positions.extend_from_slice(&pos[v * 3..v * 3 + 3]);
        m.normals.extend_from_slice(&nrm[v * 3..v * 3 + 3]);
        m.colors.extend_from_slice(&col[v * 3..v * 3 + 3]);
    }
    m
}

/// Coxa, canela e pé de um visual.
#[derive(Debug, Clone)]
pub struct Legs {
    pub thigh: TriMesh,
    pub shin: TriMesh,
    /// Elipsoide do pé: raios e centro no nó do pé.
    pub foot_radii: [f32; 3],
    pub foot_center: [f32; 3],
    /// Cor do calçado (linear).
    pub foot_color: [f32; 3],
}

/// Pernas como em `dress0`: `hi` é a geometria de perto (`mk(1)`), senão a de longe (contorno e LOD).
pub fn legs(p: &Palettes, c: &Cfg, hi: bool) -> Legs {
    let bt = c.get("bt") == 1;
    let bo = c.get("shoes") == 1;
    let sk = p.color(Palette::Skin, c.get("skin"), c.flag("tb"));
    let pc = p.color(Palette::Pal, c.get("pc"), c.flag("pb"));
    let sc = p.color(Palette::Pal, c.get("sc"), c.flag("cb"));
    let ts = |y: f64, r: f64, c| Station { p: [0.0, y, 0.0], r, c };
    let (rad, ck) = if hi { (6, 2) } else { (5, 1) };
    // coxa: calota no quadril e no joelho; bermuda: a pele começa em -0,2
    let mut s = vec![ts(0.0, 0.085, pc)];
    if bt {
        s.extend([ts(-0.2, 0.08, pc), ts(-0.2, 0.08, sk)]);
    }
    s.push(ts(-0.38, 0.075, if bt { sk } else { pc }));
    let thigh = tube(&s, rad, true, true, ck);
    // canela: calota no joelho e no tornozelo; bota: cano na cor do calçado
    let top = if bt { sk } else { pc };
    let mut s = vec![ts(0.0, 0.074, top)];
    if bo {
        s.extend([ts(-0.18, 0.069, top), ts(-0.18, 0.069, sc)]);
    }
    s.push(ts(-0.39, 0.064, if bo { sc } else { top }));
    let shin = tube(&s, rad, true, true, ck);
    let (foot_radii, foot_center) =
        if bo { ([0.115, 0.115, 0.14], [0.0, 0.0, 0.02]) } else { ([0.125, 0.09, 0.17], [0.0, -0.01, 0.05]) };
    Legs { thigh, shin, foot_radii, foot_center, foot_color: hex_to_linear(sc).map(|v| v as f32) }
}

/// Altura do corpo acima dos pés (`userData.fy`): a bota é mais alta.
pub fn fy(c: &Cfg) -> f32 {
    0.235 + if c.get("shoes") == 1 { 0.026 } else { 0.012 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cfg::tests_support::palettes;
    use std::collections::BTreeMap;

    /// Malhas do avatar de referência exportado pelo navegador (`assets/avatars/default.glb`).
    fn reference() -> (Cfg, Vec<TriMesh>) {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/avatars");
        let b = std::fs::read(dir.join("default.glb")).unwrap();
        let jl = u32::from_le_bytes(b[12..16].try_into().unwrap()) as usize;
        let j: serde_json::Value = serde_json::from_slice(&b[20..20 + jl]).unwrap();
        let bin = &b[20 + jl + 8..];
        let acc = |i: &serde_json::Value| -> Vec<f32> {
            let a = &j["accessors"][i.as_u64().unwrap() as usize];
            let v = &j["bufferViews"][a["bufferView"].as_u64().unwrap() as usize];
            let off = v["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
            let n = a["count"].as_u64().unwrap() as usize * 3;
            (0..n).map(|k| f32::from_le_bytes(bin[off + k * 4..off + k * 4 + 4].try_into().unwrap())).collect()
        };
        let meshes = j["meshes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| {
                let a = &m["primitives"][0]["attributes"];
                TriMesh {
                    positions: acc(&a["POSITION"]),
                    normals: acc(&a["NORMAL"]),
                    colors: if a["COLOR_0"].is_null() { vec![] } else { acc(&a["COLOR_0"]) },
                }
            })
            .collect();
        let d: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("default.json")).unwrap()).unwrap();
        let raw: BTreeMap<String, u32> = d["cfg"]
            .as_object()
            .unwrap()
            .iter()
            .filter_map(|(k, v)| v.as_u64().map(|n| (k.clone(), n as u32)))
            .collect();
        (palettes().clean(&raw).unwrap(), meshes)
    }

    #[test]
    fn legs_match_browser_export() {
        let (cfg, meshes) = reference();
        assert_eq!((cfg.get("bt"), cfg.get("shoes")), (1, 1));
        let l = legs(&palettes(), &cfg, false);
        // nós do GLB: malha 1 = canela (filha de knL), malha 2 = coxa (filha de lgL); o export usa a geometria de longe
        for (got, want) in [(&l.shin, &meshes[1]), (&l.thigh, &meshes[2])] {
            assert_eq!(got.positions.len(), want.positions.len());
            let close = |a: &[f32], b: &[f32], eps: f32| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps);
            assert!(close(&got.positions, &want.positions, 1e-7), "posições");
            assert!(close(&got.normals, &want.normals, 1e-6), "normais");
            assert!(close(&got.colors, &want.colors, 1e-7), "cores");
        }
        assert_eq!(fy(&cfg), 0.261);
    }

    #[test]
    fn shorts_and_boots_change_rings() {
        let p = palettes();
        let mut c = Cfg::default();
        for (k, v) in [("bt", 0), ("shoes", 0)] {
            c.0.insert(k.into(), v);
        }
        let plain = legs(&p, &c, true);
        c.0.insert("bt".into(), 1);
        c.0.insert("shoes".into(), 1);
        let both = legs(&p, &c, true);
        // perto: 6 lados, calotas de 2 anéis; cada estação a mais acrescenta 6 × 2 triângulos
        assert_eq!(plain.thigh.positions.len() / 9, (2 + 2 + 2 - 1) * 6 * 2);
        assert_eq!(both.thigh.positions.len() / 9, (2 + 4 + 2 - 1) * 6 * 2);
        assert_ne!(plain.foot_radii, both.foot_radii);
        assert!((fy(&c) - 0.261).abs() < 1e-6);
    }
}
