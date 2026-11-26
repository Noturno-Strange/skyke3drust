//! Autenticação por desafio e resposta e sessões (`lib/auth.js`). O token de sessão só existe na memória (guardado
//! pelo hash) e morre quando a geração ou a época da identidade mudam.

use crate::identity::{IdentityRegistry, Record, payload, verify_signature};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use indexmap::IndexMap;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

const HOUR: u64 = 3_600_000;

/// Que chave assina cada ação: a principal, a de recuperação ou qualquer uma das duas.
pub fn action_rule(action: &str) -> Option<&'static str> {
    Some(match action {
        "join" | "rotate" | "rotate-recovery" | "remove" | "cancel-recovery" | "passkey" => "primary",
        "revoke" => "both",
        "recover" => "recovery",
        _ => return None,
    })
}
const RECOVERY_ACTIONS: [&str; 2] = ["revoke", "recover"];

fn token_key(t: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(t.as_bytes()))
}

fn rand_token() -> String {
    URL_SAFE_NO_PAD.encode(crate::random_bytes::<32>())
}

#[derive(Debug, Clone)]
struct Challenge {
    id: String,
    action: String,
    exp: u64,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub gen_: i64,
    pub epoch: i64,
    pub scope: String,
    pub exp: u64,
}

#[derive(Debug, Clone)]
struct PkChallenge {
    purpose: String,
    id: String,
    owner: u64,
    exp: u64,
}

#[derive(Debug)]
pub struct Auth {
    challenge_ttl: u64,
    session_ttl: u64,
    recovery_ttl: u64,
    max_challenges: usize,
    max_sessions: usize,
    pk_ttl: u64,
    challenges: IndexMap<String, Challenge>,
    sessions: IndexMap<String, Session>,
    by_id: HashMap<String, HashSet<String>>,
    pk_challenges: IndexMap<String, PkChallenge>,
    pk_by_owner: HashMap<u64, HashMap<String, String>>,
    ghost_pk: String,
}

impl Default for Auth {
    fn default() -> Self {
        Self::new()
    }
}

impl Auth {
    pub fn new() -> Self {
        Self {
            challenge_ttl: 60_000,
            session_ttl: 12 * HOUR,
            recovery_ttl: 15 * 60_000,
            max_challenges: 20_000,
            max_sessions: 100_000,
            pk_ttl: 120_000,
            challenges: IndexMap::new(),
            sessions: IndexMap::new(),
            by_id: HashMap::new(),
            pk_challenges: IndexMap::new(),
            pk_by_owner: HashMap::new(),
            ghost_pk: sky_identity::KeyPair::generate().public_b64().unwrap_or_default(),
        }
    }

    fn drop_session(&mut self, key: &str) {
        let Some(s) = self.sessions.shift_remove(key) else { return };
        if let Some(set) = self.by_id.get_mut(&s.id) {
            set.remove(key);
            if set.is_empty() {
                self.by_id.remove(&s.id);
            }
        }
    }

    fn sweep_challenges(&mut self) -> usize {
        let t = crate::now();
        let (a, b) = (self.challenges.len(), self.pk_challenges.len());
        self.challenges.retain(|_, c| c.exp > t);
        self.pk_challenges.retain(|_, c| c.exp > t);
        a - self.challenges.len() + b - self.pk_challenges.len()
    }

    fn sweep_sessions(&mut self) -> usize {
        let t = crate::now();
        let gone: Vec<String> = self.sessions.iter().filter(|(_, s)| s.exp <= t).map(|(k, _)| k.clone()).collect();
        for k in &gone {
            self.drop_session(k);
        }
        gone.len()
    }

    pub fn sweep(&mut self) {
        self.sweep_challenges();
        self.sweep_sessions();
    }

    pub fn challenge(&mut self, reg: &mut IdentityRegistry, id: &str, action: &str) -> Value {
        if !crate::identity::id_re(id) || action_rule(action).is_none() {
            return json!({"ok": false, "reason": "invalid"});
        }
        if self.max_challenges > 0 {
            if self.challenges.len() >= self.max_challenges {
                self.sweep_challenges();
            }
            if self.challenges.len() >= self.max_challenges {
                return json!({"ok": false, "reason": "busy"});
            }
        }
        let rec = reg.get(id);
        let nonce = rand_token();
        let exp = crate::now() + self.challenge_ttl;
        self.challenges.insert(nonce.clone(), Challenge { id: id.into(), action: action.into(), exp });
        json!({
            "ok": true, "nonce": nonce, "exp": exp, "gen": rec.as_ref().map_or(1, |r| r.gen_),
            "rgen": rec.as_ref().map_or(1, |r| r.rgen), "epoch": rec.as_ref().map_or(0, |r| r.epoch)
        })
    }

    pub fn verify_proof(
        &mut self,
        reg: &mut IdentityRegistry,
        id: &str,
        nonce: &str,
        sig: &str,
        action: &str,
    ) -> Value {
        let Some(c) = self.challenges.shift_remove(nonce) else { return json!({"ok": false, "reason": "challenge"}) };
        if c.id != id || c.action != action {
            return json!({"ok": false, "reason": "challenge"});
        }
        if c.exp <= crate::now() {
            return json!({"ok": false, "reason": "expired"});
        }
        let data = payload::auth(id, nonce, action);
        let Some(rec) = reg.get(id) else {
            // mesmo custo para identidade inexistente (não revela quem existe)
            verify_signature(&self.ghost_pk, &data, sig);
            return json!({"ok": false, "reason": "bad-signature"});
        };
        let scope = match action_rule(action) {
            Some("primary") if reg.verify(id, &data, sig) => "primary",
            Some("recovery") if reg.verify_recovery(id, &data, sig) => "recovery",
            Some("both") if reg.verify(id, &data, sig) => "primary",
            Some("both") if reg.verify_recovery(id, &data, sig) => "recovery",
            _ => "",
        };
        if scope.is_empty() {
            return json!({"ok": false, "reason": "bad-signature"});
        }
        if scope == "primary" {
            reg.touch(id);
        }
        self.mint(id, &rec, scope)
    }

    fn mint(&mut self, id: &str, rec: &Record, scope: &str) -> Value {
        if self.max_sessions > 0 && self.sessions.len() >= self.max_sessions {
            self.sweep_sessions();
            while self.sessions.len() >= self.max_sessions {
                let Some(k) = self.sessions.keys().next().cloned() else { break };
                self.drop_session(&k);
            }
        }
        let token = rand_token();
        let key = token_key(&token);
        let exp = crate::now() + if scope == "recovery" { self.recovery_ttl } else { self.session_ttl };
        self.sessions
            .insert(key.clone(), Session { id: id.into(), gen_: rec.gen_, epoch: rec.epoch, scope: scope.into(), exp });
        self.by_id.entry(id.into()).or_default().insert(key);
        json!({"ok": true, "token": token, "exp": exp, "gen": rec.gen_, "rgen": rec.rgen, "epoch": rec.epoch, "scope": scope})
    }

    /// Sessão nova sem desafio (depois de um registro ou de uma troca de chave que já provou a posse).
    pub fn issue(&mut self, reg: &mut IdentityRegistry, id: &str, scope: &str) -> Option<Value> {
        if scope != "primary" && scope != "recovery" {
            return None;
        }
        let rec = reg.get(id)?;
        Some(self.mint(id, &rec, scope))
    }

    pub fn verify_token(&mut self, reg: &mut IdentityRegistry, token: &str, action: Option<&str>) -> Option<Session> {
        if token.is_empty() || token.len() > 200 {
            return None;
        }
        let key = token_key(token);
        let s = self.sessions.get(&key)?.clone();
        if s.exp <= crate::now() {
            self.drop_session(&key);
            return None;
        }
        let Some(rec) = reg.get(&s.id) else {
            self.drop_session(&key);
            return None;
        };
        if rec.gen_ != s.gen_ || rec.epoch != s.epoch {
            self.drop_session(&key);
            return None;
        }
        if let Some(a) = action {
            let rule = action_rule(a)?;
            if s.scope == "recovery" {
                if !RECOVERY_ACTIONS.contains(&a) {
                    return None;
                }
            } else if rule == "recovery" {
                return None;
            }
        }
        Some(s)
    }

    pub fn revoke_sessions(&mut self, id: &str) -> usize {
        let Some(set) = self.by_id.get(id).cloned() else { return 0 };
        for k in &set {
            self.drop_session(k);
        }
        set.len()
    }

    /// Desafio WebAuthn (`reg` para cadastrar, `login` para entrar), um por conexão e finalidade.
    pub fn pk_challenge(&mut self, purpose: &str, id: &str, owner: u64) -> Value {
        if purpose != "reg" && purpose != "login" {
            return json!({"ok": false, "reason": "invalid"});
        }
        if self.max_challenges > 0 {
            if self.pk_challenges.len() >= self.max_challenges {
                self.sweep_challenges();
            }
            if self.pk_challenges.len() >= self.max_challenges {
                return json!({"ok": false, "reason": "busy"});
            }
        }
        let mine = self.pk_by_owner.entry(owner).or_default();
        if let Some(prev) = mine.get(purpose).cloned() {
            self.pk_challenges.shift_remove(&prev);
        }
        let challenge = rand_token();
        let exp = crate::now() + self.pk_ttl;
        self.pk_challenges
            .insert(challenge.clone(), PkChallenge { purpose: purpose.into(), id: id.into(), owner, exp });
        self.pk_by_owner.entry(owner).or_default().insert(purpose.into(), challenge.clone());
        json!({"ok": true, "challenge": challenge, "exp": exp})
    }

    pub fn take_pk_challenge(&mut self, challenge: &str, purpose: &str, owner: u64, id: &str) -> Value {
        let Some(c) = self.pk_challenges.shift_remove(challenge) else {
            return json!({"ok": false, "reason": "challenge"});
        };
        if let Some(mine) = self.pk_by_owner.get_mut(&c.owner)
            && mine.get(&c.purpose).is_some_and(|x| x == challenge)
        {
            mine.remove(&c.purpose);
        }
        if c.purpose != purpose || c.owner != owner || c.id != id {
            return json!({"ok": false, "reason": "challenge"});
        }
        if c.exp <= crate::now() {
            return json!({"ok": false, "reason": "expired"});
        }
        json!({"ok": true})
    }

    /// A conexão fechou: os desafios WebAuthn dela deixam de valer.
    pub fn forget_owner(&mut self, owner: u64) {
        if let Some(m) = self.pk_by_owner.remove(&owner) {
            for c in m.values() {
                self.pk_challenges.shift_remove(c);
            }
        }
    }

    pub fn stats(&self) -> Value {
        json!({"challenges": self.challenges.len() + self.pk_challenges.len(), "sessions": self.sessions.len(), "ids": self.by_id.len()})
    }
}
