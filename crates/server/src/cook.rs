//! Culinária (`lib/cook.js`): o servidor acha a receita pelos ingredientes e o método (panela ou brasa), gasta a despensa
//! da fazenda, dá experiência (mais na primeira vez), moedas e atributos, desbloqueia o alimento e as estrelas (renascer
//! no nível 999).

use crate::catalog::Catalog;
use crate::levels::{MAX_LEVEL, level_info, level_of, xp_for, xp_mult};
use crate::persist::{read_json_file, write_json_atomic};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

const MAX_COOKS: usize = 20000;
const COUNT_MAX: i64 = 1_000_000;
const FIRST_XP: f64 = 10.0;
const REPEAT_XP: f64 = 3.0;
const BONUS_PER_RANK: f64 = 0.02;
const RANK_MAX: i64 = 10;
const COIN_RATE: f64 = 1.0 / 3.0;
const STAR_COINS: f64 = 0.05;
const HIST_MAX: usize = 100;

fn cat() -> &'static Catalog {
    Catalog::embedded()
}

fn rank_of(n: i64) -> i64 {
    RANK_MAX.min((n as f64).sqrt().floor() as i64)
}

const ATTRS: [(&str, &str, &str); 3] = [
    ("fogo", "Fogo", "Receitas feitas na fogueira"),
    ("paladar", "Paladar", "Receitas diferentes que você já fez"),
    ("despensa", "Despensa", "Ingredientes diferentes que você já usou"),
];

fn recipes_with(b: Option<i64>) -> Vec<String> {
    cat().raw["RECIPES"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| match b {
            None => r["b"].is_null() || r["b"] == 0 || r["b"] == false,
            Some(x) => r["b"] == x,
        })
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect()
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rec {
    pub x: i64,
    pub d: IndexMap<String, i64>,
    pub cozinhar: i64,
    pub assar: i64,
    pub u: IndexSet<String>,
    pub st: i64,
    pub sh: Vec<i64>,
}

impl Rec {
    fn distinct(&self) -> usize {
        self.d.len()
    }
    fn total(&self) -> i64 {
        self.cozinhar + self.assar
    }
}

struct Badge {
    id: &'static str,
    n: &'static str,
    i: &'static str,
    t: String,
}

fn badges() -> &'static Vec<Badge> {
    use std::sync::OnceLock;
    static B: OnceLock<Vec<Badge>> = OnceLock::new();
    B.get_or_init(|| {
        let b = |id, n, i, t: &str| Badge { id, n, i, t: t.into() };
        vec![
            b("aprendiz", "Aprendiz de Cozinha", "🥄", "Prepare a primeira receita"),
            b("cozinheiro", "Cozinheiro de Fogueira", "🍳", "Prepare 5 receitas diferentes"),
            b("mestre_panela", "Mestre da Panela", "🍲", "Cozinhe 10 vezes na panela"),
            b("mestre_braseiro", "Mestre do Braseiro", "🔥", "Asse 10 vezes nas brasas"),
            b("chef", "Chef", "🧑‍🍳", "Prepare 20 receitas diferentes"),
            b(
                "mestre_cuca",
                "Mestre-cuca",
                "🏆",
                &format!("Prepare as {} receitas clássicas", recipes_with(None).len()),
            ),
            b(
                "sabores_brasil",
                "Sabores do Brasil",
                "🌎",
                &format!("Prepare as {} receitas dos biomas", recipes_with(Some(1)).len()),
            ),
            b("estrela1", "Primeira Estrela", "⭐", "Chegue ao nível 999 e ganhe uma estrela"),
            b("estrela5", "Cinco Estrelas", "🌟", "Ganhe 5 estrelas"),
            b("estrela10", "Constelação", "✨", "Ganhe 10 estrelas"),
            b("gourmet", "Gourmet", "🍽️", "Prepare 60 receitas diferentes"),
            b(
                "banquete_estelar",
                "Banquete Estelar",
                "🌠",
                &format!("Prepare as {} receitas da expansão", recipes_with(Some(2)).len()),
            ),
        ]
    })
}

fn badge_ok(id: &str, r: &Rec) -> bool {
    match id {
        "aprendiz" => r.distinct() >= 1,
        "cozinheiro" => r.distinct() >= 5,
        "mestre_panela" => r.cozinhar >= 10,
        "mestre_braseiro" => r.assar >= 10,
        "chef" => r.distinct() >= 20,
        "mestre_cuca" => recipes_with(None).iter().all(|x| r.d.contains_key(x)),
        "sabores_brasil" => recipes_with(Some(1)).iter().all(|x| r.d.contains_key(x)),
        "estrela1" => r.st >= 1,
        "estrela5" => r.st >= 5,
        "estrela10" => r.st >= 10,
        "gourmet" => r.distinct() >= 60,
        "banquete_estelar" => recipes_with(Some(2)).iter().all(|x| r.d.contains_key(x)),
        _ => false,
    }
}

fn badge_ids(r: &Rec) -> Vec<&'static str> {
    badges().iter().filter(|b| badge_ok(b.id, r)).map(|b| b.id).collect()
}

fn new_badges(before: &[&str], r: &Rec) -> Value {
    Value::Array(
        badges()
            .iter()
            .filter(|b| badge_ok(b.id, r) && !before.contains(&b.id))
            .map(|b| json!({"id": b.id, "n": b.n, "i": b.i}))
            .collect(),
    )
}

fn attrs(r: &Rec) -> Vec<(&'static str, &'static str, &'static str, i64, i64)> {
    ATTRS
        .iter()
        .map(|(id, n, t)| {
            let v = match *id {
                "fogo" => r.total(),
                "paladar" => r.distinct() as i64,
                _ => r.u.len() as i64,
            };
            (*id, *n, *t, v, rank_of(v))
        })
        .collect()
}

/// Chave da receita: método e ingredientes em ordem (`recipeKey`).
pub fn recipe_key(m: &str, g: &[String]) -> String {
    let mut g = g.to_vec();
    g.sort();
    format!("{m}:{}", g.join(","))
}

fn recipe_by_key(key: &str) -> Option<&'static Value> {
    use std::sync::OnceLock;
    static M: OnceLock<std::collections::HashMap<String, &'static Value>> = OnceLock::new();
    M.get_or_init(|| {
        cat().raw["RECIPES"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|r| {
                let g: Vec<String> =
                    r["g"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
                (recipe_key(r["m"].as_str().unwrap_or(""), &g), r)
            })
            .collect()
    })
    .get(key)
    .copied()
}

fn int(v: &Value) -> i64 {
    crate::validate::int(v, 0, COUNT_MAX).unwrap_or(0)
}

/// Gasto da despensa (a fazenda): devolve `false` quando falta ingrediente.
pub type Consume<'a> = &'a mut dyn FnMut(&str, &[String]) -> bool;

#[derive(Debug)]
pub struct Cook {
    file: Option<PathBuf>,
    players: IndexMap<String, Rec>,
    dirty: bool,
    due: Option<u64>,
}

fn fail(r: &str) -> Value {
    json!({"ok": false, "reason": r})
}

impl Cook {
    pub fn new(file: &str) -> Self {
        let mut c = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            players: IndexMap::new(),
            dirty: false,
            due: None,
        };
        if let Some(f) = &c.file {
            match read_json_file(f) {
                Ok(Some(obj)) if obj["v"] == 1 => {
                    for (pid, r) in obj["p"].as_object().into_iter().flatten() {
                        if c.players.len() >= MAX_COOKS {
                            break;
                        }
                        if crate::pubprofile::pid_re(pid)
                            && let Some(rec) = Self::clean(r)
                        {
                            c.players.insert(pid.clone(), rec);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!("culinária: {e}"),
            }
        }
        c
    }

    fn clean(r: &Value) -> Option<Rec> {
        r.as_object()?;
        let d: IndexMap<String, i64> = r["d"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, n)| cat().recipes.contains_key(*id) && int(n) > 0)
            .map(|(id, n)| (id.clone(), int(n)))
            .collect();
        let u: IndexSet<String> = r["u"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter(|x| cat().ing_ids.contains(*x))
            .map(str::to_owned)
            .collect();
        let x = (xp_for(MAX_LEVEL as f64) as i64).min(int(&r["x"]));
        let st = int(&r["st"]);
        let sh: Vec<i64> = match r["sh"].as_array() {
            Some(a) if st > 0 => {
                let v: Vec<i64> = a.iter().filter_map(|t| crate::validate::int(t, 1, (1 << 53) - 1)).collect();
                let keep = (HIST_MAX as i64).min(st) as usize;
                v[v.len().saturating_sub(keep)..].to_vec()
            }
            _ => Vec::new(),
        };
        let rec = Rec { x, d, cozinhar: int(&r["k"]["cozinhar"]), assar: int(&r["k"]["assar"]), u, st, sh };
        (!rec.d.is_empty() || rec.x != 0 || rec.st != 0).then_some(rec)
    }

    fn to_json(r: &Rec) -> Value {
        json!({"x": r.x, "d": r.d, "k": {"cozinhar": r.cozinhar, "assar": r.assar}, "u": r.u, "st": r.st, "sh": r.sh})
    }

    fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let p: Map<String, Value> = self.players.iter().map(|(k, r)| (k.clone(), Self::to_json(r))).collect();
        if let Err(e) = write_json_atomic(&f, &json!({"v": 1, "p": p}), false) {
            self.dirty = true;
            tracing::error!("culinária: {e}");
        }
    }

    fn save(&mut self) {
        if self.file.is_some() {
            self.dirty = true;
            self.due.get_or_insert(crate::now() + 5000);
        }
    }

    pub fn tick(&mut self, now: u64) {
        if self.due.is_some_and(|d| now >= d) {
            self.due = None;
            self.write();
        }
    }

    pub fn flush(&mut self) {
        self.due = None;
        self.write();
    }

    pub fn has(&self, pid: &str) -> bool {
        self.players.contains_key(pid)
    }

    pub fn len(&self) -> usize {
        self.players.len()
    }

    pub fn is_empty(&self) -> bool {
        self.players.is_empty()
    }

    pub fn view(&self, pid: &str) -> Value {
        let blank = Rec::default();
        let r = self.players.get(pid).unwrap_or(&blank);
        let have = badge_ids(r);
        let mut o = level_info(r.x as f64);
        o["rank"] = json!(r.st);
        o["hist"] = json!(r.sh[r.sh.len().saturating_sub(12)..]);
        o["mult"] = json!((xp_mult(level_of(r.x as f64), r.st) * 100.0).round() / 100.0);
        o["done"] = json!(r.d);
        o["total"] = json!(r.total());
        o["attrs"] = Value::Array(
            attrs(r).into_iter().map(|(id, n, t, v, rk)| json!({"id": id, "n": n, "t": t, "v": v, "r": rk})).collect(),
        );
        o["badges"] = Value::Array(
            badges()
                .iter()
                .map(|b| json!({"id": b.id, "n": b.n, "i": b.i, "t": b.t, "on": have.contains(&b.id)}))
                .collect(),
        );
        o
    }

    pub fn public_of(&self, pid: &str) -> Value {
        let blank = Rec::default();
        let r = self.players.get(pid).unwrap_or(&blank);
        let lv = level_of(r.x as f64);
        json!({
            "lv": lv, "max": lv >= MAX_LEVEL, "rank": r.st, "hist": r.sh[r.sh.len().saturating_sub(12)..],
            "kinds": r.distinct(), "total": cat().recipes.len()
        })
    }

    /// O alimento da receita já foi preparado (libera o item na mochila)?
    pub fn unlocked(&self, pid: &str, item: &str) -> bool {
        self.players.get(pid).is_some_and(|r| r.d.contains_key(item))
    }

    pub fn badges_of(&self, pid: &str) -> Vec<Value> {
        let blank = Rec::default();
        let have = badge_ids(self.players.get(pid).unwrap_or(&blank));
        badges()
            .iter()
            .filter(|b| have.contains(&b.id))
            .map(|b| json!({"id": b.id, "name": b.n, "icon": b.i}))
            .collect()
    }

    pub fn cook(
        &mut self,
        pid: &str,
        method: &str,
        ids: &[String],
        gain: crate::farm::Gain<'_>,
        consume: Consume<'_>,
    ) -> Value {
        if !crate::pubprofile::pid_re(pid) {
            return fail("pid");
        }
        if !cat().cook_methods.contains(method) {
            return fail("metodo");
        }
        let uniq: std::collections::HashSet<&String> = ids.iter().collect();
        if ids.len() < 2
            || ids.len() > cat().cook_slots
            || uniq.len() != ids.len()
            || !ids.iter().all(|id| cat().ing_ids.contains(id))
        {
            return fail("ingredientes");
        }
        let Some(recipe) = recipe_by_key(&recipe_key(method, ids)) else { return fail("nada") };
        if !self.players.contains_key(pid) && self.players.len() >= MAX_COOKS {
            return fail("full");
        }
        let g: Vec<String> =
            recipe["g"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
        if !consume(pid, &g) {
            return fail("despensa");
        }
        let rid = recipe["id"].as_str().unwrap_or("").to_owned();
        let mut r = self.players.get(pid).cloned().unwrap_or_default();
        let lv_before = level_of(r.x as f64);
        let ranks_before: Vec<i64> = attrs(&r).iter().map(|a| a.4).collect();
        let badges_before = badge_ids(&r);
        let first = !r.d.contains_key(&rid);
        let bonus = 1.0 + BONUS_PER_RANK * ranks_before.iter().sum::<i64>() as f64;
        let raw = ((if first { FIRST_XP } else { REPEAT_XP }) * g.len() as f64 * bonus).round();
        let f = gain(pid, raw, 1.0).clamp(0.0, 1.0);
        let got = (raw * xp_mult(lv_before, r.st) * f).round() as i64;
        let coins = (raw * COIN_RATE * (1.0 + STAR_COINS * r.st as f64) * f).round() as i64;
        let n = r.d.get(&rid).copied().unwrap_or(0);
        r.d.insert(rid.clone(), COUNT_MAX.min(n + 1));
        if method == "assar" {
            r.assar = COUNT_MAX.min(r.assar + 1);
        } else {
            r.cozinhar = COUNT_MAX.min(r.cozinhar + 1);
        }
        for id in &g {
            r.u.insert(id.clone());
        }
        r.x = (xp_for(MAX_LEVEL as f64) as i64).min(r.x + got);
        let attrs_after = attrs(&r);
        let lv = level_of(r.x as f64);
        let up: Vec<&str> =
            attrs_after.iter().zip(&ranks_before).filter(|(a, b)| a.4 > **b).map(|(a, _)| a.1).collect();
        let nb = new_badges(&badges_before, &r);
        self.players.insert(pid.into(), r);
        self.save();
        json!({
            "ok": true, "recipe": rid, "first": first, "xp": got, "coins": coins, "g": g,
            "cut": if f < 1.0 { ((1.0 - f) * 100.0).round() as i64 } else { 0 }, "lv": lv, "lvUp": lv > lv_before, "up": up,
            "badges": nb, "state": self.view(pid)
        })
    }

    pub fn renew(&mut self, pid: &str) -> Value {
        if !crate::pubprofile::pid_re(pid) {
            return fail("pid");
        }
        let Some(r) = self.players.get_mut(pid).filter(|r| level_of(r.x as f64) >= MAX_LEVEL) else {
            return fail("nivel");
        };
        let before = badge_ids(r);
        r.x = 0;
        r.st = COUNT_MAX.min(r.st + 1);
        r.sh.push(crate::now() as i64);
        if r.sh.len() > HIST_MAX {
            let cut = r.sh.len() - HIST_MAX;
            r.sh.drain(..cut);
        }
        let rank = r.st;
        let nb = new_badges(&before, r);
        self.save();
        json!({"ok": true, "rank": rank, "badges": nb, "state": self.view(pid)})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooks_a_recipe() {
        let mut c = Cook::new("");
        let pid = crate::game::pid_of("abcdefghijk2");
        let g: Vec<String> = ["cenoura", "batata", "cebola", "sal"].iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(c.cook(&pid, "cozinhar", &g, &mut |_, _, _| 1.0, &mut |_, _| false)["reason"], "despensa");
        let r = c.cook(&pid, "cozinhar", &g, &mut |_, _, _| 1.0, &mut |_, _| true);
        assert_eq!(
            (r["recipe"].as_str(), r["first"].as_bool(), r["xp"].as_i64()),
            (Some("sopa_legumes"), Some(true), Some(40))
        );
        assert!(c.unlocked(&pid, "sopa_legumes"));
        assert_eq!(c.cook(&pid, "assar", &g, &mut |_, _, _| 1.0, &mut |_, _| true)["reason"], "nada");
    }
}
