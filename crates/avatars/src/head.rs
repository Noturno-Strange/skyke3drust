//! Textura da cabeça (pele, couro cabeludo pintado e rosto), portada de `headMat`, `hpx`, `hw` e `HPAINT` em
//! `assets/app-avatar.js`. Tudo é calculado por pixel com funções de distância, sem desenho em canvas, então a saída
//! pode ser comparada pixel a pixel com a do navegador (teste `matches_browser_reference`).
//!
//! A textura (512×256) cobre a esfera da cabeça (`SphereGeometry(1, 28, 20)` escalada para `HR`), com o UV padrão do
//! three.js: x = longitude, y = latitude a partir do topo.

use crate::cfg::{Cfg, Palette, Palettes, hex_to_linear};
use std::f64::consts::PI;
use std::sync::OnceLock;

pub const FW: usize = 512;
pub const FH: usize = 256;
const HR: [f64; 3] = [0.36, 0.38, 0.34];
const HEDGE: [f64; 6] = [1.0, 1.05, 1.3, 1.4, 1.75, 2.0];

fn fract(x: f64) -> f64 {
    x - x.floor()
}
/// `hh`: ruído de hash 1D.
fn hh(n: f64) -> f64 {
    fract((n * 127.1 + 311.7).sin() * 43758.5453)
}
fn hl(i: f64, j: f64, k: f64) -> f64 {
    hh(i * 12.9898 + j * 78.233 + k * 37.719)
}
/// `vn`: ruído de valor 3D com interpolação suave.
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
    let a = ph - PI / 2.0;
    a.sin().atan2(a.cos()).abs()
}
fn hedge(e: &[f64; 6], a: f64) -> f64 {
    let k = [0.0, 0.9, 1.35, 1.57, 2.1, PI];
    for i in 1..6 {
        if a <= k[i] {
            let t = (a - k[i - 1]) / (k[i] - k[i - 1]);
            return e[i - 1] + (e[i] - e[i - 1]) * t * t * (3.0 - 2.0 * t);
        }
    }
    e[5]
}
fn sstep(a: f64, b: f64, t: f64) -> f64 {
    let u = ((t - a) / (b - a)).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Dados por pixel independentes do visual (`hpx`), em `f32` como os `Float32Array` do original.
struct Px {
    x: Vec<f32>,
    y: Vec<f32>,
    z: Vec<f32>,
    th: Vec<f32>,
    an: Vec<f32>,
    nz: Vec<f32>,
    nh: Vec<f32>,
}

fn px() -> &'static Px {
    static PX: OnceLock<Px> = OnceLock::new();
    PX.get_or_init(|| {
        let n = FW * FH;
        let mut p = Px {
            x: vec![0.0; n],
            y: vec![0.0; n],
            z: vec![0.0; n],
            th: vec![0.0; n],
            an: vec![0.0; n],
            nz: vec![0.0; n],
            nh: vec![0.0; n],
        };
        for py in 0..FH {
            let th = PI * (py as f64 + 0.5) / FH as f64;
            let (sn, cs) = (th.sin(), th.cos());
            for pxx in 0..FW {
                let ph = 2.0 * PI * (pxx as f64 + 0.5) / FW as f64;
                let i = py * FW + pxx;
                let (cp, sp) = (ph.cos(), ph.sin());
                p.x[i] = (-cp * sn * HR[0]) as f32;
                p.y[i] = (cs * HR[1]) as f32;
                p.z[i] = (sp * sn * HR[2]) as f32;
                p.th[i] = th as f32;
                p.an[i] = hang(ph) as f32;
                p.nz[i] = (0.1 * (vn(-cp * sn * 3.4 + 3.0, cs * 3.4, sp * sn * 3.4) - 0.5)) as f32;
                p.nh[i] = hh(i as f64 * 0.7) as f32;
            }
        }
        p
    })
}

/// Linha do cabelo por estilo (`HPAINT`): borda `E` e, em alguns estilos, a área de cabelo raspado (`sv`).
struct Paint {
    e: [f64; 6],
    sv: Option<fn(f64, f64, f64) -> bool>,
}

fn hpaint(hs: u32) -> Paint {
    let p = |e: [f64; 6]| Paint { e, sv: None };
    match hs {
        0 => p([0.9, 0.95, 1.1, 1.2, 1.7, 2.1]),
        5 => p([0.88, 0.94, 1.2, 1.3, 1.8, 2.1]),
        11 | 16 => p([1.0, 1.05, 1.3, 1.6, 2.1, 2.3]),
        24 => Paint { e: [1.1, 1.15, 1.5, 1.55, 1.9, 2.1], sv: Some(|_, y, _| y < 0.14) },
        25 => Paint { e: [1.1, 1.15, 1.45, 1.55, 1.9, 2.1], sv: Some(|_, _, _| true) },
        27 => p([1.0, 1.05, 1.3, 1.5, 1.9, 2.1]),
        3 | 17 => Paint { e: [1.15, 1.2, 1.5, 1.55, 1.9, 2.1], sv: Some(|x, _, _| x.abs() > 0.13) },
        7 | 31 => Paint { e: [1.2, 1.25, 1.5, 1.55, 1.9, 2.1], sv: Some(|_, y, _| y < 0.18) },
        _ => p(HEDGE),
    }
}

/// Peso do couro cabeludo pintado e flag de raspado (`hw`).
fn hair_weight(paint: &Paint) -> (Vec<f32>, Vec<u8>) {
    let h = px();
    let n = FW * FH;
    let (mut w, mut sv) = (vec![0f32; n], vec![0u8; n]);
    for i in 0..n {
        let th = f64::from(h.th[i]);
        if th >= 2.5 {
            continue;
        }
        let e = hedge(&paint.e, f64::from(h.an[i])) + f64::from(h.nz[i]);
        if th < e {
            let q = 1.0 - sstep(e - 0.14, e, th);
            if q > 0.0 {
                w[i] = q as f32;
                sv[i] = u8::from(paint.sv.is_some_and(|f| f(f64::from(h.x[i]), f64::from(h.y[i]), f64::from(h.z[i]))));
            }
        }
    }
    (w, sv)
}

fn enc(v: f64) -> f64 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

/// `Uint8ClampedArray`: limita a 0–255 e arredonda para o par mais próximo nos empates.
fn clamp_u8(v: f64) -> u8 {
    if v.is_nan() {
        return 0;
    }
    let v = v.clamp(0.0, 255.0);
    let f = v.floor();
    let d = v - f;
    let r = if d > 0.5 || (d == 0.5 && (f as u32) % 2 == 1) { f + 1.0 } else { f };
    r as u8
}

fn cov(q: f64) -> f64 {
    if q > 0.0045 {
        0.0
    } else if q < -0.0045 {
        1.0
    } else {
        0.5 - q / 0.009
    }
}
fn el(x: f64, y: f64, cx: f64, cy: f64, rx: f64, ry: f64) -> f64 {
    (((x - cx) / rx).hypot((y - cy) / ry) - 1.0) * rx.min(ry)
}
fn sg(x: f64, y: f64, ax: f64, ay: f64, bx: f64, by: f64, t: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let u = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    (x - ax - dx * u).hypot(y - ay - dy * u) - t / 2.0
}
fn arc(x: f64, y: f64, cx: f64, cy: f64, r: f64, t: f64, up: bool) -> f64 {
    let dx = x - cx;
    let dy = (y - cy) * if up { 1.0 } else { -1.0 };
    if dy >= 0.0 { (dx.hypot(dy) - r).abs() - t / 2.0 } else { (dx.abs() - r).hypot(dy) - t / 2.0 }
}

/// Abertura da boca: 0 normal, 1 falando, 2 gritando (`mv`).
pub fn head_texture(pal: &Palettes, c: &Cfg, mv: usize) -> Vec<u8> {
    let hat = c.get("hat");
    let fc = c.get("fc");
    let eb = c.flag("eb");
    let paint = if hat > 0 { Paint { e: [1.0, 1.05, 1.35, 1.5, 1.9, 2.1], sv: None } } else { hpaint(c.get("hs")) };
    let (w, svf) = hair_weight(&paint);
    let h = px();

    let sk = hex_to_linear(pal.color(Palette::Skin, c.get("skin"), c.flag("tb")));
    let c1 = hex_to_linear(pal.color(Palette::Hair, c.get("hair"), c.flag("rb")));
    let dk = c1.map(|v| v * 0.8);
    let shv = [0, 1, 2].map(|i| sk[i] + (c1[i] - sk[i]) * 0.5);
    let ec0 = hex_to_linear(pal.color(Palette::Eye, c.get("eye"), false));
    let ec = if eb { hex_to_linear(pal.color(Palette::Eye, c.get("eye"), true)) } else { ec0 };
    let er = ec0.map(|v| v * 0.45);
    let red = hex_to_linear("#d62839");
    let ink = hex_to_linear("#0a0705");
    let wh = [1.0; 3];
    let tg2 = hex_to_linear("#ff4f86");
    let bl = hex_to_linear("#ff5f8f");
    let tear = hex_to_linear("#6ec6ff");
    let lens = hex_to_linear("#bfe2ff");
    let ms = [[1.0, 1.0], [1.0, 1.6], [1.3, 2.6]][mv.min(2)];

    let mut out = vec![0u8; FW * FH * 4];
    for i in 0..FW * FH {
        let (x, y, z) = (f64::from(h.x[i]), f64::from(h.y[i]), f64::from(h.z[i]));
        let mut col = sk;
        let put = |cc: [f64; 3], q: f64, al: f64, col: &mut [f64; 3]| {
            let k = cov(q) * al;
            if k > 0.0 {
                for j in 0..3 {
                    col[j] += (cc[j] - col[j]) * k;
                }
            }
        };
        let wi = f64::from(w[i]);
        if wi > 0.0 {
            let base = if svf[i] == 1 { shv } else { dk };
            let n = 0.95 + 0.1 * f64::from(h.nh[i]);
            for j in 0..3 {
                col[j] += (base[j] * n - col[j]) * wi;
            }
        }
        if z > 0.0 && y < 0.22 && y > -0.32 {
            // bochechas
            if fc == 7 {
                for sd in [-1.0, 1.0] {
                    put(bl, el(x, y, sd * 0.215, -0.125, 0.068, 0.042), 1.0, &mut col);
                }
            }
            for sd in [-1.0f64, 1.0] {
                let ex = sd * 0.13;
                if fc == 2 || (fc == 3 && sd == 1.0) {
                    put(ink, arc(x, y, ex, 0.04, 0.065, 0.028, true), 1.0, &mut col);
                } else if fc == 11 {
                    put(ink, arc(x, y, ex, 0.075, 0.065, 0.028, false), 1.0, &mut col);
                } else {
                    let bb = if fc == 4 { 1.15 } else { 1.0 };
                    let cl: Option<[f64; 4]> = match fc {
                        9 => Some([ex + sd * 0.11, 0.14, ex - sd * 0.08, 0.07]),
                        10 => Some([ex + sd * 0.11, 0.07, ex - sd * 0.08, 0.16]),
                        _ => None,
                    };
                    let cut = |q: f64| -> f64 {
                        let Some(cl) = cl else { return q };
                        let (dx, dy) = (cl[2] - cl[0], cl[3] - cl[1]);
                        let l = dx.hypot(dy);
                        q.max(sign(dx) * ((x - cl[0]) * -dy + (y - cl[1]) * dx) / l)
                    };
                    let q = cut(el(x, y, ex, 0.06, 0.095 * bb, 0.11 * bb));
                    put(ink, q - 0.011, 1.0, &mut col);
                    put(wh, q, 1.0, &mut col);
                    let ix = if fc == 4 { 0.04 } else { 0.055 };
                    let iy = if fc == 4 { 0.05 } else { 0.065 };
                    if eb {
                        put(er, cut(el(x, y, ex, 0.06, ix, iy)), 1.0, &mut col);
                    }
                    let (ix2, iy2) = if eb { (ix * 0.8, iy * 0.8) } else { (ix, iy) };
                    put(ec, cut(el(x, y, ex, 0.06, ix2, iy2)), 1.0, &mut col);
                    let rr = if eb { 0.028 } else { 0.02 };
                    put(wh, cut(el(x, y, ex + 0.02, 0.09, rr, rr)), 1.0, &mut col);
                    if eb {
                        put(wh, cut(el(x, y, ex - 0.022, 0.035, 0.011, 0.011)), 1.0, &mut col);
                    }
                    if let Some(cl) = cl {
                        put(ink, sg(x, y, cl[0], cl[1], cl[2], cl[3], 0.03), 1.0, &mut col);
                        if fc == 9 {
                            put(ink, sg(x, y, ex + sd * 0.1, 0.2, ex - sd * 0.08, 0.15, 0.024), 1.0, &mut col);
                        } else {
                            put(ink, sg(x, y, ex + sd * 0.1, 0.15, ex - sd * 0.09, 0.2, 0.024), 1.0, &mut col);
                        }
                    }
                    if fc == 10 && sd == 1.0 {
                        put(tear, el(x, y, 0.2, -0.07, 0.022, 0.04), 1.0, &mut col);
                    }
                }
                // óculos
                if fc == 12 {
                    let lq = el(x, y, ex, 0.06, 0.122, 0.128);
                    let gap = lq.max(-(el(x, y, ex, 0.06, 0.095, 0.11) - 0.011));
                    put(lens, gap, 0.3, &mut col);
                    put(wh, sg(x, y, ex - 0.105, 0.07, ex - 0.075, 0.135, 0.016).max(gap), 0.85, &mut col);
                    put(ink, lq.abs() - 0.012, 1.0, &mut col);
                    put(ink, sg(x, y, ex + sd * 0.122, 0.075, ex + sd * 0.23, 0.095, 0.022), 1.0, &mut col);
                }
            }
            if fc == 12 {
                put(ink, arc(x, y, 0.0, 0.06, 0.03, 0.02, true), 1.0, &mut col);
            }
            let (mx, my) = (x / ms[0], (y + 0.1) / ms[1]);
            match fc {
                1 => put(red, arc(mx, my, 0.0, 0.05, 0.1, 0.024, false), 1.0, &mut col),
                12 => put(red, arc(mx, my, 0.0, 0.0, 0.085, 0.024, false), 1.0, &mut col),
                8 => put(red, sg(mx, my, -0.06, 0.0, 0.06, 0.0, 0.026), 1.0, &mut col),
                9 => put(red, sg(mx, my, -0.07, 0.01, 0.07, 0.01, 0.026), 1.0, &mut col),
                10 => put(red, arc(mx, my, 0.0, -0.04, 0.07, 0.026, true), 1.0, &mut col),
                11 => put(red, el(mx, my, 0.0, -0.02, 0.05, 0.04).max(my), 1.0, &mut col),
                4 => put(red, el(mx, my, 0.0, -0.05, 0.055, 0.055), 1.0, &mut col),
                _ => {
                    put(red, (mx.hypot(my) - 0.12).max(my), 1.0, &mut col);
                    if fc == 6 {
                        put(wh, el(mx, my, 0.0, -0.005, 0.11, 0.0495).max(my + 0.005), 1.0, &mut col);
                    }
                    if fc == 5 {
                        put(tg2, el(mx, my, 0.03, -0.09, 0.045, 0.06).max(mx.hypot(my) - 0.115), 1.0, &mut col);
                    }
                }
            }
        }
        let o = i * 4;
        out[o] = clamp_u8(enc(col[0]) * 255.0);
        out[o + 1] = clamp_u8(enc(col[1]) * 255.0);
        out[o + 2] = clamp_u8(enc(col[2]) * 255.0);
        out[o + 3] = 255;
    }
    out
}

/// `Math.sign`
fn sign(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn js_helpers() {
        assert_eq!(clamp_u8(2.5), 2);
        assert_eq!(clamp_u8(3.5), 4);
        assert_eq!(clamp_u8(-3.0), 0);
        assert_eq!(clamp_u8(300.0), 255);
        assert!((hedge(&HEDGE, 0.0) - 1.0).abs() < 1e-12);
        assert!((hedge(&HEDGE, PI) - 2.0).abs() < 1e-12);
    }

    /// Compara com texturas geradas pelo próprio `headMat` do navegador (`tools/asset-exporter`, pasta
    /// `assets/avatars/ref-head/`). Pequenas diferenças de último bit em `Math.sin` entre motores mudam alguns poucos
    /// pixels do ruído; exige quase todos iguais e nenhum muito diferente.
    #[test]
    fn matches_browser_reference() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/avatars/ref-head");
        let Ok(list) = std::fs::read_to_string(dir.join("cases.json")) else {
            panic!("faltam as referências em {}", dir.display());
        };
        let cases: Vec<(String, BTreeMap<String, u32>, usize)> = serde_json::from_str(&list).unwrap();
        let pal = crate::cfg::tests_support::palettes();
        assert!(cases.len() >= 10);
        for (file, raw, mv) in cases {
            let cfg = pal.clean(&raw).unwrap();
            let ours = head_texture(&pal, &cfg, mv);
            let img = image::open(dir.join(&file)).unwrap().to_rgba8();
            assert_eq!(img.dimensions(), (FW as u32, FH as u32));
            let (mut diff, mut maxd) = (0usize, 0u8);
            for (a, b) in ours.chunks(4).zip(img.as_raw().chunks(4)) {
                let d = (0..3).map(|k| a[k].abs_diff(b[k])).max().unwrap_or(0);
                if d > 1 {
                    diff += 1;
                }
                maxd = maxd.max(d);
            }
            let pct = diff as f64 * 100.0 / (FW * FH) as f64;
            eprintln!("{file}: {pct:.4}% de pixels com diferença > 1, máxima {maxd}");
            assert!(pct < 0.5 && maxd < 64, "{file}: {pct:.3}% de pixels diferentes, diferença máxima {maxd}");
        }
    }
}
