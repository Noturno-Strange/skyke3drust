//! HTTP e WebSocket (a parte de rede de `server.js`): páginas e arquivos do cliente, `/ticket`, `/launcher.json`,
//! `/status`, e o handshake do WebSocket com as mesmas conferências (host e origem, banimento, ticket, limites por IP).

use crate::game::{Game, Link, Out};
use crate::netutil::{first_client_ip, rate_key};
use crate::web::{Site, etag_for, pick};
use axum::body::Body;
use axum::extract::ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade};
use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::{HeaderMap, Method, Request, Response};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// Estado compartilhado pelo servidor HTTP.
#[derive(Debug)]
pub struct App {
    pub game: Mutex<Game>,
    pub site: Site,
    pub origins: Vec<String>,
    pub allowed_hosts: Vec<String>,
    pub trust: Vec<(IpAddr, u8)>,
    pub trust_cf: bool,
    pub launcher_info: String,
    pub status_token: String,
}

fn lock(app: &App) -> std::sync::MutexGuard<'_, Game> {
    app.game.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------- endereços, host e origem ----------

/// `TRUST_PROXY`: endereços e redes (CIDR) dos proxies confiáveis.
pub fn parse_trust(raw: &str) -> Vec<(IpAddr, u8)> {
    let mut out = Vec::new();
    for s in raw.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let (a, bits) = match s.split_once('/') {
            Some((a, b)) => (a, b.parse::<u8>().ok()),
            None => (s, None),
        };
        match a.parse::<IpAddr>() {
            Ok(ip) => {
                let max = if ip.is_ipv4() { 32 } else { 128 };
                let b = match bits {
                    Some(b) if b <= max => b,
                    Some(_) => {
                        tracing::warn!("TRUST_PROXY ignorado: {s}");
                        continue;
                    }
                    None if s.contains('/') => {
                        tracing::warn!("TRUST_PROXY ignorado: {s}");
                        continue;
                    }
                    None => max,
                };
                out.push((ip, b));
            }
            Err(_) => tracing::warn!("TRUST_PROXY ignorado: {s}"),
        }
    }
    out
}

fn in_net(ip: IpAddr, net: (IpAddr, u8)) -> bool {
    match (ip, net.0) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let m = if net.1 == 0 { 0 } else { u32::MAX << (32 - u32::from(net.1)) };
            u32::from(a) & m == u32::from(b) & m
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let m = if net.1 == 0 { 0 } else { u128::MAX << (128 - u32::from(net.1)) };
            u128::from(a) & m == u128::from(b) & m
        }
        _ => false,
    }
}

impl App {
    fn trusted(&self, ip: &str) -> bool {
        ip.parse::<IpAddr>().is_ok_and(|a| self.trust.iter().any(|n| in_net(a, *n)))
    }

    /// IP do cliente (atrás de proxy confiável, o do `X-Forwarded-For`).
    pub fn ip_of(&self, remote: &SocketAddr, h: &HeaderMap) -> String {
        first_client_ip(
            &remote.ip().to_string(),
            one_header(h, "x-forwarded-for").as_deref(),
            one_header(h, "cf-connecting-ip").as_deref(),
            self.trust_cf,
            |ip| self.trusted(ip),
        )
    }

    fn host_allowed(&self, h: &Option<(String, Option<u16>)>, prod: bool) -> bool {
        let Some((host, port)) = h else { return false };
        if self.allowed_hosts.contains(&host_key(host, *port)) {
            return true;
        }
        !prod && dev_host_allowed(host)
    }
}

/// O valor do cabeçalho quando ele aparece exatamente uma vez.
pub fn one_header(h: &HeaderMap, name: &str) -> Option<String> {
    let mut it = h.get_all(name).iter();
    let first = it.next()?;
    if it.next().is_some() {
        return None;
    }
    first.to_str().ok().map(str::to_owned)
}

/// Origem (`parseOrigin`): esquema, host minúsculo e porta.
pub fn parse_origin(raw: Option<&str>) -> Option<(String, String, String, Option<u16>)> {
    let s = raw?;
    if s.is_empty() || s.trim() != s || s == "null" || s.ends_with('/') {
        return None;
    }
    let u = url::Url::parse(s).ok()?;
    if u.scheme() != "http" && u.scheme() != "https" {
        return None;
    }
    if u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    Some((u.origin().ascii_serialization(), u.scheme().to_owned(), host, u.port()))
}

/// Host do cabeçalho `Host` (`parseHost`): 80 e 443 viram "sem porta".
pub fn parse_host(raw: Option<&str>) -> Option<(String, Option<u16>)> {
    let s = raw?;
    if s.is_empty() || s.trim() != s || s.chars().any(|c| c.is_whitespace() || "/\\@?#".contains(c)) {
        return None;
    }
    let u = url::Url::parse(&format!("http://{s}")).ok()?;
    if u.path() != "/"
        || u.query().is_some()
        || u.fragment().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
    {
        return None;
    }
    let host = u.host_str()?.to_lowercase();
    if host.is_empty() {
        return None;
    }
    let port = u.port().filter(|p| *p != 80 && *p != 443);
    Some((host, port))
}

pub fn host_key(host: &str, port: Option<u16>) -> String {
    match port {
        Some(p) => format!("{host}:{p}"),
        None => host.to_owned(),
    }
}

fn dev_host_allowed(host: &str) -> bool {
    let mut h = host.to_lowercase().trim_start_matches('[').trim_end_matches(']').trim_end_matches('.').to_owned();
    if h == "localhost" || h.ends_with(".localhost") || h.ends_with(".ts.net") {
        return true;
    }
    if let Some(r) = h.strip_prefix("::ffff:")
        && r.parse::<std::net::Ipv4Addr>().is_ok()
    {
        h = r.to_owned();
    }
    match h.parse::<IpAddr>() {
        Ok(IpAddr::V4(v)) => {
            let p = v.octets();
            p[0] == 127
                || p[0] == 10
                || (p[0] == 172 && (16..=31).contains(&p[1]))
                || (p[0] == 192 && p[1] == 168)
                || (p[0] == 100 && (64..=127).contains(&p[1]))
                || (p[0] == 169 && p[1] == 254)
        }
        Ok(IpAddr::V6(_)) => h == "::1" || h.starts_with("fc") || h.starts_with("fd") || h.starts_with("fe80:"),
        Err(_) => false,
    }
}

fn origin_allowed(
    app: &App,
    o: &Option<(String, String, String, Option<u16>)>,
    h: &Option<(String, Option<u16>)>,
    prod: bool,
) -> bool {
    let (Some((origin, scheme, ohost, oport)), Some((host, port))) = (o, h) else { return false };
    if prod && scheme != "https" {
        return false;
    }
    if app.origins.contains(origin) {
        return true;
    }
    let np = |p: Option<u16>| p.filter(|p| *p != 80 && *p != 443);
    ohost == host && np(*oport) == np(*port)
}

fn query_param(uri: &str, name: &str) -> Option<String> {
    let (_, q) = uri.split_once('?')?;
    for part in q.split('&') {
        if let Some((k, v)) = part.split_once('=')
            && k == name
            && !k.is_empty()
        {
            return url::form_urlencoded::parse(format!("x={v}").as_bytes()).next().map(|(_, v)| v.into_owned());
        }
    }
    None
}

// ---------- respostas ----------

fn resp(site: &Site, status: u16, ctype: &str, body: Vec<u8>, extra: &[(&str, String)], head: bool) -> Response<Body> {
    let mut b = Response::builder().status(status);
    for (k, v) in site.headers() {
        if k == "Content-Type" {
            continue;
        }
        b = b.header(k, v);
    }
    b = b.header("Content-Type", ctype);
    for (k, v) in extra {
        b = b.header(*k, v.as_str());
    }
    if head && status != 304 {
        // HEAD diz o tamanho do corpo que o GET mandaria
        b = b.header("Content-Length", body.len());
    }
    b.body(if head { Body::empty() } else { Body::from(body) }).unwrap_or_default()
}

fn plain(site: &Site, status: u16, text: &str, extra: &[(&str, String)], head: bool) -> Response<Body> {
    resp(site, status, "text/plain; charset=utf-8", text.as_bytes().to_vec(), extra, head)
}

fn json_resp(site: &Site, status: u16, v: &Value, head: bool) -> Response<Body> {
    resp(
        site,
        status,
        "application/json; charset=utf-8",
        v.to_string().into_bytes(),
        &[("Cache-Control", "no-store".into()), ("X-Robots-Tag", "noindex".into())],
        head,
    )
}

fn send_cached(site: &Site, h: &HeaderMap, a: &crate::web::Asset, head: bool) -> Response<Body> {
    let accept = h.get("accept-encoding").and_then(|v| v.to_str().ok()).unwrap_or("");
    let (enc, buf) = pick(&a.v, accept);
    let etag = etag_for(&a.etag, enc);
    let inm = h.get("if-none-match").and_then(|v| v.to_str().ok()).unwrap_or("");
    let mut extra =
        vec![("Cache-Control", a.cache.clone()), ("ETag", etag.clone()), ("Vary", "Accept-Encoding".into())];
    if inm.split(',').any(|x| x.trim() == etag) {
        return resp(site, 304, &a.ctype, Vec::new(), &extra, true);
    }
    if !enc.is_empty() {
        extra.push(("Content-Encoding", enc.into()));
    }
    resp(site, 200, &a.ctype, buf.to_vec(), &extra, head)
}

/// Todas as requisições HTTP (como o `onRequest` do original).
pub async fn handle(app: Arc<App>, remote: SocketAddr, req: Request<Body>) -> Response<Body> {
    let h = req.headers().clone();
    let ip = rate_key(&app.ip_of(&remote, &h));
    let method = req.method().clone();
    let head = method == Method::HEAD;
    let site = &app.site;
    let uri = req.uri().to_string();
    let path = req.uri().path().to_owned();
    let is_ws = h.get("upgrade").and_then(|v| v.to_str().ok()).is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
    if is_ws && method == Method::GET {
        return upgrade(app.clone(), ip, h, uri, req).await;
    }
    {
        let mut g = lock(&app);
        let ban = g.ban_check(&ip);
        if ban > 0 {
            g.abuse.r429 += 1;
            return plain(site, 429, "429", &[("Retry-After", ban.div_ceil(1000).max(1).to_string())], head);
        }
        let hr = g.set.http_rate;
        if hr > 0 && g.over(&ip, "http", hr, 60000) {
            g.abuse.r429 += 1;
            return plain(site, 429, "429", &[("Retry-After", "60".into())], head);
        }
    }
    if path.starts_with("/status/") && path != "/status/players.json" && path != "/status/reports.json" {
        let known = [
            "/status/ban",
            "/status/unban",
            "/status/capacity",
            "/status/report",
            "/status/badge",
            "/status/keypass",
            "/status/recover",
        ];
        if known.contains(&path.as_str()) {
            if method != Method::POST {
                return plain(site, 405, "405", &[("Allow", "POST".into())], head);
            }
            return status_action(&app, &ip, &path, &h, req).await;
        }
    }
    if path == "/radio" && method != Method::GET {
        return plain(site, 405, "405", &[("Allow", "GET".into())], head);
    }
    if method != Method::GET && method != Method::HEAD {
        return plain(site, 405, "405", &[("Allow", "GET, HEAD".into())], head);
    }
    match path.as_str() {
        "/launcher.json" => resp(
            site,
            200,
            "application/json; charset=utf-8",
            app.launcher_info.clone().into_bytes(),
            &[("Cache-Control", "no-cache".into())],
            head,
        ),
        "/ticket" => {
            let mut g = lock(&app);
            if g.set.ticket_mode == "off" {
                return plain(site, 404, "404", &[], head);
            }
            let tr = g.set.ticket_rate;
            if tr > 0 && g.over(&ip, "tk", tr, 60000) {
                g.abuse.r429 += 1;
                return plain(site, 429, "429", &[("Retry-After", "60".into())], head);
            }
            let bits = ticket_bits(&g, &ip);
            let t = crate::admission::issue(&g.ticket_secret, &ip, bits, 60000, crate::now());
            resp(
                site,
                200,
                "application/json; charset=utf-8",
                t.to_string().into_bytes(),
                &[("Cache-Control", "no-store".into())],
                head,
            )
        }
        "/status" | "/status.json" | "/status/players.json" | "/status/reports.json" => {
            status(&app, &ip, &path, &h, head)
        }
        "/" | "/index.html" => send_cached(site, &h, &site.index, head),
        "/privacidade" | "/termos" => resp(
            site,
            200,
            "text/html; charset=utf-8",
            site.privacy_html.clone(),
            &[("Cache-Control", "no-cache".into())],
            head,
        ),
        "/profile" => resp(
            site,
            200,
            "text/html; charset=utf-8",
            site.profile_html.clone(),
            &[("Cache-Control", "no-cache".into()), ("X-Robots-Tag", "noindex".into())],
            head,
        ),
        "/fazenda" => resp(
            site,
            200,
            "text/html; charset=utf-8",
            site.farm_html.clone(),
            &[("Cache-Control", "no-cache".into()), ("X-Robots-Tag", "noindex".into())],
            head,
        ),
        "/vendor/three-r186.min.js" => send_cached(site, &h, &site.three, head),
        "/radio" => radio(&app, &ip, &uri, &method, head).await,
        "/farm.json" | "/profile.json" => {
            let mut g = lock(&app);
            let key = if path == "/farm.json" { "farm" } else { "prof" };
            if g.over(&ip, key, 60, 60000) {
                g.abuse.r429 += 1;
                return plain(site, 429, "429", &[("Retry-After", "60".into())], head);
            }
            let id = query_param(&uri, "id").unwrap_or_default();
            let v = if path == "/farm.json" { g.farm_json(&id) } else { g.profile_json(&id) };
            let (status, body) = match v {
                Some(v) => (200, v),
                None => (404, json!({"ok": false})),
            };
            json_resp(site, status, &body, head)
        }
        p => {
            if let Some(a) = site.pwa.get(p).or_else(|| site.assets.get(p)) {
                return send_cached(site, &h, a, head);
            }
            plain(site, 404, "404", &[], head)
        }
    }
}

/// Dificuldade da prova do ticket: zero em regime normal, sobe sob pressão.
fn ticket_bits(g: &Game, ip: &str) -> u32 {
    if g.set.ticket_mode != "enforce" || g.set.pow_max == 0 {
        return 0;
    }
    let pedidos = g.count_hits(ip, "tk", 60000);
    let entradas = g.count_hits(ip, "cr", 60000) + g.count_hits(ip, "jn", 60000);
    if pedidos >= 12 || entradas >= 12 {
        return g.set.pow_max.min(16);
    }
    if pedidos >= 6 || entradas >= 6 {
        return g.set.pow_max.min(14);
    }
    let gr = g.set.global_rate;
    if gr > 0 && g.count_hits("@", "adm", 60000) >= ((gr as f64 * 0.6) as usize).max(1) {
        return g.set.pow_max.min(12);
    }
    0
}

fn status(app: &App, ip: &str, path: &str, h: &HeaderMap, head: bool) -> Response<Body> {
    let site = &app.site;
    if app.status_token.len() < 12 {
        return plain(site, 404, "404", &[], head);
    }
    if path == "/status" {
        return resp(
            site,
            200,
            "text/html; charset=utf-8",
            site.status_html.clone(),
            &[("Cache-Control", "no-store".into()), ("X-Robots-Tag", "noindex".into())],
            head,
        );
    }
    let mut g = lock(app);
    if let Err(r) = status_auth(app, &mut g, ip, h) {
        return r;
    }
    if path == "/status/players.json" && !head {
        let body = g.mod_players(ip).to_string();
        let gz = h
            .get("accept-encoding")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|a| a.split(',').any(|x| x.trim().split(';').next() == Some("gzip")));
        let mut r = json_resp(site, 200, &Value::Null, false);
        if body.len() > 2048 && gz {
            use std::io::Write;
            let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            if e.write_all(body.as_bytes()).is_ok()
                && let Ok(z) = e.finish()
            {
                *r.body_mut() = Body::from(z);
                r.headers_mut().insert("Content-Encoding", axum::http::HeaderValue::from_static("gzip"));
                r.headers_mut().insert("Vary", axum::http::HeaderValue::from_static("Accept-Encoding"));
                return r;
            }
        }
        *r.body_mut() = Body::from(body);
        return r;
    }
    let v = match path {
        "/status/players.json" => g.mod_players(ip),
        "/status/reports.json" => g.reports_json(),
        _ => g.stats(),
    };
    json_resp(site, 200, &v, head)
}

/// Confere o token do painel; `Err` já é a resposta (401 ou 429).
#[allow(clippy::result_large_err)]
fn status_auth(app: &App, g: &mut Game, ip: &str, h: &HeaderMap) -> Result<(), Response<Body>> {
    let site = &app.site;
    if g.blocked(ip, "sf", 6, 600_000) {
        return Err(json_resp(site, 429, &json!({"erro": "muitas tentativas"}), false));
    }
    let auth = h.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("");
    let t = auth.strip_prefix("Bearer ").unwrap_or("");
    use hmac::{Hmac, Mac};
    let hm = |x: &str| {
        let mut m = Hmac::<sha2::Sha256>::new_from_slice(b"skyke3d-status").unwrap_or_else(|_| unreachable!());
        m.update(x.as_bytes());
        m.finalize().into_bytes().to_vec()
    };
    if hm(t) != hm(&app.status_token) {
        g.over(ip, "sf", 999, 600_000);
        return Err(json_resp(site, 401, &json!({"erro": "não autorizado"}), false));
    }
    Ok(())
}

async fn status_action(app: &App, ip: &str, path: &str, h: &HeaderMap, req: Request<Body>) -> Response<Body> {
    let site = &app.site;
    if app.status_token.len() < 12 {
        return plain(site, 404, "404", &[], false);
    }
    {
        let mut g = lock(app);
        if let Err(r) = status_auth(app, &mut g, ip, h) {
            return r;
        }
    }
    let ct = h.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_lowercase();
    if !ct.starts_with("application/json") || ct[16..].starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_') {
        return json_resp(site, 415, &json!({"erro": "use application/json"}), false);
    }
    let big = || {
        let mut r = json_resp(site, 413, &json!({"erro": "corpo grande demais"}), false);
        r.headers_mut().insert("Connection", axum::http::HeaderValue::from_static("close"));
        r
    };
    if h.get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > 2048)
    {
        return big();
    }
    let bytes = match axum::body::to_bytes(req.into_body(), 2048).await {
        Ok(b) => b,
        Err(e) => {
            if e.to_string().to_lowercase().contains("length limit") {
                return big();
            }
            return json_resp(site, 400, &json!({"erro": "JSON inválido"}), false);
        }
    };
    let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
        return json_resp(site, 400, &json!({"erro": "JSON inválido"}), false);
    };
    if !body.is_object() {
        return json_resp(site, 400, &json!({"erro": "JSON inválido"}), false);
    }
    let mut g = lock(app);
    let preview = path == "/status/capacity" && body["dry"] == true;
    if if preview { g.over(ip, "sp", 240, 60000) } else { g.over(ip, "sa", 30, 60000) } {
        g.abuse.r429 += 1;
        return json_resp(site, 429, &json!({"erro": "muitas ações, aguarde um minuto"}), false);
    }
    let trusted = |x: &str| app.trusted(x);
    let (code, out) = g.status_action(path, &body, ip, &trusted);
    json_resp(site, code, &out, false)
}

// ---------- WebSocket ----------

async fn upgrade(app: Arc<App>, ip: String, h: HeaderMap, uri: String, req: Request<Body>) -> Response<Body> {
    let host = parse_host(one_header(&h, "host").as_deref());
    let origin_raw = one_header(&h, "origin");
    let origin = parse_origin(origin_raw.as_deref());
    let page_origin = origin.as_ref().map(|o| o.0.clone()).unwrap_or_default();
    let reject =
        |code: u16, msg: &str| Response::builder().status(code).body(Body::from(msg.to_owned())).unwrap_or_default();
    {
        let mut g = lock(&app);
        let prod = g.set.prod;
        if uri.len() > 2048 {
            return reject(414, "url");
        }
        if !app.host_allowed(&host, prod) || !origin_allowed(&app, &origin, &host, prod) {
            g.abuse.r403 += 1;
            let bad = g.hit_count(&ip, "hsbad", 30, 60000);
            if bad.count == 20 {
                g.ban_strike(&ip, "origem");
            }
            if bad.over {
                g.abuse.r429 += 1;
                return reject(429, "limite");
            }
            return reject(403, "origem negada");
        }
        if g.ban_check(&ip) > 0 {
            g.abuse.r_ban += 1;
            return reject(429, "banido");
        }
        if g.over(&ip, "hs", 120, 60000) {
            g.abuse.r429 += 1;
            return reject(429, "limite");
        }
        if g.set.ticket_mode != "off" {
            let param = query_param(&uri, "ticket").unwrap_or_default();
            let secret = g.ticket_secret;
            let now = crate::now();
            let r = crate::admission::verify(&secret, &param, &ip, now, 120_000, Some(&mut g.ticket_used));
            if let Err(why) = r {
                g.abuse.ticket += 1;
                if why == "pow" {
                    g.abuse.pow += 1;
                }
                if g.set.ticket_mode == "enforce" {
                    if g.hit_count(&ip, "tkf", 10, 60000).count == 11 {
                        g.ban_strike(&ip, "ticket");
                    }
                    return reject(403, "ticket");
                }
            }
        }
        let mine = g.conns_per_ip.get(&ip).copied().unwrap_or(0);
        if mine >= g.set.perip || g.conns.len() as i64 >= g.max_conn() {
            g.abuse.r429 += 1;
            return reject(429, "limite");
        }
        let gr = g.set.global_rate;
        if gr > 0 && g.over(&ip, "adm1", (gr / 10).max(4), 60000) {
            g.abuse.r429 += 1;
            return reject(429, "limite");
        }
        if gr > 0 && g.over("@", "adm", gr, 60000) {
            g.abuse.r429 += 1;
            return reject(429, "limite");
        }
    }
    let (mut parts, _body) = req.into_parts();
    let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &()).await {
        Ok(w) => w,
        Err(e) => return axum::response::IntoResponse::into_response(e),
    };
    ws.max_message_size(32 * 1024).max_frame_size(32 * 1024).on_upgrade(move |sock| run_ws(app, sock, ip, page_origin))
}

async fn run_ws(app: Arc<App>, sock: WebSocket, ip: String, origin: String) {
    let (mut sink, mut stream) = sock.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Out>();
    let buffered = Arc::new(AtomicUsize::new(0));
    let cid = lock(&app).open(&ip, &origin, Link { tx, buffered: buffered.clone() });
    let buf2 = buffered.clone();
    let writer = tokio::spawn(async move {
        while let Some(o) = rx.recv().await {
            let r = match o {
                Out::Text(s) => {
                    let n = s.len();
                    let r = sink.send(Message::Text(s.as_ref().into())).await;
                    buf2.fetch_sub(n.min(buf2.load(Ordering::Relaxed)), Ordering::Relaxed);
                    r
                }
                Out::Bin(b) => {
                    let n = b.len();
                    let r = sink.send(Message::Binary(b.to_vec().into())).await;
                    buf2.fetch_sub(n.min(buf2.load(Ordering::Relaxed)), Ordering::Relaxed);
                    r
                }
                Out::Ping => sink.send(Message::Ping(Vec::new().into())).await,
                Out::Close(code, reason) => {
                    let _ = sink.send(Message::Close(Some(CloseFrame { code, reason: reason.into() }))).await;
                    // depois do fechamento educado não sai mais nada; o leitor espera o eco do cliente
                    continue;
                }
                Out::Kill => break,
            };
            if r.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });
    let mut code: Option<u16> = None;
    loop {
        let msg = tokio::select! {
            m = stream.next() => m,
            _ = tokio::time::sleep(std::time::Duration::from_secs(1)), if writer.is_finished() => None,
        };
        let Some(msg) = msg else { break };
        match msg {
            Ok(Message::Text(t)) => lock(&app).on_text(cid, t.as_str()),
            Ok(Message::Binary(b)) => lock(&app).on_binary(cid, &b),
            Ok(Message::Pong(_)) => {
                let mut g = lock(&app);
                if let Some(c) = g.conns.get_mut(&cid) {
                    c.alive = true;
                }
            }
            Ok(Message::Ping(_)) => {}
            Ok(Message::Close(f)) => {
                code = Some(f.map_or(1005, |f| f.code));
                break;
            }
            Err(e) => {
                // mensagem maior que o limite: o `ws` do original fecha com 1009
                if e.to_string().to_lowercase().contains("too long") || e.to_string().to_lowercase().contains("size") {
                    code = Some(1009);
                    let link = { lock(&app).conns.get(&cid).and_then(|c| c.link.clone()) };
                    if let Some(l) = link {
                        let _ = l.tx.send(Out::Close(1009, String::new()));
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                break;
            }
        }
    }
    {
        let mut g = lock(&app);
        let ours = g.conns.get(&cid).and_then(|c| c.closing);
        let final_code = code.filter(|c| *c != 1005).or(ours).unwrap_or(1006);
        if let Some(l) = g.conns.get(&cid).and_then(|c| c.link.clone()) {
            let _ = l.tx.send(Out::Kill);
        }
        g.on_close(cid, final_code);
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(2), writer).await;
}

/// O endereço de quem conectou, para o roteador.
pub type Remote = ConnectInfo<SocketAddr>;

/// `/radio?c=SALA&k=CHAVE`: o áudio do DJ da sala, só para quem está nela (a chave `rk` do jogador).
async fn radio(app: &App, ip: &str, uri: &str, method: &Method, head: bool) -> Response<Body> {
    let site = &app.site;
    if method != Method::GET {
        return plain(site, 405, "405", &[("Allow", "GET".into())], head);
    }
    let (listener, radio) = {
        let mut g = lock(app);
        if g.over(ip, "radio", 30, 60000) {
            g.abuse.r429 += 1;
            return plain(site, 429, "429", &[("Retry-After", "30".into())], head);
        }
        let code = crate::validate::norm_code(&Value::String(query_param(uri, "c").unwrap_or_default()));
        let rk = crate::validate::norm_token(&Value::String(query_param(uri, "k").unwrap_or_default()));
        let (Some(code), Some(rk)) = (code, rk) else { return plain(site, 404, "404", &[], false) };
        let Some(room) = g.rooms.get(&code) else { return plain(site, 404, "404", &[], false) };
        let who = room.players.values().any(|p| !p.bot && !p.rk.is_empty() && crate::game::token_eq(&p.rk, &rk));
        let Some((url, _, _)) = room.dj.clone().filter(|_| who) else { return plain(site, 404, "404", &[], false) };
        let radio = g.radio.clone();
        (radio.attach(&code, &url), radio)
    };
    drop(radio);
    let Some(mut l) = listener else { return plain(site, 503, "503", &[("Retry-After", "30".into())], false) };
    // espera a rádio abrir (ou falhar) antes de mandar os cabeçalhos
    let ct = loop {
        if let Some(c) = l.ct.borrow().clone() {
            break c;
        }
        if tokio::time::timeout(std::time::Duration::from_secs(20), l.ct.changed()).await.map_or(true, |r| r.is_err()) {
            break String::new();
        }
    };
    if ct.is_empty() {
        return Response::builder().status(502).body(Body::empty()).unwrap_or_default();
    }
    let rx = std::mem::replace(&mut l.rx, tokio::sync::broadcast::channel(1).1);
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx)
        .filter_map(|c| async move { c.ok().map(Ok::<_, std::io::Error>) });
    // o ouvinte vive enquanto o corpo da resposta estiver sendo lido
    let stream = futures_util::stream::StreamExt::map(stream, move |x| {
        let _keep = &l;
        x
    });
    Response::builder()
        .status(200)
        .header("Content-Type", ct)
        .header("Cache-Control", "no-store")
        .header("X-Content-Type-Options", "nosniff")
        .header("Accept-Ranges", "none")
        .header("Referrer-Policy", "no-referrer")
        .body(Body::from_stream(stream))
        .unwrap_or_default()
}
