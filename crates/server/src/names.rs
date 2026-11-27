//! Nomes reservados por dono (`lib/names.js`): o dono (HMAC da chave do aparelho ou do id de identidade) continua dono
//! offline e depois de reiniciar, sem guardar IP. `NAMES_FILE=0` desliga a reserva.

use crate::persist::{read_json_file, write_json_atomic};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};
use sha2::Sha256;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone)]
struct Entry {
    o: String,
    oi: String,
    at: f64,
}

#[derive(Debug)]
pub struct NameRegistry {
    file: Option<PathBuf>,
    max: usize,
    per_key: usize,
    names: IndexMap<String, Entry>,
    by_owner: HashMap<String, IndexSet<String>>,
    salt: String,
    dirty: bool,
    due: Option<u64>,
    pub saved_at: u64,
}

impl NameRegistry {
    pub fn new(file: &str, max: usize, per_key: usize) -> Self {
        let mut r = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            max,
            per_key,
            names: IndexMap::new(),
            by_owner: HashMap::new(),
            salt: String::new(),
            dirty: false,
            due: None,
            saved_at: 0,
        };
        r.load();
        r
    }

    pub fn enabled(&self) -> bool {
        self.file.is_some()
    }

    fn mac(&self, pre: &str, v: &str) -> String {
        let Ok(mut m) = Hmac::<Sha256>::new_from_slice(self.salt.as_bytes()) else { return String::new() };
        m.update(pre.as_bytes());
        m.update(v.as_bytes());
        URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
    }

    /// Dono pela chave do aparelho (clientes antigos).
    pub fn owner(&self, key: &str) -> String {
        self.mac("k|", key)
    }

    /// Dono pelo id de identidade.
    pub fn owner_id(&self, id: &str) -> String {
        self.mac("i|", id)
    }

    fn bind(&mut self, name: &str, e: Entry) {
        for k in [&e.o, &e.oi] {
            if !k.is_empty() {
                self.by_owner.entry(k.clone()).or_default().insert(name.to_owned());
            }
        }
        self.names.insert(name.to_owned(), e);
    }

    fn unbind(&mut self, name: &str) {
        let Some(e) = self.names.shift_remove(name) else { return };
        for k in [&e.o, &e.oi] {
            if let Some(set) = self.by_owner.get_mut(k) {
                set.shift_remove(name);
                if set.is_empty() {
                    self.by_owner.remove(k);
                }
            }
        }
    }

    fn trim_owner(&mut self, oid: &str) {
        if self.per_key < 1 {
            return;
        }
        loop {
            let Some(set) = self.by_owner.get(oid) else { return };
            if set.len() <= self.per_key {
                return;
            }
            let old = set
                .iter()
                .filter_map(|n| self.names.get(n).map(|e| (n.clone(), e.at)))
                .fold((String::new(), f64::INFINITY), |acc, (n, at)| if at < acc.1 { (n, at) } else { acc })
                .0;
            if old.is_empty() {
                return;
            }
            self.unbind(&old);
        }
    }

    fn trim_global(&mut self) {
        while self.max > 0 && self.names.len() > self.max {
            let Some(old) = self.names.keys().next().cloned() else { break };
            self.unbind(&old);
        }
    }

    fn touch(&mut self, name: &str) {
        if let Some(mut e) = self.names.shift_remove(name) {
            e.at = crate::now() as f64;
            self.names.insert(name.to_owned(), e);
        }
    }

    fn load(&mut self) {
        if let Some(f) = self.file.clone() {
            match read_json_file(&f) {
                Ok(Some(obj)) if obj["v"] == 1 => {
                    if let Some(s) = obj["salt"].as_str().filter(|s| s.len() >= 16) {
                        self.salt = s.to_owned();
                    }
                    if let Some(src) = obj["names"].as_object() {
                        for (name, e) in src {
                            if name.is_empty() || crate::validate::len16(name) > 64 {
                                continue;
                            }
                            if self.max > 0 && self.names.len() >= self.max {
                                break;
                            }
                            let s = |k: &str| e[k].as_str().filter(|x| x.len() <= 128).unwrap_or("").to_owned();
                            let (o, oi) = (s("o"), s("oi"));
                            let Some(at) = e["at"].as_f64().filter(|x| x.is_finite()) else { continue };
                            if o.is_empty() && oi.is_empty() {
                                continue;
                            }
                            self.bind(name, Entry { o, oi, at });
                        }
                        self.trim_global();
                        let owners: Vec<String> = self.by_owner.keys().cloned().collect();
                        for o in owners {
                            self.trim_owner(&o);
                        }
                    }
                }
                Ok(_) => {}
                Err(_) => {
                    let mut bak = f.as_os_str().to_owned();
                    bak.push(".bak");
                    let _ = std::fs::rename(&f, PathBuf::from(bak));
                    tracing::warn!("nomes: arquivo inválido, começando limpo.");
                }
            }
        }
        if self.salt.is_empty() {
            self.salt = URL_SAFE_NO_PAD.encode(crate::random_bytes::<24>());
        }
    }

    pub fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        self.saved_at = crate::now();
        let out: Map<String, Value> = self
            .names
            .iter()
            .map(|(n, e)| {
                let mut o = json!({"o": e.o, "at": e.at});
                if !e.oi.is_empty() {
                    o["oi"] = json!(e.oi);
                }
                (n.clone(), o)
            })
            .collect();
        if let Err(e) =
            write_json_atomic(&f, &json!({"v": 1, "salt": self.salt, "savedAt": crate::now(), "names": out}), false)
        {
            self.dirty = true;
            tracing::error!("nomes: {e}");
        }
    }

    fn save(&mut self) {
        if self.file.is_none() {
            return;
        }
        self.dirty = true;
        if self.due.is_none() {
            self.due = Some(crate::now() + 2000);
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
        if self.dirty {
            self.write();
        }
    }

    pub fn free(&self, name: &str, oid: &str) -> bool {
        if !self.enabled() {
            return true;
        }
        self.names.get(name).is_none_or(|e| e.o == oid || e.oi == oid)
    }

    /// O nome está livre para esta pessoa (identidade `id` e, se houver, chave antiga `fk`)?
    pub fn free_for(&self, name: &str, id: &str, fk: &str) -> bool {
        if !self.enabled() {
            return true;
        }
        let Some(e) = self.names.get(name) else { return true };
        e.oi == self.owner_id(id) || (!fk.is_empty() && e.o == self.owner(fk))
    }

    pub fn claim(&mut self, name: &str, oid: &str) -> bool {
        if !self.enabled() {
            return true;
        }
        if let Some(e) = self.names.get(name) {
            if e.o != oid && e.oi != oid {
                return false;
            }
            self.touch(name);
            self.save();
            return true;
        }
        self.bind(name, Entry { o: oid.into(), oi: String::new(), at: crate::now() as f64 });
        self.trim_owner(oid);
        self.trim_global();
        self.save();
        true
    }

    pub fn claim_for(&mut self, name: &str, id: &str, fk: &str) -> bool {
        if !self.enabled() {
            return true;
        }
        let oi = self.owner_id(id);
        if let Some(e) = self.names.get(name).cloned() {
            if e.oi == oi {
                self.touch(name);
                self.save();
                return true;
            }
            if !fk.is_empty() && e.o == self.owner(fk) && e.oi.is_empty() {
                if let Some(x) = self.names.get_mut(name) {
                    x.oi = oi;
                }
                self.touch(name);
                self.save();
                return true;
            }
            return false;
        }
        self.bind(name, Entry { o: String::new(), oi: oi.clone(), at: crate::now() as f64 });
        self.trim_owner(&oi);
        self.trim_global();
        self.save();
        true
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}
