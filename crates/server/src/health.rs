//! Medicina lúdica (`lib/health.js`): depois de 30 min ativos o jogador fica indisposto (uma doença de fantasia) e só um
//! amigo cura, com um remédio feito com a despensa da fazenda. Quem cura ganha experiência de cuidador.

use crate::catalog::Catalog;
use crate::levels::{MAX_LEVEL, level_info, xp_for};
use crate::persist::{read_json_file, write_json_atomic};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::path::PathBuf;

pub const ACTIVE_MS: i64 = 30 * 60000;
const STOCK_MAX: i64 = 99;
const MAX: usize = 20000;

#[derive(Debug, Clone, Default)]
pub struct Rec {
    pub ill: String,
    pub active: i64,
    pub immune: i64,
    pub meds: IndexMap<String, i64>,
    pub xp: i64,
    pub helped: i64,
}

fn illnesses() -> Vec<String> {
    Catalog::embedded().raw["MEDICINE"]["ILLNESSES"]
        .as_object()
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default()
}

fn med(id: &str) -> Option<&'static Value> {
    Catalog::embedded().medicine.get(id)
}

fn number(v: &Value, cap: i64) -> i64 {
    crate::validate::int(v, 0, (1 << 53) - 1).map_or(0, |n| n.min(cap))
}

#[derive(Debug)]
pub struct Health {
    file: Option<PathBuf>,
    players: IndexMap<String, Rec>,
    dirty: bool,
    due: Option<u64>,
}

fn now() -> i64 {
    crate::now() as i64
}

impl Health {
    pub fn new(file: &str) -> Self {
        let mut h = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            players: IndexMap::new(),
            dirty: false,
            due: None,
        };
        if let Some(f) = &h.file {
            match read_json_file(f) {
                Ok(Some(d)) if d["v"] == 1 => {
                    let ills = illnesses();
                    for (pid, r) in d["p"].as_object().into_iter().flatten() {
                        if h.players.len() >= MAX {
                            break;
                        }
                        if !crate::pubprofile::pid_re(pid) || !r.is_object() {
                            continue;
                        }
                        let meds = r["meds"]
                            .as_object()
                            .into_iter()
                            .flatten()
                            .filter(|(id, n)| med(id).is_some() && number(n, STOCK_MAX) > 0)
                            .map(|(id, n)| (id.clone(), number(n, STOCK_MAX)))
                            .collect();
                        let ill = r["ill"].as_str().filter(|i| ills.iter().any(|x| x == i)).unwrap_or("").to_owned();
                        h.players.insert(
                            pid.clone(),
                            Rec {
                                ill,
                                active: number(&r["active"], ACTIVE_MS),
                                immune: number(&r["immune"], now() + ACTIVE_MS),
                                meds,
                                xp: number(&r["xp"], xp_for(MAX_LEVEL as f64) as i64),
                                helped: number(&r["helped"], 1_000_000),
                            },
                        );
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!("saúde: {e}"),
            }
        }
        h
    }

    pub fn flush(&mut self) {
        self.due = None;
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        let p: Map<String, Value> = self
            .players
            .iter()
            .map(|(k, r)| (k.clone(), json!({"ill": r.ill, "active": r.active, "immune": r.immune, "meds": r.meds, "xp": r.xp, "helped": r.helped})))
            .collect();
        match write_json_atomic(&f, &json!({"v": 1, "p": p}), false) {
            Ok(()) => self.dirty = false,
            Err(e) => tracing::error!("saúde: {e}"),
        }
    }

    fn save(&mut self) {
        self.dirty = true;
        if self.file.is_some() {
            self.due.get_or_insert(crate::now() + 1000);
        }
    }

    pub fn tick_file(&mut self, now: u64) {
        if self.due.is_some_and(|d| now >= d) {
            self.flush();
        }
    }

    fn own(&mut self, pid: &str) -> Option<&mut Rec> {
        if !crate::pubprofile::pid_re(pid) {
            return None;
        }
        if !self.players.contains_key(pid) {
            if self.players.len() >= MAX {
                return None;
            }
            self.players.insert(pid.into(), Rec::default());
        }
        self.players.get_mut(pid)
    }

    pub fn public_of(&self, pid: &str) -> Value {
        let r = self.players.get(pid);
        let ill = r.map_or("", |r| r.ill.as_str());
        json!({"ill": ill, "tired": !ill.is_empty()})
    }

    pub fn view(&self, pid: &str) -> Value {
        let r = self.players.get(pid);
        let mut v = level_info(r.map_or(0.0, |r| r.xp as f64));
        let p = self.public_of(pid);
        v["ill"] = p["ill"].clone();
        v["tired"] = p["tired"].clone();
        v["meds"] = json!(r.map(|r| r.meds.clone()).unwrap_or_default());
        v["helped"] = json!(r.map_or(0, |r| r.helped));
        v["now"] = json!(now());
        v
    }

    /// Mais `elapsed_ms` de jogo ativo; `true` quando a pessoa ficou doente agora.
    pub fn tick(&mut self, pid: &str, elapsed_ms: i64) -> bool {
        let ills = illnesses();
        let t = now();
        let Some(r) = self.own(pid) else { return false };
        let was = r.ill.clone();
        if r.ill.is_empty() && t >= r.immune {
            r.active = ACTIVE_MS.min(r.active + elapsed_ms.clamp(0, 10000));
            if r.active >= ACTIVE_MS && !ills.is_empty() {
                let i = ((crate::game::rnd().max(0.0) * ills.len() as f64).floor() as usize).min(ills.len() - 1);
                r.ill = ills[i].clone();
                r.active = 0;
            }
            let changed = was != r.ill;
            self.save();
            return changed;
        }
        false
    }

    pub fn craft(&mut self, pid: &str, id: &str, consume: &mut dyn FnMut(&str, &[String]) -> bool) -> Value {
        let Some(m) = med(id) else { return json!({"ok": false, "reason": "receita"}) };
        let g: Vec<String> =
            m["g"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
        let Some(r) = self.own(pid) else { return json!({"ok": false, "reason": "full"}) };
        if r.meds.get(id).copied().unwrap_or(0) >= STOCK_MAX {
            return json!({"ok": false, "reason": "cheio"});
        }
        if !consume(pid, &g) {
            return json!({"ok": false, "reason": "despensa"});
        }
        let Some(r) = self.players.get_mut(pid) else { return json!({"ok": false, "reason": "full"}) };
        *r.meds.entry(id.into()).or_insert(0) += 1;
        self.save();
        json!({"ok": true, "state": self.view(pid)})
    }

    pub fn heal(&mut self, healer: &str, patient: &str, id: &str) -> Value {
        if healer == patient {
            return json!({"ok": false, "reason": "autocura"});
        }
        if med(id).is_none() {
            return json!({"ok": false, "reason": "receita"});
        }
        if !self.players.get(patient).is_some_and(|p| !p.ill.is_empty()) {
            return json!({"ok": false, "reason": "saudavel"});
        }
        if !self.players.get(healer).is_some_and(|r| r.meds.get(id).copied().unwrap_or(0) > 0) {
            return json!({"ok": false, "reason": "remedio"});
        }
        let t = now();
        if let Some(r) = self.players.get_mut(healer) {
            let left = r.meds.get(id).copied().unwrap_or(0) - 1;
            if left > 0 {
                r.meds.insert(id.into(), left);
            } else {
                r.meds.shift_remove(id);
            }
            r.helped = 1_000_000.min(r.helped + 1);
            r.xp = (xp_for(MAX_LEVEL as f64) as i64).min(r.xp + 20);
        }
        if let Some(p) = self.players.get_mut(patient) {
            p.ill.clear();
            p.active = 0;
            p.immune = t + ACTIVE_MS;
        }
        self.save();
        json!({"ok": true, "state": self.view(healer), "patient": self.public_of(patient)})
    }
}
