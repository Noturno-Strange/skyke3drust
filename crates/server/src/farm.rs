//! Fazenda (`lib/farm.js`): canteiros, sementes, rega, colheita, experiência de fazendeiro, loja (terreno, propriedade e
//! casa), safras (renascer no nível 999), moedas, despensa de ingredientes e emblemas. O servidor decide tudo.

use crate::catalog::Catalog;
use crate::levels::{MAX_LEVEL, level_info, level_of, xp_for, xp_mult};
use crate::persist::{read_json_file, write_json_atomic};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::PathBuf;

const MAX_FARMS: usize = 20000;
const COUNT_MAX: i64 = 1_000_000;
pub const COINS_MAX: i64 = 100_000_000;
const OLD_W: i64 = 6;
const FIRST_BONUS: f64 = 3.0;
const WATER_CUT: f64 = 0.25;
const SAFRA_COINS: f64 = 0.05;
const HIST_MAX: usize = 100;
pub const HOME_BONUS: f64 = 0.1;

#[derive(Debug, Clone, PartialEq)]
pub struct Plant {
    pub c: String,
    pub a: i64,
    pub e: i64,
    pub w: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Best {
    pub c: String,
    pub n: i64,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rec {
    pub x: i64,
    pub s: BTreeMap<i64, Plant>,
    pub d: IndexMap<String, i64>,
    pub q: i64,
    pub h: i64,
    pub w: i64,
    pub p: i64,
    pub b: Option<Best>,
    pub m: i64,
    pub me: i64,
    pub tl: i64,
    pub tp: i64,
    pub th: i64,
    pub sf: i64,
    pub sh: Vec<i64>,
    pub mc: i64,
    pub stock: IndexMap<String, i64>,
}

fn int(v: &Value, top: i64) -> i64 {
    crate::validate::int(v, 0, top).unwrap_or(0)
}

fn safe_int(v: &Value) -> Option<i64> {
    crate::validate::int(v, -(1 << 53) + 1, (1 << 53) - 1)
}

struct Badge {
    id: &'static str,
    n: &'static str,
    i: &'static str,
    t: String,
}

/// Os números que os emblemas olham (`stats`).
struct Stats<'a> {
    r: &'a Rec,
    distinct: usize,
}

fn cat() -> &'static Catalog {
    Catalog::embedded()
}

fn crop(id: &str) -> Option<&'static Value> {
    cat().crops.get(id)
}

fn crops_in(c: &str) -> Vec<String> {
    cat().raw["CROPS"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|x| x["c"] == c)
        .filter_map(|x| x["id"].as_str().map(str::to_owned))
        .collect()
}

fn classic() -> Vec<String> {
    cat().raw["CROPS"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|x| !truthy(&x["x"]))
        .filter_map(|x| x["id"].as_str().map(str::to_owned))
        .collect()
}

fn expansion() -> Vec<String> {
    cat().raw["CROPS"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|x| truthy(&x["x"]))
        .filter_map(|x| x["id"].as_str().map(str::to_owned))
        .collect()
}

fn truthy(v: &Value) -> bool {
    !(v.is_null() || *v == false || *v == 0 || *v == "")
}

fn badges() -> &'static Vec<Badge> {
    use std::sync::OnceLock;
    static B: OnceLock<Vec<Badge>> = OnceLock::new();
    B.get_or_init(|| {
        let b = |id, n, i, t: &str| Badge { id, n, i, t: t.into() };
        vec![
            b("semente", "Primeira Semente", "🌱", "Faça a primeira colheita"),
            b("horta", "Horta Variada", "🥕", "Colha 10 plantas diferentes"),
            b("jardineiro", "Jardineiro", "🌷", "Colha 5 flores diferentes"),
            b("pomar", "Pomar Cheio", "🍎", "Colha 10 frutas diferentes"),
            b("regador", "Regador", "💧", "Regue 50 vezes"),
            b("cesto", "Cesto Cheio", "🧺", "Faça 100 colheitas"),
            b("celeiro", "Celeiro Lotado", "🚜", "Colha 1000 unidades no total"),
            b("sortudo", "Sortudo", "🍀", "Colha um trevo da sorte"),
            b("fazendeiro", "Fazendeiro de Verdade", "🧑‍🌾", "Chegue ao nível 10 de fazendeiro"),
            b("mestre_fazenda", "Mestre da Fazenda", "🏆", &format!("Colha as {} plantas clássicas", classic().len())),
            b("safra1", "Primeira Safra", "🌾", "Chegue ao nível 999 e comece uma nova safra"),
            b("safra5", "Cinco Safras", "🌻", "Complete 5 safras"),
            b("safra10", "Lenda do Campo", "👑", "Complete 10 safras"),
            b("botanico", "Botânico dos Biomas", "🌴", "Colha 20 plantas da expansão"),
            b(
                "colheita_estelar",
                "Colheita Estelar",
                "🌌",
                &format!("Colha as {} plantas da expansão", expansion().len()),
            ),
            b("milionario", "Fazendeiro Milionário", "💰", "Ganhe 1.000.000 de moedas no total"),
        ]
    })
}

fn badge_ok(id: &str, s: &Stats<'_>) -> bool {
    let r = s.r;
    let count_in = |list: Vec<String>| list.iter().filter(|id| r.d.contains_key(*id)).count();
    match id {
        "semente" => r.h >= 1,
        "horta" => s.distinct >= 10,
        "jardineiro" => count_in(crops_in("Flores")) >= 5,
        "pomar" => count_in(crops_in("Frutas")) >= 10,
        "regador" => r.w >= 50,
        "cesto" => r.h >= 100,
        "celeiro" => r.q >= 1000,
        "sortudo" => r.d.contains_key("trevo"),
        "fazendeiro" => level_of(r.x as f64) >= 10 || r.sf > 0,
        "mestre_fazenda" => classic().iter().all(|id| r.d.contains_key(id)),
        "safra1" => r.sf >= 1,
        "safra5" => r.sf >= 5,
        "safra10" => r.sf >= 10,
        "botanico" => count_in(expansion()) >= 20,
        "colheita_estelar" => expansion().iter().all(|id| r.d.contains_key(id)),
        "milionario" => r.me >= 1_000_000,
        _ => false,
    }
}

fn badge_ids(r: &Rec) -> Vec<&'static str> {
    let s = Stats { r, distinct: r.d.len() };
    badges().iter().filter(|b| badge_ok(b.id, &s)).map(|b| b.id).collect()
}

fn tier_list(k: &str) -> &'static Vec<Value> {
    use std::sync::OnceLock;
    static E: OnceLock<Vec<Value>> = OnceLock::new();
    cat().raw[k].as_array().unwrap_or_else(|| E.get_or_init(Vec::new))
}

fn tier_ok(v: &Value, k: &str) -> i64 {
    let n = tier_list(k).len() as i64;
    (n - 1).min(int(v, n - 1)).max(0)
}

/// Canteiros abertos pelo terreno (`farmOpen`).
pub fn farm_open(tl: i64) -> Vec<i64> {
    let land = tier_list("FARM_LAND");
    let i = tl.clamp(0, land.len() as i64 - 1) as usize;
    let p = land.get(i).and_then(|l| l["p"].as_u64()).unwrap_or(0) as usize;
    cat().raw["FARM_ORDER"].as_array().into_iter().flatten().take(p).filter_map(Value::as_i64).collect()
}

fn ing_ok(id: &str) -> bool {
    cat().ing_ids.contains(id)
}

fn crops_of(g: &str) -> Vec<String> {
    cat().raw["CROPS"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|x| x["g"] == g)
        .filter_map(|x| x["id"].as_str().map(str::to_owned))
        .collect()
}

fn slots() -> i64 {
    cat().farm_cells
}

impl Rec {
    fn from_json(r: &Value, v: i64) -> Option<Self> {
        r.as_object()?;
        let mut d = IndexMap::new();
        for (id, n) in r["d"].as_object().into_iter().flatten() {
            if crop(id).is_some() && int(n, COUNT_MAX) > 0 {
                d.insert(id.clone(), int(n, COUNT_MAX));
            }
        }
        let mut old = Vec::new();
        let mut s = BTreeMap::new();
        let mut keys: Vec<(&String, &Value)> = r["s"].as_object().into_iter().flatten().collect();
        keys.sort_by_key(|(k, _)| k.parse::<i64>().unwrap_or(i64::MAX));
        for (k, p) in keys {
            let Ok(i) = k.parse::<i64>() else { continue };
            if i < 0 || i >= (if v == 1 { OLD_W * OLD_W } else { slots() }) || !p.is_object() {
                continue;
            }
            let a = safe_int(&p["a"]).filter(|a| *a > 0).unwrap_or(0);
            let e = safe_int(&p["e"]).filter(|e| *e >= a).unwrap_or(0);
            if let Some(c) = p["c"].as_str().filter(|c| crop(c).is_some())
                && a > 0
                && e > 0
            {
                let pl = Plant { c: c.into(), a, e, w: i64::from(truthy(&p["w"])) };
                if v == 1 {
                    old.push(pl)
                } else {
                    s.insert(i, pl);
                }
            }
        }
        let b = r["b"]["c"].as_str().filter(|c| crop(c).is_some() && int(&r["b"]["n"], COUNT_MAX) > 0).map(|c| Best {
            c: c.into(),
            n: int(&r["b"]["n"], COUNT_MAX),
            at: safe_int(&r["b"]["at"]).unwrap_or(0),
        });
        let x = (xp_for(MAX_LEVEL as f64) as i64).min(int(&r["x"], COUNT_MAX));
        let mut tl = tier_ok(&r["tl"], "FARM_LAND");
        if v == 1 {
            let need = (old.len() as i64).max(36.min(6 + 2 * level_of(x as f64)));
            tl = tier_list("FARM_LAND")
                .iter()
                .position(|l| l["p"].as_i64().unwrap_or(0) >= need)
                .map_or(-1, |p| p as i64);
            s.clear();
            let order: Vec<i64> =
                cat().raw["FARM_ORDER"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect();
            for (k, pl) in old.into_iter().enumerate() {
                if let Some(o) = order.get(k) {
                    s.insert(*o, pl);
                }
            }
        }
        let sf = int(&r["sf"], COUNT_MAX);
        let sh: Vec<i64> = match r["sh"].as_array() {
            Some(a) if sf > 0 => {
                let v: Vec<i64> = a.iter().filter_map(safe_int).filter(|t| *t > 0).collect();
                let keep = (HIST_MAX as i64).min(sf) as usize;
                v[v.len().saturating_sub(keep)..].to_vec()
            }
            _ => Vec::new(),
        };
        let stock: IndexMap<String, i64> = r["stock"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, n)| ing_ok(id) && int(n, COUNT_MAX) > 0)
            .map(|(id, n)| (id.clone(), n.as_i64().unwrap_or(0)))
            .collect();
        let rec = Rec {
            x,
            s,
            d,
            q: int(&r["q"], COUNT_MAX),
            h: int(&r["h"], COUNT_MAX),
            w: int(&r["w"], COUNT_MAX),
            p: int(&r["p"], COUNT_MAX),
            b,
            m: int(&r["m"], COINS_MAX),
            me: int(&r["me"], COINS_MAX),
            tl,
            tp: tier_ok(&r["tp"], "FARM_PROP"),
            th: tier_ok(&r["th"], "FARM_HOUSE"),
            sf,
            sh,
            mc: int(&r["mc"], COINS_MAX),
            stock,
        };
        (rec.x != 0 || rec.p != 0 || rec.m != 0 || rec.sf != 0 || !rec.s.is_empty()).then_some(rec)
    }

    fn to_json(&self) -> Value {
        let s: Map<String, Value> =
            self.s.iter().map(|(k, p)| (k.to_string(), json!({"c": p.c, "a": p.a, "e": p.e, "w": p.w}))).collect();
        json!({
            "x": self.x, "s": s, "d": self.d, "q": self.q, "h": self.h, "w": self.w, "p": self.p,
            "b": self.b.as_ref().map(|b| json!({"c": b.c, "n": b.n, "at": b.at})), "m": self.m, "me": self.me,
            "tl": self.tl, "tp": self.tp, "th": self.th, "sf": self.sf, "sh": self.sh, "mc": self.mc, "stock": self.stock
        })
    }
}

/// Fator de ganho do antifraude: (id, tipo, quantidade, escala) → 0 a 1.
pub type Gain<'a> = &'a mut dyn FnMut(&str, f64, f64) -> f64;

#[derive(Debug)]
pub struct Farm {
    file: Option<PathBuf>,
    players: IndexMap<String, Rec>,
    dirty: bool,
    due: Option<u64>,
}

fn pid_ok(pid: &str) -> bool {
    crate::pubprofile::pid_re(pid)
}

fn now() -> i64 {
    crate::now() as i64
}

impl Farm {
    pub fn new(file: &str) -> Self {
        let mut f = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            players: IndexMap::new(),
            dirty: false,
            due: None,
        };
        if let Some(p) = &f.file {
            match read_json_file(p) {
                Ok(Some(obj)) if obj["v"] == 1 || obj["v"] == 2 => {
                    let v = obj["v"].as_i64().unwrap_or(2);
                    for (pid, r) in obj["p"].as_object().into_iter().flatten() {
                        if f.players.len() >= MAX_FARMS {
                            break;
                        }
                        if pid_ok(pid)
                            && let Some(rec) = Rec::from_json(r, v)
                        {
                            f.players.insert(pid.clone(), rec);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!("fazenda: {e}"),
            }
        }
        f
    }

    fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let p: Map<String, Value> = self.players.iter().map(|(k, r)| (k.clone(), r.to_json())).collect();
        if let Err(e) = write_json_atomic(&f, &json!({"v": 2, "p": p}), false) {
            self.dirty = true;
            tracing::error!("fazenda: {e}");
        }
    }

    fn save(&mut self) {
        if self.file.is_none() {
            return;
        }
        self.dirty = true;
        self.due.get_or_insert(crate::now() + 5000);
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

    fn records(r: &Rec) -> Value {
        let (mut fav, mut fav_n) = (String::new(), 0);
        for (id, n) in &r.d {
            if *n > fav_n {
                fav = id.clone();
                fav_n = *n;
            }
        }
        json!({
            "harvests": r.h, "units": r.q, "kinds": r.d.len(), "total": cat().crops.len(), "water": r.w, "planted": r.p,
            "earned": r.me, "cooked": r.mc, "best": r.b.as_ref().map(|b| json!({"c": b.c, "n": b.n, "at": b.at})),
            "fav": if fav.is_empty() { Value::Null } else { json!({"c": fav, "n": fav_n}) }
        })
    }

    fn plots(r: &Rec) -> Value {
        Value::Array(
            (0..slots())
                .map(|i| r.s.get(&i).map_or(Value::Null, |p| json!({"c": p.c, "a": p.a, "e": p.e, "w": p.w})))
                .collect(),
        )
    }

    fn level(r: &Rec) -> Value {
        let mut o = level_info(r.x as f64);
        o["rank"] = json!(r.sf);
        o["hist"] = json!(r.sh[r.sh.len().saturating_sub(12)..]);
        o["mult"] = json!((xp_mult(level_of(r.x as f64), r.sf) * 100.0).round() / 100.0);
        o["open"] = tier_list("FARM_LAND").get(r.tl.max(0) as usize).map_or(Value::Null, |l| l["p"].clone());
        o["land"] = json!(r.tl);
        o["prop"] = json!(r.tp);
        o["house"] = json!(r.th);
        o
    }

    pub fn view(&self, pid: &str) -> Value {
        let blank = Rec::default();
        let r = self.players.get(pid).unwrap_or(&blank);
        let have = badge_ids(r);
        let mut o = Self::level(r);
        o["coins"] = json!(r.m);
        o["pantry"] = json!(r.stock);
        o["possessions"] = json!(1);
        o["now"] = json!(now());
        o["plots"] = Self::plots(r);
        o["done"] = json!(r.d);
        o["rec"] = Self::records(r);
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
        let have = badge_ids(r);
        let mut o = Self::level(r);
        o["now"] = json!(now());
        o["plots"] = Self::plots(r);
        o["rec"] = Self::records(r);
        o["badges"] = Value::Array(
            badges().iter().filter(|b| have.contains(&b.id)).map(|b| json!({"id": b.id, "n": b.n, "i": b.i})).collect(),
        );
        o
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

    fn own(&mut self, pid: &str) -> Option<&mut Rec> {
        if !self.players.contains_key(pid) {
            if self.players.len() >= MAX_FARMS {
                return None;
            }
            self.players.insert(pid.into(), Rec::default());
        }
        self.players.get_mut(pid)
    }

    fn fail(r: &str) -> Value {
        json!({"ok": false, "reason": r})
    }

    pub fn plant(&mut self, pid: &str, i: i64, c: &str) -> Value {
        if !pid_ok(pid) {
            return Self::fail("pid");
        }
        if i < -1 || i >= slots() {
            return Self::fail("canteiro");
        }
        let Some(cr) = crop(c) else { return Self::fail("semente") };
        let cur = self.players.get(pid);
        let lv = level_of(cur.map_or(0.0, |r| r.x as f64));
        let open = farm_open(cur.map_or(0, |r| r.tl));
        if cr["lv"].as_i64().unwrap_or(1) > lv {
            return Self::fail("nivel");
        }
        if i >= 0 && !open.contains(&i) {
            return Self::fail("fechado");
        }
        let cand = if i >= 0 { vec![i] } else { open };
        let free: Vec<i64> = cand.into_iter().filter(|k| !cur.is_some_and(|r| r.s.contains_key(k))).collect();
        if free.is_empty() {
            return Self::fail(if i >= 0 { "ocupado" } else { "cheio" });
        }
        let t = cr["t"].as_i64().unwrap_or(60);
        let n = now();
        let Some(r) = self.own(pid) else { return Self::fail("full") };
        for k in &free {
            r.s.insert(*k, Plant { c: c.into(), a: n, e: n + t * 1000, w: 0 });
        }
        r.p = COUNT_MAX.min(r.p + free.len() as i64);
        self.save();
        json!({"ok": true, "n": free.len(), "state": self.view(pid)})
    }

    pub fn water(&mut self, pid: &str, i: i64) -> Value {
        if !pid_ok(pid) {
            return Self::fail("pid");
        }
        if i < -1 || i >= slots() {
            return Self::fail("canteiro");
        }
        let n = now();
        let Some(r) = self.players.get_mut(pid) else { return Self::fail("vazio") };
        let list: Vec<i64> = if i >= 0 { vec![i] } else { (0..slots()).collect() };
        let (mut done, mut why) = (0, "vazio");
        for k in list {
            let Some(p) = r.s.get_mut(&k) else { continue };
            if p.e <= n {
                why = "pronta";
                continue;
            }
            if p.w != 0 {
                why = "regada";
                continue;
            }
            p.e = n.max((p.e as f64 - (p.e - n) as f64 * WATER_CUT).round() as i64);
            p.w = 1;
            done += 1;
        }
        if done == 0 {
            return Self::fail(why);
        }
        r.w = COUNT_MAX.min(r.w + done);
        self.save();
        json!({"ok": true, "n": done, "state": self.view(pid)})
    }

    pub fn dig(&mut self, pid: &str, i: i64) -> Value {
        if !pid_ok(pid) {
            return Self::fail("pid");
        }
        if i < 0 || i >= slots() {
            return Self::fail("canteiro");
        }
        let Some(r) = self.players.get_mut(pid).filter(|r| r.s.contains_key(&i)) else { return Self::fail("vazio") };
        r.s.remove(&i);
        self.save();
        json!({"ok": true, "n": 1, "state": self.view(pid)})
    }

    pub fn harvest(&mut self, pid: &str, i: i64, gain: Gain<'_>) -> Value {
        if !pid_ok(pid) {
            return Self::fail("pid");
        }
        if i < -1 || i >= slots() {
            return Self::fail("canteiro");
        }
        let n = now();
        let Some(r) = self.players.get_mut(pid) else { return Self::fail("vazio") };
        let list: Vec<i64> = if i >= 0 { vec![i] } else { (0..slots()).collect() };
        let lv_before = level_of(r.x as f64);
        let badges_before = badge_ids(r);
        let pb = tier_list("FARM_PROP").get(r.tp as usize).and_then(|x| x["b"].as_f64()).unwrap_or(0.0);
        let hb = tier_list("FARM_HOUSE").get(r.th as usize).and_then(|x| x["b"].as_f64()).unwrap_or(0.0);
        let mut got = Vec::new();
        let (mut raw, mut cash, mut why) = (0.0, 0.0, "vazio");
        for k in list {
            let Some(p) = r.s.get(&k).cloned() else { continue };
            if p.e > n {
                why = "crescendo";
                continue;
            }
            let Some(cr) = crop(&p.c) else { continue };
            let y0 = cr["y"][0].as_i64().unwrap_or(1);
            let y1 = cr["y"][1].as_i64().unwrap_or(y0);
            let base = y0 + (crate::game::rnd() * (y1 - y0 + 1) as f64).floor() as i64 + i64::from(p.w != 0);
            let amount = 1.max((base as f64 * (1.0 + pb)).round() as i64);
            let first = !r.d.contains_key(&p.c);
            let xp = cr["xp"].as_f64().unwrap_or(0.0);
            raw += xp * if first { FIRST_BONUS } else { 1.0 } * (1.0 + hb);
            cash += amount as f64 * cr["$"].as_f64().unwrap_or(0.0);
            *r.d.entry(p.c.clone()).or_insert(0) = COUNT_MAX.min(r.d.get(&p.c).copied().unwrap_or(0) + 1);
            if let Some(g) = cr["g"].as_str().filter(|g| ing_ok(g)) {
                let v = r.stock.get(g).copied().unwrap_or(0);
                r.stock.insert(g.into(), COUNT_MAX.min(v + amount));
            }
            r.q = COUNT_MAX.min(r.q + amount);
            r.h = COUNT_MAX.min(r.h + 1);
            let best_xp = |c: &str| crop(c).and_then(|x| x["xp"].as_f64()).unwrap_or(0.0);
            if r.b.as_ref().is_none_or(|b| amount > b.n || (amount == b.n && xp > best_xp(&b.c))) {
                r.b = Some(Best { c: p.c.clone(), n: amount, at: n });
            }
            got.push(json!({"i": k, "c": p.c, "n": amount, "first": first}));
            r.s.remove(&k);
        }
        if got.is_empty() {
            return Self::fail(why);
        }
        let open = tier_list("FARM_LAND").get(r.tl.max(0) as usize).and_then(|l| l["p"].as_f64()).unwrap_or(6.0);
        let f = gain(pid, (raw / (1.0 + hb)).round(), open / 6.0).clamp(0.0, 1.0);
        let xp = (raw * xp_mult(lv_before, r.sf) * f).round() as i64;
        let coins = (cash * (1.0 + SAFRA_COINS * r.sf as f64) * f).round() as i64;
        r.x = (xp_for(MAX_LEVEL as f64) as i64).min(r.x + xp);
        r.m = COINS_MAX.min(r.m + coins);
        r.me = COINS_MAX.min(r.me + coins);
        let lv = level_of(r.x as f64);
        let new_badges: Vec<Value> = badge_ids(r)
            .into_iter()
            .filter(|b| !badges_before.contains(b))
            .filter_map(|id| badges().iter().find(|b| b.id == id))
            .map(|b| json!({"id": b.id, "n": b.n, "i": b.i}))
            .collect();
        self.save();
        json!({
            "ok": true, "n": got.len(), "got": got, "xp": xp, "coins": coins,
            "cut": if f < 1.0 { ((1.0 - f) * 100.0).round() as i64 } else { 0 }, "lv": lv, "lvUp": lv > lv_before,
            "badges": new_badges, "state": self.view(pid)
        })
    }

    pub fn buy(&mut self, pid: &str, kind: &str) -> Value {
        if !pid_ok(pid) {
            return Self::fail("pid");
        }
        let list = match kind {
            "land" => "FARM_LAND",
            "prop" => "FARM_PROP",
            "house" => "FARM_HOUSE",
            _ => return Self::fail("loja"),
        };
        let r = self.players.get(pid);
        let cur = r.map_or(0, |r| match kind {
            "land" => r.tl,
            "prop" => r.tp,
            _ => r.th,
        });
        let Some(next) = tier_list(list).get((cur + 1) as usize) else { return Self::fail("maximo") };
        if level_of(r.map_or(0.0, |r| r.x as f64)) < next["lv"].as_i64().unwrap_or(1) {
            return Self::fail("nivel");
        }
        let price = next["$"].as_i64().unwrap_or(0);
        let Some(r) = self.players.get_mut(pid).filter(|r| r.m >= price) else { return Self::fail("moedas") };
        r.m -= price;
        match kind {
            "land" => r.tl = cur + 1,
            "prop" => r.tp = cur + 1,
            _ => r.th = cur + 1,
        }
        self.save();
        json!({"ok": true, "kind": kind, "tier": cur + 1, "name": next["n"], "state": self.view(pid)})
    }

    pub fn renew(&mut self, pid: &str) -> Value {
        if !pid_ok(pid) {
            return Self::fail("pid");
        }
        let Some(r) = self.players.get_mut(pid).filter(|r| level_of(r.x as f64) >= MAX_LEVEL) else {
            return Self::fail("nivel");
        };
        let before = badge_ids(r);
        r.x = 0;
        r.sf = COUNT_MAX.min(r.sf + 1);
        r.sh.push(now());
        if r.sh.len() > HIST_MAX {
            let cut = r.sh.len() - HIST_MAX;
            r.sh.drain(..cut);
        }
        let rank = r.sf;
        let nb: Vec<Value> = badge_ids(r)
            .into_iter()
            .filter(|b| !before.contains(b))
            .filter_map(|id| badges().iter().find(|b| b.id == id))
            .map(|b| json!({"id": b.id, "n": b.n, "i": b.i}))
            .collect();
        self.save();
        json!({"ok": true, "rank": rank, "badges": nb, "state": self.view(pid)})
    }

    /// Moedas ganhas na culinária: entram na mesma carteira. Devolve o que entrou de fato.
    pub fn earn(&mut self, pid: &str, coins: i64) -> i64 {
        if !pid_ok(pid) || coins <= 0 {
            return 0;
        }
        let Some(r) = self.own(pid) else { return 0 };
        let got = coins.min(COINS_MAX - r.m);
        if got <= 0 {
            return 0;
        }
        r.m += got;
        r.me = COINS_MAX.min(r.me + got);
        r.mc = COINS_MAX.min(r.mc + got);
        self.save();
        got
    }

    /// Quantos ingredientes da receita o jogador já colheu na própria fazenda.
    pub fn grown(&self, pid: &str, ingredients: &[String]) -> usize {
        let Some(r) = self.players.get(pid) else { return 0 };
        ingredients.iter().filter(|g| crops_of(g).iter().any(|id| r.d.contains_key(id))).count()
    }

    pub fn harvested(&self, pid: &str, c: &str) -> bool {
        crop(c).is_some() && self.players.get(pid).is_some_and(|r| r.d.contains_key(c))
    }

    /// `farmdo`: plantar, regar, colher, arrancar, comprar ou renascer.
    pub fn act(&mut self, pid: &str, m: &Value, gain: Gain<'_>) -> Value {
        let i = m["i"].as_i64().unwrap_or(-1);
        match m["k"].as_str().unwrap_or("") {
            "plant" => self.plant(pid, i, m["c"].as_str().unwrap_or("")),
            "water" => self.water(pid, i),
            "harvest" => self.harvest(pid, i, gain),
            "dig" => self.dig(pid, i),
            "buy" => self.buy(pid, m["b"].as_str().unwrap_or("")),
            "renew" => self.renew(pid),
            _ => Self::fail("acao"),
        }
    }

    /// Gasta ingredientes da despensa (culinária e remédios); `false` = falta algum.
    pub fn consume(&mut self, pid: &str, ids: &[String]) -> bool {
        let mut need: IndexMap<&str, i64> = IndexMap::new();
        for id in ids {
            if !ing_ok(id) {
                return false;
            }
            *need.entry(id).or_insert(0) += 1;
        }
        let Some(r) = self.players.get_mut(pid) else { return false };
        if need.iter().any(|(id, n)| r.stock.get(*id).copied().unwrap_or(0) < *n) {
            return false;
        }
        for (id, n) in need {
            let left = r.stock.get(id).copied().unwrap_or(0) - n;
            if left == 0 {
                r.stock.shift_remove(id);
            } else {
                r.stock.insert(id.into(), left);
            }
        }
        self.save();
        true
    }

    /// Compra de um mantimento que não vem da horta (sal, açúcar…).
    pub fn supply(&mut self, pid: &str, id: &str) -> Value {
        if !pid_ok(pid) || !ing_ok(id) || !crops_of(id).is_empty() {
            return Self::fail("ingrediente");
        }
        let price = cat().raw["SUPPLY_PRICE"].as_i64().unwrap_or(4);
        let Some(r) = self.players.get_mut(pid).filter(|r| r.m >= price) else { return Self::fail("moedas") };
        if r.stock.get(id).copied().unwrap_or(0) >= COUNT_MAX {
            return Self::fail("maximo");
        }
        r.m -= price;
        *r.stock.entry(id.into()).or_insert(0) += 1;
        self.save();
        json!({"ok": true, "state": self.view(pid)})
    }

    /// Moedas na carteira (para o console e o painel).
    pub fn coins(&self, pid: &str) -> i64 {
        self.players.get(pid).map_or(0, |r| r.m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plant_water_harvest() {
        let mut f = Farm::new("");
        let pid = crate::game::pid_of("abcdefghijk2");
        assert_eq!(farm_open(0).len(), 6);
        assert_eq!(f.plant(&pid, -1, "tomate")["n"], 6);
        assert_eq!(f.plant(&pid, 0, "tomate")["reason"], "ocupado");
        assert_eq!(f.water(&pid, -1)["n"], 6);
        assert_eq!(f.harvest(&pid, -1, &mut |_, _, _| 1.0)["reason"], "crescendo");
        for p in f.players.get_mut(&pid).unwrap().s.values_mut() {
            p.e = 1;
        }
        let r = f.harvest(&pid, -1, &mut |_, _, _| 1.0);
        assert_eq!(r["n"], 6);
        assert!(r["xp"].as_i64().unwrap() > 0 && r["coins"].as_i64().unwrap() > 0);
        assert!(f.harvested(&pid, "tomate"));
        assert!(f.consume(&pid, &["tomate".into()]));
        let back = Rec::from_json(&f.players[&pid].to_json(), 2).unwrap();
        assert_eq!(back, f.players[&pid]);
    }
}
