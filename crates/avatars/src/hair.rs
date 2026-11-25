//! Cabelo e chapéus.
//!
//! A geometria de cada estilo (`hs`) e de cada chapéu depende só do estilo e vem do jogo original, exportada pelo
//! `tools/asset-exporter` (`assets/avatars/hair/hs-NN.bin`, `hat-N.bin`). A cor é aplicada por triângulo, a partir do
//! centro de cada triângulo e de um fator de variação (`paintHair` + `col0` e variações por estilo em `hairGeo`, e o
//! `col` de `hatGeo`), portada aqui e conferida com as cores que o navegador gera (`paint-NN.bin`).

use crate::cfg::{Cfg, Palette, Palettes, hex_to_linear};

const HR: [f64; 3] = [0.36, 0.38, 0.34];
const HEDGE: [f64; 6] = [1.0, 1.05, 1.3, 1.4, 1.75, 2.0];
const MAGIC: u32 = 0x4859_4b53; // "SKYH"

/// Geometria exportada de um estilo de cabelo ou de um chapéu.
#[derive(Debug, Clone)]
pub struct HairShape {
    /// Triângulos soltos: 9 floats (3 vértices) por triângulo.
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    /// Centro de cada triângulo (para a cor).
    pub centers: Vec<f32>,
    /// Variação de brilho de cada triângulo.
    pub jitter: Vec<f32>,
    /// Malha grossa do contorno: posições e direção de expansão (`onrm`).
    pub lo_positions: Vec<f32>,
    pub lo_offsets: Vec<f32>,
}

#[derive(Debug, thiserror::Error)]
pub enum HairError {
    #[error("arquivo de cabelo inválido")]
    Format,
}

impl HairShape {
    pub fn triangles(&self) -> usize {
        self.jitter.len()
    }

    /// Lê o binário do exportador (`SKYH`, versão 1).
    pub fn parse(b: &[u8]) -> Result<Self, HairError> {
        let u32_at = |o: usize| b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]));
        if u32_at(0) != Some(MAGIC) || u32_at(4) != Some(1) {
            return Err(HairError::Format);
        }
        let n = u32_at(8).ok_or(HairError::Format)? as usize;
        let m = u32_at(12).ok_or(HairError::Format)? as usize;
        // limite generoso contra arquivo corrompido (os estilos têm até ~10 mil triângulos)
        if n > 200_000 || m > 200_000 {
            return Err(HairError::Format);
        }
        let mut o = 16;
        let mut take = |len: usize| -> Result<Vec<f32>, HairError> {
            let end = o + len * 4;
            let s = b.get(o..end).ok_or(HairError::Format)?;
            o = end;
            Ok(s.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
        };
        Ok(Self {
            positions: take(9 * n)?,
            normals: take(9 * n)?,
            centers: take(3 * n)?,
            jitter: take(n)?,
            lo_positions: take(9 * m)?,
            lo_offsets: take(9 * m)?,
        })
    }
}

// ---- funções do original ----
fn fract(x: f64) -> f64 {
    x - x.floor()
}
fn hh(n: f64) -> f64 {
    fract((n * 127.1 + 311.7).sin() * 43758.5453)
}
fn hl(i: f64, j: f64, k: f64) -> f64 {
    hh(i * 12.9898 + j * 78.233 + k * 37.719)
}
fn vn(x: f64, y: f64, z: f64) -> f64 {
    let (i, j, k) = (x.floor(), y.floor(), z.floor());
    let (u, v, w) = (x - i, y - j, z - k);
    let (a, b, d) = (u * u * (3.0 - 2.0 * u), v * v * (3.0 - 2.0 * v), w * w * (3.0 - 2.0 * w));
    let m = |p: f64, q: f64, t: f64| p + (q - p) * t;
    let l = |p: f64, q: f64, r: f64| hl(i + p, j + q, k + r);
    m(
        m(m(l(0., 0., 0.), l(1., 0., 0.), a), m(l(0., 1., 0.), l(1., 1., 0.), a), b),
        m(m(l(0., 0., 1.), l(1., 0., 1.), a), m(l(0., 1., 1.), l(1., 1., 1.), a), b),
        d,
    )
}
fn hang(ph: f64) -> f64 {
    let a = ph - std::f64::consts::FRAC_PI_2;
    a.sin().atan2(a.cos()).abs()
}
fn hedge(e: &[f64; 6], a: f64) -> f64 {
    let k = [0.0, 0.9, 1.35, 1.57, 2.1, std::f64::consts::PI];
    for i in 1..6 {
        if a <= k[i] {
            let t = (a - k[i - 1]) / (k[i] - k[i - 1]);
            return e[i - 1] + (e[i] - e[i - 1]) * t * t * (3.0 - 2.0 * t);
        }
    }
    e[5]
}
fn sd_e(x: f64, y: f64, z: f64, r: [f64; 3]) -> f64 {
    ((x / r[0]).powi(2) + (y / r[1]).powi(2) + (z / r[2]).powi(2)).sqrt().mul_add(1.0, -1.0) * r[0].min(r[1]).min(r[2])
}
fn hd(x: f64, y: f64, z: f64) -> f64 {
    sd_e(x, y, z, HR)
}
fn hsp(th: f64, ph: f64, o: f64) -> [f64; 3] {
    [th.sin() * ph.cos() * (HR[0] + o), th.cos() * (HR[1] + o), th.sin() * ph.sin() * (HR[2] + o)]
}
fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}
fn clamp01(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// Esferas do estilo "Confete" (20): `fibL(70, 1.9, ...)` com as que ficam dentro da linha do cabelo.
fn confetti_spheres() -> Vec<[f64; 4]> {
    let (n, tmax) = (70, 1.9f64);
    let mut out = Vec::new();
    for k in 0..n {
        let th = (1.0 - (1.0 - tmax.cos()) * (f64::from(k) + 0.5) / f64::from(n)).acos();
        let ph = f64::from(k) * 2.39996;
        if th < hedge(&HEDGE, hang(ph)) - 0.08 {
            let p = hsp(th, ph, 0.2);
            out.push([p[0], p[1], p[2], 0.1]);
        }
    }
    out
}

/// Cor (linear) de cada triângulo multiplicada pela variação, como `paintHair`: 9 floats por triângulo.
pub fn paint(shape: &HairShape, pal: &Palettes, c: &Cfg) -> Vec<f32> {
    let n = shape.triangles();
    let hat = c.get("hat");
    let col: Box<dyn Fn(f64, f64, f64) -> [f64; 3]> = if hat > 0 {
        let gloss = c.flag("hb");
        let c1 = hex_to_linear(pal.color(Palette::Pal, c.get("hc"), gloss));
        let c2 = hex_to_linear(pal.color(Palette::Pal, (c.get("hc") + 5) % pal.pal.len() as u32, gloss));
        if hat == 1 {
            const E2: [f64; 6] = [1.0, 1.05, 1.4, 1.75, 2.0, 2.05];
            Box::new(move |x, y, z| {
                let (qx, qy, qz) = (x / HR[0], y / HR[1], z / HR[2]);
                let l = (qx * qx + qy * qy + qz * qz).sqrt().max(f64::MIN_POSITIVE);
                let l = if l == 0.0 { 1.0 } else { l };
                let th = (qy / l).clamp(-1.0, 1.0).acos();
                let ph = qz.atan2(qx);
                let e = hedge(&E2, hang(ph));
                if th > e - 0.16 || y > 0.46 { c2.map(|v| v * (0.9 + 0.1 * (ph * 30.0).sin())) } else { c1 }
            })
        } else {
            Box::new(move |x, y, z| {
                let b = 0.23 - 0.12 * (0.0f64).max(-z / 0.4 + x.abs() / 0.4 * 0.3);
                let r = x.hypot(z);
                if y - b < 0.085 && y > b - 0.02 && r < 0.5 { c2 } else { c1 }
            })
        }
    } else {
        let c1 = hex_to_linear(pal.color(Palette::Hair, c.get("hair"), c.flag("rb")));
        let c2 = lerp3(c1, [1.0; 3], 0.12);
        let sk = hex_to_linear(pal.color(Palette::Skin, c.get("skin"), c.flag("tb")));
        let shv = lerp3(sk, c1, 0.5);
        let col0 = move |x: f64, y: f64, z: f64| {
            let d = hd(x, y, z);
            let ph = z.atan2(x);
            let th = x.hypot(z).atan2(y);
            let st = 0.5 + 0.5 * (ph * 6.0 + th * 3.0 + vn(x * 3.0, y * 3.0, z * 3.0) * 2.5).sin();
            let sh = (0.78 + 0.22 * clamp01(d / 0.14)) * (0.92 + 0.16 * st);
            lerp3(c1, c2, 0.4 * clamp01((d - 0.03) / 0.3)).map(|v| v * sh)
        };
        match c.get("hs") {
            3 | 17 => Box::new(move |x, y, z| lerp3(col0(x, y, z), shv, if x.abs() > 0.13 { 0.7 } else { 0.0 })),
            7 | 31 => Box::new(move |x, y, z| {
                lerp3(col0(x, y, z), shv, if y < 0.18 && hd(x, y, z) < 0.03 { 0.7 } else { 0.0 })
            }),
            24 => Box::new(move |x, y, z| {
                lerp3(col0(x, y, z), shv, if y < 0.14 && hd(x, y, z) < 0.04 { 0.7 } else { 0.0 })
            }),
            25 => Box::new(move |x, y, z| lerp3(col0(x, y, z), shv, 0.75)),
            20 => {
                let r = confetti_spheres();
                let colors: Vec<[f64; 3]> = (0..r.len()).map(|b| hex_to_linear(&pal.hairc[(b * 5 + 3) % 12])).collect();
                Box::new(move |x, y, z| {
                    // o original escolhe a ÚLTIMA esfera que contém o ponto (a condição `d < bd + 9` é sempre verdadeira)
                    let mut b = None;
                    for (i, q) in r.iter().enumerate() {
                        let d = ((x - q[0]).powi(2) + (y - q[1]).powi(2) + (z - q[2]).powi(2)).sqrt();
                        if d < q[3] + 0.04 {
                            b = Some(i);
                        }
                    }
                    b.map_or(c1, |i| colors[i])
                })
            }
            _ => Box::new(col0),
        }
    };
    let mut out = Vec::with_capacity(n * 9);
    for t in 0..n {
        let (x, y, z) =
            (f64::from(shape.centers[t * 3]), f64::from(shape.centers[t * 3 + 1]), f64::from(shape.centers[t * 3 + 2]));
        let c = col(x, y, z);
        let j = f64::from(shape.jitter[t]);
        let v = [(c[0] * j) as f32, (c[1] * j) as f32, (c[2] * j) as f32];
        for _ in 0..3 {
            out.extend_from_slice(&v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn dir() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/avatars/hair")
    }

    #[test]
    fn parses_every_style() {
        for hs in 0..34 {
            let s = HairShape::parse(&std::fs::read(dir().join(format!("hs-{hs:02}.bin"))).unwrap()).unwrap();
            assert!(s.triangles() > 100, "estilo {hs}");
            assert_eq!(s.positions.len(), s.triangles() * 9);
        }
        for h in 1..4 {
            HairShape::parse(&std::fs::read(dir().join(format!("hat-{h}.bin"))).unwrap()).unwrap();
        }
        assert!(HairShape::parse(b"nada").is_err());
    }

    /// Cores iguais às do `paintHair` do navegador (tolerância de arredondamento de f32).
    #[test]
    fn paint_matches_browser() {
        let pal = crate::cfg::tests_support::palettes();
        let cases: Vec<(String, BTreeMap<String, u32>)> =
            serde_json::from_str(&std::fs::read_to_string(dir().join("cases.json")).unwrap()).unwrap();
        assert!(cases.len() >= 10);
        for (file, raw) in cases {
            let cfg = pal.clean(&raw).unwrap();
            let src = if cfg.get("hat") > 0 {
                format!("hat-{}.bin", cfg.get("hat"))
            } else {
                format!("hs-{:02}.bin", cfg.get("hs"))
            };
            let shape = HairShape::parse(&std::fs::read(dir().join(src)).unwrap()).unwrap();
            let ours = paint(&shape, &pal, &cfg);
            let b = std::fs::read(dir().join(&file)).unwrap();
            let theirs: Vec<f32> = b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect();
            assert_eq!(ours.len(), theirs.len(), "{file}");
            let max = ours.iter().zip(&theirs).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            eprintln!("{file}: diferença máxima {max:e}");
            assert!(max < 1e-4, "{file}: diferença máxima {max}");
        }
    }
}
