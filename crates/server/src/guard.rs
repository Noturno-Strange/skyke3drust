//! Antifraude da fazenda e da culinária (`lib/guard.js`): ritmo de robô, rajadas, cansaço do dia (ganho menor depois
//! do teto diário) e identidades em série (poucas fazendas e cozinhas novas por origem por dia).

use indexmap::IndexMap;
use serde_json::{Value, json};
use std::collections::HashMap;

const HOUR: u64 = 3_600_000;
const DAY: u64 = 24 * HOUR;

#[derive(Debug, Clone, Default)]
struct P {
    t: Vec<u64>,
    score: i64,
    at: u64,
    until: u64,
    why: String,
    gain: HashMap<String, [f64; 24]>,
    hour: u64,
}

#[derive(Debug, Default, Clone, Copy)]
struct Stats {
    robo: u64,
    rajada: u64,
    descanso: u64,
    cansaco: u64,
    novos: u64,
}

#[derive(Debug)]
pub struct Guard {
    ps: IndexMap<String, P>,
    fresh: IndexMap<String, Vec<u64>>,
    max: usize,
    burst: f64,
    new_per_day: usize,
    stats: Stats,
}

impl Default for Guard {
    fn default() -> Self {
        Self {
            ps: IndexMap::new(),
            fresh: IndexMap::new(),
            max: 20000,
            burst: 90.0,
            new_per_day: 3,
            stats: Stats::default(),
        }
    }
}

fn cap_of(kind: &str) -> f64 {
    match kind {
        "farm" => 6000.0,
        "cook" => 3000.0,
        _ => f64::INFINITY,
    }
}

impl Guard {
    fn get(&mut self, pid: &str) -> &mut P {
        let n = crate::now();
        if !self.ps.contains_key(pid) {
            if self.ps.len() >= self.max {
                self.ps.shift_remove_index(0);
            }
            self.ps.insert(pid.into(), P { at: n, hour: n / HOUR, ..Default::default() });
        }
        let p = self.ps.get_mut(pid).unwrap_or_else(|| unreachable!());
        let drop = n.saturating_sub(p.at) / 300_000;
        if drop > 0 {
            p.score = (p.score - drop as i64).max(0);
            p.at += drop * 300_000;
        }
        if p.score == 0 && p.until < n {
            p.why.clear();
        }
        p
    }

    fn flag(&mut self, pid: &str, pts: i64, why: &str) {
        let n = crate::now();
        let p = self.get(pid);
        p.score += pts;
        p.why = why.into();
        if p.score >= 10 && p.until < n {
            p.until = n + 600_000;
            self.stats.descanso += 1;
        }
    }

    /// Pode agir agora? `Err((motivo, espera em s))` durante o descanso.
    pub fn check(&mut self, pid: &str) -> Value {
        let n = crate::now();
        let p = self.get(pid);
        if p.until > n {
            json!({"ok": false, "reason": "descanso", "wait": (p.until - n).div_ceil(1000)})
        } else {
            json!({"ok": true})
        }
    }

    /// Uma ação feita: confere rajada e ritmo de robô.
    pub fn act(&mut self, pid: &str) {
        let n = crate::now();
        let burst = self.burst;
        let p = self.get(pid);
        p.t.push(n);
        if p.t.len() > 24 {
            p.t.remove(0);
        }
        let last_min = p.t.iter().filter(|x| n - **x < 60000).count();
        if p.t.len() >= 24 && last_min >= 24 && ((n - p.t[0]) as f64) < 24.0 * 60000.0 / burst {
            p.t.clear();
            self.flag(pid, 2, "rajada");
            self.stats.rajada += 1;
            return;
        }
        if p.t.len() >= 17 {
            let iv: Vec<f64> = (p.t.len() - 16..p.t.len()).map(|i| (p.t[i] - p.t[i - 1]) as f64).collect();
            let mean = iv.iter().sum::<f64>() / iv.len() as f64;
            let sd = (iv.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / iv.len() as f64).sqrt();
            if mean < 4000.0 && mean > 0.0 && sd / mean < 0.1 {
                p.t.clear();
                self.flag(pid, 3, "ritmo de robô");
                self.stats.robo += 1;
            }
        }
    }

    /// Fator do ganho (0 a 1): cansaço do dia, suspeita e descanso.
    pub fn factor(&mut self, pid: &str, kind: &str, amount: f64, scale: f64) -> f64 {
        let n = crate::now();
        let h = n / HOUR;
        let p = self.get(pid);
        p.gain.entry(kind.into()).or_insert([0.0; 24]);
        let mut k = p.hour + 1;
        while k <= h && k <= p.hour + 24 {
            for g in p.gain.values_mut() {
                g[(k % 24) as usize] = 0.0;
            }
            k += 1;
        }
        p.hour = p.hour.max(h);
        let g = p.gain.get(kind).copied().unwrap_or([0.0; 24]);
        let day: f64 = g.iter().sum();
        let cap = cap_of(kind) * scale.max(1.0);
        let amt = amount.max(0.0);
        let mut f = if day >= cap * 2.0 {
            0.2
        } else if day >= cap {
            0.5
        } else {
            1.0
        };
        if amt > 0.0 && cap.is_finite() {
            let part = |lo: f64, hi: f64| ((day + amt).min(hi) - day.max(lo)).max(0.0);
            f = (part(0.0, cap) + part(cap, cap * 2.0) * 0.5 + part(cap * 2.0, f64::INFINITY) * 0.2) / amt;
        }
        let (score, until) = (p.score, p.until);
        if let Some(x) = p.gain.get_mut(kind) {
            x[(h % 24) as usize] += amount.max(0.0);
        }
        if f < 1.0 {
            self.stats.cansaco += 1;
        }
        if score >= 5 {
            f *= 0.5;
        }
        if until > n {
            f = 0.0;
        }
        f
    }

    /// Esta origem ainda pode abrir uma fazenda (ou cozinha) nova hoje?
    pub fn may_open(&mut self, origin: &str, kind: &str) -> bool {
        let k = format!("{kind}|{origin}");
        let n = crate::now();
        let list: Vec<u64> =
            self.fresh.get(&k).map(|l| l.iter().copied().filter(|x| n - x < DAY).collect()).unwrap_or_default();
        let ok = list.len() < self.new_per_day;
        self.fresh.insert(k, list);
        if !ok {
            self.stats.novos += 1;
        }
        ok
    }

    pub fn opened(&mut self, origin: &str, kind: &str) {
        self.fresh.entry(format!("{kind}|{origin}")).or_default().push(crate::now());
        if self.fresh.len() > self.max {
            self.fresh.shift_remove_index(0);
        }
    }

    pub fn report(&mut self) -> Value {
        let n = crate::now();
        let pids: Vec<String> = self.ps.keys().cloned().collect();
        let mut list = Vec::new();
        for pid in pids {
            let p = self.get(&pid).clone();
            if p.score > 0 || p.until > n {
                list.push(json!({"id": &pid[..8.min(pid.len())], "score": p.score, "why": p.why, "rest": if p.until > n { (p.until - n).div_ceil(1000) } else { 0 }}));
            }
        }
        list.sort_by_key(|x| std::cmp::Reverse(x["score"].as_i64().unwrap_or(0)));
        let resting = list.iter().filter(|x| x["rest"].as_u64().unwrap_or(0) > 0).count();
        let s = self.stats;
        json!({
            "watched": list.len(), "resting": resting, "top": list.into_iter().take(20).collect::<Vec<_>>(),
            "robo": s.robo, "rajada": s.rajada, "descanso": s.descanso, "cansaco": s.cansaco, "novos": s.novos
        })
    }
}
