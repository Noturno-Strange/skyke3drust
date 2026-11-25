//! Segurança compartilhada por cliente e servidor.
//!
//! * [`pow`] — ticket de admissão e prova de trabalho do registro de identidade (compatíveis com o servidor Node.js).
//! * [`RateLimiter`] — janela fixa por chave, como `hitCount`/`over` do `server.js`.
//! * [`clean_text`] — remove caracteres de controle como `strip` em `lib/validate.js`.
//!
//! A regra de nomes reservados e parecidos (`nm`/`skeleton` em `lib/validate.js`) será portada junto com o servidor Rust;
//! enquanto o servidor Node.js for a autoridade, ele continua validando todos os nomes.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod pow;

use std::collections::HashMap;
use std::hash::Hash;

/// Resultado de um registro no limitador.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hit {
    /// Quantos eventos já houve na janela atual (incluindo este).
    pub count: u32,
    /// Passou do limite?
    pub over: bool,
}

/// Limitador de frequência por chave, em janela fixa (mesma semântica de `hitCount` no servidor original).
#[derive(Debug)]
pub struct RateLimiter<K> {
    windows: HashMap<K, (u64, u32)>,
    max_keys: usize,
}

impl<K: Eq + Hash + Clone> RateLimiter<K> {
    /// `max_keys` limita a memória: acima disso, as janelas vencidas são descartadas e, se ainda faltar espaço,
    /// o registro é tratado como excesso (falha fechada).
    pub fn new(max_keys: usize) -> Self {
        Self { windows: HashMap::new(), max_keys }
    }

    pub fn hit(&mut self, key: &K, limit: u32, window_ms: u64, now_ms: u64) -> Hit {
        if !self.windows.contains_key(key) && self.windows.len() >= self.max_keys {
            self.prune(now_ms, window_ms);
            if self.windows.len() >= self.max_keys {
                return Hit { count: u32::MAX, over: true };
            }
        }
        let e = self.windows.entry(key.clone()).or_insert((now_ms, 0));
        if now_ms.saturating_sub(e.0) >= window_ms {
            *e = (now_ms, 0);
        }
        e.1 = e.1.saturating_add(1);
        Hit { count: e.1, over: e.1 > limit }
    }

    /// Remove janelas mais velhas que `window_ms`.
    pub fn prune(&mut self, now_ms: u64, window_ms: u64) {
        self.windows.retain(|_, (start, _)| now_ms.saturating_sub(*start) < window_ms);
    }

    pub fn len(&self) -> usize {
        self.windows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
}

/// Remove controles C0/C1 e caracteres invisíveis comuns, como o `strip` do validador original.
pub fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|&c| {
            let u = c as u32;
            !(u <= 0x1f
                || (0x7f..=0x9f).contains(&u)
                || matches!(u, 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x2064 | 0xfeff | 0x2800 | 0x2028 | 0x2029 | 0x00ad))
        })
        .collect()
}

/// Texto de chat como o cliente original envia: sem controles, aparado e com no máximo `max` caracteres.
pub fn clean_chat(s: &str, max: usize) -> String {
    clean_text(s).trim().chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_window() {
        let mut r = RateLimiter::new(10);
        let k = "ip".to_string();
        for i in 1..=3 {
            assert_eq!(r.hit(&k, 3, 1000, 0), Hit { count: i, over: false });
        }
        assert!(r.hit(&k, 3, 1000, 10).over);
        assert!(!r.hit(&k, 3, 1000, 1000).over);
    }

    #[test]
    fn bounded_memory_fails_closed() {
        let mut r = RateLimiter::new(2);
        r.hit(&1, 5, 1000, 0);
        r.hit(&2, 5, 1000, 0);
        assert!(r.hit(&3, 5, 1000, 10).over);
        // depois que as janelas vencem, há espaço de novo
        assert!(!r.hit(&3, 5, 1000, 5000).over);
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn strips_controls() {
        assert_eq!(clean_text("a\u{0}b\u{200b}c\u{85}d"), "abcd");
        assert_eq!(clean_chat("  oi\n  ", 200), "oi");
        assert_eq!(clean_chat("abcdef", 3), "abc");
    }
}
