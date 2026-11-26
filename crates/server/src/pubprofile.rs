//! Perfil público opcional (`lib/pubprofile.js`): nome, aparência e textos curtos, ligado pelo próprio jogador. O
//! código público (`pidOf`) é derivado do id de identidade e nunca o revela. Curtidas e livro de visitas só existem na
//! memória (somem com o perfil).

use crate::catalog::Catalog;
use crate::persist::{read_json_file, write_json_atomic};
use crate::playerdata::{clean_cfg_out, clean_text};
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};
use std::path::PathBuf;

const DAY: u64 = 86_400_000;
pub const BIO_MAX: usize = 300;
const LIKE_MAX: usize = 80;
const MOOD_MAX: usize = 60;
const LIKES_MAX: usize = 5000;
const BOOK_MAX: usize = 20;
const NOTE_MAX: usize = 120;
const PER_AUTHOR: usize = 3;

pub fn pid_re(s: &str) -> bool {
    s.len() == 20 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn world_id(w: &Value) -> String {
    let Some(w) = w.as_str() else { return String::new() };
    let cat = Catalog::embedded();
    if cat.styles.iter().any(|s| s == w) { w.into() } else { cat.st_legacy.get(w).cloned().unwrap_or_default() }
}

/// Texto com link (`hasLink`): endereços, domínios comuns, convites do Discord e do Telegram.
pub fn has_link(s: &str) -> bool {
    use std::sync::OnceLock;
    static R: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let r = R.get_or_init(|| {
        regex::Regex::new(r"(?i)(https?:|www\.|//|(?-u:\b)[a-z0-9-]{2,}\.(com|net|org|br|io|gg|me|ly|app|xyz|link|ru|tk|ml|ga|cf)(?-u:\b)|discord\.gg|t\.me)").ok()
    });
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    r.as_ref().is_some_and(|r| r.is_match(&compact))
}

/// Campos do perfil já limpos (`cleanFields`); `Err` com o motivo.
pub fn clean_fields(f: &Value, censor: bool) -> Result<Value, &'static str> {
    let c = |s: String| if censor { crate::profanity::censor(&s) } else { s };
    let t = |k: &str, max: usize| {
        clean_text(
            &match &f[k] {
                Value::Null => String::new(),
                Value::String(s) => s.clone(),
                v => v.to_string(),
            },
            max,
        )
    };
    let bio = c(t("bio", BIO_MAX));
    let like = c(t("like", LIKE_MAX));
    let mood = c(t("mood", MOOD_MAX));
    if has_link(&bio) || has_link(&like) || has_link(&mood) {
        return Err("link");
    }
    Ok(json!({"bio": bio, "like": like, "mood": mood, "world": world_id(&f["world"])}))
}

#[derive(Debug, Clone, Default)]
struct Stored {
    n: String,
    c: Value,
    b: String,
    l: String,
    m: String,
    w: String,
    at: u64,
}

#[derive(Debug, Clone)]
struct Note {
    id: u64,
    by: String,
    name: String,
    text: String,
    at: u64,
}

#[derive(Debug, Default)]
struct Live {
    conns: IndexSet<u64>,
    likes: IndexSet<String>,
    book: Vec<Note>,
    next_msg: u64,
    seen: u64,
}

/// Nome e aparência atuais de uma conexão (quando ela está numa sala).
pub type MeOf<'a> = &'a dyn Fn(u64) -> Option<(String, Value)>;

#[derive(Debug)]
pub struct PubProfiles {
    file: Option<PathBuf>,
    ttl_ms: u64,
    stale_ms: u64,
    max: usize,
    store: IndexMap<String, Stored>,
    live: IndexMap<String, Live>,
    dirty: bool,
    due: Option<u64>,
}

impl PubProfiles {
    pub fn new(file: &str) -> Self {
        let mut p = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            ttl_ms: DAY,
            stale_ms: 180 * DAY,
            max: 10000,
            store: IndexMap::new(),
            live: IndexMap::new(),
            dirty: false,
            due: None,
        };
        p.load();
        p
    }

    fn load(&mut self) {
        let Some(f) = self.file.clone() else { return };
        let obj = match read_json_file(&f) {
            Ok(o) => o,
            Err(_) => {
                let mut bak = f.as_os_str().to_owned();
                bak.push(".bak");
                let _ = std::fs::rename(&f, PathBuf::from(bak));
                tracing::warn!("perfis: arquivo inválido, começando limpo.");
                None
            }
        };
        let Some(obj) = obj.filter(|o| o["v"] == 1) else { return };
        let Some(p) = obj["p"].as_object() else { return };
        let t = crate::now();
        for (pid, e) in p {
            if self.store.len() >= self.max {
                break;
            }
            if !pid_re(pid) || !e.is_object() {
                continue;
            }
            let (Some(n), Some(at)) = (e["n"].as_str(), e["at"].as_f64().filter(|x| x.is_finite())) else { continue };
            if (t as f64) - at > self.stale_ms as f64 {
                continue;
            }
            let s = |k: &str, max| clean_text(e[k].as_str().unwrap_or(""), max);
            self.store.insert(
                pid.clone(),
                Stored {
                    n: clean_text(n, 16),
                    c: clean_cfg_out(&e["c"]),
                    b: s("b", BIO_MAX),
                    l: s("l", LIKE_MAX),
                    m: s("m", MOOD_MAX),
                    w: world_id(&e["w"]),
                    at: at as u64,
                },
            );
        }
    }

    fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let p: Map<String, Value> = self
            .store
            .iter()
            .map(|(k, s)| (k.clone(), json!({"n": s.n, "c": s.c, "b": s.b, "l": s.l, "m": s.m, "w": s.w, "at": s.at})))
            .collect();
        if let Err(e) = write_json_atomic(&f, &json!({"v": 1, "savedAt": crate::now(), "p": p}), false) {
            self.dirty = true;
            tracing::error!("perfis: {e}");
        }
    }

    fn save(&mut self) {
        if self.file.is_none() {
            return;
        }
        self.dirty = true;
        if self.due.is_none() {
            self.due = Some(crate::now() + 5000);
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

    fn ent(&mut self, pid: &str) -> &mut Live {
        self.live.entry(pid.into()).or_insert_with(|| Live { next_msg: 1, ..Default::default() })
    }

    fn make_room(&mut self) -> bool {
        if self.store.len() < self.max {
            return true;
        }
        let old = self
            .store
            .iter()
            .filter(|(pid, _)| !self.live.get(*pid).is_some_and(|e| !e.conns.is_empty()))
            .min_by_key(|(_, s)| s.at)
            .map(|(k, _)| k.clone());
        let Some(old) = old else { return false };
        self.store.shift_remove(&old);
        self.live.shift_remove(&old);
        true
    }

    /// Liga (ou atualiza) o perfil: `me` é o nome e a aparência atuais de quem publicou.
    pub fn publish(&mut self, pid: &str, conn: u64, f: &Value, me: Option<(String, Value)>) -> bool {
        if !pid_re(pid) {
            return false;
        }
        if !self.store.contains_key(pid) {
            if !self.make_room() {
                return false;
            }
            self.store.insert(pid.into(), Stored { c: json!({}), ..Default::default() });
        }
        let now = crate::now();
        if let Some(s) = self.store.get_mut(pid) {
            if let Some((name, cfg)) = me.filter(|(n, _)| !n.is_empty()) {
                s.n = clean_text(&name, 16);
                s.c = clean_cfg_out(&cfg);
            }
            s.b = f["bio"].as_str().unwrap_or("").into();
            s.l = f["like"].as_str().unwrap_or("").into();
            s.m = f["mood"].as_str().unwrap_or("").into();
            s.w = f["world"].as_str().unwrap_or("").into();
            s.at = now;
        }
        self.ent(pid).conns.insert(conn);
        self.save();
        true
    }

    pub fn fields_of(&self, pid: &str) -> Option<Value> {
        self.store.get(pid).map(|s| json!({"bio": s.b, "like": s.l, "mood": s.m, "world": s.w}))
    }

    pub fn withdraw(&mut self, pid: &str) {
        if self.store.shift_remove(pid).is_some() {
            self.save();
        }
        self.live.shift_remove(pid);
    }

    /// A conexão saiu: guarda o último nome e a aparência (`snap`) quando era a última aba.
    pub fn leave(&mut self, pid: &str, conn: u64, snap: Option<(String, Value)>) {
        let now = crate::now();
        let empty = match self.live.get_mut(pid) {
            Some(e) => {
                e.conns.shift_remove(&conn);
                if e.conns.is_empty() {
                    e.seen = now;
                }
                e.conns.is_empty()
            }
            None => true,
        };
        if empty && let Some(s) = self.store.get_mut(pid) {
            if let Some((name, cfg)) = snap.filter(|(n, _)| !n.is_empty()) {
                s.n = clean_text(&name, 16);
                s.c = clean_cfg_out(&cfg);
            }
            s.at = now;
            self.save();
        }
    }

    pub fn has(&self, pid: &str) -> bool {
        pid_re(pid) && self.store.contains_key(pid)
    }

    /// O perfil como a página `/profile` vê (sem o código de ninguém).
    pub fn get(&self, pid: &str, me_of: MeOf<'_>) -> Option<Value> {
        if !pid_re(pid) {
            return None;
        }
        let s = self.store.get(pid)?;
        let e = self.live.get(pid);
        let (mut name, mut cfg, mut online) = (s.n.clone(), s.c.clone(), false);
        if let Some(e) = e {
            for c in &e.conns {
                if let Some((n, c)) = me_of(*c).filter(|(n, _)| !n.is_empty()) {
                    name = n;
                    cfg = c;
                    online = true;
                    break;
                }
            }
        }
        if name.is_empty() {
            return None;
        }
        let cat = Catalog::embedded();
        let world =
            if s.w.is_empty() { String::new() } else { cat.raw["ST"][&s.w]["n"].as_str().unwrap_or("").to_owned() };
        Some(json!({
            "name": name, "bio": s.b, "like": s.l, "mood": s.m, "world": world, "online": online,
            "seenAt": if online { 0 } else { s.at }, "likes": e.map_or(0, |e| e.likes.len()), "book": self.book_of(e),
            "cfg": clean_cfg_out(&cfg)
        }))
    }

    fn book_of(&self, e: Option<&Live>) -> Value {
        Value::Array(
            e.map(|e| e.book.iter().rev().map(|m| json!({"name": m.name, "text": m.text, "at": m.at})).collect())
                .unwrap_or_default(),
        )
    }

    pub fn like(&mut self, pid: &str, voter: &str) -> Value {
        if !pid_re(pid) || !pid_re(voter) {
            return json!({"ok": false, "reason": "invalid"});
        }
        if pid == voter {
            return json!({"ok": false, "reason": "self"});
        }
        if !self.store.contains_key(pid) {
            return json!({"ok": false, "reason": "notfound"});
        }
        let e = self.ent(pid);
        if e.likes.contains(voter) {
            return json!({"ok": true, "dup": true, "count": e.likes.len()});
        }
        if e.likes.len() >= LIKES_MAX {
            return json!({"ok": false, "reason": "full"});
        }
        e.likes.insert(voter.into());
        json!({"ok": true, "count": e.likes.len()})
    }

    /// Recado no livro de visitas; `Ok` traz as conexões do dono (para atualizar a lista dele).
    pub fn post(&mut self, pid: &str, author: &str, name: &str, text: &str) -> Result<Vec<u64>, &'static str> {
        if !pid_re(pid) || !pid_re(author) {
            return Err("invalid");
        }
        if pid == author {
            return Err("self");
        }
        if !self.store.contains_key(pid) {
            return Err("notfound");
        }
        let t = clean_text(text, NOTE_MAX);
        if t.is_empty() {
            return Err("empty");
        }
        if has_link(&t) {
            return Err("link");
        }
        let now = crate::now();
        let e = self.ent(pid);
        if e.book.iter().filter(|m| m.by == author).count() >= PER_AUTHOR {
            return Err("limit");
        }
        let id = e.next_msg;
        e.next_msg += 1;
        e.book.push(Note { id, by: author.into(), name: crate::validate::slice16(name, 16), text: t, at: now });
        while e.book.len() > BOOK_MAX {
            e.book.remove(0);
        }
        Ok(e.conns.iter().copied().collect())
    }

    pub fn list_book(&self, pid: &str) -> Value {
        Value::Array(
            self.live
                .get(pid)
                .map(|e| {
                    e.book
                        .iter()
                        .rev()
                        .map(|m| json!({"id": m.id, "name": m.name, "text": m.text, "at": m.at}))
                        .collect()
                })
                .unwrap_or_default(),
        )
    }

    pub fn remove_note(&mut self, pid: &str, id: i64) -> bool {
        let Some(e) = self.live.get_mut(pid) else { return false };
        let n = e.book.len();
        e.book.retain(|m| m.id as i64 != id);
        e.book.len() < n
    }

    pub fn sweep(&mut self) -> usize {
        let t = crate::now();
        let gone: Vec<String> = self
            .store
            .iter()
            .filter(|(pid, s)| {
                !self.live.get(*pid).is_some_and(|e| !e.conns.is_empty()) && t.saturating_sub(s.at) >= self.stale_ms
            })
            .map(|(k, _)| k.clone())
            .collect();
        for pid in &gone {
            self.store.shift_remove(pid);
            self.live.shift_remove(pid);
        }
        let ttl = self.ttl_ms;
        self.live.retain(|_, e| !e.conns.is_empty() || t.saturating_sub(e.seen) < ttl);
        if !gone.is_empty() {
            self.save();
        }
        gone.len()
    }

    pub fn stats(&self) -> Value {
        json!({"total": self.store.len(), "online": self.live.values().filter(|e| !e.conns.is_empty()).count()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_flow() {
        let mut p = PubProfiles::new("");
        let pid = crate::game::pid_of("abcdefghijk2");
        let f = clean_fields(&json!({"bio": "Oi!", "world": "proxima"}), true).unwrap();
        assert_eq!(f["world"], "cerron");
        assert!(clean_fields(&json!({"bio": "veja meusite.com"}), true).is_err());
        assert!(p.publish(&pid, 7, &f, Some(("Ana".into(), json!({"skin": 1})))));
        let me = |c: u64| (c == 7).then(|| ("Ana".to_owned(), json!({"skin": 2})));
        let g = p.get(&pid, &me).unwrap();
        assert_eq!((g["online"].as_bool(), g["cfg"]["skin"].as_i64()), (Some(true), Some(2)));
        let other = crate::game::pid_of("zzzzzzzzzzz2");
        assert_eq!(p.like(&pid, &other)["count"], 1);
        assert_eq!(p.post(&pid, &other, "Bia", "Que legal"), Ok(vec![7]));
        assert_eq!(p.list_book(&pid)[0]["text"], "Que legal");
    }
}
