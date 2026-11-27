//! Vídeos e karaokê da sala (`lib/video.js` e `lib/karaoke.js`): cada jogador pode ter um vídeo (até 4 por sala) que os
//! outros escolhem assistir, com o relógio do servidor; o karaokê é uma fila com tempo por música (até 13 min).

use crate::game::{Game, Out};
use indexmap::IndexSet;
use serde_json::{Value, json};

pub const MAX_VIDS: usize = 4;
const KDEF: u64 = 780_000;
const KEXTRA: u64 = 21_000;
const KMAX: u64 = 780_000;
const KINVITE_MS: u64 = 15_000;

#[derive(Debug, Clone)]
pub struct Vid {
    pub vid: String,
    pub by: String,
    pub name: String,
    pub vs: (f64, bool, u64),
    pub viewers: IndexSet<String>,
    pub ended: u64,
    pub end_at: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct KarEntry {
    pub id: String,
    pub vid: Option<String>,
}

/// Hora (no relógio do servidor) em que o dono leu a posição: a do cliente se for plausível (até 3 s atrás).
fn sync_at(ts: &Value) -> u64 {
    let now = crate::now();
    match ts.as_f64() {
        Some(t) if t <= (now + 250) as f64 && t >= now as f64 - 3000.0 => (t as u64).min(now),
        _ => now,
    }
}

impl Game {
    fn player_conn(&self, code: &str, id: &str) -> Option<u64> {
        self.rooms.get(code)?.players.get(id)?.conn
    }

    pub fn drop_vid(&mut self, code: &str, by: &str, why: Option<&str>, except: Option<u64>) -> bool {
        let Some(r) = self.rooms.get_mut(code) else { return false };
        if r.vids.shift_remove(by).is_none() {
            return false;
        }
        let m = match why {
            Some(w) => json!({"t": "vstop", "by": by, "why": w}),
            None => json!({"t": "vstop", "by": by}),
        };
        self.bc(code, &m, except);
        true
    }

    /// Quem sai deixa de assistir e o vídeo dele acaba.
    pub fn vid_leave(&mut self, code: &str, by: &str) {
        if let Some(r) = self.rooms.get_mut(code) {
            for v in r.vids.values_mut() {
                v.viewers.shift_remove(by);
            }
        }
        self.drop_vid(code, by, None, None);
    }

    /// O vídeo de quem sai acaba; o DJ dele também.
    pub fn stop_video(&mut self, code: &str, by: &str) {
        self.vid_leave(code, by);
        if self.rooms.get(code).and_then(|r| r.dj.as_ref()).is_some_and(|d| d.1 == by) {
            self.dj_clear(code);
        }
    }

    pub fn dj_clear(&mut self, code: &str) {
        let Some(r) = self.rooms.get_mut(code) else { return };
        if r.dj.take().is_some() {
            self.radio.stop(code);
            self.bc(code, &json!({"t": "djstop"}), None);
        }
    }

    pub fn dj_msg(&mut self, cid: u64, t: &str, m: &Value, code: &str, me: &str) {
        let Some(r) = self.rooms.get(code) else { return };
        if t == "djstop" {
            if r.dj.as_ref().is_some_and(|d| d.1 == me) || (r.dj.is_some() && r.host.as_deref() == Some(me)) {
                self.dj_clear(code);
            }
            return;
        }
        if r.dj.as_ref().is_some_and(|d| d.1 != me) {
            return self.send(cid, &json!({"t": "djbusy"}));
        }
        let Some(tx) = self.later.clone() else { return };
        let url = m["url"].as_str().unwrap_or("").to_owned();
        let (code, me) = (code.to_owned(), me.to_owned());
        tokio::spawn(async move {
            let r = crate::radio::probe(&url).await;
            let _ = tx.send(Box::new(move |g: &mut Game| {
                let Some(room) = g.rooms.get(&code).filter(|r| r.players.contains_key(&me)) else { return };
                let conn = room.players.get(&me).and_then(|p| p.conn);
                let name = room.players.get(&me).map(|p| p.name.clone()).unwrap_or_default();
                if room.dj.as_ref().is_some_and(|d| d.1 != me) {
                    if let Some(c) = conn {
                        g.send(c, &json!({"t": "djbusy"}));
                    }
                    return;
                }
                if let Err(why) = r {
                    if let Some(c) = conn {
                        g.send(c, &json!({"t": "djerr", "msg": crate::radio::reason_text(why)}));
                    }
                    return;
                }
                if room.dj.is_some() {
                    g.radio.stop(&code);
                }
                let host = url::Url::parse(&url)
                    .ok()
                    .and_then(|u| {
                        u.host_str().map(|h| match u.port() {
                            Some(p) => format!("{h}:{p}"),
                            None => h.to_owned(),
                        })
                    })
                    .unwrap_or_default();
                if let Some(room) = g.rooms.get_mut(&code) {
                    room.dj = Some((url.clone(), me.clone(), name.clone()));
                }
                g.bc(&code, &json!({"t": "dj", "url": "", "host": host, "name": name, "by": me}), None);
            }));
        });
    }

    /// O que quem entra precisa saber: vídeos tocando, rádio e karaokê.
    pub fn media_greet(&mut self, cid: u64, code: &str) {
        let Some(r) = self.rooms.get(code) else { return };
        if let Some((url, by, name)) = r.dj.clone() {
            let host = url::Url::parse(&url)
                .ok()
                .and_then(|u| {
                    u.host_str().map(|h| match u.port() {
                        Some(p) => format!("{h}:{p}"),
                        None => h.to_owned(),
                    })
                })
                .unwrap_or_default();
            self.send(cid, &json!({"t": "dj", "url": "", "host": host, "name": name, "by": by}));
        }
        let Some(r) = self.rooms.get(code) else { return };
        let msgs: Vec<Value> = r
            .vids
            .values()
            .filter(|v| v.ended == 0)
            .map(|v| json!({"t": "vinv", "vid": v.vid, "name": v.name, "by": v.by}))
            .collect();
        let k = r.kar.first().map(|k| r.players.get(&k.id).map_or("Alguém".to_owned(), |p| p.name.clone()));
        for m in msgs {
            self.send(cid, &m);
        }
        if let Some(name) = k {
            self.send(cid, &json!({"t": "kinv", "name": name}));
        }
    }

    fn send_viewers(&mut self, code: &str, by: &str, msg: &Value) {
        let Some(r) = self.rooms.get(code) else { return };
        let Some(v) = r.vids.get(by) else { return };
        let to: Vec<u64> = v.viewers.iter().filter_map(|id| r.players.get(id).and_then(|p| p.conn)).collect();
        for c in to {
            self.send(c, msg);
        }
    }

    pub fn video_msg(&mut self, cid: u64, t: &str, m: &Value, code: &str, me: &str) {
        let idle = self.video_idle_ms;
        let my_name = self.rooms.get(code).and_then(|r| r.players.get(me)).map(|p| p.name.clone()).unwrap_or_default();
        let Some(r) = self.rooms.get_mut(code) else { return };
        match t {
            "video" => {
                let vid = m["vid"].as_str().unwrap_or("").to_owned();
                let cur = r.vids.get(me).cloned();
                let swap = cur.is_some();
                if cur.as_ref().is_some_and(|c| c.vid == vid && c.ended == 0) {
                    return;
                }
                if !swap && r.vids.len() >= MAX_VIDS {
                    return self.send(cid, &json!({"t": "vbusy"}));
                }
                for v in r.vids.values_mut() {
                    v.viewers.shift_remove(me);
                }
                let viewers = cur.map(|c| c.viewers).unwrap_or_default();
                r.vids.insert(
                    me.into(),
                    Vid {
                        vid: vid.clone(),
                        by: me.into(),
                        name: my_name.clone(),
                        vs: (0.0, true, crate::now()),
                        viewers,
                        ended: 0,
                        end_at: None,
                    },
                );
                let mut msg = json!({"t": "vinv", "vid": vid, "name": my_name, "by": me});
                if swap {
                    msg["swap"] = json!(1);
                    self.bc(code, &msg, None);
                } else {
                    self.bc(code, &msg, Some(cid));
                }
            }
            "vsync" => {
                let Some(v) = r.vids.get_mut(me).filter(|v| v.ended == 0) else { return };
                let at = sync_at(&m["ts"]);
                v.vs = (m["time"].as_f64().unwrap_or(0.0), m["playing"] == true, at);
                self.send_viewers(
                    code,
                    me,
                    &json!({"t": "vsync", "by": me, "time": m["time"], "playing": m["playing"], "at": at}),
                );
            }
            "vend" => {
                let Some(v) = r.vids.get_mut(me).filter(|v| v.ended == 0) else { return };
                let now = crate::now();
                v.ended = now;
                v.end_at = Some(now + idle);
                self.send_viewers(code, me, &json!({"t": "vend", "by": me, "left": idle}));
            }
            "vstop" => {
                let by = m["by"].as_str().unwrap_or(me).to_owned();
                let host = r.host.as_deref() == Some(me);
                if r.vids.contains_key(&by) && (by == me || host) {
                    self.drop_vid(code, &by, None, Some(cid));
                }
            }
            "vjoin" => {
                let by = m["by"].as_str().unwrap_or("").to_owned();
                if !r.vids.contains_key(&by) || by == me {
                    return;
                }
                self.drop_vid(code, me, None, Some(cid));
                let Some(r) = self.rooms.get_mut(code) else { return };
                for o in r.vids.values_mut() {
                    o.viewers.shift_remove(me);
                }
                let Some(v) = r.vids.get_mut(&by) else { return };
                v.viewers.insert(me.into());
                let out = if v.ended > 0 {
                    json!({"t": "vend", "by": v.by, "left": idle.saturating_sub(crate::now() - v.ended)})
                } else {
                    json!({"t": "vsync", "by": v.by, "time": v.vs.0, "playing": v.vs.1, "at": v.vs.2})
                };
                self.send(cid, &out);
            }
            "vleave" => {
                if let Some(v) = r.vids.get_mut(m["by"].as_str().unwrap_or("")) {
                    v.viewers.shift_remove(me);
                }
            }
            _ => {}
        }
    }

    /// Vídeos que terminaram e ficaram parados além do prazo.
    pub fn video_timers(&mut self, now: u64) {
        let due: Vec<(String, String)> = self
            .rooms
            .values()
            .flat_map(|r| {
                r.vids.values().filter(|v| v.end_at.is_some_and(|e| now >= e)).map(|v| (r.code.clone(), v.by.clone()))
            })
            .collect();
        for (code, by) in due {
            self.drop_vid(&code, &by, Some("idle"), None);
        }
    }

    // ---------- karaokê ----------

    pub fn kq(&self, code: &str) -> Value {
        let Some(r) = self.rooms.get(code) else { return json!([]) };
        Value::Array(r.kar.iter().map(|k| json!({"id": k.id, "name": r.players.get(&k.id).map_or("?", |p| p.name.as_str()), "vid": k.vid})).collect())
    }

    fn kstart(&mut self, code: &str) {
        let Some(r) = self.rooms.get_mut(code) else { return };
        r.ks = None;
        let vid = r.kar.first().is_some_and(|h| h.vid.is_some());
        let now = crate::now();
        r.kar_end = now + if vid { KDEF } else { 120_000 };
        r.kar_at = now;
        r.kdur = false;
        r.kcap = vid;
        r.kar_turn += 1;
    }

    fn kbc(&mut self, code: &str) {
        let Some(r) = self.rooms.get_mut(code) else { return };
        let h = r.kar.first().map(|k| k.id.clone());
        if h != r.kar_head {
            r.kar_head = h.clone();
            if h.is_some() {
                self.kstart(code);
            }
        }
        let Some(r) = self.rooms.get(code) else { return };
        let (left, turn) = (r.kar_end.saturating_sub(crate::now()), r.kar_turn);
        let q = self.kq(code);
        self.bc(code, &json!({"t": "kar", "q": q, "left": left, "turn": turn}), None);
    }

    fn front(&mut self, code: &str, pref_vid: bool) {
        let Some(r) = self.rooms.get_mut(code) else { return };
        let online = |r: &crate::game::Room, k: &KarEntry| r.players.get(&k.id).is_some_and(|p| p.online);
        let vid = pref_vid && r.kar.iter().any(|k| k.vid.is_some() && online(r, k));
        for _ in 0..r.kar.len() {
            let Some(h) = r.kar.first() else { break };
            if online(r, h) && (!vid || h.vid.is_some()) {
                break;
            }
            let x = r.kar.remove(0);
            r.kar.push(x);
        }
    }

    fn krot(&mut self, code: &str) {
        let Some(r) = self.rooms.get_mut(code) else { return };
        if r.kar.is_empty() {
            return;
        }
        let mut f = r.kar.remove(0);
        f.vid = None;
        r.kar.push(f);
        self.front(code, true);
        if let Some(r) = self.rooms.get_mut(code) {
            r.kar_head = r.kar.first().map(|k| k.id.clone());
        }
        self.kstart(code);
        self.kbc(code);
    }

    fn kinvite(&mut self, code: &str, from: &str) {
        let now = crate::now();
        let Some(r) = self.rooms.get_mut(code) else { return };
        if now.saturating_sub(r.invite_at) < KINVITE_MS {
            return;
        }
        r.invite_at = now;
        let name = r.players.get(from).map(|p| p.name.clone()).unwrap_or_default();
        let to: Vec<u64> = r
            .players
            .values()
            .filter(|p| !p.bot && p.ready && p.online && p.id != from && !r.kar.iter().any(|k| k.id == p.id))
            .filter_map(|p| p.conn)
            .collect();
        for c in to {
            self.send(c, &json!({"t": "kinv", "name": name, "by": from}));
        }
    }

    /// Sai da fila (e o próximo assume quando era a vez dele).
    pub fn kar_drop(&mut self, code: &str, id: &str) {
        let Some(r) = self.rooms.get_mut(code) else { return };
        if !r.kar.iter().any(|k| k.id == id) {
            return;
        }
        let head = r.kar.first().is_some_and(|k| k.id == id);
        r.kar.retain(|k| k.id != id);
        if head {
            self.front(code, false);
        }
        self.kbc(code);
    }

    /// Caiu a conexão: quem estava cantando sai; quem esperava a vez mantém o lugar.
    pub fn kar_away(&mut self, code: &str, id: &str) {
        if self.rooms.get(code).and_then(|r| r.kar.first()).is_some_and(|k| k.id == id) {
            self.kar_drop(code, id);
        }
    }

    pub fn kar_tick(&mut self, code: &str, now: u64) {
        let Some(r) = self.rooms.get(code) else { return };
        if r.kar.is_empty() || now < r.kar_end {
            return;
        }
        let text = if r.kcap {
            "⏱ Limite de 13 min do karaokê atingido! Próximo cantor."
        } else {
            "Fim da música! Próximo cantor."
        };
        self.bc(code, &json!({"t": "chat", "name": "🎶 Karaokê", "text": text}), None);
        self.krot(code);
    }

    fn singing(&self, code: &str, me: &str) -> bool {
        self.rooms.get(code).and_then(|r| r.kar.first()).is_some_and(|k| k.id == me)
    }

    pub fn karaoke_msg(&mut self, cid: u64, t: &str, m: &Value, code: &str, me: &str) {
        let max_q = self.set.max_per_room as usize;
        match t {
            "kjoin" => {
                let Some(r) = self.rooms.get_mut(code) else { return };
                if r.kar.iter().any(|k| k.id == me) || r.kar.len() >= max_q {
                    return;
                }
                let was = r.kar.is_empty();
                r.kar.push(KarEntry { id: me.into(), vid: m["vid"].as_str().map(str::to_owned) });
                let ks = r.ks.clone();
                self.kbc(code);
                if was {
                    self.kinvite(code, me);
                } else if let Some(ks) = ks {
                    self.send(cid, &ks);
                }
            }
            "ksong" => {
                let Some(r) = self.rooms.get_mut(code) else { return };
                let Some(i) = r.kar.iter().position(|k| k.id == me) else { return };
                let hd = i == 0;
                if hd && r.kar[0].vid.is_some() {
                    return;
                }
                r.kar[i].vid = m["vid"].as_str().map(str::to_owned);
                if hd {
                    self.kstart(code);
                }
                self.kbc(code);
            }
            "kleave" => self.kar_drop(code, me),
            "kpass" => {
                if self.singing(code, me) {
                    self.krot(code);
                }
            }
            "kdur" => {
                if !self.singing(code, me) {
                    return;
                }
                let Some(r) = self.rooms.get_mut(code) else { return };
                if r.kdur {
                    return;
                }
                r.kdur = true;
                let cap = if r.kar_at > 0 { r.kar_at } else { crate::now() } + KMAX;
                let end = crate::now() + (m["d"].as_f64().unwrap_or(0.0) * 1000.0) as u64 + KEXTRA;
                r.kar_end = end.min(cap);
                r.kcap = end >= cap;
                self.kbc(code);
            }
            "kend" => {
                if self.singing(code, me)
                    && self.rooms.get(code).is_some_and(|r| crate::now().saturating_sub(r.kar_at) > 10_000)
                {
                    self.krot(code);
                }
            }
            "ksync" => {
                if !self.singing(code, me) {
                    return;
                }
                let o = json!({"t": "ksync", "time": m["time"], "playing": m["playing"], "at": sync_at(&m["ts"])});
                let Some(r) = self.rooms.get_mut(code) else { return };
                r.ks = Some(o.clone());
                let to: Vec<u64> = r
                    .players
                    .values()
                    .filter(|p| p.id != me && !p.bot && p.ready && p.online && r.kar.iter().any(|k| k.id == p.id))
                    .filter_map(|p| p.conn)
                    .collect();
                let s: std::sync::Arc<str> = std::sync::Arc::from(crate::jsjson::to_string(&o));
                for c in to {
                    self.send_raw_to(c, Out::Text(s.clone()), s.len());
                }
            }
            _ => {}
        }
        let _ = self.player_conn(code, me);
    }
}
