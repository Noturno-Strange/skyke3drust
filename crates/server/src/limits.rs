//! Limitador por janela deslizante e banimentos escalonados (`lib/limits.js`).
//!
//! As chaves ficam em ordem de uso (a tocada vai para o fim) e, passando do teto, as mais antigas saem primeiro.

use indexmap::IndexMap;
use std::collections::VecDeque;

/// Resultado de um [`Limiter::hit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    pub over: bool,
    pub count: usize,
}

/// `createLimiter`: conta eventos por chave numa janela de tempo.
#[derive(Debug)]
pub struct Limiter {
    map: IndexMap<String, VecDeque<u64>>,
    max_keys: usize,
    sweep_ms: u64,
    max_age_ms: u64,
    last_sweep: u64,
}

impl Limiter {
    pub fn new(max_keys: usize) -> Self {
        Self::with(max_keys, 60_000, 600_000)
    }

    pub fn with(max_keys: usize, sweep_ms: u64, max_age_ms: u64) -> Self {
        Self { map: IndexMap::new(), max_keys, sweep_ms, max_age_ms, last_sweep: 0 }
    }

    fn sweep(&mut self, now: u64, force: bool) {
        if !force && now.saturating_sub(self.last_sweep) < self.sweep_ms {
            return;
        }
        self.last_sweep = now;
        let age = self.max_age_ms;
        self.map.retain(|_, a| a.back().is_some_and(|&t| now.saturating_sub(t) <= age));
    }

    fn trim(&mut self) {
        while self.map.len() > self.max_keys {
            self.map.shift_remove_index(0);
        }
    }

    /// Registra um evento e diz se passou de `max` na janela `win`.
    pub fn hit(&mut self, key: &str, max: usize, win: u64, now: u64) -> Hit {
        self.sweep(now, false);
        let mut a = self.map.shift_remove(key).unwrap_or_default();
        while a.front().is_some_and(|&t| now.saturating_sub(t) >= win) {
            a.pop_front();
        }
        a.push_back(now);
        let count = a.len();
        self.map.insert(key.to_owned(), a);
        if self.map.len() > self.max_keys {
            self.trim();
        }
        Hit { over: count > max, count }
    }

    /// Quantos eventos da chave estão dentro da janela (sem registrar nada).
    pub fn count(&self, key: &str, win: u64, now: u64) -> usize {
        self.map.get(key).map_or(0, |a| a.iter().filter(|&&t| now.saturating_sub(t) < win).count())
    }

    pub fn over(&mut self, key: &str, max: usize, win: u64, now: u64) -> bool {
        self.hit(key, max, win, now).over
    }

    pub fn blocked(&self, key: &str, max: usize, win: u64, now: u64) -> bool {
        self.count(key, win, now) >= max
    }

    pub fn prune(&mut self, now: u64) {
        self.sweep(now, true);
        self.trim();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BanRec {
    pub level: usize,
    pub until: u64,
    pub last: u64,
}

/// `createBanStore`: banimento que escala de nível a cada reincidência e esquece depois de `reset_ms` limpo.
#[derive(Debug)]
pub struct BanStore {
    map: IndexMap<String, BanRec>,
    pub levels: Vec<u64>,
    reset_ms: u64,
    max: usize,
    last_expire: u64,
}

impl BanStore {
    pub fn new(levels: Vec<u64>, reset_ms: u64, max: usize) -> Self {
        Self { map: IndexMap::new(), levels, reset_ms, max, last_expire: 0 }
    }

    fn clamp(&self, i: usize) -> usize {
        i.min(self.levels.len().saturating_sub(1))
    }

    fn expire(&mut self, now: u64) {
        self.last_expire = now;
        let r = self.reset_ms;
        self.map.retain(|_, e| !(e.until <= now && now.saturating_sub(e.last) > r));
    }

    fn trim(&mut self, now: u64) {
        if now.saturating_sub(self.last_expire) >= 60_000 {
            self.expire(now);
        }
        while self.map.len() > self.max {
            self.map.shift_remove_index(0);
        }
    }

    fn put(&mut self, key: &str, rec: BanRec, now: u64) {
        self.map.shift_remove(key);
        self.map.insert(key.to_owned(), rec);
        if self.map.len() > self.max {
            self.trim(now);
        }
    }

    /// Milissegundos que faltam do banimento (0 = livre).
    pub fn check(&mut self, key: &str, now: u64) -> u64 {
        let Some(e) = self.map.get(key).copied() else { return 0 };
        if e.until > now {
            return e.until - now;
        }
        if now.saturating_sub(e.last) > self.reset_ms {
            self.map.shift_remove(key);
        }
        0
    }

    /// Uma infração: bane pelo tempo do nível atual e sobe um nível. Devolve (ms, até, nível usado).
    pub fn strike(&mut self, key: &str, now: u64) -> (u64, u64, usize) {
        let e = match self.map.get(key) {
            Some(e) if !(e.until <= now && now.saturating_sub(e.last) > self.reset_ms) => *e,
            _ => BanRec { level: 0, until: 0, last: 0 },
        };
        let lvl = self.clamp(e.level);
        let ms = self.levels[lvl];
        let rec = BanRec { level: self.clamp(lvl + 1), until: now + ms, last: now };
        self.put(key, rec, now);
        (ms, rec.until, lvl)
    }

    /// Banimento manual (não escala o nível).
    pub fn block(&mut self, key: &str, ms: u64, now: u64) -> Option<u64> {
        if ms == 0 {
            return None;
        }
        let level = self.map.get(key).map_or(0, |e| self.clamp(e.level));
        let rec = BanRec { level, until: now + ms, last: now };
        self.put(key, rec, now);
        Some(rec.until)
    }

    pub fn unban(&mut self, key: &str) -> bool {
        self.map.shift_remove(key).is_some()
    }

    /// Banimentos ativos, o que expira por último primeiro.
    pub fn list(&self, now: u64) -> Vec<(String, BanRec)> {
        let mut out: Vec<_> = self.map.iter().filter(|(_, e)| e.until > now).map(|(k, e)| (k.clone(), *e)).collect();
        out.sort_by_key(|x| std::cmp::Reverse(x.1.until));
        out
    }

    pub fn prune(&mut self, now: u64) {
        self.expire(now);
        while self.map.len() > self.max {
            self.map.shift_remove_index(0);
        }
    }

    pub fn snapshot(&self, now: u64) -> serde_json::Map<String, serde_json::Value> {
        let mut out = serde_json::Map::new();
        for (k, e) in &self.map {
            if e.until > now || now.saturating_sub(e.last) <= self.reset_ms {
                out.insert(k.clone(), serde_json::json!([e.until, e.level, e.last]));
            }
        }
        out
    }

    pub fn restore(&mut self, obj: &serde_json::Value) {
        let Some(o) = obj.as_object() else { return };
        for (k, v) in o {
            if self.map.len() >= self.max {
                break;
            }
            let Some(a) = v.as_array().filter(|a| a.len() == 3) else { continue };
            let n: Vec<f64> = a.iter().filter_map(serde_json::Value::as_f64).collect();
            if n.len() != 3 || n.iter().any(|x| !x.is_finite()) {
                continue;
            }
            let level = self.clamp(n[1].max(0.0) as usize);
            self.map.insert(k.clone(), BanRec { until: n[0].max(0.0) as u64, level, last: n[2].max(0.0) as u64 });
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_and_order() {
        let mut l = Limiter::new(2);
        assert!(!l.over("a", 2, 1000, 0));
        assert!(!l.over("a", 2, 1000, 10));
        assert!(l.over("a", 2, 1000, 20));
        assert_eq!(l.count("a", 1000, 1015), 1);
        assert!(!l.over("a", 2, 1000, 1015));
        l.hit("b", 9, 1000, 0);
        l.hit("c", 9, 1000, 0);
        assert_eq!(l.len(), 2);
        assert_eq!(l.count("a", 1000, 1015), 0);
    }

    #[test]
    fn ban_levels() {
        let mut b = BanStore::new(vec![100, 1000, 5000], 10_000, 10);
        assert_eq!(b.strike("x", 0).0, 100);
        assert_eq!(b.check("x", 50), 50);
        assert_eq!(b.strike("x", 200).0, 1000);
        assert_eq!(b.strike("x", 2000).0, 5000);
        assert_eq!(b.strike("x", 8000).0, 5000);
        assert_eq!(b.strike("x", 100_000).0, 100);
    }
}
