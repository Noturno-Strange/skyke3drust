//! Corpo do avatar (`dress0` em `assets/app-avatar.js`): tronco (faixas de esfera `bandG` da camisa e da calça,
//! pescoço e ombros em tubos), braços (tubo e mão em bola), coxas e canelas (`tubeG`, que mudam com bermuda `bt` e
//! bota `shoes`) e os pés (elipsoides).
//!
//! A geometria é calculada como no three.js: posições em `f64` gravadas em `f32`, normais de `computeVertexNormals`
//! (somadas face a face no atributo `f32` e normalizadas no fim) ou radiais, e os triângulos expandidos na ordem dos
//! índices, separados pelo brilho como `splitGl` faz: cada junta tem até duas malhas, a normal e a brilhante.

use crate::cfg::{Cfg, Palette, Palettes, hex_to_linear};
use std::f64::consts::PI;

type V3 = [f64; 3];

const DEG: f64 = PI / 180.0;
/// Centro e raio da esfera da camisa (`TCY`, `TRR`) e da calça (`PCY`, `PRR`).
const TCY: f64 = 1.05;
const TRR: f64 = 0.36;
const PCY: f64 = 0.78;
const PRR: f64 = 0.26;

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
fn lin(hex: &str) -> [f32; 3] {
    hex_to_linear(hex).map(|v| v as f32)
}

/// Estação do tubo (`TS`): centro, raio, cor `#rrggbb` e se a parte tem cor brilhante.
#[derive(Debug, Clone)]
pub struct Station<'a> {
    pub p: V3,
    pub r: f64,
    pub c: &'a str,
    pub g: bool,
}

/// Malha de triângulos soltos (3 vértices por triângulo), cores lineares.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriMesh {
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub colors: Vec<f32>,
}

impl TriMesh {
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// `BufferGeometry` indexada (atributos `f32`), com o atributo `gl` (brilho) por vértice.
#[derive(Debug, Clone, Default)]
struct Geo {
    pos: Vec<f32>,
    nrm: Vec<f32>,
    col: Vec<f32>,
    gl: Vec<bool>,
    idx: Vec<usize>,
}

/// `computeVertexNormals` sobre a malha indexada.
fn vertex_normals(pos: &[f32], idx: &[usize]) -> Vec<f32> {
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
    nrm
}

/// `tubeG(st, rad, capS, capE, capK)`.
fn tube(st: &[Station<'_>], rad: usize, cap_s: bool, cap_e: bool, cap_k: usize) -> Geo {
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
    // anéis: centro, base, raio, cor, brilho
    type Ring<'a> = (V3, (V3, V3), f64, &'a str, bool);
    let mut rings: Vec<Ring<'_>> = Vec::new();
    let cap = |rings: &mut Vec<_>, i: usize, sg: f64, rev: bool| {
        let s = &st[i];
        for j in 1..=cap_k {
            let k = if rev { cap_k + 1 - j } else { j };
            let a = k as f64 / cap_k as f64 * PI / 2.0;
            let o = sg * s.r * a.sin();
            let c = [s.p[0] + t[i][0] * o, s.p[1] + t[i][1] * o, s.p[2] + t[i][2] * o];
            rings.push((c, f[i], s.r * a.cos(), s.c, s.g));
        }
    };
    if cap_s {
        cap(&mut rings, 0, -1.0, true);
    }
    for (i, s) in st.iter().enumerate() {
        rings.push((s.p, f[i], s.r, s.c, s.g));
    }
    if cap_e {
        cap(&mut rings, n - 1, 1.0, false);
    }
    let nv = rings.len() * rad;
    let mut g = Geo { pos: vec![0f32; nv * 3], col: vec![0f32; nv * 3], gl: vec![false; nv], ..Geo::default() };
    for (j, (c, (nn, b), r, h, gl)) in rings.iter().enumerate() {
        let lin = lin(h);
        for k in 0..rad {
            let a = k as f64 / rad as f64 * PI * 2.0;
            let (cs, sn) = (a.cos(), a.sin());
            let v = j * rad + k;
            for d in 0..3 {
                g.pos[v * 3 + d] = (c[d] + r * (cs * nn[d] + sn * b[d])) as f32;
            }
            g.col[v * 3..v * 3 + 3].copy_from_slice(&lin);
            g.gl[v] = *gl;
        }
    }
    for j in 0..rings.len().saturating_sub(1) {
        for k in 0..rad {
            let a = j * rad + k;
            let b = j * rad + (k + 1) % rad;
            g.idx.extend_from_slice(&[a, b, a + rad, b, b + rad, a + rad]);
        }
    }
    g.nrm = vertex_normals(&g.pos, &g.idx);
    g
}

/// `bandG(cy, R, angs, segs, c, gl)`: faixa de esfera (`LatheGeometry`) entre ângulos polares, com normais radiais.
fn band(cy: f64, r: f64, angs: &[f64], segs: usize, c: &str, gl: bool) -> Geo {
    let pts: Vec<(f64, f64)> =
        angs.iter().map(|a| ((r * (a * DEG).sin()).max(0.0), cy + r * (a * DEG).cos())).collect();
    let np = pts.len();
    let mut g = Geo::default();
    let lin = lin(c);
    let inv = 1.0 / segs as f64;
    for i in 0..=segs {
        let phi = i as f64 * inv * (PI * 2.0);
        let (sn, cs) = (phi.sin(), phi.cos());
        for &(px, py) in &pts {
            let v = [(px * sn) as f32, py as f32, (px * cs) as f32];
            g.pos.extend_from_slice(&v);
            let (x, y, z) = (f64::from(v[0]), f64::from(v[1]) - cy, f64::from(v[2]));
            let l = (x * x + y * y + z * z).sqrt();
            let l = if l == 0.0 { 1.0 } else { l };
            g.nrm.extend_from_slice(&[(x / l) as f32, (y / l) as f32, (z / l) as f32]);
            g.col.extend_from_slice(&lin);
            g.gl.push(gl);
        }
    }
    for i in 0..segs {
        for j in 0..np - 1 {
            let base = j + i * np;
            let (a, b, c, d) = (base, base + np, base + np + 1, base + 1);
            g.idx.extend_from_slice(&[a, b, d, c, d, b]);
        }
    }
    g
}

/// `SphereGeometry(1, w, h)`: posições, normais e índices (sem cor).
pub fn sphere(w: usize, h: usize) -> (Vec<f32>, Vec<f32>, Vec<u32>) {
    let (mut pos, mut nrm, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    for iy in 0..=h {
        let v = iy as f64 / h as f64;
        for ix in 0..=w {
            let u = ix as f64 / w as f64;
            let p = [-(u * PI * 2.0).cos() * (v * PI).sin(), (v * PI).cos(), (u * PI * 2.0).sin() * (v * PI).sin()];
            let p32 = p.map(|x| x as f32);
            pos.extend_from_slice(&p32);
            nrm.extend(normalize(p32.map(f64::from)).map(|x| x as f32));
        }
    }
    let at = |iy: usize, ix: usize| (iy * (w + 1) + ix) as u32;
    for iy in 0..h {
        for ix in 0..w {
            let (a, b, c, d) = (at(iy, ix + 1), at(iy, ix), at(iy + 1, ix), at(iy + 1, ix + 1));
            if iy != 0 {
                idx.extend_from_slice(&[a, b, d]);
            }
            if iy != h - 1 {
                idx.extend_from_slice(&[b, c, d]);
            }
        }
    }
    (pos, nrm, idx)
}

/// `glBalls([[rx, ry, rz, c, x, y, z]], SphereGeometry(1, w, h), gl)` (`ballsGeo`): elipsoides numa geometria só.
fn balls(a: &[([f64; 3], &str, V3)], w: usize, h: usize, gl: bool) -> Geo {
    let (sp, sn, si) = sphere(w, h);
    let mut g = Geo::default();
    for (r, c, o) in a {
        let base = g.gl.len();
        let lin = lin(c);
        for (p, n) in sp.chunks_exact(3).zip(sn.chunks_exact(3)) {
            for d in 0..3 {
                g.pos.push((f64::from(p[d]) * r[d] + o[d]) as f32);
            }
            let q = [f64::from(n[0]) / r[0], f64::from(n[1]) / r[1], f64::from(n[2]) / r[2]];
            let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
            let l = if l == 0.0 { 1.0 } else { l };
            g.nrm.extend(q.map(|x| (x / l) as f32));
            g.col.extend_from_slice(&lin);
            g.gl.push(gl);
        }
        g.idx.extend(si.iter().map(|&i| base + i as usize));
    }
    g
}

/// `splitGl`: junta as peças e separa os triângulos pelo brilho do primeiro vértice (0 = normal, 1 = brilhante).
fn split_gl(gs: &[Geo]) -> [TriMesh; 2] {
    let mut out = [TriMesh::default(), TriMesh::default()];
    for g in gs {
        for tri in g.idx.chunks_exact(3) {
            let m = &mut out[usize::from(g.gl[tri[0]])];
            for &v in tri {
                m.positions.extend_from_slice(&g.pos[v * 3..v * 3 + 3]);
                m.normals.extend_from_slice(&g.nrm[v * 3..v * 3 + 3]);
                m.colors.extend_from_slice(&g.col[v * 3..v * 3 + 3]);
            }
        }
    }
    out
}

/// Peças de uma junta (`limb`): `hi` de perto e `lo` de longe (também a forma do contorno), cada uma com a malha
/// normal (`[0]`) e a brilhante (`[1]`), que podem estar vazias.
#[derive(Debug, Clone)]
pub struct Limb {
    pub hi: [TriMesh; 2],
    pub lo: [TriMesh; 2],
}

fn limb(mk: impl Fn(bool) -> Vec<Geo>) -> Limb {
    Limb { hi: split_gl(&mk(true)), lo: split_gl(&mk(false)) }
}

/// Pé: elipsoide `SphereGeometry(1, 10, 7)` com escala e centro no nó do pé; o contorno usa `SphereGeometry(1, 8, 6)`.
#[derive(Debug, Clone, PartialEq)]
pub struct Foot {
    pub radii: [f32; 3],
    pub center: [f32; 3],
    /// Cor do calçado (linear).
    pub color: [f32; 3],
    pub gloss: bool,
}

/// Corpo inteiro de um visual: as juntas na hierarquia do original (tronco em `fig`, braços em `armL`/`armR`,
/// coxas em `lgL`/`lgR`, canelas em `knL`/`knR`, pés em `ftL`/`ftR`).
#[derive(Debug, Clone)]
pub struct Body {
    pub torso: Limb,
    pub arm: Limb,
    pub thigh: Limb,
    pub shin: Limb,
    pub foot: Foot,
    /// Orelhas: cor (linear) e brilho; a forma é a do avatar de referência.
    pub ear_color: [f32; 3],
    pub ear_gloss: bool,
    /// Cabeça com cor brilhante (pele `tb`).
    pub head_gloss: bool,
    /// Altura do corpo acima dos pés (`userData.fy`).
    pub fy: f32,
}

/// `dress0` sem a cabeça: tronco, braços, pernas e pés do visual.
pub fn body(p: &Palettes, c: &Cfg) -> Body {
    let (bt, bo) = (c.get("bt") == 1, c.get("shoes") == 1);
    let (sb, pb, cb, tb) = (c.flag("sb"), c.flag("pb"), c.flag("cb"), c.flag("tb"));
    let sk = p.color(Palette::Skin, c.get("skin"), tb);
    let sh = p.color(Palette::Pal, c.get("shirt"), sb);
    let pc = p.color(Palette::Pal, c.get("pc"), pb);
    let sc = p.color(Palette::Pal, c.get("sc"), cb);
    let ts = |x: f64, y: f64, r: f64, c, g| Station { p: [x, y, 0.0], r, c, g };
    // ponto onde as esferas da camisa e da calça se cruzam, e os ângulos de cada uma nesse ponto
    let yi = TCY - (TRR * TRR - PRR * PRR + (TCY - PCY).powi(2)) / (2.0 * (TCY - PCY));
    let tth = ((yi - TCY) / TRR).acos() / DEG;
    let pth = ((yi - PCY) / PRR).acos() / DEG;
    let torso = limb(|q| {
        let (n, rd, ck) = if q { (14, 6, 2) } else { (10, 5, 1) };
        let ta: &[f64] = if q { &[tth, 112.0, 90.0, 68.0, 45.0, 22.0, 0.0] } else { &[tth, 100.0, 60.0, 25.0, 0.0] };
        let pa: &[f64] = if q { &[180.0, 152.0, 125.0, pth] } else { &[180.0, 135.0, pth] };
        let mut g = vec![
            band(TCY, TRR, ta, n, sh, sb),
            band(PCY, PRR, pa, n, pc, pb),
            tube(&[ts(0.0, 1.33, 0.075, sk, tb), ts(0.0, 1.6, 0.075, sk, tb)], rd, false, false, 0),
        ];
        for s in [-1.0, 1.0] {
            let pp = [s * 0.357, 1.34, 0.0];
            let d = normalize(sub([0.0, TCY, 0.0], pp));
            let i = [d[0] * 0.26 + pp[0], d[1] * 0.26 + pp[1]];
            g.push(tube(&[ts(i[0], i[1], 0.095, sh, sb), ts(pp[0], pp[1], 0.095, sh, sb)], rd, false, true, ck));
        }
        g
    });
    let arm = limb(|q| {
        let st = [
            ts(0.0, 0.0, 0.095, sh, sb),
            ts(0.0, -0.28, 0.095, sh, sb),
            ts(0.0, -0.28, 0.074, sh, sb),
            ts(0.0, -0.28, 0.074, sk, tb),
            ts(0.0, -0.66, 0.058, sk, tb),
        ];
        let (w, h) = if q { (10, 7) } else { (8, 6) };
        vec![
            tube(&st, if q { 8 } else { 5 }, true, false, if q { 3 } else { 1 }),
            balls(&[([0.098; 3], sk, [0.0, -0.68, 0.0])], w, h, tb),
        ]
    });
    // coxa: calota no quadril e no joelho; bermuda: a pele começa em -0,2
    let thigh = limb(|q| {
        let mut s = vec![ts(0.0, 0.0, 0.085, pc, pb)];
        if bt {
            s.extend([ts(0.0, -0.2, 0.08, pc, pb), ts(0.0, -0.2, 0.08, sk, tb)]);
        }
        s.push(if bt { ts(0.0, -0.38, 0.075, sk, tb) } else { ts(0.0, -0.38, 0.075, pc, pb) });
        vec![tube(&s, if q { 6 } else { 5 }, true, true, if q { 2 } else { 1 })]
    });
    // canela: calota no joelho e no tornozelo; bota: cano na cor do calçado
    let shin = limb(|q| {
        let (top, tg) = if bt { (sk, tb) } else { (pc, pb) };
        let mut s = vec![ts(0.0, 0.0, 0.074, top, tg)];
        if bo {
            s.extend([ts(0.0, -0.18, 0.069, top, tg), ts(0.0, -0.18, 0.069, sc, cb)]);
        }
        s.push(if bo { ts(0.0, -0.39, 0.064, sc, cb) } else { ts(0.0, -0.39, 0.064, top, tg) });
        vec![tube(&s, if q { 6 } else { 5 }, true, true, if q { 2 } else { 1 })]
    });
    let (radii, center) =
        if bo { ([0.115, 0.115, 0.14], [0.0, 0.0, 0.02]) } else { ([0.125, 0.09, 0.17], [0.0, -0.01, 0.05]) };
    Body {
        torso,
        arm,
        thigh,
        shin,
        foot: Foot { radii, center, color: lin(sc), gloss: cb },
        ear_color: lin(sk),
        ear_gloss: tb,
        head_gloss: tb,
        fy: fy(c),
    }
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
    fn body_matches_browser_export() {
        let (cfg, meshes) = reference();
        assert_eq!((cfg.get("bt"), cfg.get("shoes")), (1, 1));
        let b = body(&palettes(), &cfg);
        // nós do GLB: malha 1 = canela (knL), 2 = coxa (lgL), 5 = braço (armL), 7 = tronco (fig); o export usa a
        // geometria de longe, e o visual de referência não tem cor brilhante
        for (name, got, want) in [
            ("canela", &b.shin, &meshes[1]),
            ("coxa", &b.thigh, &meshes[2]),
            ("braço", &b.arm, &meshes[5]),
            ("braço direito", &b.arm, &meshes[6]),
            ("tronco", &b.torso, &meshes[7]),
        ] {
            assert!(got.lo[1].is_empty() && got.hi[1].is_empty(), "{name}: brilho");
            let got = &got.lo[0];
            assert_eq!(got.positions.len(), want.positions.len(), "{name}");
            let close = |a: &[f32], b: &[f32], eps: f32| a.iter().zip(b).all(|(x, y)| (x - y).abs() <= eps);
            assert!(close(&got.positions, &want.positions, 1e-7), "{name}: posições");
            assert!(close(&got.normals, &want.normals, 1e-6), "{name}: normais");
            assert!(close(&got.colors, &want.colors, 1e-7), "{name}: cores");
        }
        // pé: a esfera unitária SphereGeometry(1, 10, 7) (malha 0, indexada no GLB)
        let (sp, sn, _) = sphere(10, 7);
        assert!(sp.iter().zip(&meshes[0].positions).all(|(a, b)| (a - b).abs() <= 1e-7));
        assert!(sn.iter().zip(&meshes[0].normals).all(|(a, b)| (a - b).abs() <= 1e-7));
        assert_eq!(b.fy, 0.261);
    }

    #[test]
    fn shorts_and_boots_change_rings() {
        let p = palettes();
        let mut c = Cfg::default();
        for (k, v) in [("bt", 0), ("shoes", 0)] {
            c.0.insert(k.into(), v);
        }
        let plain = body(&p, &c);
        c.0.insert("bt".into(), 1);
        c.0.insert("shoes".into(), 1);
        let both = body(&p, &c);
        // perto: 6 lados, calotas de 2 anéis; cada estação a mais acrescenta 6 × 2 triângulos
        assert_eq!(plain.thigh.hi[0].positions.len() / 9, (2 + 2 + 2 - 1) * 6 * 2);
        assert_eq!(both.thigh.hi[0].positions.len() / 9, (2 + 4 + 2 - 1) * 6 * 2);
        assert_ne!(plain.foot.radii, both.foot.radii);
        assert!((fy(&c) - 0.261).abs() < 1e-6);
    }

    #[test]
    fn glossy_parts_split() {
        let p = palettes();
        let mut c = Cfg::default();
        // camisa brilhante: o tronco divide entre a camisa (brilho) e calça e pescoço (normal); a mão de pele fica normal
        c.0.insert("sb".into(), 1);
        let b = body(&p, &c);
        assert!(!b.torso.hi[0].is_empty() && !b.torso.hi[1].is_empty());
        assert!(!b.arm.hi[0].is_empty() && !b.arm.hi[1].is_empty());
        assert!(b.thigh.hi[1].is_empty());
        let n = |l: &Limb| l.hi[0].positions.len() + l.hi[1].positions.len();
        assert_eq!(n(&b.torso), n(&body(&p, &Cfg::default()).torso));
    }
}
