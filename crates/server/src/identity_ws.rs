//! Mensagens de identidade pelo WebSocket (`lib/identity-ws.js`): desafio e prova, registro, troca de chaves,
//! recuperação (com carência), autorização do administrador, revogação de sessões e passkeys.

use crate::game::{Game, Who};
use crate::seclog::Ev;
use serde_json::{Value, json};

const PK_FAIL_MAX: usize = 10;
const PK_FAIL_MS: u64 = 10 * 60000;

fn id_msg(code: &str) -> &'static str {
    match code {
        "notfound" => "Identidade não encontrada.",
        "exists" => "Esta identidade já foi registrada. Use a chave de recuperação.",
        "stale" => "A identidade mudou. Atualize a página e tente de novo.",
        "bad-signature" => "Assinatura inválida.",
        "pending" => "Já existe uma recuperação em andamento.",
        "full" => "Limite de identidades do servidor atingido.",
        "corrupt" => "O registro de identidades está indisponível. Tente mais tarde.",
        "io" => "Não foi possível salvar a identidade. Tente de novo.",
        "challenge" => "Desafio inválido ou expirado. Peça outro.",
        "expired" => "Desafio expirado. Peça outro.",
        "busy" => "Muitas tentativas. Aguarde um instante.",
        "session" => "Sessão expirada. Entre novamente.",
        "scope" => "Esta sessão não permite essa ação.",
        "rate" => "Muitas tentativas. Aguarde um minuto.",
        "limit" => "Muitas contas novas deste local hoje. Tente de novo amanhã.",
        "pow" => "O servidor está recebendo muitos registros agora. Tente de novo em instantes.",
        "required" => "Entre com sua identidade para jogar.",
        "registered" => "Esta identidade já foi registrada. Atualize a página e entre com sua chave.",
        "pk-off" => "As passkeys estão desligadas neste servidor.",
        "pk-origin" => "Passkeys só funcionam pelo endereço do jogo com https (ou em localhost).",
        "pk-full" => "Você já tem 10 passkeys. Remova uma para cadastrar outra.",
        "pk-exists" => "Esta passkey já está cadastrada na sua conta.",
        "pk-missing" => "Passkey não encontrada.",
        "pk-fail" => "Passkey não reconhecida. Use uma passkey cadastrada nesta conta.",
        "pk-locked" => {
            "Muitas tentativas com passkey nesta conta. Aguarde alguns minutos ou use o código de recuperação."
        }
        "keypass-limit" => {
            "Você já usou as 3 renovações da chave de recuperação. Para renovar de novo, fale com o administrador do servidor."
        }
        "keypass-wait" => "A chave de recuperação só pode ser renovada uma vez por mês. Aguarde a data indicada.",
        "grant-bad" => "Autorização não reconhecida. Confira com o administrador.",
        "grant-expired" => "Esta autorização venceu. Peça outra ao administrador.",
        "used" => "Esta autorização já foi usada. Se precisar de outra, fale com o administrador.",
        _ => "Dados de identidade inválidos.",
    }
}

fn kp_out(k: &Value) -> Value {
    if k.is_null() {
        return Value::Null;
    }
    json!({"used": k["used"], "max": k["max"], "left": k["left"], "next": k["next"], "last": k["last"]})
}

fn reason(r: &Value) -> &str {
    r["reason"].as_str().unwrap_or("invalid")
}

/// Dificuldade da prova do registro: sobe com o ritmo de registros (`bitsFor`).
pub fn bits_for(base: u32, max: u32, recent: usize) -> u32 {
    if base == 0 {
        return 0;
    }
    let bump = if recent >= 120 {
        6
    } else if recent >= 30 {
        4
    } else {
        0
    };
    base.max(max).min(base + bump)
}

impl Game {
    fn id_err(&mut self, cid: u64, scope: &str, code: &str, msg: Option<&str>, extra: Option<Value>) {
        let mut m = json!({"t": "err", "scope": scope, "code": code, "msg": msg.unwrap_or_else(|| id_msg(code))});
        if let Some(Value::Object(e)) = extra {
            for (k, v) in e {
                if !v.is_null() {
                    m[k] = v;
                }
            }
        }
        self.send(cid, &m);
    }

    /// Derruba as outras conexões da identidade (troca de chave, revogação, recuperação aplicada).
    pub fn close_identity_conns(&mut self, id: &str, except: Option<u64>, scope: Option<&str>) {
        if id.is_empty() {
            return;
        }
        let targets: Vec<u64> = self
            .conns
            .values()
            .filter(|c| Some(c.id) != except && c.ident_id == id && scope.is_none_or(|s| c.ident_scope == s))
            .map(|c| c.id)
            .collect();
        for w in targets {
            self.send(
                w,
                &json!({"t": "err", "scope": "identity", "code": "revoked", "msg": "Sessão encerrada por segurança."}),
            );
            self.safe_close(w, 4001, "identity revoked");
        }
    }

    /// Quem está entrando numa sala: sessão (`token`), chave antiga do aparelho (`k`) ou id efêmero.
    pub fn identity_of(&mut self, cid: u64, m: &Value) -> Option<Who> {
        let k = m["k"].as_str().unwrap_or("").to_owned();
        if let Some(tk) = m["token"].as_str() {
            let Some(v) = self.auth.verify_token(&mut self.identity, tk, Some("join")) else {
                self.id_err(cid, "auth", "session", None, None);
                return None;
            };
            return Some(Who { id: v.id, fk: k, gen_: v.gen_, epoch: v.epoch, scope: v.scope });
        }
        if !self.set.legacy_identity {
            self.id_err(cid, "auth", "required", None, None);
            return None;
        }
        if k.is_empty() {
            if self.set.names_on {
                self.id_err(
                    cid,
                    "auth",
                    "required",
                    Some("Atualize a página para entrar: falta a chave deste aparelho."),
                    None,
                );
                return None;
            }
            let c = self.conns.get_mut(&cid)?;
            if c.eph_id.is_empty() {
                c.eph_id = crate::new_id();
            }
            return Some(Who { id: c.eph_id.clone(), ..Default::default() });
        }
        let id = sky_identity::fid_of(&k);
        if self.identity.has(&id) {
            self.id_err(cid, "auth", "registered", None, None);
            return None;
        }
        Some(Who { id, fk: k, ..Default::default() })
    }

    fn ip_of(&self, cid: u64) -> String {
        self.conns.get(&cid).map(|c| c.ip.clone()).unwrap_or_default()
    }

    /// Trata as mensagens de identidade; `false` = não é uma delas.
    pub fn identity_msg(&mut self, cid: u64, m: &Value) -> bool {
        let t = m["t"].as_str().unwrap_or("");
        let s = |k: &str| m[k].as_str().unwrap_or("").to_owned();
        let i = |k: &str| m[k].as_i64().unwrap_or(0);
        let ip = self.ip_of(cid);
        match t {
            "auth" => {
                if self.over(&ip, "idauth", 60, 60000) {
                    self.id_err(cid, "auth", "rate", None, None);
                    return true;
                }
                let r = self.auth.challenge(&mut self.identity, &s("id"), &s("action"));
                if r["ok"] != true {
                    self.id_err(cid, "auth", reason(&r), None, None);
                    return true;
                }
                self.send(cid, &json!({"t": "challenge", "id": m["id"], "action": m["action"], "nonce": r["nonce"], "exp": r["exp"]}));
            }
            "proof" => {
                if self.over(&ip, "idproof", 60, 60000) {
                    self.id_err(cid, "proof", "rate", None, None);
                    return true;
                }
                let id = s("id");
                let r = self.auth.verify_proof(&mut self.identity, &id, &s("nonce"), &s("sig"), &s("action"));
                if r["ok"] != true {
                    if self.hit_count(&ip, "idfail", 20, 600_000).count == 21 {
                        self.ban_strike(&ip, "identidade");
                    }
                    if reason(&r) == "bad-signature" {
                        let a = s("action");
                        self.seclog.log(
                            "auth.fail",
                            Ev { id: &id, ip: &ip, ok: Some(false), reason: &a, ..Default::default() },
                        );
                    }
                    self.id_err(cid, "proof", reason(&r), None, None);
                    return true;
                }
                let scope = r["scope"].as_str().unwrap_or("").to_owned();
                self.set_ident(cid, &id, r["gen"].as_i64().unwrap_or(0), r["epoch"].as_i64().unwrap_or(0), &scope);
                let cur = self.identity.get(&id);
                let mut out = json!({
                    "t": "session", "id": id, "token": r["token"], "exp": r["exp"], "gen": r["gen"], "rgen": r["rgen"],
                    "epoch": r["epoch"], "scope": scope
                });
                if let Some(p) = cur.and_then(|c| c.pending) {
                    out["pending"] = json!({"until": p.until});
                }
                if scope == "primary" {
                    out["kp"] = kp_out(&self.identity.keypass(&id).unwrap_or(Value::Null));
                }
                self.send(cid, &out);
            }
            "idreg" => {
                if self.over(&ip, "idreg", 20, 60000) {
                    self.id_err(cid, "idreg", "rate", None, None);
                    return true;
                }
                let (k, pk, rpk) = (s("k"), s("pk"), s("rpk"));
                let id = sky_identity::fid_of(&k);
                if let Some(cur) = self.identity.get(&id) {
                    if cur.pk == pk && cur.rpk == rpk {
                        self.send(cid, &json!({"t": "idok", "op": "register", "id": id, "gen": cur.gen_, "rgen": cur.rgen, "epoch": cur.epoch, "existed": true}));
                    } else {
                        self.id_err(cid, "idreg", "exists", None, None);
                    }
                    return true;
                }
                let recent = self.count_hits("@", "idregg", 60000);
                let bits = bits_for(self.set.identity_pow, self.set.identity_pow_max, recent);
                if bits > 0 && !sky_security::pow::idreg_ok(&k, &pk, &rpk, &s("pow"), bits) {
                    self.id_err(cid, "idreg", "pow", None, Some(json!({"bits": bits})));
                    return true;
                }
                let day = self.set.identity_reg_day;
                if day > 0 {
                    let key = if ip.ends_with("::/64") {
                        format!("{}::/48", ip.split(':').take(3).collect::<Vec<_>>().join(":"))
                    } else {
                        ip.clone()
                    };
                    if self.day_limiter.hit(&format!("idregd|{key}"), day, 86_400_000, crate::now()).over {
                        self.id_err(cid, "idreg", "limit", None, None);
                        return true;
                    }
                }
                let r = self.identity.register(&id, &pk, &rpk, true);
                if r["ok"] != true {
                    self.id_err(cid, "idreg", reason(&r), None, None);
                    return true;
                }
                self.hit_count("@", "idregg", 1_000_000_000, 60000);
                self.seclog.log("identity.create", Ev { id: &id, ip: &ip, ..Default::default() });
                self.send(cid, &json!({"t": "idok", "op": "register", "id": id, "gen": r["gen"], "rgen": r["rgen"], "epoch": r["epoch"]}));
            }
            "idrot" => {
                if self.over(&ip, "idrot", 12, 60000) {
                    self.id_err(cid, "idrot", "rate", None, None);
                    return true;
                }
                let has_pk = m.get("pk").is_some();
                let action = if has_pk { "rotate" } else { "rotate-recovery" };
                let Some(v) = self.auth.verify_token(&mut self.identity, &s("token"), Some(action)) else {
                    self.id_err(cid, "idrot", "session", None, None);
                    return true;
                };
                if v.scope != "primary" {
                    self.id_err(cid, "idrot", "scope", None, None);
                    return true;
                }
                let r = if has_pk {
                    self.identity.rotate_primary(&v.id, i("gen"), &s("pk"), &s("sigOld"), &s("sigRec"))
                } else {
                    self.identity.rotate_recovery(&v.id, i("rgen"), &s("rpk"), &s("sigRec"), &s("sigOld"))
                };
                let ev = if has_pk { "key.rotate" } else { "recovery-code.rotate" };
                if r["ok"] != true {
                    let rs = reason(&r).to_owned();
                    if matches!(rs.as_str(), "bad-signature" | "keypass-limit" | "keypass-wait") {
                        self.seclog
                            .log(ev, Ev { id: &v.id, ip: &ip, ok: Some(false), reason: &rs, ..Default::default() });
                    }
                    let extra = json!({"until": r.get("until").cloned().unwrap_or(Value::Null), "kp": kp_out(r.get("keypass").unwrap_or(&Value::Null))});
                    self.id_err(cid, "idrot", &rs, None, Some(extra));
                    return true;
                }
                if r["idempotent"] != true {
                    let n = r["keypass"]["used"].as_i64();
                    self.seclog.log(ev, Ev { id: &v.id, ip: &ip, n, ..Default::default() });
                }
                self.auth.revoke_sessions(&v.id);
                let Some(rec) = self.identity.get(&v.id) else {
                    self.id_err(cid, "idrot", "notfound", None, None);
                    return true;
                };
                let fresh = self.auth.issue(&mut self.identity, &v.id, "primary");
                self.set_ident(cid, &v.id, rec.gen_, rec.epoch, "primary");
                self.close_identity_conns(&v.id, Some(cid), None);
                let mut out = json!({"t": "idok", "op": action, "id": v.id, "gen": rec.gen_, "rgen": rec.rgen, "epoch": rec.epoch});
                if let Some(f) = fresh {
                    out["token"] = f["token"].clone();
                    out["exp"] = f["exp"].clone();
                }
                out["kp"] = kp_out(&self.identity.keypass(&v.id).unwrap_or(Value::Null));
                self.send(cid, &out);
            }
            "idrec" => {
                let Some(v) = self.auth.verify_token(&mut self.identity, &s("token"), Some("recover")) else {
                    self.id_err(cid, "idrec", "session", None, None);
                    return true;
                };
                if v.scope != "recovery" {
                    self.id_err(cid, "idrec", "scope", None, None);
                    return true;
                }
                let grace = self.set.grace_ms.max(m["grace"].as_u64().unwrap_or(0));
                let was_pending = self.identity.get(&v.id).is_some_and(|r| r.pending.is_some());
                let new_rpk = m["rpk"].as_str().map(str::to_owned);
                let r =
                    self.identity.recover_primary(&v.id, i("gen"), &s("pk"), new_rpk.as_deref(), &s("sigRec"), grace);
                if r["ok"] != true {
                    let rs = reason(&r).to_owned();
                    if rs == "bad-signature" || rs == "notfound" {
                        self.seclog.log(
                            "recovery.request",
                            Ev { id: &v.id, ip: &ip, ok: Some(false), reason: "bad-signature", ..Default::default() },
                        );
                    }
                    let code = if rs == "notfound" { "bad-signature".to_owned() } else { rs.clone() };
                    let extra = (rs == "pending").then(|| json!({"until": r["until"]}));
                    self.id_err(cid, "idrec", &code, None, extra);
                    return true;
                }
                let Some(rec) = self.identity.get(&v.id) else {
                    self.id_err(cid, "idrec", "notfound", None, None);
                    return true;
                };
                if r["pending"] == true {
                    if !was_pending {
                        let n = (grace as f64 / 3_600_000.0).round() as i64;
                        self.seclog
                            .log("recovery.request", Ev { id: &v.id, ip: &ip, n: Some(n), ..Default::default() });
                    }
                    self.send(cid, &json!({"t": "idok", "op": "recover", "id": v.id, "gen": r["gen"], "rgen": rec.rgen, "epoch": r["epoch"], "pending": true, "until": r["until"]}));
                    return true;
                }
                if r["idempotent"] != true {
                    self.seclog.log("recovery.apply", Ev { id: &v.id, ip: &ip, ..Default::default() });
                }
                self.auth.revoke_sessions(&v.id);
                let fresh = self.auth.issue(&mut self.identity, &v.id, "recovery");
                self.set_ident(cid, &v.id, rec.gen_, rec.epoch, "recovery");
                self.close_identity_conns(&v.id, Some(cid), None);
                let mut out = json!({"t": "idok", "op": "recover", "id": v.id, "gen": rec.gen_, "rgen": rec.rgen, "epoch": rec.epoch});
                if let Some(f) = fresh {
                    out["token"] = f["token"].clone();
                    out["exp"] = f["exp"].clone();
                }
                self.send(cid, &out);
            }
            "idgrant" => {
                if self.over(&ip, "idgrant", 10, 60000) {
                    self.id_err(cid, "idgrant", "rate", None, None);
                    return true;
                }
                let r = self.identity.recover_redeem(&s("token"), &s("rpk"));
                if r["ok"] != true {
                    let rs = reason(&r).to_owned();
                    if matches!(rs.as_str(), "invalid" | "expired" | "used") {
                        let why = if rs == "invalid" { "grant-unknown" } else { rs.as_str() };
                        self.seclog.log(
                            "recovery-code.rotate",
                            Ev { ip: &ip, ok: Some(false), reason: why, ..Default::default() },
                        );
                    }
                    let code = match rs.as_str() {
                        "invalid" => "grant-bad",
                        "expired" => "grant-expired",
                        x => x,
                    }
                    .to_owned();
                    self.id_err(cid, "idgrant", &code, None, None);
                    return true;
                }
                let id = r["id"].as_str().unwrap_or("").to_owned();
                if r["cancelled"] == true {
                    self.seclog.log("recovery.cancel", Ev { id: &id, ip: &ip, reason: "grant", ..Default::default() });
                }
                self.seclog.log(
                    "recovery-code.rotate",
                    Ev { id: &id, ip: &ip, reason: "grant", n: r["rgen"].as_i64(), ..Default::default() },
                );
                self.auth.revoke_sessions(&id);
                self.close_identity_conns(&id, Some(cid), None);
                self.send(cid, &json!({"t": "idok", "op": "recover-grant", "id": id, "gen": r["gen"], "rgen": r["rgen"], "epoch": r["epoch"]}));
            }
            "idcancel" => {
                let Some(v) = self.auth.verify_token(&mut self.identity, &s("token"), Some("cancel-recovery")) else {
                    self.id_err(cid, "idcancel", "session", None, None);
                    return true;
                };
                if v.scope != "primary" {
                    self.id_err(cid, "idcancel", "scope", None, None);
                    return true;
                }
                let had = self.identity.get(&v.id).is_some_and(|r| r.pending.is_some());
                let r = self.identity.cancel_recovery(&v.id, i("gen"), &s("sigOld"));
                if r["ok"] != true {
                    self.id_err(cid, "idcancel", reason(&r), None, None);
                    return true;
                }
                if had {
                    self.seclog.log("recovery.cancel", Ev { id: &v.id, ip: &ip, ..Default::default() });
                }
                let rgen = self.identity.get(&v.id).map_or(0, |r| r.rgen);
                self.send(cid, &json!({"t": "idok", "op": "cancel-recovery", "id": v.id, "gen": r["gen"], "rgen": rgen, "epoch": v.epoch}));
            }
            "idrev" => {
                let Some(v) = self.auth.verify_token(&mut self.identity, &s("token"), Some("revoke")) else {
                    self.id_err(cid, "idrev", "session", None, None);
                    return true;
                };
                let primary = v.scope == "primary";
                if primary {
                    if m.get("sigOld").is_none() {
                        self.id_err(cid, "idrev", "invalid", None, None);
                        return true;
                    }
                    let r = self.identity.revoke_sessions(&v.id, i("epoch"), &s("sigOld"));
                    if r["ok"] != true {
                        self.id_err(cid, "idrev", reason(&r), None, None);
                        return true;
                    }
                    if r["idempotent"] != true {
                        self.seclog
                            .log("sessions.revoke", Ev { id: &v.id, ip: &ip, reason: "primary", ..Default::default() });
                    }
                } else {
                    self.seclog
                        .log("sessions.revoke", Ev { id: &v.id, ip: &ip, reason: "recovery", ..Default::default() });
                }
                self.auth.revoke_sessions(&v.id);
                let Some(rec) = self.identity.get(&v.id) else {
                    self.id_err(cid, "idrev", "notfound", None, None);
                    return true;
                };
                let scope = if primary { "primary" } else { "recovery" };
                let fresh = self.auth.issue(&mut self.identity, &v.id, scope);
                self.set_ident(cid, &v.id, rec.gen_, rec.epoch, scope);
                self.close_identity_conns(&v.id, Some(cid), None);
                let mut out = json!({"t": "idok", "op": "revoke", "id": v.id, "gen": rec.gen_, "rgen": rec.rgen, "epoch": rec.epoch});
                if !primary {
                    out["scope"] = json!("recovery");
                }
                if let Some(f) = fresh {
                    out["token"] = f["token"].clone();
                    out["exp"] = f["exp"].clone();
                }
                self.send(cid, &out);
            }
            "pkopt" | "pkreg" | "pklist" | "pkdel" | "pklogin" => self.passkey_msg(cid, t, m),
            _ => return false,
        }
        true
    }

    /// Passkeys: por enquanto o servidor Rust responde que estão desligadas (a verificação WebAuthn vem depois).
    fn passkey_msg(&mut self, cid: u64, t: &str, m: &Value) {
        let _ = (PK_FAIL_MAX, PK_FAIL_MS, m);
        if t == "pklist" {
            let Some(v) =
                self.auth.verify_token(&mut self.identity, m["token"].as_str().unwrap_or(""), Some("passkey"))
            else {
                return self.id_err(cid, t, "session", None, None);
            };
            if v.scope != "primary" {
                return self.id_err(cid, t, "scope", None, None);
            }
            let list = self.identity.passkeys(&v.id);
            return self.send(cid, &json!({"t": "idok", "op": "pklist", "id": v.id, "list": list, "on": false}));
        }
        self.id_err(cid, t, "pk-off", None, None);
    }
}
