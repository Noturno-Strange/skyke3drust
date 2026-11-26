//! Arquivos do cliente do navegador (`lib/assets.js`, `lib/compress.js` e a parte estática de `server.js`): os nomes em
//! `/assets/` ganham o hash do conteúdo (imutáveis), cada arquivo é comprimido uma vez na subida (brotli e gzip) e as
//! páginas saem com a mesma política de segurança (CSP com os hashes dos blocos embutidos) do original.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use regex::Regex;
use sha2::{Digest, Sha256, Sha384};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

const MIN_BYTES: usize = 1024;

/// Um arquivo pronto: o original e as versões comprimidas (só quando ficam menores).
#[derive(Debug, Clone)]
pub struct Variants {
    pub raw: Vec<u8>,
    pub br: Option<Vec<u8>>,
    pub gzip: Option<Vec<u8>>,
}

pub fn compress(buf: &[u8]) -> Variants {
    let mut v = Variants { raw: buf.to_vec(), br: None, gzip: None };
    if buf.len() < MIN_BYTES {
        return v;
    }
    let mut br = Vec::new();
    {
        let params = brotli::enc::BrotliEncoderParams {
            quality: 11,
            mode: brotli::enc::backward_references::BrotliEncoderMode::BROTLI_MODE_TEXT,
            size_hint: buf.len(),
            ..Default::default()
        };
        let _ = brotli::BrotliCompress(&mut &buf[..], &mut br, &params);
    }
    if !br.is_empty() && br.len() < buf.len() {
        v.br = Some(br);
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::new(9));
    if gz.write_all(buf).is_ok()
        && let Ok(g) = gz.finish()
        && g.len() < buf.len()
    {
        v.gzip = Some(g);
    }
    v
}

/// `Accept-Encoding` → peso de cada codificação (`parseAccept`).
fn parse_accept(header: &str) -> HashMap<String, f64> {
    let mut q = HashMap::new();
    for part in header.split(',') {
        let low = part.trim().to_lowercase();
        let mut it = low.split(';');
        let Some(name) = it.next().filter(|n| !n.is_empty()) else { continue };
        let mut w = 1.0;
        for p in it {
            let p = p.trim();
            if !p.starts_with('q') || !p[1..].trim_start().starts_with('=') {
                continue;
            }
            let val = p[1..].trim_start()[1..].trim();
            let ok = val.len() <= 5
                && val.chars().next().is_some_and(|c| c.is_ascii_digit())
                && (val.len() == 1 || (val.as_bytes()[1] == b'.' && val[2..].bytes().all(|b| b.is_ascii_digit())));
            w = if ok { val.parse::<f64>().map_or(0.0, |n| n.clamp(0.0, 1.0)) } else { 0.0 };
        }
        q.insert(name.to_owned(), w);
    }
    q
}

/// Escolhe a versão para o cliente: brotli, gzip ou sem compressão.
pub fn pick<'a>(v: &'a Variants, accept: &str) -> (&'static str, &'a [u8]) {
    let q = parse_accept(accept);
    let ok = |n: &str| q.get(n).or_else(|| q.get("*")).copied().unwrap_or(0.0) > 0.0;
    if let Some(b) = v.br.as_ref().filter(|_| ok("br")) {
        return ("br", b);
    }
    if let Some(g) = v.gzip.as_ref().filter(|_| ok("gzip") || ok("x-gzip")) {
        return ("gzip", g);
    }
    ("", &v.raw)
}

pub fn etag_for(base: &str, enc: &str) -> String {
    if enc.is_empty() { base.to_owned() } else { format!("{}-{enc}\"", &base[..base.len() - 1]) }
}

pub fn etag_of(buf: &[u8]) -> String {
    format!("\"{}\"", &URL_SAFE_NO_PAD.encode(Sha256::digest(buf))[..27])
}

/// Arquivo servido com cache e ETag.
#[derive(Debug, Clone)]
pub struct Asset {
    pub v: Variants,
    pub etag: String,
    pub ctype: String,
    pub cache: String,
}

impl Asset {
    fn new(buf: &[u8], ctype: &str, cache: &str) -> Self {
        Self { v: compress(buf), etag: etag_of(buf), ctype: ctype.into(), cache: cache.into() }
    }
}

fn asset_type(ext: &str) -> Option<&'static str> {
    Some(match ext {
        ".css" => "text/css; charset=utf-8",
        ".js" => "application/javascript; charset=utf-8",
        ".jpg" => "image/jpeg",
        ".woff2" => "font/woff2",
        _ => return None,
    })
}

/// `assets.build`: troca `/assets/x.js` por `/assets/x.<hash>.js` no HTML (e as `url()` dentro dos CSS).
fn build(html: &str, root: &Path, files: &mut HashMap<String, (Vec<u8>, &'static str)>) -> Result<String, String> {
    let re = Regex::new(r#"(?-u:\b)(href|src)="/assets/([A-Za-z0-9][A-Za-z0-9._-]*)""#).map_err(|e| e.to_string())?;
    let css = Regex::new(r#"url\((['"]?)/assets/([A-Za-z0-9][A-Za-z0-9._-]*)(['"]?)\)"#).map_err(|e| e.to_string())?;
    let mut done: HashMap<String, String> = HashMap::new();
    fn resolve(
        name: &str,
        depth: u8,
        root: &Path,
        css: &Regex,
        done: &mut HashMap<String, String>,
        files: &mut HashMap<String, (Vec<u8>, &'static str)>,
    ) -> Result<String, String> {
        if let Some(u) = done.get(name) {
            return Ok(u.clone());
        }
        let ext = name.rfind('.').map(|i| name[i..].to_lowercase()).unwrap_or_default();
        let ctype = asset_type(&ext).ok_or_else(|| format!("tipo de arquivo não suportado em /assets/: {name}"))?;
        let mut buf = std::fs::read(root.join("assets").join(name))
            .map_err(|_| format!("arquivo estático ausente ou ilegível: assets/{name}"))?;
        if ext == ".css" && depth == 0 {
            let text = String::from_utf8_lossy(&buf).into_owned();
            let mut out = String::new();
            let mut last = 0;
            for c in css.captures_iter(&text) {
                let (Some(m), Some(q1), Some(n), Some(q2)) = (c.get(0), c.get(1), c.get(2), c.get(3)) else { continue };
                if q1.as_str() != q2.as_str() {
                    continue;
                }
                out += &text[last..m.start()];
                let u = resolve(n.as_str(), 1, root, css, done, files)?;
                out += &format!("url({}{u}{})", q1.as_str(), q1.as_str());
                last = m.end();
            }
            out += &text[last..];
            buf = out.into_bytes();
        }
        let hash = &crate::hex(&Sha256::digest(&buf))[..10];
        let url = format!("/assets/{}.{hash}{ext}", &name[..name.len() - ext.len()]);
        files.insert(url.clone(), (buf, ctype));
        done.insert(name.into(), url.clone());
        Ok(url)
    }
    let mut out = String::new();
    let mut last = 0;
    for c in re.captures_iter(html) {
        let (Some(m), Some(attr), Some(name)) = (c.get(0), c.get(1), c.get(2)) else { continue };
        out += &html[last..m.start()];
        let u = resolve(name.as_str(), 0, root, &css, &mut done, files)?;
        out += &format!("{}=\"{u}\"", attr.as_str());
        last = m.end();
    }
    out += &html[last..];
    Ok(out)
}

fn inline_hashes(html: &str, tag: &str) -> Vec<String> {
    let Ok(re) = Regex::new(&format!(r"(?is)<{tag}\b([^>]*)>(.*?)</{tag}\s*>")) else { return Vec::new() };
    let src = Regex::new(r"(?i)\bsrc\s*=").ok();
    let mut out = Vec::new();
    for c in re.captures_iter(html) {
        let attrs = c.get(1).map_or("", |m| m.as_str());
        if src.as_ref().is_some_and(|s| s.is_match(attrs)) {
            continue;
        }
        let body = c.get(2).map_or("", |m| m.as_str());
        let h = format!("'sha384-{}'", STANDARD.encode(Sha384::digest(body.as_bytes())));
        if !out.contains(&h) {
            out.push(h);
        }
    }
    out
}

fn html_esc(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".into(),
            '<' => "&lt;".into(),
            '>' => "&gt;".into(),
            '"' => "&quot;".into(),
            '\'' => "&#39;".into(),
            c => c.to_string(),
        })
        .collect()
}

/// Tudo o que o servidor HTTP entrega pronto.
#[derive(Debug)]
pub struct Site {
    pub index: Asset,
    pub three: Asset,
    pub assets: HashMap<String, Asset>,
    pub pwa: HashMap<String, Asset>,
    pub status_html: Vec<u8>,
    pub privacy_html: Vec<u8>,
    pub profile_html: Vec<u8>,
    pub farm_html: Vec<u8>,
    pub csp: String,
    pub prod: bool,
}

/// Opções da página de privacidade.
#[derive(Debug, Clone, Default)]
pub struct Privacy {
    pub who: String,
    pub mail: String,
    pub proxy: String,
    pub proxy_br: bool,
}

impl Site {
    /// Lê a pasta do cliente do navegador (a mesma estrutura do repositório original).
    pub fn load(root: &Path, prod: bool, allowed_hosts: &[String], privacy: &Privacy) -> Result<Self, String> {
        let read = |rel: &str| {
            std::fs::read(root.join(rel))
                .map_err(|_| format!("arquivo estático ausente ou ilegível: {rel} (em {})", root.display()))
        };
        let text = |rel: &str| read(rel).map(|b| String::from_utf8_lossy(&b).into_owned());
        let mut files = HashMap::new();
        let index = build(&text("index.html")?, root, &mut files)?;
        let profile = build(&text("profile.html")?, root, &mut files)?;
        let farm = build(&text("fazenda.html")?, root, &mut files)?;
        let status = text("status.html")?;
        let cut = |s: &str| s.trim().chars().take(200).collect::<String>();
        let (who, mail) = (cut(&privacy.who), cut(&privacy.mail));
        if who.is_empty() || mail.is_empty() {
            tracing::warn!(
                "Página /privacidade sem responsável ou contato: defina PRIVACY_CONTROLLER e PRIVACY_EMAIL."
            );
        }
        let proxy: String = privacy.proxy.trim().chars().take(100).collect();
        let privacy_html = text("privacidade.html")?
            .replace("{{RESPONSAVEL}}", &if who.is_empty() { "[responsável ainda não informado]".into() } else { html_esc(&who) })
            .replace("{{EMAIL}}", &if mail.is_empty() { "[contato ainda não informado]".into() } else { html_esc(&mail) })
            .replace(
                "{{PROXY}}",
                &if proxy.is_empty() {
                    String::new()
                } else {
                    format!(
                        "<li><b>{}</b>, sempre: todo o tráfego do jogo (páginas, chat, posições e voz) passa por esse serviço de proteção e entrega{}. Ele recebe o seu IP e vê o tráfego decifrado ao passar, mas não guarda o conteúdo para o Skyke3D.</li>",
                        html_esc(&proxy),
                        if privacy.proxy_br { "" } else { ", que pode ficar fora do Brasil" }
                    )
                },
            );
        let three = read("vendor/three-r186.min.js")?;
        let mut scripts = Vec::new();
        let mut styles = Vec::new();
        for h in [&index, &status, &profile, &farm] {
            for x in inline_hashes(h, "script") {
                if !scripts.contains(&x) {
                    scripts.push(x);
                }
            }
        }
        for h in [&index, &status, &privacy_html, &profile, &farm] {
            for x in inline_hashes(h, "style") {
                if !styles.contains(&x) {
                    styles.push(x);
                }
            }
        }
        let connect = if prod {
            let mut v = vec!["'self'".to_owned()];
            for h in allowed_hosts {
                v.push(format!("wss://{h}"));
                v.push(format!("https://{h}"));
            }
            v.join(" ")
        } else {
            "'self' ws: wss:".into()
        };
        let csp = [
            "default-src 'self'".to_owned(),
            format!("script-src 'self' {} https://www.youtube.com/iframe_api https://www.youtube.com/s/player/ https://s.ytimg.com/yts/jsbin/", scripts.join(" ")),
            "script-src-attr 'none'".into(),
            "style-src 'self'".into(),
            format!("style-src-elem 'self' {}", styles.join(" ")),
            "style-src-attr 'unsafe-inline'".into(),
            "font-src 'self'".into(),
            "img-src 'self' data: blob: https://i.ytimg.com https://img.youtube.com".into(),
            "frame-src https://www.youtube.com https://www.youtube-nocookie.com".into(),
            format!("connect-src {connect}"),
            "media-src 'self' blob: data:".into(),
            "object-src 'none'".into(),
            "base-uri 'none'".into(),
            "frame-ancestors 'none'".into(),
            "form-action 'self'".into(),
        ]
        .join("; ");
        let pwa_list = [
            ("/manifest.webmanifest", "manifest.webmanifest", "application/manifest+json; charset=utf-8", "no-cache"),
            ("/sw.js", "sw.js", "application/javascript; charset=utf-8", "no-cache"),
            ("/icon-192.png", "icon-192.png", "image/png", "public, max-age=604800"),
            ("/icon-512.png", "icon-512.png", "image/png", "public, max-age=604800"),
            ("/icon-maskable-512.png", "icon-maskable-512.png", "image/png", "public, max-age=604800"),
        ];
        let mut pwa = HashMap::new();
        for (url, f, t, c) in pwa_list {
            pwa.insert(url.to_owned(), Asset::new(&read(&format!("pwa/{f}"))?, t, c));
        }
        Ok(Self {
            index: Asset::new(index.as_bytes(), "text/html; charset=utf-8", "no-cache"),
            three: Asset::new(&three, "application/javascript; charset=utf-8", "public, max-age=31536000, immutable"),
            assets: files
                .into_iter()
                .map(|(u, (b, t))| (u, Asset::new(&b, t, "public, max-age=31536000, immutable")))
                .collect(),
            pwa,
            status_html: status.into_bytes(),
            privacy_html: privacy_html.into_bytes(),
            profile_html: profile.into_bytes(),
            farm_html: farm.into_bytes(),
            csp,
            prod,
        })
    }

    /// Cabeçalhos de segurança de toda resposta (`HDR`).
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut h = vec![
            ("Content-Type", "text/html; charset=utf-8".to_owned()),
            ("X-Content-Type-Options", "nosniff".into()),
            ("X-Frame-Options", "DENY".into()),
            ("Referrer-Policy", "no-referrer".into()),
            ("Permissions-Policy", "microphone=(self), camera=(), geolocation=()".into()),
            ("Cross-Origin-Opener-Policy", "same-origin".into()),
            ("Cross-Origin-Resource-Policy", "same-origin".into()),
        ];
        if self.prod {
            h.push(("Strict-Transport-Security", "max-age=31536000; includeSubDomains".into()));
        }
        h.push(("Content-Security-Policy", self.csp.clone()));
        h
    }
}

/// Pasta do cliente do navegador: `WEB_ROOT`, ou a pasta atual quando ela já tem `index.html` (repositório original).
pub fn web_root(cfg: &crate::config::Config) -> PathBuf {
    if let Some(w) = cfg.raw("WEB_ROOT").filter(|w| !w.is_empty()) {
        let p = PathBuf::from(w);
        return if p.is_absolute() { p } else { cfg.dir.join(p) };
    }
    if cfg.dir.join("index.html").exists() { cfg.dir.clone() } else { cfg.dir.join("web") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodings() {
        let v = Variants { raw: vec![1], br: Some(vec![2]), gzip: Some(vec![3]) };
        assert_eq!(pick(&v, "gzip, br").0, "br");
        assert_eq!(pick(&v, "gzip, br;q=0").0, "gzip");
        assert_eq!(pick(&v, "br;q=abc, gzip;q=0").0, "");
        assert_eq!(pick(&v, "*").0, "br");
        assert_eq!(etag_for("\"abc\"", "br"), "\"abc-br\"");
    }
}
