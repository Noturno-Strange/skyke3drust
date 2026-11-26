//! O estado do servidor e as regras do jogo (`server.js`): conexões, salas, jogadores e bots, a entrada (criar, entrar
//! e reconectar), o limite por tipo de mensagem, os tratadores de cada mensagem e o relógio de 100 ms que move os bots e
//! envia o estado das salas em binário.
//!
//! Tudo fica numa estrutura só, protegida por um `Mutex` (como o laço único do Node): cada conexão envia o que recebe
//! para cá e recebe as respostas por um canal próprio.

use crate::admission::Used;
use crate::auth::Auth;
use crate::banmgr::BanManager;
use crate::bots::{Bot, BotChat};
use crate::catalog::Catalog;
use crate::config::Config;
use crate::identity::IdentityRegistry;
use crate::limits::Limiter;
use crate::names::NameRegistry;
use crate::seclog::SecLog;
use crate::validate::{self as V, normalize_message};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use indexmap::{IndexMap, IndexSet};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::mpsc::UnboundedSender;

pub const VERSION: &str = "3.30.1";
const MSG_PER_SEC: u32 = 150;
const SIG_PER_SEC: u32 = 120;
const BIN_PER_SEC: u32 = 150;
const MAX_BUFFERED: usize = 1 << 20;
const RESUME_GONE: &str = "Sessão expirada ou inválida. Entre novamente com o código da sala.";
pub const BOT_NAMES: [&str; 20] = [
    "Marina", "Caio", "Duda", "Bruno", "Lia", "Theo", "Nina", "Davi", "Bia", "Rafa", "Luna", "Gabi", "Enzo", "Maya",
    "Tiago", "Alice", "Leo", "Clara", "Pedro", "Sofia",
];

/// Limite por tipo de mensagem e por conexão: (intervalo mínimo, máximo na janela, janela), em ms (`RATE`).
pub fn rate_rule(t: &str) -> Option<(u64, usize, u64)> {
    Some(match t {
        "chat" => (700, 5, 5000),
        "gchat" => (2500, 6, 60000),
        "gsub" => (300, 6, 10000),
        "accountget" => (200, 30, 60000),
        "supply" => (300, 60, 60000),
        "healthget" => (500, 30, 60000),
        "medcraft" | "medheal" => (1200, 20, 60000),
        "friends" => (1000, 6, 60000),
        "auth" | "proof" => (0, 30, 60000),
        "idreg" | "idrot" | "idrec" | "idgrant" | "idcancel" | "idrev" | "pkreg" => (0, 6, 60000),
        "pkopt" | "pklist" => (0, 20, 60000),
        "pkdel" | "pklogin" => (0, 10, 60000),
        "pubp" => (300, 10, 60000),
        "plike" => (500, 20, 60000),
        "petget" | "cookget" | "giftget" | "giftsee" | "farmget" | "gblist" => (500, 20, 60000),
        "cookdo" => (1200, 20, 60000),
        "cookrenew" => (2000, 5, 60000),
        "giftsend" => (2000, 8, 60000),
        "giftdel" | "gbdel" | "facc" => (300, 20, 60000),
        "farmdo" => (150, 120, 60000),
        "petadopt" => (2000, 4, 60000),
        "petact" => (400, 30, 60000),
        "petdress" => (500, 10, 60000),
        "petrel" => (2000, 3, 60000),
        "petsocial" => (1000, 15, 60000),
        "gbook" | "freq" => (2000, 6, 60000),
        "finv" => (3000, 10, 60000),
        "item" => (150, 15, 5000),
        "gest" => (1500, 8, 10000),
        "emo" => (400, 12, 5000),
        "cfg" => (200, 15, 5000),
        "video" => (0, 4, 30000),
        "kjoin" | "ksong" => (1000, 4, 10000),
        "bots" => (1000, 3, 5000),
        "move" => (25, 40, 1000),
        "sig" => (0, 300, 10000),
        "speak" | "vlisten" => (100, 10, 1000),
        "vmute" => (0, 60, 10000),
        "vcap" => (1000, 3, 10000),
        "ice" => (60000, 1, 60000),
        "vjoin" | "vleave" | "vstop" | "vend" | "djstop" | "kleave" => (500, 4, 5000),
        "dj" => (2000, 4, 30000),
        "kpass" => (1000, 3, 5000),
        "kdur" | "kend" => (1000, 2, 10000),
        "ksync" | "vsync" => (200, 10, 5000),
        "tsync" => (100, 10, 5000),
        "report" => (15000, 3, 600_000),
        "create" => (1000, 3, 60000),
        "join" | "resume" => (1000, 5, 60000),
        "leave" => (0, 1, 1000),
        _ => return None,
    })
}

/// Ações de interface: acima do limite são descartadas sem punir.
const SOFT: [&str; 26] = [
    "move",
    "healthget",
    "farmget",
    "cookget",
    "petget",
    "friends",
    "gsub",
    "cfg",
    "item",
    "emo",
    "gest",
    "speak",
    "vlisten",
    "vmute",
    "vcap",
    "vjoin",
    "vleave",
    "vstop",
    "vend",
    "djstop",
    "kleave",
    "kpass",
    "ksync",
    "vsync",
    "tsync",
    "bots",
];
/// Estados que, acima do limite, guardam só o último e o aplicam quando o limite liberar.
const TRAIL: [&str; 5] = ["cfg", "item", "vlisten", "speak", "move"];
/// Limite por conta (todas as abas juntas), além do limite por conexão (`ACTION_LIMIT`).
const ACCOUNT_LIMITED: [&str; 16] = [
    "chat",
    "gchat",
    "emo",
    "gest",
    "finv",
    "freq",
    "gbook",
    "plike",
    "farmdo",
    "cookdo",
    "cookrenew",
    "medcraft",
    "medheal",
    "giftsend",
    "supply",
    "petact",
];

// ---------- saída ----------

/// O que vai para uma conexão.
#[derive(Debug, Clone)]
pub enum Out {
    Text(Arc<str>),
    Bin(Arc<[u8]>),
    Close(u16, String),
    Ping,
    /// Derruba sem fechamento educado (`terminate`).
    Kill,
}

/// Canal de saída de uma conexão e quantos bytes ainda não saíram (`bufferedAmount`).
#[derive(Debug, Clone)]
pub struct Link {
    pub tx: UnboundedSender<Out>,
    pub buffered: Arc<AtomicUsize>,
}

// ---------- estado ----------

#[derive(Debug, Default)]
pub struct Conn {
    pub id: u64,
    pub link: Option<Link>,
    pub open: bool,
    pub ip: String,
    pub page_origin: String,
    pub me: Option<String>,
    pub room: Option<String>,
    pub last_txt: String,
    pub last_at: u64,
    pub gest_snd: u64,
    pub gest_hits: Vec<u64>,
    pub since: u64,
    pub fid: String,
    pub h: String,
    pub watch: HashSet<String>,
    pub shown: HashSet<String>,
    pub g_on: bool,
    pub g_last: String,
    pub g_last_at: u64,
    pub pp: String,
    pub ident_id: String,
    pub ident_gen: i64,
    pub ident_epoch: i64,
    pub ident_scope: String,
    pub eph_id: String,
    rl: HashMap<String, (u64, Vec<u64>)>,
    defer: HashMap<String, (Option<Value>, Option<u64>)>,
    n: u32,
    sig_n: u32,
    bin_n: u32,
    win_sec: u64,
    last_warn: u64,
    bad_hits: Vec<u64>,
    punished: bool,
    /// Prazo para entrar numa sala (`jt`).
    jt: Option<u64>,
    jt_at: u64,
    pub alive: bool,
    /// Código de fechamento que este lado mandou.
    pub closing: Option<u16>,
}

#[derive(Debug, Clone, Default)]
pub struct Ident {
    pub id: String,
    pub gen_: i64,
    pub epoch: i64,
}

#[derive(Debug, Clone, Default)]
pub struct Player {
    pub id: String,
    pub bot: bool,
    pub name: String,
    pub cfg: Value,
    pub x: f64,
    pub z: f64,
    pub ry: f64,
    pub y: f64,
    pub s: i64,
    pub item: Value,
    pub talk: bool,
    pub talk_at: u64,
    pub lst: String,
    pub ip: String,
    pub oid: String,
    pub mk: String,
    pub fid: String,
    pub pp: String,
    pub conn: Option<u64>,
    pub ready: bool,
    pub online: bool,
    pub retire: Option<u64>,
    pub token: String,
    pub rk: String,
    pub sig_w: IndexMap<String, (u64, u32, u32)>,
    pub vlisten: bool,
    pub vmute: IndexSet<String>,
    pub vop: Option<bool>,
    pub vfw: (u64, u32, u64),
    pub vslow: bool,
    pub tx: f64,
    pub tz: f64,
    pub next: f64,
    pub ident: Option<Ident>,
}

#[derive(Debug, Clone)]
pub struct VoiceTalk {
    pub id: String,
    pub name: String,
    pub from: u64,
    pub to: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Room {
    pub code: String,
    pub style: String,
    pub host: Option<String>,
    pub want: i64,
    pub players: IndexMap<String, Player>,
    pub vlog: Vec<VoiceTalk>,
    pub vopus: bool,
    pub bot_chat: Option<BotChat>,
    pub bot_at: u64,
    pub full_at: u64,
    pub kar_end: u64,
    pub kar_turn: u64,
    pub chat_log: Vec<Value>,
}

impl Room {
    fn humans(&self) -> impl Iterator<Item = &Player> {
        self.players.values().filter(|p| !p.bot)
    }
    pub fn human_count(&self) -> usize {
        self.humans().count()
    }
    fn bot_list(&self) -> Vec<Bot> {
        self.players.values().filter(|p| p.bot).map(|p| Bot { id: p.id.clone(), name: p.name.clone() }).collect()
    }
    fn online_humans(&self) -> impl Iterator<Item = &Player> {
        self.players.values().filter(|p| !p.bot && p.online && p.conn.is_some())
    }
}

/// Quem está entrando (`identityOf`): id, chave antiga do aparelho, geração, época e escopo da sessão.
#[derive(Debug, Clone, Default)]
pub struct Who {
    pub id: String,
    pub fk: String,
    pub gen_: i64,
    pub epoch: i64,
    pub scope: String,
}

/// Opções lidas na subida (`server.properties` e ambiente).
#[derive(Debug, Clone)]
pub struct Settings {
    pub port: u16,
    pub bind_host: String,
    pub public_url: String,
    pub prod: bool,
    pub max_rooms: i64,
    pub max_per_room: i64,
    pub cap_file: String,
    pub perip: usize,
    pub perip_rate: usize,
    pub perip_room: usize,
    pub global_rate: usize,
    pub http_rate: usize,
    pub maxbots: i64,
    pub ticket_mode: String,
    pub ticket_rate: usize,
    pub pow_max: u32,
    pub reconnect_ms: u64,
    pub legacy_identity: bool,
    pub names_on: bool,
    pub identity_pow: u32,
    pub identity_pow_max: u32,
    pub identity_reg_day: usize,
    pub grace_ms: u64,
    pub passkeys: bool,
    pub voice_ws: bool,
    pub voice_privacy: String,
    pub status_token: String,
    pub raw_perip: usize,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Abuse {
    pub r429: u64,
    pub r403: u64,
    pub r_ban: u64,
    pub ticket: u64,
    pub pow: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct Perf {
    pub in_t: u64,
    pub out_t: u64,
    pub bytes_t: u64,
    pub bp: u64,
    pub slow: u64,
    pub tlast: f64,
}

/// Limite por conta (`lib/action-limit.js`): mesmas regras do limite por conexão, somando todas as abas.
#[derive(Debug, Default)]
pub struct ActionLimit {
    hits: HashMap<String, (u64, Vec<u64>)>,
}

impl ActionLimit {
    pub fn take(&mut self, who: &str, t: &str, now: u64) -> bool {
        let rule = if t == "gestSound" {
            Some((5000, 6, 60000))
        } else if ACCOUNT_LIMITED.contains(&t) {
            rate_rule(t)
        } else {
            None
        };
        let Some((min, max, win)) = rule else { return true };
        if who.is_empty() {
            return true;
        }
        if self.hits.len() > 20000 {
            self.hits.retain(|_, (last, _)| now.saturating_sub(*last) < 600_000);
        }
        let s = self.hits.entry(format!("{who}|{t}")).or_default();
        if min > 0 && now.saturating_sub(s.0) < min {
            return false;
        }
        s.1.retain(|x| now.saturating_sub(*x) < win);
        if s.1.len() >= max {
            return false;
        }
        s.0 = now;
        s.1.push(now);
        true
    }
}

#[derive(Debug)]
pub struct Game {
    pub cat: &'static Catalog,
    pub cfg: Config,
    pub set: Settings,
    pub conns: BTreeMap<u64, Conn>,
    pub rooms: IndexMap<String, Room>,
    pub nid: u64,
    pub next_conn: u64,
    pub limiter: Limiter,
    pub day_limiter: Limiter,
    pub conns_per_ip: HashMap<String, usize>,
    pub bm: BanManager,
    pub ticket_secret: [u8; 32],
    pub ticket_used: Used,
    pub abuse: Abuse,
    pub perf: Perf,
    pub names: NameRegistry,
    pub identity: IdentityRegistry,
    pub auth: Auth,
    pub seclog: SecLog,
    pub action_limit: ActionLimit,
    pub social: crate::social::Social,
    pub player_data: crate::playerdata::PlayerData,
    pub pubp: crate::pubprofile::PubProfiles,
    pub farm: crate::farm::Farm,
    pub cook: crate::cook::Cook,
    pub guard: crate::guard::Guard,
    pub badges: crate::badges::Badges,
    pub health: crate::health::Health,
    pub gifts: crate::gifts::Gifts,
    health_seen: HashMap<String, String>,
    mute_salt: [u8; 16],
    tick_n: u64,
    pub started: u64,
    /// Comandos de operador pedidos pelo console (`stop`).
    pub stopping: bool,
}

pub fn rnd() -> f64 {
    let b = crate::random_bytes::<8>();
    (u64::from_le_bytes(b) >> 11) as f64 / (1u64 << 53) as f64
}
fn rr(a: f64, b: f64) -> f64 {
    a + rnd() * (b - a)
}
fn new_token() -> String {
    URL_SAFE_NO_PAD.encode(crate::random_bytes::<32>())
}
pub fn token_eq(a: &str, b: &str) -> bool {
    crate::admission::safe_eq(a, b)
}

/// Código público do perfil (`pidOf` de `lib/pubprofile.js`).
pub fn pid_of(id: &str) -> String {
    use sha2::{Digest, Sha256};
    crate::hex(&Sha256::digest(format!("skyke3d-profile:{id}")))[..20].to_owned()
}

impl Game {
    pub fn new(cfg: Config, set: Settings) -> Self {
        let bm = BanManager::new(
            cfg.on("BAN", true),
            &cfg.data_file("BAN_FILE", "bans.json"),
            vec![120_000, 1_800_000, 21_600_000],
            86_400_000,
            20000,
        );
        let names = NameRegistry::new(
            &cfg.data_file("NAMES_FILE", "names.json"),
            cfg.int("NAMES_MAX", 100_000, 100, 1_000_000).unwrap_or(100_000) as usize,
            cfg.int("NAMES_PER_KEY", 20, 0, 10000).unwrap_or(20) as usize,
        );
        let id_file = cfg.data_file("IDENTITY_FILE", "identity.json");
        let identity = IdentityRegistry::new(
            &id_file,
            cfg.int("IDENTITY_MAX", 20000, 100, 1_000_000).unwrap_or(20000) as usize,
            cfg.int("IDENTITY_UNUSED_HOURS", 24, 0, 8760).unwrap_or(24) as u64 * 3_600_000,
            cfg.int("KEYPASS_INTERVAL_MS", 30 * 24 * 3_600_000, 1000, 366 * 24 * 3_600_000)
                .unwrap_or(30 * 24 * 3_600_000) as u64,
        );
        let sec_file = match cfg.raw("SECURITY_LOG") {
            None if !id_file.is_empty() => std::path::Path::new(&id_file)
                .parent()
                .unwrap_or(std::path::Path::new("."))
                .join("security.log")
                .to_string_lossy()
                .into_owned(),
            None => String::new(),
            Some(v) if v == "0" => String::new(),
            Some(v) => v,
        };
        let player_data = crate::playerdata::PlayerData::new(&cfg.data_file("PLAYER_DATA_FILE", "players.json"));
        Self {
            cat: Catalog::embedded(),
            set,
            conns: BTreeMap::new(),
            rooms: IndexMap::new(),
            nid: 1,
            next_conn: 1,
            limiter: Limiter::new(50000),
            day_limiter: Limiter::with(50000, 60_000, 25 * 3_600_000),
            conns_per_ip: HashMap::new(),
            bm,
            ticket_secret: crate::random_bytes::<32>(),
            ticket_used: Used::new(50000),
            abuse: Abuse::default(),
            perf: Perf::default(),
            names,
            identity,
            auth: Auth::new(),
            seclog: SecLog::new(&sec_file),
            action_limit: ActionLimit::default(),
            social: crate::social::Social::default(),
            player_data,
            pubp: crate::pubprofile::PubProfiles::new(&cfg.data_file("PROFILES_FILE", "profiles.json")),
            farm: crate::farm::Farm::new(&cfg.data_file("FARM_FILE", "farm.json")),
            cook: crate::cook::Cook::new(&cfg.data_file("COOK_FILE", "cook.json")),
            guard: crate::guard::Guard::default(),
            badges: crate::badges::Badges::new(&cfg.data_file("BADGES_FILE", "badges.json")),
            health: crate::health::Health::new(&cfg.data_file("HEALTH_FILE", "health.json")),
            health_seen: HashMap::new(),
            gifts: crate::gifts::Gifts::new(&cfg.data_file("GIFTS_FILE", "gifts.json")),
            mute_salt: crate::random_bytes::<16>(),
            tick_n: 0,
            started: crate::now(),
            stopping: false,
            cfg,
        }
    }

    // ---------- limites ----------

    pub fn over(&mut self, ip: &str, key: &str, max: usize, ms: u64) -> bool {
        self.limiter.over(&format!("{key}|{ip}"), max, ms, crate::now())
    }
    pub fn blocked(&self, ip: &str, key: &str, max: usize, ms: u64) -> bool {
        self.limiter.blocked(&format!("{key}|{ip}"), max, ms, crate::now())
    }
    pub fn hit_count(&mut self, ip: &str, key: &str, max: usize, ms: u64) -> crate::limits::Hit {
        self.limiter.hit(&format!("{key}|{ip}"), max, ms, crate::now())
    }
    pub fn count_hits(&self, ip: &str, key: &str, ms: u64) -> usize {
        self.limiter.count(&format!("{key}|{ip}"), ms, crate::now())
    }
    pub fn ban_check(&mut self, ip: &str) -> u64 {
        self.bm.check(ip, crate::now())
    }
    pub fn ban_strike(&mut self, ip: &str, why: &str) -> u64 {
        let ms = self.bm.strike(ip, why, crate::now());
        if ms > 0 {
            self.abuse.r_ban += 1;
        }
        ms
    }

    pub fn max_conn(&self) -> i64 {
        crate::capacity::conn_limit(self.set.max_rooms, self.set.max_per_room)
    }

    // ---------- envio ----------

    pub fn send_raw_to(&mut self, cid: u64, out: Out, len: usize) {
        let Some(c) = self.conns.get(&cid) else { return };
        let Some(link) = c.link.clone().filter(|_| c.open) else { return };
        if link.buffered.load(Ordering::Relaxed) > MAX_BUFFERED {
            self.perf.bp += 1;
            let _ = link.tx.send(Out::Kill);
            if let Some(c) = self.conns.get_mut(&cid) {
                c.open = false;
            }
            return;
        }
        link.buffered.fetch_add(len, Ordering::Relaxed);
        if link.tx.send(out).is_ok() {
            self.perf.out_t += 1;
            self.perf.bytes_t += len as u64;
        }
    }

    pub fn send(&mut self, cid: u64, m: &Value) {
        let s: Arc<str> = Arc::from(m.to_string());
        let n = s.len();
        self.send_raw_to(cid, Out::Text(s), n);
    }

    /// Para todos os humanos prontos da sala, menos `ex`.
    pub fn bc(&mut self, code: &str, m: &Value, ex: Option<u64>) {
        let Some(r) = self.rooms.get(code) else { return };
        let to: Vec<u64> = r
            .players
            .values()
            .filter(|p| !p.bot && p.ready && p.online)
            .filter_map(|p| p.conn)
            .filter(|c| Some(*c) != ex)
            .collect();
        if to.is_empty() {
            return;
        }
        let s: Arc<str> = Arc::from(m.to_string());
        for c in to {
            self.send_raw_to(c, Out::Text(s.clone()), s.len());
        }
    }

    fn bc_bin(&mut self, code: &str, buf: Vec<u8>) {
        let Some(r) = self.rooms.get(code) else { return };
        let to: Vec<u64> =
            r.players.values().filter(|p| !p.bot && p.ready && p.online).filter_map(|p| p.conn).collect();
        let b: Arc<[u8]> = Arc::from(buf);
        for c in to {
            self.send_raw_to(c, Out::Bin(b.clone()), b.len());
        }
    }

    pub fn safe_close(&mut self, cid: u64, code: u16, reason: &str) {
        let Some(c) = self.conns.get_mut(&cid) else { return };
        if !c.open {
            return;
        }
        c.closing = Some(code);
        c.open = false;
        if let Some(l) = &c.link {
            let _ = l.tx.send(Out::Close(code, reason.into()));
        }
    }

    fn resume_fail(&mut self, cid: u64, msg: &str, reason: &str, extra: Option<Value>) {
        let mut m = json!({"t": "err", "scope": "resume", "msg": msg});
        if let Some(Value::Object(e)) = extra {
            for (k, v) in e {
                m[k] = v;
            }
        }
        self.send(cid, &m);
        self.safe_close(cid, 1000, reason);
    }

    // ---------- jogadores ----------

    fn health_pub(&self, p: &Player) -> Value {
        match p.ident.as_ref().filter(|i| !i.id.is_empty()) {
            Some(i) => self.health.public_of(&pid_of(&i.id)),
            None => json!({"ill": "", "tired": false, "until": 0}),
        }
    }

    pub fn pub_of(&self, p: &Player) -> Value {
        json!({
            "id": p.id, "name": p.name, "cfg": p.cfg, "x": p.x, "z": p.z, "ry": p.ry, "y": p.y, "s": p.s, "bot": p.bot,
            "item": p.item, "talk": p.talk, "vl": p.vlisten, "mk": p.mk, "fid": p.fid, "health": self.health_pub(p), "pp": p.pp
        })
    }

    fn mute_key(&self, code: &str, ip: &str) -> String {
        use hmac::{Hmac, Mac};
        let Ok(mut m) = Hmac::<sha2::Sha256>::new_from_slice(&self.mute_salt) else { return String::new() };
        m.update(format!("{code}|{ip}").as_bytes());
        URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())[..12].to_owned()
    }

    fn total_bots(&self) -> i64 {
        self.rooms.values().map(|r| r.players.values().filter(|p| p.bot).count() as i64).sum()
    }

    fn new_bot(&mut self, code: &str) -> Option<Player> {
        let used: HashSet<String> = self.rooms.get(code)?.players.values().map(|p| p.name.clone()).collect();
        let id = format!("b{}", self.nid);
        self.nid += 1;
        let cr: HashMap<&str, i64> = self.cat.cr.iter().map(|(k, n)| (k.as_str(), *n)).collect();
        let r = |k: &str| (rnd() * cr.get(k).copied().unwrap_or(1) as f64) as i64;
        let one_in_3 = || if (rnd() * 3.0) as i64 != 0 { 0 } else { 1 };
        let hat = if (rnd() * 3.0) as i64 != 0 {
            0
        } else {
            1 + (rnd() * (cr.get("hat").copied().unwrap_or(1) - 1) as f64) as i64
        };
        let raw = json!({
            "skin": r("skin"), "eye": r("eye"), "shirt": r("shirt"), "hair": r("hair"), "hc": r("hc"), "hs": r("hs"), "fc": r("fc"),
            "hat": hat, "bt": r("bt"), "pc": r("pc"), "sc": r("sc"), "shoes": r("shoes"), "eb": one_in_3(), "sb": one_in_3(),
            "pb": one_in_3(), "cb": one_in_3(), "hb": one_in_3()
        });
        let cfg = V::clean_cfg(&raw, self.cat).unwrap_or(Value::Null);
        let name = BOT_NAMES
            .iter()
            .find(|n| !used.contains(**n))
            .map_or_else(|| format!("Bot{}", self.nid), |n| (*n).to_owned());
        let (x, z) = (rr(-6.0, 6.0), rr(-2.0, 4.0));
        let b = Player {
            id: id.clone(),
            bot: true,
            name,
            cfg,
            x,
            z,
            tx: x,
            tz: z,
            item: Value::Null,
            ..Default::default()
        };
        self.rooms.get_mut(code)?.players.insert(id, b.clone());
        Some(b)
    }

    /// Ajusta os bots da sala ao pedido do anfitrião, ao espaço livre e ao teto do servidor.
    fn fit(&mut self, code: &str) {
        let Some(r) = self.rooms.get(code) else { return };
        let h = r.human_count() as i64;
        let bots: Vec<String> = r.players.values().filter(|p| p.bot).map(|p| p.id.clone()).collect();
        let target = if self.set.maxbots > 0 {
            0.max(r.want.min(self.set.max_per_room - h).min(self.set.maxbots - self.total_bots() + bots.len() as i64))
        } else {
            0
        };
        let mut n = bots.len() as i64;
        let mut bots = bots;
        while n > target {
            let Some(b) = bots.pop() else { break };
            if let Some(r) = self.rooms.get_mut(code) {
                r.players.shift_remove(&b);
            }
            self.bc(code, &json!({"t": "pl", "id": b}), None);
            n -= 1;
        }
        while n < target {
            let Some(b) = self.new_bot(code) else { break };
            let p = self.pub_of(&b);
            self.bc(code, &json!({"t": "pj", "p": p}), None);
            n += 1;
        }
    }

    fn key_humans(&self, code: &str, key: &str) -> usize {
        self.rooms.get(code).map_or(0, |r| r.humans().filter(|p| p.ip == key).count())
    }

    fn own_oids(&self, who: &Who) -> HashSet<String> {
        let mut s = HashSet::from([self.names.owner_id(&who.id)]);
        if !who.fk.is_empty() {
            s.insert(self.names.owner(&who.fk));
        }
        if who.gen_ == 0 && who.fk.is_empty() {
            s.insert(self.names.owner(""));
        }
        s
    }

    fn name_owner(&self, who: &Who) -> String {
        if who.gen_ > 0 { self.names.owner_id(&who.id) } else { self.names.owner(&who.fk) }
    }

    fn names_in_use(&self, mine: &HashSet<String>) -> HashSet<String> {
        let mut s = HashSet::new();
        for r in self.rooms.values() {
            for p in r.humans() {
                if p.online || !mine.contains(&p.oid) {
                    s.insert(V::name_key(&p.name));
                }
            }
        }
        s
    }

    fn name_own_online(&self, mine: &HashSet<String>, key: &str) -> bool {
        self.rooms
            .values()
            .any(|r| r.humans().any(|p| p.online && mine.contains(&p.oid) && V::name_key(&p.name) == key))
    }

    fn name_busy(&mut self, cid: u64, name: &str, who: &Who) -> bool {
        let key = V::name_key(name);
        let mine = self.own_oids(who);
        let used = self.names_in_use(&mine);
        if !used.contains(&key) && self.names.free_for(&key, &who.id, &who.fk) {
            return false;
        }
        let mut sug = String::new();
        for i in 2..100 {
            let s = i.to_string();
            let n = V::slice16(name, self.cat.max_name - s.len()) + &s;
            let nk = V::name_key(&n);
            if !used.contains(&nk)
                && self.names.free_for(&nk, &who.id, &who.fk)
                && crate::profanity::bad_name(&n, true).is_none()
            {
                sug = n;
                break;
            }
        }
        let own = self.set.names_on && self.name_own_online(&mine, &key);
        let msg = if own {
            "Este nome está online em outra janela."
        } else {
            "Este nome já está em uso por outra pessoa."
        };
        self.send(cid, &json!({"t": "err", "field": "name", "sug": sug, "msg": msg}));
        true
    }

    fn uniq_name(&self, code: &str, name: &str, who: &Who) -> String {
        let used: HashSet<String> = self
            .rooms
            .get(code)
            .map(|r| r.players.values().map(|p| V::name_key(&p.name)).collect())
            .unwrap_or_default();
        let mut n = name.to_owned();
        let mut i = 2;
        while (used.contains(&V::name_key(&n)) || !self.names.free_for(&V::name_key(&n), &who.id, &who.fk)) && i < 100 {
            let s = format!(" {i}");
            n = V::slice16(&n, (self.cat.max_name as isize - V::len16(&s) as isize).max(1) as usize) + &s;
            i += 1;
        }
        n
    }

    fn host_transfer(&mut self, code: &str) {
        let Some(r) = self.rooms.get(code) else { return };
        let cur_ok =
            r.host.as_ref().and_then(|h| r.players.get(h)).is_some_and(|p| !p.bot && p.online && p.conn.is_some());
        if cur_ok {
            return;
        }
        let first = r.online_humans().next().map(|p| p.id.clone());
        let host = r.host.clone();
        match first {
            Some(h) => {
                if host.as_deref() != Some(&h) {
                    if let Some(r) = self.rooms.get_mut(code) {
                        r.host = Some(h.clone());
                    }
                    self.bc(code, &json!({"t": "host", "id": h}), None);
                }
            }
            None if host.is_some() => {
                if let Some(r) = self.rooms.get_mut(code) {
                    r.host = None;
                }
                self.bc(code, &json!({"t": "host", "id": null}), None);
            }
            None => {}
        }
    }

    fn destroy_room(&mut self, code: &str) {
        self.rooms.shift_remove(code);
    }

    fn talk_log(&mut self, code: &str, pid: &str, on: bool) {
        let now = crate::now();
        let Some(r) = self.rooms.get_mut(code) else { return };
        let Some(p) = r.players.get_mut(pid) else { return };
        if on {
            p.talk_at = now;
            return;
        }
        if p.talk_at == 0 {
            return;
        }
        let e = VoiceTalk { id: p.id.clone(), name: p.name.clone(), from: p.talk_at, to: now };
        p.talk_at = 0;
        r.vlog.push(e);
        while r.vlog.len() > 60 || r.vlog.first().is_some_and(|x| now - x.to > 600_000) {
            r.vlog.remove(0);
        }
    }

    fn vcodec_sync(&mut self, code: &str) {
        let Some(r) = self.rooms.get(code) else { return };
        let o = !r.players.values().any(|p| !p.bot && p.online && p.vop == Some(false));
        if o == r.vopus {
            return;
        }
        if let Some(r) = self.rooms.get_mut(code) {
            r.vopus = o;
        }
        self.bc(code, &json!({"t": "vcodec", "opus": o}), None);
    }

    fn stop_speaking(&mut self, code: &str, pid: &str) {
        let talking = self.rooms.get(code).and_then(|r| r.players.get(pid)).is_some_and(|p| p.talk);
        if talking {
            if let Some(p) = self.rooms.get_mut(code).and_then(|r| r.players.get_mut(pid)) {
                p.talk = false;
            }
            self.talk_log(code, pid, false);
            self.bc(code, &json!({"t": "speak", "id": pid, "v": false}), None);
        }
    }

    pub fn drop_player(&mut self, code: &str, pid: &str) {
        let Some(r) = self.rooms.get_mut(code) else { return };
        let Some(p) = r.players.get_mut(pid) else { return };
        p.retire = None;
        p.online = false;
        p.ready = false;
        p.conn = None;
        self.stop_speaking(code, pid);
        let Some(r) = self.rooms.get_mut(code) else { return };
        r.players.shift_remove(pid);
        let was_host = r.host.as_deref() == Some(pid);
        if was_host {
            r.host = None;
            self.host_transfer(code);
        }
        self.bc(code, &json!({"t": "pl", "id": pid}), None);
        self.vcodec_sync(code);
        self.fit(code);
        if self.rooms.get(code).is_some_and(|r| r.human_count() == 0) {
            self.destroy_room(code);
        }
    }

    fn mark_offline(&mut self, code: &str, pid: &str) {
        let retire = crate::now() + self.set.reconnect_ms;
        let Some(p) = self.rooms.get_mut(code).and_then(|r| r.players.get_mut(pid)) else { return };
        p.online = false;
        p.ready = false;
        p.conn = None;
        let was_listening = std::mem::take(&mut p.vlisten);
        p.retire = Some(retire);
        if was_listening {
            self.bc(code, &json!({"t": "vl", "id": pid, "v": false}), None);
        }
        self.stop_speaking(code, pid);
        if self.rooms.get(code).is_some_and(|r| r.host.as_deref() == Some(pid)) {
            self.host_transfer(code);
        }
    }

    fn ice_cfg(&self) -> Value {
        if self.set.voice_ws {
            return json!({"iceServers": [], "voice": true, "ws": true});
        }
        if self.set.voice_privacy == "relay" {
            json!({"iceServers": [], "policy": "relay", "voice": false, "reason": "turn"})
        } else {
            json!({"iceServers": [{"urls": "stun:stun.l.google.com:19302"}], "voice": true})
        }
    }

    fn join_payload(&self, code: &str, pid: &str, resume: bool) -> Value {
        let Some(r) = self.rooms.get(code) else { return Value::Null };
        let Some(me) = r.players.get(pid) else { return Value::Null };
        let mut o = json!({
            "t": "joined", "ver": VERSION, "url": self.set.public_url, "ice": self.ice_cfg(), "vopus": r.vopus, "id": me.id,
            "code": r.code, "style": r.style, "host": r.host, "want": r.want, "max": self.set.max_per_room,
            "players": r.players.values().map(|p| self.pub_of(p)).collect::<Vec<_>>(), "kar": [],
            "kl": r.kar_end.saturating_sub(crate::now()), "turn": r.kar_turn, "token": me.token, "rk": me.rk,
            "self": self.pub_of(me), "reconnect": self.set.reconnect_ms
        });
        if resume {
            o["resume"] = json!(1);
        }
        o
    }

    #[allow(clippy::too_many_arguments)]
    fn add_human(&mut self, code: &str, cid: u64, ip: &str, name: &str, cfg: Value, who: &Who) -> String {
        let oid = self.name_owner(who);
        let id = format!("p{}", self.nid);
        self.nid += 1;
        let p = Player {
            id: id.clone(),
            name: self.uniq_name(code, name, who),
            cfg,
            x: 0.0,
            z: 5.0,
            tx: 0.0,
            tz: 5.0,
            item: Value::Null,
            ip: ip.into(),
            oid,
            mk: self.mute_key(code, ip),
            conn: Some(cid),
            online: true,
            token: new_token(),
            rk: new_token(),
            ident: Some(Ident { id: who.id.clone(), gen_: who.gen_, epoch: who.epoch }),
            ..Default::default()
        };
        let key = V::name_key(&p.name);
        if let Some(r) = self.rooms.get_mut(code) {
            r.players.insert(id.clone(), p);
        }
        self.names.claim_for(&key, &who.id, &who.fk);
        if let Some(c) = self.conns.get_mut(&cid) {
            c.ident_id = who.id.clone();
            c.ident_gen = who.gen_;
            c.ident_epoch = who.epoch;
        }
        if self.rooms.get(code).is_some_and(|r| r.host.is_none()) {
            if let Some(r) = self.rooms.get_mut(code) {
                r.host = Some(id.clone());
            }
            self.bc(code, &json!({"t": "host", "id": id}), None);
        }
        self.fit(code);
        let jp = self.join_payload(code, &id, false);
        self.send(cid, &jp);
        let pj = self.rooms.get(code).and_then(|r| r.players.get(&id)).map(|p| self.pub_of(p)).unwrap_or(Value::Null);
        self.bc(code, &json!({"t": "pj", "p": pj}), None);
        if let Some(p) = self.rooms.get_mut(code).and_then(|r| r.players.get_mut(&id)) {
            p.ready = true;
        }
        id
    }

    fn resume_session(&mut self, code: &str, cid: u64, ip: &str, token: &str) -> Option<String> {
        let found = self
            .rooms
            .get(code)?
            .players
            .values()
            .find(|q| !q.bot && !q.token.is_empty() && token_eq(&q.token, token))
            .cloned();
        let Some(p) = found else {
            self.resume_fail(cid, RESUME_GONE, "resume", None);
            return None;
        };
        if p.online || p.conn.is_some() {
            self.resume_fail(
                cid,
                "Sua sessão ainda está ativa. Tentando novamente…",
                "busy",
                Some(json!({"retry": 1})),
            );
            return None;
        }
        if let Some(i) = p.ident.as_ref().filter(|i| !i.id.is_empty()) {
            let rec = self.identity.get(&i.id);
            if i.gen_ > 0 {
                if rec.is_none_or(|r| r.gen_ != i.gen_ || r.epoch != i.epoch) {
                    if let Some(x) = self.rooms.get_mut(code).and_then(|r| r.players.get_mut(&p.id)) {
                        x.retire = x.retire.or(Some(crate::now()));
                    }
                    self.resume_fail(cid, "Sua identidade mudou. Entre novamente.", "identity", None);
                    return None;
                }
            } else if rec.is_some() {
                self.resume_fail(cid, "Sua identidade foi registrada. Entre novamente.", "identity", None);
                return None;
            }
        }
        let pid = p.id.clone();
        let x = self.rooms.get_mut(code)?.players.get_mut(&pid)?;
        x.retire = None;
        x.conn = Some(cid);
        x.online = true;
        x.ready = false;
        x.vslow = false;
        x.ip = ip.into();
        x.token = new_token();
        let (key, oid) = (V::name_key(&x.name), x.oid.clone());
        self.names.claim(&key, &oid);
        if let (Some(i), Some(c)) = (p.ident.as_ref().filter(|i| !i.id.is_empty()), self.conns.get_mut(&cid)) {
            c.ident_id = i.id.clone();
            c.ident_gen = i.gen_;
            c.ident_epoch = i.epoch;
        }
        if self.rooms.get(code).is_some_and(|r| r.host.is_none()) {
            if let Some(r) = self.rooms.get_mut(code) {
                r.host = Some(pid.clone());
            }
            self.bc(code, &json!({"t": "host", "id": pid}), None);
        }
        let jp = self.join_payload(code, &pid, true);
        self.send(cid, &jp);
        if let Some(x) = self.rooms.get_mut(code).and_then(|r| r.players.get_mut(&pid)) {
            x.ready = true;
        }
        Some(pid)
    }

    fn admit_new(&mut self, ip: &str, cid: u64) -> bool {
        let pr = self.set.perip_rate;
        let a = self.hit_count(ip, "admr", pr, 600_000);
        if !a.over {
            return true;
        }
        if a.count > pr * 2 && self.hit_count(ip, "admx", 6, 60000).count == 7 {
            self.ban_strike(ip, "admissao");
        }
        self.abuse.r429 += 1;
        self.send(cid, &json!({"t": "err", "msg": "Muitas entradas deste local. Aguarde alguns minutos."}));
        false
    }

    fn room_code() -> String {
        let a: Vec<char> = V::CODE_ALPHABET.chars().collect();
        crate::random_bytes::<10>().iter().map(|x| a[*x as usize % a.len()]).collect()
    }

    pub fn set_ident(&mut self, cid: u64, id: &str, gen_: i64, epoch: i64, scope: &str) {
        let Some(c) = self.conns.get_mut(&cid) else { return };
        c.ident_id = id.into();
        c.ident_gen = gen_;
        c.ident_epoch = epoch;
        c.ident_scope = scope.into();
        if let (Some(code), Some(me)) = (c.room.clone(), c.me.clone())
            && let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me))
        {
            p.ident = Some(Ident { id: id.into(), gen_, epoch });
        }
    }

    fn account_enter(&mut self, cid: u64) {
        let Some(c) = self.conns.get(&cid) else { return };
        if c.ident_scope != "primary" || c.ident_id.is_empty() {
            return;
        }
        let (Some(code), Some(me)) = (c.room.clone(), c.me.clone()) else { return };
        let id = c.ident_id.clone();
        let Some((name, cfg)) = self.me_info(cid) else { return };
        self.player_data.update(&id, json!({"name": name, "cfg": cfg}));
        let pid = pid_of(&id);
        let fields =
            self.pubp.fields_of(&pid).unwrap_or_else(|| json!({"bio": "", "like": "", "mood": "", "world": ""}));
        if self.pubp.publish(&pid, cid, &fields, Some((name, cfg))) {
            if let Some(c) = self.conns.get_mut(&cid) {
                c.pp = pid.clone();
            }
            if let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) {
                p.pp = pid.clone();
            }
            self.send(cid, &json!({"t": "pubp", "on": 1, "pp": pid, "f": fields}));
            self.bc(&code, &json!({"t": "ppid", "id": me, "pp": pid}), Some(cid));
        }
    }

    /// Nome e visual atuais de quem está numa sala por esta conexão.
    pub fn me_info(&self, cid: u64) -> Option<(String, Value)> {
        let c = self.conns.get(&cid)?;
        let p = self.rooms.get(c.room.as_ref()?)?.players.get(c.me.as_ref()?)?;
        Some((p.name.clone(), p.cfg.clone()))
    }

    /// O perfil público como a página vê (`PUB.get`).
    pub fn pub_get(&self, pid: &str) -> Option<Value> {
        let me = |c: u64| self.me_info(c);
        self.pubp.get(pid, &me)
    }

    fn lobby(&mut self, cid: u64, m: &Value) {
        let ip = self.conns.get(&cid).map(|c| c.ip.clone()).unwrap_or_default();
        if self.ban_check(&ip) > 0 {
            return self.send(cid, &json!({"t": "err", "msg": "Muitas tentativas. Aguarde alguns minutos."}));
        }
        let t = m["t"].as_str().unwrap_or("");
        let name = m["name"].as_str().unwrap_or("").to_owned();
        match t {
            "create" => {
                if self.over(&ip, "cr", 5, 60000) {
                    return self.send(cid, &json!({"t": "err", "msg": "Muitas salas criadas. Aguarde um minuto."}));
                }
                let Some(who) = self.identity_of(cid, m) else { return };
                if self.name_busy(cid, &name, &who) || !self.admit_new(&ip, cid) {
                    return;
                }
                if self.rooms.len() as i64 >= self.set.max_rooms {
                    let mr = self.set.max_rooms;
                    return self
                        .send(cid, &json!({"t": "err", "msg": format!("Servidor cheio (limite de {mr} salas).")}));
                }
                let mut code = Self::room_code();
                while self.rooms.contains_key(&code) {
                    code = Self::room_code();
                }
                let room = Room {
                    code: code.clone(),
                    style: m["style"].as_str().unwrap_or("amazora").into(),
                    want: m["bots"].as_i64().unwrap_or(0),
                    vopus: true,
                    ..Default::default()
                };
                self.rooms.insert(code.clone(), room);
                self.enter(cid, &code, &ip, &name, m, &who);
            }
            "join" => {
                if self.over(&ip, "jn", 15, 60000) {
                    return self.send(cid, &json!({"t": "err", "msg": "Muitas tentativas. Aguarde um minuto."}));
                }
                if self.blocked(&ip, "jf", 8, 600_000) {
                    return self
                        .send(cid, &json!({"t": "err", "msg": "Muitas tentativas incorretas. Aguarde 10 minutos."}));
                }
                let code = m["code"].as_str().unwrap_or("").to_owned();
                if !self.rooms.contains_key(&code) {
                    if self.hit_count(&ip, "jf", 999, 600_000).count == 12 {
                        self.ban_strike(&ip, "codigos");
                    }
                    return self.send(cid, &json!({"t": "err", "msg": "Sala não encontrada. Confira o código."}));
                }
                let Some(who) = self.identity_of(cid, m) else { return };
                if self.name_busy(cid, &name, &who) || !self.admit_new(&ip, cid) {
                    return;
                }
                let max = self.set.max_per_room;
                if self.rooms.get(&code).is_some_and(|r| r.human_count() as i64 >= max) {
                    return self
                        .send(cid, &json!({"t": "err", "msg": format!("Sala lotada (limite de {max} pessoas).")}));
                }
                if self.key_humans(&code, &ip) >= self.set.perip_room {
                    return self.send(cid, &json!({"t": "err", "msg": "Limite de acessos do mesmo local nesta sala."}));
                }
                self.enter(cid, &code, &ip, &name, m, &who);
            }
            "resume" => {
                if self.over(&ip, "rs", 60, 60000) {
                    return self.resume_fail(cid, "Muitas tentativas de reconexão. Aguarde um minuto.", "resume", None);
                }
                let code = m["code"].as_str().unwrap_or("").to_owned();
                if !self.rooms.contains_key(&code) {
                    return self.resume_fail(cid, RESUME_GONE, "resume", None);
                }
                let token = m["token"].as_str().unwrap_or("");
                if let Some(pid) = self.resume_session(&code, cid, &ip, token) {
                    let ident = self.rooms.get(&code).and_then(|r| r.players.get(&pid)).and_then(|p| p.ident.clone());
                    if let Some(c) = self.conns.get_mut(&cid) {
                        c.room = Some(code.clone());
                        c.me = Some(pid);
                        c.since = crate::now();
                    }
                    if let Some(i) = ident.filter(|i| !i.id.is_empty()) {
                        self.set_ident(cid, &i.id, i.gen_, i.epoch, "");
                    }
                    self.account_enter(cid);
                }
            }
            _ => {}
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn enter(&mut self, cid: u64, code: &str, ip: &str, name: &str, m: &Value, who: &Who) {
        if let Some(c) = self.conns.get_mut(&cid) {
            c.room = Some(code.into());
        }
        let pid = self.add_human(code, cid, ip, name, m["cfg"].clone(), who);
        if let Some(c) = self.conns.get_mut(&cid) {
            c.me = Some(pid);
            c.since = crate::now();
        }
        self.set_ident(cid, &who.id, who.gen_, who.epoch, &who.scope);
        self.account_enter(cid);
    }

    // ---------- conexões ----------

    /// Conexão nova (o handshake já passou por `verify_client`).
    pub fn open(&mut self, ip: &str, origin: &str, link: Link) -> u64 {
        let id = self.next_conn;
        self.next_conn += 1;
        *self.conns_per_ip.entry(ip.into()).or_default() += 1;
        let now = crate::now();
        self.conns.insert(
            id,
            Conn {
                id,
                link: Some(link),
                open: true,
                ip: ip.into(),
                page_origin: origin.into(),
                alive: true,
                jt: Some(now + 15000),
                jt_at: now,
                ..Default::default()
            },
        );
        id
    }

    fn rate(&mut self, cid: u64, t: &str) -> bool {
        let Some((min, max, win)) = rate_rule(t) else { return true };
        let now = crate::now();
        let Some(c) = self.conns.get_mut(&cid) else { return false };
        let s = c.rl.entry(t.into()).or_insert((0, Vec::new()));
        if min > 0 && now.saturating_sub(s.0) < min {
            return false;
        }
        s.1.retain(|x| now.saturating_sub(*x) < win);
        if s.1.len() >= max {
            return false;
        }
        s.0 = now;
        s.1.push(now);
        true
    }

    fn wait_for(&self, cid: u64, t: &str) -> u64 {
        let (Some((min, max, win)), Some(s)) = (rate_rule(t), self.conns.get(&cid).and_then(|c| c.rl.get(t))) else {
            return 0;
        };
        let now = crate::now();
        let mut w = if min > 0 { (s.0 + min) as i64 - now as i64 } else { 0 };
        let h: Vec<u64> = s.1.iter().copied().filter(|x| now.saturating_sub(*x) < win).collect();
        if h.len() >= max {
            w = w.max((h[h.len() - max] + win) as i64 - now as i64);
        }
        w.max(0) as u64
    }

    fn trail(&mut self, cid: u64, m: Value) {
        let t = m["t"].as_str().unwrap_or("").to_owned();
        let due = crate::now() + self.wait_for(cid, &t) + 30;
        let Some(c) = self.conns.get_mut(&cid) else { return };
        let d = c.defer.entry(t).or_insert((None, None));
        d.0 = Some(m);
        if d.1.is_none() {
            d.1 = Some(due);
        }
    }

    fn violation(&mut self, cid: u64) -> bool {
        let now = crate::now();
        let Some(c) = self.conns.get_mut(&cid) else { return false };
        c.bad_hits.retain(|t| now - t < 10000);
        c.bad_hits.push(now);
        if c.bad_hits.len() < 8 {
            return false;
        }
        let first = !c.punished;
        c.punished = true;
        let ip = c.ip.clone();
        if first {
            self.ban_strike(&ip, "violacao");
        }
        self.send(cid, &json!({"t": "sys", "text": "Você foi desconectado por enviar ações demais em pouco tempo."}));
        self.safe_close(cid, 1008, "rate limit");
        true
    }

    fn close_policy(&mut self, cid: u64, text: &str, why: &str) {
        if let Some(c) = self.conns.get_mut(&cid)
            && !why.is_empty()
            && !c.punished
        {
            c.punished = true;
            let ip = c.ip.clone();
            self.ban_strike(&ip, why);
        }
        self.send(cid, &json!({"t": "sys", "text": text}));
        self.safe_close(cid, 1008, "policy");
    }

    fn bump_join(&mut self, cid: u64) {
        let now = crate::now();
        if let Some(c) = self.conns.get_mut(&cid)
            && c.me.is_none()
            && now - c.jt_at <= 300_000
        {
            c.jt = Some(now + 120_000);
        }
    }

    /// Mensagem de texto de uma conexão.
    pub fn on_text(&mut self, cid: u64, raw: &str) {
        self.perf.in_t += 1;
        let sec = crate::now() / 1000;
        let Some(c) = self.conns.get_mut(&cid) else { return };
        if sec != c.win_sec {
            c.win_sec = sec;
            c.n = 0;
            c.sig_n = 0;
            c.bin_n = 0;
        }
        let msg: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(_) => {
                c.n += 1;
                if c.n > MSG_PER_SEC {
                    self.close_policy(cid, "Muitas mensagens. A conexão foi encerrada.", "flood");
                } else {
                    self.violation(cid);
                }
                return;
            }
        };
        if msg["t"] == "sig" {
            c.sig_n += 1;
            if c.sig_n > SIG_PER_SEC {
                return;
            }
        } else {
            c.n += 1;
            if c.n > MSG_PER_SEC {
                self.close_policy(cid, "Muitas mensagens. A conexão foi encerrada.", "flood");
                return;
            }
        }
        let in_room = c.me.is_some();
        let Some(m) = normalize_message(&msg, self.cat) else {
            self.violation(cid);
            if !in_room && msg["t"] == "join" {
                self.send(cid, &json!({"t": "err", "msg": "Não foi possível entrar: confira o código da sala e os dados do avatar."}));
            } else if !in_room && msg["t"] == "resume" {
                self.resume_fail(cid, RESUME_GONE, "resume", None);
            }
            return;
        };
        if !in_room && (m["t"] == "create" || m["t"] == "join") {
            if m["named"] != true {
                return self.send(cid, &json!({"t": "err", "msg": "Escolha um nome para entrar."}));
            }
            if crate::profanity::bad_name(m["name"].as_str().unwrap_or(""), true).is_some() {
                return self.send(cid, &json!({"t": "err", "msg": "Esse nome não é permitido. Escolha outro."}));
            }
        }
        self.handle(cid, m);
    }

    /// Mensagem binária (voz).
    pub fn on_binary(&mut self, cid: u64, raw: &[u8]) {
        self.perf.in_t += 1;
        let sec = crate::now() / 1000;
        let Some(c) = self.conns.get_mut(&cid) else { return };
        if sec != c.win_sec {
            c.win_sec = sec;
            c.n = 0;
            c.sig_n = 0;
            c.bin_n = 0;
        }
        c.bin_n += 1;
        if c.bin_n > BIN_PER_SEC {
            self.close_policy(cid, "Muitas mensagens. A conexão foi encerrada.", "flood");
            return;
        }
        if c.me.is_some() && !self.voice_in(cid, raw) {
            self.violation(cid);
        }
    }

    /// Voz pelo WebSocket: por enquanto só confere o formato (o repasse vem com `voicews`).
    fn voice_in(&mut self, _cid: u64, raw: &[u8]) -> bool {
        !raw.is_empty()
    }

    fn handle(&mut self, cid: u64, m: Value) {
        let t = m["t"].as_str().unwrap_or("").to_owned();
        let socket_limited = !self.rate(cid, &t);
        let (who, in_room) = self.conns.get(&cid).map_or((String::new(), false), |c| {
            (if c.ident_id.is_empty() { c.fid.clone() } else { c.ident_id.clone() }, c.me.is_some())
        });
        let account_limited = !socket_limited && in_room && !self.action_limit.take(&who, &t, crate::now());
        if socket_limited || account_limited {
            let rid = m["rid"].clone();
            match t.as_str() {
                "sig" => return,
                _ if TRAIL.contains(&t.as_str()) && !(t == "item" && m["use"] == 1) => return self.trail(cid, m),
                "report" => {
                    return self.send(
                        cid,
                        &json!({"t": "rep", "err": "Aguarde alguns segundos antes de enviar outra denúncia."}),
                    );
                }
                "farmdo" => {
                    return self
                        .send(cid, &json!({"t": "farmr", "rid": rid, "k": m["k"], "ok": false, "reason": "calma"}));
                }
                "giftsend" => {
                    return self
                        .send(cid, &json!({"t": "giftr", "rid": rid, "to": m["to"], "ok": false, "reason": "calma"}));
                }
                "cookdo" => return self.send(cid, &json!({"t": "cookr", "rid": rid, "ok": false, "reason": "calma"})),
                "cookrenew" => {
                    return self
                        .send(cid, &json!({"t": "cookr", "rid": rid, "renew": 1, "ok": false, "reason": "calma"}));
                }
                "medcraft" | "medheal" => {
                    return self.send(cid, &json!({"t": "medr", "rid": rid, "ok": false, "reason": "calma"}));
                }
                "supply" => {
                    return self.send(cid, &json!({"t": "supplyr", "rid": rid, "ok": false, "reason": "calma"}));
                }
                "finv" => return self.send(cid, &json!({"t": "finvr", "id": m["to"], "ok": false, "reason": "wait"})),
                "giftget" | "giftsee" | "giftdel" => return,
                _ if SOFT.contains(&t.as_str()) => return,
                _ => {}
            }
            let now = crate::now();
            if (t == "chat" || t == "gchat")
                && let Some(c) = self.conns.get_mut(&cid)
                && now - c.last_warn > 5000
            {
                c.last_warn = now;
                self.send(cid, &json!({"t": "sys", "text": "⏳ Calma! Aguarde um instante entre mensagens."}));
            }
            if account_limited {
                return;
            }
            self.violation(cid);
            return;
        }
        if !(t == "item" && m["use"] == 1)
            && let Some(d) = self.conns.get_mut(&cid).and_then(|c| c.defer.get_mut(&t))
        {
            d.0 = None;
        }
        if self.identity_msg(cid, &m) {
            if !in_room {
                self.bump_join(cid);
            }
            return;
        }
        if t == "accountget" {
            return self.account_get(cid, &m);
        }
        if !in_room {
            self.lobby(cid, &m);
            if let Some(c) = self.conns.get_mut(&cid)
                && c.me.is_some()
            {
                c.jt = None;
            }
            return;
        }
        self.room_msg(cid, &t, &m);
    }

    fn account_get(&mut self, cid: u64, m: &Value) {
        if let Some(tk) = m["token"].as_str() {
            let v = self.auth.verify_token(&mut self.identity, tk, Some("join"));
            let ok = self.conns.get(&cid).is_some_and(|c| {
                v.as_ref().is_some_and(|v| v.scope == "primary" && (c.me.is_none() || c.ident_id == v.id))
            });
            let Some(v) = v.filter(|_| ok) else { return self.send(cid, &json!({"t": "account", "err": "session"})) };
            self.set_ident(cid, &v.id, v.gen_, v.epoch, &v.scope);
        }
        let Some(c) = self.conns.get(&cid) else { return };
        if c.ident_scope != "primary" || c.ident_id.is_empty() {
            return self.send(cid, &json!({"t": "account", "err": "session"}));
        }
        let id = c.ident_id.clone();
        let pid = pid_of(&id);
        let data = self
            .player_data
            .get(&id)
            .or_else(|| self.pub_get(&pid).map(|pr| json!({"name": pr["name"], "cfg": pr["cfg"], "friends": null})));
        let profile = self.pubp.fields_of(&pid);
        self.send(cid, &json!({"t": "account", "data": data, "profile": profile}));
    }

    /// Mensagens de quem já está numa sala.
    fn room_msg(&mut self, cid: u64, t: &str, m: &Value) {
        let Some(c) = self.conns.get(&cid) else { return };
        let (Some(code), Some(me)) = (c.room.clone(), c.me.clone()) else { return };
        if !self.rooms.get(&code).is_some_and(|r| r.players.contains_key(&me)) {
            return;
        }
        match t {
            "leave" => {
                self.soc_leave(cid);
                if let Some(c) = self.conns.get_mut(&cid) {
                    c.me = None;
                    c.room = None;
                }
                self.drop_player(&code, &me);
                self.safe_close(cid, 4000, "leave");
            }
            "move" => {
                if let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) {
                    p.x = m["x"].as_f64().unwrap_or(p.x);
                    p.z = m["z"].as_f64().unwrap_or(p.z);
                    p.ry = m["ry"].as_f64().unwrap_or(0.0);
                    p.y = m["y"].as_f64().unwrap_or(0.0);
                    p.s = m["s"].as_i64().unwrap_or(0);
                }
            }
            "cfg" => {
                let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) else { return };
                p.cfg = m["cfg"].clone();
                let name = p.name.clone();
                let (scope, id) =
                    self.conns.get(&cid).map(|c| (c.ident_scope.clone(), c.ident_id.clone())).unwrap_or_default();
                if scope == "primary" {
                    self.player_data.update(&id, json!({"name": name.clone(), "cfg": m["cfg"]}));
                    let pid = pid_of(&id);
                    if let Some(f) = self.pubp.fields_of(&pid) {
                        self.pubp.publish(&pid, cid, &f, Some((name, m["cfg"].clone())));
                    }
                }
                self.bc(&code, &json!({"t": "cfg", "id": me, "cfg": m["cfg"]}), Some(cid));
            }
            "chat" => {
                let text = m["text"].as_str().unwrap_or("").to_owned();
                let now = crate::now();
                let Some(c) = self.conns.get_mut(&cid) else { return };
                if text == c.last_txt && now - c.last_at < 5000 {
                    return;
                }
                c.last_txt = text.clone();
                c.last_at = now;
                let txt = crate::profanity::censor(&text);
                let name =
                    self.rooms.get(&code).and_then(|r| r.players.get(&me)).map(|p| p.name.clone()).unwrap_or_default();
                let msg = json!({"t": "chat", "id": me, "name": name, "text": txt});
                if let Some(r) = self.rooms.get_mut(&code) {
                    r.chat_log.push(json!({"at": now, "id": me, "name": name, "text": txt}));
                    if r.chat_log.len() > 30 {
                        r.chat_log.remove(0);
                    }
                }
                self.bc(&code, &msg, None);
                if let Some(r) = self.rooms.get_mut(&code) {
                    let bots = r.bot_list();
                    let bc = r.bot_chat.get_or_insert_with(BotChat::default);
                    bc.hear(&txt, &bots, now as f64, &name, &mut rnd);
                }
            }
            "item" => {
                let item = m["item"].clone();
                if let Some(i) = item.as_str()
                    && self.cat.recipe_items.contains(i)
                {
                    // alimento de receita fica bloqueado até o jogador prepará-la pela primeira vez
                    let ok = self.conns.get(&cid).is_some_and(|c| {
                        c.ident_scope == "primary"
                            && !c.ident_id.is_empty()
                            && self.cook.unlocked(&pid_of(&c.ident_id), i)
                    });
                    if !ok {
                        return;
                    }
                }
                let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) else { return };
                if m["use"] == 1 {
                    if item.is_null() || p.item != item {
                        return;
                    }
                    let msg = json!({"t": "item", "id": me, "item": p.item, "use": 1});
                    if item.as_str().is_some_and(|i| self.cat.food.contains(i)) {
                        p.item = Value::Null;
                    }
                    self.bc(&code, &msg, None);
                    return;
                }
                p.item = item.clone();
                self.bc(&code, &json!({"t": "item", "id": me, "item": item, "use": 0}), None);
            }
            "gest" => {
                let g = m["g"].as_str().unwrap_or("");
                if self.cat.sound_gests.contains(g) {
                    let who = self
                        .conns
                        .get(&cid)
                        .map(|c| if c.ident_id.is_empty() { c.fid.clone() } else { c.ident_id.clone() })
                        .unwrap_or_default();
                    let now = crate::now();
                    let limited = !self.action_limit.take(&who, "gestSound", now);
                    let Some(c) = self.conns.get_mut(&cid) else { return };
                    c.gest_hits.retain(|x| now - x < 60000);
                    if limited || now - c.gest_snd < 5000 || c.gest_hits.len() >= 6 {
                        return self
                            .send(cid, &json!({"t": "sys", "text": "⏳ Gestos com som têm limite para evitar spam."}));
                    }
                    c.gest_snd = now;
                    c.gest_hits.push(now);
                }
                self.bc(&code, &json!({"t": "gest", "id": me, "g": g}), Some(cid));
            }
            "emo" => self.bc(&code, &json!({"t": "emo", "id": me, "i": m["i"]}), Some(cid)),
            "speak" => {
                let v = m["v"] == true;
                let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) else { return };
                if v == p.talk {
                    return;
                }
                p.talk = v;
                self.talk_log(&code, &me, v);
                self.bc(&code, &json!({"t": "speak", "id": me, "v": v}), Some(cid));
            }
            "ice" => {
                let ice = self.ice_cfg();
                self.send(cid, &json!({"t": "ice", "ice": ice}));
            }
            "vlisten" => {
                let v = m["v"] == true;
                let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) else { return };
                if v == p.vlisten {
                    return;
                }
                p.vlisten = v;
                self.bc(&code, &json!({"t": "vl", "id": me, "v": v}), None);
            }
            "vmute" => {
                let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) else { return };
                let id = m["id"].as_str().unwrap_or("").to_owned();
                if m["v"] == true {
                    p.vmute.insert(id);
                    if p.vmute.len() > 200 {
                        p.vmute.shift_remove_index(0);
                    }
                } else {
                    p.vmute.shift_remove(&id);
                }
            }
            "vcap" => {
                if let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) {
                    p.vop = m["opus"].as_bool();
                }
                self.vcodec_sync(&code);
            }
            "bots" => {
                let Some(r) = self.rooms.get_mut(&code) else { return };
                if r.host.as_deref() != Some(&me) {
                    return;
                }
                r.want = m["n"].as_i64().unwrap_or(0);
                let want_bot = r.want.min(self.set.max_per_room - r.human_count() as i64);
                self.fit(&code);
                let have = self.rooms.get(&code).map_or(0, |r| r.players.values().filter(|p| p.bot).count() as i64);
                if have < want_bot {
                    self.send(cid, &json!({"t": "sys", "text": "Não há mais bots disponíveis no momento."}));
                }
                let want = self.rooms.get(&code).map_or(0, |r| r.want);
                self.bc(&code, &json!({"t": "bots", "want": want}), None);
            }
            "tsync" => self.send(cid, &json!({"t": "tsync", "c": m["c"], "s": crate::now()})),
            "friends" => self.soc_friends(cid, m),
            "pubp" | "plike" | "gbook" | "gblist" | "gbdel" => self.profile_msg(cid, t, m),
            "farmget" | "farmdo" | "supply" | "cookget" | "cookdo" | "cookrenew" => {
                self.economy_msg(cid, t, m, &code, &me)
            }
            "healthget" | "medcraft" | "medheal" => self.health_msg(cid, t, m, &code, &me),
            "giftget" | "giftsee" | "giftsend" | "giftdel" => self.gift_msg(cid, t, m),
            "freq" => self.soc_request(cid, m["to"].as_str().unwrap_or("")),
            "facc" => self.soc_accept(cid, m["to"].as_str().unwrap_or("")),
            "finv" => self.soc_invite(cid, m["to"].as_str().unwrap_or("")),
            "gsub" => self.soc_subscribe(cid, m["on"] == true),
            "gchat" => self.soc_say(cid, m["text"].as_str().unwrap_or("")),
            _ => {}
        }
    }

    /// A conexão fechou (com o código recebido ou o que este lado mandou).
    pub fn on_close(&mut self, cid: u64, code: u16) {
        let Some(c) = self.conns.get(&cid) else { return };
        let ip = c.ip.clone();
        if let Some(n) = self.conns_per_ip.get_mut(&ip) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                self.conns_per_ip.remove(&ip);
            }
        }
        self.soc_leave(cid);
        self.auth.forget_owner(cid);
        let pp = self.conns.get(&cid).map(|c| c.pp.clone()).unwrap_or_default();
        if !pp.is_empty() {
            let snap = self.me_info(cid);
            self.pubp.leave(&pp, cid, snap);
        }
        let Some(c) = self.conns.remove(&cid) else { return };
        let (Some(room), Some(me)) = (c.room, c.me) else { return };
        if matches!(code, 4000 | 4001 | 1008 | 1009) {
            self.drop_player(&room, &me);
        } else {
            self.mark_offline(&room, &me);
        }
    }

    // ---------- relógios ----------

    /// Prazos curtos (verificados a cada 10 ms): reconexão vencida, entrada demorada e estados adiados.
    pub fn timers(&mut self, now: u64) {
        let mut retire = Vec::new();
        for r in self.rooms.values() {
            for p in r.players.values() {
                if p.retire.is_some_and(|t| now >= t) {
                    retire.push((r.code.clone(), p.id.clone()));
                }
            }
        }
        for (code, pid) in retire {
            self.drop_player(&code, &pid);
        }
        let late: Vec<u64> =
            self.conns.values().filter(|c| c.me.is_none() && c.jt.is_some_and(|t| now >= t)).map(|c| c.id).collect();
        for cid in late {
            if let Some(c) = self.conns.get_mut(&cid) {
                c.jt = None;
            }
            self.safe_close(cid, 1000, "");
        }
        let mut due = Vec::new();
        for c in self.conns.values_mut() {
            for (t, d) in &mut c.defer {
                if d.1.is_some_and(|x| now >= x) {
                    d.1 = None;
                    if let Some(m) = d.0.take() {
                        due.push((c.id, t.clone(), m));
                    }
                }
            }
        }
        for (cid, _, m) in due {
            let ok = self.conns.get(&cid).is_some_and(|c| c.open && c.me.is_some() && c.room.is_some());
            if ok {
                self.handle(cid, m);
            }
        }
    }

    /// A cada 100 ms: bots andam e falam; a cada 200 ms o estado das salas sai em binário.
    pub fn tick(&mut self, now: u64) {
        let t0 = std::time::Instant::now();
        self.tick_n += 1;
        let send_st = self.tick_n % 2 == 0;
        let codes: Vec<String> = self.rooms.keys().cloned().collect();
        for code in codes {
            let mut said = None;
            let Some(r) = self.rooms.get_mut(&code) else { continue };
            let full = send_st && now >= r.full_at;
            if now >= r.bot_at {
                r.bot_at = now + 250;
                let bl = r.bot_list();
                let online = r.online_humans().next().is_some();
                if !bl.is_empty() || r.bot_chat.is_some() {
                    let bc = r.bot_chat.get_or_insert_with(BotChat::default);
                    if let Some((i, text)) = bc.tick(now as f64, &bl, online, &mut rnd) {
                        said = Some(json!({"t": "chat", "name": bl[i].name, "text": text}));
                    }
                }
            }
            let mut st = Vec::new();
            for p in r.players.values_mut() {
                if p.bot {
                    if now as f64 > p.next {
                        p.next = now as f64 + 3000.0 + rnd() * 5000.0;
                        let (tx, tz) = V::clamp_pos(Some(rr(-6.0, 6.0)), Some(rr(-2.5, 4.5)));
                        p.tx = tx;
                        p.tz = tz;
                    }
                    let (dx, dz) = (p.tx - p.x, p.tz - p.z);
                    let d = dx.hypot(dz);
                    if d > 0.2 {
                        p.x += dx / d * 0.14;
                        p.z += dz / d * 0.14;
                        p.ry = dx.atan2(dz);
                    }
                }
                if !send_st {
                    continue;
                }
                let k = format!("{:.2},{:.2},{:.2},{:.2},{}", p.x, p.z, p.ry, p.y, i32::from(p.s != 0));
                if full || k != p.lst {
                    p.lst = k;
                    st.push((p.id.clone(), p.x, p.z, p.ry, p.y, p.s != 0));
                }
            }
            if full {
                r.full_at = now + 5000;
            }
            if let Some(m) = said {
                self.bc(&code, &m, None);
            }
            if !st.is_empty() {
                self.bc_bin(&code, st_pack(&st));
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.perf.tlast = ms;
        if ms > 50.0 {
            self.perf.slow += 1;
        }
    }

    /// A cada minuto e outros relógios lentos.
    pub fn slow_tick(&mut self, now: u64) {
        self.bm.tick(now);
        self.names.tick(now);
        self.identity.tick(now);
        self.player_data.tick(now);
        self.pubp.tick(now);
        self.farm.tick(now);
        self.health.tick_file(now);
        self.gifts.tick(now);
        self.cook.tick(now);
        self.badges.tick(now);
        let settled = std::mem::take(&mut self.identity.settled);
        for id in settled {
            self.seclog.log("recovery.apply", crate::seclog::Ev { id: &id, ..Default::default() });
            self.auth.revoke_sessions(&id);
            self.close_identity_conns(&id, None, Some("primary"));
        }
    }

    pub fn ten_minutes(&mut self) {
        self.pubp.sweep();
    }

    pub fn minute(&mut self, now: u64) {
        self.limiter.prune(now);
        self.bm.bans.prune(now);
        self.auth.sweep();
    }

    /// Ping a cada 30 s: quem não respondeu ao anterior cai.
    pub fn ping_round(&mut self) -> Vec<u64> {
        let mut alive = Vec::new();
        let dead: Vec<u64> = self.conns.values().filter(|c| !c.alive).map(|c| c.id).collect();
        for cid in dead {
            if let Some(l) = self.conns.get(&cid).and_then(|c| c.link.clone()) {
                let _ = l.tx.send(Out::Kill);
            }
        }
        for c in self.conns.values_mut() {
            if c.alive {
                c.alive = false;
                alive.push(c.id);
            }
        }
        alive
    }

    pub fn flush_all(&mut self) {
        let now = crate::now();
        self.bm.flush(now);
        self.names.flush();
        self.identity.flush();
        self.player_data.flush();
        self.pubp.flush();
        self.farm.flush();
        self.health.flush();
        self.gifts.flush();
        self.cook.flush();
        self.badges.flush();
    }

    /// Números do painel `/status.json` (`stats`).
    pub fn stats(&self) -> Value {
        let (mut humans, mut bots) = (0, 0);
        let mut styles = serde_json::Map::new();
        for r in self.rooms.values() {
            let h = r.human_count();
            humans += h;
            bots += r.players.len() - h;
            let b = styles.entry(r.style.clone()).or_insert_with(|| json!({"rooms": 0, "humans": 0}));
            b["rooms"] = json!(b["rooms"].as_u64().unwrap_or(0) + 1);
            b["humans"] = json!(b["humans"].as_u64().unwrap_or(0) + h as u64);
        }
        let mut identity = self.identity.stats();
        identity["sessions"] = self.auth.stats()["sessions"].clone();
        identity["pow"] = json!(self.set.identity_pow);
        identity["powMax"] = json!(self.set.identity_pow_max);
        identity["regDay"] = json!(self.set.identity_reg_day);
        identity["graceMs"] = json!(self.set.grace_ms);
        identity["passkeysOn"] = json!(self.set.passkeys);
        json!({
            "server": "rust", "version": VERSION, "rooms": self.rooms.len(), "maxRooms": self.set.max_rooms,
            "maxPerRoom": self.set.max_per_room,
            "capacity": crate::capacity::estimate(self.set.max_rooms, self.set.max_per_room, None),
            "voice": {"transport": if self.set.voice_ws { "ws" } else { "webrtc" }, "privacy": self.set.voice_privacy, "on": self.set.voice_ws, "turn": false},
            "humans": humans, "bots": bots, "karaoke": 0, "video": 0, "styles": styles, "connections": self.conns.len(),
            "identity": identity, "security": self.seclog.stats(),
            "uptime": (crate::now() - self.started) / 1000,
            "abuse": {
                "bans": self.bm.bans.len(),
                "rejections": {"http429": self.abuse.r429, "origem": self.abuse.r403, "ban": self.abuse.r_ban},
                "ticket": {"falhas": self.abuse.ticket, "pow": self.abuse.pow, "mode": self.set.ticket_mode, "maxBits": self.set.pow_max, "rate": self.set.ticket_rate},
                "limits": {"perip": self.set.perip, "peripRate": self.set.perip_rate, "peripRoom": self.set.perip_room, "globalRate": self.set.global_rate, "http": self.set.http_rate, "maxBots": self.set.maxbots, "ban": self.bm.on}
            },
            "perf": {"tick": {"last": self.perf.tlast, "slow": self.perf.slow, "budget": 100}, "msgs": {"inTotal": self.perf.in_t, "outTotal": self.perf.out_t}, "buffer": {"terminated": self.perf.bp}}
        })
    }

    pub fn humans_online(&self) -> usize {
        self.rooms.values().map(|r| r.online_humans().count()).sum()
    }
}

/// Estado em binário: `[1]` e, por jogador, u32 id (bit alto = bot), i16 x·100, i16 z·100, i16 giro·10000, u8 altura·100,
/// u8 sentado.
pub fn st_pack(s: &[(String, f64, f64, f64, f64, bool)]) -> Vec<u8> {
    let i16v = |v: f64| v.round().clamp(-32768.0, 32767.0) as i16;
    let mut b = Vec::with_capacity(1 + s.len() * 12);
    b.push(1);
    for (id, x, z, ry, y, sit) in s {
        let n: u32 = id[1..].parse::<u64>().map_or(0, |n| (n & 0x7fff_ffff) as u32);
        let w = if id.starts_with('b') { 0x8000_0000 | n } else { n };
        b.extend_from_slice(&w.to_le_bytes());
        b.extend_from_slice(&i16v(x * 100.0).to_le_bytes());
        b.extend_from_slice(&i16v(z * 100.0).to_le_bytes());
        b.extend_from_slice(&i16v(ry.sin().atan2(ry.cos()) * 10000.0).to_le_bytes());
        b.push((y * 100.0).round().clamp(0.0, 255.0) as u8);
        b.push(u8::from(*sit));
    }
    b
}

impl Game {
    /// Perfil público, curtidas e livro de visitas.
    fn profile_msg(&mut self, cid: u64, t: &str, m: &Value) {
        let Some(c) = self.conns.get(&cid) else { return };
        let (scope, id, pp) = (c.ident_scope.clone(), c.ident_id.clone(), c.pp.clone());
        let primary = scope == "primary" && !id.is_empty();
        match t {
            "pubp" => {
                if !primary {
                    return self.send(cid, &json!({"t": "err", "scope": "pubp", "msg": "Entre com sua identidade para ter um perfil público."}));
                }
                let pid = pid_of(&id);
                if !pp.is_empty() && pp != pid {
                    self.pubp.leave(&pp, cid, None);
                }
                let src = if m["on"] == true { m.clone() } else { self.pubp.fields_of(&pid).unwrap_or(Value::Null) };
                let f = match crate::pubprofile::clean_fields(&src, true) {
                    Ok(f) => f,
                    Err(e) => return self.send(cid, &json!({"t": "pubp", "on": 0, "err": e})),
                };
                let me = self.me_info(cid);
                if !self.pubp.publish(&pid, cid, &f, me) {
                    return self.send(cid, &json!({"t": "pubp", "on": 0, "err": "full"}));
                }
                if let Some(c) = self.conns.get_mut(&cid) {
                    c.pp = pid.clone();
                }
                let (code, me) = self.conns.get(&cid).map(|c| (c.room.clone(), c.me.clone())).unwrap_or_default();
                if let (Some(code), Some(me)) = (code, me) {
                    if let Some(p) = self.rooms.get_mut(&code).and_then(|r| r.players.get_mut(&me)) {
                        p.pp = pid.clone();
                    }
                    let fields = self.pubp.fields_of(&pid);
                    self.send(cid, &json!({"t": "pubp", "on": 1, "pp": pid, "f": fields}));
                    self.bc(&code, &json!({"t": "ppid", "id": me, "pp": pid}), Some(cid));
                }
            }
            "plike" => {
                if scope != "primary" {
                    return self.send(
                        cid,
                        &json!({"t": "err", "scope": "plike", "msg": "Entre com sua identidade para curtir."}),
                    );
                }
                let to = m["to"].as_str().unwrap_or("");
                let r = self.pubp.like(to, &pid_of(&id));
                self.send(cid, &json!({"t": "plike", "to": to, "ok": r["ok"], "dup": r["dup"] == true, "reason": r["reason"].as_str().unwrap_or(""), "count": r["count"].as_u64().unwrap_or(0)}));
            }
            "gbook" => {
                if scope != "primary" {
                    return self.send(cid, &json!({"t": "err", "scope": "gbook", "msg": "Entre com sua identidade para escrever no livro de visitas."}));
                }
                let to = m["to"].as_str().unwrap_or("");
                let name = self.me_info(cid).map(|x| x.0).unwrap_or_default();
                let text = crate::profanity::censor(m["text"].as_str().unwrap_or(""));
                match self.pubp.post(to, &pid_of(&id), &name, &text) {
                    Ok(owners) => {
                        self.send(cid, &json!({"t": "gbook", "to": to, "ok": true, "reason": ""}));
                        for o in owners {
                            let opp = self.conns.get(&o).map(|c| c.pp.clone()).unwrap_or_default();
                            if !opp.is_empty() {
                                let list = self.pubp.list_book(&opp);
                                self.send(o, &json!({"t": "gblist", "list": list}));
                            }
                        }
                    }
                    Err(e) => self.send(cid, &json!({"t": "gbook", "to": to, "ok": false, "reason": e})),
                }
            }
            "gblist" => {
                let list = if pp.is_empty() { json!([]) } else { self.pubp.list_book(&pp) };
                self.send(cid, &json!({"t": "gblist", "list": list}));
            }
            "gbdel" => {
                if !pp.is_empty() {
                    self.pubp.remove_note(&pp, m["id"].as_i64().unwrap_or(0));
                }
                let list = if pp.is_empty() { json!([]) } else { self.pubp.list_book(&pp) };
                self.send(cid, &json!({"t": "gblist", "list": list}));
            }
            _ => {}
        }
    }
}

/// Origem para o teto de contas novas por dia: IPv4 inteiro, IPv6 pelo bloco /48.
pub fn reg_day_key(ip: &str) -> String {
    if ip.ends_with("::/64") {
        format!("{}::/48", ip.split(':').take(3).collect::<Vec<_>>().join(":"))
    } else {
        ip.to_owned()
    }
}

impl Game {
    /// Código público de quem age, ou vazio (e avisa) sem identidade autenticada (`farmOwner`, `cookOwner`…).
    pub fn owner_of(&mut self, cid: u64, scope: &str, msg: &str) -> String {
        let pid = self
            .conns
            .get(&cid)
            .filter(|c| c.ident_scope == "primary" && !c.ident_id.is_empty())
            .map(|c| pid_of(&c.ident_id));
        match pid {
            Some(p) => p,
            None => {
                self.send(cid, &json!({"t": "err", "scope": scope, "msg": msg}));
                String::new()
            }
        }
    }

    fn cook_view(&self, pid: &str) -> Value {
        let mut v = self.cook.view(pid);
        v["t"] = json!("cook");
        v["pantry"] = self.farm.view(pid)["pantry"].clone();
        v
    }

    fn farm_view(&self, pid: &str) -> Value {
        let mut v = self.farm.view(pid);
        v["t"] = json!("farm");
        v
    }

    /// Fazenda e culinária.
    fn economy_msg(&mut self, cid: u64, t: &str, m: &Value, code: &str, me: &str) {
        const FARM_MSG: &str = "Entre com sua identidade para plantar e guardar sua fazenda.";
        const COOK_MSG: &str = "Entre com sua identidade para cozinhar e guardar seu progresso.";
        let rid = m["rid"].clone();
        let ip = self.conns.get(&cid).map(|c| c.ip.clone()).unwrap_or_default();
        match t {
            "farmget" => {
                let pid = self.owner_of(cid, "farm", FARM_MSG);
                if !pid.is_empty() {
                    let v = self.farm_view(&pid);
                    self.send(cid, &v);
                }
            }
            "supply" => {
                let pid = self.owner_of(cid, "farm", FARM_MSG);
                if pid.is_empty() {
                    return;
                }
                let mut r = self.farm.supply(&pid, m["i"].as_str().unwrap_or(""));
                let ok = r["ok"] == true;
                r["t"] = json!("supplyr");
                r["rid"] = rid;
                self.send(cid, &r);
                if ok {
                    let v = self.cook_view(&pid);
                    self.send(cid, &v);
                }
            }
            "cookget" => {
                let pid = self.owner_of(cid, "cook", COOK_MSG);
                if !pid.is_empty() {
                    let v = self.cook_view(&pid);
                    self.send(cid, &v);
                }
            }
            "farmdo" => {
                let pid = self.owner_of(cid, "farm", FARM_MSG);
                if pid.is_empty() {
                    return;
                }
                let k = m["k"].clone();
                let g = self.guard.check(&pid);
                if g["ok"] != true {
                    return self.send(cid, &json!({"t": "farmr", "rid": rid, "k": k, "ok": false, "reason": g["reason"], "wait": g["wait"]}));
                }
                let origin = reg_day_key(&ip);
                let fresh = !self.farm.has(&pid);
                if fresh && k == "plant" && !self.guard.may_open(&origin, "farm") {
                    return self.send(cid, &json!({"t": "farmr", "rid": rid, "k": k, "ok": false, "reason": "novos"}));
                }
                let guard = &mut self.guard;
                let mut gain = |p: &str, xp: f64, scale: f64| guard.factor(p, "farm", xp, scale);
                let mut r = self.farm.act(&pid, m, &mut gain);
                let ok = r["ok"] == true;
                if ok {
                    self.guard.act(&pid);
                    if fresh {
                        self.guard.opened(&origin, "farm");
                    }
                }
                let first_crop = r["got"][0]["c"].clone();
                let rank = r["rank"].clone();
                r["t"] = json!("farmr");
                r["rid"] = rid;
                r["k"] = k.clone();
                self.send(cid, &r);
                if ok && k == "harvest" {
                    let v = self.cook_view(&pid);
                    self.send(cid, &v);
                    if !first_crop.is_null() {
                        self.bc(code, &json!({"t": "farmfx", "id": me, "c": first_crop}), Some(cid));
                    }
                }
                if ok && k == "renew" {
                    self.bc(code, &json!({"t": "farmfx", "id": me, "sf": rank}), Some(cid));
                }
            }
            "cookdo" => {
                let pid = self.owner_of(cid, "cook", COOK_MSG);
                if pid.is_empty() {
                    return;
                }
                let (x, z) = self.rooms.get(code).and_then(|r| r.players.get(me)).map_or((0.0, 0.0), |p| (p.x, p.z));
                if (x - 0.0).hypot(z - 1.2) > 6.5 {
                    return self.send(cid, &json!({"t": "cookr", "rid": rid, "ok": false, "reason": "longe"}));
                }
                let g = self.guard.check(&pid);
                if g["ok"] != true {
                    return self.send(
                        cid,
                        &json!({"t": "cookr", "rid": rid, "ok": false, "reason": g["reason"], "wait": g["wait"]}),
                    );
                }
                let origin = reg_day_key(&ip);
                let fresh = !self.cook.has(&pid);
                if fresh && !self.guard.may_open(&origin, "cook") {
                    return self.send(cid, &json!({"t": "cookr", "rid": rid, "ok": false, "reason": "novos"}));
                }
                let ids: Vec<String> =
                    m["g"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
                let method = m["m"].as_str().unwrap_or("").to_owned();
                let (guard, farm) = (&mut self.guard, &mut self.farm);
                let mut gain = |p: &str, xp: f64, scale: f64| guard.factor(p, "cook", xp, scale);
                let mut consume = |p: &str, ids: &[String]| farm.consume(p, ids);
                let mut r = self.cook.cook(&pid, &method, &ids, &mut gain, &mut consume);
                let ok = r["ok"] == true;
                if ok {
                    self.guard.act(&pid);
                    if fresh {
                        self.guard.opened(&origin, "cook");
                    }
                    let g: Vec<String> =
                        r["g"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_owned)).collect();
                    let home = self.farm.grown(&pid, &g);
                    let base = r["coins"].as_f64().unwrap_or(0.0);
                    let coins =
                        self.farm.earn(&pid, (base * (1.0 + crate::farm::HOME_BONUS * home as f64)).round() as i64);
                    r["home"] = json!(home);
                    r["coins"] = json!(coins);
                    r["wallet"] = self.farm.view(&pid)["coins"].clone();
                    if let Some(o) = r.as_object_mut() {
                        o.remove("g");
                    }
                    r["pantry"] = self.farm.view(&pid)["pantry"].clone();
                    let fv = self.farm_view(&pid);
                    self.send(cid, &fv);
                }
                let recipe = r["recipe"].clone();
                r["t"] = json!("cookr");
                r["rid"] = rid;
                self.send(cid, &r);
                if ok {
                    self.bc(code, &json!({"t": "cookfx", "id": me, "r": recipe}), Some(cid));
                }
            }
            "cookrenew" => {
                let pid = self.owner_of(cid, "cook", COOK_MSG);
                if pid.is_empty() {
                    return;
                }
                let mut r = self.cook.renew(&pid);
                let ok = r["ok"] == true;
                let rank = r["rank"].clone();
                r["t"] = json!("cookr");
                r["rid"] = rid;
                r["renew"] = json!(1);
                self.send(cid, &r);
                if ok {
                    self.bc(code, &json!({"t": "cookfx", "id": me, "st": rank}), Some(cid));
                }
            }
            _ => {}
        }
    }

    /// `/profile.json`: o perfil público com emblemas e o resumo de cada parte do jogo.
    pub fn profile_json(&self, pid: &str) -> Option<Value> {
        let mut pr = self.pub_get(pid)?;
        let mut extra = self.cook.badges_of(pid);
        extra.extend(self.farm.badges_of(pid));
        pr["badges"] = self.badges.of(pid, &pr, &extra);
        pr["pet"] = Value::Null;
        pr["farm"] = self.farm.public_of(pid);
        pr["cook"] = self.cook.public_of(pid);
        pr["health"] = self.health.public_of(pid);
        pr["gifts"] = self.gifts.public_of(pid);
        Some(pr)
    }

    /// `/farm.json`: a fazenda de quem tem perfil público.
    pub fn farm_json(&self, pid: &str) -> Option<Value> {
        let pr = self.pub_get(pid)?;
        let mut f = self.farm.public_of(pid);
        f["name"] = pr["name"].clone();
        Some(f)
    }
}

impl Game {
    /// Relógio da saúde (a cada 10 s): um por pessoa, mesmo com várias abas.
    pub fn health_tick(&mut self) {
        let mut seen = HashSet::new();
        let mut who: Vec<(String, bool)> = Vec::new();
        for r in self.rooms.values() {
            for p in r.players.values() {
                if p.bot || !p.online {
                    continue;
                }
                let Some(i) = p.ident.as_ref().filter(|i| !i.id.is_empty()) else { continue };
                if seen.insert(pid_of(&i.id)) {
                    who.push((i.id.clone(), p.s != 0));
                }
            }
        }
        for (id, sitting) in who {
            let pid = pid_of(&id);
            self.health.tick(&pid, if sitting { 0 } else { 10000 });
            if self.health_seen.get(&pid) != Some(&self.health.public_of(&pid).to_string()) {
                self.health_notify(&id);
            }
        }
        self.health_seen.retain(|k, _| seen.contains(k));
    }

    fn health_notify(&mut self, id: &str) {
        let pid = pid_of(id);
        let status = self.health.public_of(&pid);
        self.health_seen.insert(pid.clone(), status.to_string());
        let mut view = self.health.view(&pid);
        view["t"] = json!("health");
        let mine: Vec<u64> =
            self.conns.values().filter(|c| c.ident_scope == "primary" && c.ident_id == id).map(|c| c.id).collect();
        for c in mine {
            self.send(c, &view);
        }
        let mut targets = Vec::new();
        for r in self.rooms.values() {
            for p in r.players.values() {
                if p.ident.as_ref().is_some_and(|i| i.id == id) {
                    targets.push((r.code.clone(), p.id.clone()));
                }
            }
        }
        for (code, pid2) in targets {
            let mut m = json!({"t": "healthstate", "id": pid2});
            for (k, v) in status.as_object().into_iter().flatten() {
                m[k] = v.clone();
            }
            self.bc(&code, &m, None);
        }
    }

    fn health_msg(&mut self, cid: u64, t: &str, m: &Value, code: &str, me: &str) {
        const MSG: &str = "Entre com sua identidade para cozinhar e guardar seu progresso.";
        let pid = self.owner_of(cid, "cook", MSG);
        if pid.is_empty() {
            return;
        }
        let rid = m["rid"].clone();
        let i = m["i"].as_str().unwrap_or("").to_owned();
        match t {
            "healthget" => {
                let mut v = self.health.view(&pid);
                v["t"] = json!("health");
                self.send(cid, &v);
            }
            "medcraft" => {
                let g = self.guard.check(&pid);
                if g["ok"] != true {
                    return self.send(cid, &json!({"t": "medr", "rid": rid, "ok": false, "reason": g["reason"]}));
                }
                let farm = &mut self.farm;
                let mut consume = |p: &str, ids: &[String]| farm.consume(p, ids);
                let mut r = self.health.craft(&pid, &i, &mut consume);
                let ok = r["ok"] == true;
                r["t"] = json!("medr");
                r["rid"] = rid;
                r["k"] = json!("craft");
                r["i"] = json!(i);
                self.send(cid, &r);
                if ok {
                    self.guard.act(&pid);
                    let fv = self.farm_view(&pid);
                    self.send(cid, &fv);
                    let cv = self.cook_view(&pid);
                    self.send(cid, &cv);
                }
            }
            "medheal" => {
                let to = m["to"].as_str().unwrap_or("");
                let error = |g: &mut Game, reason: &str| {
                    g.send(cid, &json!({"t": "medr", "rid": rid, "ok": false, "reason": reason}))
                };
                let Some(p) = self.rooms.get(code).and_then(|r| r.players.get(to)).cloned() else {
                    return error(self, "alvo");
                };
                let Some(pi) = p.ident.clone().filter(|i| !p.bot && p.online && !i.id.is_empty()) else {
                    return error(self, "alvo");
                };
                let my_id = self.conns.get(&cid).map(|c| c.ident_id.clone()).unwrap_or_default();
                if pi.id == my_id {
                    return error(self, "autocura");
                }
                let (mx, mz) = self.rooms.get(code).and_then(|r| r.players.get(me)).map_or((0.0, 0.0), |x| (x.x, x.z));
                if (mx - p.x).hypot(mz - p.z) > 3.0 {
                    return error(self, "longe");
                }
                let friends = |d: Option<Value>, other: &str| {
                    d.is_some_and(|d| d["friends"].as_array().is_some_and(|a| a.iter().any(|x| x == other)))
                };
                if !(friends(self.player_data.get(&my_id), &pi.id) && friends(self.player_data.get(&pi.id), &my_id)) {
                    return error(self, "amigo");
                }
                let mut r = self.health.heal(&pid, &pid_of(&pi.id), &i);
                let ok = r["ok"] == true;
                r["t"] = json!("medr");
                r["rid"] = rid.clone();
                r["k"] = json!("heal");
                r["i"] = json!(i);
                self.send(cid, &r);
                if !ok {
                    return;
                }
                self.health_notify(&pi.id);
                self.bc(code, &json!({"t": "medfx", "id": me, "to": p.id, "i": i}), None);
            }
            _ => {}
        }
    }
}

impl Game {
    /// Presentes: a caixa, ver a de outro, enviar e apagar.
    fn gift_msg(&mut self, cid: u64, t: &str, m: &Value) {
        const MSG: &str = "Entre com sua identidade para enviar e receber presentes.";
        let merge = |mut a: Value, b: Value| {
            for (k, v) in b.as_object().into_iter().flatten() {
                a[k] = v.clone();
            }
            a
        };
        match t {
            "giftsee" => {
                let to = m["to"].as_str().unwrap_or("");
                let out = match self.pub_get(to) {
                    Some(pr) => merge(json!({"t": "giftbox", "to": to, "name": pr["name"]}), self.gifts.public_of(to)),
                    None => json!({"t": "giftbox", "to": to, "none": 1}),
                };
                self.send(cid, &out);
            }
            "giftget" | "giftdel" => {
                let pid = self.owner_of(cid, "gift", MSG);
                if pid.is_empty() {
                    return;
                }
                if t == "giftget" && m["seen"] == true {
                    self.gifts.seen(&pid);
                }
                if t == "giftdel" {
                    self.gifts.remove(&pid, m["id"].as_i64().unwrap_or(0));
                }
                let q = self.gifts.quota(&pid);
                let out = merge(merge(json!({"t": "gifts"}), self.gifts.box_of(&pid)), q);
                self.send(cid, &out);
            }
            "giftsend" => {
                let pid = self.owner_of(cid, "gift", MSG);
                if pid.is_empty() {
                    return;
                }
                let (to, k, i) =
                    (m["to"].as_str().unwrap_or(""), m["k"].as_str().unwrap_or(""), m["i"].as_str().unwrap_or(""));
                let can = if k == "c" { self.farm.harvested(&pid, i) } else { self.cook.unlocked(&pid, i) };
                let name = self.me_info(cid).map(|x| x.0).unwrap_or_default();
                let r = if self.pubp.has(to) {
                    self.gifts.send(&pid, to, k, i, &name, can)
                } else {
                    json!({"ok": false, "reason": "sem_perfil"})
                };
                let q = self.gifts.quota(&pid);
                let ok = r["ok"] == true;
                let gift = r["gift"].clone();
                let out = merge(merge(json!({"t": "giftr", "rid": m["rid"], "to": to}), r), q);
                self.send(cid, &out);
                if !ok {
                    return;
                }
                let unseen = self.gifts.unseen(to);
                let targets: Vec<u64> = self
                    .conns
                    .values()
                    .filter(|c| {
                        c.id != cid && c.ident_scope == "primary" && !c.ident_id.is_empty() && pid_of(&c.ident_id) == to
                    })
                    .map(|c| c.id)
                    .collect();
                for w in targets {
                    self.send(w, &json!({"t": "giftin", "n": gift["n"], "k": gift["k"], "i": gift["i"], "at": gift["at"], "unseen": unseen}));
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_packet() {
        let b = st_pack(&[("b7".into(), 1.234, -2.0, 7.0, 0.5, true)]);
        assert_eq!(b.len(), 13);
        assert_eq!(u32::from_le_bytes([b[1], b[2], b[3], b[4]]), 0x8000_0007);
        assert_eq!(i16::from_le_bytes([b[5], b[6]]), 123);
        assert_eq!(i16::from_le_bytes([b[9], b[10]]), ((7.0f64.sin().atan2(7.0f64.cos())) * 10000.0).round() as i16);
        assert_eq!((b[11], b[12]), (50, 1));
        assert_eq!(pid_of("abc").len(), 20);
    }
}
