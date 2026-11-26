//! Voz (`lib/voicews.js` e a parte de voz de `server.js`). Pelo WebSocket (padrão) o servidor repassa os quadros de áudio
//! só para quem está ouvindo e não silenciou quem fala, sem decodificar nem gravar. Pela malha WebRTC ele só repassa a
//! sinalização e, no modo privado, deixa passar apenas candidatos "relay" (TURN), sem endereços.

use crate::game::{Game, Out};
use base64::Engine;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;

const FRAME_IN: u8 = 2;
const MAX_FRAME: usize = 600;
const MAX_FPS: u32 = 50;
const MAX_BPS: u64 = 12000;
const VOICE_BUFFER: usize = 64 * 1024;
const VOICE_RESUME: usize = 16 * 1024;

/// Configuração da voz lida na subida.
#[derive(Debug, Clone, Default)]
pub struct VoiceCfg {
    pub turn_urls: Vec<String>,
    pub turn_user: String,
    pub turn_pass: String,
    pub turn_secret: String,
    pub turn_ttl: u64,
    pub turn_label: String,
}

impl VoiceCfg {
    pub fn turn_ok(&self) -> bool {
        !self.turn_urls.is_empty()
            && (!self.turn_secret.is_empty() || (!self.turn_user.is_empty() && !self.turn_pass.is_empty()))
    }
}

/// Medidores da voz para o painel.
#[derive(Debug, Default, Clone, Copy)]
pub struct VoicePerf {
    pub v_in: u64,
    pub v_out: u64,
    pub v_drop: u64,
    pub v_bytes: u64,
    pub v_lim: u64,
    pub v_bad: u64,
}

/// Janela de 1 s por quem fala: no máximo 50 quadros e 12 KB.
fn frame_ok(w: &mut (u64, u32, u64), now: u64, len: u64) -> bool {
    if now - w.0 >= 1000 {
        *w = (now, 0, 0);
    }
    if w.1 >= MAX_FPS || w.2 + len > MAX_BPS {
        return false;
    }
    w.1 += 1;
    w.2 += len;
    true
}

/// Cabe mais voz para este ouvinte? Quem passou de 64 KB na fila só volta abaixo de 16 KB.
fn roomy(vslow: &mut bool, buffered: usize) -> bool {
    if *vslow {
        if buffered > VOICE_RESUME {
            return false;
        }
        *vslow = false;
        return true;
    }
    if buffered <= VOICE_BUFFER {
        return true;
    }
    *vslow = true;
    false
}

fn no_raddr(s: &str) -> String {
    use std::sync::OnceLock;
    static R: OnceLock<Option<regex::Regex>> = OnceLock::new();
    match R.get_or_init(|| regex::Regex::new(r" raddr \S+ rport \d+").ok()) {
        Some(r) => r.replace_all(s, " raddr 0.0.0.0 rport 0").into_owned(),
        None => s.into(),
    }
}

/// Candidato de verdade do tipo relay (o tipo na posição da gramática do SDP).
fn relay_cand(c: &str) -> bool {
    use std::sync::OnceLock;
    static R: OnceLock<Option<regex::Regex>> = OnceLock::new();
    if c.is_empty() {
        return true;
    }
    R.get_or_init(|| regex::Regex::new(r"(?i)^candidate:\S+ \d+ (?:udp|tcp) \d+ \S+ \d+ typ (\w+)(?: |$)").ok())
        .as_ref()
        .and_then(|r| r.captures(c))
        .and_then(|m| m.get(1))
        .is_some_and(|t| t.as_str() == "relay")
}

fn relay_sdp(sdp: &str) -> String {
    use std::sync::OnceLock;
    static RTCP: OnceLock<Option<regex::Regex>> = OnceLock::new();
    static ORIG: OnceLock<Option<regex::Regex>> = OnceLock::new();
    static TAIL: OnceLock<Option<regex::Regex>> = OnceLock::new();
    let rtcp = RTCP.get_or_init(|| regex::Regex::new(r"^a=rtcp:\d+ IN IP[46] ").ok());
    let orig = ORIG.get_or_init(|| regex::Regex::new(r"^o=\S+ \S+ \S+ IN IP[46] ").ok());
    let tail = TAIL.get_or_init(|| regex::Regex::new(r" IN IP[46] .*$").ok());
    let cut = |l: &str| tail.as_ref().map_or(l.to_owned(), |t| t.replace(l, " IN IP4 0.0.0.0").into_owned());
    sdp.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .filter(|l| !(l.starts_with("a=candidate:") && !relay_cand(&l[2..])) && !l.starts_with("a=remote-candidates:"))
        .map(|l| {
            if l.starts_with("a=candidate:") {
                no_raddr(l)
            } else if l.starts_with("c=IN IP4 ") {
                "c=IN IP4 0.0.0.0".into()
            } else if l.starts_with("c=IN IP6 ") {
                "c=IN IP6 ::".into()
            } else if rtcp.as_ref().is_some_and(|r| r.is_match(l)) || orig.as_ref().is_some_and(|r| r.is_match(l)) {
                cut(l)
            } else {
                l.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\r\n")
}

impl Game {
    pub fn voice_on(&self) -> bool {
        self.set.voice_ws || self.set.voice_privacy == "direct" || self.voice.turn_ok()
    }

    /// Servidores ICE para o navegador (`iceCfg`), com credenciais TURN temporárias quando há segredo.
    pub fn ice_config(&self) -> Value {
        if self.set.voice_ws {
            return json!({"iceServers": [], "voice": true, "ws": true});
        }
        let relay = self.set.voice_privacy == "relay";
        let mut ice: Vec<Value> =
            if relay { Vec::new() } else { vec![json!({"urls": "stun:stun.l.google.com:19302"})] };
        let v = &self.voice;
        if v.turn_ok() {
            let (mut u, mut c) = (v.turn_user.clone(), v.turn_pass.clone());
            if !v.turn_secret.is_empty() {
                u = format!("{}:{}", crate::now() / 1000 + v.turn_ttl, v.turn_label);
                if let Ok(mut m) = Hmac::<sha1::Sha1>::new_from_slice(v.turn_secret.as_bytes()) {
                    m.update(u.as_bytes());
                    c = base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes());
                }
            }
            ice.push(json!({"urls": v.turn_urls, "username": u, "credential": c}));
        }
        if relay {
            json!({"iceServers": ice, "policy": "relay", "voice": self.voice_on(), "reason": if v.turn_ok() { Value::Null } else { json!("turn") }})
        } else {
            json!({"iceServers": ice, "voice": true})
        }
    }

    /// Quadro de voz de quem está na sala; `false` só para quadro malformado (vira violação).
    pub fn voice_frame(&mut self, cid: u64, raw: &[u8]) -> bool {
        let Some(c) = self.conns.get(&cid) else { return true };
        let (Some(code), Some(me)) = (c.room.clone(), c.me.clone()) else { return true };
        if !self.set.voice_ws {
            return true;
        }
        let now = crate::now();
        let Some(r) = self.rooms.get_mut(&code) else { return true };
        let Some(p) = r.players.get_mut(&me) else { return true };
        if !p.ready || !p.talk {
            return true;
        }
        if raw.len() < 4 || raw.len() > MAX_FRAME || raw[0] != FRAME_IN || !(raw[1] == 1 || raw[1] == 2) {
            self.vperf.v_bad += 1;
            return false;
        }
        if !frame_ok(&mut p.vfw, now, raw.len() as u64) {
            self.vperf.v_lim += 1;
            return true;
        }
        self.vperf.v_in += 1;
        let id_num: u32 = me[1..].parse::<u64>().map_or(0, |n| (n & 0x7fff_ffff) as u32);
        let mut out = Vec::with_capacity(4 + raw.len());
        out.push(FRAME_IN);
        out.extend_from_slice(&id_num.to_le_bytes());
        out.extend_from_slice(&raw[1..3]);
        out.extend_from_slice(&raw[3..]);
        let buf: Arc<[u8]> = Arc::from(out);
        let targets: Vec<(String, u64)> = r
            .players
            .values()
            .filter(|q| q.id != me && !q.bot && q.ready && q.online && q.vlisten && !q.vmute.contains(&me))
            .filter_map(|q| q.conn.map(|c| (q.id.clone(), c)))
            .collect();
        for (pid, conn) in targets {
            let buffered =
                self.conns.get(&conn).and_then(|c| c.link.as_ref()).map_or(0, |l| l.buffered.load(Ordering::Relaxed));
            let ok = self
                .rooms
                .get_mut(&code)
                .and_then(|r| r.players.get_mut(&pid))
                .is_some_and(|q| roomy(&mut q.vslow, buffered));
            if !ok {
                self.vperf.v_drop += 1;
                continue;
            }
            self.send_raw_to(conn, Out::Bin(buf.clone()), buf.len());
            self.vperf.v_out += 1;
            self.vperf.v_bytes += buf.len() as u64;
        }
        true
    }

    /// Sinalização WebRTC (`sig`): só entre dois com o microfone ligado, com teto por par e o filtro de privacidade.
    pub fn voice_sig(&mut self, code: &str, me: &str, m: &Value) {
        if self.set.voice_ws || !self.voice_on() {
            return;
        }
        let to_id = m["to"].as_str().unwrap_or("").to_owned();
        let data = &m["data"];
        let Some(r) = self.rooms.get(code) else { return };
        let Some(to) = r.players.get(&to_id).filter(|p| !p.bot && p.id != me && p.online && p.conn.is_some()).cloned()
        else {
            return;
        };
        let Some(mep) = r.players.get(me).cloned() else { return };
        let x = data["x"].as_str().unwrap_or("");
        if x != "close" && x != "busy" && (!mep.vlisten || !to.vlisten) {
            return;
        }
        let now = crate::now();
        let Some(p) = self.rooms.get_mut(code).and_then(|r| r.players.get_mut(me)) else { return };
        let w = match p.sig_w.get(&to_id) {
            Some(w) if now - w.0 <= 5000 => *w,
            _ => {
                if p.sig_w.len() >= 64 && !p.sig_w.contains_key(&to_id) {
                    p.sig_w.shift_remove_index(0);
                }
                (now, 0, 0)
            }
        };
        let mut w = w;
        w.2 += 1;
        if data.get("d").is_some() {
            w.1 += 1;
        }
        p.sig_w.insert(to_id.clone(), w);
        if w.2 > 120 || (data.get("d").is_some() && w.1 > 4) {
            return;
        }
        let sd = if !x.is_empty() || self.set.voice_privacy != "relay" {
            data.clone()
        } else if let Some(d) = data.get("d") {
            json!({"d": {"type": d["type"], "sdp": relay_sdp(d["sdp"].as_str().unwrap_or(""))}})
        } else if let Some(c) = data.get("c") {
            let cand = c["candidate"].as_str().unwrap_or("");
            if !relay_cand(cand) {
                return;
            }
            let mut c2 = c.clone();
            c2["candidate"] = json!(no_raddr(cand));
            json!({"c": c2})
        } else {
            data.clone()
        };
        if let Some(conn) = to.conn {
            self.send(conn, &json!({"t": "sig", "from": me, "data": sd}));
        }
    }

    pub fn voice_now(&self) -> Value {
        let (mut talking, mut listening, mut congested) = (0, 0, 0);
        for r in self.rooms.values() {
            for p in r.players.values().filter(|p| !p.bot && p.online) {
                talking += usize::from(p.talk);
                listening += usize::from(p.vlisten);
                congested += usize::from(p.vslow);
            }
        }
        json!({"talking": talking, "listening": listening, "congested": congested})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn privacy_filter() {
        assert!(relay_cand("candidate:1 1 udp 2 1.2.3.4 5 typ relay raddr 9.9.9.9 rport 7"));
        assert!(!relay_cand("candidate:1 1 udp 2 1.2.3.4 5 typ host x typ relay"));
        let sdp = "o=- 1 2 IN IP4 10.0.0.1\r\nc=IN IP4 10.0.0.1\r\na=candidate:1 1 udp 2 10.0.0.1 5 typ host\r\na=candidate:2 1 udp 2 3.3.3.3 5 typ relay raddr 9.9.9.9 rport 7";
        let out = relay_sdp(sdp);
        assert!(!out.contains("10.0.0.1") && !out.contains("typ host") && out.contains("raddr 0.0.0.0 rport 0"));
        let mut w = (0, 0, 0);
        assert!(frame_ok(&mut w, 1000, 100));
        assert!(!frame_ok(&mut w, 1000, 12000));
    }
}
