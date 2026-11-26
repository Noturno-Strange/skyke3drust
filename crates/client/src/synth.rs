//! Um pequeno Web Audio em Rust: o bastante para portar os sons sintetizados do original (`assets/app-sound.js`) quase
//! linha a linha. Grafo de nós (osciladores, ruído, filtros biquad, ganho, panorâmica, reverb, limitador) com parâmetros
//! automatizáveis como `AudioParam` (`setValueAtTime`, rampas linear e exponencial, `setTargetAtTime`, curvas) e
//! modulação por outro nó (vibrato, tremido). O mixer roda num `rodio::Source` próprio, em blocos.
//!
//! Os filtros seguem as fórmulas do Web Audio (as do "Audio EQ Cookbook"; em passa-baixa e passa-alta o `Q` é em dB).
//! Os nós de um som recebem um prazo de vida e somem sozinhos depois dele.

use std::f32::consts::PI;
use std::sync::{Arc, Mutex};

pub type Id = usize;

/// Tipo de onda do oscilador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum Wave {
    Sine,
    Triangle,
    Square,
    Saw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    Lowpass,
    Highpass,
    Bandpass,
}

#[derive(Debug, Clone, Copy)]
enum Ev {
    Set(f64, f32),
    Lin(f64, f32),
    Exp(f64, f32),
    Target(f64, f32, f32),
}

/// `AudioParam`: valor com eventos no tempo e nós que somam à saída (modulação).
#[derive(Debug, Clone, Default)]
pub struct Param {
    /// Valor e tempo de onde parte o primeiro evento (o que já passou fica consolidado aqui).
    base: (f64, f32),
    ev: Vec<Ev>,
    mods: Vec<Id>,
}

impl Param {
    fn new(v: f32) -> Self {
        Self { base: (f64::NEG_INFINITY, v), ..Default::default() }
    }

    /// Valor da automação em `t` (sem a modulação), pelas regras do Web Audio: uma rampa vai do evento anterior até o
    /// tempo dela; `setTargetAtTime` se aproxima do alvo a partir do valor que havia quando começou.
    fn at(&mut self, t: f64) -> f32 {
        // consolida eventos que já terminaram e não são o último (a rampa seguinte parte do valor deles)
        while self.ev.len() >= 2 && ev_t(&self.ev[1]) <= t && !matches!(self.ev[0], Ev::Target(..)) {
            let e = self.ev.remove(0);
            self.base = (ev_t(&e), ev_v(&e));
        }
        let (mut t0, mut v0) = self.base;
        let mut target: Option<(f64, f32, f32, f32)> = None;
        for e in &self.ev {
            // valor corrente no início deste evento, se havia uma aproximação de alvo em andamento
            let start_v = |tt: f64, target: &Option<(f64, f32, f32, f32)>, v0: f32| match target {
                Some((et, x, tc, from)) => x + (from - x) * (-(tt - et) / f64::from(tc.max(1e-4))).exp() as f32,
                None => v0,
            };
            match *e {
                Ev::Set(et, x) => {
                    if t < et {
                        break;
                    }
                    (t0, v0, target) = (et, x, None);
                }
                Ev::Lin(et, x) | Ev::Exp(et, x) => {
                    let from = start_v(t0.max(f64::MIN), &target, v0);
                    if t < et {
                        if !t0.is_finite() || et <= t0 {
                            return from;
                        }
                        let k = ((t - t0) / (et - t0)).clamp(0.0, 1.0);
                        return if matches!(e, Ev::Lin(..)) {
                            from + (x - from) * k as f32
                        } else if from != 0.0 && from.signum() == x.signum() {
                            (f64::from(from) * (f64::from(x) / f64::from(from)).powf(k)) as f32
                        } else {
                            from
                        };
                    }
                    (t0, v0, target) = (et, x, None);
                }
                Ev::Target(et, x, tc) => {
                    if t < et {
                        break;
                    }
                    let from = start_v(et, &target, v0);
                    (t0, v0, target) = (et, from, Some((et, x, tc, from)));
                }
            }
        }
        match target {
            Some((et, x, tc, from)) => x + (from - x) * (-(t - et) / f64::from(tc.max(1e-4))).exp() as f32,
            None => v0,
        }
    }

    pub fn set(&mut self, t: f64, v: f32) {
        self.ev.push(Ev::Set(t, v));
    }
    pub fn lin(&mut self, t: f64, v: f32) {
        self.ev.push(Ev::Lin(t, v));
    }
    pub fn exp(&mut self, t: f64, v: f32) {
        self.ev.push(Ev::Exp(t, v));
    }
    pub fn target(&mut self, t: f64, v: f32, tc: f32) {
        self.ev.push(Ev::Target(t, v, tc));
    }
    /// `setValueCurveAtTime`: pontos espalhados por `d` segundos, com interpolação linear.
    pub fn curve(&mut self, c: &[f32], t: f64, d: f64) {
        let n = c.len().max(2) - 1;
        for (i, v) in c.iter().enumerate() {
            let tt = t + d * i as f64 / n as f64;
            if i == 0 { self.set(tt, *v) } else { self.lin(tt, *v) }
        }
    }
    /// Valor fixo (como `param.value = x`), sem automação.
    pub fn fixed(&mut self, v: f32) {
        self.ev.clear();
        self.base = (f64::NEG_INFINITY, v);
    }
}

fn ev_v(e: &Ev) -> f32 {
    match *e {
        Ev::Set(_, v) | Ev::Lin(_, v) | Ev::Exp(_, v) | Ev::Target(_, v, _) => v,
    }
}

fn ev_t(e: &Ev) -> f64 {
    match *e {
        Ev::Set(t, _) | Ev::Lin(t, _) | Ev::Exp(t, _) | Ev::Target(t, _, _) => t,
    }
}

#[derive(Debug)]
enum Kind {
    Osc { wave: Wave, freq: Param, detune: Param, phase: f32, start: f64, stop: f64 },
    Noise { buf: Arc<Vec<f32>>, pos: f64, looped: bool, start: f64, stop: f64 },
    Biquad { ty: Filter, freq: Param, q: Param, z: [[f32; 2]; 2], co: Option<(f32, f32, [f32; 5])> },
    Gain { gain: Param },
    Pan { pan: Param },
    Reverb(Box<Reverb>),
    Limiter,
}

#[derive(Debug)]
struct Node {
    kind: Kind,
    inputs: Vec<Id>,
    /// Some daqui a pouco (sons pontuais); `f64::INFINITY` para os fixos.
    expire: f64,
    out: [f32; 2],
    tick: u64,
}

/// Reverb curta (cerca de 1,1 s): filtros pente com amortecimento e passa-tudo em série, à moda do Freeverb.
#[derive(Debug)]
struct Reverb {
    combs: [[(Vec<f32>, usize, f32); 4]; 2],
    alls: [[(Vec<f32>, usize); 2]; 2],
}

impl Reverb {
    fn new(sr: f32) -> Self {
        let k = sr / 44100.0;
        let comb = |n: usize| (vec![0.0; (n as f32 * k) as usize + 1], 0usize, 0.0f32);
        let all = |n: usize| (vec![0.0; (n as f32 * k) as usize + 1], 0usize);
        Self {
            combs: [[comb(1116), comb(1188), comb(1277), comb(1356)], [comb(1139), comb(1211), comb(1300), comb(1379)]],
            alls: [[all(556), all(441)], [all(579), all(464)]],
        }
    }
    fn run(&mut self, x: f32) -> [f32; 2] {
        let mut out = [0.0; 2];
        for (ch, o) in out.iter_mut().enumerate() {
            let mut acc = 0.0;
            for (buf, i, lp) in &mut self.combs[ch] {
                let y = buf[*i];
                *lp = y * 0.55 + *lp * 0.45;
                buf[*i] = x + *lp * 0.78;
                *i = (*i + 1) % buf.len();
                acc += y;
            }
            let mut y = acc * 0.25;
            for (buf, i) in &mut self.alls[ch] {
                let b = buf[*i];
                buf[*i] = y + b * 0.5;
                y = b - y;
                *i = (*i + 1) % buf.len();
            }
            *o = y;
        }
        out
    }
}

/// Limitador suave do original (`LIM`): reto até 0,6 e curva macia até 0,95.
pub fn limit(x: f32) -> f32 {
    let m = x.abs().min(1.0);
    x.signum() * if m < 0.6 { m } else { 0.6 + 0.35 * ((m - 0.6) / 0.35).tanh() }
}

/// Gerador aleatório simples (ruído e variações), sem depender do relógio do sistema.
#[derive(Debug, Clone)]
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    /// `Math.random()`: [0, 1).
    pub fn f(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
}

/// O grafo e o relógio de áudio.
#[derive(Debug)]
pub struct Engine {
    pub sr: f32,
    /// Tempo atual (s), como `AudioContext.currentTime`.
    pub t: f64,
    nodes: Vec<Option<Node>>,
    free: Vec<Id>,
    tick: u64,
    out: Id,
    pub rng: Rng,
    pub noise: Arc<Vec<f32>>,
}

impl Engine {
    pub fn new(sr: f32) -> Self {
        let mut rng = Rng::new(0x9e37_79b9_7f4a_7c15);
        let noise = Arc::new((0..(sr * 3.0) as usize).map(|_| rng.f() * 2.0 - 1.0).collect());
        let mut e = Self { sr, t: 0.0, nodes: Vec::new(), free: Vec::new(), tick: 0, out: 0, rng, noise };
        e.out = e.add(Kind::Limiter, f64::INFINITY);
        e
    }

    /// Saída final (passa pelo limitador).
    pub fn dest(&self) -> Id {
        self.out
    }

    fn add(&mut self, kind: Kind, expire: f64) -> Id {
        let n = Node { kind, inputs: Vec::new(), expire, out: [0.0; 2], tick: u64::MAX };
        match self.free.pop() {
            Some(i) => {
                self.nodes[i] = Some(n);
                i
            }
            None => {
                self.nodes.push(Some(n));
                self.nodes.len() - 1
            }
        }
    }

    pub fn osc(&mut self, wave: Wave, f: f32, start: f64, stop: f64) -> Id {
        let expire = stop + 0.05;
        self.add(Kind::Osc { wave, freq: Param::new(f), detune: Param::new(0.0), phase: 0.0, start, stop }, expire)
    }
    /// Fonte de ruído: começa em `start`, do ponto `offset` (s) do buffer, por `dur` segundos (ou em laço).
    pub fn noise_src(&mut self, buf: Arc<Vec<f32>>, start: f64, offset: f32, dur: Option<f64>) -> Id {
        let stop = dur.map_or(f64::INFINITY, |d| start + d);
        let pos = f64::from(offset * self.sr);
        let expire = stop + 0.05;
        self.add(Kind::Noise { buf, pos, looped: dur.is_none(), start, stop }, expire)
    }
    pub fn biquad(&mut self, ty: Filter, f: f32, q: f32, expire: f64) -> Id {
        self.add(Kind::Biquad { ty, freq: Param::new(f), q: Param::new(q), z: [[0.0; 2]; 2], co: None }, expire)
    }
    pub fn gain(&mut self, g: f32, expire: f64) -> Id {
        self.add(Kind::Gain { gain: Param::new(g) }, expire)
    }
    pub fn pan(&mut self, p: f32, expire: f64) -> Id {
        self.add(Kind::Pan { pan: Param::new(p) }, expire)
    }
    pub fn reverb(&mut self) -> Id {
        let r = Box::new(Reverb::new(self.sr));
        self.add(Kind::Reverb(r), f64::INFINITY)
    }

    pub fn connect(&mut self, from: Id, to: Id) {
        if let Some(Some(n)) = self.nodes.get_mut(to) {
            n.inputs.push(from);
        }
    }
    pub fn set_expire(&mut self, id: Id, t: f64) {
        if let Some(Some(n)) = self.nodes.get_mut(id) {
            n.expire = t;
        }
    }

    /// Parâmetro de um nó: `freq`, `detune`, `gain`, `q`, `pan`.
    pub fn param(&mut self, id: Id, which: &str) -> Option<&mut Param> {
        let n = self.nodes.get_mut(id)?.as_mut()?;
        match (&mut n.kind, which) {
            (Kind::Osc { freq, .. } | Kind::Biquad { freq, .. }, "freq") => Some(freq),
            (Kind::Osc { detune, .. }, "detune") => Some(detune),
            (Kind::Gain { gain }, "gain") => Some(gain),
            (Kind::Biquad { q, .. }, "q") => Some(q),
            (Kind::Pan { pan }, "pan") => Some(pan),
            _ => None,
        }
    }
    /// Liga a saída de `from` a um parâmetro (soma ao valor, como no Web Audio).
    pub fn modulate(&mut self, from: Id, to: Id, which: &str) {
        if let Some(p) = self.param(to, which) {
            p.mods.push(from);
        }
    }

    fn eval(&mut self, id: Id) -> [f32; 2] {
        let tick = self.tick;
        let Some(Some(n)) = self.nodes.get(id) else { return [0.0; 2] };
        if n.tick == tick {
            return n.out;
        }
        let inputs = n.inputs.clone();
        let mut sum = [0.0f32; 2];
        for i in inputs {
            let o = self.eval(i);
            sum[0] += o[0];
            sum[1] += o[1];
        }
        // modulações dos parâmetros deste nó
        let mods: Vec<(u8, Id)> = match &self.nodes[id].as_ref().map(|n| &n.kind) {
            Some(Kind::Osc { freq, detune, .. }) => {
                freq.mods.iter().map(|m| (0u8, *m)).chain(detune.mods.iter().map(|m| (1u8, *m))).collect()
            }
            Some(Kind::Gain { gain }) => gain.mods.iter().map(|m| (0u8, *m)).collect(),
            Some(Kind::Biquad { freq, .. }) => freq.mods.iter().map(|m| (0u8, *m)).collect(),
            _ => Vec::new(),
        };
        let mut m = [0.0f32; 2];
        for (w, mid) in mods {
            m[w as usize] += self.eval(mid)[0];
        }
        let (t, sr) = (self.t, self.sr);
        let Some(Some(n)) = self.nodes.get_mut(id) else { return [0.0; 2] };
        let out = match &mut n.kind {
            Kind::Osc { wave, freq, detune, phase, start, stop } => {
                if t < *start || t >= *stop {
                    [0.0; 2]
                } else {
                    let f = (freq.at(t) + m[0]) * 2f32.powf((detune.at(t) + m[1]) / 1200.0);
                    let dt = (f / sr).clamp(-0.5, 0.5);
                    let p = *phase;
                    let y = match wave {
                        Wave::Sine => (p * 2.0 * PI).sin(),
                        Wave::Triangle => tri(p),
                        Wave::Saw => 2.0 * p - 1.0 - blep(p, dt),
                        Wave::Square => {
                            let s = if p < 0.5 { 1.0 } else { -1.0 };
                            s + blep(p, dt) - blep((p + 0.5) % 1.0, dt)
                        }
                    };
                    *phase = (p + dt).rem_euclid(1.0);
                    [y, y]
                }
            }
            Kind::Noise { buf, pos, looped, start, stop } => {
                if t < *start || t >= *stop || buf.is_empty() {
                    [0.0; 2]
                } else {
                    let len = buf.len() as f64;
                    if *pos >= len {
                        if *looped { *pos %= len } else { return [0.0; 2] }
                    }
                    let y = buf[*pos as usize];
                    *pos += 1.0;
                    [y, y]
                }
            }
            Kind::Biquad { ty, freq, q, z, co } => {
                let f = (freq.at(t) + m[0]).clamp(10.0, sr * 0.49);
                let qv = q.at(t);
                if co.is_none_or(|(cf, cq, _)| (cf - f).abs() > 0.5 || (cq - qv).abs() > 1e-4) {
                    *co = Some((f, qv, coeffs(*ty, f, qv, sr)));
                }
                let c = co.map(|x| x.2).unwrap_or_default();
                let mut y = [0.0; 2];
                for ch in 0..2 {
                    // forma direta transposta II
                    let x = sum[ch];
                    let o = c[0] * x + z[ch][0];
                    z[ch][0] = c[1] * x - c[3] * o + z[ch][1];
                    z[ch][1] = c[2] * x - c[4] * o;
                    y[ch] = o;
                }
                y
            }
            Kind::Gain { gain } => {
                let g = gain.at(t) + m[0];
                [sum[0] * g, sum[1] * g]
            }
            Kind::Pan { pan } => {
                let x = (pan.at(t).clamp(-1.0, 1.0) + 1.0) / 2.0;
                let mono = (sum[0] + sum[1]) * 0.5;
                [mono * (x * PI / 2.0).cos(), mono * (x * PI / 2.0).sin()]
            }
            Kind::Reverb(r) => r.run((sum[0] + sum[1]) * 0.5),
            Kind::Limiter => [limit(sum[0]), limit(sum[1])],
        };
        n.out = out;
        n.tick = tick;
        out
    }

    /// Avança uma amostra e devolve o estéreo da saída.
    pub fn frame(&mut self) -> [f32; 2] {
        self.tick = self.tick.wrapping_add(1);
        let o = self.eval(self.out);
        self.t += 1.0 / f64::from(self.sr);
        o
    }

    /// Remove os nós vencidos (e as ligações para eles).
    pub fn collect(&mut self) {
        let t = self.t;
        let mut gone = Vec::new();
        for (i, n) in self.nodes.iter_mut().enumerate() {
            if n.as_ref().is_some_and(|n| n.expire < t) {
                *n = None;
                gone.push(i);
            }
        }
        if gone.is_empty() {
            return;
        }
        for n in self.nodes.iter_mut().flatten() {
            n.inputs.retain(|i| !gone.contains(i));
            match &mut n.kind {
                Kind::Osc { freq, detune, .. } => {
                    freq.mods.retain(|i| !gone.contains(i));
                    detune.mods.retain(|i| !gone.contains(i));
                }
                Kind::Gain { gain } => gain.mods.retain(|i| !gone.contains(i)),
                Kind::Biquad { freq, .. } => freq.mods.retain(|i| !gone.contains(i)),
                _ => {}
            }
        }
        self.free.extend(gone);
    }

    #[cfg(test)]
    pub fn live(&self) -> usize {
        self.nodes.iter().flatten().count()
    }
}

fn tri(p: f32) -> f32 {
    // triângulo começando em 0 e subindo, como o do Web Audio
    let x = (p + 0.25).rem_euclid(1.0);
    if x < 0.5 { 4.0 * x - 1.0 } else { 3.0 - 4.0 * x }
}

/// Correção PolyBLEP: suaviza os saltos da serra e da quadrada (menos chiado de aliasing).
fn blep(p: f32, dt: f32) -> f32 {
    let dt = dt.abs().max(1e-6);
    if p < dt {
        let x = p / dt;
        x + x - x * x - 1.0
    } else if p > 1.0 - dt {
        let x = (p - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Coeficientes normalizados [b0, b1, b2, a1, a2] do biquad (Web Audio).
fn coeffs(ty: Filter, f: f32, q: f32, sr: f32) -> [f32; 5] {
    let w0 = 2.0 * PI * f / sr;
    let (s, c) = w0.sin_cos();
    let (b0, b1, b2, a0, a1, a2) = match ty {
        Filter::Lowpass | Filter::Highpass => {
            let ql = 10f32.powf(q / 20.0).max(1e-4);
            let alpha = s / (2.0 * ql);
            if ty == Filter::Lowpass {
                ((1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
            } else {
                ((1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
            }
        }
        Filter::Bandpass => {
            let alpha = s / (2.0 * q.max(1e-4));
            (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * c, 1.0 - alpha)
        }
    };
    [b0 / a0, b1 / a0, b2 / a0, a1 / a0, a2 / a0]
}

/// Fonte para o `rodio`: renderiza o motor em blocos (uma trava por bloco).
pub struct Stream {
    pub engine: Arc<Mutex<Engine>>,
    buf: Vec<f32>,
    i: usize,
    blocks: u32,
}

impl Stream {
    pub fn new(engine: Arc<Mutex<Engine>>) -> Self {
        Self { engine, buf: Vec::new(), i: 0, blocks: 0 }
    }
    fn refill(&mut self) {
        self.buf.clear();
        self.i = 0;
        let Ok(mut e) = self.engine.lock() else {
            self.buf.resize(512 * 2, 0.0);
            return;
        };
        for _ in 0..512 {
            let o = e.frame();
            self.buf.extend_from_slice(&o);
        }
        self.blocks += 1;
        if self.blocks % 16 == 0 {
            e.collect();
        }
    }
}

impl Iterator for Stream {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.i >= self.buf.len() {
            self.refill();
        }
        let v = self.buf.get(self.i).copied().unwrap_or(0.0);
        self.i += 1;
        Some(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn params_follow_web_audio() {
        let mut p = Param::new(1.0);
        p.set(0.0, 0.0);
        p.lin(1.0, 1.0);
        p.exp(2.0, 0.001);
        assert!((p.at(0.5) - 0.5).abs() < 1e-3);
        assert!((p.at(1.0) - 1.0).abs() < 1e-3);
        // exponencial: na metade, a média geométrica
        assert!((p.at(1.5) - (0.001f32).sqrt()).abs() < 1e-3);
        assert!((p.at(3.0) - 0.001).abs() < 1e-6);
        let mut q = Param::new(0.0);
        q.target(0.0, 1.0, 0.5);
        assert!((q.at(0.5) - (1.0 - (-1.0f32).exp())).abs() < 1e-3);
    }

    #[test]
    fn tone_and_filter_and_cleanup() {
        let mut e = Engine::new(8000.0);
        let o = e.osc(Wave::Sine, 440.0, 0.0, 0.5);
        let g = e.gain(0.5, 0.55);
        e.connect(o, g);
        let d = e.dest();
        e.connect(g, d);
        let mut peak = 0.0f32;
        for _ in 0..4000 {
            peak = peak.max(e.frame()[0].abs());
        }
        assert!((peak - 0.5).abs() < 0.02);
        for _ in 0..1000 {
            e.frame();
        }
        e.collect();
        assert_eq!(e.live(), 1);
        // passa-baixa de 200 Hz quase apaga um seno de 3 kHz
        let mut e = Engine::new(16000.0);
        let o = e.osc(Wave::Sine, 3000.0, 0.0, 1.0);
        let f = e.biquad(Filter::Lowpass, 200.0, 0.0, 2.0);
        e.connect(o, f);
        let d = e.dest();
        e.connect(f, d);
        let mut peak = 0.0f32;
        for i in 0..16000 {
            let y = e.frame()[0].abs();
            if i > 2000 {
                peak = peak.max(y);
            }
        }
        assert!(peak < 0.02, "{peak}");
        assert!((limit(0.5) - 0.5).abs() < 1e-6 && limit(3.0) < 0.951);
    }
}
