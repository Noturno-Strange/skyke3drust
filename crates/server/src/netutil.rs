//! Endereços de rede (`lib/netutil.js`): normalização, chave de limite (IPv6 por /64) e IP real atrás de proxies.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// IP normalizado em texto, ou `?` quando não é um IP.
pub fn norm_ip(value: &str) -> String {
    let mut s = value.trim();
    if s.is_empty() {
        return "?".into();
    }
    if let Some(z) = s.find('%')
        && z > 0
    {
        s = &s[..z];
    }
    if let Some(r) = s.strip_prefix("::ffff:")
        && r.parse::<Ipv4Addr>().is_ok()
    {
        s = r;
    }
    match s.parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => s.to_ascii_lowercase(),
        Ok(IpAddr::V6(_)) => s.to_ascii_lowercase(),
        Err(_) => "?".into(),
    }
}

pub fn is_ip(s: &str) -> u8 {
    match s.parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => 4,
        Ok(IpAddr::V6(_)) => 6,
        Err(_) => 0,
    }
}

/// Chave de limite: IPv4 inteiro, IPv6 pelo bloco /64.
pub fn rate_key(value: &str) -> String {
    let ip = norm_ip(value);
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(_)) => ip,
        Ok(IpAddr::V6(v6)) => {
            let g = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", g[0], g[1], g[2], g[3])
        }
        Err(_) => ip,
    }
}

/// IP do cliente: o remoto, ou, atrás de proxy confiável, o último salto não confiável do `X-Forwarded-For`.
pub fn first_client_ip(
    remote: &str,
    xff: Option<&str>,
    cf: Option<&str>,
    cf_enabled: bool,
    trusted: impl Fn(&str) -> bool,
) -> String {
    let remote_ip = norm_ip(remote);
    if is_ip(&remote_ip) == 0 || !trusted(&remote_ip) {
        return remote_ip;
    }
    if cf_enabled && let Some(c) = cf {
        let c = norm_ip(c);
        if is_ip(&c) != 0 {
            return c;
        }
    }
    let Some(xff) = xff.filter(|x| !x.is_empty()) else { return remote_ip };
    let parts: Vec<&str> = xff.split(',').collect();
    let last = norm_ip(parts[parts.len() - 1]);
    if is_ip(&last) == 0 {
        return remote_ip;
    }
    let mut fallback = last;
    for p in parts.iter().rev() {
        let ip = norm_ip(p);
        if is_ip(&ip) == 0 {
            return remote_ip;
        }
        if !trusted(&ip) {
            return ip;
        }
        fallback = ip;
    }
    fallback
}

/// Endereço roteável na internet (para links externos, como as rádios).
pub fn is_public_address(value: &str) -> bool {
    let ip = norm_ip(value);
    match ip.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => public_v4(v4),
        Ok(IpAddr::V6(v6)) => public_v6(v6),
        Err(_) => false,
    }
}

fn public_v4(v: Ipv4Addr) -> bool {
    let [a, b, c, _] = v.octets();
    if a == 0 || a == 10 || a == 127 || a >= 224 {
        return false;
    }
    if a == 100 && (64..=127).contains(&b) {
        return false;
    }
    if a == 169 && b == 254 {
        return false;
    }
    if a == 172 && (16..=31).contains(&b) {
        return false;
    }
    if a == 192 && (b == 168 || (b == 0 && (c == 0 || c == 2)) || (b == 88 && c == 99)) {
        return false;
    }
    if a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)) {
        return false;
    }
    if a == 203 && b == 0 && c == 113 {
        return false;
    }
    true
}

fn public_v6(v: Ipv6Addr) -> bool {
    let w = v.segments();
    if w.iter().all(|&x| x == 0) || (w[..7].iter().all(|&x| x == 0) && w[7] == 1) {
        return false;
    }
    if w[..5].iter().all(|&x| x == 0) && w[5] == 0xffff {
        return public_v4(Ipv4Addr::new((w[6] >> 8) as u8, w[6] as u8, (w[7] >> 8) as u8, w[7] as u8));
    }
    if w[..6].iter().all(|&x| x == 0) {
        return false;
    }
    if w[..4].iter().all(|&x| x == 0) && w[4] == 0xffff && w[5] == 0 {
        return false;
    }
    if (w[0] & 0xfe00) == 0xfc00 || (w[0] & 0xffc0) == 0xfe80 || (w[0] & 0xffc0) == 0xfec0 || (w[0] & 0xff00) == 0xff00
    {
        return false;
    }
    if (w[0] == 0x64 && w[1] == 0xff9b) || (w[0] == 0x100 && w[1] == 0 && w[2] == 0 && w[3] == 0) || w[0] == 0x2002 {
        return false;
    }
    if w[0] == 0x2001 && (w[1] < 0x200 || w[1] == 0x0db8) {
        return false;
    }
    if (w[0] == 0x3fff && w[1] < 0x1000) || w[0] == 0x5f00 {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_and_proxies() {
        assert_eq!(norm_ip("::ffff:1.2.3.4"), "1.2.3.4");
        assert_eq!(norm_ip("fe80::1%eth0"), "fe80::1");
        assert_eq!(norm_ip("x"), "?");
        assert_eq!(rate_key("2001:db8:1:2:3:4:5:6"), "2001:db8:1:2::/64");
        let t = |ip: &str| ip == "127.0.0.1";
        assert_eq!(first_client_ip("127.0.0.1", Some("9.9.9.9, 8.8.8.8"), None, false, t), "8.8.8.8");
        assert_eq!(first_client_ip("5.5.5.5", Some("8.8.8.8"), None, false, t), "5.5.5.5");
        assert!(is_public_address("8.8.8.8") && !is_public_address("192.168.0.1") && !is_public_address("::1"));
    }
}
