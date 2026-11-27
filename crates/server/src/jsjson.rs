//! JSON com os números do JavaScript: `JSON.stringify(50)` dá `50`, mas o `serde_json` escreve um `f64` inteiro como
//! `50.0`. O navegador aceita os dois, o cliente nativo (que lê inteiros como inteiros) não: um `"lo": 50.0` virava 0 e
//! a barra de nível da fazenda mostrava "0 / 0 XP". Tudo o que o servidor manda passa por aqui.

use serde_json::{Number, Value};

/// Maior inteiro exato de um `f64` (`Number.MAX_SAFE_INTEGER`).
const SAFE: f64 = 9_007_199_254_740_991.0;

fn whole(n: &Number) -> Option<i64> {
    let f = n.as_f64()?;
    if n.is_f64() && f.fract() == 0.0 && f.abs() <= SAFE { Some(f as i64) } else { None }
}

fn needs(v: &Value) -> bool {
    match v {
        Value::Number(n) => whole(n).is_some(),
        Value::Array(a) => a.iter().any(needs),
        Value::Object(o) => o.values().any(needs),
        _ => false,
    }
}

fn fix(v: &mut Value) {
    match v {
        Value::Number(n) => {
            if let Some(i) = whole(n) {
                // -0 também vira 0, como no JSON.stringify
                *v = Value::from(i);
            }
        }
        Value::Array(a) => a.iter_mut().for_each(fix),
        Value::Object(o) => o.values_mut().for_each(fix),
        _ => {}
    }
}

/// Texto JSON como o `JSON.stringify` do Node escreveria.
pub fn to_string(v: &Value) -> String {
    if needs(v) {
        let mut c = v.clone();
        fix(&mut c);
        c.to_string()
    } else {
        v.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn whole_floats_are_written_as_integers() {
        let v = json!({"xp": 64.0, "lo": 50.0, "hi": 150.0, "r": 0.5, "z": -0.0, "n": [1.0, 2.5], "s": "1.0", "big": 1e300});
        assert_eq!(to_string(&v), r#"{"xp":64,"lo":50,"hi":150,"r":0.5,"z":0,"n":[1,2.5],"s":"1.0","big":1e+300}"#);
        assert_eq!(to_string(&json!({"a": 1})), r#"{"a":1}"#);
    }
}
