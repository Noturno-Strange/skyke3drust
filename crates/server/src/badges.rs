//! Emblemas (`lib/badges.js`): os concedidos pelo administrador (equipe, apoiador, pioneiro) e os derivados do perfil.

use crate::persist::{read_json_file, write_json_atomic};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

const MAX_PIDS: usize = 5000;
pub const GRANTED: [(&str, &str, &str); 3] =
    [("equipe", "Equipe", "🛡️"), ("apoiador", "Apoiador", "💜"), ("pioneiro", "Pioneiro", "🚀")];

fn granted(id: &str) -> Option<(&'static str, &'static str)> {
    GRANTED.iter().find(|g| g.0 == id).map(|g| (g.1, g.2))
}

#[derive(Debug)]
pub struct Badges {
    file: Option<PathBuf>,
    grants: IndexMap<String, IndexSet<String>>,
    dirty: bool,
    due: Option<u64>,
}

impl Badges {
    pub fn new(file: &str) -> Self {
        let mut b = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            grants: IndexMap::new(),
            dirty: false,
            due: None,
        };
        if let Some(f) = &b.file {
            match read_json_file(f) {
                Ok(Some(obj)) if obj["v"] == 1 => {
                    for (pid, ids) in obj["g"].as_object().into_iter().flatten() {
                        if b.grants.len() >= MAX_PIDS {
                            break;
                        }
                        let Some(a) = ids.as_array() else { continue };
                        if !crate::pubprofile::pid_re(pid) {
                            continue;
                        }
                        let set: IndexSet<String> = a
                            .iter()
                            .filter_map(Value::as_str)
                            .filter(|x| granted(x).is_some())
                            .map(str::to_owned)
                            .collect();
                        if !set.is_empty() {
                            b.grants.insert(pid.clone(), set);
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => tracing::error!("emblemas: {e}"),
            }
        }
        b
    }

    fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let g: Map<String, Value> = self.grants.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        if let Err(e) = write_json_atomic(&f, &json!({"v": 1, "g": g}), false) {
            self.dirty = true;
            tracing::error!("emblemas: {e}");
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

    /// Concede ou retira um emblema (painel do administrador ou console).
    pub fn set(&mut self, pid: &str, id: &str, on: bool) -> Value {
        if !crate::pubprofile::pid_re(pid) {
            return json!({"ok": false, "reason": "pid"});
        }
        if granted(id).is_none() {
            return json!({"ok": false, "reason": "badge"});
        }
        if on {
            if !self.grants.contains_key(pid) && self.grants.len() >= MAX_PIDS {
                return json!({"ok": false, "reason": "full"});
            }
            self.grants.entry(pid.into()).or_default().insert(id.into());
        } else if let Some(s) = self.grants.get_mut(pid) {
            s.shift_remove(id);
            if s.is_empty() {
                self.grants.shift_remove(pid);
            }
        }
        if self.file.is_some() {
            self.dirty = true;
            self.due.get_or_insert(crate::now() + 2000);
        }
        json!({"ok": true, "badges": self.grants.get(pid).map(|s| s.iter().cloned().collect::<Vec<_>>()).unwrap_or_default()})
    }

    /// Emblemas de um perfil: concedidos, derivados do perfil (`profile` como `PUB.get`) e os extras (fazenda, culinária).
    pub fn of(&self, pid: &str, profile: &Value, extra: &[Value]) -> Value {
        let mut out: Vec<Value> = self
            .grants
            .get(pid)
            .into_iter()
            .flatten()
            .filter_map(|id| granted(id).map(|(n, i)| json!({"id": id, "name": n, "icon": i})))
            .collect();
        let s = |k: &str| profile[k].as_str().unwrap_or("");
        if crate::validate::len16(s("bio")) >= 20
            && !s("like").is_empty()
            && !s("mood").is_empty()
            && !s("world").is_empty()
        {
            out.push(json!({"id": "completo", "name": "Perfil completo", "icon": "✍️"}));
        }
        if profile["likes"].as_u64().unwrap_or(0) >= 10 {
            out.push(json!({"id": "querido", "name": "Querido pela galera", "icon": "❤️"}));
        }
        for e in extra {
            out.push(json!({"id": e["id"], "name": e["name"], "icon": e["icon"]}));
        }
        Value::Array(out)
    }

    pub fn len(&self) -> usize {
        self.grants.len()
    }

    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }
}
