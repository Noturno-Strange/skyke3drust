//! Rádio do DJ (`lib/radio.js`): o áudio passa pelo servidor, então a rádio só vê o IP do servidor. Um "hub" por sala
//! abre a transmissão uma vez e repassa a todos os ouvintes. Só https, domínio público (o DNS é conferido e fixado),
//! até 3 redirecionamentos, MP3 ou AAC, no máximo 96 KB/s, e o hub fecha 20 s depois que o último ouvinte sai.

use bytes::Bytes;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, watch};

const CONNECT: Duration = Duration::from_secs(8);
const DNS: Duration = Duration::from_secs(3);
const IDLE: Duration = Duration::from_secs(15);
const GRACE: Duration = Duration::from_secs(20);
const MAX_REDIRECTS: u32 = 3;
const MAX_BPS: usize = 96 * 1024;
const RATE_WINDOW: Duration = Duration::from_secs(5);

const NAO_ABRIU: &str =
    "Não foi possível abrir essa rádio. Confira se o link é de uma transmissão de áudio (MP3 ou AAC) em https.";

/// Mensagem para o jogador quando a rádio não abre.
pub fn reason_text(r: &str) -> &'static str {
    match r {
        "url" => "Link inválido. Use o link de uma rádio online.",
        "dns" => "Não foi possível encontrar essa rádio.",
        "ogg" => "Esse formato de rádio não é suportado. Tente outra rádio.",
        _ => NAO_ABRIU,
    }
}

fn ok_type(ct: &str) -> bool {
    let c = ct.to_lowercase();
    [
        "audio/mpeg",
        "audio/mp3",
        "audio/mpeg3",
        "audio/x-mpeg",
        "audio/x-mpeg-3",
        "audio/x-mp3",
        "audio/aac",
        "audio/aacp",
        "audio/x-aac",
        "audio/aacplus",
        "audio/mp4",
        "audio/x-m4a",
        "audio/mp4a-latm",
    ]
    .iter()
    .any(|t| c.starts_with(t) && c[t.len()..].chars().next().is_none_or(|x| !x.is_ascii_alphanumeric() && x != '_'))
}

fn ogg_type(ct: &str) -> bool {
    let c = ct.to_lowercase();
    ["audio/ogg", "audio/x-ogg", "application/ogg", "application/x-ogg"]
        .iter()
        .any(|t| c.starts_with(t) && c[t.len()..].chars().next().is_none_or(|x| !x.is_ascii_alphanumeric() && x != '_'))
}

async fn resolve(host: &str, port: u16) -> Option<SocketAddr> {
    let r = tokio::time::timeout(DNS, tokio::net::lookup_host((host, port))).await.ok()?.ok()?;
    let list: Vec<SocketAddr> = r.collect();
    if list.is_empty() || !list.iter().all(|a| crate::netutil::is_public_address(&a.ip().to_string())) {
        return None;
    }
    list.first().copied()
}

/// Abre a transmissão: (resposta, tipo de conteúdo) ou o motivo da falha.
pub async fn open(url: &str) -> Result<(reqwest::Response, String), &'static str> {
    let mut cur = url.to_owned();
    for left in (0..=MAX_REDIRECTS).rev() {
        let norm = crate::validate::stream_url(&serde_json::Value::String(cur.clone())).ok_or("url")?;
        let u = url::Url::parse(&norm).map_err(|_| "url")?;
        let host = u.host_str().ok_or("url")?.to_owned();
        let port = u.port().unwrap_or(443);
        let addr = resolve(&host, port).await.ok_or("dns")?;
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT)
            .resolve(&host, addr)
            .build()
            .map_err(|_| "connect")?;
        let req = client
            .get(u.clone())
            .header("User-Agent", "Skyke3D-Radio/1.0")
            .header("Accept", "audio/*")
            .header("Icy-MetaData", "0")
            .send();
        let res = match tokio::time::timeout(CONNECT, req).await {
            Err(_) => return Err("timeout"),
            Ok(Err(_)) => return Err("connect"),
            Ok(Ok(r)) => r,
        };
        let code = res.status().as_u16();
        if (300..400).contains(&code)
            && let Some(loc) = res.headers().get("location").and_then(|v| v.to_str().ok())
        {
            if left == 0 {
                return Err("redirects");
            }
            cur = u.join(loc).map_err(|_| "url")?.to_string();
            continue;
        }
        if code != 200 {
            return Err("status");
        }
        let ct = res.headers().get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_owned();
        if ogg_type(&ct) {
            return Err("ogg");
        }
        if !ok_type(&ct) {
            return Err("type");
        }
        let ct = ct.split(';').next().unwrap_or("").trim().to_lowercase();
        return Ok((res, ct));
    }
    Err("redirects")
}

/// Confere se a rádio abre (antes de anunciar o DJ para a sala).
pub async fn probe(url: &str) -> Result<(), &'static str> {
    open(url).await.map(|_| ())
}

#[derive(Debug)]
struct Hub {
    url: String,
    listeners: usize,
    tx: broadcast::Sender<Bytes>,
    /// Tipo de conteúdo quando a transmissão abre; `Some("")` = falhou.
    ct: watch::Receiver<Option<String>>,
    task: tokio::task::JoinHandle<()>,
    gen_: u64,
}

#[derive(Debug, Default)]
pub struct Radio {
    hubs: Mutex<HashMap<String, Hub>>,
    max_hubs: usize,
    next: Mutex<u64>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Um ouvinte ligado a um hub: os pedaços de áudio e o tipo de conteúdo.
#[derive(Debug)]
pub struct Listener {
    pub rx: broadcast::Receiver<Bytes>,
    pub ct: watch::Receiver<Option<String>>,
    radio: Arc<Radio>,
    key: String,
    gen_: u64,
}

impl Drop for Listener {
    fn drop(&mut self) {
        let mut hubs = lock(&self.radio.hubs);
        let Some(h) = hubs.get_mut(&self.key).filter(|h| h.gen_ == self.gen_) else { return };
        h.listeners = h.listeners.saturating_sub(1);
        if h.listeners == 0 {
            let (radio, key, g) = (self.radio.clone(), self.key.clone(), self.gen_);
            tokio::spawn(async move {
                tokio::time::sleep(GRACE).await;
                let mut hubs = lock(&radio.hubs);
                if hubs.get(&key).is_some_and(|h| h.gen_ == g && h.listeners == 0)
                    && let Some(h) = hubs.remove(&key)
                {
                    h.task.abort();
                }
            });
        }
    }
}

impl Radio {
    pub fn new(max_hubs: usize) -> Arc<Self> {
        Arc::new(Self { hubs: Mutex::new(HashMap::new()), max_hubs, next: Mutex::new(1) })
    }

    /// Liga um ouvinte ao hub da sala (abrindo a transmissão se preciso). `None` = limite de hubs.
    pub fn attach(self: &Arc<Self>, key: &str, url: &str) -> Option<Listener> {
        let mut hubs = lock(&self.hubs);
        if hubs.get(key).is_some_and(|h| h.url != url || h.task.is_finished())
            && let Some(h) = hubs.remove(key)
        {
            h.task.abort();
        }
        if !hubs.contains_key(key) {
            if hubs.len() >= self.max_hubs {
                return None;
            }
            let gen_ = {
                let mut n = lock(&self.next);
                *n += 1;
                *n
            };
            let (tx, _) = broadcast::channel::<Bytes>(64);
            let (ct_tx, ct_rx) = watch::channel(None);
            let (u, tx2, radio, k) = (url.to_owned(), tx.clone(), self.clone(), key.to_owned());
            let task = tokio::spawn(async move {
                pump(&u, tx2, ct_tx).await;
                let mut hubs = lock(&radio.hubs);
                if hubs.get(&k).is_some_and(|h| h.gen_ == gen_) {
                    hubs.remove(&k);
                }
            });
            hubs.insert(key.into(), Hub { url: url.into(), listeners: 0, tx, ct: ct_rx, task, gen_ });
        }
        let h = hubs.get_mut(key)?;
        h.listeners += 1;
        Some(Listener { rx: h.tx.subscribe(), ct: h.ct.clone(), radio: self.clone(), key: key.into(), gen_: h.gen_ })
    }

    pub fn stop(&self, key: &str) {
        if let Some(h) = lock(&self.hubs).remove(key) {
            h.task.abort();
        }
    }

    pub fn stats(&self) -> (usize, usize) {
        let h = lock(&self.hubs);
        (h.len(), h.values().map(|x| x.listeners).sum())
    }
}

/// Lê a transmissão e repassa aos ouvintes até cair, ficar parada 15 s ou passar de 96 KB/s.
async fn pump(url: &str, tx: broadcast::Sender<Bytes>, ct: watch::Sender<Option<String>>) {
    let (mut res, kind) = match open(url).await {
        Ok(x) => x,
        Err(_) => {
            let _ = ct.send(Some(String::new()));
            return;
        }
    };
    let _ = ct.send(Some(kind));
    let mut window = tokio::time::Instant::now();
    let mut bytes = 0usize;
    loop {
        let chunk = match tokio::time::timeout(IDLE, res.chunk()).await {
            Ok(Ok(Some(c))) => c,
            _ => break,
        };
        bytes += chunk.len();
        if window.elapsed() >= RATE_WINDOW {
            if bytes as f64 / RATE_WINDOW.as_secs_f64() > MAX_BPS as f64 {
                break;
            }
            bytes = 0;
            window = tokio::time::Instant::now();
        }
        let _ = tx.send(chunk);
    }
}
