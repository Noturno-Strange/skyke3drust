//! Métricas do painel `/status.json` (`perfStats`, `resourceStats` e `lib/records.js`): o custo do tick, o atraso do
//! laço, mensagens e bytes por segundo, o maior buffer de saída, a voz, os recordes (gravados em disco) e o uso de CPU,
//! memória e rede da máquina (Linux).

use crate::game::Game;
use crate::persist::{read_json_file, write_json_atomic};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering;

pub const RECORD_KEYS: [&str; 7] = ["humans", "rooms", "rss", "heap", "sysMem", "rx", "tx"];

#[derive(Debug, Clone, Copy, Default)]
struct Sample {
    inn: u64,
    out: u64,
    bytes: u64,
    vin: u64,
    vout: u64,
    vdrop: u64,
    vbytes: u64,
    tn: u64,
    tsum: f64,
    tmax: f64,
    p50: f64,
    p99: f64,
    lmax: f64,
    bmax: usize,
}

#[derive(Debug, Default)]
pub struct Metrics {
    ring: VecDeque<Sample>,
    prev: Sample,
    tick: (u64, f64, f64),
    lags: Vec<f64>,
    pub records: indexmap::IndexMap<&'static str, (f64, u64)>,
    rec_file: Option<PathBuf>,
    rec_dirty: bool,
    rec_saved: u64,
    net_prev: Option<(u64, u64, u64)>,
    cpu_prev: Option<(u64, u64, u64)>,
    proc_prev: Option<(u64, u64)>,
    pub hist: VecDeque<Value>,
    pub peak: (usize, usize),
}

/// Bytes recebidos e enviados por todas as interfaces menos a de loopback (`/proc/net/dev`).
fn net_counters() -> Option<(u64, u64)> {
    let t = std::fs::read_to_string("/proc/net/dev").ok()?;
    let (mut rx, mut tx) = (0, 0);
    for line in t.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else { continue };
        if name.trim().is_empty() || name.trim() == "lo" {
            continue;
        }
        let f: Vec<u64> = rest.split_whitespace().map(|x| x.parse().unwrap_or(0)).collect();
        if f.len() >= 9 {
            rx += f[0];
            tx += f[8];
        }
    }
    Some((rx, tx))
}

/// Memória residente do processo em bytes.
fn rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|k| k.parse::<u64>().ok())
        })
        .map_or(0, |k| k * 1024)
}

/// (total, livre) da máquina em bytes.
fn mem() -> (u64, u64) {
    let t = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let get = |k: &str| {
        t.lines()
            .find(|l| l.starts_with(k))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
            * 1024
    };
    (get("MemTotal:"), get("MemAvailable:"))
}

/// (ocupado, total) de CPU desde o boot, em tiques.
fn cpu_ticks() -> Option<(u64, u64)> {
    let t = std::fs::read_to_string("/proc/stat").ok()?;
    let f: Vec<u64> = t.lines().next()?.split_whitespace().skip(1).map(|x| x.parse().unwrap_or(0)).collect();
    let total: u64 = f.iter().take(8).sum();
    let idle = f.get(3).copied().unwrap_or(0) + f.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

/// Tiques de CPU usados por este processo.
fn proc_ticks() -> Option<u64> {
    let t = std::fs::read_to_string("/proc/self/stat").ok()?;
    let after = t.rsplit_once(')')?.1;
    let f: Vec<&str> = after.split_whitespace().collect();
    Some(f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?)
}

fn r2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}
fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() as f64 - 1.0) * p).round() as usize]
}

impl Metrics {
    pub fn new(rec_file: &str) -> Self {
        let mut m = Self { rec_file: (!rec_file.is_empty()).then(|| PathBuf::from(rec_file)), ..Default::default() };
        for k in RECORD_KEYS {
            m.records.insert(k, (0.0, 0));
        }
        if let Some(f) = &m.rec_file {
            match read_json_file(f) {
                Ok(Some(j)) => {
                    for k in RECORD_KEYS {
                        let x = &j["records"][k];
                        if let (Some(v), Some(at)) = (x["v"].as_f64().filter(|v| *v >= 0.0), x["at"].as_f64()) {
                            m.records.insert(k, (v, at as u64));
                        }
                    }
                }
                Ok(None) => {}
                Err(_) => tracing::warn!("recordes: arquivo inválido, começando do zero."),
            }
        }
        m.net_prev = net_counters().map(|(a, b)| (a, b, crate::now()));
        m.proc_prev = proc_ticks().zip(Some(crate::now()));
        m.cpu_prev = cpu_ticks().map(|(a, b)| (a, b, 0));
        m
    }

    /// Custo de um tick (ms).
    pub fn tick_done(&mut self, ms: f64) {
        self.tick.0 += 1;
        self.tick.1 += ms;
        if ms > self.tick.2 {
            self.tick.2 = ms;
        }
    }

    /// Atraso do relógio de 10 ms (como o `monitorEventLoopDelay` do Node), em ms.
    pub fn lag(&mut self, ms: f64) {
        if self.lags.len() < 2000 {
            self.lags.push(ms.max(0.0));
        }
    }

    pub fn up(&mut self, k: &'static str, v: f64) {
        if let Some(r) = self.records.get_mut(k)
            && v.is_finite()
            && v > r.0
        {
            *r = (r1(v), crate::now() / 1000);
            self.rec_dirty = true;
        }
    }

    pub fn write(&mut self) {
        let Some(f) = self.rec_file.clone() else { return };
        if !self.rec_dirty {
            return;
        }
        self.rec_dirty = false;
        self.rec_saved = crate::now();
        let rec: serde_json::Map<String, Value> =
            self.records.iter().map(|(k, (v, at))| ((*k).to_owned(), json!({"v": v, "at": at}))).collect();
        if let Err(e) = write_json_atomic(&f, &json!({"v": 1, "savedAt": crate::now(), "records": rec}), false) {
            tracing::error!("recordes: {e}");
        }
    }

    pub fn records_json(&self) -> Value {
        Value::Object(self.records.iter().map(|(k, (v, at))| ((*k).to_owned(), json!({"v": v, "at": at}))).collect())
    }
}

impl Game {
    /// A cada segundo: uma amostra para o anel de 60 s.
    pub fn perf_second(&mut self) {
        let bmax = self
            .conns
            .values()
            .filter_map(|c| c.link.as_ref())
            .map(|l| l.buffered.load(Ordering::Relaxed))
            .max()
            .unwrap_or(0);
        let m = &mut self.metrics;
        let mut lags = std::mem::take(&mut m.lags);
        let lmax = lags.iter().copied().fold(0.0, f64::max);
        let cur = Sample {
            inn: self.perf.in_t,
            out: self.perf.out_t,
            bytes: self.perf.bytes_t,
            vin: self.vperf.v_in,
            vout: self.vperf.v_out,
            vdrop: self.vperf.v_drop,
            vbytes: self.vperf.v_bytes,
            ..Default::default()
        };
        let p = m.prev;
        m.ring.push_back(Sample {
            inn: cur.inn - p.inn,
            out: cur.out - p.out,
            bytes: cur.bytes - p.bytes,
            vin: cur.vin - p.vin,
            vout: cur.vout - p.vout,
            vdrop: cur.vdrop - p.vdrop,
            vbytes: cur.vbytes - p.vbytes,
            tn: m.tick.0,
            tsum: m.tick.1,
            tmax: m.tick.2,
            p50: pct(&mut lags, 0.5),
            p99: pct(&mut lags, 0.99),
            lmax,
            bmax,
        });
        m.prev = cur;
        m.tick = (0, 0.0, 0.0);
        while m.ring.len() > 60 {
            m.ring.pop_front();
        }
    }

    pub fn perf_stats(&self) -> Value {
        let ring = &self.metrics.ring;
        let last: Vec<&Sample> = ring.iter().rev().take(10).collect();
        let n = last.len().max(1) as f64;
        let sum = |f: fn(&Sample) -> f64, v: &[&Sample]| v.iter().map(|s| f(s)).fold(0.0, |a, b| a + b);
        let all: Vec<&Sample> = ring.iter().collect();
        let mx = |f: fn(&Sample) -> f64| all.iter().map(|s| f(s)).fold(0.0, f64::max);
        let tn = sum(|s| s.tn as f64, &all);
        json!({
            "tick": {
                "avg": if tn > 0.0 { r2(sum(|s| s.tsum, &all) / tn) } else { 0.0 }, "max": r2(mx(|s| s.tmax)),
                "last": r2(self.perf.tlast), "slow": self.perf.slow, "budget": 100
            },
            "loop": {"p50": r2(mx(|s| s.p50)), "p99": r2(mx(|s| s.p99)), "max": r2(mx(|s| s.lmax))},
            "msgs": {
                "inPerSec": r1(sum(|s| s.inn as f64, &last) / n), "outPerSec": r1(sum(|s| s.out as f64, &last) / n),
                "bytesOutPerSec": (sum(|s| s.bytes as f64, &last) / n).round(), "inTotal": self.perf.in_t, "outTotal": self.perf.out_t
            },
            "buffer": {"max": mx(|s| s.bmax as f64), "terminated": self.perf.bp},
            "voice": {
                "inPerSec": r1(sum(|s| s.vin as f64, &last) / n), "outPerSec": r1(sum(|s| s.vout as f64, &last) / n),
                "bytesOutPerSec": (sum(|s| s.vbytes as f64, &last) / n).round(), "dropped60": sum(|s| s.vdrop as f64, &all),
                "talking": self.voice_now()["talking"], "listening": self.voice_now()["listening"], "congested": self.voice_now()["congested"],
                "framesIn": self.vperf.v_in, "framesOut": self.vperf.v_out, "dropped": self.vperf.v_drop, "limited": self.vperf.v_lim,
                "malformed": self.vperf.v_bad
            }
        })
    }

    /// A cada 5 s: recordes de jogadores, salas, memória e rede.
    pub fn records_sample(&mut self) {
        let h = self.humans_online() as f64;
        let rooms = self.rooms.len() as f64;
        let m = &mut self.metrics;
        m.up("humans", h);
        m.up("rooms", rooms);
        let r = rss() as f64;
        m.up("rss", r);
        m.up("heap", r);
        let (total, free) = mem();
        if total > 0 {
            m.up("sysMem", (total - free) as f64 / total as f64 * 100.0);
        }
        let now = crate::now();
        if let (Some((rx, tx)), Some((prx, ptx, at))) = (net_counters(), m.net_prev) {
            let secs = ((now - at) as f64 / 1000.0).max(1.0);
            m.up("rx", rx.saturating_sub(prx) as f64 / secs);
            m.up("tx", tx.saturating_sub(ptx) as f64 / secs);
        }
        m.net_prev = net_counters().map(|(a, b)| (a, b, now));
        if m.rec_dirty && now - m.rec_saved > 30_000 {
            m.write();
        }
    }

    /// A cada 15 s: histórico de salas e pessoas e os picos.
    pub fn history_sample(&mut self) {
        let (rooms, humans) = (self.rooms.len(), self.rooms.values().map(|r| r.human_count()).sum::<usize>());
        let m = &mut self.metrics;
        m.peak = (m.peak.0.max(rooms), m.peak.1.max(humans));
        m.hist.push_back(json!([crate::now() / 1000, rooms, humans]));
        while m.hist.len() > 240 {
            m.hist.pop_front();
        }
    }

    pub fn resource_stats(&mut self) -> Value {
        let now = crate::now();
        let m = &mut self.metrics;
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get());
        let cpu_system = match (cpu_ticks(), m.cpu_prev) {
            (Some((b, t)), Some((pb, pt, _))) if t > pt => {
                ((b - pb) as f64 / (t - pt) as f64 * 100.0).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };
        let cpu_process = match (proc_ticks(), m.proc_prev) {
            (Some(p), Some((pp, at))) if now > at => {
                ((p - pp) as f64 * 10.0 / (now - at) as f64 * 100.0 / cores as f64).clamp(0.0, 100.0)
            }
            _ => 0.0,
        };
        m.cpu_prev = cpu_ticks().map(|(a, b)| (a, b, now));
        m.proc_prev = proc_ticks().zip(Some(now));
        let (total, free) = mem();
        let used = total.saturating_sub(free);
        let r = rss();
        let net = net_counters();
        json!({
            "cpu": {"system": r1(cpu_system), "process": r1(cpu_process), "cores": cores},
            "memory": {"total": total, "used": used, "free": free, "percent": if total > 0 { r1(used as f64 / total as f64 * 100.0) } else { 0.0 }},
            "game": {"rss": r, "heapTotal": r, "heapUsed": r, "external": 0},
            "network": match net {
                Some((rx, tx)) => json!({"available": true, "rxTotal": rx, "txTotal": tx, "rxRate": 0, "txRate": 0}),
                None => json!({"available": false, "rxTotal": 0, "txTotal": 0, "rxRate": 0, "txRate": 0}),
            }
        })
    }
}
