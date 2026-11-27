//! Genes do mascote (`lib/pet-genes.js`): sorteio determinístico a partir de uma semente (xmur3 + sfc32, com a mesma
//! aritmética de 32 bits do JavaScript), pesos por gene e ajustes para combinações que não combinam.

use indexmap::IndexMap;

pub const GENE_KEYS: [&str; 14] = ["bd", "hd", "ey", "es", "ea", "tl", "ft", "pt", "pl", "tn", "ck", "sp", "pe", "ac"];
pub const GENE_SIZE: [i64; 14] = [8, 6, 7, 3, 8, 5, 3, 6, 16, 3, 3, 6, 6, 3];
const WEIGHTS: [&[f64]; 14] = [
    &[14.0, 14.0, 14.0, 12.0, 12.0, 12.0, 11.0, 11.0],
    &[20.0, 18.0, 16.0, 16.0, 15.0, 15.0],
    &[20.0, 18.0, 16.0, 14.0, 12.0, 12.0, 8.0],
    &[30.0, 45.0, 25.0],
    &[18.0, 16.0, 14.0, 12.0, 12.0, 10.0, 10.0, 8.0],
    &[20.0, 25.0, 20.0, 15.0, 20.0],
    &[40.0, 40.0, 20.0],
    &[24.0, 16.0, 16.0, 14.0, 16.0, 14.0],
    &[7.0, 7.0, 7.0, 7.0, 6.0, 7.0, 7.0, 6.0, 7.0, 6.0, 6.0, 5.0, 6.0, 7.0, 4.0, 5.0],
    &[60.0, 20.0, 20.0],
    &[30.0, 50.0, 20.0],
    &[78.0, 7.0, 7.0, 3.25, 3.25, 1.5],
    &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
    &[70.0, 15.0, 15.0],
];
pub const PERSONALITIES: [&str; 6] = ["Curioso", "Dorminhoco", "Brincalhão", "Calmo", "Guloso", "Sonhador"];
pub const SPECIALS: [&str; 6] =
    ["Nenhuma", "Aura brilhante", "Marca de estrela", "Arco-íris", "Constelação", "Cósmico"];
pub const RARITIES: [&str; 4] = ["Comum", "Incomum", "Rara", "Extremamente rara"];
const SPECIAL_TIER: [usize; 6] = [0, 1, 1, 2, 2, 3];

pub type Genes = IndexMap<&'static str, i64>;

fn xmur3(s: &str) -> impl FnMut() -> u32 + use<> {
    let units: Vec<u16> = s.encode_utf16().collect();
    let mut h: u32 = 1_779_033_703 ^ units.len() as u32;
    for c in units {
        h = (h ^ u32::from(c)).wrapping_mul(3_432_918_353);
        h = h.rotate_left(13);
    }
    move || {
        h = (h ^ (h >> 16)).wrapping_mul(2_246_822_507);
        h = (h ^ (h >> 13)).wrapping_mul(3_266_489_909);
        h ^= h >> 16;
        h
    }
}

/// Gerador pseudoaleatório determinístico (`makeRng`): números de 0 a 1.
pub fn make_rng(text: &str) -> impl FnMut() -> f64 + use<> {
    let mut s = xmur3(text);
    let (mut a, mut b, mut c, mut d) = (s(), s(), s(), s());
    move || {
        let mut t = a.wrapping_add(b);
        a = b ^ (b >> 9);
        b = c.wrapping_add(c << 3);
        c = c.rotate_left(21);
        d = d.wrapping_add(1);
        t = t.wrapping_add(d);
        c = c.wrapping_add(t);
        f64::from(t) / 4_294_967_296.0
    }
}

fn pick_weighted(rng: &mut impl FnMut() -> f64, w: &[f64]) -> i64 {
    let total: f64 = w.iter().sum();
    let mut r = rng() * total;
    for (i, x) in w.iter().enumerate() {
        r -= x;
        if r < 0.0 {
            return i as i64;
        }
    }
    w.len() as i64 - 1
}

pub fn harmonize(g: &mut Genes) {
    let v = |g: &Genes, k: &str| g.get(k).copied().unwrap_or(0);
    let smooth = [0, 3, 7].contains(&v(g, "bd"));
    if v(g, "ea") == 3 && ![0, 1, 3, 5].contains(&v(g, "hd")) {
        g.insert("ea", 1);
    }
    if v(g, "ea") == 7 && !smooth {
        g.insert("ea", 0);
    }
    if v(g, "tl") == 3 && !smooth && v(g, "ea") != 7 {
        g.insert("tl", 2);
    }
    if v(g, "pt") == 5 && v(g, "pl") == 14 {
        g.insert("pt", 2);
    }
    if v(g, "sp") == 5 && (v(g, "pl") == 12 || v(g, "pl") == 4) {
        g.insert("pl", 10);
    }
    if v(g, "ey") == 3 && v(g, "es") == 2 {
        g.insert("es", 1);
    }
}

pub fn genes_from_seed(seed: &str, attempt: u32) -> Genes {
    let mut rng = make_rng(&format!("skyke3d-pet:{seed}:{attempt}"));
    let mut g = Genes::new();
    for (i, k) in GENE_KEYS.iter().enumerate() {
        g.insert(k, pick_weighted(&mut rng, WEIGHTS[i]));
    }
    harmonize(&mut g);
    g
}

pub fn genome_key(g: &Genes) -> String {
    GENE_KEYS.iter().map(|k| g.get(k).copied().unwrap_or(0).to_string()).collect::<Vec<_>>().join(".")
}

pub fn clean_genes(v: &serde_json::Value) -> Option<Genes> {
    v.as_object()?;
    let mut g = Genes::new();
    for (i, k) in GENE_KEYS.iter().enumerate() {
        g.insert(k, crate::validate::int(&v[*k], 0, GENE_SIZE[i] - 1)?);
    }
    Some(g)
}

pub fn rarity_of(g: &Genes) -> usize {
    SPECIAL_TIER.get(g.get("sp").copied().unwrap_or(0) as usize).copied().unwrap_or(0)
}

/// Um genoma que ninguém tem ainda (até 60 tentativas com a mesma semente).
pub fn generate_unique(taken: &std::collections::HashSet<String>, seed: &str, tries: u32) -> Option<(u32, Genes)> {
    (0..tries).map(|n| (n, genes_from_seed(seed, n))).find(|(_, g)| !taken.contains(&genome_key(g)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Valores do `lib/pet-genes.js` original (Node) para as mesmas sementes.
    #[test]
    fn matches_node() {
        let mut r = make_rng("teste");
        let first: Vec<f64> = (0..3).map(|_| r()).collect();
        assert_eq!(first, NODE_RNG.to_vec());
        assert_eq!(genome_key(&genes_from_seed("abc123", 0)), NODE_GENOME_0);
        assert_eq!(genome_key(&genes_from_seed("abc123", 7)), NODE_GENOME_7);
    }
    const NODE_RNG: [f64; 3] = [0.03952012094669044, 0.5279629938304424, 0.9655187746975571];
    const NODE_GENOME_0: &str = "4.4.4.1.1.2.1.4.8.1.0.2.4.0";
    const NODE_GENOME_7: &str = "3.1.1.1.0.4.1.5.6.1.0.5.0.0";
}
