//! Mascote (`lib/pet-care.js` e `lib/pets.js`): adoção com genoma único, necessidades que caem com o tempo (também
//! offline, até 96 h), cuidados com tempo de espera, vínculo, conquistas, acessórios, visitas de quem vê o perfil
//! público e a liberação (com 30 dias para adotar outro).

use crate::persist::{read_json_file, write_json_atomic};
use crate::pet_genes::{self as G, Genes};
use crate::playerdata::clean_text;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const MIN: f64 = 60_000.0;
const HOUR: f64 = 3_600_000.0;
const DAY: f64 = 86_400_000.0;
pub const NEEDS: [&str; 6] = ["food", "energy", "joy", "clean", "health", "fun"];
const MAX_OFFLINE_H: f64 = 96.0;
const BOND_PER_DAY: f64 = 12.0;
pub const ACTION_IDS: [&str; 6] = ["feed", "play", "rest", "clean", "pet", "special"];
const STAGES: [(i64, &str); 4] = [(0, "Filhote"), (3, "Jovem"), (10, "Crescido"), (30, "Veterano")];
const BOND_LEVELS: [&str; 6] = ["Conhecidos", "Colegas", "Amigos", "Companheiros", "Inseparáveis", "Almas gêmeas"];
const COSMETICS: usize = 5;
const COSMETIC_UNLOCK: [(i64, &str); 4] =
    [(1, "primeira-refeicao"), (2, "uma-semana"), (3, "um-mes"), (4, "explorador")];
const ACHIEVEMENTS: [(&str, &str); 12] = [
    ("adotado", "Primeiro dia juntos"),
    ("nasceu-especial", "Nasceu especial"),
    ("primeira-refeicao", "Primeira refeição"),
    ("brincalhao", "Brincalhão (10 brincadeiras)"),
    ("limpinho", "Limpinho (10 banhos)"),
    ("soneca", "Soneca em dia (5 descansos)"),
    ("explorador", "Primeira aventura"),
    ("cem-carinhos", "100 cuidados"),
    ("amigos", "Amigos de verdade (vínculo 20)"),
    ("inseparaveis", "Inseparáveis (vínculo 80)"),
    ("uma-semana", "Uma semana de vida"),
    ("um-mes", "Um mês de vida"),
];
const HISTORY_MAX: usize = 20;
const NAME_MAX: usize = 16;
const MAX_PETS: usize = 20000;
const RELEASE_LOCK: f64 = 30.0 * DAY;
const RID_KEEP: usize = 16;
const SOCIAL_COOLDOWN: f64 = 6.0 * HOUR;
const SOCIAL_DAY_CAP: i64 = 200;

fn decay(k: &str) -> f64 {
    match k {
        "food" => 3.0,
        "energy" => 2.0,
        "joy" => 1.5,
        "clean" => 1.2,
        "fun" => 2.5,
        _ => 0.0,
    }
}
fn floor(k: &str) -> f64 {
    match k {
        "food" | "energy" => 8.0,
        "joy" => 15.0,
        "clean" | "fun" => 10.0,
        "health" => 30.0,
        _ => 0.0,
    }
}
/// Personalidade: necessidades que caem mais rápido ou mais devagar.
fn pers_mul(pe: i64, k: &str) -> f64 {
    match (pe, k) {
        (0, "fun") => 1.2,
        (1, "energy") => 1.3,
        (2, "fun") => 1.25,
        (2, "energy") => 1.1,
        (3, "joy") => 0.8,
        (3, "fun") => 0.9,
        (4, "food") => 1.3,
        (5, "joy") => 1.1,
        (5, "energy") => 0.9,
        _ => 1.0,
    }
}

struct Action {
    cd: f64,
    need: Option<(&'static str, bool, f64, &'static str)>,
    d: &'static [(&'static str, f64)],
    bond: f64,
}

fn action(a: &str) -> Option<Action> {
    Some(match a {
        "feed" => Action {
            cd: 15.0 * MIN,
            need: Some(("food", true, 95.0, "full")),
            d: &[("food", 30.0), ("joy", 3.0)],
            bond: 0.5,
        },
        "play" => Action {
            cd: 15.0 * MIN,
            need: Some(("energy", false, 15.0, "tired")),
            d: &[("fun", 30.0), ("joy", 6.0), ("energy", -10.0), ("food", -4.0)],
            bond: 0.5,
        },
        "rest" => {
            Action { cd: 30.0 * MIN, need: Some(("energy", true, 90.0, "rested")), d: &[("energy", 40.0)], bond: 0.5 }
        }
        "clean" => Action {
            cd: 20.0 * MIN,
            need: Some(("clean", true, 90.0, "clean")),
            d: &[("clean", 40.0), ("health", 3.0)],
            bond: 0.5,
        },
        "pet" => Action { cd: 10.0 * MIN, need: None, d: &[("joy", 8.0)], bond: 1.0 },
        "special" => Action {
            cd: 4.0 * HOUR,
            need: Some(("energy", false, 30.0, "tired")),
            d: &[("joy", 10.0), ("fun", 10.0), ("energy", -8.0)],
            bond: 3.0,
        },
        _ => return None,
    })
}

#[derive(Debug, Clone)]
pub struct Pet {
    sd: String,
    n: i64,
    g: Genes,
    nm: String,
    c: f64,
    t: f64,
    a: IndexMap<&'static str, f64>,
    cd: IndexMap<String, f64>,
    ct: IndexMap<&'static str, i64>,
    hs: Vec<(String, f64)>,
    un: IndexMap<String, f64>,
    cos: i64,
    bd: (f64, f64),
}

fn ct_keys() -> [&'static str; 7] {
    ["feed", "play", "rest", "clean", "pet", "special", "total"]
}

fn age_days(p: &Pet, now: f64) -> i64 {
    ((now - p.c) / DAY).floor().max(0.0) as i64
}

fn stage_of(days: i64) -> (usize, &'static str) {
    let mut idx = 0;
    for (i, s) in STAGES.iter().enumerate() {
        if days >= s.0 {
            idx = i;
        }
    }
    (idx, STAGES[idx].1)
}

fn bond_level(b: f64) -> usize {
    ((b / 20.0).floor() as usize).min(BOND_LEVELS.len() - 1)
}

fn ach_ok(id: &str, p: &Pet, age: i64) -> bool {
    let ct = |k: &str| p.ct.get(k).copied().unwrap_or(0);
    match id {
        "adotado" => true,
        "nasceu-especial" => G::rarity_of(&p.g) >= 2,
        "primeira-refeicao" => ct("feed") >= 1,
        "brincalhao" => ct("play") >= 10,
        "limpinho" => ct("clean") >= 10,
        "soneca" => ct("rest") >= 5,
        "explorador" => ct("special") >= 1,
        "cem-carinhos" => ct("total") >= 100,
        "amigos" => p.a["bond"] >= 20.0,
        "inseparaveis" => p.a["bond"] >= 80.0,
        "uma-semana" => age >= 7,
        "um-mes" => age >= 30,
        _ => false,
    }
}

fn unlock(p: &mut Pet, now: f64) -> Vec<String> {
    let age = age_days(p, now);
    let mut fresh = Vec::new();
    for (id, _) in ACHIEVEMENTS {
        if !p.un.contains_key(id) && ach_ok(id, p, age) {
            let mark = match id {
                "uma-semana" => Some(7.0),
                "um-mes" => Some(30.0),
                _ => None,
            };
            p.un.insert(id.into(), mark.map_or(now, |d| p.c + d * DAY));
            fresh.push(id.to_owned());
        }
    }
    fresh
}

fn settle(p: &mut Pet, now: f64) {
    if now <= p.t {
        return;
    }
    let h = ((now - p.t) / HOUR).min(MAX_OFFLINE_H);
    let pe = p.g.get("pe").copied().unwrap_or(0);
    for k in ["food", "energy", "joy", "clean", "fun"] {
        let mut rate = decay(k) * pers_mul(pe, k);
        if k == "joy" && p.a["fun"] < 25.0 {
            rate += 0.8;
        }
        let v = (p.a[k] - rate * h).max(floor(k));
        p.a.insert(k, v);
    }
    let bad = p.a["food"] < 25.0 || p.a["clean"] < 25.0 || p.a["energy"] < 15.0;
    let hv = if bad { (p.a["health"] - 0.8 * h).max(floor("health")) } else { (p.a["health"] + 0.6 * h).min(100.0) };
    p.a.insert("health", hv);
    p.t = now;
}

fn unlocked_cosmetics(p: &Pet) -> Vec<i64> {
    let mut out = vec![0];
    let ac = p.g.get("ac").copied().unwrap_or(0);
    if ac != 0 {
        out.push(ac);
    }
    for (id, ach) in COSMETIC_UNLOCK {
        if p.un.contains_key(ach) && !out.contains(&id) {
            out.push(id);
        }
    }
    out.sort_unstable();
    out
}

fn mood(p: &Pet) -> Value {
    let a = &p.a;
    let (state, text) = if a["energy"] < 15.0 {
        ("sleep", "Está com sono.")
    } else if a["food"] < 25.0 {
        ("sad", "Está com fome.")
    } else if a["clean"] < 25.0 {
        ("sad", "Quer um banho.")
    } else if a["joy"] < 30.0 || a["fun"] < 20.0 {
        ("sad", "Quer companhia e diversão.")
    } else if a["joy"] >= 75.0 && a["fun"] >= 50.0 {
        ("happy", "Está muito feliz!")
    } else {
        ("idle", "Está tranquilo.")
    };
    json!({"state": state, "text": text})
}

fn needs_attention(p: &Pet) -> bool {
    let a = &p.a;
    a["food"] < 25.0 || a["clean"] < 25.0 || a["energy"] < 15.0 || a["joy"] < 30.0 || a["fun"] < 20.0
}

fn ach_name(id: &str) -> String {
    ACHIEVEMENTS.iter().find(|a| a.0 == id).map_or_else(|| id.to_owned(), |a| a.1.to_owned())
}

fn genes_json(g: &Genes) -> Value {
    Value::Object(g.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect())
}

fn owner_view(p: &Pet, now: f64) -> Value {
    let cd: Map<String, Value> = ACTION_IDS
        .iter()
        .filter_map(|k| p.cd.get(*k).filter(|u| **u > now).map(|u| ((*k).to_owned(), json!(u - now))))
        .collect();
    let days = age_days(p, now);
    let (si, sn) = stage_of(days);
    let g = |k: &str| p.g.get(k).copied().unwrap_or(0) as usize;
    json!({
        "name": p.nm, "genes": genes_json(&p.g), "cos": p.cos, "cosOk": unlocked_cosmetics(p), "created": p.c, "age": days,
        "stage": si, "stageName": sn, "pers": G::PERSONALITIES[g("pe")], "special": G::SPECIALS[g("sp")],
        "rarity": G::RARITIES[G::rarity_of(&p.g)],
        "a": p.a.iter().map(|(k, v)| ((*k).to_owned(), json!(v.round() as i64))).collect::<Map<String, Value>>(),
        "cd": cd, "mood": mood(p), "attention": needs_attention(p),
        "stats": p.ct.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect::<Map<String, Value>>(),
        "hist": p.hs[p.hs.len().saturating_sub(10)..].iter().map(|(a, t)| json!([a, t])).collect::<Vec<_>>(),
        "ach": p.un.iter().map(|(id, t)| json!({"id": id, "n": ach_name(id), "t": t})).collect::<Vec<_>>(),
        "next": ACHIEVEMENTS.iter().filter(|a| !p.un.contains_key(a.0)).map(|a| json!({"id": a.0, "n": a.1})).collect::<Vec<_>>()
    })
}

fn public_view(p: &Pet, now: f64) -> Value {
    let days = age_days(p, now);
    let (si, sn) = stage_of(days);
    let g = |k: &str| p.g.get(k).copied().unwrap_or(0) as usize;
    let mut ach: Vec<(&String, &f64)> = p.un.iter().collect();
    ach.sort_by(|a, b| a.1.total_cmp(b.1));
    let ct = |k: &str| p.ct.get(k).copied().unwrap_or(0);
    let bl = bond_level(p.a["bond"]);
    json!({
        "name": p.nm, "genes": genes_json(&p.g), "cos": p.cos, "created": p.c, "age": days, "stage": si, "stageName": sn,
        "pers": G::PERSONALITIES[g("pe")], "special": G::SPECIALS[g("sp")], "rarity": G::RARITIES[G::rarity_of(&p.g)],
        "bond": bl, "bondName": BOND_LEVELS[bl],
        "ach": ach.into_iter().map(|(id, t)| json!({"id": id, "n": ach_name(id), "t": t})).collect::<Vec<_>>(),
        "stats": {"total": ct("total"), "feed": ct("feed"), "play": ct("play"), "clean": ct("clean"), "rest": ct("rest"), "special": ct("special")}
    })
}

fn new_pet(seed: &str, attempt: u32, genes: Genes, name: &str, now: f64) -> Pet {
    let a: IndexMap<&'static str, f64> = [
        ("food", 70.0),
        ("energy", 80.0),
        ("joy", 70.0),
        ("clean", 80.0),
        ("health", 100.0),
        ("fun", 60.0),
        ("bond", 0.0),
    ]
    .into_iter()
    .collect();
    let cos = genes.get("ac").copied().unwrap_or(0);
    let mut p = Pet {
        sd: seed.into(),
        n: i64::from(attempt),
        g: genes,
        nm: name.into(),
        c: now,
        t: now,
        a,
        cd: IndexMap::new(),
        ct: ct_keys().into_iter().map(|k| (k, 0)).collect(),
        hs: vec![("adotado".into(), now)],
        un: IndexMap::new(),
        cos,
        bd: ((now / DAY).floor(), 0.0),
    };
    unlock(&mut p, now);
    p
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

/// Um cuidado (`act` de `pet-care.js`).
fn care(p: &mut Pet, a: &str, now: f64) -> Value {
    let Some(act) = action(a) else { return json!({"ok": false, "reason": "action"}) };
    settle(p, now);
    let until = p.cd.get(a).copied().unwrap_or(0.0);
    if until > now {
        return json!({"ok": false, "reason": "cooldown", "wait": until - now});
    }
    if let Some((k, le, v, why)) = act.need {
        let cur = p.a[k];
        if if le { cur > v } else { cur < v } {
            return json!({"ok": false, "reason": why});
        }
    }
    for (k, dv) in act.d {
        let v = (p.a[k] + dv).clamp(0.0, 100.0).max(floor(k));
        p.a.insert(k, v);
    }
    let day = (now / DAY).floor();
    if p.bd.0 != day {
        p.bd = (day, 0.0);
    }
    let gain = act.bond.min(BOND_PER_DAY - p.bd.1).max(0.0);
    p.bd.1 += gain;
    let b = round1((p.a["bond"] + gain).min(100.0));
    p.a.insert("bond", b);
    p.cd.insert(a.into(), now + act.cd);
    if let Some((k, _)) = ct_keys().iter().map(|k| (*k, ())).find(|(k, _)| *k == a) {
        *p.ct.entry(k).or_insert(0) += 1;
    }
    *p.ct.entry("total").or_insert(0) += 1;
    p.hs.push((a.into(), now));
    while p.hs.len() > HISTORY_MAX {
        p.hs.remove(0);
    }
    json!({"ok": true, "gain": gain, "fresh": unlock(p, now)})
}

#[derive(Debug, Default)]
struct Social {
    counts: IndexMap<&'static str, i64>,
    by: HashMap<String, f64>,
    day: f64,
    n: i64,
}

#[derive(Debug)]
pub struct Pets {
    file: Option<PathBuf>,
    pets: IndexMap<String, Pet>,
    genomes: HashSet<String>,
    released: IndexMap<String, f64>,
    recent: HashMap<String, IndexMap<String, Value>>,
    social: HashMap<String, Social>,
    dirty: bool,
    due: Option<u64>,
}

fn now() -> f64 {
    crate::now() as f64
}

fn fin(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

impl Pets {
    pub fn new(file: &str) -> Self {
        let mut s = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            pets: IndexMap::new(),
            genomes: HashSet::new(),
            released: IndexMap::new(),
            recent: HashMap::new(),
            social: HashMap::new(),
            dirty: false,
            due: None,
        };
        s.load();
        s
    }

    fn clean_record(e: &Value, t: f64) -> Option<Pet> {
        e.as_object()?;
        let g = G::clean_genes(&e["g"])?;
        let sd = e["sd"]
            .as_str()
            .filter(|s| s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))?;
        let n = crate::validate::int(&e["n"], 0, 200)?;
        let c = fin(&e["c"]).filter(|c| *c <= t + DAY)?;
        let mut a = IndexMap::new();
        for k in NEEDS.iter().chain(std::iter::once(&"bond")) {
            a.insert(*k, fin(&e["a"][*k]).unwrap_or(50.0).clamp(0.0, 100.0));
        }
        let cd: IndexMap<String, f64> = ACTION_IDS
            .iter()
            .filter_map(|k| fin(&e["cd"][*k]).filter(|v| *v > t && *v < t + 5.0 * HOUR).map(|v| ((*k).to_owned(), v)))
            .collect();
        let ct = ct_keys()
            .into_iter()
            .map(|k| (k, crate::validate::int(&e["ct"][k], 0, 999_999_999).unwrap_or(0)))
            .collect();
        let hs: Vec<(String, f64)> = e["hs"]
            .as_array()
            .map(|a| {
                a[a.len().saturating_sub(20)..]
                    .iter()
                    .filter_map(|h| {
                        let s = h[0].as_str()?;
                        ((s == "adotado" || ACTION_IDS.contains(&s)) && fin(&h[1]).is_some())
                            .then(|| (s.to_owned(), h[1].as_f64().unwrap_or(0.0)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let un: IndexMap<String, f64> = e["un"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(id, ts)| ACHIEVEMENTS.iter().any(|x| x.0 == id.as_str()) && fin(ts).is_some())
            .map(|(id, ts)| (id.clone(), ts.as_f64().unwrap_or(0.0)))
            .collect();
        let bd = match (fin(&e["bd"]["d"]), fin(&e["bd"]["g"])) {
            (Some(d), Some(g)) => (d, g.clamp(0.0, BOND_PER_DAY)),
            _ => (0.0, 0.0),
        };
        let nm = clean_text(e["nm"].as_str().unwrap_or(""), NAME_MAX);
        Some(Pet {
            sd: sd.into(),
            n,
            g,
            nm: if nm.is_empty() { "Mascote".into() } else { nm },
            c,
            t: fin(&e["t"]).unwrap_or(c).min(t),
            a,
            cd,
            ct,
            hs,
            un,
            cos: crate::validate::int(&e["cos"], 0, COSMETICS as i64 - 1).unwrap_or(0),
            bd,
        })
    }

    fn load(&mut self) {
        let Some(f) = self.file.clone() else { return };
        let obj = match read_json_file(&f) {
            Ok(o) => o,
            Err(_) => {
                let mut bak = f.as_os_str().to_owned();
                bak.push(".bak");
                let _ = std::fs::rename(&f, PathBuf::from(bak));
                tracing::warn!("mascotes: arquivo inválido, começando limpo (o antigo ficou em .bak).");
                None
            }
        };
        let Some(obj) = obj.filter(|o| o["v"] == 1) else { return };
        let t = now();
        for (pid, e) in obj["pets"].as_object().into_iter().flatten() {
            if self.pets.len() >= MAX_PETS || !crate::pubprofile::pid_re(pid) {
                continue;
            }
            let Some(r) = Self::clean_record(e, t) else { continue };
            if !self.genomes.insert(G::genome_key(&r.g)) {
                continue;
            }
            self.pets.insert(pid.clone(), r);
        }
        for (pid, ts) in obj["rel"].as_object().into_iter().flatten() {
            if let Some(ts) = fin(ts).filter(|ts| crate::pubprofile::pid_re(pid) && t - ts < RELEASE_LOCK) {
                self.released.insert(pid.clone(), ts);
            }
        }
    }

    fn to_json(p: &Pet) -> Value {
        json!({
            "sd": p.sd, "n": p.n, "g": genes_json(&p.g), "nm": p.nm, "c": p.c, "t": p.t,
            "a": p.a.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect::<Map<String, Value>>(),
            "cd": p.cd, "ct": p.ct.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect::<Map<String, Value>>(),
            "hs": p.hs.iter().map(|(a, t)| json!([a, t])).collect::<Vec<_>>(), "un": p.un, "cos": p.cos,
            "bd": {"d": p.bd.0, "g": p.bd.1}
        })
    }

    fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let out: Map<String, Value> = self.pets.iter().map(|(k, p)| (k.clone(), Self::to_json(p))).collect();
        let rel: Map<String, Value> = self.released.iter().map(|(k, t)| (k.clone(), json!(t))).collect();
        if let Err(e) = write_json_atomic(&f, &json!({"v": 1, "savedAt": crate::now(), "pets": out, "rel": rel}), false)
        {
            self.dirty = true;
            tracing::error!("mascotes: {e}");
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

    pub fn adopt(&mut self, pid: &str, name: &str) -> Value {
        if !crate::pubprofile::pid_re(pid) {
            return json!({"ok": false, "reason": "invalid"});
        }
        if self.pets.contains_key(pid) {
            return json!({"ok": false, "reason": "exists"});
        }
        let t = now();
        if let Some(lock) = self.released.get(pid).filter(|l| t - **l < RELEASE_LOCK) {
            return json!({"ok": false, "reason": "locked", "wait": RELEASE_LOCK - (t - lock)});
        }
        let nm = clean_text(name, NAME_MAX);
        if nm.is_empty() || crate::pubprofile::has_link(&nm) {
            return json!({"ok": false, "reason": "name"});
        }
        if self.pets.len() >= MAX_PETS {
            return json!({"ok": false, "reason": "full"});
        }
        let seed = crate::hex(&crate::random_bytes::<8>());
        let Some((attempt, genes)) = G::generate_unique(&self.genomes, &seed, 60) else {
            return json!({"ok": false, "reason": "busy"});
        };
        self.genomes.insert(G::genome_key(&genes));
        let p = new_pet(&seed, attempt, genes, &nm, t);
        let view = owner_view(&p, t);
        self.pets.insert(pid.into(), p);
        self.released.shift_remove(pid);
        self.save();
        json!({"ok": true, "pet": view})
    }

    pub fn view(&mut self, pid: &str) -> Value {
        let t = now();
        let Some(p) = self.pets.get_mut(pid) else { return Value::Null };
        settle(p, t);
        unlock(p, t);
        owner_view(p, t)
    }

    pub fn act(&mut self, pid: &str, a: &str, rid: &str) -> Value {
        if !self.pets.contains_key(pid) {
            return json!({"ok": false, "reason": "none"});
        }
        if !(6..=24).contains(&rid.len()) || !rid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            return json!({"ok": false, "reason": "rid"});
        }
        if let Some(prev) = self.recent.get(pid).and_then(|m| m.get(rid)) {
            let mut o = prev.clone();
            o["dup"] = json!(true);
            return o;
        }
        let t = now();
        let Some(p) = self.pets.get_mut(pid) else { return json!({"ok": false, "reason": "none"}) };
        let r = care(p, a, t);
        let ok = r["ok"] == true;
        let out = if ok {
            json!({"ok": true, "gain": r["gain"], "fresh": r["fresh"], "action": a, "pet": owner_view(p, t)})
        } else {
            let mut o = r.clone();
            o["pet"] = owner_view(p, t);
            o
        };
        if ok {
            self.save();
        }
        let memo = self.recent.entry(pid.into()).or_default();
        memo.insert(rid.into(), out.clone());
        while memo.len() > RID_KEEP {
            memo.shift_remove_index(0);
        }
        out
    }

    pub fn dress(&mut self, pid: &str, id: i64) -> Value {
        let t = now();
        let Some(p) = self.pets.get_mut(pid) else { return json!({"ok": false, "reason": "none"}) };
        if !unlocked_cosmetics(p).contains(&id) {
            return json!({"ok": false, "reason": "locked"});
        }
        p.cos = id;
        settle(p, t);
        let v = owner_view(p, t);
        self.save();
        json!({"ok": true, "pet": v})
    }

    pub fn release(&mut self, pid: &str) -> Value {
        let Some(p) = self.pets.shift_remove(pid) else { return json!({"ok": false, "reason": "none"}) };
        self.genomes.remove(&G::genome_key(&p.g));
        self.recent.remove(pid);
        self.social.remove(pid);
        self.released.insert(pid.into(), now());
        self.save();
        json!({"ok": true})
    }

    pub fn public_of(&mut self, pid: &str) -> Value {
        let t = now();
        let Some(p) = self.pets.get_mut(pid) else { return Value::Null };
        unlock(p, t);
        let mut v = public_view(p, t);
        v["social"] = match self.social.get(pid) {
            Some(s) => json!(s.counts),
            None => json!({"wave": 0, "hug": 0, "gift": 0}),
        };
        v
    }

    /// Visita de quem vê o perfil público: aceno, abraço ou presente (contadores só na memória).
    pub fn visit(&mut self, pid: &str, voter: &str, kind: &str) -> Value {
        if !crate::pubprofile::pid_re(pid) || !crate::pubprofile::pid_re(voter) {
            return json!({"ok": false, "reason": "invalid"});
        }
        let Some(k) = ["wave", "hug", "gift"].into_iter().find(|x| *x == kind) else {
            return json!({"ok": false, "reason": "kind"});
        };
        if pid == voter {
            return json!({"ok": false, "reason": "self"});
        }
        if !self.pets.contains_key(pid) {
            return json!({"ok": false, "reason": "none"});
        }
        let t = now();
        let s = self.social.entry(pid.into()).or_insert_with(|| Social {
            counts: [("wave", 0), ("hug", 0), ("gift", 0)].into_iter().collect(),
            ..Default::default()
        });
        let day = (t / DAY).floor();
        if s.day != day {
            s.day = day;
            s.n = 0;
        }
        if s.n >= SOCIAL_DAY_CAP {
            return json!({"ok": false, "reason": "busy"});
        }
        let key = format!("{voter}:{k}");
        let last = s.by.get(&key).copied().unwrap_or(0.0);
        if t - last < SOCIAL_COOLDOWN {
            return json!({"ok": false, "reason": "cooldown", "wait": SOCIAL_COOLDOWN - (t - last)});
        }
        s.by.insert(key, t);
        *s.counts.entry(k).or_insert(0) += 1;
        s.n += 1;
        json!({"ok": true, "counts": s.counts})
    }

    pub fn sweep(&mut self) {
        let t = now();
        self.social.retain(|_, s| {
            s.by.retain(|_, ts| t - *ts < SOCIAL_COOLDOWN);
            !s.by.is_empty() || t - s.day * DAY < DAY
        });
        self.released.retain(|_, ts| t - *ts < RELEASE_LOCK);
    }

    pub fn len(&self) -> usize {
        self.pets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pets.is_empty()
    }

    pub fn genome_count(&self) -> usize {
        self.genomes.len()
    }
}

/// Conjunto usado só para não repetir genomas.
pub type Genomes = IndexSet<String>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adopt_and_care() {
        let mut p = Pets::new("");
        let pid = crate::game::pid_of("abcdefghijk2");
        let r = p.adopt(&pid, "Tico");
        assert_eq!(r["ok"], true);
        assert_eq!(r["pet"]["a"]["food"], 70);
        assert_eq!(p.adopt(&pid, "Teco")["reason"], "exists");
        let a = p.act(&pid, "feed", "abc123");
        assert_eq!((a["ok"].as_bool(), a["gain"].as_f64()), (Some(true), Some(0.5)));
        assert_eq!(p.act(&pid, "feed", "abc123")["dup"], true);
        assert_eq!(p.act(&pid, "feed", "abc124")["reason"], "cooldown");
        assert!(p.public_of(&pid)["ach"].as_array().unwrap().len() >= 2);
        assert_eq!(p.release(&pid)["ok"], true);
        assert_eq!(p.adopt(&pid, "Tico")["reason"], "locked");
    }
}
