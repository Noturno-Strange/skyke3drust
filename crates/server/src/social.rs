//! Amigos, presença, convites e chat global (`lib/social.js`). Cada conexão ganha um apelido público aleatório (`h`):
//! a sala e o chat global só veem o apelido, nunca o id de amigo.

use crate::game::Game;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const TEN_MIN: u64 = 600_000;
const MIN_AGE: u64 = 20_000;
const CAP: usize = 30;
const IP_CAP: usize = 5;
const B32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

#[derive(Debug, Default)]
pub struct Social {
    /// id de amigo → conexões (abas) dessa pessoa, na ordem em que entraram.
    online: HashMap<String, indexmap::IndexSet<u64>>,
    asked: HashMap<String, u64>,
    invited: HashMap<String, u64>,
    spoke: HashMap<String, u64>,
    handles: HashMap<String, u64>,
    shown: Option<usize>,
    people: usize,
    burst: Vec<u64>,
    ip_burst: HashMap<String, Vec<u64>>,
}

impl Social {
    pub fn people(&self) -> usize {
        self.people
    }
    fn tabs(&self, id: &str) -> Vec<u64> {
        self.online.get(id).map(|s| s.iter().copied().collect()).unwrap_or_default()
    }
    fn one(&self, id: &str) -> Option<u64> {
        self.online.get(id).and_then(|s| s.first().copied())
    }
    fn fresh(map: &HashMap<String, u64>, key: &str, ms: u64) -> bool {
        map.get(key).copied().unwrap_or(0) > crate::now().saturating_sub(ms)
    }
    fn prune(&mut self) {
        let old = crate::now().saturating_sub(TEN_MIN);
        for m in [&mut self.asked, &mut self.invited, &mut self.spoke] {
            if m.len() > 500 {
                m.retain(|_, t| *t >= old);
            }
        }
    }
}

/// Links trocados por 🔗 no chat global (`LINK`).
fn strip_links(text: &str) -> String {
    use regex::Regex;
    use std::sync::OnceLock;
    static R: OnceLock<Option<Regex>> = OnceLock::new();
    let r = R.get_or_init(|| {
        Regex::new(r"(?i)(?-u:\b)(?:https?://|www\.)\S+|(?-u:\b)[A-Za-z0-9_-]+\.(?:com|net|org|br|io|gg|me|xyz|ly|tv|app|site|link)(?:/\S*)?").ok()
    });
    match r {
        Some(r) => r.replace_all(text, "🔗").into_owned(),
        None => text.into(),
    }
}

impl Game {
    fn soc_active(&self, cid: u64) -> bool {
        self.conns.get(&cid).is_some_and(|c| {
            c.me.is_some()
                && c.room.is_some()
                && !c.fid.is_empty()
                && self.social.online.get(&c.fid).is_some_and(|s| s.contains(&cid))
        })
    }

    fn soc_mutual(&self, a: Option<u64>, b: Option<u64>) -> bool {
        let (Some(a), Some(b)) = (a.and_then(|a| self.conns.get(&a)), b.and_then(|b| self.conns.get(&b))) else {
            return false;
        };
        a.watch.contains(&b.fid) && b.watch.contains(&a.fid)
    }

    fn soc_to_all(&mut self, id: &str, m: &Value) {
        for c in self.social.tabs(id) {
            self.send(c, m);
        }
    }

    fn my_name(&self, cid: u64) -> String {
        let Some(c) = self.conns.get(&cid) else { return String::new() };
        let (Some(code), Some(me)) = (&c.room, &c.me) else { return String::new() };
        self.rooms.get(code).and_then(|r| r.players.get(me)).map(|p| p.name.clone()).unwrap_or_default()
    }

    /// Presença de `c` (on/off) mostrada para `to`.
    fn soc_pres(&mut self, to: u64, c: u64, on: bool) {
        let Some(fid) = self.conns.get(&c).map(|x| x.fid.clone()) else { return };
        if on {
            let name = self.my_name(c);
            let hs: Vec<String> =
                self.social.tabs(&fid).iter().filter_map(|x| self.conns.get(x).map(|y| y.h.clone())).collect();
            if let Some(t) = self.conns.get_mut(&to) {
                t.shown.insert(fid.clone());
            }
            self.send(to, &json!({"t": "fpres", "id": fid, "on": 1, "name": name, "hs": hs}));
        } else if self.conns.get_mut(&to).is_some_and(|t| t.shown.remove(&fid)) {
            self.send(to, &json!({"t": "fpres", "id": fid, "on": 0}));
        }
    }

    fn new_handle(&self) -> String {
        loop {
            let h: String = crate::random_bytes::<12>().iter().map(|x| B32[(*x & 31) as usize] as char).collect();
            if !self.social.handles.contains_key(&h) {
                return h;
            }
        }
    }

    /// Entra na rede de amigos com o id `fid` e a lista `ids`; devolve o apelido público.
    pub fn soc_join_fid(&mut self, cid: u64, fid: &str, ids: &[String]) -> String {
        if !crate::identity::id_re(fid) {
            return String::new();
        }
        if self.conns.get(&cid).is_some_and(|c| !c.fid.is_empty() && c.fid != fid) {
            self.soc_leave(cid);
        }
        let was = self.conns.get(&cid).map(|c| c.watch.clone()).unwrap_or_default();
        let watch: HashSet<String> = ids.iter().filter(|x| *x != fid).cloned().collect();
        let need_h = self.conns.get(&cid).is_some_and(|c| c.h.is_empty());
        let h = if need_h { self.new_handle() } else { self.conns.get(&cid).map(|c| c.h.clone()).unwrap_or_default() };
        let Some(c) = self.conns.get_mut(&cid) else { return String::new() };
        c.fid = fid.into();
        c.watch = watch.clone();
        c.h = h.clone();
        if need_h {
            self.social.handles.insert(h.clone(), cid);
        }
        let set = self.social.online.entry(fid.into()).or_default();
        if set.insert(cid) {
            self.social.people += 1;
        }
        self.send(cid, &json!({"t": "fme", "id": fid, "h": h}));
        let all: HashSet<String> = was.union(&watch).cloned().collect();
        for id in all {
            let Some(o) = self.social.one(&id) else { continue };
            let m = self.soc_mutual(Some(cid), Some(o));
            self.soc_pres(cid, o, m);
            if self.conns.get(&o).is_some_and(|oc| oc.watch.contains(fid)) {
                for x in self.social.tabs(&id) {
                    self.soc_pres(x, cid, m);
                }
            }
        }
        h
    }

    pub fn soc_leave(&mut self, cid: u64) {
        let Some(c) = self.conns.get(&cid) else { return };
        let fid = c.fid.clone();
        let watch = c.watch.clone();
        let removed = !fid.is_empty() && self.social.online.get_mut(&fid).is_some_and(|s| s.shift_remove(&cid));
        if removed {
            self.social.people = self.social.people.saturating_sub(1);
            let rest = self.social.one(&fid);
            if self.social.online.get(&fid).is_some_and(|s| s.is_empty()) {
                self.social.online.remove(&fid);
            }
            for id in &watch {
                for o in self.social.tabs(id) {
                    if !self.conns.get(&o).is_some_and(|oc| oc.watch.contains(&fid)) {
                        continue;
                    }
                    match rest {
                        Some(r) => {
                            if self.soc_mutual(Some(o), Some(r)) {
                                self.soc_pres(o, r, true);
                            }
                        }
                        None => self.soc_pres(o, cid, false),
                    }
                }
            }
        }
        let Some(c) = self.conns.get_mut(&cid) else { return };
        let h = std::mem::take(&mut c.h);
        self.social.handles.remove(&h);
        c.fid.clear();
        c.watch.clear();
        c.shown.clear();
        c.g_on = false;
    }

    /// `friends`: a lista de amigos e a chave (ou a identidade autenticada).
    pub fn soc_friends(&mut self, cid: u64, m: &Value) {
        let Some(c) = self.conns.get(&cid) else { return };
        let fid = if !c.ident_scope.is_empty() {
            c.ident_id.clone()
        } else if let Some(k) = m["k"].as_str() {
            sky_identity::fid_of(k)
        } else {
            c.ident_id.clone()
        };
        if fid.is_empty() {
            return self.send(
                cid,
                &json!({"t": "err", "scope": "friends", "msg": "Entre com sua identidade para usar amigos."}),
            );
        }
        let ids: Vec<String> =
            m["ids"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
        if c.ident_scope == "primary" {
            let id = c.ident_id.clone();
            let mut patch = json!({"friends": ids});
            if let Some(names) = m["names"].as_array() {
                let fnames: serde_json::Map<String, Value> = ids.iter().cloned().zip(names.iter().cloned()).collect();
                patch["friendNames"] = Value::Object(fnames);
            }
            self.player_data.update(&id, patch);
        }
        let h = self.soc_join_fid(cid, &fid, &ids);
        let Some(c) = self.conns.get(&cid) else { return };
        let (Some(code), Some(me)) = (c.room.clone(), c.me.clone()) else { return };
        let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) else { return };
        if p.fid == h {
            return;
        }
        p.fid = h.clone();
        self.bc(&code, &json!({"t": "pfid", "id": me, "fid": h}), Some(cid));
    }

    pub fn soc_request(&mut self, cid: u64, h: &str) {
        if !self.soc_active(cid) {
            return;
        }
        let t = self.social.handles.get(h).copied().filter(|t| self.soc_active(*t));
        let to = t.and_then(|t| self.conns.get(&t)).map(|c| c.fid.clone()).unwrap_or_default();
        let (my_fid, my_room, my_h) =
            self.conns.get(&cid).map(|c| (c.fid.clone(), c.room.clone(), c.h.clone())).unwrap_or_default();
        if !to.is_empty() && (to == my_fid || self.soc_mutual(Some(cid), t)) {
            return;
        }
        let near = !to.is_empty()
            && self.social.tabs(&to).iter().any(|o| self.conns.get(o).is_some_and(|x| x.room == my_room));
        if !near && !(!to.is_empty() && Social::fresh(&self.social.spoke, &to, TEN_MIN)) {
            return self.send(cid, &json!({"t": "sys", "text": "Pedido não enviado: só dá para pedir amizade a quem está na sua sala ou falou no chat global há pouco."}));
        }
        let k = format!("{my_fid}>{to}");
        if Social::fresh(&self.social.asked, &k, TEN_MIN) {
            return;
        }
        self.social.asked.insert(k, crate::now());
        self.social.prune();
        let name = self.my_name(cid);
        self.soc_to_all(&to, &json!({"t": "freq", "id": my_fid, "h": my_h, "name": name}));
    }

    pub fn soc_accept(&mut self, cid: u64, from: &str) {
        let my_fid = self.conns.get(&cid).map(|c| c.fid.clone()).unwrap_or_default();
        let k = format!("{from}>{my_fid}");
        if !self.soc_active(cid) || !Social::fresh(&self.social.asked, &k, TEN_MIN) {
            return;
        }
        self.social.asked.remove(&k);
        let name = self.my_name(cid);
        self.soc_to_all(from, &json!({"t": "facc", "id": my_fid, "name": name}));
    }

    pub fn soc_invite(&mut self, cid: u64, to: &str) {
        let (my_fid, my_room) = self.conns.get(&cid).map(|c| (c.fid.clone(), c.room.clone())).unwrap_or_default();
        let k = format!("{my_fid}>{to}");
        if !self.soc_active(cid) {
            return;
        }
        let result = |g: &mut Game, ok: bool, reason: &str| {
            let mut m = json!({"t": "finvr", "id": to, "ok": ok});
            if !reason.is_empty() {
                m["reason"] = json!(reason);
            }
            g.send(cid, &m);
        };
        let one = self.social.one(to);
        if !self.soc_mutual(Some(cid), one) {
            return result(self, false, "unavailable");
        }
        if Social::fresh(&self.social.invited, &k, 30000) {
            return result(self, false, "wait");
        }
        let away: Vec<u64> =
            self.social.tabs(to).into_iter().filter(|o| self.conns.get(o).is_some_and(|x| x.room != my_room)).collect();
        if away.is_empty() {
            return result(self, false, "same");
        }
        self.social.invited.insert(k, crate::now());
        self.social.prune();
        let m = json!({"t": "finv", "id": my_fid, "name": self.my_name(cid), "code": my_room});
        for o in away {
            self.send(o, &m);
        }
        result(self, true, "");
    }

    fn soc_each(&self) -> Vec<u64> {
        self.social
            .online
            .values()
            .flatten()
            .copied()
            .filter(|o| self.conns.get(o).is_some_and(|c| c.g_on) && self.soc_active(*o))
            .collect()
    }

    pub fn soc_subscribe(&mut self, cid: u64, on: bool) {
        if !self.soc_active(cid) {
            return;
        }
        if let Some(c) = self.conns.get_mut(&cid) {
            c.g_on = on;
        }
        if on {
            let n = self.social.people;
            self.send(cid, &json!({"t": "gn", "n": n}));
        }
    }

    pub fn soc_say(&mut self, cid: u64, text: &str) {
        if !self.soc_active(cid) {
            return;
        }
        let n = crate::now();
        let Some(c) = self.conns.get(&cid) else { return };
        if n.saturating_sub(c.since) < MIN_AGE {
            return self.send(
                cid,
                &json!({"t": "gsys", "text": "⏳ Fique um pouquinho na sala antes de falar no chat global."}),
            );
        }
        if text == c.g_last && n.saturating_sub(c.g_last_at) < 60000 {
            return;
        }
        let ipk = c.ip.clone();
        self.social.burst.retain(|x| n - x < 10000);
        let mut mine: Vec<u64> = self.social.ip_burst.get(&ipk).cloned().unwrap_or_default();
        mine.retain(|x| n - x < 10000);
        if self.social.burst.len() >= CAP || mine.len() >= IP_CAP {
            return self.send(cid, &json!({"t": "gsys", "text": "⏳ O chat global está muito movimentado. Tente de novo em alguns segundos."}));
        }
        let clean = crate::profanity::censor(&strip_links(text)).trim().to_owned();
        if clean.is_empty() || clean == "🔗" {
            return;
        }
        if let Some(c) = self.conns.get_mut(&cid) {
            c.g_last = text.into();
            c.g_last_at = n;
        }
        self.social.burst.push(n);
        mine.push(n);
        self.social.ip_burst.insert(ipk, mine);
        if self.social.ip_burst.len() > 1000 {
            self.social.ip_burst.retain(|_, a| a.last().is_some_and(|l| n - l < 10000));
        }
        let (fid, h) = self.conns.get(&cid).map(|c| (c.fid.clone(), c.h.clone())).unwrap_or_default();
        self.social.spoke.insert(fid, n);
        self.social.prune();
        let m = json!({"t": "gchat", "id": h, "name": self.my_name(cid), "text": clean, "at": n});
        let s: Arc<str> = Arc::from(crate::jsjson::to_string(&m));
        for o in self.soc_each() {
            self.send_raw_to(o, crate::game::Out::Text(s.clone()), s.len());
        }
    }

    /// A cada 10 s: a contagem de pessoas online vai para quem acompanha o chat global, quando muda.
    pub fn soc_tick(&mut self) {
        let people = self.social.people;
        if self.social.shown == Some(people) {
            return;
        }
        self.social.shown = Some(people);
        for o in self.soc_each() {
            self.send(o, &json!({"t": "gn", "n": people}));
        }
    }
}
