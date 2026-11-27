//! Curva de níveis da fazenda e da culinária (`lib/levels.js`): 25·l·(l−1) até o nível 20, depois 1000 por nível, até 999.

pub const MAX_LEVEL: i64 = 999;
pub const SOFT: i64 = 20;
pub const STEP: f64 = 1000.0;
pub const PER_LEVEL: f64 = 0.01;
pub const PER_RANK: f64 = 0.1;

pub fn xp_for(lv: f64) -> f64 {
    let l = lv.floor().clamp(1.0, MAX_LEVEL as f64);
    let s = SOFT as f64;
    if l <= s { 25.0 * l * (l - 1.0) } else { 25.0 * s * (s - 1.0) + STEP * (l - s) }
}

pub fn level_of(xp: f64) -> i64 {
    let x = if xp.is_finite() { xp.max(0.0) } else { 0.0 };
    if x >= xp_for(SOFT as f64) {
        return MAX_LEVEL.min(SOFT + ((x - xp_for(SOFT as f64)) / STEP).floor() as i64);
    }
    let mut lv = 1;
    while lv < SOFT && x >= xp_for((lv + 1) as f64) {
        lv += 1;
    }
    lv
}

pub fn xp_mult(lv: i64, rank: i64) -> f64 {
    (1.0 + PER_LEVEL * (lv - 1) as f64) * (1.0 + PER_RANK * rank as f64)
}

/// (xp, nível, xp do nível, xp do próximo, no máximo?)
pub fn level_info(xp: f64) -> serde_json::Value {
    let lv = level_of(xp);
    let lo = xp_for(lv as f64);
    let hi = if lv >= MAX_LEVEL { lo } else { xp_for((lv + 1) as f64) };
    serde_json::json!({"xp": xp, "lv": lv, "lo": lo, "hi": hi, "max": lv >= MAX_LEVEL})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curve() {
        assert_eq!(xp_for(2.0), 50.0);
        assert_eq!(level_of(49.0), 1);
        assert_eq!(level_of(50.0), 2);
        assert_eq!(level_of(9500.0), 20);
        assert_eq!(level_of(10500.0), 21);
        assert_eq!(level_of(1e12), 999);
    }
}
