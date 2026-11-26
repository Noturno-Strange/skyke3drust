//! Validação de tudo o que o cliente envia (`lib/validate.js`). Cada mensagem passa por [`normalize_message`]: o que
//! não estiver no formato esperado vira `None` e é descartado. Os textos são limpos (controles, invisíveis) e cortados
//! pelo comprimento em unidades UTF-16, como `String.slice` do JavaScript.

use crate::catalog::Catalog;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

pub const MAX_CHAT: usize = 200;
pub const MAX_GCHAT: usize = 140;
pub const MAX_FRIENDS: usize = 100;
pub const MAX_SDP: usize = 12000;
pub const MAX_CANDIDATE: usize = 600;
pub const MAX_CODE: usize = 10;
pub const MAX_VIDEO_TIME: f64 = 86400.0;
pub const MAX_KDUR: f64 = 780.0;
pub const MAX_GRACE: i64 = 7 * 24 * 3_600_000;
pub const MAX_GEN: i64 = 0x7fff_ffff;
pub const CODE_ALPHABET: &str = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub const RESERVED_NAMES: [&str; 29] = [
    "karaoke",
    "sistema",
    "system",
    "servidor",
    "server",
    "admin",
    "administrador",
    "administrator",
    "moderador",
    "moderator",
    "skyke",
    "skyke3d",
    "suporte",
    "support",
    "moderacao",
    "moderadora",
    "moderadores",
    "moderation",
    "mod",
    "staff",
    "equipe",
    "oficial",
    "official",
    "dono",
    "owner",
    "administracao",
    "adm",
    "denuncias",
    "bot",
];

pub const REPORT_REASONS: [(&str, &str); 7] = [
    ("assedio", "Assédio ou ofensa"),
    ("odio", "Discurso de ódio ou discriminação"),
    ("sexual", "Conteúdo sexual ou impróprio"),
    ("spam", "Spam ou flood"),
    ("golpe", "Golpe, link malicioso ou dados pessoais"),
    ("trapaca", "Trapaça ou abuso de falha"),
    ("outro", "Outro motivo"),
];

const VOICE_Q: [&str; 5] = ["eco", "padrao", "alta", "musica", "estereo"];
const ID_ACTIONS: [&str; 6] = ["join", "rotate", "rotate-recovery", "revoke", "recover", "cancel-recovery"];
const PET_ACTIONS: [&str; 6] = ["feed", "play", "rest", "clean", "pet", "special"];
const PET_SOCIAL: [&str; 3] = ["wave", "hug", "gift"];
const FARM_ACTS: [&str; 6] = ["plant", "water", "harvest", "dig", "buy", "renew"];
const FARM_SHOP: [&str; 3] = ["land", "prop", "house"];

// ---------- texto ----------

/// Comprimento em unidades UTF-16 (`String.length`).
pub fn len16(s: &str) -> usize {
    s.encode_utf16().count()
}

/// `s.slice(0, n)` em unidades UTF-16 (um par substituto cortado ao meio fica de fora).
pub fn slice16(s: &str, n: usize) -> String {
    let mut used = 0;
    let mut out = String::new();
    for c in s.chars() {
        let l = c.len_utf16();
        if used + l > n {
            break;
        }
        used += l;
        out.push(c);
    }
    out
}

/// `Default_Ignorable_Code_Point`, fora os seletores de variação de emoji (FE0E e FE0F), mais U+2800, U+2028 e U+2029.
fn invisible(c: char) -> bool {
    let u = c as u32;
    if u == 0xFE0E || u == 0xFE0F {
        return false;
    }
    matches!(u,
        0x00AD | 0x034F | 0x061C | 0x115F..=0x1160 | 0x17B4..=0x17B5 | 0x180B..=0x180F | 0x200B..=0x200F
        | 0x202A..=0x202E | 0x2060..=0x206F | 0x3164 | 0xFE00..=0xFE0F | 0xFEFF | 0xFFA0 | 0xFFF0..=0xFFF8
        | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0000..=0xE0FFF | 0x2800 | 0x2028 | 0x2029)
}

/// Tira caracteres de controle e invisíveis.
pub fn strip(s: &str) -> String {
    s.chars().filter(|&c| !(c as u32 <= 0x1f || (0x7f..=0x9f).contains(&(c as u32)) || invisible(c))).collect()
}

/// Letra ou número (`[\p{L}\p{N}]`).
fn ln(c: char) -> bool {
    c.is_alphanumeric() && !is_combining_mark(c)
}

fn confusable(c: char) -> Option<char> {
    Some(match c {
        'а' => 'a',
        'в' => 'b',
        'е' => 'e',
        'ё' => 'e',
        'к' => 'k',
        'м' => 'm',
        'н' => 'h',
        'о' => 'o',
        'р' => 'p',
        'с' => 'c',
        'т' => 't',
        'у' => 'y',
        'х' => 'x',
        'і' => 'i',
        'ї' => 'i',
        'ј' => 'j',
        'ѕ' => 's',
        'ԁ' => 'd',
        'ԛ' => 'q',
        'ԝ' => 'w',
        'ɡ' => 'g',
        'α' => 'a',
        'β' => 'b',
        'ε' => 'e',
        'η' => 'n',
        'ι' => 'i',
        'κ' => 'k',
        'μ' => 'u',
        'ν' => 'v',
        'ο' => 'o',
        'ρ' => 'p',
        'τ' => 't',
        'υ' => 'u',
        'χ' => 'x',
        'ω' => 'w',
        'ϲ' => 'c',
        _ => return None,
    })
}

pub fn unconfuse(s: &str) -> String {
    s.chars()
        .map(|c| {
            let u = c as u32;
            if (0x0370..=0x03ff).contains(&u) || (0x0400..=0x052f).contains(&u) || u == 0x0261 {
                confusable(c).unwrap_or(c)
            } else {
                c
            }
        })
        .collect()
}

const LOOKALIKES: [(char, &str); 24] = [
    ('a', "ɑᴀɐɒⱥαаａ4@"),
    ('b', "ʙɓƀƅьβｂ8"),
    ('c', "ᴄɔƈϲϹсｃ("),
    ('d', "ɖɗᴅԁｄ"),
    ('e', "ɛɘǝᴇεеёєｅ3з"),
    ('f', "ƒꜰｆ"),
    ('g', "ɡɠɢƍｇ9"),
    ('h', "ɦʜɧһｈ"),
    ('i', "ıɪɨᵻᴉіїｉⅰǀ|!1lɭⅼℓʟłɫӏӀ"),
    ('j', "ʝᴊјｊ"),
    ('k', "ƙᴋκкｋ"),
    ('m', "ɱᴍмｍ"),
    ('n', "ɴɲɳпոｎ"),
    ('o', "ɵᴏοоօｏ0"),
    ('p', "ƥᴘρрｐ"),
    ('q', "ԛｑ"),
    ('r', "ɾɼʀгｒ"),
    ('s', "ʂꜱѕｓ5$"),
    ('t', "ƫʈᴛτтｔ7+"),
    ('u', "ʋυսｕv"),
    ('w', "ᴡʍԝωｗ"),
    ('x', "хχｘ"),
    ('y', "ʏƴуүｙ"),
    ('z', "ʐʑᴢｚ2"),
];

fn skel_char(c: char) -> char {
    static MAP: OnceLock<std::collections::HashMap<char, char>> = OnceLock::new();
    let m = MAP.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        for (l, o) in LOOKALIKES {
            for c in o.chars() {
                m.insert(c, l);
            }
        }
        m
    });
    m.get(&c).copied().unwrap_or(c)
}

/// Esqueleto do nome (sósias, acentos e separadores somem): impede "Adm1n", "ɑdmin" e afins.
pub fn skeleton(s: &str) -> String {
    let a: String = s.to_lowercase().chars().map(skel_char).collect();
    let b: String = a.nfkd().filter(|&c| !is_combining_mark(c)).map(skel_char).collect();
    b.replace("rn", "m").replace("uu", "w").chars().filter(|&c| ln(c)).collect()
}

/// Chave de comparação de nomes (sem acentos, minúsculas, só letras e números).
pub fn name_key(s: &str) -> String {
    let a: String = s.nfkd().filter(|&c| !is_combining_mark(c)).collect::<String>().to_lowercase();
    unconfuse(&a).chars().filter(|&c| ln(c)).collect()
}

fn reserved(key: &str) -> bool {
    RESERVED_NAMES.contains(&key)
}

/// Nome do jogador limpo; reservado, invisível ou vazio vira `Visitante`.
pub fn nm(s: &str, max_name: usize) -> String {
    let t: String = strip(s).chars().filter(|c| !"<>&\"'`".contains(*c)).collect();
    let name = slice16(t.trim(), max_name);
    if name.is_empty() {
        return "Visitante".into();
    }
    let visible =
        name.chars().any(|c| !c.is_whitespace() && c != '\u{FE0E}' && c != '\u{FE0F}' && !is_combining_mark(c));
    let key = name_key(&name);
    let digits: String = key
        .chars()
        .map(|c| match c {
            '0' => 'o',
            '1' => 'i',
            '3' => 'e',
            '4' => 'a',
            '5' => 's',
            '7' => 't',
            c => c,
        })
        .collect();
    static SK: OnceLock<HashSet<String>> = OnceLock::new();
    let sk = SK.get_or_init(|| RESERVED_NAMES.iter().map(|n| skeleton(n)).collect());
    if !visible || reserved(&key) || reserved(&digits) || sk.contains(&skeleton(&name)) {
        return "Visitante".into();
    }
    name
}

/// Observação de denúncia: NFC, espaços normais, até 300.
pub fn clean_note(s: &str) -> String {
    let n: String = s
        .nfc()
        .map(|c| {
            let u = c as u32;
            if u <= 0x1f
                || (0x7f..=0x9f).contains(&u)
                || (0x200b..=0x200f).contains(&u)
                || (0x2028..=0x202e).contains(&u)
                || (0x2060..=0x206f).contains(&u)
                || u == 0xfeff
            {
                ' '
            } else {
                c
            }
        })
        .collect();
    slice16(&n.split_whitespace().collect::<Vec<_>>().join(" "), 300)
}

/// Sequência de um mesmo caractere repetido mais de `keep_over` vezes vira 3 (`/(.)\1{n,}/g` → `$1$1$1`).
fn squeeze(s: &str, min_run: usize) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        let mut j = i;
        // `.` do JavaScript não casa com quebras de linha
        if c != '\n' && c != '\r' && c != '\u{2028}' && c != '\u{2029}' {
            while j < cs.len() && cs[j] == c {
                j += 1;
            }
        } else {
            j = i + 1;
        }
        let run = j - i;
        if run > min_run {
            out.extend([c, c, c]);
        } else {
            out.extend(&cs[i..j]);
        }
        i = j;
    }
    out
}

// ---------- tipos ----------

pub fn is_obj(v: &Value) -> bool {
    v.is_object()
}

pub fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

/// `Number.isInteger(v) && v >= min && v <= max`.
pub fn int(v: &Value, min: i64, max: i64) -> Option<i64> {
    let f = v.as_f64()?;
    if f.fract() != 0.0 || !f.is_finite() {
        return None;
    }
    let i = f as i64;
    (i >= min && i <= max).then_some(i)
}

fn s(v: &Value) -> Option<&str> {
    v.as_str()
}

fn re_b64(v: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&v.len()) && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

pub fn fkey(v: &str) -> bool {
    re_b64(v, 43, 43)
}
pub fn fid(v: &str) -> bool {
    v.len() == 12 && v.bytes().all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}
pub fn sig86(v: &str) -> bool {
    re_b64(v, 86, 86)
}
pub fn token_ok(v: &str) -> bool {
    re_b64(v, 43, 43)
}
fn rid_ok(v: &Value) -> Option<&str> {
    s(v).filter(|r| re_b64(r, 6, 24))
}
fn hex20(v: &Value) -> Option<&str> {
    s(v).filter(|t| t.len() == 20 && t.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
}
/// Ids de jogador do servidor (`p1`, `p2`…).
pub fn player_id(v: &Value) -> Option<&str> {
    s(v).filter(|t| t.len() >= 2 && t.len() <= 10 && t.starts_with('p') && t[1..].bytes().all(|b| b.is_ascii_digit()))
}
/// Jogadores ou bots (`p…`, `b…`).
fn actor_id(v: &Value) -> Option<&str> {
    s(v).filter(|t| {
        t.len() >= 2
            && t.len() <= 13
            && (t.starts_with('p') || t.starts_with('b'))
            && t[1..].bytes().all(|b| b.is_ascii_digit())
    })
}
/// Chave pública em base64url (`PK_RE`).
pub fn pk_ok(v: &str) -> bool {
    re_b64(v, 80, 200)
}
fn wa_b64(v: &Value, max: usize) -> bool {
    s(v).is_some_and(|t| re_b64(t, 1, max))
}
fn wa_cid(v: &Value) -> bool {
    s(v).is_some_and(|t| re_b64(t, 16, 1366))
}

pub fn video_id(v: &Value) -> Option<String> {
    s(v).filter(|t| re_b64(t, 11, 11)).map(str::to_owned)
}

pub fn norm_code(v: &Value) -> Option<String> {
    let t = s(v)?;
    let c: String = strip(t)
        .to_uppercase()
        .chars()
        .filter(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        .take(MAX_CODE)
        .collect();
    (c.len() == MAX_CODE && c.chars().all(|ch| CODE_ALPHABET.contains(ch))).then_some(c)
}

pub fn norm_token(v: &Value) -> Option<String> {
    s(v).filter(|t| token_ok(t)).map(str::to_owned)
}

/// Fica dentro da área caminhável (círculo de 10,2 m em volta da fogueira).
pub fn clamp_pos(x: Option<f64>, z: Option<f64>) -> (f64, f64) {
    const WX: f64 = 0.0;
    const WZ: f64 = 1.2;
    const WR: f64 = 10.2;
    let (Some(x), Some(z)) = (x, z) else { return (WX, 5.0) };
    let (dx, dz) = (x - WX, z - WZ);
    let d = dx.hypot(dz);
    if d > WR {
        let k = WR / if d == 0.0 { 1.0 } else { d };
        return (WX + dx * k, WZ + dz * k);
    }
    (x, z)
}

/// Visual do avatar: cada campo um inteiro na faixa do catálogo; campos removidos de versões antigas são trocados.
pub fn clean_cfg(c: &Value, cat: &Catalog) -> Option<Value> {
    if c.is_null() {
        return Some(Value::Object(cat.cr.iter().map(|(k, _)| (k.clone(), json!(0))).collect()));
    }
    let o = c.as_object()?;
    let mut out = Map::new();
    for (k, n) in &cat.cr {
        let Some(v) = o.get(k) else {
            out.insert(k.clone(), json!(0));
            continue;
        };
        if let Some(g) = cat.cfg_gone.get(k)
            && let Some(i) = int(v, i64::MIN, i64::MAX)
            && let Some(to) = g.get(&i.to_string())
        {
            out.insert(k.clone(), to.clone());
            continue;
        }
        out.insert(k.clone(), json!(int(v, 0, n - 1)?));
    }
    Some(Value::Object(out))
}

static RADIO_PORTS: OnceLock<std::sync::RwLock<Vec<u16>>> = OnceLock::new();
fn radio_ports() -> &'static std::sync::RwLock<Vec<u16>> {
    RADIO_PORTS.get_or_init(|| std::sync::RwLock::new(vec![443, 8443, 8000, 8080, 8100, 8200, 8888, 9000, 9443]))
}
pub fn set_radio_ports(list: Vec<u16>) {
    if let Ok(mut p) = radio_ports().write() {
        *p = list;
    }
}

const BLOCKED_SUFFIX: [&str; 12] = [
    ".localhost",
    ".local",
    ".internal",
    ".intranet",
    ".lan",
    ".home",
    ".corp",
    ".private",
    ".arpa",
    ".test",
    ".invalid",
    ".example",
];

/// Link de rádio aceito: https, domínio público, porta da lista, sem usuário nem senha.
pub fn stream_url(v: &Value) -> Option<String> {
    const MAX: usize = 300;
    let v = s(v)?;
    if len16(v) > MAX + 20 {
        return None;
    }
    let raw = v.trim_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if raw.is_empty()
        || len16(raw) > MAX
        || raw.chars().any(|c| (c as u32) <= 0x20 || (0x7f..=0x9f).contains(&(c as u32)) || "\"'<>`\\".contains(c))
    {
        return None;
    }
    let mut u = url::Url::parse(raw).ok()?;
    if u.scheme() != "https" || !u.username().is_empty() || u.password().is_some() {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    let host = host.trim_end_matches('.');
    if host.is_empty() || host.len() > 253 || !host.contains('.') || host.contains(':') || host.starts_with('[') {
        return None;
    }
    if host.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        || host.rsplit('.').next().is_some_and(|l| l.to_lowercase().starts_with("0x"))
    {
        return None;
    }
    if !host.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-') {
        return None;
    }
    if host == "localhost" || BLOCKED_SUFFIX.iter().any(|x| host.ends_with(x)) {
        return None;
    }
    if let Some(p) = u.port()
        && !radio_ports().read().is_ok_and(|l| l.contains(&p))
    {
        return None;
    }
    u.set_fragment(None);
    let out = u.to_string();
    (len16(&out) <= MAX).then_some(out)
}

fn with_ts(mut o: Value, ts: &Value) -> Value {
    if let Some(t) = num(ts).filter(|t| *t > 0.0 && *t < 1e14) {
        o["ts"] = json!(t);
    }
    o
}

fn clean_sig(m: &Value) -> Option<Value> {
    let d = m.get("data").filter(|d| d.is_object())?;
    let to = actor_id(&m["to"])?;
    let data = if let Some(x) = d.get("d").filter(|x| x.is_object())
        && (x["type"] == "offer" || x["type"] == "answer")
        && s(&x["sdp"]).is_some_and(|p| len16(p) <= MAX_SDP)
    {
        json!({"d": {"type": x["type"], "sdp": x["sdp"]}})
    } else if let Some(c) = d.get("c").filter(|x| x.is_object())
        && s(&c["candidate"]).is_some_and(|p| len16(p) <= MAX_CANDIDATE)
    {
        json!({"c": {
            "candidate": c["candidate"],
            "sdpMid": s(&c["sdpMid"]).map(|m| slice16(m, 32)),
            "sdpMLineIndex": int(&c["sdpMLineIndex"], 0, 9).unwrap_or(0)
        }})
    } else if d["x"] == "vq" {
        let q = s(&d["q"]).filter(|q| VOICE_Q.contains(q))?;
        json!({"x": "vq", "q": q})
    } else if d["x"] == "busy" || d["x"] == "close" {
        json!({"x": d["x"]})
    } else {
        return None;
    };
    Some(json!({"t": "sig", "to": to, "data": data}))
}

fn opt_str_ok(m: &Value, k: &str, max16: usize) -> bool {
    match m.get(k) {
        None => true,
        Some(v) => s(v).is_some_and(|t| len16(t) <= max16),
    }
}

fn name_and_keys(m: &Value, cat: &Catalog) -> Option<(String, String, Option<String>, Value)> {
    if !opt_str_ok(m, "name", 64) {
        return None;
    }
    if m.get("k").is_some_and(|k| !s(k).is_some_and(fkey)) {
        return None;
    }
    if m.get("token").is_some_and(|k| !s(k).is_some_and(token_ok)) {
        return None;
    }
    let cfg = clean_cfg(m.get("cfg").unwrap_or(&Value::Null), cat)?;
    let name = nm(s(&m["name"]).unwrap_or(""), cat.max_name);
    Some((name, s(&m["k"]).unwrap_or("").to_owned(), s(&m["token"]).map(str::to_owned), cfg))
}

/// Normaliza uma mensagem do cliente; `None` = descartar.
pub fn normalize_message(m: &Value, cat: &Catalog) -> Option<Value> {
    m.as_object()?;
    let t = s(&m["t"])?;
    let rid = || rid_ok(&m["rid"]);
    Some(match t {
        "create" => {
            let raw_style = m.get("style");
            let style = raw_style.and_then(Value::as_str).map(|x| cat.style_id(x));
            if raw_style.is_some() && !style.as_ref().is_some_and(|x| cat.styles.contains(x)) {
                return None;
            }
            if m.get("bots").is_some() && int(&m["bots"], 0, cat.max_bots).is_none() {
                return None;
            }
            let (name, k, token, cfg) = name_and_keys(m, cat)?;
            let named = name_key(&name) != "visitante";
            let mut o = json!({
                "t": "create", "style": style.filter(|x| cat.styles.contains(x)).unwrap_or_else(|| "amazora".into()),
                "bots": int(&m["bots"], 0, cat.max_bots).unwrap_or(0), "cfg": cfg, "name": name, "k": k, "named": named
            });
            if let Some(tk) = token {
                o["token"] = json!(tk);
            }
            o
        }
        "join" => {
            let code = norm_code(&m["code"])?;
            let (name, k, token, cfg) = name_and_keys(m, cat)?;
            let named = name_key(&name) != "visitante";
            let mut o = json!({"t": "join", "code": code, "cfg": cfg, "name": name, "k": k, "named": named});
            if let Some(tk) = token {
                o["token"] = json!(tk);
            }
            o
        }
        "resume" => json!({"t": "resume", "code": norm_code(&m["code"])?, "token": norm_token(&m["token"])?}),
        "leave" => json!({"t": "leave"}),
        "move" => {
            let (x, z) = (num(&m["x"])?, num(&m["z"])?);
            for k in ["ry", "y"] {
                if m.get(k).is_some() && num(&m[k]).is_none() {
                    return None;
                }
            }
            if m.get("s").is_some() && int(&m["s"], 0, 1).is_none() {
                return None;
            }
            let (x, z) = clamp_pos(Some(x), Some(z));
            json!({
                "t": "move", "x": x, "z": z, "ry": num(&m["ry"]).map_or(0.0, |r| r.clamp(-10.0, 10.0)),
                "y": num(&m["y"]).map_or(0.0, |y| y.clamp(0.0, 1.6)), "s": if int(&m["s"], 1, 1).is_some() { 1 } else { 0 }
            })
        }
        "pubp" => {
            let on = m["on"].as_bool()?;
            for k in ["bio", "like", "mood", "world"] {
                if m.get(k).is_some() && !m[k].is_string() {
                    return None;
                }
            }
            let f = |k: &str| s(&m[k]).map_or(String::new(), |x| slice16(&strip(x), 400));
            json!({"t": "pubp", "on": on, "bio": f("bio"), "like": f("like"), "mood": f("mood"), "world": slice16(&f("world"), 20)})
        }
        "gbook" => json!({"t": "gbook", "to": hex20(&m["to"])?, "text": slice16(&strip(s(&m["text"])?), 300)}),
        "gblist" => json!({"t": "gblist"}),
        "gbdel" => json!({"t": "gbdel", "id": int(&m["id"], 1, 1_000_000_000)?}),
        "petget" => json!({"t": "petget"}),
        "petadopt" => json!({"t": "petadopt", "name": slice16(&strip(s(&m["name"])?), 40)}),
        "petact" => {
            let a = s(&m["a"]).filter(|a| PET_ACTIONS.contains(a))?;
            json!({"t": "petact", "a": a, "rid": rid()?})
        }
        "petdress" => json!({"t": "petdress", "c": int(&m["c"], 0, 4)?}),
        "petrel" => {
            if m["sure"] != true {
                return None;
            }
            json!({"t": "petrel", "sure": true})
        }
        "petsocial" => {
            let k = s(&m["k"]).filter(|k| PET_SOCIAL.contains(k))?;
            json!({"t": "petsocial", "to": hex20(&m["to"])?, "k": k})
        }
        "healthget" => json!({"t": "healthget"}),
        "medcraft" | "medheal" => {
            let i = s(&m["i"]).filter(|i| cat.medicine.contains_key(*i))?;
            let r = s(&m["rid"]).filter(|r| r.starts_with("med") && re_b64(&r[3..], 6, 21))?;
            let mut o = json!({"t": t, "i": i, "rid": r});
            if t == "medheal" {
                o["to"] = json!(player_id(&m["to"])?);
            }
            o
        }
        "accountget" => match m.get("token") {
            None => json!({"t": "accountget"}),
            Some(tk) => json!({"t": "accountget", "token": norm_token(tk)?}),
        },
        "supply" => {
            let i = s(&m["i"]).filter(|i| cat.ing_ids.contains(*i))?;
            json!({"t": "supply", "i": i, "rid": rid()?})
        }
        "cookget" => json!({"t": "cookget"}),
        "cookrenew" => json!({"t": "cookrenew", "rid": rid()?}),
        "cookdo" => {
            let me = s(&m["m"]).filter(|x| cat.cook_methods.contains(*x))?;
            let g = m["g"].as_array().filter(|g| g.len() >= 2 && g.len() <= cat.cook_slots)?;
            let ids: Vec<&str> = g.iter().map(Value::as_str).collect::<Option<_>>()?;
            if !ids.iter().all(|x| cat.ing_ids.contains(*x)) || ids.iter().collect::<HashSet<_>>().len() != ids.len() {
                return None;
            }
            json!({"t": "cookdo", "m": me, "g": ids, "rid": rid()?})
        }
        "farmget" => json!({"t": "farmget"}),
        "farmdo" => {
            let k = s(&m["k"]).filter(|k| FARM_ACTS.contains(k))?;
            match k {
                "renew" => json!({"t": "farmdo", "k": "renew", "rid": rid()?}),
                "buy" => {
                    let b = s(&m["b"]).filter(|b| FARM_SHOP.contains(b))?;
                    json!({"t": "farmdo", "k": "buy", "b": b, "rid": rid()?})
                }
                _ => {
                    let i = int(&m["i"], if k == "dig" { 0 } else { -1 }, cat.farm_cells - 1)?;
                    if k == "plant" {
                        let c = s(&m["c"]).filter(|c| cat.crops.contains_key(*c))?;
                        json!({"t": "farmdo", "k": k, "i": i, "c": c, "rid": rid()?})
                    } else {
                        json!({"t": "farmdo", "k": k, "i": i, "rid": rid()?})
                    }
                }
            }
        }
        "plike" => json!({"t": "plike", "to": hex20(&m["to"])?}),
        "giftget" => json!({"t": "giftget", "seen": m["seen"] == true}),
        "giftsee" => json!({"t": "giftsee", "to": hex20(&m["to"])?}),
        "giftsend" => {
            let to = hex20(&m["to"])?;
            let k = s(&m["k"]).filter(|k| *k == "c" || *k == "r")?;
            let i = s(&m["i"])
                .filter(|i| if k == "c" { cat.crops.contains_key(*i) } else { cat.recipes.contains_key(*i) })?;
            json!({"t": "giftsend", "to": to, "k": k, "i": i, "rid": rid()?})
        }
        "giftdel" => json!({"t": "giftdel", "id": int(&m["id"], 1, 1_000_000_000)?}),
        "cfg" => json!({"t": "cfg", "cfg": clean_cfg(m.get("cfg").unwrap_or(&Value::Null), cat)?}),
        "chat" => {
            let text = squeeze(&strip(s(&m["text"])?), 9);
            let text = text.trim();
            if text.is_empty() || len16(text) > MAX_CHAT {
                return None;
            }
            let lower = text.to_lowercase();
            if lower.matches("http://").count() + lower.matches("https://").count() > 3 {
                return None;
            }
            json!({"t": "chat", "text": text})
        }
        "item" => {
            let it = &m["item"];
            if !(it.is_null() && m.get("item").is_some()) && !s(it).is_some_and(|i| cat.items.contains(i)) {
                return None;
            }
            if m.get("use").is_some() && int(&m["use"], 0, 1).is_none() {
                return None;
            }
            json!({"t": "item", "item": it, "use": if int(&m["use"], 1, 1).is_some() { 1 } else { 0 }})
        }
        "gest" => json!({"t": "gest", "g": s(&m["g"]).filter(|g| *g == "stop" || cat.gests.contains(*g))?}),
        "emo" => json!({"t": "emo", "i": int(&m["i"], 0, cat.max_emo)?}),
        "speak" => json!({"t": "speak", "v": m["v"].as_bool()?}),
        "ice" => json!({"t": "ice"}),
        "vlisten" => json!({"t": "vlisten", "v": m["v"].as_bool()?}),
        "vmute" => json!({"t": "vmute", "id": actor_id(&m["id"])?, "v": m["v"].as_bool()?}),
        "vcap" => json!({"t": "vcap", "opus": m["opus"].as_bool()?}),
        "bots" => json!({"t": "bots", "n": int(&m["n"], 0, cat.max_bots)?}),
        "sig" => clean_sig(m)?,
        "video" => json!({"t": "video", "vid": video_id(&m["vid"])?}),
        "vsync" | "ksync" => {
            let time = num(&m["time"]).filter(|x| *x >= 0.0 && *x <= MAX_VIDEO_TIME)?;
            let playing = m["playing"].as_bool()?;
            with_ts(json!({"t": t, "time": time, "playing": playing}), &m["ts"])
        }
        "report" => {
            let id = player_id(&m["id"])?;
            let reason = s(&m["reason"]).filter(|r| REPORT_REASONS.iter().any(|(k, _)| k == r))?;
            if m.get("note").is_some() && !m["note"].is_string() {
                return None;
            }
            json!({"t": "report", "id": id, "reason": reason, "note": clean_note(s(&m["note"]).unwrap_or(""))})
        }
        "tsync" => json!({"t": "tsync", "c": num(&m["c"]).filter(|c| *c > 0.0 && *c < 1e14)?}),
        "dj" => json!({"t": "dj", "url": stream_url(&m["url"])?}),
        "djstop" => json!({"t": "djstop"}),
        "vstop" => match m.get("by") {
            None => json!({"t": "vstop"}),
            Some(b) => json!({"t": "vstop", "by": player_id(b)?}),
        },
        "vend" => json!({"t": "vend"}),
        "vjoin" | "vleave" => json!({"t": t, "by": player_id(&m["by"])?}),
        "kjoin" | "ksong" => json!({"t": t, "vid": video_id(&m["vid"])?}),
        "kleave" => json!({"t": "kleave"}),
        "kpass" => json!({"t": "kpass"}),
        "kdur" => json!({"t": "kdur", "d": num(&m["d"]).filter(|d| *d >= 10.0)?.min(MAX_KDUR)}),
        "kend" => json!({"t": "kend"}),
        "friends" => {
            if m.get("k").is_some() && !s(&m["k"]).is_some_and(fkey) {
                return None;
            }
            let ids = m["ids"].as_array().filter(|a| a.len() <= MAX_FRIENDS)?;
            let ids: Vec<&str> = ids.iter().map(|x| s(x).filter(|f| fid(f))).collect::<Option<_>>()?;
            let names = match m.get("names") {
                None => None,
                Some(n) => {
                    let a = n.as_array().filter(|a| a.len() == ids.len())?;
                    let v: Vec<String> = a
                        .iter()
                        .map(|x| s(x).filter(|y| len16(y) <= 16).map(|y| slice16(&strip(y), 16)))
                        .collect::<Option<_>>()?;
                    Some(v)
                }
            };
            let mut seen = HashSet::new();
            let uniq: Vec<&str> = ids.into_iter().filter(|x| seen.insert(*x)).collect();
            let mut o = json!({"t": "friends", "ids": uniq});
            if let Some(k) = s(&m["k"]) {
                o["k"] = json!(k);
            }
            if let Some(n) = names {
                o["names"] = json!(n);
            }
            o
        }
        "freq" | "facc" | "finv" => json!({"t": t, "to": s(&m["to"]).filter(|x| fid(x))?}),
        "auth" => {
            let id = s(&m["id"]).filter(|x| fid(x))?;
            json!({"t": "auth", "id": id, "action": s(&m["action"]).filter(|a| ID_ACTIONS.contains(a))?})
        }
        "proof" => {
            let id = s(&m["id"]).filter(|x| fid(x))?;
            let action = s(&m["action"]).filter(|a| ID_ACTIONS.contains(a))?;
            json!({"t": "proof", "id": id, "action": action, "nonce": norm_token(&m["nonce"])?, "sig": s(&m["sig"]).filter(|x| sig86(x))?})
        }
        "idreg" => {
            let k = s(&m["k"]).filter(|x| fkey(x))?;
            let pk = s(&m["pk"]).filter(|x| pk_ok(x))?;
            let rpk = s(&m["rpk"]).filter(|x| pk_ok(x))?;
            let mut o = json!({"t": "idreg", "k": k, "pk": pk, "rpk": rpk});
            if let Some(p) = m.get("pow") {
                o["pow"] =
                    json!(s(p).filter(|x| !x.is_empty() && x.len() <= 12 && x.bytes().all(|b| b.is_ascii_digit()))?);
            }
            o
        }
        "idrot" => {
            let token = norm_token(&m["token"])?;
            let so = s(&m["sigOld"]).filter(|x| sig86(x))?;
            let sr = s(&m["sigRec"]).filter(|x| sig86(x))?;
            let (has_pk, has_rpk) = (m.get("pk").is_some(), m.get("rpk").is_some());
            if has_pk == has_rpk {
                return None;
            }
            if has_pk {
                let pk = s(&m["pk"]).filter(|x| pk_ok(x))?;
                json!({"t": "idrot", "token": token, "gen": int(&m["gen"], 1, MAX_GEN)?, "pk": pk, "sigOld": so, "sigRec": sr})
            } else {
                let rpk = s(&m["rpk"]).filter(|x| pk_ok(x))?;
                json!({"t": "idrot", "token": token, "rgen": int(&m["rgen"], 1, MAX_GEN)?, "rpk": rpk, "sigOld": so, "sigRec": sr})
            }
        }
        "idrec" => {
            let token = norm_token(&m["token"])?;
            let gen_ = int(&m["gen"], 1, MAX_GEN)?;
            let pk = s(&m["pk"]).filter(|x| pk_ok(x))?;
            let sr = s(&m["sigRec"]).filter(|x| sig86(x))?;
            let rpk = s(&m["rpk"]).filter(|x| pk_ok(x))?;
            let mut o = json!({"t": "idrec", "token": token, "gen": gen_, "pk": pk, "rpk": rpk, "sigRec": sr});
            if let Some(g) = m.get("grace") {
                o["grace"] = json!(int(g, 0, MAX_GRACE)?);
            }
            o
        }
        "idgrant" => {
            let tk = s(&m["token"]).filter(|x| {
                x.len() == 21
                    && x.starts_with('r')
                    && x[1..].bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })?;
            json!({"t": "idgrant", "token": tk, "rpk": s(&m["rpk"]).filter(|x| pk_ok(x))?})
        }
        "idcancel" => {
            let token = norm_token(&m["token"])?;
            json!({"t": "idcancel", "token": token, "gen": int(&m["gen"], 1, MAX_GEN)?, "sigOld": s(&m["sigOld"]).filter(|x| sig86(x))?})
        }
        "idrev" => {
            let token = norm_token(&m["token"])?;
            let epoch = int(&m["epoch"], 0, MAX_GEN)?;
            let mut o = json!({"t": "idrev", "token": token, "epoch": epoch});
            if let Some(so) = m.get("sigOld") {
                o["sigOld"] = json!(s(so).filter(|x| sig86(x))?);
            }
            o
        }
        "pkopt" => {
            if m["purpose"] == "login" {
                json!({"t": "pkopt", "purpose": "login"})
            } else if m["purpose"] == "reg" {
                json!({"t": "pkopt", "purpose": "reg", "token": norm_token(&m["token"])?})
            } else {
                return None;
            }
        }
        "pkreg" => {
            let token = norm_token(&m["token"])?;
            let challenge = norm_token(&m["challenge"])?;
            if !wa_b64(&m["cd"], 2048) || !wa_b64(&m["ad"], 2048) || !wa_b64(&m["spki"], 1200) {
                return None;
            }
            let alg = int(&m["alg"], -257, -7).filter(|a| [-7, -8, -257].contains(a))?;
            if !wa_cid(&m["cid"]) {
                return None;
            }
            let name: String = strip(s(&m["name"]).unwrap_or("")).chars().filter(|c| !"<>&\"'`".contains(*c)).collect();
            json!({
                "t": "pkreg", "token": token, "challenge": challenge, "cd": m["cd"], "ad": m["ad"], "spki": m["spki"],
                "alg": alg, "cid": m["cid"], "name": slice16(name.trim(), 40)
            })
        }
        "pklist" => json!({"t": "pklist", "token": norm_token(&m["token"])?}),
        "pkdel" => {
            let token = norm_token(&m["token"])?;
            if m["all"] == true {
                json!({"t": "pkdel", "token": token, "all": true})
            } else if wa_cid(&m["cid"]) {
                json!({"t": "pkdel", "token": token, "cid": m["cid"]})
            } else {
                return None;
            }
        }
        "pklogin" => {
            let challenge = norm_token(&m["challenge"])?;
            if !wa_cid(&m["cid"]) || !wa_b64(&m["cd"], 2048) || !wa_b64(&m["ad"], 2048) || !wa_b64(&m["sig"], 1024) {
                return None;
            }
            let pk = s(&m["pk"]).filter(|x| pk_ok(x))?;
            json!({"t": "pklogin", "challenge": challenge, "cid": m["cid"], "cd": m["cd"], "ad": m["ad"], "sig": m["sig"], "pk": pk})
        }
        "gsub" => json!({"t": "gsub", "on": m["on"].as_bool()?}),
        "gchat" => {
            let t1 = squeeze(&strip(s(&m["text"])?), 5);
            let text = t1.split_whitespace().collect::<Vec<_>>().join(" ");
            if text.is_empty() || len16(&text) > MAX_GCHAT {
                return None;
            }
            json!({"t": "gchat", "text": text})
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(nm("  Ana  ", 16), "Ana");
        assert_eq!(nm("Adm1n", 16), "Visitante");
        assert_eq!(nm("ɑdmin", 16), "Visitante");
        assert_eq!(nm("A\u{200b}na<b>", 16), "Anab");
        assert_eq!(nm("\u{FE0F}", 16), "Visitante");
        assert_eq!(nm("12345678901234567890", 16), "1234567890123456");
        assert_eq!(slice16("😀😀", 3), "😀");
    }

    #[test]
    fn messages() {
        let cat = Catalog::embedded();
        let m = normalize_message(&json!({"t": "move", "x": 50, "z": 1.2, "ry": 99}), cat).unwrap();
        assert!((m["x"].as_f64().unwrap() - 10.2).abs() < 1e-9 && m["ry"] == 10.0 && m["s"] == 0);
        assert!(normalize_message(&json!({"t": "chat", "text": "   "}), cat).is_none());
        assert_eq!(normalize_message(&json!({"t": "chat", "text": "oiiiiiiiiiiii"}), cat).unwrap()["text"], "oiii");
        let c = normalize_message(&json!({"t": "create", "name": "Bia", "bots": 3}), cat).unwrap();
        assert_eq!(c["style"], "amazora");
        assert_eq!(c["named"], true);
        assert!(normalize_message(&json!({"t": "create", "bots": 99}), cat).is_none());
        assert!(normalize_message(&json!({"t": "join", "code": "abcde-fghjk"}), cat).is_some());
        assert!(normalize_message(&json!({"t": "join", "code": "ABCDE-FGHJ1"}), cat).is_none());
        assert_eq!(
            stream_url(&json!("https://radio.exemplo.com.br:8000/x#y")).as_deref(),
            Some("https://radio.exemplo.com.br:8000/x")
        );
        assert!(stream_url(&json!("https://192.168.0.1/x")).is_none());
        assert!(stream_url(&json!("http://radio.exemplo.com/x")).is_none());
    }
}
