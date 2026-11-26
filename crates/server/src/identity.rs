//! Registro de identidades (`lib/identity.js`): só chaves públicas P-256, gerações, época de revogação, pedido de
//! recuperação em carência, passkeys, renovações da chave de recuperação (keypass) e autorizações de recuperação dadas
//! pelo administrador. Não guarda nomes, mensagens nem IP.

use crate::persist::{read_json_file, write_json_atomic};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;

pub const MAX_GEN: i64 = 0x7fff_ffff;
pub const DOMAIN: &str = "skyke3d/id/v1";
pub const MAX_PASSKEYS: usize = 10;
pub const PK_NAME_MAX: usize = 40;
pub const KEYPASS_MAX: i64 = 3;
pub const KEYPASS_INTERVAL_MS: u64 = 30 * 24 * 3_600_000;
const KEYPASS_LOG_MAX: usize = 12;
pub const REC_GRANT_TTL: u64 = 24 * 3_600_000;
const REC_GRANT_DOMAIN: &str = "skyke3d/recgrant/v1";

/// Textos assinados (`payload`).
pub mod payload {
    use super::DOMAIN;
    pub fn register(id: &str, pk: &str, rpk: &str) -> String {
        [DOMAIN, "register", id, pk, rpk].join("\n")
    }
    pub fn auth(id: &str, nonce: &str, action: &str) -> String {
        [DOMAIN, "auth", id, nonce, action].join("\n")
    }
    pub fn rotate(id: &str, gen_: i64, new_pk: &str) -> String {
        [DOMAIN, "rotate", id, &gen_.to_string(), new_pk].join("\n")
    }
    pub fn rotate_recovery(id: &str, rgen: i64, new_rpk: &str) -> String {
        [DOMAIN, "rotate-recovery", id, &rgen.to_string(), new_rpk].join("\n")
    }
    pub fn revoke(id: &str, epoch: i64) -> String {
        [DOMAIN, "revoke", id, &epoch.to_string()].join("\n")
    }
    pub fn remove(id: &str) -> String {
        [DOMAIN, "remove", id].join("\n")
    }
    pub fn recover(id: &str, gen_: i64, new_pk: &str, new_rpk: Option<&str>) -> String {
        let g = gen_.to_string();
        let mut v = vec![DOMAIN, "recover", id, &g, new_pk];
        if let Some(r) = new_rpk {
            v.push(r);
        }
        v.join("\n")
    }
    pub fn cancel_recovery(id: &str, gen_: i64) -> String {
        [DOMAIN, "cancel-recovery", id, &gen_.to_string()].join("\n")
    }
}

fn b64ok(s: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}
pub fn pk_re(s: &str) -> bool {
    b64ok(s, 80, 200)
}
fn sig_re(s: &str) -> bool {
    b64ok(s, 80, 100)
}
pub fn id_re(s: &str) -> bool {
    s.len() == 12 && s.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// Chave pública P-256 válida (`importPublicKey`).
pub fn import_public_key(b64: &str) -> bool {
    use p256::pkcs8::DecodePublicKey;
    if !pk_re(b64) {
        return false;
    }
    let Ok(der) = URL_SAFE_NO_PAD.decode(b64) else { return false };
    p256::ecdsa::VerifyingKey::from_public_key_der(&der).is_ok()
}

/// Assinatura ECDSA P-256 (r‖s, 64 bytes) de `data` com a chave `pk`.
pub fn verify_signature(pk: &str, data: &str, sig: &str) -> bool {
    if !sig_re(sig) || !pk_re(pk) {
        return false;
    }
    match URL_SAFE_NO_PAD.decode(sig) {
        Ok(s) if s.len() == 64 => sky_identity::verify_b64(pk, data, sig).unwrap_or(false),
        _ => false,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Pending {
    pub pk: String,
    pub until: u64,
    pub rpk: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Passkey {
    pub cid: String,
    pub pk: String,
    pub alg: i64,
    pub sc: i64,
    pub name: String,
    pub created: f64,
    pub used: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Keypass {
    pub n: i64,
    pub at: u64,
    pub log: Vec<u64>,
    pub adm: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecGrant {
    pub h: String,
    pub exp: u64,
    pub rpk: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub id: String,
    pub pk: String,
    pub gen_: i64,
    pub rpk: String,
    pub rgen: i64,
    pub epoch: i64,
    pub created: f64,
    pub at: f64,
    pub pending: Option<Pending>,
    pub pks: Vec<Passkey>,
    pub legacy: bool,
    pub used: bool,
    pub kp: Option<Keypass>,
    pub rec_grant: Option<RecGrant>,
}

fn fin(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}
fn int_in(v: &Value, min: i64, max: i64) -> Option<i64> {
    let f = v.as_f64()?;
    (f.fract() == 0.0 && f >= min as f64 && f <= max as f64).then_some(f as i64)
}

fn valid_passkey(p: &Value) -> Option<Passkey> {
    let cid = p["cid"].as_str().filter(|c| b64ok(c, 16, 1366))?;
    let pk = p["pk"].as_str().filter(|c| b64ok(c, 40, 1200))?;
    let alg = int_in(&p["alg"], -257, -7).filter(|a| [-7, -8, -257].contains(a))?;
    let sc = int_in(&p["sc"], 0, 0xffff_ffff)?;
    let name = p["name"].as_str().filter(|n| crate::validate::len16(n) <= PK_NAME_MAX)?;
    let created = fin(&p["created"])?;
    let used = match p.get("used") {
        None => None,
        Some(u) => Some(fin(u)?),
    };
    Some(Passkey { cid: cid.into(), pk: pk.into(), alg, sc, name: name.into(), created, used })
}

impl Passkey {
    fn to_json(&self) -> Value {
        let mut o = json!({"cid": self.cid, "pk": self.pk, "alg": self.alg, "sc": self.sc, "name": self.name, "created": self.created});
        if let Some(u) = self.used {
            o["used"] = json!(u);
        }
        o
    }
}

impl Record {
    fn from_json(id: &str, e: &Value) -> Option<Self> {
        let pk = e["pk"].as_str().filter(|x| pk_re(x))?;
        let rpk = e["rpk"].as_str().filter(|x| pk_re(x))?;
        let gen_ = int_in(&e["gen"], 1, MAX_GEN)?;
        let rgen = int_in(&e["rgen"], 1, MAX_GEN)?;
        let epoch = int_in(&e["epoch"], 0, MAX_GEN)?;
        let created = fin(&e["created"])?;
        let at = fin(&e["at"])?;
        let pending = match e.get("pending") {
            None => None,
            Some(p) => {
                p.as_object()?;
                let ppk = p["pk"].as_str().filter(|x| pk_re(x))?;
                let until = fin(&p["until"]).filter(|u| *u >= 0.0)?;
                let prpk = match p.get("rpk") {
                    None => None,
                    Some(r) => Some(r.as_str().filter(|x| pk_re(x))?.to_owned()),
                };
                Some(Pending { pk: ppk.into(), until: until as u64, rpk: prpk })
            }
        };
        let mut pks = Vec::new();
        if let Some(a) = e.get("pks") {
            let a = a.as_array().filter(|a| a.len() <= MAX_PASSKEYS)?;
            for p in a {
                pks.push(valid_passkey(p)?);
            }
        }
        let kp = match e.get("kp") {
            None => None,
            Some(k) => {
                k.as_object()?;
                let n = int_in(&k["n"], 0, MAX_GEN)?;
                let at = fin(&k["at"]).filter(|a| *a >= 0.0)?;
                let log = k["log"].as_array().filter(|l| l.len() <= KEYPASS_LOG_MAX)?;
                let log: Vec<u64> = log.iter().map(|x| fin(x).map(|f| f.max(0.0) as u64)).collect::<Option<_>>()?;
                let adm = match k.get("adm") {
                    None => 0,
                    Some(a) => int_in(a, 0, MAX_GEN)?,
                };
                Some(Keypass { n, at: at as u64, log, adm })
            }
        };
        let rec_grant = match e.get("recGrant") {
            None => None,
            Some(g) => {
                g.as_object()?;
                let h = g["h"].as_str().filter(|h| b64ok(h, 43, 43))?;
                let exp = fin(&g["exp"]).filter(|x| *x >= 0.0)?;
                let rpk = match g.get("rpk") {
                    None => None,
                    Some(r) => Some(r.as_str().filter(|x| pk_re(x))?.to_owned()),
                };
                Some(RecGrant { h: h.into(), exp: exp as u64, rpk })
            }
        };
        Some(Self {
            id: id.into(),
            pk: pk.into(),
            gen_,
            rpk: rpk.into(),
            rgen,
            epoch,
            created,
            at,
            pending,
            pks,
            legacy: e["legacy"] == true,
            used: e["used"] == true,
            kp,
            rec_grant,
        })
    }

    fn to_json(&self) -> Value {
        let mut o = json!({
            "pk": self.pk, "gen": self.gen_, "rpk": self.rpk, "rgen": self.rgen, "epoch": self.epoch,
            "created": self.created, "at": self.at
        });
        if let Some(p) = &self.pending {
            let mut pj = json!({"pk": p.pk, "until": p.until});
            if let Some(r) = &p.rpk {
                pj["rpk"] = json!(r);
            }
            o["pending"] = pj;
        }
        if !self.pks.is_empty() {
            o["pks"] = Value::Array(self.pks.iter().map(Passkey::to_json).collect());
        }
        if self.legacy {
            o["legacy"] = json!(true);
        }
        if self.used {
            o["used"] = json!(true);
        }
        if let Some(k) = &self.kp {
            let mut kj = json!({"n": k.n, "at": k.at, "log": k.log});
            if k.adm != 0 {
                kj["adm"] = json!(k.adm);
            }
            o["kp"] = kj;
        }
        if let Some(g) = &self.rec_grant {
            let mut gj = json!({"h": g.h, "exp": g.exp});
            if let Some(r) = &g.rpk {
                gj["rpk"] = json!(r);
            }
            o["recGrant"] = gj;
        }
        o
    }
}

/// Resultado de uma operação: `{ok: true, ...}` ou `{ok: false, reason, ...}`, como no original.
pub type Res = Value;
fn fail(reason: &str) -> Res {
    json!({"ok": false, "reason": reason})
}

#[derive(Debug)]
pub struct IdentityRegistry {
    file: Option<PathBuf>,
    pub max: usize,
    unused_ms: u64,
    lazy_ms: u64,
    keypass_max: i64,
    keypass_interval_ms: u64,
    ids: IndexMap<String, Record>,
    unused: IndexSet<String>,
    pendings: IndexMap<String, u64>,
    cid_idx: HashMap<String, String>,
    pub saved_at: u64,
    pub corrupt: bool,
    main_valid: bool,
    lazy_due: Option<u64>,
    /// Identidades cuja recuperação venceu a carência desde a última leitura (o servidor derruba as sessões delas).
    pub settled: Vec<String>,
}

impl IdentityRegistry {
    pub fn new(file: &str, max: usize, unused_ms: u64, keypass_interval_ms: u64) -> Self {
        let mut r = Self {
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            max,
            unused_ms,
            lazy_ms: 30_000,
            keypass_max: KEYPASS_MAX,
            keypass_interval_ms,
            ids: IndexMap::new(),
            unused: IndexSet::new(),
            pendings: IndexMap::new(),
            cid_idx: HashMap::new(),
            saved_at: 0,
            corrupt: false,
            main_valid: false,
            lazy_due: None,
            settled: Vec::new(),
        };
        if let Some(f) = r.file.clone() {
            let main = r.read_valid(&f);
            if main.1 {
                r.main_valid = true;
            } else {
                let mut bak = f.as_os_str().to_owned();
                bak.push(".bak");
                let bak = r.read_valid(&PathBuf::from(bak));
                if bak.1 {
                    tracing::warn!("identidade: arquivo principal inválido; usando {}.bak", f.display());
                } else if main.0 || bak.0 {
                    r.corrupt = true;
                    tracing::warn!(
                        "identidade: registro inválido em {}; operações de identidade bloqueadas até reparar.",
                        f.display()
                    );
                }
            }
        }
        r
    }

    fn read_valid(&mut self, p: &PathBuf) -> (bool, bool) {
        match read_json_file(p) {
            Ok(None) => (false, false),
            Ok(Some(obj)) => (true, self.parse(&obj)),
            Err(_) => (true, false),
        }
    }

    fn parse(&mut self, obj: &Value) -> bool {
        if obj["v"] != 1 {
            return false;
        }
        let Some(src) = obj["ids"].as_object() else { return false };
        let mut next = IndexMap::new();
        let mut cids = HashMap::new();
        for (id, e) in src {
            if !id_re(id) || !e.is_object() {
                return false;
            }
            let Some(rec) = Record::from_json(id, e) else { return false };
            for p in &rec.pks {
                if cids.insert(p.cid.clone(), id.clone()).is_some() {
                    return false;
                }
            }
            next.insert(id.clone(), rec);
        }
        if self.max > 0 && next.len() > self.max {
            return false;
        }
        self.unused = next.values().filter(|r| !r.used).map(|r| r.id.clone()).collect();
        self.pendings = next.values().filter_map(|r| r.pending.as_ref().map(|p| (r.id.clone(), p.until))).collect();
        self.cid_idx = cids;
        self.ids = next;
        true
    }

    fn write(&mut self) -> bool {
        let Some(f) = self.file.clone() else { return true };
        if self.corrupt {
            return false;
        }
        let at = crate::now();
        let out: Map<String, Value> = self.ids.iter().map(|(k, r)| (k.clone(), r.to_json())).collect();
        if self.main_valid {
            let mut bak = f.as_os_str().to_owned();
            bak.push(".bak");
            let _ = std::fs::copy(&f, PathBuf::from(bak));
        }
        match write_json_atomic(&f, &json!({"v": 1, "savedAt": at, "ids": out}), true) {
            Ok(()) => {
                self.saved_at = at;
                self.main_valid = true;
                true
            }
            Err(e) => {
                tracing::error!("identidade: {e}");
                false
            }
        }
    }

    fn apply_recovery(rec: &Record, pk: &str, rpk: Option<&str>, t: u64) -> Record {
        let mut next = rec.clone();
        next.pk = pk.into();
        next.gen_ = rec.gen_ + 1;
        next.epoch = rec.epoch + 1;
        next.at = t as f64;
        if let Some(r) = rpk {
            next.rpk = r.into();
            next.rgen = rec.rgen + 1;
        }
        next.pending = None;
        next
    }

    /// Aplica as recuperações cuja carência venceu.
    fn settle(&mut self) -> bool {
        if self.corrupt || self.pendings.is_empty() {
            return true;
        }
        let t = crate::now();
        let due: Vec<String> = self.pendings.iter().filter(|(_, u)| **u <= t).map(|(k, _)| k.clone()).collect();
        if due.is_empty() {
            return true;
        }
        let mut before = Vec::new();
        for id in due {
            self.pendings.shift_remove(&id);
            let Some(rec) = self.ids.get(&id).cloned() else { continue };
            let Some(p) = rec.pending.clone() else { continue };
            self.ids.insert(id.clone(), Self::apply_recovery(&rec, &p.pk, p.rpk.as_deref(), t));
            before.push(rec);
        }
        if before.is_empty() {
            return true;
        }
        if !self.write() {
            for rec in before {
                if let Some(p) = &rec.pending {
                    self.pendings.insert(rec.id.clone(), p.until);
                }
                self.ids.insert(rec.id.clone(), rec);
            }
            return false;
        }
        self.settled.extend(before.into_iter().map(|r| r.id));
        true
    }

    pub fn get(&mut self, id: &str) -> Option<Record> {
        self.settle();
        self.ids.get(id).cloned()
    }

    pub fn has(&self, id: &str) -> bool {
        self.ids.contains_key(id)
    }

    pub fn len(&self) -> usize {
        self.ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub fn register(&mut self, id: &str, pk: &str, rpk: &str, legacy: bool) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !id_re(id) || !import_public_key(pk) || !import_public_key(rpk) {
            return fail("invalid");
        }
        if self.ids.contains_key(id) {
            return fail("exists");
        }
        let mut freed = None;
        if self.max > 0 && self.ids.len() >= self.max {
            freed = self.reclaim_oldest_unused();
            if freed.is_none() {
                return fail("full");
            }
        }
        let at = crate::now() as f64;
        let rec = Record {
            id: id.into(),
            pk: pk.into(),
            gen_: 1,
            rpk: rpk.into(),
            rgen: 1,
            epoch: 0,
            created: at,
            at,
            pending: None,
            pks: Vec::new(),
            legacy,
            used: false,
            kp: None,
            rec_grant: None,
        };
        self.ids.insert(id.into(), rec);
        self.unused.insert(id.into());
        if !self.write() {
            self.ids.shift_remove(id);
            self.unused.shift_remove(id);
            if let Some(f) = freed {
                self.unused.insert(f.id.clone());
                self.ids.insert(f.id.clone(), f);
            }
            return fail("io");
        }
        json!({"ok": true, "gen": 1, "rgen": 1, "epoch": 0})
    }

    fn reclaim_oldest_unused(&mut self) -> Option<Record> {
        if self.unused_ms == 0 {
            return None;
        }
        while let Some(id) = self.unused.first().cloned() {
            let Some(rec) = self.ids.get(&id) else {
                self.unused.shift_remove(&id);
                continue;
            };
            if (crate::now() as f64) - rec.created < self.unused_ms as f64 {
                return None;
            }
            let rec = self.ids.shift_remove(&id)?;
            self.unused.shift_remove(&id);
            self.pendings.shift_remove(&id);
            for p in &rec.pks {
                self.cid_idx.remove(&p.cid);
            }
            return Some(rec);
        }
        None
    }

    /// Faxina: identidades que nunca autenticaram e passaram do prazo.
    pub fn sweep_unused(&mut self) -> usize {
        if self.corrupt || self.unused_ms == 0 {
            return 0;
        }
        let mut gone = Vec::new();
        while let Some(r) = self.reclaim_oldest_unused() {
            gone.push(r);
        }
        if gone.is_empty() {
            return 0;
        }
        if !self.write() {
            for g in gone {
                self.unused.insert(g.id.clone());
                if let Some(p) = &g.pending {
                    self.pendings.insert(g.id.clone(), p.until);
                }
                for p in &g.pks {
                    self.cid_idx.insert(p.cid.clone(), g.id.clone());
                }
                self.ids.insert(g.id.clone(), g);
            }
            return 0;
        }
        gone.len()
    }

    /// Primeira autenticação: a identidade deixa de ser "sem uso" (gravado sem pressa, em até 30 s).
    pub fn touch(&mut self, id: &str) {
        let Some(rec) = self.ids.get_mut(id) else { return };
        if rec.used {
            return;
        }
        rec.used = true;
        self.unused.shift_remove(id);
        if self.file.is_none() || self.lazy_due.is_some() {
            return;
        }
        self.lazy_due = Some(crate::now() + self.lazy_ms);
    }

    /// Relógio do servidor: gravação adiada do `touch` e carências vencidas.
    pub fn tick(&mut self, now: u64) {
        if self.lazy_due.is_some_and(|d| now >= d) {
            self.lazy_due = None;
            self.write();
        }
        self.settle();
    }

    fn commit(&mut self, id: &str, old: Record, next: Record) -> bool {
        self.ids.insert(id.into(), next);
        if !self.write() {
            self.ids.insert(id.into(), old);
            return false;
        }
        true
    }

    pub fn rotate_primary(&mut self, id: &str, expected_gen: i64, new_pk: &str, sig_old: &str, sig_rec: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        if !import_public_key(new_pk) {
            return fail("invalid");
        }
        if expected_gen != rec.gen_ {
            if rec.gen_ == expected_gen + 1 && rec.pk == new_pk {
                return json!({"ok": true, "gen": rec.gen_, "idempotent": true});
            }
            return json!({"ok": false, "reason": "stale", "gen": rec.gen_});
        }
        let data = payload::rotate(id, expected_gen, new_pk);
        if !verify_signature(&rec.pk, &data, sig_old) || !verify_signature(&rec.rpk, &data, sig_rec) {
            return fail("bad-signature");
        }
        let mut next = rec.clone();
        next.pk = new_pk.into();
        next.gen_ += 1;
        next.epoch += 1;
        next.at = crate::now() as f64;
        next.pending = None;
        next.rec_grant = None;
        let (g, e) = (next.gen_, next.epoch);
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        self.pendings.shift_remove(id);
        json!({"ok": true, "gen": g, "epoch": e})
    }

    fn keypass_of(&self, rec: &Record, t: u64) -> Value {
        let kp = rec.kp.clone().unwrap_or_default();
        let left = (self.keypass_max - kp.n).max(0);
        let wait = if kp.n > 0 && kp.at > 0 { kp.at + self.keypass_interval_ms } else { 0 };
        json!({
            "used": kp.n, "max": self.keypass_max, "left": left, "next": if left > 0 && wait > t { wait } else { 0 },
            "last": kp.at, "log": kp.log, "intervalMs": self.keypass_interval_ms
        })
    }

    pub fn keypass(&self, id: &str) -> Option<Value> {
        self.ids.get(id).map(|r| self.keypass_of(r, crate::now()))
    }

    pub fn rotate_recovery(
        &mut self,
        id: &str,
        expected_rgen: i64,
        new_rpk: &str,
        sig_rec: &str,
        sig_old: &str,
    ) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        if !import_public_key(new_rpk) {
            return fail("invalid");
        }
        if expected_rgen != rec.rgen {
            if rec.rgen == expected_rgen + 1 && rec.rpk == new_rpk {
                return json!({"ok": true, "rgen": rec.rgen, "idempotent": true});
            }
            return json!({"ok": false, "reason": "stale", "rgen": rec.rgen});
        }
        let data = payload::rotate_recovery(id, expected_rgen, new_rpk);
        if !verify_signature(&rec.rpk, &data, sig_rec) || !verify_signature(&rec.pk, &data, sig_old) {
            return fail("bad-signature");
        }
        let t = crate::now();
        let kp = rec.kp.clone().unwrap_or_default();
        if kp.n >= self.keypass_max {
            return json!({"ok": false, "reason": "keypass-limit", "keypass": self.keypass_of(&rec, t)});
        }
        if kp.n > 0 && kp.at > 0 && t.saturating_sub(kp.at) < self.keypass_interval_ms {
            return json!({"ok": false, "reason": "keypass-wait", "until": kp.at + self.keypass_interval_ms, "keypass": self.keypass_of(&rec, t)});
        }
        let mut log = kp.log.clone();
        log.push(t);
        if log.len() > KEYPASS_LOG_MAX {
            log.drain(..log.len() - KEYPASS_LOG_MAX);
        }
        let mut next = rec.clone();
        next.rpk = new_rpk.into();
        next.rgen += 1;
        next.epoch += 1;
        next.at = t as f64;
        next.kp = Some(Keypass { n: kp.n + 1, at: t, log, adm: kp.adm });
        next.pending = None;
        next.rec_grant = None;
        let (rg, e) = (next.rgen, next.epoch);
        let kpv = self.keypass_of(&next, t);
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        self.pendings.shift_remove(id);
        json!({"ok": true, "rgen": rg, "epoch": e, "keypass": kpv})
    }

    /// O administrador libera mais uma renovação da chave de recuperação.
    pub fn keypass_grant(&mut self, id: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        let kp = rec.kp.clone().unwrap_or_default();
        let now = crate::now();
        if kp.n < self.keypass_max && !(kp.at > 0 && now.saturating_sub(kp.at) < self.keypass_interval_ms) {
            return fail("not-needed");
        }
        let mut next = rec.clone();
        next.kp = Some(Keypass { n: kp.n.min(self.keypass_max - 1), at: 0, log: kp.log.clone(), adm: kp.adm + 1 });
        let kpv = self.keypass_of(&next, now);
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        json!({"ok": true, "keypass": kpv})
    }

    pub fn list_ids(&self) -> Vec<String> {
        self.ids.keys().cloned().collect()
    }

    fn grant_hash(token: &str) -> String {
        URL_SAFE_NO_PAD.encode(Sha256::digest(format!("{REC_GRANT_DOMAIN}|{token}")))
    }

    /// Autorização de uso único (24 h) para registrar uma chave de recuperação nova (recuperação assistida).
    pub fn recover_grant(&mut self, id: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        let token = format!("r{}", crate::hex(&crate::random_bytes::<10>()));
        let exp = crate::now() + REC_GRANT_TTL;
        let mut next = rec.clone();
        next.rec_grant = Some(RecGrant { h: Self::grant_hash(&token), exp, rpk: None });
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        json!({"ok": true, "token": token, "exp": exp})
    }

    pub fn recover_grant_info(&self, id: &str) -> Option<Value> {
        let g = self.ids.get(id)?.rec_grant.as_ref()?;
        Some(json!({"active": g.exp > crate::now(), "exp": g.exp, "used": g.rpk.is_some()}))
    }

    pub fn recover_redeem(&mut self, token: &str, rpk: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let ok_token = token.len() == 21
            && token.starts_with('r')
            && token[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !ok_token || !import_public_key(rpk) {
            return fail("invalid");
        }
        let h = Self::grant_hash(token);
        let hit = self
            .ids
            .iter()
            .find(|(_, r)| r.rec_grant.as_ref().is_some_and(|g| crate::admission::safe_eq(&g.h, &h)))
            .map(|(k, r)| (k.clone(), r.clone()));
        let Some((id, rec)) = hit else { return fail("invalid") };
        let Some(grant) = rec.rec_grant.clone() else { return fail("invalid") };
        if crate::now() >= grant.exp {
            return fail("expired");
        }
        if let Some(g_rpk) = &grant.rpk {
            if rec.rpk == rpk && &rec.rpk == g_rpk {
                return json!({"ok": true, "id": id, "gen": rec.gen_, "rgen": rec.rgen, "epoch": rec.epoch, "idempotent": true});
            }
            return fail("used");
        }
        let t = crate::now();
        let had_pending = rec.pending.is_some();
        let mut next = rec.clone();
        next.rpk = rpk.into();
        next.rgen += 1;
        next.epoch += 1;
        next.at = t as f64;
        next.rec_grant = Some(RecGrant { h: grant.h, exp: grant.exp, rpk: Some(rpk.into()) });
        next.pending = None;
        let (g, rg, e) = (next.gen_, next.rgen, next.epoch);
        if !self.commit(&id, rec, next) {
            return fail("io");
        }
        if had_pending {
            self.pendings.shift_remove(&id);
        }
        let mut o = json!({"ok": true, "id": id, "gen": g, "rgen": rg, "epoch": e});
        if had_pending {
            o["cancelled"] = json!(true);
        }
        o
    }

    pub fn revoke_sessions(&mut self, id: &str, expected_epoch: i64, sig_old: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        if expected_epoch != rec.epoch {
            if rec.epoch == expected_epoch + 1 {
                return json!({"ok": true, "epoch": rec.epoch, "idempotent": true});
            }
            return json!({"ok": false, "reason": "stale", "epoch": rec.epoch});
        }
        if !verify_signature(&rec.pk, &payload::revoke(id, expected_epoch), sig_old) {
            return fail("bad-signature");
        }
        let mut next = rec.clone();
        next.epoch += 1;
        next.at = crate::now() as f64;
        let e = next.epoch;
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        json!({"ok": true, "epoch": e})
    }

    pub fn remove(&mut self, id: &str, sig_old: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        if !verify_signature(&rec.pk, &payload::remove(id), sig_old) {
            return fail("bad-signature");
        }
        self.ids.shift_remove(id);
        self.unused.shift_remove(id);
        if !self.write() {
            if !rec.used {
                self.unused.insert(id.into());
            }
            self.ids.insert(id.into(), rec);
            return fail("io");
        }
        self.pendings.shift_remove(id);
        for p in &rec.pks {
            self.cid_idx.remove(&p.cid);
        }
        json!({"ok": true})
    }

    #[allow(clippy::too_many_arguments)]
    pub fn recover_primary(
        &mut self,
        id: &str,
        expected_gen: i64,
        new_pk: &str,
        new_rpk: Option<&str>,
        sig_rec: &str,
        grace_ms: u64,
    ) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        if !import_public_key(new_pk) || new_rpk.is_some_and(|r| !import_public_key(r) || r == rec.rpk) {
            return fail("invalid");
        }
        if expected_gen != rec.gen_ {
            if rec.gen_ == expected_gen + 1 && rec.pk == new_pk {
                return json!({"ok": true, "gen": rec.gen_, "idempotent": true});
            }
            return json!({"ok": false, "reason": "stale", "gen": rec.gen_});
        }
        if let Some(p) = &rec.pending {
            if p.pk == new_pk && p.rpk.as_deref().unwrap_or("") == new_rpk.unwrap_or("") {
                return json!({"ok": true, "gen": rec.gen_, "epoch": rec.epoch, "pending": true, "until": p.until});
            }
            return json!({"ok": false, "reason": "pending", "until": p.until});
        }
        if !verify_signature(&rec.rpk, &payload::recover(id, expected_gen, new_pk, new_rpk), sig_rec) {
            return fail("bad-signature");
        }
        let now = crate::now();
        if grace_ms == 0 {
            let applied = Self::apply_recovery(&rec, new_pk, new_rpk, now);
            let (g, rg, e) = (applied.gen_, applied.rgen, applied.epoch);
            if !self.commit(id, rec, applied) {
                return fail("io");
            }
            return json!({"ok": true, "gen": g, "rgen": rg, "epoch": e});
        }
        let until = now + grace_ms;
        let mut next = rec.clone();
        next.pending = Some(Pending { pk: new_pk.into(), until, rpk: new_rpk.map(str::to_owned) });
        next.at = now as f64;
        let (g, e) = (rec.gen_, rec.epoch);
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        self.pendings.insert(id.into(), until);
        json!({"ok": true, "gen": g, "epoch": e, "pending": true, "until": until})
    }

    pub fn cancel_recovery(&mut self, id: &str, expected_gen: i64, sig_old: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        if expected_gen != rec.gen_ {
            return json!({"ok": false, "reason": "stale", "gen": rec.gen_});
        }
        if rec.pending.is_none() {
            return json!({"ok": true, "gen": rec.gen_});
        }
        if !verify_signature(&rec.pk, &payload::cancel_recovery(id, expected_gen), sig_old) {
            return fail("bad-signature");
        }
        let mut next = rec.clone();
        next.at = crate::now() as f64;
        next.pending = None;
        let g = rec.gen_;
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        self.pendings.shift_remove(id);
        json!({"ok": true, "gen": g})
    }

    /// Passkey nova (já conferida pelo WebAuthn): `cred` traz `cid`, `pk`, `alg` e `sc`.
    pub fn add_passkey(&mut self, id: &str, cred: &Value, name: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        let mut c = cred.clone();
        c["name"] = json!(crate::validate::slice16(name, PK_NAME_MAX));
        c["created"] = json!(crate::now());
        let Some(p) = valid_passkey(&c) else { return fail("invalid") };
        if let Some(owner) = self.cid_idx.get(&p.cid) {
            return fail(if owner == id { "pk-exists" } else { "invalid" });
        }
        if rec.pks.len() >= MAX_PASSKEYS {
            return fail("pk-full");
        }
        let mut next = rec.clone();
        next.pks.push(p.clone());
        next.at = crate::now() as f64;
        let n = next.pks.len();
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        self.cid_idx.insert(p.cid, id.into());
        json!({"ok": true, "n": n})
    }

    pub fn remove_passkey(&mut self, id: &str, cid: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        let rest: Vec<Passkey> = rec.pks.iter().filter(|p| p.cid != cid).cloned().collect();
        if rest.len() == rec.pks.len() {
            return fail("pk-missing");
        }
        let mut next = rec.clone();
        next.at = crate::now() as f64;
        next.pks = rest;
        let n = next.pks.len();
        if !self.commit(id, rec, next) {
            return fail("io");
        }
        self.cid_idx.remove(cid);
        json!({"ok": true, "n": n})
    }

    pub fn passkey_by_cid(&mut self, cid: &str) -> Option<(String, Passkey)> {
        let id = self.cid_idx.get(cid)?.clone();
        self.settle();
        let rec = self.ids.get(&id)?;
        rec.pks.iter().find(|p| p.cid == cid).map(|p| (id.clone(), p.clone()))
    }

    pub fn passkeys(&self, id: &str) -> Value {
        Value::Array(
            self.ids
                .get(id)
                .map(|r| {
                    r.pks
                        .iter()
                        .map(|p| json!({"cid": p.cid, "name": p.name, "created": p.created, "used": p.used.unwrap_or(0.0)}))
                        .collect()
                })
                .unwrap_or_default(),
        )
    }

    /// Entrar por passkey num aparelho novo: a chave principal passa a ser a do aparelho (gera nova geração e época).
    pub fn attach_device(&mut self, id: &str, cid: &str, sc: i64, new_pk: &str) -> Res {
        if self.corrupt {
            return fail("corrupt");
        }
        if !self.settle() {
            return fail("io");
        }
        let Some(rec) = self.ids.get(id).cloned() else { return fail("notfound") };
        if !import_public_key(new_pk) || sc < 0 {
            return fail("invalid");
        }
        let Some(i) = rec.pks.iter().position(|p| p.cid == cid) else { return fail("pk-missing") };
        let t = crate::now();
        let mut next = rec.clone();
        next.pks[i].sc = sc;
        next.pks[i].used = Some(t as f64);
        next.pk = new_pk.into();
        next.gen_ += 1;
        next.epoch += 1;
        next.at = t as f64;
        next.used = true;
        let cancelled = rec.pending.is_some();
        next.pending = None;
        let (g, rg, e) = (next.gen_, next.rgen, next.epoch);
        self.unused.shift_remove(id);
        if !self.commit(id, rec.clone(), next) {
            if !rec.used {
                self.unused.insert(id.into());
            }
            return fail("io");
        }
        self.pendings.shift_remove(id);
        json!({"ok": true, "gen": g, "rgen": rg, "epoch": e, "cancelled": cancelled})
    }

    pub fn verify(&mut self, id: &str, data: &str, sig: &str) -> bool {
        self.settle();
        self.ids.get(id).is_some_and(|r| verify_signature(&r.pk, data, sig))
    }

    pub fn verify_recovery(&mut self, id: &str, data: &str, sig: &str) -> bool {
        self.settle();
        self.ids.get(id).is_some_and(|r| verify_signature(&r.rpk, data, sig))
    }

    pub fn flush(&mut self) -> bool {
        self.lazy_due = None;
        if self.file.is_some() { self.write() } else { true }
    }

    pub fn stats(&self) -> Value {
        let bytes = self.file.as_ref().and_then(|f| std::fs::metadata(f).ok()).map_or(0, |m| m.len());
        json!({
            "count": self.ids.len(), "unused": self.unused.len(), "pending": self.pendings.len(),
            "passkeys": self.cid_idx.len(), "max": self.max, "corrupt": self.corrupt, "savedAt": self.saved_at, "bytes": bytes
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sky_identity::KeyPair;

    #[test]
    fn register_rotate_recover() {
        let mut r = IdentityRegistry::new("", 100, 0, KEYPASS_INTERVAL_MS);
        let (a, b, c) = (KeyPair::generate(), KeyPair::generate(), KeyPair::generate());
        let (pa, pb, pc) = (a.public_b64().unwrap(), b.public_b64().unwrap(), c.public_b64().unwrap());
        let id = "abcdefghijk2";
        assert_eq!(r.register(id, &pa, &pb, false)["ok"], true);
        assert_eq!(r.register(id, &pa, &pb, false)["reason"], "exists");
        let data = payload::rotate(id, 1, &pc);
        let res = r.rotate_primary(id, 1, &pc, &a.sign_b64(&data), &b.sign_b64(&data));
        assert_eq!(res, json!({"ok": true, "gen": 2, "epoch": 1}));
        assert_eq!(r.rotate_primary(id, 1, &pc, "x", "y")["idempotent"], true);
        // recuperação imediata (carência 0) com a chave de recuperação
        let d = KeyPair::generate();
        let pd = d.public_b64().unwrap();
        let data = payload::recover(id, 2, &pd, None);
        let res = r.recover_primary(id, 2, &pd, None, &b.sign_b64(&data), 0);
        assert_eq!(res["gen"], 3);
        assert!(r.verify(id, "oi", &d.sign_b64("oi")));
        assert!(!r.verify(id, "oi", &a.sign_b64("oi")));
    }
}
