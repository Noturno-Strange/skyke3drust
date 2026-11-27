//! Moderação (`lib/reports.js`, `lib/report-hook.js` e o painel `/status` de `server.js`): denúncias com o registro de
//! voz (quem falou e quando, nunca o áudio), banimento por IP sem expor o IP (só o hash), capacidade, emblemas,
//! renovação extra da chave de recuperação (keypass) e recuperação assistida.

use crate::game::{Game, pid_of};
use crate::persist::{read_json_file, write_json_atomic};
use crate::validate::REPORT_REASONS;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

pub const BAN_MAX_MIN: i64 = 43200;
const MAX: usize = 500;
const TTL: u64 = 30 * 86_400_000;
const PER_AUTHOR: usize = 10;

fn reason_text(r: &str) -> &'static str {
    REPORT_REASONS.iter().find(|x| x.0 == r).map_or("", |x| x.1)
}

/// Rótulo curto da sala (sem revelar o código de entrada).
pub fn room_tag(code: &str) -> String {
    crate::hex(&Sha256::digest(code.as_bytes()))[..4].to_owned()
}

#[derive(Debug)]
pub struct Reports {
    file: Option<PathBuf>,
    pub list: Vec<Value>,
    dirty: bool,
    due: Option<u64>,
}

impl Reports {
    pub fn new(file: &str) -> Self {
        let mut r =
            Self { file: (!file.is_empty()).then(|| PathBuf::from(file)), list: Vec::new(), dirty: false, due: None };
        if let Some(f) = &r.file {
            match read_json_file(f) {
                Ok(Some(j)) => {
                    let now = crate::now();
                    for x in j["reports"].as_array().into_iter().flatten() {
                        let ok = x["id"].as_str().is_some_and(|i| {
                            i.len() == 12 && i.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
                        }) && x["at"].as_f64().is_some_and(|a| (now as f64) - a < TTL as f64)
                            && x["tk"].is_string()
                            && !reason_text(x["reason"].as_str().unwrap_or("")).is_empty()
                            && x["by"].is_object()
                            && x["target"].is_object()
                            && x["chat"].is_array();
                        if ok {
                            r.list.push(x.clone());
                        }
                    }
                    r.list.truncate(MAX);
                }
                Ok(None) => {}
                Err(_) => tracing::warn!("denúncias: arquivo inválido, começando vazio."),
            }
        }
        r
    }

    pub fn write(&mut self) {
        let Some(f) = self.file.clone() else { return };
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Err(e) = write_json_atomic(&f, &json!({"v": 1, "reports": self.list}), false) {
            tracing::error!("denúncias: {e}");
        }
    }

    pub fn save(&mut self) {
        self.dirty = true;
        self.due.get_or_insert(crate::now() + 3000);
    }

    pub fn tick(&mut self, now: u64) {
        if self.due.is_some_and(|d| now >= d) {
            self.due = None;
            self.write();
        }
    }

    fn expire(&mut self) {
        let t = crate::now() as f64;
        self.list.retain(|r| t - r["at"].as_f64().unwrap_or(0.0) <= TTL as f64);
    }

    fn evict(&mut self) {
        while self.list.len() > MAX {
            let i = match self.list.iter().rposition(|x| x["status"] != "open") {
                Some(i) => i,
                None => {
                    let mut n: indexmap::IndexMap<String, usize> = indexmap::IndexMap::new();
                    for x in &self.list {
                        *n.entry(x["rk"].as_str().unwrap_or("").to_owned()).or_default() += 1;
                    }
                    let top = n.iter().fold(None::<(&String, usize)>, |t, (k, c)| {
                        if t.is_none_or(|t| *c > t.1) { Some((k, *c)) } else { t }
                    });
                    let alvo = top.map(|t| t.0.clone()).unwrap_or_default();
                    self.list.iter().rposition(|x| x["rk"] == alvo.as_str()).unwrap_or(self.list.len() - 1)
                }
            };
            self.list.remove(i);
        }
    }

    fn add(&mut self, r: Value) -> bool {
        if self.list.iter().filter(|x| x["rk"] == r["rk"] && x["status"] == "open").count() >= PER_AUTHOR {
            return false;
        }
        self.list.insert(0, r);
        self.evict();
        self.save();
        true
    }

    fn recent(&self, rk: &str, tk: &str, ms: f64) -> bool {
        let t = crate::now() as f64;
        self.list.iter().any(|r| r["rk"] == rk && r["tk"] == tk && t - r["at"].as_f64().unwrap_or(0.0) < ms)
    }
}

fn md_esc(s: &str) -> String {
    let mut o = String::new();
    for c in s.chars() {
        if "\\`*_~|>#[]()".contains(c) {
            o.push('\\');
        }
        o.push(c);
        if c == '@' {
            o.push('\u{200b}');
        }
    }
    o
}

fn clip(s: &str, n: usize) -> String {
    if crate::validate::len16(s) > n { crate::validate::slice16(s, n - 1) + "…" } else { s.to_owned() }
}

/// Corpo do aviso no Discord (`hookBody`).
pub fn hook_body(r: &Value) -> Value {
    let tv: Vec<&Value> = r["voice"].as_array().into_iter().flatten().filter(|x| x["who"] == "alvo").collect();
    let vs = if tv.is_empty() {
        String::new()
    } else {
        let ms: f64 = tv.iter().map(|x| x["to"].as_f64().unwrap_or(0.0) - x["from"].as_f64().unwrap_or(0.0)).sum();
        format!("{} fala(s), {} s no total nos últimos 10 min", tv.len(), (ms / 1000.0).round())
    };
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    let mut fields = vec![
        json!({"name": "Denunciado", "value": clip(&format!("{} · ID {}", md_esc(&s(&r["target"]["name"])), s(&r["target"]["host"])), 1000), "inline": true}),
        json!({"name": "Denunciante", "value": clip(&format!("{} · ID {}", md_esc(&s(&r["by"]["name"])), s(&r["by"]["host"])), 1000), "inline": true}),
        json!({"name": "Sala", "value": format!("{} · {}", s(&r["room"]), s(&r["style"])), "inline": true}),
    ];
    if !s(&r["note"]).is_empty() {
        fields.push(json!({"name": "Observação", "value": clip(&md_esc(&s(&r["note"])), 1000)}));
    }
    if !vs.is_empty() {
        fields.push(json!({"name": "Voz do denunciado", "value": vs}));
    }
    fields.push(json!({"name": "Denúncia", "value": r["id"]}));
    json!({
        "username": "Skyke3D · Denúncias", "allowed_mentions": {"parse": []},
        "embeds": [{
            "title": clip(&format!("🚩 {}", reason_text(&s(&r["reason"]))), 250), "color": 0xff5a5a,
            "timestamp": crate::seclog::iso(r["at"].as_u64().unwrap_or(0)), "fields": fields,
            "footer": {"text": "Para punir: painel /status → Moderação → Denúncias"}
        }]
    })
}

/// Endereço do webhook do Discord, se válido.
pub fn webhook_url(raw: &str) -> String {
    let u = raw.trim();
    let ok = [
        "https://discord.com/api/webhooks/",
        "https://discordapp.com/api/webhooks/",
        "https://ptb.discord.com/api/webhooks/",
        "https://canary.discord.com/api/webhooks/",
        "https://ptb.discordapp.com/api/webhooks/",
        "https://canary.discordapp.com/api/webhooks/",
    ]
    .iter()
    .find_map(|p| u.strip_prefix(p))
    .and_then(|rest| rest.split_once('/'))
    .is_some_and(|(id, tk)| {
        (5..=30).contains(&id.len())
            && id.bytes().all(|b| b.is_ascii_digit())
            && (20..=100).contains(&tk.len())
            && tk.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    });
    if u.is_empty() {
        return String::new();
    }
    if !ok {
        tracing::warn!("DISCORD_REPORT_WEBHOOK ignorado: use a URL de um webhook do Discord.");
        return String::new();
    }
    u.to_owned()
}

fn out(code: u16, v: Value) -> (u16, Value) {
    (code, v)
}

impl Game {
    /// Denúncia feita por um jogador da sala.
    pub fn report_new(&mut self, code: &str, me: &str, m: &Value) -> Value {
        let Some(room) = self.rooms.get(code) else { return json!({"err": "Esse jogador não está nesta sala."}) };
        let id = m["id"].as_str().unwrap_or("");
        let Some(tg) = room.players.get(id).filter(|p| !p.bot && !p.ip.is_empty()).cloned() else {
            return json!({"err": "Esse jogador não está nesta sala."});
        };
        let Some(mep) = room.players.get(me).cloned() else {
            return json!({"err": "Esse jogador não está nesta sala."});
        };
        if tg.id == mep.id {
            return json!({"err": "Você não pode denunciar a si mesmo."});
        }
        if self.over(&mep.ip, "rp", 6, 3_600_000) {
            return json!({"err": "Você já enviou muitas denúncias. Tente de novo mais tarde."});
        }
        if self.over("@", "rpg", 120, 3_600_000) {
            return json!({"err": "Muitas denúncias chegando agora. Tente de novo em alguns minutos."});
        }
        let now = crate::now();
        let (tk, rk) = (self.bm.key(&tg.ip), self.bm.key(&mep.ip));
        self.reports.expire();
        if self.reports.recent(&rk, &tk, 600_000.0) {
            return json!({"ok": 1, "dup": 1});
        }
        let voice: Vec<Value> = self
            .voice_recent(code, now)
            .into_iter()
            .map(|x| {
                let who = if x.0 == tg.id {
                    "alvo"
                } else if x.0 == mep.id {
                    "autor"
                } else {
                    ""
                };
                json!({"from": x.2, "to": x.3, "name": x.1, "live": i32::from(x.4), "who": who})
            })
            .collect();
        let style = self.rooms.get(code).map(|r| r.style.clone()).unwrap_or_default();
        let r = json!({
            "id": crate::hex(&crate::random_bytes::<6>()), "at": now, "room": room_tag(code), "style": style,
            "reason": m["reason"], "note": m["note"].as_str().unwrap_or(""),
            "by": {"name": mep.name, "host": self.bm.host_id(&mep.ip), "pid": mep.id},
            "target": {"name": tg.name, "host": self.bm.host_id(&tg.ip), "pid": tg.id},
            "chat": [], "voice": voice, "status": "open", "tk": tk, "rk": rk
        });
        if !self.reports.add(r.clone()) {
            return json!({"err": "Você já tem muitas denúncias em análise. Aguarde a moderação."});
        }
        self.notify_hook(&r);
        tracing::warn!(
            "denúncia {} ({}) na sala {}",
            r["id"].as_str().unwrap_or(""),
            m["reason"].as_str().unwrap_or(""),
            room_tag(code)
        );
        json!({"ok": 1})
    }

    /// Falas dos últimos 10 min: (id, nome, de, até, ao vivo).
    fn voice_recent(&self, code: &str, now: u64) -> Vec<(String, String, u64, u64, bool)> {
        let Some(r) = self.rooms.get(code) else { return Vec::new() };
        let mut l: Vec<(String, String, u64, u64, bool)> = r
            .vlog
            .iter()
            .filter(|x| now - x.to <= 600_000)
            .map(|x| (x.id.clone(), x.name.clone(), x.from, x.to, false))
            .collect();
        for p in r.players.values() {
            if p.talk && p.talk_at > 0 {
                l.push((p.id.clone(), p.name.clone(), p.talk_at, now, true));
            }
        }
        let n = l.len();
        l.split_off(n.saturating_sub(40))
    }

    fn notify_hook(&mut self, r: &Value) {
        if self.webhook.is_empty() {
            return;
        }
        let now = crate::now();
        if now - self.hook_at > 60000 {
            self.hook_at = now;
            self.hook_n = 0;
        }
        self.hook_n += 1;
        if self.hook_n > 20 {
            return;
        }
        let (url, body) = (self.webhook.clone(), hook_body(r));
        tokio::spawn(async move {
            let c = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .redirect(reqwest::redirect::Policy::none())
                .build();
            match c {
                Ok(c) => match c.post(url).json(&body).send().await {
                    Ok(res) if !res.status().is_success() => {
                        tracing::warn!("denúncia: webhook respondeu {}", res.status())
                    }
                    Ok(_) => {}
                    Err(_) => tracing::warn!("denúncia: webhook indisponível"),
                },
                Err(_) => tracing::warn!("denúncia: webhook indisponível"),
            }
        });
    }

    pub fn reports_json(&self) -> Value {
        let list: Vec<Value> = self
            .reports
            .list
            .iter()
            .map(|r| {
                json!({
                    "id": r["id"], "at": r["at"], "room": r["room"], "style": r["style"], "reason": r["reason"],
                    "reasonText": reason_text(r["reason"].as_str().unwrap_or("")), "note": r["note"], "by": r["by"], "target": r["target"],
                    "chat": r["chat"], "voice": if r["voice"].is_array() { r["voice"].clone() } else { json!([]) }, "status": r["status"],
                    "doneAt": r["doneAt"].as_u64().unwrap_or(0)
                })
            })
            .collect();
        json!({"reports": list, "webhook": !self.webhook.is_empty(), "persisted": self.reports.file.is_some()})
    }

    /// Lista do painel: jogadores agrupados por IP (só o ID curto do IP aparece).
    #[allow(clippy::type_complexity)]
    pub fn mod_players(&mut self, me_ip: &str) -> Value {
        let mut groups: indexmap::IndexMap<String, Value> = indexmap::IndexMap::new();
        let rooms: Vec<(String, String, Vec<(String, String, String, bool)>)> = self
            .rooms
            .values()
            .map(|r| {
                (
                    room_tag(&r.code),
                    r.style.clone(),
                    r.players
                        .values()
                        .filter(|p| !p.bot && !p.ip.is_empty())
                        .map(|p| (p.ip.clone(), p.id.clone(), p.name.clone(), p.online && p.conn.is_some()))
                        .collect(),
                )
            })
            .collect();
        for (tag, style, ps) in rooms {
            for (ip, id, name, online) in ps {
                if !groups.contains_key(&ip) {
                    let g = json!({
                        "id": self.bm.host_id(&ip), "conns": self.conns_per_ip.get(&ip).copied().unwrap_or(0), "players": [],
                        "banMs": if self.bm.on { self.ban_check(&ip) } else { 0 }, "self": ip == me_ip
                    });
                    groups.insert(ip.clone(), g);
                }
                if let Some(a) = groups.get_mut(&ip).and_then(|g| g["players"].as_array_mut()) {
                    a.push(json!({"id": id, "name": name, "room": tag, "style": style, "online": online}));
                }
            }
        }
        let mut hosts: Vec<Value> = groups.into_values().collect();
        hosts.sort_by(|a, b| {
            let n = |x: &Value| x["players"].as_array().map_or(0, Vec::len);
            n(b).cmp(&n(a)).then_with(|| a["id"].as_str().cmp(&b["id"].as_str()))
        });
        let now = crate::now();
        let bans: Vec<Value> = if self.bm.on {
            self.bm
                .bans
                .list(now)
                .into_iter()
                .map(|(k, e)| json!({"id": &k[..12.min(k.len())], "left": e.until - now}))
                .collect()
        } else {
            Vec::new()
        };
        json!({"now": now, "ban": {"on": self.bm.on, "maxMinutes": BAN_MAX_MIN}, "hosts": hosts, "bans": bans})
    }

    /// Fecha as conexões de um IP e tira da sala quem estava esperando reconexão.
    fn kick_ip(&mut self, key: &str) -> usize {
        let ws: Vec<u64> = self.conns.values().filter(|c| c.ip == key).map(|c| c.id).collect();
        for w in &ws {
            self.send(*w, &json!({"t": "sys", "text": "Você foi removido pelo administrador."}));
            self.safe_close(*w, 1008, "banido");
        }
        let gone: Vec<(String, String)> = self
            .rooms
            .values()
            .flat_map(|r| {
                r.players
                    .values()
                    .filter(|p| !p.bot && p.ip == key && !p.online)
                    .map(|p| (r.code.clone(), p.id.clone()))
            })
            .collect();
        for (c, p) in gone {
            self.drop_player(&c, &p);
        }
        ws.len()
    }

    fn id_ok(s: &Value) -> Option<&str> {
        s.as_str().filter(|x| {
            (6..=43).contains(&x.len()) && x.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
    }

    /// Ações do painel `/status/*`: (código HTTP, corpo).
    pub fn status_action(
        &mut self,
        path: &str,
        body: &Value,
        me_ip: &str,
        trusted: &dyn Fn(&str) -> bool,
    ) -> (u16, Value) {
        let mins_ok = |v: &Value| crate::validate::int(v, 1, BAN_MAX_MIN);
        match path {
            "/status/ban" => {
                if !self.bm.on {
                    return out(409, json!({"erro": "Banimento desativado no servidor (BAN=0)."}));
                }
                let Some(mins) = mins_ok(&body["minutes"]) else {
                    return out(400, json!({"erro": format!("Duração inválida (de 1 a {BAN_MAX_MIN} minutos).")}));
                };
                let Some(id) = Self::id_ok(&body["id"]) else { return out(400, json!({"erro": "ID inválido."})) };
                let found: indexmap::IndexSet<String> = self
                    .rooms
                    .values()
                    .flat_map(|r| r.players.values())
                    .filter(|p| !p.bot && !p.ip.is_empty() && self.bm.key(&p.ip).starts_with(id))
                    .map(|p| p.ip.clone())
                    .collect();
                if found.len() > 1 {
                    return out(409, json!({"erro": "Identificador ambíguo."}));
                }
                let Some(ip) = found.into_iter().next() else {
                    return out(404, json!({"erro": "Esse IP não está mais conectado. Atualize a lista."}));
                };
                if ip == me_ip {
                    return out(409, json!({"erro": "Você não pode banir o seu próprio IP."}));
                }
                if trusted(&ip) {
                    return out(
                        409,
                        json!({"erro": "Este IP é de um proxy confiável (TRUST_PROXY) e não pode ser banido."}),
                    );
                }
                let k = self.bm.key(&ip);
                let now = crate::now();
                self.bm.bans.block(&k, mins as u64 * 60000, now);
                self.bm.save(now);
                let removidos = self.kick_ip(&ip);
                tracing::warn!("banido pelo painel ({mins} min, {removidos} conexões encerradas)");
                out(200, json!({"ok": 1, "id": self.bm.host_id(&ip), "minutes": mins, "removidos": removidos}))
            }
            "/status/unban" => {
                if !self.bm.on {
                    return out(409, json!({"erro": "Banimento desativado no servidor (BAN=0)."}));
                }
                let Some(id) = Self::id_ok(&body["id"]) else { return out(400, json!({"erro": "ID inválido."})) };
                let now = crate::now();
                let m: Vec<String> =
                    self.bm.bans.list(now).into_iter().map(|(k, _)| k).filter(|k| k.starts_with(id)).collect();
                if m.len() > 1 {
                    return out(409, json!({"erro": "Identificador ambíguo."}));
                }
                if m.is_empty() || !self.bm.bans.unban(&m[0]) {
                    return out(404, json!({"erro": "Banimento não encontrado."}));
                }
                self.bm.save(now);
                out(200, json!({"ok": 1}))
            }
            "/status/badge" => {
                let (Some(pid), Some(badge), Some(on)) =
                    (body["pid"].as_str(), body["badge"].as_str(), body["on"].as_bool())
                else {
                    return out(400, json!({"erro": "Informe pid, badge e on."}));
                };
                let r = self.badges.set(pid, badge, on);
                if r["ok"] != true {
                    let (code, erro) = match r["reason"].as_str() {
                        Some("pid") => (400, "Código de perfil inválido."),
                        Some("badge") => (400, "Emblema desconhecido."),
                        _ => (409, "Lista cheia."),
                    };
                    return out(code, json!({"erro": erro}));
                }
                out(200, json!({"ok": 1, "pid": pid, "badges": r["badges"]}))
            }
            "/status/keypass" | "/status/recover" => {
                let Some(pid) = body["pid"].as_str().filter(|p| crate::pubprofile::pid_re(p)) else {
                    return out(400, json!({"erro": "Informe o código do perfil (20 caracteres do link /profile#…)."}));
                };
                let Some(id) = self.identity.list_ids().into_iter().find(|x| pid_of(x) == pid) else {
                    return out(404, json!({"erro": "Nenhuma conta com esse código de perfil."}));
                };
                if path == "/status/keypass" {
                    if body["dry"] == true {
                        return out(200, json!({"ok": 1, "pid": pid, "keypass": self.identity.keypass(&id)}));
                    }
                    let r = self.identity.keypass_grant(&id);
                    if r["ok"] != true {
                        let nn = r["reason"] == "not-needed";
                        let erro = if nn {
                            "Esta conta ainda pode renovar a chave sozinha: não há o que liberar."
                        } else {
                            "Não foi possível gravar o registro de identidades."
                        };
                        return out(
                            if nn { 409 } else { 500 },
                            json!({"erro": erro, "keypass": self.identity.keypass(&id)}),
                        );
                    }
                    let n = r["keypass"]["used"].as_i64();
                    self.seclog.log("keypass.grant", crate::seclog::Ev { id: &id, n, ..Default::default() });
                    tracing::warn!("Keypass: uma renovação liberada pelo painel");
                    return out(200, json!({"ok": 1, "pid": pid, "keypass": r["keypass"]}));
                }
                if body["dry"] == true {
                    return out(200, json!({"ok": 1, "pid": pid, "grant": self.identity.recover_grant_info(&id)}));
                }
                let r = self.identity.recover_grant(&id);
                if r["ok"] != true {
                    return out(500, json!({"erro": "Não foi possível gravar o registro de identidades."}));
                }
                self.seclog.log("recovery.grant", crate::seclog::Ev { id: &id, ..Default::default() });
                tracing::warn!("Recuperação: autorização de uso único emitida pelo painel");
                out(200, json!({"ok": 1, "pid": pid, "token": r["token"], "exp": r["exp"]}))
            }
            "/status/capacity" => {
                let v = crate::capacity::validate(
                    crate::capacity::int_of(&body["rooms"]),
                    crate::capacity::int_of(&body["perRoom"]),
                );
                let (rooms, per) = match v {
                    Ok(x) => x,
                    Err(e) => return out(400, json!({"erro": e})),
                };
                if body["dry"] == true {
                    return out(200, json!({"ok": 1, "dry": 1, "capacity": self.capacity_view(rooms, per)}));
                }
                let before = format!("{}×{}", self.set.max_rooms, self.set.max_per_room);
                self.set.max_rooms = rooms;
                self.set.max_per_room = per;
                let codes: Vec<String> = self.rooms.keys().cloned().collect();
                for c in codes {
                    self.bc(&c, &json!({"t": "cap", "max": per}), None);
                }
                let saved = crate::capacity::save_file(&self.set.cap_file, rooms, per);
                tracing::warn!(
                    "capacidade alterada pelo painel: {before} → {rooms} salas × {per} jogadores ({} conexões){}",
                    self.max_conn(),
                    if saved { "" } else { " [não foi possível gravar o arquivo]" }
                );
                out(200, json!({"ok": 1, "saved": saved, "capacity": self.capacity_view(rooms, per)}))
            }
            "/status/report" => {
                let Some(id) = body["id"]
                    .as_str()
                    .filter(|i| i.len() == 12 && i.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
                else {
                    return out(400, json!({"erro": "Denúncia inválida."}));
                };
                let Some(i) = self.reports.list.iter().position(|r| r["id"] == id) else {
                    return out(404, json!({"erro": "Denúncia não encontrada."}));
                };
                let now = crate::now();
                if body["action"] == "dismiss" {
                    self.reports.list[i]["status"] = json!("dismissed");
                    self.reports.list[i]["doneAt"] = json!(now);
                    self.reports.save();
                    return out(200, json!({"ok": 1}));
                }
                if body["action"] != "ban" {
                    return out(400, json!({"erro": "Ação inválida."}));
                }
                if !self.bm.on {
                    return out(409, json!({"erro": "Banimento desativado no servidor (BAN=0)."}));
                }
                let Some(mins) = mins_ok(&body["minutes"]) else {
                    return out(400, json!({"erro": format!("Duração inválida (de 1 a {BAN_MAX_MIN} minutos).")}));
                };
                let tk = self.reports.list[i]["tk"].as_str().unwrap_or("").to_owned();
                if tk == self.bm.key(me_ip) {
                    return out(409, json!({"erro": "Você não pode banir o seu próprio IP."}));
                }
                let ips: indexmap::IndexSet<String> = self
                    .conns
                    .values()
                    .filter(|c| !c.ip.is_empty() && self.bm.key(&c.ip) == tk)
                    .map(|c| c.ip.clone())
                    .collect();
                if ips.iter().any(|ip| trusted(ip)) {
                    return out(
                        409,
                        json!({"erro": "Este IP é de um proxy confiável (TRUST_PROXY) e não pode ser banido."}),
                    );
                }
                self.bm.bans.block(&tk, mins as u64 * 60000, now);
                self.bm.save(now);
                let removidos: usize = ips.iter().map(|k| self.kick_ip(k)).sum();
                for r in &mut self.reports.list {
                    if r["tk"] == tk.as_str() && (r["status"] == "open" || r["id"] == id) {
                        r["status"] = json!("banned");
                        r["doneAt"] = json!(now);
                    }
                }
                self.reports.save();
                tracing::warn!("banido pelo painel por denúncia ({mins} min, {removidos} conexões encerradas)");
                out(200, json!({"ok": 1, "minutes": mins, "removidos": removidos}))
            }
            _ => out(404, json!({"erro": "404"})),
        }
    }

    pub fn capacity_view(&self, rooms: i64, per: i64) -> Value {
        let (mut h, mut b) = (0, 0);
        let mut over = 0;
        for r in self.rooms.values() {
            let hh = r.human_count();
            h += hh;
            b += r.players.len() - hh;
            if hh as i64 > per {
                over += 1;
            }
        }
        json!({
            "rooms": rooms, "perRoom": per, "conns": crate::capacity::conn_limit(rooms, per),
            "ceilings": {"minRooms": crate::capacity::MIN_ROOMS, "maxRooms": crate::capacity::MAX_ROOMS, "minPerRoom": crate::capacity::MIN_PER_ROOM, "maxPerRoom": crate::capacity::MAX_PER_ROOM, "maxPlayers": crate::capacity::MAX_PLAYERS},
            "defaults": {"rooms": crate::capacity::DEFAULT_ROOMS, "perRoom": crate::capacity::DEFAULT_PER_ROOM},
            "persisted": !self.set.cap_file.is_empty(),
            "open": {"rooms": self.rooms.len(), "humans": h, "connections": self.conns.len(), "roomsOverLimit": (self.rooms.len() as i64 - rooms).max(0), "roomsOverSize": over},
            "estimate": crate::capacity::estimate(rooms, per, Some(((h + b) as i64, self.perf.tlast)))
        })
    }
}
