//! Falas dos bots (`lib/bots.js`): respondem ao chat com piadas, sem inundar a sala (fila curta, intervalo mínimo e no
//! máximo 5 falas a cada 30 s), revezando quem fala e dando a vez ao bot chamado pelo nome.

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

const GAP_MS: f64 = 2600.0;
const GAP_JITTER_MS: f64 = 1800.0;
const REPLY_DELAY_MS: (f64, f64) = (900.0, 2600.0);
const MAX_QUEUE: usize = 3;
const WINDOW_MS: f64 = 30_000.0;
const MAX_PER_WINDOW: usize = 5;
const COOLDOWN_BASE_MS: f64 = 15_000.0;
const COOLDOWN_PER_BOT_MS: f64 = 2200.0;

pub const JOKES: [&str; 29] = [
    "Eu só entendo 3 coisas: andar em círculos, fogueira e pizza. 🍕",
    "Tô aqui só pelo suco, não conta pra ninguém. 🧃",
    "Isso foi dito por alguém que claramente não dividiu a pizza. 🍕",
    "Minha bateria social acabou, mas continuo aqui. 🔋",
    "{n}, você falou, eu ouvi e mesmo assim não entendi nada. 😅",
    "Chequei meu manual de bots: a resposta é \"talvez\". 🤖",
    "Eu ia responder algo inteligente, mas lembrei que sou um bot. 🤖",
    "Pera, deixa eu fingir que tô pensando... 🤔 ...pronto, tô.",
    "Sou um bot com 100% de entusiasmo e 0% de noção. 🎉",
    "Alguém viu minha dignidade? Estava aqui agorinha. 🔍",
    "Eu dançaria, mas esqueci de baixar as pernas. 💃",
    "Juro que não sou eu quem anda em círculos. Ou sou? 🌀",
    "Isso me lembrou de uma piada, mas esqueci o final. Fim. 🎭",
    "Se eu tivesse um real para cada frase dessas, teria 0 reais. 💸",
    "A fogueira concorda com {n}. Ela só estala, mas concorda. 🔥",
    "Eu não durmo, não como e mesmo assim estou cansado. 😴",
    "Calma, {n}, eu sou só um boneco de pixels com opiniões duvidosas. 🧱",
    "Mandei isso pro meu departamento de ideias: ainda tá carregando. ⏳",
    "Sábias palavras, {n}. Não entendi, mas sábias. 🧙",
    "Se der erro, a culpa é do lag. Sempre do lag. 📶",
    "Tô ouvindo tudo e entendendo metade. Média de bot. 📊",
    "Eu votaria nisso, mas bots não têm título de eleitor. 🗳️",
    "Fala isso de novo que eu finjo que entendi melhor. 😎",
    "Meu cérebro é uma batata com Wi-Fi. 🥔",
    "Hoje eu acordei bot e escolhi a violência... de sorrir. 😁",
    "Isso é tão verdade que meu código até deu um \"uau\". ✨",
    "Apertei F pra sentar e sentei no meio da conversa. 🪑",
    "Já pensou se o tronco da fogueira falasse? Seria menos barulhento que eu. 🪵",
    "Eu respondo tudo igual: com confiança e sem saber. 😌",
];

/// Sem acentos, minúsculas, só `a-z0-9` separados por um espaço.
pub fn normalize(text: &str) -> String {
    let low: String = text.nfd().filter(|c| !is_combining_mark(*c)).collect::<String>().to_lowercase();
    let mut out = String::new();
    let mut sep = false;
    for c in low.chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if sep && !out.is_empty() {
                out.push(' ');
            }
            sep = false;
            out.push(c);
        } else {
            sep = true;
        }
    }
    out
}

#[derive(Debug, Clone)]
struct Queued {
    due: f64,
    text: String,
    prefer: Option<String>,
}

/// Estado do papo dos bots de uma sala.
#[derive(Debug, Clone, Default)]
pub struct BotChat {
    q: Vec<Queued>,
    next: f64,
    rr: usize,
    spoken: std::collections::HashMap<String, f64>,
    win: Vec<f64>,
    last_joke: Option<usize>,
}

/// Bot como o papo vê: id e nome.
#[derive(Debug, Clone)]
pub struct Bot {
    pub id: String,
    pub name: String,
}

/// Contém a palavra inteira (`\b…\b` sobre o texto já normalizado).
fn has_word(hay: &str, w: &str) -> bool {
    if w.is_empty() {
        return false;
    }
    let ok = |c: Option<char>| c.is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
    hay.match_indices(w).any(|(i, _)| ok(hay[..i].chars().last()) && ok(hay[i + w.len()..].chars().next()))
}

impl BotChat {
    fn pick(&mut self, rand: &mut impl FnMut() -> f64) -> &'static str {
        let mut i = (rand() * JOKES.len() as f64) as usize % JOKES.len();
        if self.last_joke == Some(i) {
            i = (i + 1) % JOKES.len();
        }
        self.last_joke = Some(i);
        JOKES[i]
    }

    /// Alguém falou na sala: talvez um bot responda depois de um tempinho.
    pub fn hear(&mut self, text: &str, bots: &[Bot], now: f64, who: &str, rand: &mut impl FnMut() -> f64) -> bool {
        if bots.is_empty() {
            return false;
        }
        let norm = normalize(text);
        if norm.is_empty() {
            return false;
        }
        self.win.retain(|t| now - t < WINDOW_MS);
        if self.q.len() >= MAX_QUEUE || self.win.len() + self.q.len() >= MAX_PER_WINDOW {
            return false;
        }
        let named = bots.iter().find(|b| has_word(&norm, &normalize(&b.name))).map(|b| b.id.clone());
        let due = now + REPLY_DELAY_MS.0 + rand() * (REPLY_DELAY_MS.1 - REPLY_DELAY_MS.0);
        let who = if who.is_empty() { "amigo" } else { who };
        let text = self.pick(rand).replace("{n}", who);
        self.q.push(Queued { due, text, prefer: named });
        true
    }

    fn choose(&mut self, bots: &[Bot], now: f64, prefer: Option<&str>) -> usize {
        let cool = COOLDOWN_BASE_MS.max(bots.len() as f64 * COOLDOWN_PER_BOT_MS);
        let ready = |s: &Self, b: &Bot| now - s.spoken.get(&b.id).copied().unwrap_or(0.0) >= cool;
        if let Some(p) = prefer
            && let Some(i) = bots.iter().position(|b| b.id == p)
            && ready(self, &bots[i])
        {
            return i;
        }
        let n = bots.len();
        for k in 0..n {
            let i = (self.rr + k) % n;
            if ready(self, &bots[i]) {
                self.rr = (i + 1) % n;
                return i;
            }
        }
        let sp = |b: &Bot| self.spoken.get(&b.id).copied().unwrap_or(0.0);
        let mut best = 0;
        for (i, b) in bots.iter().enumerate() {
            if sp(b) < sp(&bots[best]) {
                best = i;
            }
        }
        self.rr = (best + 1) % n;
        best
    }

    /// A cada 250 ms: devolve (índice do bot, texto) quando chega a hora de uma fala.
    pub fn tick(
        &mut self,
        now: f64,
        bots: &[Bot],
        humans_online: bool,
        rand: &mut impl FnMut() -> f64,
    ) -> Option<(usize, String)> {
        if bots.is_empty() || !humans_online {
            self.q.clear();
            return None;
        }
        let head = self.q.first()?.clone();
        if now < head.due || now < self.next {
            return None;
        }
        self.win.retain(|t| now - t < WINDOW_MS);
        self.q.remove(0);
        let i = self.choose(bots, now, head.prefer.as_deref());
        self.next = now + GAP_MS + rand() * GAP_JITTER_MS;
        self.spoken.insert(bots[i].id.clone(), now);
        self.win.push(now);
        self.spoken.retain(|id, _| bots.iter().any(|b| &b.id == id));
        Some((i, head.text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_in_turn() {
        let bots = vec![Bot { id: "b1".into(), name: "Marina".into() }, Bot { id: "b2".into(), name: "Caio".into() }];
        let mut st = BotChat::default();
        let mut r = || 0.0;
        let t0 = 100_000.0;
        assert!(st.hear("oi Caio, tudo bem?", &bots, t0, "Ana", &mut r));
        assert!(st.tick(t0 + 500.0, &bots, true, &mut r).is_none());
        let (i, text) = st.tick(t0 + 1000.0, &bots, true, &mut r).unwrap();
        assert_eq!(bots[i].id, "b2");
        assert_eq!(text, JOKES[0]);
        assert_eq!(normalize("Olá, João!"), "ola joao");
    }
}
