//! Limites de capacidade (`lib/capacity.js`): salas, jogadores por sala e a estimativa de custo mostrada no painel.

use serde_json::{Value, json};
use std::path::Path;

pub const MIN_ROOMS: i64 = 1;
pub const MAX_ROOMS: i64 = 150;
pub const MIN_PER_ROOM: i64 = 2;
pub const MAX_PER_ROOM: i64 = 40;
pub const MAX_PLAYERS: i64 = 3000;
pub const DEFAULT_ROOMS: i64 = 60;
pub const DEFAULT_PER_ROOM: i64 = 16;
pub const TICK_HZ: i64 = 5;
pub const STATE_BYTES: i64 = 12;
const WARN_BPS: f64 = 12.5e6;
const CRIT_BPS: f64 = 37.5e6;
const WARN_TICK: f64 = 50.0;
const CRIT_TICK: f64 = 80.0;
const FD_WARN: i64 = 900;

/// Confere salas × jogadores por sala; o erro é a frase mostrada ao administrador.
pub fn validate(rooms: Option<i64>, per_room: Option<i64>) -> Result<(i64, i64), String> {
    let Some(rooms) = rooms.filter(|r| (MIN_ROOMS..=MAX_ROOMS).contains(r)) else {
        return Err(format!("Limite de salas inválido (de {MIN_ROOMS} a {MAX_ROOMS})."));
    };
    let Some(per) = per_room.filter(|p| (MIN_PER_ROOM..=MAX_PER_ROOM).contains(p)) else {
        return Err(format!("Limite de jogadores por sala inválido (de {MIN_PER_ROOM} a {MAX_PER_ROOM})."));
    };
    if rooms * per > MAX_PLAYERS {
        return Err(format!(
            "O servidor foi projetado para no máximo {MAX_PLAYERS} jogadores ao mesmo tempo (salas × jogadores por sala = {}).",
            rooms * per
        ));
    }
    Ok((rooms, per))
}

/// Inteiro de um JSON só se for um número inteiro de verdade (`Number.isInteger`).
pub fn int_of(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().filter(|f| f.fract() == 0.0 && f.abs() < 9e15).map(|f| f as i64))
}

pub fn conn_limit(rooms: i64, per_room: i64) -> i64 {
    ((rooms * per_room) as f64 * 1.05).ceil() as i64 + 25
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Estimativa de custo (`estimate`): `perf` = (população, tick médio em ms).
pub fn estimate(rooms: i64, per_room: i64, perf: Option<(i64, f64)>) -> Value {
    let players = rooms * per_room;
    let conns = conn_limit(rooms, per_room);
    let bytes_worst = rooms * per_room * per_room * STATE_BYTES * TICK_HZ;
    let tick_ms = perf.filter(|(p, t)| *p >= 10 && *t > 0.0).map(|(p, t)| round1(t / p as f64 * players as f64));
    let fds = conns + 200;
    let mut notes: Vec<String> = Vec::new();
    let mut level = "ok";
    let up = |l: &'static str, level: &mut &'static str| {
        if l == "crit" || *level == "ok" {
            *level = l;
        }
    };
    let b = bytes_worst as f64;
    if b > CRIT_BPS {
        up("crit", &mut level);
        notes.push("Banda de envio no pior caso acima de 300 Mbit/s.".into());
    } else if b > WARN_BPS {
        up("warn", &mut level);
        notes.push("Banda de envio no pior caso acima de 100 Mbit/s: confira o link do servidor.".into());
    }
    match tick_ms {
        Some(t) if t > CRIT_TICK => {
            up("crit", &mut level);
            notes.push(format!("O tick projetado passa de {CRIT_TICK} ms (o orçamento é 100 ms): o jogo vai travar."));
        }
        Some(t) if t > WARN_TICK => {
            up("warn", &mut level);
            notes.push(format!("O tick projetado passa de {WARN_TICK} ms (o orçamento é 100 ms)."));
        }
        _ => {}
    }
    if fds > FD_WARN {
        up("warn", &mut level);
        notes.push(format!(
            "São necessários cerca de {fds} descritores de arquivo: confira o limite do sistema (ulimit -n / LimitNOFILE), que costuma ser 1024."
        ));
    }
    if tick_ms.is_none() {
        notes.push(
            "Custo do tick: ainda sem jogadores suficientes online para projetar (precisa de pelo menos 10).".into(),
        );
    }
    json!({
        "players": players, "conns": conns, "fds": fds, "msgsPerSec": players * TICK_HZ, "bytesWorst": bytes_worst,
        "mbitWorst": round1(b * 8.0 / 1e6), "tickMs": tick_ms, "level": level, "notes": notes
    })
}

pub fn load_file(file: &str) -> Option<(i64, i64)> {
    if file.is_empty() {
        return None;
    }
    let j: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    validate(int_of(&j["rooms"]), int_of(&j["perRoom"])).ok()
}

pub fn save_file(file: &str, rooms: i64, per_room: i64) -> bool {
    if file.is_empty() {
        return true;
    }
    crate::persist::write_json_atomic(
        Path::new(file),
        &json!({"v": 1, "savedAt": crate::now(), "rooms": rooms, "perRoom": per_room}),
        false,
    )
    .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits() {
        assert_eq!(validate(Some(60), Some(16)), Ok((60, 16)));
        assert!(validate(Some(150), Some(40)).is_err());
        assert!(validate(Some(0), Some(16)).is_err());
        assert_eq!(conn_limit(60, 16), 1033);
        assert_eq!(estimate(60, 16, None)["level"], "warn");
    }
}
