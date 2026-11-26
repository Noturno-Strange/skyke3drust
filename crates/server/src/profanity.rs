//! Filtro de palavrões e de nomes ofensivos (`lib/profanity.js`). O texto é "dobrado" (acentos, sósias, leet speak)
//! para casar com as listas, mas a máscara cai sobre o texto original, letra a letra.
//!
//! O original usa `(?<![a-z])` e `(?![a-z])`, que o `regex` do Rust não tem: a busca anda posição por posição como a
//! do JavaScript, confere à mão a letra anterior e troca o "não seguido de letra" por consumir uma não letra (ou o fim),
//! o que preserva a mesma escolha de alternativa e de repetição.

use crate::validate::{RESERVED_NAMES, skeleton, unconfuse};
use regex::Regex;
use std::collections::HashSet;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

const BASE: [&str; 72] = [
    "porra",
    "caralho",
    "caraio",
    "merda",
    "bosta",
    "puta",
    "puto",
    "putaria",
    "putinha",
    "putao",
    "putona",
    "vagabunda",
    "vagabundo",
    "vadia",
    "piranha",
    "buceta",
    "boceta",
    "bucetinha",
    "xoxota",
    "xereca",
    "piroca",
    "punheta",
    "punheteiro",
    "cuzao",
    "cuzinho",
    "cu",
    "foda",
    "foder",
    "fodeu",
    "fodido",
    "fodida",
    "fodase",
    "fodasse",
    "fodendo",
    "fodo",
    "fodam",
    "fodi",
    "fodemos",
    "arrombado",
    "arrombada",
    "desgraca",
    "desgracado",
    "desgracada",
    "corno",
    "corna",
    "cornudo",
    "otario",
    "otaria",
    "filhodaputa",
    "filhadaputa",
    "viado",
    "viada",
    "viadinho",
    "sapatao",
    "traveco",
    "boiola",
    "retardado",
    "retardada",
    "mongoloide",
    "crioulo",
    "crioula",
    "fdp",
    "pqp",
    "vsf",
    "vtnc",
    "vtmnc",
    "tnc",
    "tmnc",
    "krl",
    "crl",
    "crlh",
    "krlh",
];
const STRICT: [&str; 12] = ["cu", "fdp", "pqp", "vsf", "vtnc", "vtmnc", "tnc", "tmnc", "krl", "crl", "crlh", "krlh"];
const HATE: [&str; 61] = [
    "hitler",
    "hitller",
    "adolf",
    "nazi",
    "nazis",
    "nazista",
    "nazistas",
    "neonazi",
    "neonazista",
    "nazismo",
    "nazifascista",
    "fascista",
    "fascismo",
    "skinhead",
    "sieg",
    "heil",
    "seigheil",
    "reich",
    "furher",
    "fuhrer",
    "goebbels",
    "himmler",
    "mengele",
    "auschwitz",
    "holocausto",
    "kkk",
    "klansman",
    "supremacista",
    "whitepower",
    "poderbranco",
    "nigger",
    "niggers",
    "nigga",
    "niggas",
    "faggot",
    "faggots",
    "fag",
    "fags",
    "retard",
    "retards",
    "chink",
    "spic",
    "kike",
    "tranny",
    "paki",
    "coon",
    "wetback",
    "beaner",
    "gook",
    "macaco",
    "macaca",
    "bicha",
    "bichona",
    "sapatao",
    "traveco",
    "gayzao",
    "estuprador",
    "estupro",
    "pedofilo",
    "pedofilia",
    "pedo",
];
const RESERVED: [&str; 15] = [
    "admin",
    "administrador",
    "adm",
    "moderador",
    "moderacao",
    "mod",
    "suporte",
    "staff",
    "oficial",
    "skyke",
    "skyke3d",
    "sistema",
    "servidor",
    "dono",
    "criador",
];

fn cls(c: char) -> String {
    match c {
        'u' => "[uv]".into(),
        'v' => "[vu]".into(),
        'l' => "[li]".into(),
        'i' => "[il]".into(),
        c => regex::escape(&c.to_string()),
    }
}

fn pattern(w: &str) -> String {
    if w == "cu" {
        return "c+u+s?".into();
    }
    if STRICT.contains(&w) {
        return w.chars().map(|c| format!("{c}+")).collect::<Vec<_>>().join("[^a-z0-9]?") + "s?";
    }
    w.chars().map(|c| format!("{}+", cls(c))).collect::<Vec<_>>().join("[^a-z0-9]{0,3}")
        + "(?:s|inha|inhas|inho|inhos|ao|oes|ona|onas)?"
}

fn build(words: &[&str], pat: impl Fn(&str) -> String) -> Regex {
    let mut v: Vec<&str> = Vec::new();
    for w in words {
        if !v.contains(w) {
            v.push(w);
        }
    }
    // mais longas primeiro (ordenação estável, como `Array.sort`)
    v.sort_by_key(|w| std::cmp::Reverse(w.len()));
    let alts: Vec<String> = v.iter().map(|w| pat(w)).collect();
    // `(?s)`: o `[^a-z0-9]` também casa quebra de linha, como no JavaScript
    Regex::new(&format!("(?s)^({})(?:[^a-z]|$)", alts.join("|")))
        .unwrap_or_else(|_| Regex::new("^$").unwrap_or_else(|_| unreachable!()))
}

fn rx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| build(&BASE, pattern))
}

fn hx() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        build(&HATE, |w| w.chars().map(|c| format!("{}+", cls(c))).collect::<Vec<_>>().join("[^a-z0-9]{0,2}"))
    })
}

fn leet(c: char) -> Option<char> {
    Some(match c {
        '0' => 'o',
        '1' => 'i',
        '3' => 'e',
        '4' => 'a',
        '5' => 's',
        '7' => 't',
        _ => return None,
    })
}

fn is_letter(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_lowercase())
}

/// Texto "dobrado", um caractere por caractere do original (NFC).
fn fold(text: &str) -> Vec<char> {
    let mut base: Vec<char> = text
        .nfc()
        .map(|ch| {
            // `normalize('NFKD')[0]`: a primeira unidade UTF-16; fora do plano básico sobra meio par (não é letra)
            let first = ch.to_string().nfkd().next().unwrap_or(ch);
            if first.len_utf16() == 2 {
                return '\u{FFFD}';
            }
            let low: String = first.to_lowercase().collect();
            // `toLowerCase` pode gerar mais de um caractere; o original pega só o primeiro de NFKD antes
            let u = unconfuse(&low);
            u.chars().next().unwrap_or(ch)
        })
        .collect();
    for _ in 0..3 {
        let prev_round = base.clone();
        let mut changed = false;
        for i in 0..base.len() {
            let c = prev_round[i];
            let prev = if i > 0 { prev_round.get(i - 1).copied() } else { None };
            let next = prev_round.get(i + 1).copied();
            let r = if let Some(l) = leet(c) {
                if is_letter(prev) || is_letter(next) { l } else { c }
            } else if c == '!' {
                if is_letter(prev) && is_letter(next) { 'i' } else { c }
            } else if c == '@' || c == '$' {
                if is_letter(prev) || is_letter(next) { if c == '@' { 'a' } else { 's' } } else { c }
            } else {
                c
            };
            if r != c {
                changed = true;
            }
            base[i] = r;
        }
        if !changed {
            break;
        }
    }
    base
}

/// Todas as ocorrências (início e fim em caracteres), como a busca global do JavaScript.
fn find_all(re: &Regex, f: &[char]) -> Vec<(usize, usize)> {
    let s: String = f.iter().collect();
    let idx: Vec<usize> = s.char_indices().map(|(b, _)| b).chain(std::iter::once(s.len())).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < f.len() {
        if i == 0 || !f[i - 1].is_ascii_lowercase() {
            let n = re.captures(&s[idx[i]..]).and_then(|c| c.get(1)).map_or(0, |m| m.as_str().chars().count());
            if n > 0 {
                out.push((i, i + n));
                i += n;
                continue;
            }
        }
        i += 1;
    }
    out
}

const MASK: &str = "!@#$%&*";
fn mask_of(len: usize) -> &'static str {
    &MASK[..len.clamp(3, MASK.len())]
}

/// Troca os palavrões por `!@#…` (do tamanho da palavra, de 3 a 7).
pub fn censor(text: &str) -> String {
    if text.is_empty() {
        return text.into();
    }
    let chars: Vec<char> = text.nfc().collect();
    let folded = fold(text);
    if folded.len() != chars.len() {
        return text.into();
    }
    let hits = find_all(rx(), &folded);
    if hits.is_empty() {
        return text.into();
    }
    let mut out = String::new();
    let mut last = 0;
    for (a, b) in hits {
        out.extend(&chars[last..a]);
        out += mask_of(b - a);
        last = b;
    }
    out.extend(&chars[last..]);
    out
}

pub fn has_profanity(text: &str) -> bool {
    censor(text) != text
}

/// Nome não permitido: vazio, palavrão, ódio ou reservado. Devolve o motivo.
pub fn bad_name(name: &str, reserved: bool) -> Option<&'static str> {
    if name.trim().is_empty() {
        return Some("vazio");
    }
    let f = fold(name);
    if !find_all(rx(), &f).is_empty() {
        return Some("palavrao");
    }
    if !find_all(hx(), &f).is_empty() {
        return Some("odio");
    }
    let mut k = String::new();
    for c in f.iter().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) {
        if !k.ends_with(*c) {
            k.push(*c);
        }
    }
    const HATE_K: [&str; 12] = [
        "hitler",
        "nazista",
        "neonazi",
        "heilhitler",
        "siegheil",
        "goebbels",
        "himmler",
        "auschwitz",
        "nigger",
        "faggot",
        "whitepower",
        "poderbranco",
    ];
    if HATE_K.iter().any(|h| k.contains(h)) {
        return Some("odio");
    }
    if name.chars().filter(char::is_ascii_digit).collect::<String>().contains("1488") {
        return Some("odio");
    }
    let cru: String = name.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit()).collect();
    static SK: OnceLock<HashSet<String>> = OnceLock::new();
    let sk = SK.get_or_init(|| RESERVED.iter().chain(RESERVED_NAMES.iter()).map(|w| skeleton(w)).collect());
    if reserved && (RESERVED.contains(&k.as_str()) || RESERVED.contains(&cru.as_str()) || sk.contains(&skeleton(name)))
    {
        return Some("reservado");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saídas do `lib/profanity.js` original (Node) para os mesmos textos.
    #[test]
    fn matches_node() {
        let table = [
            ("Merdinha", "Merdinha", None),
            ("que merda", "que !@#$%", Some("palavrao")),
            ("p0rr4 de jogo", "!@#$% de jogo", Some("palavrao")),
            ("cuidado", "cuidado", None),
            ("vai tnc", "vai !@#", Some("palavrao")),
            ("putinhas!", "!@#$%&*!", Some("palavrao")),
            ("merda 😀", "!@#$% 😀", Some("palavrao")),
            ("computador", "computador", None),
            ("f.o.d.a", "!@#$%&*", Some("palavrao")),
            ("c u", "c u", None),
            ("pqp!!", "!@#!!", Some("palavrao")),
            ("fdps", "!@#$", Some("palavrao")),
            ("kkkk", "kkkk", Some("odio")),
            ("Hitler", "Hitler", Some("odio")),
            ("x1488", "x1488", Some("odio")),
            ("Admin", "Admin", Some("reservado")),
            ("m e r d a", "!@#$%&*", Some("palavrao")),
            ("buceta123", "buceta123", None),
            ("viado", "!@#$%", Some("palavrao")),
        ];
        for (t, c, b) in table {
            assert_eq!(censor(t), c, "{t}");
            assert_eq!(bad_name(t, true), b, "{t}");
        }
    }
}
