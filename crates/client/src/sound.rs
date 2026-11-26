//! Sons do jogo, sintetizados como no original (`assets/app-sound.js` e `assets/lofi.js`), sobre o motor de `synth`:
//! efeitos (interface, corpo, passos, emoções, itens, culinária, fazenda, medicina), a música lofi de cada mundo e da tela
//! inicial (sequenciador com piano elétrico, baixo, bateria e melodia) e o ambiente de cada mundo (bichos com chamados
//! de gravações reais, cada um de um ponto sorteado, com eco da mata). Três canais com volume nos Ajustes: efeitos,
//! ambiente e música.

use crate::synth::{Engine, Filter, Id, Stream, Wave};
use bevy::audio::{AddAudioSource, AudioPlayer, Decodable, Source};
use bevy::prelude::*;
use std::num::NonZero;
use std::sync::{Arc, Mutex};

const SR: u32 = 44_100;

/// Pedido de som (`sfx(k, vol)`): o volume já leva em conta a distância de quem fez.
#[derive(Message, Debug, Clone)]
pub struct Sfx(pub String, pub f32);

/// Volume pela distância até quem fez o som (`dvol`): 1 de perto, nada a partir de 20 m.
pub fn dvol(d: f32) -> f32 {
    (1.0 - d / 20.0).max(0.0).powf(1.5)
}

/// Ganho de calibração de cada som (`TRIM`, medido no original por `tools/audio-audit.js`).
fn trim(k: &str) -> f32 {
    match k {
        "rain" => 0.53,
        "erro" => 2.14,
        "pickup" => 2.16,
        "wash" => 4.95,
        "peel" => 5.75,
        "cook_done" => 2.07,
        "eat" => 1.66,
        "drink" => 0.96,
        "notif" => 1.87,
        "scream" => 1.14,
        "fart" => 0.9,
        "join" => 2.11,
        "leave" => 1.64,
        "chat" => 3.03,
        "click" => 4.68,
        "confirm" => 2.25,
        "page" => 3.09,
        "sit" => 1.27,
        "stand" => 3.3,
        "jump" => 4.19,
        "land" => 1.44,
        "step_sand" => 1.68,
        "step_grass" => 2.13,
        "step_water" => 2.08,
        "step_rock" => 1.78,
        "e_laugh" => 2.56,
        "e_love" => 1.57,
        "e_party" => 2.28,
        "e_clap" => 4.26,
        "e_fire" => 2.71,
        "e_sparkle" => 5.07,
        "e_sad" => 2.16,
        "e_angry" => 1.95,
        "e_wow" => 2.25,
        "e_sleep" => 2.97,
        "e_pop" => 4.42,
        "g_whoosh" => 2.85,
        "g_dance" => 1.42,
        "n_video" => 2.49,
        "n_dj" => 1.3,
        "n_karaoke" => 2.56,
        "amb_amazora" => 2.15,
        "amb_cerron" => 2.35,
        "amb_atlanthia" => 1.97,
        "amb_caatara" => 2.13,
        "amb_panthara" => 2.03,
        "amb_pamphora" => 2.41,
        "mus_menu" => 1.26,
        "mus_tutorial" => 1.23,
        "mus_amazora" => 1.33,
        "mus_cerron" => 1.3,
        "mus_atlanthia" => 1.21,
        "mus_caatara" => 1.35,
        "mus_panthara" => 1.29,
        "mus_pamphora" => 1.1,
        _ => 1.0,
    }
}

/// Som de cada emoji (`EMO_SFX`).
pub fn emo_kind(ic: &str) -> &'static str {
    match ic {
        "😂" => "e_laugh",
        "😍" | "❤️" => "e_love",
        "🥳" | "🎉" => "e_party",
        "👏" | "🙌" => "e_clap",
        "🔥" => "e_fire",
        "✨" => "e_sparkle",
        "😭" => "e_sad",
        "😡" | "🤢" => "e_angry",
        "😱" | "😎" => "e_wow",
        "😴" => "e_sleep",
        "💃" => "g_dance",
        _ => "e_pop",
    }
}

/// Chão de cada mundo para os passos (`STEPK`).
pub fn step_kind(world: &str) -> &'static str {
    match world {
        "amazora" | "atlanthia" | "pamphora" => "step_grass",
        "caatara" => "step_rock",
        "panthara" => "step_water",
        _ => "step_sand",
    }
}

// ---------- vozes (tone, burst, ping, vox) ----------

/// Opções do ruído filtrado (`burst`).
#[derive(Debug, Clone, Copy, Default)]
struct B {
    ty: Option<Filter>,
    at: Option<f32>,
    f1: Option<f32>,
}

/// Onde os sons de agora saem (`SB`) e com que volume (`SV`).
struct Ctx<'a> {
    e: &'a mut Engine,
    sb: Id,
    sv: f32,
}

impl Ctx<'_> {
    fn r(&mut self) -> f32 {
        self.e.rng.f()
    }

    fn env(&mut self, g: Id, t0: f64, d: f64, v: f32, at: f64) {
        let sv = self.sv;
        if let Some(p) = self.e.param(g, "gain") {
            p.set(t0, 0.0);
            p.lin(t0 + at.min(d * 0.5), v * sv);
            p.exp(t0 + d, 0.001);
        }
    }

    /// Oscilador com ataque curto e queda exponencial (`tone`).
    #[allow(clippy::too_many_arguments)]
    fn tone(&mut self, w: Wave, f0: f32, f1: f32, t0: f64, d: f64, v: f32, at: f64) {
        let end = t0 + d + 0.02;
        let o = self.e.osc(w, f0, t0, end);
        if let Some(p) = self.e.param(o, "freq") {
            p.set(t0, f0);
            if (f1 - f0).abs() > f32::EPSILON {
                p.exp(t0 + d, f1);
            }
        }
        let g = self.e.gain(0.0, end + 0.05);
        self.env(g, t0, d, v, at);
        self.e.connect(o, g);
        self.e.connect(g, self.sb);
    }

    /// Ruído filtrado (`burst`).
    fn burst(&mut self, t0: f64, d: f64, f: f32, v: f32, q: f32, o: B) {
        let off = self.r() * (2.9 - d as f32).max(0.0);
        let noise = self.e.noise.clone();
        let s = self.e.noise_src(noise, t0, off, Some(d + 0.02));
        let end = t0 + d + 0.08;
        let fl = self.e.biquad(o.ty.unwrap_or(Filter::Bandpass), f, q, end);
        if let Some(p) = self.e.param(fl, "freq") {
            p.set(t0, f);
            if let Some(f1) = o.f1 {
                p.exp(t0 + d, f1);
            }
        }
        let g = self.e.gain(0.0, end);
        self.env(g, t0, d, v, f64::from(o.at.unwrap_or(0.004)));
        self.e.connect(s, fl);
        self.e.connect(fl, g);
        self.e.connect(g, self.sb);
    }

    /// "Ping" de painel: seno com um harmônico leve uma oitava acima.
    fn ping(&mut self, f: f32, t0: f64, d: f64, v: f32) {
        self.tone(Wave::Sine, f, f, t0, d, v, 0.006);
        self.tone(Wave::Sine, f * 2.0, f * 2.0, t0, d * 0.45, v * 0.2, 0.006);
    }

    /// Voz sintética (serra filtrada) com vibrato opcional (`vox`).
    #[allow(clippy::too_many_arguments)]
    fn vox(&mut self, f0: f32, f1: f32, t0: f64, d: f64, v: f32, lp: f32, vib: f32) {
        let end = t0 + d + 0.02;
        let o = self.e.osc(Wave::Saw, f0, t0, end);
        if let Some(p) = self.e.param(o, "freq") {
            p.set(t0, f0);
            p.exp(t0 + d, f1);
        }
        let fl = self.e.biquad(Filter::Lowpass, if lp > 0.0 { lp } else { 1600.0 }, 2.0, end + 0.05);
        let g = self.e.gain(0.0, end + 0.05);
        self.env(g, t0, d, v, 0.03f64.min(d * 0.3));
        if vib > 0.0 {
            let l = self.e.osc(Wave::Sine, vib, t0, end);
            let lg = self.e.gain(f0 * 0.025, end + 0.05);
            self.e.connect(l, lg);
            self.e.modulate(lg, o, "freq");
        }
        self.e.connect(o, fl);
        self.e.connect(fl, g);
        self.e.connect(g, self.sb);
    }

    // atalhos com tempo relativo ao início do som (`sfxKit`)
    #[allow(clippy::too_many_arguments)]
    fn t(&mut self, t: f64, w: Wave, f0: f32, f1: f32, d: f64, v: f32, dl: f64) {
        self.tone(w, f0, f1, t + dl, d, v, 0.008);
    }
    #[allow(clippy::too_many_arguments)]
    fn b(&mut self, t: f64, d: f64, f: f32, v: f32, q: f32, dl: f64, o: B) {
        self.burst(t + dl, d, f, v, q, o);
    }
    fn p(&mut self, t: f64, f: f32, d: f64, v: f32, dl: f64) {
        self.ping(f, t + dl, d, v);
    }
    #[allow(clippy::too_many_arguments)]
    fn x(&mut self, t: f64, f0: f32, f1: f32, d: f64, v: f32, lp: f32, vib: f32, dl: f64) {
        self.vox(f0, f1, t + dl, d, v, lp, vib);
    }

    /// Todos os efeitos (`sfx` e `sfxMore`).
    fn sfx(&mut self, k: &str) {
        use Wave::{Saw, Sine, Triangle as Tri};
        let t = self.e.t + 0.01;
        let n = B::default();
        let lp = B { ty: Some(Filter::Lowpass), ..n };
        let hp = B { ty: Some(Filter::Highpass), ..n };
        match k {
            "eat" => {
                for i in 0..6 {
                    let d = f64::from(i as f32 * 0.25 + self.r() * 0.03);
                    let g = 1.0 - i as f32 * 0.1;
                    for c in 0..5 {
                        let (dd, f, o) = (0.012 + self.r() * 0.018, 2600.0 + self.r() * 3800.0, self.r() * 0.008);
                        self.b(t, f64::from(dd), f, 0.34 * g, 1.8, d + f64::from(c as f32 * 0.007 + o), n);
                    }
                    let f = 1100.0 + self.r() * 700.0;
                    self.b(t, 0.085, f, 0.34 * g, 1.1, d, n);
                    let f = 170.0 + self.r() * 40.0;
                    self.t(t, Sine, f, 80.0, 0.09, 0.2 * g, d);
                }
            }
            "drink" => {
                for i in 0..3 {
                    let d = f64::from(i as f32 * 0.38);
                    self.t(t, Sine, 300.0, 140.0, 0.18, 0.3, d);
                    self.b(t, 0.12, 500.0, 0.1, 2.0, d, lp);
                    let f = 420.0 + self.r() * 80.0;
                    self.t(t, Sine, f, 780.0, 0.08, 0.1, d + 0.19);
                }
            }
            "notif" => {
                self.b(t, 0.025, 4000.0, 0.08, 0.8, 0.0, hp);
                self.p(t, 784.0, 0.22, 0.13, 0.0);
                self.p(t, 1047.0, 0.44, 0.12, 0.12);
                self.t(t, Sine, 392.0, 392.0, 0.5, 0.05, 0.0);
            }
            "scream" => {
                self.x(t, 500.0, 1050.0, 0.7, 0.2, 2000.0, 6.0, 0.0);
                self.x(t, 1050.0, 620.0, 0.8, 0.2, 2000.0, 6.0, 0.7);
                self.b(t, 1.4, 2200.0, 0.12, 0.7, 0.0, n);
            }
            "fart" => {
                let (e, sv, sb) = (&mut *self.e, self.sv, self.sb);
                let o = e.osc(Saw, 95.0, t, t + 0.8);
                if let Some(p) = e.param(o, "freq") {
                    p.set(t, 95.0);
                    p.exp(t + 0.75, 48.0);
                }
                let fl = e.biquad(Filter::Lowpass, 420.0, 1.0, t + 0.9);
                let g = e.gain(0.15 * sv, t + 0.9);
                if let Some(p) = e.param(g, "gain") {
                    p.set(t, 0.15 * sv);
                    p.set(t + 0.55, 0.15 * sv);
                    p.lin(t + 0.8, 0.0);
                }
                let l = e.osc(Sine, 34.0, t, t + 0.8);
                if let Some(p) = e.param(l, "freq") {
                    p.set(t, 34.0);
                    p.exp(t + 0.75, 18.0);
                }
                let lg = e.gain(0.14 * sv, t + 0.9);
                e.connect(l, lg);
                e.modulate(lg, g, "gain");
                e.connect(o, fl);
                e.connect(fl, g);
                e.connect(g, sb);
                self.b(t, 0.7, 260.0, 0.2, 0.6, 0.0, n);
            }
            // interface e presença: pings de painel na escala de Dó
            "join" => {
                self.p(t, 523.0, 0.22, 0.1, 0.0);
                self.p(t, 659.0, 0.22, 0.1, 0.08);
                self.p(t, 784.0, 0.4, 0.11, 0.16);
            }
            "leave" => {
                self.p(t, 784.0, 0.2, 0.08, 0.0);
                self.p(t, 523.0, 0.36, 0.08, 0.1);
            }
            "chat" => {
                self.p(t, 659.0, 0.1, 0.06, 0.0);
                self.p(t, 880.0, 0.12, 0.045, 0.055);
            }
            "click" => {
                self.t(t, Tri, 880.0, 560.0, 0.045, 0.08, 0.0);
                self.b(t, 0.012, 3500.0, 0.05, 1.2, 0.0, hp);
            }
            "page" => {
                self.b(t, 0.24, 2200.0, 0.12, 0.9, 0.0, B { f1: Some(5600.0), at: Some(0.07), ..n });
                self.b(t, 0.07, 900.0, 0.06, 1.0, 0.19, lp);
            }
            "confirm" => {
                for (i, f) in [523.0, 659.0, 784.0, 1047.0].into_iter().enumerate() {
                    self.p(t, f, 0.26, 0.1, i as f64 * 0.08);
                }
            }
            "erro" => {
                self.x(t, 330.0, 262.0, 0.12, 0.1, 900.0, 0.0, 0.0);
                self.x(t, 262.0, 196.0, 0.18, 0.1, 900.0, 0.0, 0.12);
            }
            "pickup" => {
                self.t(t, Sine, 330.0, 660.0, 0.08, 0.08, 0.0);
                self.p(t, 784.0, 0.14, 0.06, 0.06);
            }
            "wash" => {
                for i in 0..4 {
                    let d = f64::from(i as f32 * 0.12 + self.r() * 0.03);
                    let f = 700.0 + self.r() * 300.0;
                    self.b(t, 0.1, f, 0.12, 1.4, d, lp);
                    let (a, b) = (300.0 + self.r() * 120.0, 600.0 + self.r() * 200.0);
                    self.t(t, Sine, a, b, 0.06, 0.04, d);
                }
            }
            "peel" => {
                for i in 0..3 {
                    let f = 2600.0 + self.r() * 600.0;
                    self.b(t, 0.07, f, 0.16, 1.3, f64::from(i as f32 * 0.14), B { f1: Some(1600.0), ..n });
                }
            }
            "cook_done" => {
                self.b(t, 0.22, 2400.0, 0.12, 0.6, 0.0, B { f1: Some(800.0), ..n });
                for (i, f) in [523.0, 659.0, 784.0, 1047.0].into_iter().enumerate() {
                    self.p(t, f, 0.32, 0.09, 0.06 + i as f64 * 0.09);
                }
                self.t(t, Tri, 1568.0, 1568.0, 0.22, 0.03, 0.46);
            }
            // corpo no traje espacial: baque grave + sopro de ar
            "sit" => {
                self.t(t, Sine, 130.0, 70.0, 0.2, 0.22, 0.0);
                self.b(t, 0.14, 420.0, 0.22, 0.7, 0.0, n);
                self.b(t, 0.12, 3500.0, 0.04, 0.8, 0.04, hp);
            }
            "stand" => {
                self.t(t, Sine, 180.0, 300.0, 0.12, 0.1, 0.0);
                self.b(t, 0.1, 1200.0, 0.1, 1.0, 0.0, n);
                self.b(t, 0.14, 3200.0, 0.04, 0.8, 0.0, hp);
            }
            "jump" => {
                self.b(t, 0.18, 500.0, 0.2, 0.7, 0.0, B { f1: Some(1800.0), ..n });
                self.t(t, Sine, 240.0, 520.0, 0.12, 0.08, 0.0);
            }
            "land" => {
                self.t(t, Sine, 110.0, 50.0, 0.16, 0.22, 0.0);
                self.b(t, 0.1, 300.0, 0.2, 0.7, 0.0, lp);
                self.b(t, 0.06, 2200.0, 0.05, 1.0, 0.0, n);
            }
            "step_sand" => {
                let r = 0.9 + self.r() * 0.2;
                self.t(t, Sine, 95.0 * r, 55.0 * r, 0.1, 0.1, 0.0);
                self.b(t, 0.12, 1100.0 * r, 0.17, 0.5, 0.0, B { f1: Some(380.0 * r), ..n });
            }
            "step_grass" => {
                let r = 0.9 + self.r() * 0.2;
                self.t(t, Sine, 100.0 * r, 60.0 * r, 0.08, 0.09, 0.0);
                self.b(t, 0.1, 1900.0 * r, 0.1, 0.8, 0.0, B { f1: Some(900.0 * r), ..n });
            }
            "step_water" => {
                let r = 0.9 + self.r() * 0.2;
                self.t(t, Sine, 90.0 * r, 55.0 * r, 0.08, 0.07, 0.0);
                self.b(t, 0.13, 380.0 * r, 0.12, 0.6, 0.0, n);
                self.t(t, Sine, 260.0 * r, 520.0 * r, 0.07, 0.05, 0.02);
                self.t(t, Sine, 330.0 * r, 700.0 * r, 0.05, 0.03, 0.07);
            }
            "step_rock" => {
                let r = 0.9 + self.r() * 0.2;
                self.t(t, Sine, 120.0 * r, 65.0 * r, 0.07, 0.12, 0.0);
                self.b(t, 0.04, 1500.0 * r, 0.16, 1.4, 0.0, n);
                self.b(t, 0.03, 2800.0 * r, 0.07, 1.5, 0.012, n);
            }
            // emoções
            "e_laugh" => {
                for i in 0..3 {
                    let f = i as f32 * 20.0;
                    self.x(t, 430.0 - f, 350.0 - f, 0.09, 0.14, 1500.0, 0.0, i as f64 * 0.12);
                }
            }
            "e_love" => {
                self.p(t, 523.0, 0.32, 0.09, 0.0);
                self.p(t, 784.0, 0.44, 0.09, 0.12);
                self.t(t, Sine, 659.0, 659.0, 0.5, 0.035, 0.12);
            }
            "e_party" => {
                self.b(t, 0.3, 1800.0, 0.14, 0.6, 0.0, B { f1: Some(6000.0), ..n });
                for (i, f) in [523.0, 659.0, 784.0, 1047.0].into_iter().enumerate() {
                    self.p(t, f, 0.2, 0.07, 0.06 + i as f64 * 0.07);
                }
            }
            "e_clap" => {
                for d in [0.0, 0.12, 0.22, 0.36, 0.46] {
                    let f = 1500.0 + self.r() * 300.0;
                    self.b(t, 0.045, f, 0.35, 1.1, d, n);
                    self.b(t, 0.03, 2600.0, 0.15, 1.0, d, n);
                }
            }
            "e_fire" => {
                self.b(t, 0.45, 180.0, 0.3, 0.5, 0.0, lp);
                self.b(t, 0.5, 700.0, 0.2, 0.5, 0.0, n);
                for _ in 0..6 {
                    let (f, d) = (2000.0 + self.r() * 2500.0, f64::from(self.r() * 0.4));
                    self.b(t, 0.012, f, 0.2, 1.3, d, n);
                }
            }
            "e_sparkle" => {
                for (i, f) in [1047.0, 1319.0, 1568.0].into_iter().enumerate() {
                    self.t(t, Tri, f, f, 0.14, 0.045, 0.05 + i as f64 * 0.06);
                }
            }
            "e_sad" => {
                self.x(t, 392.0, 294.0, 0.35, 0.1, 1100.0, 5.0, 0.0);
                self.x(t, 330.0, 247.0, 0.4, 0.1, 1100.0, 5.0, 0.25);
            }
            "e_angry" => {
                self.x(t, 150.0, 100.0, 0.3, 0.12, 500.0, 22.0, 0.0);
                self.b(t, 0.25, 300.0, 0.1, 0.8, 0.0, n);
            }
            "e_wow" => self.x(t, 330.0, 740.0, 0.22, 0.12, 1800.0, 0.0, 0.0),
            "e_sleep" => {
                self.b(t, 0.55, 500.0, 0.15, 0.7, 0.0, B { at: Some(0.2), f1: Some(300.0), ..n });
                self.t(t, Sine, 220.0, 160.0, 0.5, 0.05, 0.0);
            }
            "e_pop" => self.t(t, Sine, 520.0, 1000.0, 0.07, 0.08, 0.0),
            "g_whoosh" => {
                self.b(t, 0.3, 3000.0, 0.14, 0.5, 0.0, B { f1: Some(700.0), ..n });
                self.t(t, Sine, 300.0, 700.0, 0.25, 0.03, 0.0);
            }
            "g_dance" => {
                self.t(t, Sine, 120.0, 60.0, 0.12, 0.2, 0.0);
                self.t(t, Tri, 165.0, 165.0, 0.12, 0.07, 0.12);
            }
            "med_ouch" => {
                self.x(t, 290.0, 420.0, 0.11, 0.065, 1900.0, 0.0, 0.0);
                self.x(t, 420.0, 210.0, 0.18, 0.05, 900.0, 0.0, 0.1);
            }
            "med_heal" => {
                for (i, f) in [523.0, 659.0, 784.0, 1047.0].into_iter().enumerate() {
                    self.p(t, f, 0.35, 0.06, i as f64 * 0.09);
                }
                self.b(t, 0.3, 1800.0, 0.025, 0.7, 0.0, B { f1: Some(4000.0), ..n });
            }
            "n_video" => {
                self.b(t, 0.04, 3000.0, 0.2, 2.0, 0.0, n);
                self.t(t, Sine, 196.0, 392.0, 0.25, 0.08, 0.0);
                self.p(t, 784.0, 0.2, 0.1, 0.1);
                self.p(t, 1047.0, 0.36, 0.1, 0.22);
            }
            "n_dj" => {
                self.t(t, Sine, 130.0, 45.0, 0.16, 0.28, 0.0);
                for f in [523.0, 659.0, 784.0] {
                    self.t(t, Tri, f, f, 0.3, 0.05, 0.12);
                }
            }
            "n_karaoke" => {
                self.b(t, 0.05, 900.0, 0.25, 2.0, 0.0, n);
                self.p(t, 659.0, 0.2, 0.1, 0.1);
                self.p(t, 880.0, 0.32, 0.1, 0.2);
            }
            _ => {}
        }
    }

    /// Melodia curta de cada remédio (`medSound`): `note` é a nota MIDI da receita, `kind` a forma.
    fn med(&mut self, note: f32, kind: &str, craft: bool) {
        use Wave::{Sine, Triangle as Tri};
        let t = self.e.t + 0.01;
        let n = B::default();
        let f = 440.0 * 2f32.powf((note - 69.0) / 12.0);
        if craft {
            for (i, d) in [0.0, 0.12, 0.26].into_iter().enumerate() {
                self.t(t, Sine, f * (1.0 + i as f32 * 0.25), f * 1.8, 0.13, 0.09, d);
            }
            self.b(t, 0.25, 1400.0, 0.05, 0.8, 0.0, n);
            self.p(t, f * 2.0, 0.35, 0.08, 0.35);
            return;
        }
        let delay = if kind == "injection" { 0.32 } else { 0.0 };
        match kind {
            "drink" => {
                for i in 0..3 {
                    self.t(t, Sine, f * 1.1, f * 0.55, 0.13, 0.12, i as f64 * 0.13);
                }
            }
            "balm" => {
                self.b(t, 0.5, 1000.0, 0.08, 0.7, 0.0, B { f1: Some(2400.0), ..n });
                self.t(t, Sine, f, f * 1.5, 0.45, 0.065, 0.0);
            }
            "injection" => {
                self.t(t, Tri, 800.0, 350.0, 0.04, 0.07, 0.0);
                self.x(t, 290.0, 420.0, 0.11, 0.065, 1900.0, 0.0, 0.0);
                self.x(t, 420.0, 210.0, 0.18, 0.05, 900.0, 0.0, 0.1);
            }
            "mist" => {
                self.b(t, 0.45, 2600.0, 0.1, 0.7, 0.0, B { ty: Some(Filter::Highpass), f1: Some(1400.0), ..n });
                self.b(t, 0.22, 3900.0, 0.04, 1.0, 0.16, n);
            }
            "tablet" => {
                self.t(t, Tri, f * 3.0, f * 1.5, 0.055, 0.12, 0.0);
                self.t(t, Sine, f, f * 0.7, 0.12, 0.1, 0.12);
            }
            "ritual" => {
                for (i, r) in [1.0, 1.5, 2.0].into_iter().enumerate() {
                    self.p(t, f * r, 0.75, 0.075, i as f64 * 0.14);
                }
                self.b(t, 0.6, 2100.0, 0.035, 0.8, 0.0, n);
            }
            _ => {}
        }
        for (i, r) in [1.0, 1.25, 1.5, 2.0].into_iter().enumerate() {
            self.p(t, f * 2.0 * r, 0.32, 0.065, delay + 0.12 + i as f64 * 0.09);
        }
    }
}

// ---------- música lofi ----------

/// Uma faixa já lida (`SKYLOFI.build`).
#[derive(Debug, Clone)]
struct Track {
    bpm: f64,
    sw: f64,
    lead: &'static str,
    pad: f32,
    warm: f32,
    wob: f32,
    chords: Vec<(i32, Vec<i32>)>,
    mel: Vec<Vec<Option<(i32, i32)>>>,
    comp: &'static str,
    bass: &'static str,
    k: &'static str,
    s: &'static str,
    h: &'static str,
}

/// As composições do original (`TRACKS` em `assets/lofi.js`).
#[allow(clippy::type_complexity)]
const TRACKS: [(&str, f64, f64, &str, f32, f32, f32, &str, [&str; 5], [&str; 4]); 8] = [
    (
        "menu",
        72.0,
        0.3,
        "bell",
        0.7,
        2800.0,
        6.0,
        "Ebmaj9 Cm9 Fm9 Bb13",
        ["x.....x...x.....", "x.....f...x.....", "x.......x.x.....", "....x.......x...", "x.x.x.x.x.x.x.xo"],
        [
            "G5 . . Bb5 . . G5 . F5 . . . Eb5 . . .",
            "Eb5 . G5 . . F5 Eb5 . . . D5 . C5 . . .",
            "C5 . . Eb5 . F5 . . Ab5 . G5 . . . F5 .",
            "D5 . F5 . . . Eb5 . D5 . C5 . Bb4 . . .",
        ],
    ),
    (
        "tutorial",
        76.0,
        0.3,
        "koto",
        0.6,
        3100.0,
        7.0,
        "Fmaj9 Dm9 Gm9 C13",
        ["x...x.....x.....", "x.....x...f.....", "x.....x...x.....", "....x.......x..o", "x.o.x.o.x.o.x.oo"],
        [
            "A5 . C6 . . A5 . G5 . . F5 . . . G5 .",
            "A5 . . D6 . . C6 . A5 . . . G5 . . .",
            "Bb5 . . A5 . G5 . . F5 . G5 . . . D5 .",
            "E5 . G5 . . . A5 . . G5 . . F5 . . .",
        ],
    ),
    (
        "amazora",
        74.0,
        0.3,
        "kalimba",
        0.6,
        3200.0,
        6.0,
        "Gmaj9 Em9 Cmaj9 D13",
        ["x.....x...x.....", "x.....x.......f.", "x.....x.x.......", "....x.......x...", "x.x.x.x.x.x.x.x."],
        [
            "B5 . D6 . . B5 . . A5 . G5 . . . A5 .",
            "G5 . . B5 . . E6 . D6 . B5 . . . G5 .",
            "E5 . G5 . . . B5 . C6 . . . B5 . G5 .",
            "A5 . . F#5 . . D6 . . . C6 . A5 . . .",
        ],
    ),
    (
        "cerron",
        78.0,
        0.32,
        "rhodes",
        0.6,
        3000.0,
        8.0,
        "Cmaj9 Am9 Dm9 G13",
        ["x.....x...x...x.", "x.....x.f.....x.", "x.....x.x.......", "....x.......x...", "x.x.x.x.x.x.x.x."],
        [
            "E5 . . G5 . . A5 . G5 . E5 . . . D5 .",
            "C5 . . E5 . . G5 . . A5 . . G5 . . .",
            "A4 . D5 . . F5 . . E5 . D5 . . . A4 .",
            "B4 . D5 . G5 . . F5 . . D5 . B4 . . .",
        ],
    ),
    (
        "atlanthia",
        68.0,
        0.3,
        "marimba",
        0.9,
        2400.0,
        8.0,
        "Bbmaj9 Gm9 Ebmaj9 F13",
        ["x.......x.......", "x.....x.........", "x.......x.......", "........x.......", "..o...o...o...o."],
        [
            "D5 - - - F5 - - - Bb5 - - A5 G5 - - -",
            "G5 - - - Bb5 - - - A5 - G5 - D5 - - -",
            "Eb5 - - G5 - - Bb5 - C6 - - - Bb5 - - -",
            "A5 - - - G5 - F5 - C5 - - - . . . .",
        ],
    ),
    (
        "caatara",
        66.0,
        0.3,
        "flute",
        0.7,
        2600.0,
        7.0,
        "Dm7 Ebmaj7 Dm7 Cmaj7",
        ["x.....x.........", "x.......x.....x.", "x..x....x..x....", "....o.......o...", "x.o.x.o.x.o.x.o."],
        [
            "A5 - - Bb5 A5 - - - F5 - G5 - A5 - - -",
            "G5 - - - Bb5 - A5 - G5 - F5 - Eb5 - - -",
            "F5 - - A5 - - G5 - F5 - E5 - D5 - - -",
            "E5 - - G5 - - B5 - A5 - G5 - E5 - - -",
        ],
    ),
    (
        "panthara",
        62.0,
        0.36,
        "rhodes",
        0.8,
        2300.0,
        15.0,
        "Dbmaj9 Bbm9 Gbmaj9 Ab13",
        ["x.....x...x...x.", "x...x...x.....x.", "x.....x.x.......", "....x.......x...", "x.x.x.x.x.x.x.x."],
        [
            "F5 . Ab5 . . F5 . . Eb5 . Db5 . . . Eb5 .",
            "Db5 . . F5 . . Bb5 . Ab5 . F5 . . . Db5 .",
            "Bb5 . . Ab5 . . Db6 . Bb5 . . . Gb5 . . .",
            "Eb5 . . Gb5 . . Bb5 . . Ab5 . . Gb5 . . .",
        ],
    ),
    (
        "pamphora",
        68.0,
        0.28,
        "pluck",
        0.5,
        3000.0,
        6.0,
        "Am9 Fmaj9 Dm9 E7b9",
        ["x.....x...x.....", "x.......f.......", "x.......x.......", "........x.......", "o.o.o.o.o.o.o.o."],
        [
            "E5 . . A5 . . B5 . C6 . . B5 A5 . . .",
            "A5 . . C6 . . A5 . F5 . . A5 C6 . . .",
            "F5 . . A5 . . D5 . F5 . . E5 . . D5 .",
            "E5 . . G#5 . . B5 . . . A5 . . . E5 .",
        ],
    ),
];

fn pitch_class(s: &str) -> Option<i32> {
    let mut c = s.chars();
    let base = match c.next()? {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let acc = match c.next() {
        Some('#') => 1,
        Some('b') => -1,
        None => 0,
        _ => return None,
    };
    Some((base + acc + 12) % 12)
}

/// Acorde "Ebmaj9": baixo (C2 a B2) e as notas do piano (de 57 a 76), sem a raiz.
fn parse_chord(s: &str) -> Option<(i32, Vec<i32>)> {
    let split = if s.get(1..2).is_some_and(|x| x == "#" || x == "b") { 2 } else { 1 };
    let r = pitch_class(&s[..split])?;
    let q: &[i32] = match &s[split..] {
        "maj9" => &[0, 4, 7, 11, 14],
        "maj7" => &[0, 4, 7, 11],
        "maj7#11" => &[0, 4, 7, 11, 18],
        "m9" => &[0, 3, 7, 10, 14],
        "m7" => &[0, 3, 7, 10],
        "m11" => &[0, 3, 7, 10, 17],
        "13" => &[0, 4, 10, 14, 21],
        "9" => &[0, 4, 7, 10, 14],
        "7b9" => &[0, 4, 7, 10, 13],
        "7" => &[0, 4, 7, 10],
        "6" => &[0, 4, 7, 9],
        _ => return None,
    };
    let mut notes: Vec<i32> = q[1..]
        .iter()
        .map(|i| {
            let mut n = 48 + r + i;
            while n < 57 {
                n += 12;
            }
            while n > 76 {
                n -= 12;
            }
            n
        })
        .collect();
    notes.sort_unstable();
    notes.dedup();
    Some((36 + r, notes))
}

/// Um compasso de melodia: 16 posições, cada uma nada ou (nota MIDI, duração em posições).
fn parse_bar(s: &str) -> Vec<Option<(i32, i32)>> {
    let mut out = vec![None; 16];
    let mut last: Option<usize> = None;
    for (i, tk) in s.split_whitespace().take(16).enumerate() {
        match tk {
            "." => last = None,
            "-" => {
                if let Some(l) = last
                    && let Some((_, len)) = out[l].as_mut()
                {
                    *len += 1;
                }
            }
            _ => {
                let (name, oct) = tk.split_at(tk.len() - 1);
                if let (Some(pc), Ok(o)) = (pitch_class(name), oct.parse::<i32>()) {
                    out[i] = Some((12 * (o + 1) + pc, 1));
                    last = Some(i);
                }
            }
        }
    }
    out
}

fn track(id: &str) -> Option<Track> {
    let (_, bpm, sw, lead, pad, warm, wob, ch, rows, mel) = TRACKS.iter().find(|t| t.0 == id)?;
    Some(Track {
        bpm: *bpm,
        sw: *sw,
        lead,
        pad: *pad,
        warm: *warm,
        wob: *wob,
        chords: ch.split_whitespace().filter_map(parse_chord).collect(),
        mel: mel.iter().map(|b| parse_bar(b)).collect(),
        comp: rows[0],
        bass: rows[1],
        k: rows[2],
        s: rows[3],
        h: rows[4],
    })
}

fn nf(n: f32) -> f32 {
    440.0 * 2f32.powf((n - 69.0) / 12.0)
}

#[derive(Debug, Default)]
struct Music {
    k: String,
    p: Option<Track>,
    mg: Option<Id>,
    lp: Option<Id>,
    step: u64,
    next: f64,
}

/// Oscilador da música (`mo`): ataque, queda exponencial e desafinação em cents.
#[allow(clippy::too_many_arguments)]
fn mo(e: &mut Engine, mg: Id, w: Wave, f: f32, t: f64, d: f64, v: f32, f2: f32, at: f64, dc: f32) {
    let end = t + d + 0.02;
    let o = e.osc(w, f, t, end);
    if let Some(p) = e.param(o, "freq") {
        p.set(t, f);
        if f2 > 0.0 {
            p.exp(t + d, f2);
        }
    }
    if dc != 0.0
        && let Some(p) = e.param(o, "detune")
    {
        p.fixed(dc);
    }
    let g = e.gain(0.0, end + 0.05);
    if let Some(p) = e.param(g, "gain") {
        p.set(t, 0.0);
        p.lin(t + if at > 0.0 { at } else { 0.01 }, v);
        p.exp(t + d, 0.001);
    }
    e.connect(o, g);
    e.connect(g, mg);
}

/// Ruído da música (`mn`): baterias e sopros.
#[allow(clippy::too_many_arguments)]
fn mn(e: &mut Engine, mg: Id, t: f64, d: f64, ty: Filter, f: f32, v: f32, q: f32) {
    let off = e.rng.f() * 2.5;
    let noise = e.noise.clone();
    let s = e.noise_src(noise, t, off, Some(d + 0.02));
    let fl = e.biquad(ty, f, if q > 0.0 { q } else { 1.0 }, t + d + 0.08);
    let g = e.gain(v, t + d + 0.08);
    if let Some(p) = e.param(g, "gain") {
        p.set(t, v);
        p.exp(t + d, 0.001);
    }
    e.connect(s, fl);
    e.connect(fl, g);
    e.connect(g, mg);
}

/// Melodia (`mlead`): cada timbre é uma pequena receita de parciais, queda e vibrato.
#[allow(clippy::too_many_arguments)]
fn mlead(e: &mut Engine, mg: Id, lead: &str, m: i32, t: f64, d: f64, v: f32, dc: f32) {
    use Wave::{Sine, Triangle as Tri};
    let f = nf(m as f32);
    match lead {
        "rhodes" => {
            mo(e, mg, Sine, f, t, d * 1.6 + 0.5, v, 0.0, 0.012, dc);
            mo(e, mg, Sine, f * 2.0, t, 0.5, v * 0.3, 0.0, 0.01, dc);
            mo(e, mg, Sine, f * 7.0, t, 0.07, v * 0.08, 0.0, 0.004, dc);
            mo(e, mg, Sine, f, t, d * 1.2 + 0.4, v * 0.5, 0.0, 0.014, dc + 7.0);
        }
        "bell" => {
            mo(e, mg, Sine, f, t, d * 1.5 + 0.9, v, 0.0, 0.006, dc);
            mo(e, mg, Sine, f * 2.76, t, 0.6, v * 0.22, 0.0, 0.004, dc);
            mo(e, mg, Sine, f * 5.4, t, 0.25, v * 0.08, 0.0, 0.003, dc);
        }
        "pluck" => {
            mo(e, mg, Tri, f, t, d * 0.9 + 0.25, v, 0.0, 0.005, dc);
            mo(e, mg, Sine, f * 2.0, t, 0.18, v * 0.4, 0.0, 0.004, dc);
        }
        "kalimba" => {
            mo(e, mg, Sine, f, t, 0.9, v, 0.0, 0.004, dc);
            mo(e, mg, Sine, f * 4.1, t, 0.1, v * 0.3, 0.0, 0.002, dc);
            mn(e, mg, t, 0.02, Filter::Bandpass, f * 3.0, v * 0.2, 2.0);
        }
        "marimba" => {
            mo(e, mg, Sine, f, t, 0.32, v * 1.1, 0.0, 0.003, dc);
            mo(e, mg, Sine, f * 4.0, t, 0.1, v * 0.35, 0.0, 0.002, dc);
            mn(e, mg, t, 0.015, Filter::Bandpass, f * 2.0, v * 0.15, 3.0);
        }
        "koto" => {
            mo(e, mg, Tri, f * 1.012, t, 0.55 + d * 0.2, v, f, 0.003, dc);
            mo(e, mg, Sine, f * 3.0, t, 0.12, v * 0.3, 0.0, 0.002, dc);
            mn(e, mg, t, 0.012, Filter::Highpass, 4000.0, v * 0.12, 0.0);
        }
        _ => {
            // flauta: seno com vibrato, sustentado, e um sopro por cima
            let end = t + d + 0.3;
            let o = e.osc(Sine, f, t, end);
            if dc != 0.0
                && let Some(p) = e.param(o, "detune")
            {
                p.fixed(dc);
            }
            let lf = 4.6 + e.rng.f() * 0.8;
            let l = e.osc(Sine, lf, t, end);
            let lg = e.gain(f * 0.006, end + 0.05);
            e.connect(l, lg);
            e.modulate(lg, o, "freq");
            let g = e.gain(0.0, end + 0.05);
            if let Some(p) = e.param(g, "gain") {
                p.set(t, 0.0);
                p.lin(t + 0.07, v * 0.8);
                p.set(t + (d - 0.1).max(0.08), v * 0.8);
                p.exp(t + d + 0.25, 0.001);
            }
            e.connect(o, g);
            e.connect(g, mg);
            mn(e, mg, t, d, Filter::Bandpass, 2600.0, v * 0.07, 1.2);
        }
    }
}

impl Music {
    /// Agenda as notas dos próximos 0,3 s (`musTick`).
    fn tick(&mut self, e: &mut Engine) {
        let (Some(p), Some(mg)) = (self.p.clone(), self.mg) else { return };
        let spb = 60.0 / p.bpm / 4.0;
        let nb = p.chords.len().max(1) as u64;
        let at = |s: &str, i: usize| s.as_bytes().get(i).copied().unwrap_or(b'.');
        while self.next < e.t + 0.3 {
            let s = self.step;
            let bar = ((s >> 4) % nb) as usize;
            let st = (s & 15) as usize;
            let pass = (s / (16 * nb)) % 3;
            let (bass_root, notes) = p.chords[bar].clone();
            let jitter = f64::from(e.rng.f()) * 0.008;
            let t = self.next + if st & 1 == 1 { spb * p.sw } else { 0.0 } + jitter;
            let wob = p.wob * (self.next * 2.1).sin() as f32;
            let mut vel = || 0.85 + e.rng.f() * 0.3;
            let _ = &mut vel;
            if st == 0 {
                let pv = 0.03 * p.pad;
                for n in &notes {
                    mo(e, mg, Wave::Triangle, nf((n - 12) as f32), t, spb * 16.0, pv, 0.0, 1.2, -5.0);
                    mo(e, mg, Wave::Triangle, nf((n - 12) as f32), t, spb * 16.0, pv, 0.0, 1.4, 6.0);
                }
            }
            if at(p.comp, st) == b'x' {
                for (i, n) in notes.iter().enumerate() {
                    let tt = t + i as f64 * 0.011 + f64::from(e.rng.f()) * 0.006;
                    let v = 0.05 * (0.85 + e.rng.f() * 0.3);
                    let f = nf(*n as f32);
                    mo(e, mg, Wave::Sine, f, tt, spb * 5.0, v, 0.0, 0.012, wob);
                    mo(e, mg, Wave::Sine, f * 2.0, tt, spb * 1.4, v * 0.28, 0.0, 0.01, wob);
                    mo(e, mg, Wave::Sine, f * 7.0, tt, 0.06, v * 0.06, 0.0, 0.004, wob);
                }
            }
            let b = at(p.bass, st);
            if b != b'.' {
                let n = bass_root
                    + if b == b'f' {
                        7
                    } else if b == b'o' {
                        12
                    } else {
                        0
                    };
                let (v1, v2) = (0.46 * (0.85 + e.rng.f() * 0.3), 0.14 * (0.85 + e.rng.f() * 0.3));
                mo(e, mg, Wave::Sine, nf(n as f32), t, spb * 3.2, v1, 0.0, 0.012, wob);
                mo(e, mg, Wave::Triangle, nf(n as f32), t, spb * 2.0, v2, 0.0, 0.012, wob);
            }
            let kd = at(p.k, st);
            if kd != b'.' {
                let v = if kd == b'x' { 0.72 } else { 0.38 } * (0.85 + e.rng.f() * 0.3);
                mo(e, mg, Wave::Sine, 140.0, t, 0.2, v, 42.0, 0.004, 0.0);
            }
            let sn = at(p.s, st);
            if sn != b'.' {
                let x = sn == b'x';
                let (v1, v2) = (
                    (if x { 0.42 } else { 0.2 }) * (0.85 + e.rng.f() * 0.3),
                    (if x { 0.16 } else { 0.08 }) * (0.85 + e.rng.f() * 0.3),
                );
                mn(e, mg, t, 0.14, Filter::Bandpass, 1900.0, v1, 0.9);
                mo(e, mg, Wave::Triangle, 190.0, t, 0.09, v2, 130.0, 0.004, 0.0);
            }
            let hh = at(p.h, st);
            if hh != b'.' {
                let v = if hh == b'x' { 0.2 } else { 0.09 } * (0.85 + e.rng.f() * 0.3);
                mn(e, mg, t, 0.045, Filter::Highpass, 7200.0, v, 0.0);
            }
            // a segunda volta é mais enxuta; a terceira só tem o ritmo e o piano
            if pass < 2
                && let Some(Some((m, len))) = p.mel.get(bar).and_then(|b| b.get(st)).copied()
                && !(pass == 1 && (s * 7 + st as u64) % 3 == 0)
            {
                let v = 0.1 * (0.85 + e.rng.f() * 0.3);
                let d = f64::from(len.min(12)) * spb;
                let tt = t + f64::from(e.rng.f()) * 0.012;
                mlead(e, mg, p.lead, m, tt, d, v, wob);
            }
            self.next += spb;
            self.step += 1;
        }
    }
}

// ---------- ambiente (bichos de cada mundo) ----------

#[derive(Debug, Clone, Copy)]
struct Bird {
    f: f32,
    ty: u32,
    d: f32,
    gap: f32,
    n: u32,
}

#[derive(Debug, Default)]
struct Ambient {
    k: String,
    g: Option<Id>,
    rv: Option<Id>,
    /// Próximo chamado de cada bicho (ms do relógio de áudio).
    at: Vec<(&'static str, f64)>,
    frog: Vec<f32>,
    bird: Vec<Bird>,
}

fn animals(world: &str) -> &'static [&'static str] {
    match world {
        "amazora" => &["croak", "frogs", "tap", "birds", "arara"],
        "cerron" => &["tap", "hoot", "birds", "seriema"],
        "atlanthia" => &["frogs", "hoot", "tap", "birds", "sabia"],
        "caatara" => &["goat", "birds", "rolinha", "bemtevi"],
        "panthara" => &["croak", "bubbles", "frogs", "birds", "bemtevi"],
        "pamphora" => &["moo", "hoot", "birds", "queroquero"],
        _ => &[],
    }
}

/// Intervalo entre chamados (ms) e volume de cada bicho (`AMB_EVERY`, `AMB_LV`).
fn every(k: &str) -> (f64, f64, f32) {
    match k {
        "bubbles" => (600.0, 2200.0, 1.3),
        "croak" => (2000.0, 7000.0, 1.9),
        "frogs" => (1200.0, 4500.0, 1.9),
        "hoot" => (9000.0, 20000.0, 0.85),
        "tap" => (7000.0, 18000.0, 1.0),
        "goat" => (8000.0, 20000.0, 1.0),
        "birds" => (2500.0, 9000.0, 1.0),
        "moo" => (12000.0, 26000.0, 1.0),
        "arara" => (16000.0, 34000.0, 1.0),
        "seriema" => (25000.0, 50000.0, 1.0),
        "sabia" => (12000.0, 26000.0, 0.78),
        "rolinha" => (20000.0, 40000.0, 0.66),
        "bemtevi" => (14000.0, 30000.0, 0.5),
        _ => (15000.0, 32000.0, 1.0),
    }
}

/// Opções do assobio de ave (`amTone`).
#[derive(Debug, Clone, Copy, Default)]
struct AmT {
    k: f32,
    h2: f32,
    mid: f32,
    vib: f32,
    vd: f32,
    tri: bool,
}

impl Ctx<'_> {
    /// Envelope liso (`amEnv`): sobe em seno² até k e desce em cosseno².
    fn am_env(&mut self, g: Id, t: f64, d: f64, v: f32, k: f32) {
        let sv = self.sv;
        let c: Vec<f32> = (0..32)
            .map(|i| {
                let x = i as f32 / 31.0;
                v * sv
                    * if x < k {
                        (x / k * std::f32::consts::FRAC_PI_2).sin().powi(2)
                    } else {
                        ((x - k) / (1.0 - k) * std::f32::consts::FRAC_PI_2).cos().powi(2)
                    }
            })
            .collect();
        if let Some(p) = self.e.param(g, "gain") {
            p.fixed(0.0);
            p.curve(&c, t, d);
        }
    }

    /// Assobio de ave (`amTone`).
    fn am_tone(&mut self, f0: f32, f1: f32, t: f64, d: f64, v: f32, o: AmT) {
        let parts: &[(f32, f32)] = if o.h2 > 0.0 { &[(1.0, 1.0), (2.0, o.h2)] } else { &[(1.0, 1.0)] };
        for &(m, lv) in parts {
            let end = t + d + 0.02;
            let os = self.e.osc(if o.tri { Wave::Triangle } else { Wave::Sine }, f0 * m, t, end);
            if let Some(p) = self.e.param(os, "freq") {
                p.set(t, f0 * m);
                if o.mid > 0.0 {
                    p.exp(t + d * 0.4, o.mid * m);
                    p.exp(t + d, f1 * m);
                } else if (f1 - f0).abs() > f32::EPSILON {
                    p.exp(t + d, f1 * m);
                }
            }
            if o.vib > 0.0 {
                let l = self.e.osc(Wave::Sine, o.vib, t, end);
                let lg = self.e.gain(f0 * m * if o.vd > 0.0 { o.vd } else { 0.02 }, end + 0.05);
                self.e.connect(l, lg);
                self.e.modulate(lg, os, "freq");
            }
            let g = self.e.gain(0.0, end + 0.05);
            self.am_env(g, t, d, v * lv, if o.k > 0.0 { o.k } else { 0.25 });
            self.e.connect(os, g);
            self.e.connect(g, self.sb);
        }
    }

    /// Voz de bicho (`amVox`): serra pelos formantes, tremido e sopro.
    #[allow(clippy::too_many_arguments)]
    fn am_vox(
        &mut self,
        fs: &[f32],
        t: f64,
        d: f64,
        v: f32,
        fm: &[(f32, f32, f32, f32)],
        am: f32,
        ad: f32,
        br: f32,
        bv: f32,
        k: f32,
    ) {
        let end = t + d + 0.02;
        let os = self.e.osc(Wave::Saw, fs[0], t, end);
        if let Some(p) = self.e.param(os, "freq") {
            p.set(t, fs[0]);
            for (i, f) in fs.iter().enumerate().skip(1) {
                p.exp(t + d * i as f64 / (fs.len() - 1) as f64, *f);
            }
        }
        let sum = self.e.gain(1.0, end + 0.05);
        for &(f, q, lv, f2) in fm {
            let b = self.e.biquad(Filter::Bandpass, f, q, end + 0.05);
            if f2 > 0.0
                && let Some(p) = self.e.param(b, "freq")
            {
                p.set(t, f);
                p.lin(t + d * 0.45, f2);
                p.lin(t + d, f);
            }
            let bg = self.e.gain(lv, end + 0.05);
            self.e.connect(os, b);
            self.e.connect(b, bg);
            self.e.connect(bg, sum);
        }
        let g = self.e.gain(0.0, end + 0.05);
        if am > 0.0 {
            let l = self.e.osc(Wave::Sine, am, t, end);
            let lg = self.e.gain(ad, end + 0.05);
            let m = self.e.gain(1.0 - ad, end + 0.05);
            self.e.connect(l, lg);
            self.e.modulate(lg, m, "gain");
            self.e.connect(sum, m);
            self.e.connect(m, g);
        } else {
            self.e.connect(sum, g);
        }
        self.am_env(g, t, d, v, if k > 0.0 { k } else { 0.2 });
        self.e.connect(g, self.sb);
        if br > 0.0 {
            self.burst(t, d, br, v * bv, 1.5, B { at: Some(d as f32 * 0.3), ..B::default() });
        }
    }
}

impl Ambient {
    /// Ponto de onde o chamado sai (`amSpot`): longe é mais baixo, abafado e com mais eco; o lado vai no estéreo.
    fn spot(&self, e: &mut Engine, d: f32) -> Option<Id> {
        let (ag, rv) = (self.g?, self.rv?);
        let exp = e.t + 6.0;
        let g = e.gain(1.0 - 0.5 * d, exp);
        let lp = e.biquad(Filter::Lowpass, 13000.0 - 9500.0 * d, 0.5, exp);
        let pan = (e.rng.f() * 2.0 - 1.0) * 0.75;
        let p = e.pan(pan, exp);
        let s = e.gain(0.1 + 0.35 * d, exp);
        e.connect(g, lp);
        e.connect(lp, p);
        e.connect(p, ag);
        e.connect(lp, s);
        e.connect(s, rv);
        Some(g)
    }

    fn call(&self, c: &mut Ctx<'_>, k: &str) {
        let t = c.e.t + 0.02;
        let none = AmT::default();
        match k {
            "bubbles" => {
                let n = 1 + (c.r() * 3.0) as u32;
                let mut s = t;
                for _ in 0..n {
                    let f = 300.0 + c.r() * 800.0;
                    let d = f64::from(0.04 + c.r() * 0.06);
                    c.tone(Wave::Sine, f, f * 1.7, s, d, 0.05, 0.008);
                    s += f64::from(0.04 + c.r() * 0.12);
                }
            }
            "croak" => {
                let f = 95.0 + c.r() * 55.0;
                for i in (0..2 + (c.r() * 2.0) as u32).rev() {
                    let s = t + f64::from(i as f32 * (0.38 + c.r() * 0.1));
                    let am = 26.0 + c.r() * 14.0;
                    c.am_vox(
                        &[f * 1.06, f * 0.9],
                        s,
                        0.26,
                        0.11,
                        &[(350.0, 4.0, 1.0, 0.0), (900.0, 5.0, 0.5, 0.0), (1700.0, 6.0, 0.2, 0.0)],
                        am,
                        0.85,
                        0.0,
                        0.0,
                        0.0,
                    );
                }
            }
            "frogs" => {
                let f = self.frog.get((c.r() * self.frog.len() as f32) as usize).copied().unwrap_or(1000.0);
                for i in (0..2 + (c.r() * 4.0) as u32).rev() {
                    let s = t + f64::from(i as f32 * (0.1 + c.r() * 0.03));
                    let np = 4 + (c.r() * 4.0) as u32;
                    let pi = 0.008 + c.r() * 0.004;
                    for p in 0..np {
                        let ff = f * (1.0 + c.r() * 0.03);
                        c.burst(
                            s + f64::from(p as f32 * pi),
                            0.007,
                            ff,
                            0.6 * (1.0 - p as f32 / np as f32 * 0.4),
                            8.0,
                            B::default(),
                        );
                    }
                }
            }
            "hoot" => {
                let f = 310.0 + c.r() * 40.0;
                c.am_tone(f, f * 0.9, t, 0.5, 0.033, AmT { k: 0.3, h2: 0.08, ..none });
                c.burst(t, 0.5, f, 0.012, 2.0, B { at: Some(0.15), ..B::default() });
                c.am_tone(f * 0.88, f * 0.78, t + 0.62, 0.66, 0.029, AmT { k: 0.3, h2: 0.08, ..none });
                c.burst(t + 0.62, 0.66, f * 0.88, 0.01, 2.0, B { at: Some(0.18), ..B::default() });
            }
            "tap" => {
                let fw = 650.0 + c.r() * 350.0;
                let n = 12 + (c.r() * 9.0) as u32;
                let mut s = t;
                for i in 0..n {
                    let v = 1.0 - 0.5 * i as f32 / n as f32;
                    c.burst(s, 0.006, 3000.0, 0.25 * v, 0.7, B { ty: Some(Filter::Highpass), ..B::default() });
                    c.burst(s, 0.05, fw, 0.8 * v, 12.0, B::default());
                    c.burst(s, 0.035, fw * 2.3, 0.28 * v, 10.0, B::default());
                    s += f64::from(0.058 - 0.012 * i as f32 / n as f32);
                }
            }
            "goat" => {
                let f = 300.0 + c.r() * 80.0;
                let g = |c: &mut Ctx<'_>, s: f64, k: f32| {
                    let d = f64::from(0.6 + c.r() * 0.25);
                    let am = 6.0 + c.r() * 3.0;
                    c.am_vox(
                        &[f * k, f * k * 1.04, f * k * 0.9],
                        s,
                        d,
                        0.055,
                        &[(650.0, 4.0, 1.0, 0.0), (1800.0, 6.0, 0.55, 0.0), (2700.0, 7.0, 0.25, 0.0)],
                        am,
                        0.5,
                        1500.0,
                        0.08,
                        0.0,
                    );
                };
                g(c, t, 1.0);
                if c.r() < 0.6 {
                    g(c, t + 0.95, 1.05);
                }
            }
            "birds" => {
                let Some(b) = self.bird.get((c.r() * self.bird.len() as f32) as usize).copied() else { return };
                let mut s = t;
                for _ in 0..b.n + (c.r() * 3.0) as u32 {
                    let f = b.f * (1.0 + (c.r() - 0.5) * 0.06);
                    let d = f64::from(b.d * (0.8 + c.r() * 0.4));
                    match b.ty {
                        0 => c.am_tone(f * 0.92, f * 0.96, s, d, 0.07, AmT { mid: f * 1.08, ..none }),
                        1 => c.am_tone(f * 1.12, f * 0.85, s, d, 0.07, AmT { k: 0.15, ..none }),
                        2 => {
                            c.am_tone(f, f, s, d * 0.5, 0.07, none);
                            c.am_tone(f * 0.68, f * 0.62, s + d * 0.5, d * 0.6, 0.07, none);
                        }
                        _ => {
                            let n = 6 + (c.r() * 6.0) as u32;
                            for j in 0..n {
                                let k = j as f32 / n as f32;
                                c.am_tone(
                                    f * (1.04 - 0.08 * k),
                                    f * (0.98 - 0.08 * k),
                                    s + f64::from(j) * 0.034,
                                    0.022,
                                    0.06,
                                    AmT { k: 0.2, ..none },
                                );
                            }
                            s += f64::from(n) * 0.034;
                        }
                    }
                    s += d + f64::from(b.gap * (0.7 + c.r() * 0.6));
                }
            }
            "moo" => {
                let f = 100.0 + c.r() * 25.0;
                let d = f64::from(1.4 + c.r() * 0.5);
                c.am_vox(
                    &[f * 0.92, f * 1.08, f * 1.02, f * 0.9],
                    t,
                    d,
                    0.06,
                    &[(260.0, 3.0, 1.0, 700.0), (800.0, 5.0, 0.4, 1150.0), (2300.0, 6.0, 0.1, 0.0)],
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.25,
                );
            }
            "arara" => {
                let one = |c: &mut Ctx<'_>, s: f64, f: f32| {
                    for i in (0..2 + (c.r() * 3.0) as u32).rev() {
                        let ss = s + f64::from(i as f32 * (0.45 + c.r() * 0.3));
                        let d = f64::from(0.3 + c.r() * 0.12);
                        let am = 65.0 + c.r() * 20.0;
                        c.am_vox(
                            &[f * 0.85, f * 1.1, f * 0.8],
                            ss,
                            d,
                            0.06,
                            &[(1300.0, 3.0, 1.0, 0.0), (2400.0, 4.0, 0.6, 0.0), (3600.0, 5.0, 0.3, 0.0)],
                            am,
                            0.4,
                            2000.0,
                            0.2,
                            0.0,
                        );
                    }
                };
                let f = 700.0 + c.r() * 200.0;
                one(c, t, f);
                if c.r() < 0.4 {
                    let d = 0.4 + c.r() * 0.6;
                    if let Some(sb) = self.spot(c.e, d) {
                        c.sb = sb;
                    }
                    let (dt, f2) = (1.4 + c.r(), 650.0 + c.r() * 250.0);
                    one(c, t + f64::from(dt), f2);
                }
            }
            "seriema" => {
                let n = 8 + (c.r() * 7.0) as u32;
                let f = 900.0 + c.r() * 150.0;
                let mut s = t;
                for i in 0..n {
                    let fi = f * (1.0 - 0.35 * i as f32 / n as f32);
                    c.am_vox(
                        &[fi * 1.1, fi * 0.92],
                        s,
                        0.09,
                        0.055,
                        &[(fi * 1.6, 4.0, 1.0, 0.0), (fi * 3.0, 5.0, 0.45, 0.0)],
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.15,
                    );
                    s += f64::from(0.15 - 0.05 * i as f32 / n as f32);
                }
            }
            "sabia" => {
                let mut s = t;
                for _ in 0..4 + (c.r() * 5.0) as u32 {
                    let f = 1600.0 + c.r() * 1400.0;
                    let d = 0.08 + c.r() * 0.14;
                    let up = c.r() < 0.5;
                    let vib = 6.0 + c.r() * 3.0;
                    c.am_tone(
                        f,
                        f * if up { 1.12 } else { 0.88 },
                        s,
                        f64::from(d),
                        0.06,
                        AmT { vib, vd: 0.012, h2: 0.1, k: 0.3, ..none },
                    );
                    s += f64::from(d + 0.05 + c.r() * 0.12);
                }
            }
            "rolinha" => {
                let f = 500.0 + c.r() * 100.0;
                let mut s = t;
                for _ in 0..2 + (c.r() * 2.0) as u32 {
                    c.am_tone(f, f * 0.97, s, 0.2, 0.06, AmT { k: 0.35, h2: 0.12, ..none });
                    c.am_tone(f * 0.93, f * 0.9, s + 0.3, 0.17, 0.055, AmT { k: 0.35, h2: 0.12, ..none });
                    c.am_tone(
                        f * 1.02,
                        f * 0.86,
                        s + 0.58,
                        0.42,
                        0.06,
                        AmT { k: 0.3, mid: f * 1.05, h2: 0.12, ..none },
                    );
                    s += f64::from(1.5 + c.r() * 0.4);
                }
            }
            "bemtevi" => {
                let f = 1900.0 + c.r() * 300.0;
                let mut s = t;
                for _ in 0..1 + u32::from(c.r() < 0.4) {
                    c.am_tone(f * 0.95, f * 1.05, s, 0.11, 0.07, AmT { h2: 0.3, ..none });
                    c.am_tone(f * 1.25, f * 1.2, s + 0.16, 0.06, 0.065, AmT { h2: 0.3, ..none });
                    c.am_tone(f * 1.3, f * 0.82, s + 0.3, 0.28, 0.075, AmT { mid: f * 1.45, h2: 0.35, k: 0.3, ..none });
                    s += f64::from(1.3 + c.r() * 0.4);
                }
            }
            "queroquero" => {
                let f = 2300.0 + c.r() * 400.0;
                let mut s = t;
                for _ in 0..3 + (c.r() * 4.0) as u32 {
                    c.am_tone(f, f * 0.92, s, 0.09, 0.05, AmT { tri: true, h2: 0.4, k: 0.15, ..none });
                    c.burst(s, 0.03, f * 1.5, 0.05, 2.0, B::default());
                    c.am_tone(f * 0.84, f * 0.8, s + 0.12, 0.07, 0.045, AmT { tri: true, h2: 0.4, k: 0.15, ..none });
                    s += f64::from(0.3 + c.r() * 0.08);
                }
            }
            _ => {}
        }
    }
}

// ---------- chuva ----------

#[derive(Debug, Default)]
#[allow(dead_code)]
struct Rain {
    g: Option<Id>,
    parts: Vec<Id>,
}

// ---------- integração com o Bevy ----------

/// Fonte de áudio do Bevy que toca o motor (registrada com `add_audio_source`).
#[derive(Asset, TypePath, Clone)]
pub struct SynthSource(Arc<Mutex<Engine>>);

impl std::fmt::Debug for SynthSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SynthSource")
    }
}

impl Source for Stream {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> NonZero<u16> {
        NonZero::new(2).unwrap_or(NonZero::<u16>::MIN)
    }
    fn sample_rate(&self) -> NonZero<u32> {
        NonZero::new(SR).unwrap_or(NonZero::<u32>::MIN)
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

impl Decodable for SynthSource {
    type Decoder = Stream;
    fn decoder(&self) -> Stream {
        Stream::new(self.0.clone())
    }
}

/// O áudio do jogo: o motor e os canais.
#[derive(Resource)]
#[allow(dead_code)]
pub struct Sound {
    e: Arc<Mutex<Engine>>,
    sfx_g: Id,
    bg_g: Id,
    mus_g: Id,
    music: Music,
    amb: Ambient,
    rain: Rain,
    rain_w: f32,
}

impl std::fmt::Debug for Sound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sound").field("music", &self.music.k).field("amb", &self.amb.k).finish_non_exhaustive()
    }
}

impl Sound {
    fn new() -> Self {
        let mut e = Engine::new(SR as f32);
        let out = e.dest();
        // reverb compartilhada: só os efeitos (0,22) e a música (0,16) vão para ela; o ambiente fica seco
        let rv = e.reverb();
        e.connect(rv, out);
        let sfx_g = e.gain(0.8, f64::INFINITY);
        let bg_g = e.gain(0.7, f64::INFINITY);
        let mus_g = e.gain(0.6, f64::INFINITY);
        for g in [sfx_g, bg_g, mus_g] {
            e.connect(g, out);
        }
        let s = e.gain(0.22, f64::INFINITY);
        e.connect(sfx_g, s);
        e.connect(s, rv);
        let ms = e.gain(0.16, f64::INFINITY);
        e.connect(mus_g, ms);
        e.connect(ms, rv);
        Self {
            e: Arc::new(Mutex::new(e)),
            sfx_g,
            bg_g,
            mus_g,
            music: Music::default(),
            amb: Ambient::default(),
            rain: Rain::default(),
            rain_w: 0.0,
        }
    }

    /// Toca um efeito (`sfx(k, vol)`).
    pub fn play(&self, k: &str, vol: f32) {
        if vol <= 0.0 {
            return;
        }
        let Ok(mut e) = self.e.lock() else { return };
        let mut c = Ctx { e: &mut e, sb: self.sfx_g, sv: vol * trim(k) };
        c.sfx(k);
    }

    /// Som de um remédio (`medSound`): preparar ou cuidar.
    pub fn med(&self, note: f32, kind: &str, craft: bool, vol: f32) {
        let Ok(mut e) = self.e.lock() else { return };
        let g = if craft {
            2.5
        } else {
            match kind {
                "drink" => 2.0,
                "balm" => 2.7,
                "injection" => 2.85,
                "tablet" | "mist" => 2.3,
                _ => 1.65,
            }
        };
        let mut c = Ctx { e: &mut e, sb: self.sfx_g, sv: vol.min(1.0) * g };
        c.med(note, kind, craft);
    }

    /// Volumes dos três canais (Ajustes).
    pub fn volumes(&self, sfx: f32, amb: f32, mus: f32) {
        let Ok(mut e) = self.e.lock() else { return };
        for (g, v) in [(self.sfx_g, sfx), (self.bg_g, amb), (self.mus_g, mus)] {
            if let Some(p) = e.param(g, "gain") {
                p.fixed(v.clamp(0.0, 1.0));
            }
        }
    }

    /// Faixa que deve tocar (`musSync`): vazio para silêncio.
    pub fn music(&mut self, want: &str) {
        if want == self.music.k {
            return;
        }
        let Ok(mut e) = self.e.lock() else { return };
        // a faixa anterior some em fade
        if let (Some(mg), Some(lp)) = (self.music.mg.take(), self.music.lp.take()) {
            let t = e.t;
            if let Some(p) = e.param(mg, "gain") {
                p.target(t, 0.0, 0.25);
            }
            e.set_expire(mg, t + 1.8);
            e.set_expire(lp, t + 1.8);
        }
        self.music = Music::default();
        self.music.k = want.to_owned();
        let Some(p) = (if want.is_empty() { None } else { track(want).or_else(|| track("menu")) }) else { return };
        let t = e.t;
        let mg = e.gain(0.0, f64::INFINITY);
        if let Some(g) = e.param(mg, "gain") {
            g.target(t, 0.22 * trim(&format!("mus_{want}")), 0.9);
        }
        let lp = e.biquad(Filter::Lowpass, p.warm, 0.6, f64::INFINITY);
        e.connect(mg, lp);
        e.connect(lp, self.mus_g);
        self.music.mg = Some(mg);
        self.music.lp = Some(lp);
        self.music.next = t + 0.1;
        self.music.p = Some(p);
    }

    /// Ambiente do mundo (`ambSet`/`ambStop`): vazio para parar.
    pub fn ambient(&mut self, world: &str) {
        if world == self.amb.k {
            return;
        }
        let Ok(mut e) = self.e.lock() else { return };
        let t = e.t;
        if let Some(g) = self.amb.g.take() {
            if let Some(p) = e.param(g, "gain") {
                p.target(t, 0.0, 0.4);
            }
            e.set_expire(g, t + 2.2);
            if let Some(rv) = self.amb.rv.take() {
                e.set_expire(rv, t + 2.2);
            }
        }
        self.amb = Ambient { k: world.to_owned(), ..Ambient::default() };
        if world.is_empty() {
            return;
        }
        let g = e.gain(0.0, f64::INFINITY);
        if let Some(p) = e.param(g, "gain") {
            p.target(t, trim(&format!("amb_{world}")), 1.2);
        }
        e.connect(g, self.bg_g);
        let rv = e.reverb();
        e.connect(rv, g);
        self.amb.g = Some(g);
        self.amb.rv = Some(rv);
        self.amb.frog = (0..3).map(|_| 900.0 + e.rng.f() * 350.0).collect();
        self.amb.bird = (0..3)
            .map(|_| {
                let r = &mut e.rng;
                Bird {
                    f: 2200.0 + r.f() * 2200.0,
                    ty: (r.f() * 4.0) as u32,
                    d: 0.015 + r.f() * 0.05,
                    gap: 0.06 + r.f() * 0.14,
                    n: 2 + (r.f() * 4.0) as u32,
                }
            })
            .collect();
    }

    /// Chuva (`rainSnd`): intensidade de 0 a 1 (o clima chama).
    #[allow(dead_code)]
    pub fn rain(&mut self, w: f32) {
        if (w - self.rain_w).abs() < 0.01 || (w <= 0.0 && self.rain.g.is_none()) {
            return;
        }
        self.rain_w = w;
        let Ok(mut e) = self.e.lock() else { return };
        let t = e.t;
        if self.rain.g.is_none() {
            let g = e.gain(0.0, f64::INFINITY);
            e.connect(g, self.bg_g);
            let hp = e.biquad(Filter::Highpass, 160.0, 1.0, f64::INFINITY);
            e.connect(hp, g);
            let long: Arc<Vec<f32>> = Arc::new((0..(SR * 8) as usize).map(|_| e.rng.f() * 2.0 - 1.0).collect());
            let mut parts = vec![g, hp];
            for (ty, f, q, v, hz, dp) in [
                (Filter::Bandpass, 900.0, 0.5, 0.09, 0.023, 0.03),
                (Filter::Bandpass, 2600.0, 0.6, 0.05, 0.031, 0.025),
                (Filter::Highpass, 6000.0, 1.0, 0.035, 0.017, 0.018),
            ] {
                let m = e.gain(v, f64::INFINITY);
                e.connect(m, hp);
                let off = e.rng.f() * 7.9;
                let s = e.noise_src(long.clone(), t, off, None);
                let fl = e.biquad(ty, f, q, f64::INFINITY);
                e.connect(s, fl);
                e.connect(fl, m);
                // rajadas: cada faixa ondula num ritmo diferente
                let l = e.osc(Wave::Sine, hz, t, f64::INFINITY);
                let lg = e.gain(dp, f64::INFINITY);
                e.connect(l, lg);
                e.modulate(lg, m, "gain");
                parts.extend([m, s, fl, l, lg]);
            }
            // pingos perto: cliques curtos de força variada
            let n = (SR * 6) as usize;
            let mut drops = vec![0.0f32; n];
            for _ in 0..300 {
                let i = (e.rng.f() * (n - 600) as f32) as usize;
                let len = (SR as f32 * (0.002 + e.rng.f() * 0.006)) as usize;
                let v = 0.2 + 0.8 * e.rng.f() * e.rng.f() * e.rng.f();
                for j in 0..len {
                    drops[i + j] += (e.rng.f() * 2.0 - 1.0) * v * (-4.0 * j as f32 / len as f32).exp();
                }
            }
            let d = e.noise_src(Arc::new(drops), t, 0.0, None);
            let df = e.biquad(Filter::Highpass, 1800.0, 1.0, f64::INFINITY);
            let dm = e.gain(0.3, f64::INFINITY);
            e.connect(d, df);
            e.connect(df, dm);
            e.connect(dm, hp);
            parts.extend([d, df, dm]);
            self.rain = Rain { g: Some(g), parts };
        }
        if let Some(g) = self.rain.g
            && let Some(p) = e.param(g, "gain")
        {
            p.target(t, w * trim("rain"), 0.8);
        }
        if w <= 0.0 {
            for id in std::mem::take(&mut self.rain.parts) {
                e.set_expire(id, t + 5.0);
            }
            self.rain.g = None;
        }
    }

    /// A cada quadro: notas da música e chamados dos bichos.
    fn tick(&mut self) {
        let Ok(mut e) = self.e.lock() else { return };
        self.music.tick(&mut e);
        if self.amb.g.is_none() {
            return;
        }
        let now = e.t * 1000.0;
        for &k in animals(&self.amb.k) {
            let (lo, hi, lv) = every(k);
            let due = match self.amb.at.iter_mut().find(|(n, _)| *n == k) {
                Some((_, at)) => {
                    if now < *at {
                        false
                    } else {
                        *at = now + lo + f64::from(e.rng.f()) * (hi - lo);
                        true
                    }
                }
                None => {
                    let first = now + f64::from(e.rng.f()) * hi * 0.5;
                    self.amb.at.push((k, first));
                    false
                }
            };
            if due {
                let d = 0.25 + 0.75 * e.rng.f();
                let Some(sb) = self.amb.spot(&mut e, d) else { continue };
                let mut c = Ctx { e: &mut e, sb, sv: lv };
                self.amb.call(&mut c, k);
            }
        }
    }
}

#[derive(Debug)]
pub struct SoundPlugin;

impl Plugin for SoundPlugin {
    fn build(&self, app: &mut App) {
        let snd = Sound::new();
        let src = SynthSource(snd.e.clone());
        app.add_audio_source::<SynthSource>()
            .insert_resource(snd)
            .add_message::<Sfx>()
            .add_systems(Startup, move |mut commands: Commands, mut assets: ResMut<Assets<SynthSource>>| {
                commands.spawn(AudioPlayer(assets.add(src.clone())));
            })
            .add_systems(Update, (play, music, body_sounds, reactions, tick).chain());
    }
}

fn play(mut msgs: MessageReader<Sfx>, snd: Res<Sound>) {
    for Sfx(k, v) in msgs.read() {
        snd.play(k, *v);
    }
}

fn tick(mut snd: ResMut<Sound>) {
    snd.tick();
}

/// Música e ambiente de acordo com a tela e o mundo, e os volumes dos Ajustes.
fn music(
    mut snd: ResMut<Sound>,
    state: Res<State<crate::state::Screen>>,
    world: Option<Res<sky_world::CurrentWorld>>,
    settings: Res<crate::settings::Settings>,
) {
    if settings.is_changed() {
        snd.volumes(settings.sfx, settings.amb, settings.mus);
    }
    let w = world.as_ref().map(|w| w.world.id()).unwrap_or("");
    let playing = *state.get() == crate::state::Screen::Playing;
    let want = if !settings.music {
        ""
    } else if playing {
        if w.is_empty() { "amazora" } else { w }
    } else {
        "menu"
    };
    snd.music(want);
    snd.ambient(if playing { w } else { "" });
}

/// Sons do corpo de cada avatar (`sfxTick`): sentar e levantar, pular e pousar, passos no chão do mundo.
#[allow(clippy::type_complexity)]
fn body_sounds(
    mut q: Query<(Entity, &GlobalTransform, Option<&crate::player::Me>, Option<&crate::player::Remote>)>,
    me: Query<&GlobalTransform, With<crate::player::Me>>,
    world: Option<Res<sky_world::CurrentWorld>>,
    state: Res<State<crate::state::Screen>>,
    mut last: Local<std::collections::HashMap<Entity, (Vec3, bool, bool, f32)>>,
    snd: Res<Sound>,
) {
    if *state.get() != crate::state::Screen::Playing {
        last.clear();
        return;
    }
    let here = me.single().map_or(Vec3::ZERO, GlobalTransform::translation);
    let step = step_kind(world.as_ref().map(|w| w.world.id()).unwrap_or(""));
    let mut seen = Vec::new();
    for (e, gt, m, r) in &mut q {
        if m.is_none() && r.is_none() {
            continue;
        }
        seen.push(e);
        let p = gt.translation();
        let sit = m.map(|m| m.seat.is_some()).or(r.map(|r| r.sitting)).unwrap_or(false);
        let air = p.y > 0.12;
        let v = if m.is_some() { 1.0 } else { dvol(Vec2::new(p.x - here.x, p.z - here.z).length()) };
        match last.get_mut(&e) {
            Some((lp, ls, la, acc)) => {
                if *ls != sit && v > 0.03 {
                    snd.play(if sit { "sit" } else { "stand" }, v);
                }
                if *la != air && v > 0.03 {
                    snd.play(if air { "jump" } else { "land" }, v);
                }
                let d = Vec2::new(p.x - lp.x, p.z - lp.z).length();
                if d < 1.5 && !sit && !air {
                    *acc += d;
                    if *acc > 0.8 {
                        *acc = 0.0;
                        if v > 0.04 {
                            snd.play(step, v * 0.8);
                        }
                    }
                }
                (*lp, *ls, *la) = (p, sit, air);
            }
            None => {
                last.insert(e, (p, sit, air, 0.0));
            }
        }
    }
    last.retain(|e, _| seen.contains(e));
}

/// Sons que nascem de mudanças nos avatares: emoji (`showEmo`), gesto (`startGest`), item na mão (`setHeld`) e usar o
/// item (comer ou beber), com o volume da distância.
#[allow(clippy::type_complexity)]
fn reactions(
    snd: Res<Sound>,
    gcat: Res<crate::gestures::GestureCatalog>,
    me: Query<&GlobalTransform, With<crate::player::Me>>,
    emotes: Query<(&GlobalTransform, &crate::state::Emote), Added<crate::state::Emote>>,
    gests: Query<(&GlobalTransform, &crate::gestures::Gesture), Changed<crate::gestures::Gesture>>,
    held: Query<(&GlobalTransform, &crate::backpack::Held), Changed<crate::backpack::Held>>,
    using: Query<(&GlobalTransform, Option<&crate::backpack::Held>), Added<crate::backpack::Drinking>>,
) {
    let here = me.single().map_or(Vec3::ZERO, GlobalTransform::translation);
    let vol = |g: &GlobalTransform| {
        let p = g.translation();
        dvol(Vec2::new(p.x - here.x, p.z - here.z).length())
    };
    for (g, e) in &emotes {
        snd.play(emo_kind(&e.ic), vol(g));
    }
    for (g, ge) in &gests {
        if ge.e > 0.1 {
            continue;
        }
        let info = gcat.list.iter().find(|x| x.id == ge.id);
        let k = info
            .and_then(|i| i.sfx.clone())
            .unwrap_or_else(|| if info.is_some_and(|i| i.looped == 1) { "g_dance".into() } else { "g_whoosh".into() });
        snd.play(&k, vol(g));
    }
    for (g, h) in &held {
        if h.t < 0.05 && h.eat.is_none() {
            snd.play("pickup", vol(g) * 0.9);
        }
    }
    for (g, h) in &using {
        snd.play(if h.is_some_and(|h| h.item.food == 1) { "eat" } else { "drink" }, vol(g));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_sound_renders_and_music_parses() {
        let s = Sound::new();
        for k in [
            "eat",
            "drink",
            "notif",
            "scream",
            "fart",
            "join",
            "leave",
            "chat",
            "click",
            "page",
            "confirm",
            "erro",
            "pickup",
            "wash",
            "peel",
            "cook_done",
            "sit",
            "stand",
            "jump",
            "land",
            "step_sand",
            "step_grass",
            "step_water",
            "step_rock",
            "e_laugh",
            "e_love",
            "e_party",
            "e_clap",
            "e_fire",
            "e_sparkle",
            "e_sad",
            "e_angry",
            "e_wow",
            "e_sleep",
            "e_pop",
            "g_whoosh",
            "g_dance",
            "med_ouch",
            "med_heal",
            "n_video",
            "n_dj",
            "n_karaoke",
        ] {
            s.play(k, 1.0);
        }
        s.med(64.0, "ritual", false, 1.0);
        let mut snd = s;
        snd.music("cerron");
        snd.ambient("amazora");
        snd.rain(0.5);
        // dois segundos de áudio: tudo toca, nada estoura (o limitador segura) e nada vira NaN
        let mut peak = 0.0f32;
        for _ in 0..(SR * 2 / 512) {
            snd.tick();
            let mut e = snd.e.lock().unwrap();
            for _ in 0..512 {
                let o = e.frame();
                assert!(o[0].is_finite() && o[1].is_finite());
                peak = peak.max(o[0].abs());
            }
            e.collect();
        }
        assert!(peak > 0.05 && peak <= 0.951, "{peak}");
        // as faixas leem como no original
        let t = track("menu").unwrap();
        assert_eq!(t.chords[0], (39, vec![58, 62, 65, 67]));
        assert_eq!(t.mel[0][0], Some((79, 1)));
        let a = track("atlanthia").unwrap();
        assert_eq!(a.mel[0][0], Some((74, 4)));
        assert_eq!(emo_kind("😂"), "e_laugh");
        assert!((dvol(0.0) - 1.0).abs() < 1e-6 && dvol(25.0) == 0.0);
    }
}
