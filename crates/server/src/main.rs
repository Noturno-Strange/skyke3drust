//! `skyke3d-server`: o servidor do Skyke3D num executável, no estilo de um servidor de Minecraft.
//!
//! Na pasta onde roda ficam `server.properties` (criado na primeira vez), `data/` (identidades, nomes, banimentos…) e
//! `logs/latest.log`. O console aceita comandos (`help`, `list`, `say`, `kick`, `ban`, `pardon`, `save-all`, `stop`…).
//! Os jogadores do navegador entram pelo mesmo endereço (a pasta `web/` tem o cliente) e os do cliente nativo também.

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::Request;
use serde_json::json;
use sky_server::config::Config;
use sky_server::game::{Game, Out, Settings};
use sky_server::http::{App, host_key, parse_origin, parse_trust};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncBufReadExt;

fn fail(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

fn settings(cfg: &Config) -> Settings {
    let int = |v: &str, d: i64, a: i64, b: i64| cfg.int(v, d, a, b).unwrap_or_else(|e| fail(&e));
    let public_url = {
        let raw = cfg.str("PUBLIC_URL", "").trim().trim_end_matches('/').to_owned();
        if raw.is_empty() {
            String::new()
        } else {
            match parse_origin(Some(&raw)) {
                Some((o, s, h, _)) if s == "https" && public_host(&h) => o,
                _ => fail("PUBLIC_URL inválida: use um domínio público em https, ex.: https://jogo.exemplo.com"),
            }
        }
    };
    let prod = !public_url.is_empty();
    let mut max_rooms = int("MAXROOMS", 60, 1, 150);
    let mut max_per_room = int("MAX_PER_ROOM", 16, 2, 40);
    if let Err(e) = sky_server::capacity::validate(Some(max_rooms), Some(max_per_room)) {
        fail(&format!("MAXROOMS × MAX_PER_ROOM: {e}"));
    }
    let cap_file = cfg.data_file("CAPACITY_FILE", "capacity.json");
    if let Some((r, p)) = sky_server::capacity::load_file(&cap_file) {
        max_rooms = r;
        max_per_room = p;
    }
    let ticket_mode = {
        let m = cfg.str("TICKET_MODE", "").to_lowercase();
        let m = if m.is_empty() { if prod { "enforce".into() } else { "warn".to_owned() } } else { m };
        if m == "off" || m == "enforce" { m } else { "warn".into() }
    };
    let voice_ws =
        env_choice(cfg, "VOICE_TRANSPORT", "ws", &[("ws", "ws"), ("websocket", "ws"), ("webrtc", "webrtc")]) == "ws";
    let voice_privacy = env_choice(cfg, "VOICE_PRIVACY", "relay", &[("relay", "relay"), ("direct", "direct")]);
    let grace_h = int("IDENTITY_GRACE_HOURS", 0, 0, 168);
    Settings {
        port: int("PORT", 3000, 0, 65535) as u16,
        bind_host: cfg.str("BIND_HOST", "0.0.0.0"),
        public_url,
        prod,
        max_rooms,
        max_per_room,
        cap_file,
        perip: int("PERIP", 6, 1, 1000) as usize,
        perip_rate: int("PERIP_RATE", 30, 1, 1_000_000) as usize,
        perip_room: int("PERIP_ROOM", 2, 1, 1000) as usize,
        global_rate: int("GLOBAL_RATE", 180, 0, 1_000_000) as usize,
        http_rate: int("HTTP_RATE", 120, 0, 1_000_000) as usize,
        maxbots: int("MAXBOTS", 200, 0, 100_000),
        ticket_mode,
        ticket_rate: int("TICKET_RATE", 20, 0, 1_000_000) as usize,
        pow_max: int("POW_MAX", 16, 0, 24) as u32,
        reconnect_ms: cfg
            .str("RECONNECT_MS", "")
            .parse::<f64>()
            .ok()
            .filter(|x| *x > 0.0)
            .map_or(30000, |x| x.clamp(100.0, 300_000.0) as u64),
        legacy_identity: cfg.on("LEGACY_IDENTITY", true),
        names_on: !cfg.data_file("NAMES_FILE", "names.json").is_empty(),
        identity_pow: int("IDENTITY_POW", 14, 0, 24) as u32,
        identity_pow_max: int("IDENTITY_POW_MAX", 16, 0, 24) as u32,
        identity_reg_day: int("IDENTITY_REG_DAY", 200, 0, 1_000_000) as usize,
        grace_ms: int("IDENTITY_GRACE_MS", if grace_h > 0 { grace_h * 3_600_000 } else { 60000 }, 0, 7 * 24 * 3_600_000)
            as u64,
        passkeys: cfg.on("PASSKEYS", true),
        voice_ws,
        voice_privacy,
        status_token: cfg.str("STATUS_TOKEN", ""),
        raw_perip: int("RAW_PERIP", 40, 4, 100_000) as usize,
    }
}

/// Variável com lista fechada de valores (`envChoice`): valor desconhecido para o servidor e diz o que é aceito.
fn env_choice(cfg: &Config, name: &str, def: &str, choices: &[(&str, &str)]) -> String {
    let Some(raw) = cfg.raw(name).filter(|r| !r.trim().is_empty()) else { return def.into() };
    let v = raw.trim().to_lowercase();
    if let Some((_, c)) = choices.iter().find(|(k, _)| *k == v) {
        return (*c).into();
    }
    let mut vals: Vec<&str> = Vec::new();
    for (_, c) in choices {
        if !vals.contains(c) {
            vals.push(c);
        }
    }
    fail(&format!(
        "{name} inválido: \"{}\". Valores aceitos: {} (padrão: {def}).",
        raw.chars().take(40).collect::<String>(),
        vals.join(" ou ")
    ))
}

fn public_host(h: &str) -> bool {
    let h = h.trim_start_matches('[').trim_end_matches(']').trim_end_matches('.');
    !h.is_empty()
        && h.parse::<std::net::IpAddr>().is_err()
        && h != "localhost"
        && !h.ends_with(".localhost")
        && !h.ends_with(".local")
        && !h.ends_with(".ts.net")
}

fn launcher_info(root: &std::path::Path, cfg: &Config) -> String {
    let ver = |s: &str| {
        let p: Vec<&str> = s.split('.').collect();
        p.len() == 3 && p.iter().all(|x| (1..=4).contains(&x.len()) && x.bytes().all(|b| b.is_ascii_digit()))
    };
    let latest =
        std::fs::read_to_string(root.join("launcher/VERSION")).map(|s| s.trim().to_owned()).unwrap_or_default();
    let latest = if ver(&latest) { latest } else { String::new() };
    let raw_min = cfg.str("LAUNCHER_MIN", "").trim().to_owned();
    let min = if ver(&raw_min) { raw_min } else { latest.clone() };
    let sha: Vec<String> = cfg
        .str("LAUNCHER_SHA256", "")
        .split(',')
        .map(|x| x.trim().to_lowercase())
        .filter(|x| x.len() == 64 && x.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect();
    let mut o = json!({"v": 1, "latest": latest, "min": min, "download": "https://github.com/Noturno-Strange/skyke3d/releases/latest/download/skyke3d-launcher.jar"});
    if !sha.is_empty() {
        o["sha256"] = json!(sha);
    }
    o.to_string()
}

fn lock(app: &App) -> std::sync::MutexGuard<'_, Game> {
    app.game.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Um comando do console do operador. Devolve `false` para encerrar.
fn console(app: &App, line: &str) -> bool {
    let line = line.trim().trim_start_matches('/');
    if line.is_empty() {
        return true;
    }
    let (cmd, rest) = line.split_once(' ').map_or((line, ""), |(a, b)| (a, b.trim()));
    let mut g = lock(app);
    match cmd {
        "help" | "?" => println!(
            "Comandos: list, rooms, say <texto>, kick <nome>, ban <nome> [minutos], pardon <id>, bans, setmax <salas> <por sala>, save-all, status, stop"
        ),
        "list" => {
            let mut names = Vec::new();
            for r in g.rooms.values() {
                for p in r.players.values().filter(|p| !p.bot && p.online) {
                    names.push(format!("{} ({})", p.name, r.code));
                }
            }
            println!(
                "Há {} de {} jogadores online: {}",
                names.len(),
                g.set.max_rooms * g.set.max_per_room,
                names.join(", ")
            );
        }
        "rooms" => {
            for r in g.rooms.values() {
                println!(
                    "{} · {} · {} pessoas, {} bots",
                    r.code,
                    r.style,
                    r.human_count(),
                    r.players.len() - r.human_count()
                );
            }
            println!("{} de {} salas", g.rooms.len(), g.set.max_rooms);
        }
        "say" if !rest.is_empty() => {
            let codes: Vec<String> = g.rooms.keys().cloned().collect();
            for c in codes {
                g.bc(&c, &json!({"t": "sys", "text": format!("📢 {rest}")}), None);
            }
            println!("[Servidor] {rest}");
        }
        "kick" | "ban" if !rest.is_empty() => {
            let mut it = rest.rsplitn(2, ' ');
            let (last, first) = (it.next().unwrap_or(""), it.next());
            let (name, mins) = match (first, last.parse::<u64>()) {
                (Some(n), Ok(m)) if cmd == "ban" => (n.to_owned(), m),
                _ => (rest.to_owned(), 60),
            };
            let key = sky_server::validate::name_key(&name);
            let hit = g
                .rooms
                .values()
                .flat_map(|r| r.players.values())
                .find(|p| !p.bot && sky_server::validate::name_key(&p.name) == key)
                .map(|p| (p.conn, p.ip.clone(), p.name.clone()));
            match hit {
                Some((conn, ip, n)) => {
                    if cmd == "ban" {
                        let k = g.bm.key(&ip);
                        g.bm.bans.block(&k, mins.clamp(1, 43200) * 60_000, sky_server::now());
                        g.bm.save(sky_server::now());
                        println!("{n} banido por {mins} min (id {}).", g.bm.host_id(&ip));
                    } else {
                        println!("{n} desconectado.");
                    }
                    if let Some(c) = conn {
                        g.send(c, &json!({"t": "sys", "text": "Você foi desconectado pelo administrador."}));
                        g.safe_close(c, 1008, "kick");
                    }
                }
                None => println!("Ninguém online com o nome {name}."),
            }
        }
        "bans" => {
            let now = sky_server::now();
            for (k, b) in g.bm.bans.list(now) {
                println!("{} · mais {} min", &k[..12.min(k.len())], (b.until - now) / 60_000);
            }
        }
        "pardon" if !rest.is_empty() => {
            let now = sky_server::now();
            let keys: Vec<String> =
                g.bm.bans.list(now).into_iter().map(|(k, _)| k).filter(|k| k.starts_with(rest)).collect();
            for k in &keys {
                g.bm.bans.unban(k);
            }
            g.bm.save(now);
            println!("{} banimento(s) removido(s).", keys.len());
        }
        "setmax" => {
            let n: Vec<i64> = rest.split_whitespace().filter_map(|x| x.parse().ok()).collect();
            match sky_server::capacity::validate(n.first().copied(), n.get(1).copied()) {
                Ok((r, p)) => {
                    g.set.max_rooms = r;
                    g.set.max_per_room = p;
                    let codes: Vec<String> = g.rooms.keys().cloned().collect();
                    for c in codes {
                        g.bc(&c, &json!({"t": "cap", "max": p}), None);
                    }
                    let file = g.set.cap_file.clone();
                    sky_server::capacity::save_file(&file, r, p);
                    let _ = g.cfg.set("MAXROOMS", &r.to_string());
                    let _ = g.cfg.set("MAX_PER_ROOM", &p.to_string());
                    println!("Capacidade: {r} salas × {p} por sala.");
                }
                Err(e) => println!("{e}"),
            }
        }
        "save-all" => {
            g.flush_all();
            println!("Dados salvos.");
        }
        "status" => println!("{}", serde_json::to_string_pretty(&g.stats()).unwrap_or_default()),
        "stop" => {
            g.stopping = true;
            return false;
        }
        _ => println!("Comando desconhecido. Digite help."),
    }
    true
}

fn shutdown(app: &App) {
    let mut g = lock(app);
    println!("Encerrando…");
    g.flush_all();
    let ids: Vec<u64> = g.conns.keys().copied().collect();
    for c in ids {
        g.safe_close(c, 1001, "servidor encerrando");
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_target(false)
        .init();
    let dir = std::env::args()
        .skip_while(|a| a != "--dir")
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let create = !std::env::args().any(|a| a == "--no-properties");
    let cfg = Config::load(&dir, create);
    let set = settings(&cfg);
    let root = sky_server::web::web_root(&cfg);
    let origins: Vec<String> = cfg
        .str("ORIGINS", "")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| match parse_origin(Some(s)) {
            Some(o) if !set.prod || o.1 == "https" => Some(o.0),
            _ => {
                tracing::warn!("ORIGINS ignorado: {s}");
                None
            }
        })
        .collect();
    let mut allowed = Vec::new();
    for o in std::iter::once(set.public_url.clone()).chain(origins.iter().cloned()).filter(|s| !s.is_empty()) {
        if let Ok(u) = url::Url::parse(&o) {
            allowed.push(host_key(&u.host_str().unwrap_or("").to_lowercase(), u.port()));
        }
    }
    for s in cfg.str("ALLOWED_HOSTS", "").split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match sky_server::http::parse_host(Some(s)) {
            Some((h, p)) => allowed.push(host_key(&h, p)),
            None => tracing::warn!("ALLOWED_HOSTS ignorado (host inválido): {s}"),
        }
    }
    let privacy = sky_server::web::Privacy {
        who: cfg.str("PRIVACY_CONTROLLER", ""),
        mail: cfg.str("PRIVACY_EMAIL", ""),
        proxy: cfg.str("PRIVACY_PROXY", ""),
        proxy_br: cfg.yes("PRIVACY_PROXY_BR"),
    };
    let site = sky_server::web::Site::load(&root, set.prod, &allowed, &privacy).unwrap_or_else(|e| fail(&e));
    let status_token = set.status_token.clone();
    if !status_token.is_empty() && status_token.len() < 12 {
        tracing::warn!("STATUS_TOKEN muito curto (mínimo 12 caracteres): painel /status desativado.");
    }
    let trust = parse_trust(&cfg.raw("TRUST_PROXY").unwrap_or_else(|| "127.0.0.1,::1".into()));
    let app = Arc::new(App {
        launcher_info: launcher_info(&root, &cfg),
        trust_cf: cfg.yes("TRUST_CF_CONNECTING_IP"),
        game: Mutex::new(Game::new(cfg.clone(), set.clone())),
        site,
        origins,
        allowed_hosts: allowed,
        trust,
        status_token,
    });
    // trabalho que volta das tarefas assíncronas (sondagem da rádio do DJ)
    let (later_tx, mut later_rx) = tokio::sync::mpsc::unbounded_channel::<sky_server::game::Later>();
    lock(&app).later = Some(later_tx);
    let a = app.clone();
    tokio::spawn(async move {
        while let Some(f) = later_rx.recv().await {
            f(&mut lock(&a));
        }
    });
    // relógios
    let a = app.clone();
    tokio::spawn(async move {
        let mut i = tokio::time::interval(Duration::from_millis(10));
        i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last = std::time::Instant::now();
        loop {
            i.tick().await;
            let lag = last.elapsed().as_secs_f64() * 1000.0 - 10.0;
            last = std::time::Instant::now();
            let mut g = lock(&a);
            g.metrics.lag(lag);
            g.timers(sky_server::now());
        }
    });
    let a = app.clone();
    tokio::spawn(async move {
        let mut i = tokio::time::interval(Duration::from_millis(100));
        let mut n = 0u64;
        loop {
            i.tick().await;
            n += 1;
            let now = sky_server::now();
            let mut g = lock(&a);
            g.tick(now);
            if n % 10 == 0 {
                g.slow_tick(now);
                g.perf_second();
            }
            if n % 50 == 0 || n == 1 {
                g.records_sample();
            }
            if n % 150 == 0 {
                g.history_sample();
            }
            if n % 100 == 0 {
                g.soc_tick();
                g.health_tick();
            }
            if n % 300 == 0 {
                for c in g.ping_round() {
                    g.send_raw_to(c, Out::Ping, 0);
                }
            }
            if n % 600 == 0 {
                g.minute(now);
            }
            if n % 6000 == 0 {
                g.ten_minutes();
            }
        }
    });
    let bind: SocketAddr = format!("{}:{}", set.bind_host, set.port)
        .parse()
        .or_else(|_| format!("[{}]:{}", set.bind_host, set.port).parse())
        .unwrap_or_else(|_| fail("BIND_HOST inválido"));
    let listener =
        tokio::net::TcpListener::bind(bind).await.unwrap_or_else(|e| fail(&format!("erro no servidor: {e}")));
    let port = listener.local_addr().map_or(set.port, |a| a.port());
    {
        let g = lock(&app);
        if !g.set.voice_ws && g.set.voice_privacy == "relay" && !g.voice.turn_ok() {
            eprintln!(
                "Voz desligada: VOICE_PRIVACY=relay exige um servidor TURN (TURN_URL + TURN_SECRET). Para voz direta, sem esconder o IP, use VOICE_PRIVACY=direct."
            );
        }
    }
    println!(
        "Skyke3D {} — escutando em {}:{port}",
        if set.prod { format!("em produção: {}", set.public_url) } else { "em desenvolvimento".into() },
        set.bind_host
    );
    if !set.prod {
        println!(
            "Aberto para loopback, rede privada, CGNAT e *.ts.net. Defina PUBLIC_URL=https://seu-dominio para produção."
        );
    }
    {
        let g = lock(&app);
        if g.set.status_token.len() >= 12 {
            println!("Painel de status ativo em /status (acesso por token).");
        }
        if g.set.ticket_mode != "off" {
            let pow = if g.set.ticket_mode == "enforce" && g.set.pow_max > 0 {
                format!(" (PoW até {} bits)", g.set.pow_max)
            } else {
                " (sem PoW)".into()
            };
            println!("Ticket de admissão: {}{pow}.", g.set.ticket_mode);
        }
        if g.bm.on {
            let f = cfg.data_file("BAN_FILE", "bans.json");
            println!("Banimento progressivo ativo ({}).", if f.is_empty() { "somente memória".into() } else { f });
        }
        if g.set.global_rate == 0 {
            eprintln!("Limite global de admissões desativado (GLOBAL_RATE=0).");
        }
        let turn_on = g.voice.turn_ok();
        if g.set.voice_ws {
            println!(
                "Voz pelo WebSocket (VOICE_TRANSPORT=ws): o servidor repassa o áudio, sem TURN.{}",
                if turn_on {
                    " O TURN configurado fica sem uso (só vale com VOICE_TRANSPORT=webrtc); pode ser desligado."
                } else {
                    ""
                }
            );
        } else if turn_on {
            println!(
                "Voz pela malha WebRTC; TURN configurado ({}).",
                if g.voice.turn_secret.is_empty() { "credenciais estáticas" } else { "credenciais temporárias" }
            );
        }
        if g.set.http_rate == 0 {
            eprintln!("Limite HTTP por IP desativado (HTTP_RATE=0).");
        }
    }
    println!(
        "{} · cliente do navegador em {} · digite help para ver os comandos.",
        cfg.str("MOTD", "Um servidor de Skyke3D"),
        root.display()
    );
    let a = app.clone();
    let router =
        axum::Router::new().fallback(move |ConnectInfo(remote): ConnectInfo<SocketAddr>, req: Request<Body>| {
            let a = a.clone();
            async move { sky_server::http::handle(a, remote, req).await }
        });
    let (stop_tx, mut stop_rx) = tokio::sync::watch::channel(false);
    // console do operador
    let a = app.clone();
    let tx = stop_tx.clone();
    tokio::spawn(async move {
        let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(l)) = lines.next_line().await {
            if !console(&a, &l) {
                let _ = tx.send(true);
                break;
            }
        }
    });
    let tx = stop_tx.clone();
    tokio::spawn(async move {
        let ctrl_c = tokio::signal::ctrl_c();
        #[cfg(unix)]
        {
            let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .unwrap_or_else(|_| fail("sinal"));
            tokio::select! {
                _ = ctrl_c => {}
                _ = term.recv() => {}
            }
        }
        #[cfg(not(unix))]
        let _ = ctrl_c.await;
        let _ = tx.send(true);
    });
    let a = app.clone();
    // como o Node (noDelay): mensagens pequenas saem na hora, sem esperar o algoritmo de Nagle
    use axum::serve::ListenerExt;
    let listener = listener.tap_io(|tcp| {
        let _ = tcp.set_nodelay(true);
    });
    let server = axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move {
            let _ = stop_rx.wait_for(|v| *v).await;
            shutdown(&a);
            tokio::time::sleep(Duration::from_millis(300)).await;
        });
    tokio::select! {
        r = server => if let Err(e) = r { fail(&format!("erro no servidor: {e}")) },
        _ = async { let mut r = stop_tx.subscribe(); let _ = r.wait_for(|v| *v).await; tokio::time::sleep(Duration::from_secs(3)).await; } => {}
    }
}
