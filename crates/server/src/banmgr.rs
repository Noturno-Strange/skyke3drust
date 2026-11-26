//! Banimentos por IP (`lib/banmgr.js`): a chave é um hash com sal do servidor (o IP nunca é gravado).

use crate::limits::BanStore;
use crate::persist::{read_json_file, write_json_atomic};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

#[derive(Debug)]
pub struct BanManager {
    pub bans: BanStore,
    pub on: bool,
    file: Option<PathBuf>,
    salt: String,
    pending: bool,
    /// Gravação marcada para este instante (ms), como o `setTimeout` de 5 s do original.
    due: Option<u64>,
}

impl BanManager {
    pub fn new(on: bool, file: &str, levels: Vec<u64>, reset_ms: u64, max: usize) -> Self {
        let mut m = Self {
            bans: BanStore::new(levels, reset_ms, max),
            on,
            file: (!file.is_empty()).then(|| PathBuf::from(file)),
            salt: String::new(),
            pending: false,
            due: None,
        };
        if on && let Some(f) = m.file.clone() {
            match read_json_file(&f) {
                Ok(Some(obj)) if obj["v"] == 1 => {
                    if let Some(s) = obj["salt"].as_str().filter(|s| s.len() >= 16) {
                        m.salt = s.to_owned();
                    }
                    m.bans.restore(&obj["bans"]);
                }
                Ok(_) => {}
                Err(_) => {
                    let mut bak = f.as_os_str().to_owned();
                    bak.push(".bak");
                    let _ = std::fs::rename(&f, bak);
                    tracing::warn!("bans: arquivo inválido, começando limpo.");
                }
            }
        }
        if m.salt.is_empty() {
            m.salt = URL_SAFE_NO_PAD.encode(crate::random_bytes::<24>());
        }
        m
    }

    pub fn key(&self, k: &str) -> String {
        let mut h = Sha256::new();
        h.update(self.salt.as_bytes());
        h.update(b"|");
        h.update(k.as_bytes());
        URL_SAFE_NO_PAD.encode(h.finalize())
    }

    /// ID curto mostrado no painel (não revela o IP).
    pub fn host_id(&self, ip: &str) -> String {
        self.key(ip)[..12].to_owned()
    }

    pub fn check(&mut self, ip: &str, now: u64) -> u64 {
        if !self.on {
            return 0;
        }
        let k = self.key(ip);
        self.bans.check(&k, now)
    }

    pub fn strike(&mut self, ip: &str, why: &str, now: u64) -> u64 {
        if !self.on {
            return 0;
        }
        let k = self.key(ip);
        let (ms, _, _) = self.bans.strike(&k, now);
        tracing::warn!("banido por {why} ({}s)", (ms as f64 / 1000.0).round());
        self.save(now);
        ms
    }

    pub fn save(&mut self, now: u64) {
        if !self.on || self.file.is_none() {
            return;
        }
        self.pending = true;
        if self.due.is_none() {
            self.due = Some(now + 5000);
        }
    }

    /// Chamado pelo relógio do servidor: grava quando o prazo chegou.
    pub fn tick(&mut self, now: u64) {
        if self.due.is_some_and(|d| now >= d) {
            self.due = None;
            self.write(now);
        }
    }

    fn write(&mut self, now: u64) {
        if !self.on || !self.pending {
            return;
        }
        let Some(f) = &self.file else { return };
        self.pending = false;
        let obj = json!({"v": 1, "salt": self.salt, "savedAt": now, "bans": self.bans.snapshot(now)});
        if let Err(e) = write_json_atomic(f, &obj, false) {
            tracing::error!("bans: {e}");
        }
    }

    pub fn flush(&mut self, now: u64) {
        self.due = None;
        self.write(now);
    }
}
