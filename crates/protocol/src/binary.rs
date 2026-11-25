//! Pacotes binários do protocolo original.
//!
//! * Tipo 1 — estado da sala (servidor → cliente, 5 Hz): `[u8 1]` seguido de registros de 12 bytes por jogador que mudou:
//!   `u32 id (bit alto = bot)`, `i16 x·100`, `i16 z·100`, `i16 giro·10000`, `u8 altura·100`, `u8 sentado` (little-endian).
//! * Tipo 2 — voz pelo WebSocket.
//!   - cliente → servidor: `[u8 2][u8 codec][u8 seq][dados…]`
//!   - servidor → cliente: `[u8 2][u32 id][u8 codec][u8 seq][dados…]`
//!
//!   `codec` 1 = Opus (48 kHz mono), 2 = IMA-ADPCM 4 bits a 16 kHz.
//!
//! Referência: `server.js` (`stPack`) e `assets/app-net.js` (`stUnpack`), `assets/app-voice.js` (`vwsSend`, `vwsRx`).

use sky_shared::ActorId;

pub const KIND_STATE: u8 = 1;
pub const KIND_VOICE: u8 = 2;
const STATE_RECORD: usize = 12;

/// `Math.round` do JavaScript: metade arredonda para +∞ (diferente de `f32::round`).
fn js_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

fn i16_sat(v: f64) -> i16 {
    js_round(v).clamp(f64::from(i16::MIN), f64::from(i16::MAX)) as i16
}

/// Posição de um ator num pacote de estado.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ActorState {
    pub id: ActorId,
    pub x: f32,
    pub z: f32,
    /// Giro em radianos, em (-π, π].
    pub ry: f32,
    /// Altura acima do chão (0 a 1,6 m; o pacote guarda até 2,55).
    pub y: f32,
    pub sitting: bool,
}

/// Monta um pacote de estado exatamente como o servidor Node.js.
pub fn encode_state(states: &[ActorState]) -> Vec<u8> {
    let mut b = Vec::with_capacity(1 + states.len() * STATE_RECORD);
    b.push(KIND_STATE);
    for s in states {
        b.extend_from_slice(&s.id.to_wire().to_le_bytes());
        b.extend_from_slice(&i16_sat(f64::from(s.x) * 100.0).to_le_bytes());
        b.extend_from_slice(&i16_sat(f64::from(s.z) * 100.0).to_le_bytes());
        let ry = f64::from(s.ry);
        let wrapped = ry.sin().atan2(ry.cos());
        b.extend_from_slice(&i16_sat(wrapped * 10000.0).to_le_bytes());
        b.push(js_round(f64::from(s.y) * 100.0).clamp(0.0, 255.0) as u8);
        b.push(u8::from(s.sitting));
    }
    b
}

/// Lê um pacote de estado. Devolve `None` se não for do tipo 1; registros incompletos no fim são ignorados
/// (como no cliente original).
pub fn decode_state(buf: &[u8]) -> Option<Vec<ActorState>> {
    let (&kind, body) = buf.split_first()?;
    if kind != KIND_STATE {
        return None;
    }
    let out = body
        .chunks_exact(STATE_RECORD)
        .map(|r| {
            let u = u32::from_le_bytes([r[0], r[1], r[2], r[3]]);
            let i = |o: usize| f32::from(i16::from_le_bytes([r[o], r[o + 1]]));
            ActorState {
                id: ActorId::from_wire(u),
                x: i(4) / 100.0,
                z: i(6) / 100.0,
                ry: i(8) / 10000.0,
                y: f32::from(r[10]) / 100.0,
                sitting: r[11] == 1,
            }
        })
        .collect();
    Some(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VoiceCodec {
    Opus = 1,
    Adpcm = 2,
}

impl VoiceCodec {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Opus),
            2 => Some(Self::Adpcm),
            _ => None,
        }
    }
}

/// Quadro de voz enviado pelo cliente.
pub fn encode_voice_out(codec: VoiceCodec, seq: u8, data: &[u8]) -> Vec<u8> {
    let mut b = Vec::with_capacity(3 + data.len());
    b.extend_from_slice(&[KIND_VOICE, codec as u8, seq]);
    b.extend_from_slice(data);
    b
}

/// Quadro de voz recebido do servidor (já com o id de quem fala).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoiceIn<'a> {
    pub from: ActorId,
    pub codec: VoiceCodec,
    pub seq: u8,
    pub data: &'a [u8],
}

/// O cliente original descarta quadros com menos de 8 bytes.
pub fn decode_voice_in(buf: &[u8]) -> Option<VoiceIn<'_>> {
    if buf.len() < 8 || buf[0] != KIND_VOICE {
        return None;
    }
    let u = u32::from_le_bytes([buf[1], buf[2], buf[3], buf[4]]);
    Some(VoiceIn {
        from: ActorId::from_wire(u & 0x7fff_ffff),
        codec: VoiceCodec::from_u8(buf[5])?,
        seq: buf[6],
        data: &buf[7..],
    })
}

/// Tipo de um quadro binário qualquer.
pub fn kind(buf: &[u8]) -> Option<u8> {
    buf.first().copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_roundtrip() {
        let s = vec![
            ActorState { id: ActorId { bot: false, n: 3 }, x: 1.23, z: -4.56, ry: 3.0, y: 0.4, sitting: true },
            ActorState { id: ActorId { bot: true, n: 1 }, x: -10.2, z: 11.0, ry: -1.5, y: 0.0, sitting: false },
        ];
        let b = encode_state(&s);
        assert_eq!(b.len(), 1 + 24);
        let d = decode_state(&b).unwrap();
        for (a, b) in s.iter().zip(&d) {
            assert_eq!(a.id, b.id);
            assert!((a.x - b.x).abs() < 0.01);
            assert!((a.z - b.z).abs() < 0.01);
            assert!((a.ry - b.ry).abs() < 0.0002);
            assert!((a.y - b.y).abs() < 0.01);
            assert_eq!(a.sitting, b.sitting);
        }
    }

    /// Bytes gerados pelo `stPack` do servidor Node.js para
    /// `[['p3', 1.23, -4.56, 3, 0.4, 1], ['b1', -10.2, 11, 7, 0, 0]]`.
    #[test]
    fn state_matches_node_bytes() {
        let node: [u8; 25] = [1, 3, 0, 0, 0, 123, 0, 56, 254, 48, 117, 40, 1, 1, 0, 0, 128, 4, 252, 76, 4, 0, 28, 0, 0];
        let s = [
            ActorState { id: ActorId { bot: false, n: 3 }, x: 1.23, z: -4.56, ry: 3.0, y: 0.4, sitting: true },
            ActorState { id: ActorId { bot: true, n: 1 }, x: -10.2, z: 11.0, ry: 7.0, y: 0.0, sitting: false },
        ];
        assert_eq!(encode_state(&s), node);
    }

    #[test]
    fn state_rejects_other_kinds_and_ignores_tail() {
        assert!(decode_state(&[]).is_none());
        assert!(decode_state(&[2, 0, 0]).is_none());
        assert_eq!(decode_state(&[1, 0, 0, 0]).unwrap().len(), 0);
    }

    #[test]
    fn voice_frames() {
        assert_eq!(encode_voice_out(VoiceCodec::Opus, 9, &[7, 8]), vec![2, 1, 9, 7, 8]);
        let rx = [2, 5, 0, 0, 0, 1, 4, 0xaa, 0xbb];
        let v = decode_voice_in(&rx).unwrap();
        assert_eq!(v.from.to_string(), "p5");
        assert_eq!(v.codec, VoiceCodec::Opus);
        assert_eq!(v.seq, 4);
        assert_eq!(v.data, &[0xaa, 0xbb]);
        assert!(decode_voice_in(&rx[..7]).is_none());
    }
}
