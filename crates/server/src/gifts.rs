//! Presentes (`lib/gifts.js`): a caixa de presentes é uma coleção para mostrar (nada vai para a mochila nem vira moeda).
//! Só vale enviar o que o próprio jogador colheu ou preparou, até 5 por dia e 1 por amigo por dia.

use crate::catalog::Catalog;
use crate::persist::{read_json_file, write_json_atomic};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};
use std::path::PathBuf;

const DAY: i64 = 86_400_000;
const MAX_BOXES: usize = 20000;
const BOX_MAX: usize = 200;
const PER_DAY: usize = 5;
const PER_PAIR: usize = 1;

fn pid_ok(s: &str) -> bool {
    crate::pubprofile::pid_re(s)
}

/// `c` = planta, `r` = receita.
pub fn item_ok(kind: &str, item: &str) -> bool {
    let c = Catalog::embedded();
    match kind {
        "c" => c.crops.contains_key(item),
        "r" => c.recipes.contains_key(item),
        _ => false,
    }
}

fn clean_name(n: &str) -> String {
    let t: String = n
        .chars()
        .filter(|c| {
            let u = *c as u32;
            !(u <= 0x1f
                || (0x7f..=0x9f).contains(&u)
                || (0x200b..=0x200f).contains(&u)
                || (0x2028..=0x202e).contains(&u)
                || (0x2060..=0x206f).contains(&u)
                || u == 0xfeff
                || *c == '<'
                || *c == '>')
        })
        .collect();
    crate::validate::slice16(t.trim(), Catalog::embedded().max_name)
}

#[derive(Debug, Clone)]
struct Gift {
    id: i64,
    f: String,
    n: String,
    k: String,
    i: String,
    at: i64,
}

#[derive(Debug, Clone, Default)]
struct Box_ {
    g: Vec<Gift>,
    nx: i64,
    u: usize,
}

#[derive(Debug)]
pub struct Gifts {
    file: Option<PathBuf>,
    boxes: IndexMap<String, Box_>,
    sent: IndexMap<String, Vec<(i64, String)>>,
    dirty: bool,
    due: Option<u64>,
    sent_n: u64,
    refused: u64,
}

fn now() -> i64 {
    crate::now() as i64
}

impl Gifts {
    pub fn new(file: &str) -> Self {
        let mut s = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            boxes: IndexMap::new(),
            sent: IndexMap::new(),
            dirty: false,
            due: None,
            sent_n: 0,
            refused: 0,
        };
        if let Some(f) = &s.file {
            match read_json_file(f) {
                Ok(Some(obj)) if obj["v"] == 1 => {
                    let t = now();
                    for (pid, b) in obj["b"].as_object().into_iter().flatten() {
                        if s.boxes.len() >= MAX_BOXES {
                            break;
                        }
                        let Some(bx) = pid_ok(pid).then(|| Self::clean(b)).flatten() else { continue };
                        for x in &bx.g {
                            if t - x.at < DAY {
                                s.sent.entry(x.f.clone()).or_default().push((x.at, pid.clone()));
                            }
                        }
                        s.boxes.insert(pid.clone(), bx);
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!("presentes: {e}"),
            }
        }
        s
    }

    fn clean(b: &Value) -> Option<Box_> {
        let list = b["g"].as_array()?;
        let mut g = Vec::new();
        let mut top = 0;
        for x in &list[list.len().saturating_sub(BOX_MAX)..] {
            let (Some(f), Some(k), Some(i)) = (x["f"].as_str(), x["k"].as_str(), x["i"].as_str()) else { continue };
            if !pid_ok(f) || !item_ok(k, i) {
                continue;
            }
            let Some(at) = crate::validate::int(&x["at"], 1, (1 << 53) - 1) else { continue };
            let id = crate::validate::int(&x["id"], top + 1, (1 << 53) - 1).unwrap_or(top + 1);
            top = id;
            let n = clean_name(x["n"].as_str().unwrap_or(""));
            g.push(Gift {
                id,
                f: f.into(),
                n: if n.is_empty() { "Jogador".into() } else { n },
                k: k.into(),
                i: i.into(),
                at,
            });
        }
        if g.is_empty() {
            return None;
        }
        let nx = crate::validate::int(&b["nx"], top + 1, (1 << 53) - 1).unwrap_or(top + 1);
        let u = crate::validate::int(&b["u"], 0, i64::MAX).map_or(0, |u| (u as usize).min(g.len()));
        Some(Box_ { g, nx, u })
    }

    fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let b: Map<String, Value> = self
            .boxes
            .iter()
            .map(|(k, bx)| {
                let g: Vec<Value> =
                    bx.g.iter()
                        .map(|x| json!({"id": x.id, "f": x.f, "n": x.n, "k": x.k, "i": x.i, "at": x.at}))
                        .collect();
                (k.clone(), json!({"g": g, "nx": bx.nx, "u": bx.u}))
            })
            .collect();
        if let Err(e) = write_json_atomic(&f, &json!({"v": 1, "b": b}), false) {
            self.dirty = true;
            tracing::error!("presentes: {e}");
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

    fn recent(&mut self, from: &str) -> Vec<(i64, String)> {
        let t = now();
        let keep: Vec<(i64, String)> =
            self.sent.get(from).map(|l| l.iter().filter(|x| t - x.0 < DAY).cloned().collect()).unwrap_or_default();
        if keep.is_empty() {
            self.sent.shift_remove(from);
        } else {
            self.sent.insert(from.into(), keep.clone());
        }
        keep
    }

    /// Quantos ainda pode mandar hoje e quando libera o próximo.
    pub fn quota(&mut self, from: &str) -> Value {
        let l = self.recent(from);
        let left = PER_DAY.saturating_sub(l.len());
        let next = if left > 0 { 0 } else { l.iter().map(|x| x.0).min().unwrap_or(0) + DAY };
        json!({"left": left, "perDay": PER_DAY, "next": next})
    }

    /// `can(de, tipo, item)`: o servidor confere que quem envia produziu o item.
    pub fn send(&mut self, from: &str, to: &str, kind: &str, item: &str, name: &str, can: bool) -> Value {
        let mut no = |s: &mut Self, reason: &str, wait: Option<i64>| {
            s.refused += 1;
            let mut o = json!({"ok": false, "reason": reason});
            if let Some(w) = wait {
                o["wait"] = json!(w);
            }
            o
        };
        if !pid_ok(from) || !pid_ok(to) {
            return no(self, "pid", None);
        }
        if from == to {
            return no(self, "self", None);
        }
        if !item_ok(kind, item) {
            return no(self, "item", None);
        }
        if !can {
            return no(self, "nao_tem", None);
        }
        let l = self.recent(from);
        let t = now();
        let wait = |v: &[i64]| ((v.iter().min().copied().unwrap_or(t) + DAY - t) as f64 / 1000.0).ceil() as i64;
        if l.len() >= PER_DAY {
            let v: Vec<i64> = l.iter().map(|x| x.0).collect();
            return no(self, "dia", Some(wait(&v)));
        }
        let pair: Vec<i64> = l.iter().filter(|x| x.1 == to).map(|x| x.0).collect();
        if pair.len() >= PER_PAIR {
            return no(self, "par", Some(wait(&pair)));
        }
        if !self.boxes.contains_key(to) && self.boxes.len() >= MAX_BOXES {
            return no(self, "full", None);
        }
        let bx = self.boxes.entry(to.into()).or_insert_with(|| Box_ { nx: 1, ..Default::default() });
        if bx.g.len() >= BOX_MAX {
            return no(self, "cheia", None);
        }
        let n = clean_name(name);
        let gift = Gift {
            id: bx.nx,
            f: from.into(),
            n: if n.is_empty() { "Jogador".into() } else { n },
            k: kind.into(),
            i: item.into(),
            at: t,
        };
        bx.nx += 1;
        bx.g.push(gift.clone());
        bx.u = bx.g.len().min(bx.u + 1);
        self.sent.entry(from.into()).or_default().push((t, to.into()));
        self.sent_n += 1;
        self.save();
        json!({"ok": true, "gift": {"id": gift.id, "n": gift.n, "k": kind, "i": item, "at": t}, "left": PER_DAY.saturating_sub(l.len() + 1)})
    }

    pub fn box_of(&self, pid: &str) -> Value {
        let b = self.boxes.get(pid);
        json!({
            "total": b.map_or(0, |b| b.g.len()), "max": BOX_MAX, "unseen": b.map_or(0, |b| b.u),
            "list": b.map(|b| b.g.iter().rev().map(|x| json!({"id": x.id, "n": x.n, "k": x.k, "i": x.i, "at": x.at})).collect::<Vec<_>>()).unwrap_or_default()
        })
    }

    pub fn seen(&mut self, pid: &str) {
        if let Some(b) = self.boxes.get_mut(pid).filter(|b| b.u > 0) {
            b.u = 0;
            self.save();
        }
    }

    pub fn unseen(&self, pid: &str) -> usize {
        self.boxes.get(pid).map_or(0, |b| b.u)
    }

    pub fn public_of(&self, pid: &str) -> Value {
        let Some(b) = self.boxes.get(pid) else { return json!({"total": 0, "list": []}) };
        let list: Vec<Value> = b.g[b.g.len().saturating_sub(60)..]
            .iter()
            .rev()
            .map(|x| json!({"n": x.n, "k": x.k, "i": x.i, "at": x.at}))
            .collect();
        json!({"total": b.g.len(), "list": list})
    }

    pub fn remove(&mut self, pid: &str, id: i64) -> bool {
        let Some(b) = self.boxes.get_mut(pid) else { return false };
        let Some(k) = b.g.iter().position(|x| x.id == id) else { return false };
        b.g.remove(k);
        b.u = b.u.min(b.g.len());
        if b.g.is_empty() {
            self.boxes.shift_remove(pid);
        }
        self.save();
        true
    }

    pub fn stats(&self) -> Value {
        json!({"boxes": self.boxes.len(), "sent": self.sent_n, "refused": self.refused})
    }
}
