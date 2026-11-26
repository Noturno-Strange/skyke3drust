//! Dados da conta guardados no servidor (`lib/player-data.js`): nome, visual e lista de amigos de cada identidade.

use crate::persist::{read_json_file, write_json_atomic};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

/// Texto curto sem controle, sem marcação e sem espaços repetidos (`cleanText` de `lib/pubprofile.js`).
pub fn clean_text(s: &str, max: usize) -> String {
    let t: String = s
        .chars()
        .map(|c| {
            let u = c as u32;
            if u <= 0x1f
                || (0x7f..=0x9f).contains(&u)
                || (0x200b..=0x200f).contains(&u)
                || (0x2028..=0x202e).contains(&u)
                || (0x2060..=0x206f).contains(&u)
                || u == 0xfeff
            {
                ' '
            } else {
                c
            }
        })
        .filter(|c| !"<>&\"'`".contains(*c))
        .collect();
    crate::validate::slice16(&t.split_whitespace().collect::<Vec<_>>().join(" "), max)
}

/// Visual só com índices válidos do catálogo (`cleanCfgOut`).
pub fn clean_cfg_out(cfg: &Value) -> Value {
    let cat = crate::catalog::Catalog::embedded();
    let mut out = Map::new();
    if let Some(o) = cfg.as_object() {
        for (k, n) in &cat.cr {
            let raw = o.get(k).cloned().unwrap_or(Value::Null);
            let key = match &raw {
                Value::Number(x) => x.to_string(),
                Value::String(s) => s.clone(),
                Value::Bool(b) => b.to_string(),
                _ => String::new(),
            };
            let v = cat.cfg_gone.get(k).and_then(|g| g.get(&key)).cloned().unwrap_or(raw);
            if let Some(i) = crate::validate::int(&v, 0, n - 1) {
                out.insert(k.clone(), json!(i));
            }
        }
    }
    Value::Object(out)
}

fn clean(r: &Value) -> Option<Value> {
    r.as_object()?;
    let friend_names: Map<String, Value> = r["friendNames"]
        .as_object()
        .map(|o| {
            o.iter()
                .filter(|(id, _)| crate::identity::id_re(id))
                .take(100)
                .map(|(id, n)| (id.clone(), json!(clean_text(n.as_str().unwrap_or(""), 16))))
                .collect()
        })
        .unwrap_or_default();
    let friends = r["friends"].as_array().map(|a| {
        let mut seen = indexmap::IndexSet::new();
        for id in a.iter().filter_map(Value::as_str).filter(|x| crate::identity::id_re(x)) {
            seen.insert(id.to_owned());
        }
        seen.into_iter().take(100).collect::<Vec<_>>()
    });
    let name = match &r["name"] {
        Value::Null => String::new(),
        Value::String(s) => clean_text(s, 16),
        v => clean_text(&v.to_string(), 16),
    };
    Some(json!({"name": name, "cfg": clean_cfg_out(&r["cfg"]), "friendNames": friend_names, "friends": friends}))
}

#[derive(Debug)]
pub struct PlayerData {
    file: Option<PathBuf>,
    max: usize,
    players: indexmap::IndexMap<String, Value>,
    dirty: bool,
    due: Option<u64>,
}

impl PlayerData {
    pub fn new(file: &str) -> Self {
        let mut d = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            max: 20000,
            players: Default::default(),
            dirty: false,
            due: None,
        };
        if let Some(f) = &d.file {
            match read_json_file(f) {
                Ok(Some(data)) if data["v"] == 1 => {
                    for (id, r) in data["p"].as_object().into_iter().flatten() {
                        if d.players.len() >= d.max {
                            break;
                        }
                        if crate::identity::id_re(id)
                            && let Some(v) = clean(r)
                        {
                            d.players.insert(id.clone(), v);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!("dados dos jogadores: {e}"),
            }
        }
        d
    }

    pub fn get(&self, id: &str) -> Option<Value> {
        self.players.get(id).cloned()
    }

    pub fn update(&mut self, id: &str, patch: Value) -> bool {
        if !crate::identity::id_re(id) || (!self.players.contains_key(id) && self.players.len() >= self.max) {
            return false;
        }
        let mut base = self.players.get(id).cloned().unwrap_or_else(|| json!({}));
        if let (Some(b), Some(p)) = (base.as_object_mut(), patch.as_object()) {
            for (k, v) in p {
                b.insert(k.clone(), v.clone());
            }
        }
        let Some(v) = clean(&base) else { return false };
        self.players.insert(id.into(), v);
        self.dirty = true;
        if self.file.is_some() && self.due.is_none() {
            self.due = Some(crate::now() + 1000);
        }
        true
    }

    pub fn tick(&mut self, now: u64) {
        if self.due.is_some_and(|d| now >= d) {
            self.flush();
        }
    }

    pub fn flush(&mut self) {
        self.due = None;
        let Some(f) = &self.file else { return };
        if !self.dirty {
            return;
        }
        let p: Map<String, Value> = self.players.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        match write_json_atomic(f, &json!({"v": 1, "p": p}), false) {
            Ok(()) => self.dirty = false,
            Err(e) => tracing::error!("dados dos jogadores: {e}"),
        }
    }
}
