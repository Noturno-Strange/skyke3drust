//! Um pedaço do Canvas 2D do navegador sobre o `tiny-skia`, o suficiente para portar desenhos feitos com
//! `CanvasRenderingContext2D` quase linha a linha (o mascote, em `pet_art`). O caminho fica no espaço do usuário e
//! recebe a transformação atual ao preencher, contornar ou recortar; `save`/`restore` guardam transformação, alfa
//! global, recorte e estilos.

use std::f32::consts::PI;
use tiny_skia::{
    Color, FillRule, GradientStop, LineCap, LineJoin, LinearGradient, Mask, Paint, Path, PathBuilder, Pixmap, Point,
    RadialGradient, Rect, Shader, SpreadMode, Stroke, Transform,
};

/// Cor ou degradê de preenchimento/contorno.
#[derive(Debug, Clone)]
pub enum Style {
    Solid(Color),
    Linear {
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        stops: Vec<(f32, Color)>,
    },
    /// Degradê radial com o mesmo centro (raio interno `r0`, externo `r1`).
    Radial {
        cx: f32,
        cy: f32,
        r0: f32,
        r1: f32,
        stops: Vec<(f32, Color)>,
    },
}

/// Cor CSS: `#rgb`, `#rrggbb`, `#rrggbbaa` ou `rgba(r,g,b,a)`; o que não entender vira preto.
pub fn css(s: &str) -> Color {
    let s = s.trim();
    if let Some(h) = s.strip_prefix('#') {
        let v = |i: usize| u8::from_str_radix(h.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
        return match h.len() {
            3 => {
                let d = |i: usize| u8::from_str_radix(&h[i..=i].repeat(2), 16).unwrap_or(0);
                Color::from_rgba8(d(0), d(1), d(2), 255)
            }
            8 => Color::from_rgba8(v(0), v(2), v(4), v(6)),
            _ => Color::from_rgba8(v(0), v(2), v(4), 255),
        };
    }
    if let Some(inner) = s.strip_prefix("rgba(").and_then(|x| x.strip_suffix(')')) {
        let p: Vec<f32> = inner.split(',').filter_map(|x| x.trim().parse().ok()).collect();
        if p.len() == 4 {
            return Color::from_rgba8(p[0] as u8, p[1] as u8, p[2] as u8, (p[3].clamp(0.0, 1.0) * 255.0).round() as u8);
        }
    }
    Color::BLACK
}

#[derive(Clone)]
struct State {
    ts: Transform,
    alpha: f32,
    clip: Option<Mask>,
    fill: Style,
    stroke: Style,
    lw: f32,
    cap: LineCap,
    join: LineJoin,
}

pub struct Canvas {
    pub pm: Pixmap,
    st: State,
    stack: Vec<State>,
    pb: PathBuilder,
    /// Há ponto atual no caminho (para `arc` emendar com uma linha, como no navegador).
    open: bool,
}

impl Canvas {
    pub fn new(w: u32, h: u32) -> Option<Self> {
        Some(Self {
            pm: Pixmap::new(w, h)?,
            st: State {
                ts: Transform::identity(),
                alpha: 1.0,
                clip: None,
                fill: Style::Solid(Color::BLACK),
                stroke: Style::Solid(Color::BLACK),
                lw: 1.0,
                cap: LineCap::Butt,
                join: LineJoin::Miter,
            },
            stack: Vec::new(),
            pb: PathBuilder::new(),
            open: false,
        })
    }

    pub fn save(&mut self) {
        self.stack.push(self.st.clone());
    }
    pub fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.st = s;
        }
    }
    pub fn translate(&mut self, x: f32, y: f32) {
        self.st.ts = self.st.ts.pre_translate(x, y);
    }
    pub fn rotate(&mut self, a: f32) {
        self.st.ts = self.st.ts.pre_concat(Transform::from_rotate(a.to_degrees()));
    }
    pub fn scale(&mut self, x: f32, y: f32) {
        self.st.ts = self.st.ts.pre_scale(x, y);
    }
    pub fn set_alpha(&mut self, a: f32) {
        self.st.alpha = a.clamp(0.0, 1.0);
    }
    pub fn fill_style(&mut self, s: Style) {
        self.st.fill = s;
    }
    pub fn fill_color(&mut self, c: &str) {
        self.st.fill = Style::Solid(css(c));
    }
    pub fn stroke_color(&mut self, c: &str) {
        self.st.stroke = Style::Solid(css(c));
    }
    pub fn line_width(&mut self, w: f32) {
        self.st.lw = w;
    }
    pub fn round_cap(&mut self) {
        self.st.cap = LineCap::Round;
    }
    pub fn round_join(&mut self) {
        self.st.join = LineJoin::Round;
    }
    pub fn clear(&mut self) {
        self.pm.fill(Color::TRANSPARENT);
    }

    // ---------- caminho ----------
    pub fn begin_path(&mut self) {
        self.pb = PathBuilder::new();
        self.open = false;
    }
    pub fn move_to(&mut self, x: f32, y: f32) {
        self.pb.move_to(x, y);
        self.open = true;
    }
    pub fn line_to(&mut self, x: f32, y: f32) {
        if self.open {
            self.pb.line_to(x, y);
        } else {
            self.move_to(x, y);
        }
    }
    pub fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        if !self.open {
            self.move_to(cx, cy);
        }
        self.pb.quad_to(cx, cy, x, y);
    }
    pub fn bezier_to(&mut self, c1x: f32, c1y: f32, c2x: f32, c2y: f32, x: f32, y: f32) {
        if !self.open {
            self.move_to(c1x, c1y);
        }
        self.pb.cubic_to(c1x, c1y, c2x, c2y, x, y);
    }
    pub fn close_path(&mut self) {
        self.pb.close();
    }
    /// `ellipse(x, y, rx, ry, rot, a0, a1)` (sentido horário), em segmentos curtos.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(&mut self, x: f32, y: f32, rx: f32, ry: f32, rot: f32, a0: f32, a1: f32) {
        let n = (((a1 - a0).abs() / (PI * 2.0)) * 48.0).ceil().max(4.0) as usize;
        let (sr, cr) = rot.sin_cos();
        for i in 0..=n {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            let (px, py) = (rx * a.cos(), ry * a.sin());
            let (qx, qy) = (x + px * cr - py * sr, y + px * sr + py * cr);
            if i == 0 { self.line_to(qx, qy) } else { self.pb.line_to(qx, qy) }
        }
    }
    pub fn arc(&mut self, x: f32, y: f32, r: f32, a0: f32, a1: f32) {
        self.ellipse(x, y, r, r, 0.0, a0, a1);
    }

    fn path(&self) -> Option<Path> {
        self.pb.clone().finish()
    }

    fn paint(&self, s: &Style) -> Paint<'static> {
        let a = self.st.alpha;
        let fade = |c: Color| {
            let mut c = c;
            c.apply_opacity(a);
            c
        };
        let stops = |v: &[(f32, Color)]| v.iter().map(|(o, c)| GradientStop::new(*o, fade(*c))).collect::<Vec<_>>();
        let shader = match s {
            Style::Solid(c) => Shader::SolidColor(fade(*c)),
            Style::Linear { x0, y0, x1, y1, stops: v } => LinearGradient::new(
                Point::from_xy(*x0, *y0),
                Point::from_xy(*x1, *y1),
                stops(v),
                SpreadMode::Pad,
                Transform::identity(),
            )
            .unwrap_or(Shader::SolidColor(fade(v.first().map_or(Color::BLACK, |x| x.1)))),
            Style::Radial { cx, cy, r0, r1, stops: v } => {
                // o tiny-skia começa no centro: as paradas são espremidas para depois do raio interno
                let k = if *r1 > 0.0 { r0 / r1 } else { 0.0 };
                let squeezed: Vec<(f32, Color)> = v.iter().map(|(o, c)| (k + o * (1.0 - k), *c)).collect();
                let mut all = vec![(0.0, squeezed.first().map_or(Color::BLACK, |x| x.1))];
                all.extend(squeezed);
                let c = Point::from_xy(*cx, *cy);
                RadialGradient::new(c, c, r1.max(0.01), stops(&all), SpreadMode::Pad, Transform::identity())
                    .unwrap_or(Shader::SolidColor(Color::TRANSPARENT))
            }
        };
        Paint { shader, anti_alias: true, ..Paint::default() }
    }

    pub fn fill(&mut self) {
        let Some(p) = self.path() else { return };
        let paint = self.paint(&self.st.fill.clone());
        self.pm.fill_path(&p, &paint, FillRule::Winding, self.st.ts, self.st.clip.as_ref());
    }
    pub fn stroke(&mut self) {
        let Some(p) = self.path() else { return };
        let paint = self.paint(&self.st.stroke.clone());
        let stroke = Stroke { width: self.st.lw, line_cap: self.st.cap, line_join: self.st.join, ..Stroke::default() };
        self.pm.stroke_path(&p, &paint, &stroke, self.st.ts, self.st.clip.as_ref());
    }
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32) {
        let Some(r) = Rect::from_xywh(x, y, w, h) else { return };
        let p = PathBuilder::from_rect(r);
        let paint = self.paint(&self.st.fill.clone());
        self.pm.fill_path(&p, &paint, FillRule::Winding, self.st.ts, self.st.clip.as_ref());
    }
    /// Recorta pelo caminho atual (intersecção com o recorte que já houver).
    pub fn clip(&mut self) {
        let Some(p) = self.path() else { return };
        match self.st.clip.as_mut() {
            Some(m) => m.intersect_path(&p, FillRule::Winding, true, self.st.ts),
            None => {
                if let Some(mut m) = Mask::new(self.pm.width(), self.pm.height()) {
                    m.fill_path(&p, FillRule::Winding, true, self.st.ts);
                    self.st.clip = Some(m);
                }
            }
        }
    }

    /// Pixels RGBA sem alfa pré-multiplicado (para uma `Image` do Bevy).
    pub fn rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pm.data().len());
        for p in self.pm.pixels() {
            let c = p.demultiply();
            out.extend_from_slice(&[c.red(), c.green(), c.blue(), c.alpha()]);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_shapes() {
        assert_eq!(css("#ff0000").to_color_u8().red(), 255);
        assert_eq!(css("#a98bff88").to_color_u8().alpha(), 0x88);
        assert_eq!(css("rgba(0,0,0,.16)").to_color_u8().alpha(), 41);
        let mut c = Canvas::new(20, 20).unwrap();
        c.begin_path();
        c.arc(10.0, 10.0, 5.0, 0.0, PI * 2.0);
        c.fill_color("#00ff00");
        c.fill();
        let px = c.rgba();
        let at = |x: usize, y: usize| &px[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4];
        assert_eq!(at(10, 10), &[0, 255, 0, 255]);
        assert_eq!(at(1, 1)[3], 0);
        // recorte: só a metade de cima recebe o vermelho
        c.save();
        c.begin_path();
        c.move_to(0.0, 0.0);
        c.line_to(20.0, 0.0);
        c.line_to(20.0, 10.0);
        c.line_to(0.0, 10.0);
        c.close_path();
        c.clip();
        c.fill_color("#ff0000");
        c.fill_rect(0.0, 0.0, 20.0, 20.0);
        c.restore();
        let px = c.rgba();
        let at = |x: usize, y: usize| &px[(y * 20 + x) * 4..(y * 20 + x) * 4 + 4];
        assert_eq!(at(5, 5)[0], 255);
        assert_eq!(at(5, 15)[0], 0);
    }
}
