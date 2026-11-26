//! Desenho 2D dos mascotes, portado de `assets/pet-art.js`: cada criatura é montada a partir dos genes que o servidor
//! sorteou (corpo, cabeça, orelhas, olhos, cauda, pés, padrões e características especiais), em quadros de 200×180
//! unidades, com os estados de humor (parado, feliz, dormindo, comendo, brincando, triste) e a transição suave do animador.

use crate::canvas::{Canvas, Style, css};
use serde_json::Value;
use std::f32::consts::PI;

/// Cada paleta: base, sombra, destaque, barriga (gene `pl`).
const PALETTES: [[&str; 4]; 16] = [
    ["#ffb38a", "#f08a5d", "#ff6b6b", "#ffe3cf"],
    ["#8de0c0", "#4fbf9f", "#2f9e8f", "#e0fff3"],
    ["#8cc8ff", "#5a9be0", "#3b6fd0", "#e3f3ff"],
    ["#c3a6ff", "#9a78ea", "#7a4fd6", "#f1e8ff"],
    ["#e8f27a", "#c3cf3c", "#8aa11f", "#fbffd6"],
    ["#ff9ec7", "#ee6aa5", "#d63f86", "#ffe3f0"],
    ["#ff8f7a", "#e8604f", "#c93d3d", "#ffe1da"],
    ["#ffd166", "#e8a93a", "#c27d1a", "#fff2cc"],
    ["#5fd3d9", "#2fa9b5", "#1e7f95", "#d6fbfd"],
    ["#9ccf6f", "#6ea84a", "#477f2f", "#e8f7d2"],
    ["#b57bd6", "#8a54ad", "#623382", "#ecd9f7"],
    ["#8b93a8", "#5f677d", "#3d4358", "#dfe3ee"],
    ["#f6e3c3", "#d9bf94", "#b08a55", "#fffaf0"],
    ["#ffa24d", "#e87b1e", "#c45a0a", "#ffe6cc"],
    ["#7fe3b0", "#47b6e0", "#8a6ae0", "#e3fff2"],
    ["#e86ad3", "#bd43a9", "#8f2a82", "#ffe0fa"],
];
/// Corpos: rx, ry, expoente, afinamento no topo.
const BODIES: [[f32; 4]; 8] = [
    [58.0, 50.0, 2.0, 0.0],
    [50.0, 56.0, 2.0, 0.28],
    [48.0, 58.0, 2.1, 0.14],
    [62.0, 44.0, 2.4, 0.0],
    [54.0, 54.0, 2.0, 0.0],
    [44.0, 62.0, 2.2, 0.08],
    [66.0, 42.0, 2.6, 0.0],
    [48.0, 58.0, 1.9, 0.38],
];
const HEADS: [[f32; 4]; 6] = [
    [40.0, 36.0, 2.0, 0.0],
    [46.0, 32.0, 2.2, 0.0],
    [34.0, 40.0, 2.0, 0.0],
    [44.0, 34.0, 2.5, 0.05],
    [42.0, 34.0, 3.0, 0.0],
    [38.0, 38.0, 1.8, 0.3],
];
const EYE_SIZE: [f32; 3] = [0.82, 1.0, 1.28];
const STAGE_SCALE: [f32; 4] = [0.8, 0.9, 0.98, 1.05];

/// Genes (inteiros pequenos sorteados pelo servidor).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Genes {
    pub pl: usize,
    pub tn: i64,
    pub bd: usize,
    pub hd: usize,
    pub es: usize,
    pub ey: i64,
    pub tl: i64,
    pub ft: i64,
    pub ea: i64,
    pub ck: i64,
    pub pt: i64,
    pub sp: i64,
}

impl Genes {
    pub fn from_value(v: &Value) -> Self {
        let i = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);
        let u = |k: &str| i(k).max(0) as usize;
        Self {
            pl: u("pl"),
            tn: i("tn"),
            bd: u("bd"),
            hd: u("hd"),
            es: u("es"),
            ey: i("ey"),
            tl: i("tl"),
            ft: i("ft"),
            ea: i("ea"),
            ck: i("ck"),
            pt: i("pt"),
            sp: i("sp"),
        }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Mistura duas cores `#rrggbb` (`mix`).
pub fn mix(a: &str, b: &str, t: f32) -> String {
    let p = |s: &str| u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    let (pa, pb) = (p(a), p(b));
    let c = |i: u32| lerp(((pa >> i) & 255) as f32, ((pb >> i) & 255) as f32, t).round() as u32;
    format!("#{:02x}{:02x}{:02x}", c(16), c(8), c(0))
}

struct Colors {
    base: String,
    shade: String,
    accent: String,
    belly: String,
    eye_ink: String,
}

fn colors_of(g: &Genes) -> Colors {
    let pal = PALETTES.get(g.pl).unwrap_or(&PALETTES[0]);
    let tone = match g.tn {
        1 => Some(("#ffffff", 0.2)),
        2 => Some(("#241a30", 0.14)),
        _ => None,
    };
    let f = |c: &str| tone.map_or(c.to_owned(), |(t, k)| mix(c, t, k));
    Colors { base: f(pal[0]), shade: f(pal[1]), accent: f(pal[2]), belly: f(pal[3]), eye_ink: "#241a30".into() }
}

// ---------- peças ----------

/// Superelipse (de elipse a quase retângulo), com afinamento no topo.
fn super_path(c: &mut Canvas, cx: f32, cy: f32, rx: f32, ry: f32, n: f32, taper: f32) {
    const N: usize = 36;
    let e = 2.0 / n;
    c.begin_path();
    for i in 0..=N {
        let a = i as f32 / N as f32 * PI * 2.0;
        let (s, co) = a.sin_cos();
        let mut x = rx * co.signum() * co.abs().powf(e);
        let y = ry * s.signum() * s.abs().powf(e);
        x *= 1.0 - taper * (1.0 - (y / ry + 1.0) / 2.0);
        if i == 0 { c.move_to(cx + x, cy + y) } else { c.line_to(cx + x, cy + y) }
    }
    c.close_path();
}

fn ellipse(c: &mut Canvas, x: f32, y: f32, rx: f32, ry: f32, rot: f32) {
    c.begin_path();
    c.ellipse(x, y, rx.max(0.1), ry.max(0.1), rot, 0.0, PI * 2.0);
}

fn star(c: &mut Canvas, x: f32, y: f32, r: f32, rot: f32) {
    c.begin_path();
    for i in 0..10 {
        let a = rot + i as f32 * PI / 5.0 - PI / 2.0;
        let rr = if i % 2 == 1 { r * 0.45 } else { r };
        if i == 0 {
            c.move_to(x + a.cos() * rr, y + a.sin() * rr)
        } else {
            c.line_to(x + a.cos() * rr, y + a.sin() * rr)
        }
    }
    c.close_path();
}

fn fill_stroke(c: &mut Canvas, fill: Style, stroke: Option<&str>, w: f32) {
    c.fill_style(fill);
    c.fill();
    if let Some(s) = stroke {
        c.stroke_color(s);
        c.line_width(w);
        c.round_join();
        c.stroke();
    }
}

fn solid(s: &str) -> Style {
    Style::Solid(css(s))
}

fn draw_tail(c: &mut Canvas, g: &Genes, k: &Colors, sway: f32) {
    if g.tl == 0 {
        return;
    }
    c.save();
    c.translate(150.0, 132.0);
    c.rotate(sway * 0.12);
    match g.tl {
        1 => {
            ellipse(c, 6.0, 0.0, 11.0, 10.0, 0.0);
            fill_stroke(c, solid(&k.base), Some(&k.shade), 2.0);
        }
        2 => {
            c.begin_path();
            c.move_to(-6.0, 6.0);
            c.bezier_to(24.0, 10.0, 34.0, -14.0, 16.0, -26.0);
            c.bezier_to(30.0, -10.0, 14.0, 2.0, -6.0, -4.0);
            c.close_path();
            fill_stroke(c, solid(&k.base), Some(&k.shade), 2.0);
        }
        3 => {
            c.begin_path();
            c.move_to(-6.0, 6.0);
            c.line_to(20.0, -6.0);
            c.line_to(17.0, 8.0);
            c.line_to(22.0, 16.0);
            c.close_path();
            fill_stroke(c, solid(&k.accent), Some(&k.shade), 2.0);
        }
        _ => {
            for i in -1..=1 {
                c.save();
                c.rotate(i as f32 * 0.5);
                ellipse(c, 14.0, -4.0, 12.0, 5.0, 0.0);
                fill_stroke(c, solid(if i == 0 { &k.accent } else { &k.base }), Some(&k.shade), 1.5);
                c.restore();
            }
        }
    }
    c.restore();
}

fn draw_feet(c: &mut Canvas, g: &Genes, k: &Colors, hop: f32) {
    for sx in [-1.0, 1.0] {
        let (x, y) = (100.0 + sx * 24.0, 163.0 - hop);
        if g.ft == 2 {
            ellipse(c, x, y, 17.0, 7.0, 0.0);
            fill_stroke(c, solid(&k.accent), Some(&k.shade), 1.8);
            c.stroke_color(&k.shade);
            c.line_width(1.2);
            c.begin_path();
            c.move_to(x - 6.0, y - 3.0);
            c.line_to(x - 6.0, y + 5.0);
            c.move_to(x + 6.0, y - 3.0);
            c.line_to(x + 6.0, y + 5.0);
            c.stroke();
        } else {
            ellipse(c, x, y, if g.ft == 1 { 14.0 } else { 12.0 }, 8.0, 0.0);
            fill_stroke(c, solid(&k.base), Some(&k.shade), 2.0);
            if g.ft == 1 {
                c.fill_color(&k.shade);
                for d in [-5.0, 0.0, 5.0] {
                    ellipse(c, x + d, y + 3.0, 1.6, 1.6, 0.0);
                    c.fill();
                }
            }
        }
    }
}

fn draw_arms(c: &mut Canvas, k: &Colors, up: f32, wave: f32, rx: f32) {
    for sx in [-1.0f32, 1.0] {
        c.save();
        c.translate(100.0 + sx * (rx - 4.0), 112.0);
        c.rotate(sx * (0.5 - up * 1.5) + if up > 0.3 { (wave * 6.0 + sx).sin() * 0.2 } else { 0.0 });
        ellipse(c, sx * 7.0, 6.0, 8.0, 14.0, 0.0);
        fill_stroke(c, solid(&k.base), Some(&k.shade), 2.0);
        c.restore();
    }
}

fn draw_body(c: &mut Canvas, g: &Genes, k: &Colors, body_k: [f32; 2]) {
    let [rx, ry, n, tp] = *BODIES.get(g.bd).unwrap_or(&BODIES[0]);
    let (cx, cy) = (100.0, 114.0);
    super_path(c, cx, cy, rx * body_k[0], ry * body_k[1], n, tp);
    let gr = Style::Linear {
        x0: 0.0,
        y0: cy - ry,
        x1: 0.0,
        y1: cy + ry,
        stops: vec![(0.0, css(&k.base)), (1.0, css(&mix(&k.base, &k.shade, 0.55)))],
    };
    fill_stroke(c, gr, Some(&k.shade), 2.4);
    c.save();
    super_path(c, cx, cy, rx * body_k[0] - 1.0, ry * body_k[1] - 1.0, n, tp);
    c.clip();
    // barriga e padrões, sempre recortados pelo corpo
    if g.pt == 1 || g.pt == 4 {
        ellipse(c, cx, cy + ry * 0.28, rx * 0.55, ry * 0.55, 0.0);
        c.fill_color(&k.belly);
        c.fill();
    }
    if g.pt == 2 {
        c.fill_color(&mix(&k.base, &k.accent, 0.55));
        for [dx, dy, r] in
            [[-26.0, -8.0, 8.0], [24.0, 4.0, 9.0], [-10.0, 28.0, 7.0], [18.0, -22.0, 6.0], [-34.0, 22.0, 5.0]]
        {
            ellipse(c, cx + dx, cy + dy, r, r * 0.9, 0.0);
            c.fill();
        }
    }
    if g.pt == 3 {
        c.stroke_color(&mix(&k.base, &k.shade, 0.7));
        c.line_width(7.0);
        for i in -1..=2 {
            let i = i as f32;
            c.begin_path();
            c.move_to(cx - rx, cy - 6.0 + i * 18.0);
            c.quad_to(cx, cy + 6.0 + i * 18.0, cx + rx, cy - 6.0 + i * 18.0);
            c.stroke();
        }
    }
    if g.pt == 5 {
        c.fill_color(&mix(&k.base, &k.shade, 0.75));
        c.fill_rect(cx - rx, cy + ry * 0.12, rx * 2.0, ry);
    }
    c.restore();
}

fn draw_ears(c: &mut Canvas, g: &Genes, k: &Colors, head_y: f32, tilt: f32) {
    let e = g.ea;
    if e == 0 {
        return;
    }
    let h = HEADS.get(g.hd).unwrap_or(&HEADS[0]);
    let (w, top) = (h[0], head_y - h[1]);
    c.save();
    match e {
        1 => {
            for sx in [-1.0, 1.0] {
                ellipse(c, 100.0 + sx * (w * 0.62), top + 6.0, 12.0, 13.0, 0.0);
                fill_stroke(c, solid(&k.base), Some(&k.shade), 2.0);
                ellipse(c, 100.0 + sx * (w * 0.62), top + 7.0, 6.0, 7.0, 0.0);
                c.fill_color(&k.belly);
                c.fill();
            }
        }
        2 => {
            for sx in [-1.0, 1.0] {
                c.begin_path();
                c.move_to(100.0 + sx * (w * 0.25), top + 6.0);
                c.line_to(100.0 + sx * (w * 0.78), top - 20.0);
                c.line_to(100.0 + sx * (w * 0.9), top + 14.0);
                c.close_path();
                fill_stroke(c, solid(&k.base), Some(&k.shade), 2.0);
                c.begin_path();
                c.move_to(100.0 + sx * (w * 0.5), top + 2.0);
                c.line_to(100.0 + sx * (w * 0.76), top - 12.0);
                c.line_to(100.0 + sx * (w * 0.8), top + 8.0);
                c.close_path();
                c.fill_color(&k.accent);
                c.fill();
            }
        }
        3 => {
            for sx in [-1.0f32, 1.0] {
                c.save();
                c.translate(100.0 + sx * (w * 0.88), head_y - 4.0);
                c.rotate(sx * (0.35 + tilt * 0.1));
                ellipse(c, 0.0, 14.0, 10.0, 22.0, 0.0);
                fill_stroke(c, solid(&k.shade), Some(&mix(&k.shade, "#000000", 0.2)), 2.0);
                c.restore();
            }
        }
        4 => {
            for sx in [-1.0, 1.0] {
                c.stroke_color(&k.shade);
                c.line_width(3.0);
                c.begin_path();
                c.move_to(100.0 + sx * 10.0, top + 4.0);
                c.quad_to(100.0 + sx * 22.0, top - 20.0, 100.0 + sx * (28.0 + tilt * 3.0), top - 32.0);
                c.stroke();
                ellipse(c, 100.0 + sx * (28.0 + tilt * 3.0), top - 34.0, 6.0, 6.0, 0.0);
                fill_stroke(c, solid(&k.accent), Some(&k.shade), 1.8);
            }
        }
        5 => {
            for sx in [-1.0, 1.0] {
                c.begin_path();
                c.move_to(100.0 + sx * (w * 0.4), top + 8.0);
                c.quad_to(100.0 + sx * (w * 0.62), top - 6.0, 100.0 + sx * (w * 0.5), top - 24.0);
                c.quad_to(100.0 + sx * (w * 0.84), top - 8.0, 100.0 + sx * (w * 0.76), top + 14.0);
                c.close_path();
                fill_stroke(c, solid(&k.accent), Some(&k.shade), 2.0);
            }
        }
        6 => {
            c.begin_path();
            c.move_to(100.0, top + 6.0);
            c.quad_to(100.0, top - 14.0, 100.0, top - 18.0);
            c.stroke_color("#3f9a49");
            c.line_width(3.0);
            c.stroke();
            for sx in [-1.0f32, 1.0] {
                c.save();
                c.translate(100.0, top - 14.0);
                c.rotate(sx * 0.7 + tilt * 0.08);
                ellipse(c, sx * 10.0, 0.0, 12.0, 6.0, 0.0);
                fill_stroke(c, solid("#6bd16f"), Some("#3f9a49"), 1.8);
                c.restore();
            }
        }
        7 => {
            for i in -2i32..=2 {
                let f = i as f32;
                c.begin_path();
                c.move_to(100.0 + f * 9.0 - 6.0, top + 8.0);
                c.line_to(100.0 + f * 12.0, top - 18.0 + f.abs() * 6.0);
                c.line_to(100.0 + f * 9.0 + 6.0, top + 8.0);
                c.close_path();
                fill_stroke(c, solid(if i % 2 != 0 { &k.base } else { &k.accent }), Some(&k.shade), 1.8);
            }
        }
        _ => {}
    }
    c.restore();
}

#[allow(clippy::too_many_arguments)]
fn eye_at(c: &mut Canvas, g: &Genes, k: &Colors, x: f32, y: f32, open: f32, joy: f32, size: f32) {
    let s = EYE_SIZE.get(g.es).copied().unwrap_or(1.0) * size;
    if open < 0.12 || joy > 0.5 {
        // olho fechado ou de alegria: um arco
        c.stroke_color(&k.eye_ink);
        c.line_width(2.6);
        c.round_cap();
        c.begin_path();
        if joy > 0.5 {
            c.arc(x, y + 2.0, 6.0 * s, PI * 1.1, PI * 1.9);
        } else {
            c.arc(x, y - 2.0, 6.0 * s, PI * 0.15, PI * 0.85);
        }
        c.stroke();
        return;
    }
    let sh = g.ey;
    let o = open.clamp(0.0, 1.0);
    match sh {
        0 | 4 => {
            ellipse(c, x, y, 7.0 * s, 7.5 * s * o, 0.0);
            c.fill_color(&k.eye_ink);
            c.fill();
            if sh == 4 {
                ellipse(c, x, y + 1.0, 4.0 * s, 4.5 * s * o, 0.0);
                c.fill_color(&mix(&k.accent, "#ffffff", 0.15));
                c.fill();
                ellipse(c, x + 2.0 * s, y - 3.0 * s, 1.8 * s, 1.8 * s, 0.0);
                c.fill_color("#ffffff");
                c.fill();
                ellipse(c, x - 2.5 * s, y + 2.5 * s, 1.1 * s, 1.1 * s, 0.0);
                c.fill();
            }
        }
        1 => {
            ellipse(c, x, y, 5.6 * s, 9.0 * s * o, 0.0);
            c.fill_color(&k.eye_ink);
            c.fill();
        }
        2 => {
            ellipse(c, x, y, 9.0 * s, 6.0 * s * o, 0.0);
            c.fill_color(&k.eye_ink);
            c.fill();
        }
        3 => {
            ellipse(c, x, y, 7.0 * s, 7.0 * s * o, 0.0);
            c.fill_color(&k.eye_ink);
            c.fill();
            c.fill_color(&mix(&k.base, &k.shade, 0.5));
            c.fill_rect(x - 9.0 * s, y - 8.0 * s * o, 18.0 * s, 8.0 * s * o);
            c.stroke_color(&k.eye_ink);
            c.line_width(2.0);
            c.begin_path();
            c.move_to(x - 8.0 * s, y - 0.5);
            c.line_to(x + 8.0 * s, y - 0.5);
            c.stroke();
        }
        5 => {
            ellipse(c, x, y, 7.5 * s, 8.0 * s * o, 0.0);
            c.fill_color(&mix(&k.accent, "#ffe9a8", 0.4));
            c.fill();
            ellipse(c, x, y, 2.2 * s, 7.0 * s * o, 0.0);
            c.fill_color(&k.eye_ink);
            c.fill();
            c.stroke_color(&k.eye_ink);
            c.line_width(1.6);
            ellipse(c, x, y, 7.5 * s, 8.0 * s * o, 0.0);
            c.stroke();
        }
        _ => {
            ellipse(c, x, y, 4.3 * s, 4.6 * s * o, 0.0);
            c.fill_color(&k.eye_ink);
            c.fill();
        }
    }
    if sh != 4 && sh != 5 {
        ellipse(c, x + 2.0 * s, y - 2.6 * s * o, 1.7 * s, 1.7 * s, 0.0);
        c.fill_color("#ffffff");
        c.fill();
    }
}

/// Parâmetros de um estado (`STATES`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    pub open: f32,
    pub joy: f32,
    pub smile: f32,
    pub mouth: f32,
    pub bounce: f32,
    pub bob: f32,
    pub arms: f32,
    pub head_drop: f32,
    pub rate: f32,
}

impl Params {
    pub fn of(state: &str) -> Self {
        let p = |open, joy, smile, mouth, bounce, bob, arms, head_drop, rate| Params {
            open,
            joy,
            smile,
            mouth,
            bounce,
            bob,
            arms,
            head_drop,
            rate,
        };
        match state {
            "happy" => p(1.0, 1.0, 1.0, 0.4, 1.0, 0.0, 1.0, 0.0, 1.0),
            "sleep" => p(0.0, 0.0, 0.2, 0.0, 0.0, 0.4, 0.0, 3.0, 0.5),
            "eat" => p(1.0, 0.0, 0.6, 1.0, 0.0, 0.6, 0.4, 0.0, 1.0),
            "play" => p(1.0, 1.0, 1.0, 0.6, 1.6, 0.0, 1.0, 0.0, 1.0),
            "sad" => p(0.7, 0.0, -0.8, 0.0, 0.0, 0.3, 0.0, 4.0, 0.6),
            _ => p(1.0, 0.0, 0.5, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0),
        }
    }

    /// Aproxima dos parâmetros do estado-alvo (`resolve`).
    pub fn toward(&mut self, t: &Params, k: f32) {
        self.open = lerp(self.open, t.open, k);
        self.joy = lerp(self.joy, t.joy, k);
        self.smile = lerp(self.smile, t.smile, k);
        self.mouth = lerp(self.mouth, t.mouth, k);
        self.bounce = lerp(self.bounce, t.bounce, k);
        self.bob = lerp(self.bob, t.bob, k);
        self.arms = lerp(self.arms, t.arms, k);
        self.head_drop = lerp(self.head_drop, t.head_drop, k);
        self.rate = lerp(self.rate, t.rate, k);
    }
}

fn draw_face(c: &mut Canvas, g: &Genes, k: &Colors, head_y: f32, p: &Params) {
    let (dx, ey) = (17.0, head_y + 3.0);
    if g.ck != 0 {
        for sx in [-1.0, 1.0] {
            if g.ck == 1 {
                ellipse(c, 100.0 + sx * 27.0, head_y + 16.0, 7.0, 4.4, 0.0);
                c.fill_color(&mix(&k.accent, &k.base, 0.35));
                c.set_alpha(0.55);
                c.fill();
                c.set_alpha(1.0);
            } else {
                star(c, 100.0 + sx * 27.0, head_y + 16.0, 5.0, 0.0);
                c.fill_color(&mix(&k.accent, "#ffffff", 0.25));
                c.fill();
            }
        }
    }
    if g.pt == 4 {
        c.fill_color(&mix(&k.shade, "#000000", 0.15));
        for sx in [-1.0, 1.0] {
            for [a, b] in [[-3.0, 0.0], [3.0, 2.0], [0.0, 5.0]] {
                ellipse(c, 100.0 + sx * 29.0 + a, head_y + 12.0 + b, 1.1, 1.1, 0.0);
                c.fill();
            }
        }
    }
    eye_at(c, g, k, 100.0 - dx, ey, p.open, p.joy, 1.0);
    eye_at(c, g, k, 100.0 + dx, ey, p.open, p.joy, 1.0);
    // boca
    let my = head_y + 20.0;
    c.stroke_color(&k.eye_ink);
    c.line_width(2.4);
    c.round_cap();
    if p.mouth > 0.12 {
        let mo = p.mouth;
        ellipse(c, 100.0, my + 2.0, 6.0 + 3.0 * mo, 2.0 + 7.0 * mo, 0.0);
        c.fill_color("#6a2a3a");
        c.fill();
        ellipse(c, 100.0, my + 5.0 + 3.0 * mo, 3.6, 2.2 * mo + 0.6, 0.0);
        c.fill_color("#ff8fa8");
        c.fill();
    } else {
        c.begin_path();
        c.move_to(92.0, my);
        c.quad_to(100.0, my + p.smile * 9.0 + 1.0, 108.0, my);
        c.stroke();
    }
}

fn draw_special_back(c: &mut Canvas, g: &Genes, k: &Colors, t: f32) {
    if g.sp == 1 || g.sp == 5 {
        let r = 84.0 + (t * 2.0).sin() * 4.0;
        let col = if g.sp == 5 { "#a98bff".to_owned() } else { k.accent.clone() };
        let inner = css(&format!("{}88", mix(&col, "#ffffff", 0.4)));
        let outer = css(&format!("{col}00"));
        c.fill_style(Style::Radial { cx: 100.0, cy: 108.0, r0: 24.0, r1: r, stops: vec![(0.0, inner), (1.0, outer)] });
        c.begin_path();
        c.arc(100.0, 108.0, r, 0.0, PI * 2.0);
        c.fill();
    }
}

fn draw_special_front(c: &mut Canvas, g: &Genes, t: f32, head_y: f32) {
    let [rx, ry, n, tp] = *BODIES.get(g.bd).unwrap_or(&BODIES[0]);
    match g.sp {
        2 => {
            star(c, 100.0, head_y - 14.0, 6.0, 0.0);
            fill_stroke(c, solid("#ffe27a"), Some("#d9a21a"), 1.4);
        }
        3 => {
            c.save();
            super_path(c, 100.0, 114.0, rx, ry, n, tp);
            c.clip();
            let stops = ["#ff6b6b", "#ffd166", "#6bdc8a", "#5ab8ff", "#b07cff"]
                .iter()
                .enumerate()
                .map(|(i, s)| (i as f32 / 4.0, css(s)))
                .collect();
            c.set_alpha(0.33);
            c.fill_style(Style::Linear { x0: 40.0, y0: 60.0, x1: 160.0, y1: 170.0, stops });
            c.fill_rect(0.0, 0.0, 200.0, 180.0);
            c.restore();
        }
        4 => {
            let pts = [[78.0, 100.0], [96.0, 88.0], [118.0, 104.0], [106.0, 126.0], [128.0, 138.0], [84.0, 138.0]];
            c.stroke_color("rgba(255,255,255,.7)");
            c.line_width(1.2);
            c.begin_path();
            for (i, p) in pts.iter().enumerate() {
                if i == 0 { c.move_to(p[0], p[1]) } else { c.line_to(p[0], p[1]) }
            }
            c.stroke();
            c.fill_color("#ffffff");
            for (i, p) in pts.iter().enumerate() {
                let r = 2.0 + (i % 2) as f32;
                ellipse(c, p[0], p[1], r, r, 0.0);
                c.fill();
            }
        }
        5 => {
            c.save();
            super_path(c, 100.0, 114.0, rx, ry, n, tp);
            c.clip();
            c.fill_style(Style::Linear {
                x0: 0.0,
                y0: 60.0,
                x1: 0.0,
                y1: 170.0,
                stops: vec![(0.0, css("rgba(30,20,80,.55)")), (1.0, css("rgba(60,20,110,.75)"))],
            });
            c.fill_rect(0.0, 0.0, 200.0, 180.0);
            c.fill_color("#ffffff");
            for [x, y, k] in [
                [74.0, 100.0, 0.0],
                [118.0, 94.0, 1.0],
                [96.0, 126.0, 2.0],
                [128.0, 130.0, 3.0],
                [82.0, 148.0, 4.0],
                [112.0, 148.0, 5.0],
            ] {
                let tw = 0.5 + 0.5 * (t * 2.2 + k * 1.7).sin();
                ellipse(c, x, y, 1.2 + tw * 1.4, 1.2 + tw * 1.4, 0.0);
                c.fill();
            }
            c.restore();
        }
        _ => {}
    }
}

fn draw_cosmetic(c: &mut Canvas, g: &Genes, cos: i64, head_y: f32) {
    let h = HEADS.get(g.hd).unwrap_or(&HEADS[0]);
    match cos {
        1 => {
            c.save();
            c.translate(100.0 + h[0] * 0.55, head_y - h[1] * 0.78);
            for sx in [-1.0, 1.0] {
                c.begin_path();
                c.move_to(0.0, 0.0);
                c.line_to(sx * 15.0, -9.0);
                c.line_to(sx * 15.0, 9.0);
                c.close_path();
                fill_stroke(c, solid("#ff5d8f"), Some("#c93a6c"), 1.6);
            }
            ellipse(c, 0.0, 0.0, 4.5, 4.5, 0.0);
            fill_stroke(c, solid("#ff8fb3"), Some("#c93a6c"), 1.4);
            c.restore();
        }
        2 => {
            let y0 = head_y + h[1] * 0.78;
            c.begin_path();
            c.move_to(60.0, y0);
            c.quad_to(100.0, y0 + 14.0, 140.0, y0);
            c.line_to(138.0, y0 + 11.0);
            c.quad_to(100.0, y0 + 26.0, 62.0, y0 + 11.0);
            c.close_path();
            fill_stroke(c, solid("#e8604f"), Some("#a63a2f"), 1.6);
            c.begin_path();
            c.move_to(122.0, y0 + 14.0);
            c.line_to(134.0, y0 + 36.0);
            c.line_to(120.0, y0 + 33.0);
            c.close_path();
            fill_stroke(c, solid("#e8604f"), Some("#a63a2f"), 1.4);
        }
        3 => {
            let y = head_y - h[1] + 4.0;
            c.begin_path();
            c.move_to(82.0, y);
            c.line_to(86.0, y - 14.0);
            c.line_to(93.0, y - 5.0);
            c.line_to(100.0, y - 17.0);
            c.line_to(107.0, y - 5.0);
            c.line_to(114.0, y - 14.0);
            c.line_to(118.0, y);
            c.close_path();
            fill_stroke(c, solid("#ffd84d"), Some("#c79a12"), 1.6);
            star(c, 100.0, y - 7.0, 3.4, 0.0);
            c.fill_color("#ff7aa8");
            c.fill();
        }
        4 => {
            c.stroke_color("#3a2f4a");
            c.line_width(2.4);
            for sx in [-1.0, 1.0] {
                ellipse(c, 100.0 + sx * 17.0, head_y + 3.0, 12.0, 12.0, 0.0);
                c.stroke();
            }
            c.begin_path();
            c.move_to(95.0, head_y + 3.0);
            c.line_to(105.0, head_y + 3.0);
            c.stroke();
        }
        _ => {}
    }
}

fn draw_stage(c: &mut Canvas, k: &Colors, stage: usize, t: f32) {
    if stage < 1 {
        return;
    }
    c.fill_color(&mix(&k.accent, "#ffffff", 0.45));
    let n = if stage >= 3 {
        4
    } else if stage >= 2 {
        3
    } else {
        2
    };
    for i in 0..n {
        let a = t * 0.8 + i as f32 * PI * 2.0 / n as f32;
        star(c, 100.0 + a.cos() * 82.0, 96.0 + a.sin() * 40.0, 3.2 + if stage >= 3 { 1.0 } else { 0.0 }, a);
        c.fill();
    }
}

/// Um "z" de sono desenhado com traços (o original usa texto).
fn zz(c: &mut Canvas, x: f32, y: f32) {
    c.line_width(2.2);
    c.round_cap();
    c.begin_path();
    c.move_to(x, y - 9.0);
    c.line_to(x + 7.0, y - 9.0);
    c.line_to(x, y);
    c.line_to(x + 7.0, y);
    c.stroke();
}

/// Opções de um quadro (`draw(ctx, W, H, g, o)`).
pub struct Frame<'a> {
    pub t: f32,
    pub p: Params,
    pub state: &'a str,
    pub stage: usize,
    pub cos: i64,
    pub blink: f32,
}

/// Desenha um quadro no canvas (que tem qualquer tamanho; o desenho é em 200×180 unidades).
pub fn draw(c: &mut Canvas, g: &Genes, o: &Frame<'_>) {
    let (w, h) = (c.pm.width() as f32, c.pm.height() as f32);
    let (t, stage, cos) = (o.t, o.stage.min(3), o.cos);
    let p = o.p;
    let k = colors_of(g);
    let ks = STAGE_SCALE[stage];
    let rate = p.rate;
    let bounce = if p.bounce != 0.0 { -(t * 5.2 * rate).sin().abs() * 9.0 * p.bounce } else { 0.0 };
    let bob = (t * 2.1 * rate).sin() * 1.8 * p.bob;
    let dy = bounce + bob;
    let face = Params {
        open: p.open * o.blink,
        smile: p.smile + (t * 1.3).sin() * 0.05,
        mouth: p.mouth * if o.state == "eat" { 0.5 + 0.5 * (t * 9.0).sin().abs() } else { 1.0 },
        ..p
    };
    c.save();
    c.clear();
    c.scale(w / 200.0, h / 180.0);
    draw_special_back(c, g, &k, t);
    // sombra no chão
    ellipse(c, 100.0, 170.0, 52.0 * ks - bounce * 0.4, 7.0, 0.0);
    c.fill_color("rgba(0,0,0,.16)");
    c.fill();
    // tudo o que é do bicho sobe e desce junto, ancorado no chão
    c.translate(100.0, 168.0);
    c.scale(ks, ks);
    c.translate(-100.0, -168.0 + dy);
    let breath = 1.0 + (t * 2.1 * rate).sin() * 0.014 * if p.bob != 0.0 { 1.0 } else { 0.4 };
    let head_y = 66.0 + p.head_drop;
    draw_tail(c, g, &k, (t * 3.1).sin());
    draw_feet(c, g, &k, (bounce * -0.15).max(0.0));
    draw_arms(c, &k, p.arms, t, BODIES.get(g.bd).unwrap_or(&BODIES[0])[0]);
    draw_body(c, g, &k, [1.0, breath]);
    draw_special_front(c, g, t, head_y);
    draw_ears(c, g, &k, head_y, (t * 2.4).sin() * if p.bounce != 0.0 { 2.0 } else { 0.6 });
    let [hx, hy, hn, ht] = *HEADS.get(g.hd).unwrap_or(&HEADS[0]);
    let hk = if stage == 0 { 1.1 } else { 1.0 };
    super_path(c, 100.0, head_y, hx * hk, hy * hk, hn, ht);
    let gr = Style::Linear {
        x0: 0.0,
        y0: head_y - hy,
        x1: 0.0,
        y1: head_y + hy,
        stops: vec![(0.0, css(&mix(&k.base, "#ffffff", 0.08))), (1.0, css(&k.base))],
    };
    fill_stroke(c, gr, Some(&k.shade), 2.4);
    draw_face(c, g, &k, head_y, &face);
    draw_cosmetic(c, g, cos, head_y);
    draw_stage(c, &k, stage, t);
    // detalhes dos estados
    match o.state {
        "sleep" => {
            c.stroke_color(&mix(&k.accent, "#ffffff", 0.3));
            for i in 0..3 {
                let a = (t * 0.7 + i as f32 / 3.0) % 1.0;
                c.set_alpha(1.0 - a);
                zz(c, 128.0 + a * 14.0 + i as f32 * 4.0, 52.0 - a * 30.0);
            }
            c.set_alpha(1.0);
        }
        "happy" | "play" => {
            for i in 0..2 {
                let a = (t * 0.9 + i as f32 / 2.0) % 1.0;
                c.set_alpha(1.0 - a);
                star(c, 60.0 + i as f32 * 80.0 + a * 6.0, 52.0 - a * 22.0, 5.0, a * 2.0);
                c.fill_color("#ffd166");
                c.fill();
            }
            c.set_alpha(1.0);
        }
        _ => {}
    }
    if o.state == "eat" {
        ellipse(c, 100.0 + (t * 3.0).sin() * 3.0, head_y + 36.0, 6.0, 6.0, 0.0);
        fill_stroke(c, solid("#ff7a59"), Some("#c24a2f"), 1.6);
    }
    if o.state == "play" {
        let by = 138.0 - (t * 5.2).sin().abs() * 34.0;
        ellipse(c, 160.0, by, 9.0, 9.0, 0.0);
        fill_stroke(c, solid(&k.accent), Some(&mix(&k.accent, "#000000", 0.3)), 1.8);
    }
    if o.state == "sad" {
        ellipse(c, 126.0, head_y + 14.0 + (t * 18.0) % 14.0, 2.4, 3.6, 0.0);
        c.fill_color("#8fd0ff");
        c.fill();
    }
    c.restore();
}

/// Animador (`createAnimator`): estado de fundo, animação curta de uma ação e piscadas.
#[derive(Debug, Clone)]
pub struct Animator {
    pub genes: Genes,
    pub stage: usize,
    pub cos: i64,
    base: String,
    state: String,
    cur: Params,
    t: f32,
    flash_left: f32,
    blink_at: f32,
    since_draw: f32,
}

impl Animator {
    pub fn new(genes: Genes, stage: usize, cos: i64, state: &str) -> Self {
        Self {
            genes,
            stage,
            cos,
            base: state.to_owned(),
            state: state.to_owned(),
            cur: Params::of(state),
            t: 0.0,
            flash_left: 0.0,
            blink_at: 2.0,
            since_draw: f32::INFINITY,
        }
    }
    pub fn set_state(&mut self, s: &str) {
        self.base = s.to_owned();
        if self.flash_left <= 0.0 {
            self.state = s.to_owned();
        }
    }
    /// Animação curta de uma ação; depois volta ao estado de fundo.
    pub fn flash(&mut self, s: &str, secs: f32) {
        self.state = s.to_owned();
        self.flash_left = secs;
    }
    /// Avança o tempo; desenha (e devolve `true`) quando já é hora do próximo quadro: 14 por segundo com a cena calma,
    /// 28 quando ele se mexe.
    pub fn tick(&mut self, dt: f32, c: &mut Canvas) -> bool {
        let dt = dt.min(0.25);
        self.t += dt;
        self.since_draw += dt;
        if self.flash_left > 0.0 {
            self.flash_left -= dt;
            if self.flash_left <= 0.0 {
                self.state = self.base.clone();
            }
        }
        let calm = matches!(self.state.as_str(), "idle" | "sleep" | "sad");
        let gap = if calm { 1.0 / 14.0 } else { 1.0 / 28.0 };
        if self.since_draw < gap {
            return false;
        }
        let k = 1.0 - (-self.since_draw.min(0.25) * 7.0).exp();
        self.since_draw = 0.0;
        self.cur.toward(&Params::of(&self.state), k);
        let mut blink = 1.0;
        if self.t > self.blink_at {
            let x = (self.t - self.blink_at) / 0.16;
            blink = if x < 1.0 { (1.0 - 2.0 * x).abs() } else { 1.0 };
            if x >= 1.0 {
                self.blink_at = self.t + 2.4 + (self.t * 7.3) % 2.6;
            }
        }
        let state = self.state.clone();
        draw(c, &self.genes, &Frame { t: self.t, p: self.cur, state: &state, stage: self.stage, cos: self.cos, blink });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_and_every_gene_draws() {
        assert_eq!(mix("#000000", "#ffffff", 0.5), "#808080");
        assert_eq!(mix("#ff0000", "#0000ff", 0.0), "#ff0000");
        let mut c = Canvas::new(100, 90).unwrap();
        // todas as formas de cada gene, em todos os estados: nada pode faltar nem entrar em pânico
        for i in 0..8i64 {
            let g = Genes {
                pl: i as usize * 2,
                tn: i % 3,
                bd: i as usize,
                hd: i as usize % 6,
                es: i as usize % 3,
                ey: i % 7,
                tl: i % 5,
                ft: i % 3,
                ea: i,
                ck: i % 3,
                pt: i % 6,
                sp: i % 6,
            };
            for s in ["idle", "happy", "sleep", "eat", "play", "sad"] {
                draw(
                    &mut c,
                    &g,
                    &Frame {
                        t: i as f32 * 0.7,
                        p: Params::of(s),
                        state: s,
                        stage: i as usize % 4,
                        cos: i % 5,
                        blink: 1.0,
                    },
                );
            }
        }
        // o bicho ocupa o meio do quadro
        let px = c.rgba();
        let mid = (45 * 100 + 50) * 4;
        assert_eq!(px[mid + 3], 255);
    }
}
