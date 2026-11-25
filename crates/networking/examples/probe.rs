//! Entra numa sala de um servidor Skyke3D e mostra o que chega por alguns segundos.
//!
//! `cargo run -p sky-networking --example probe -- http://localhost:3000 [mundo] [segundos]`
use sky_networking::{ConnectConfig, Enter, NetEvent};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let mut a = std::env::args().skip(1);
    let server = a.next().unwrap_or_else(|| "http://localhost:3000".into());
    let world = a.next().and_then(|w| w.parse().ok()).unwrap_or_default();
    let secs: u64 = a.next().and_then(|s| s.parse().ok()).unwrap_or(3);
    let dir = std::env::temp_dir().join("skyke3d-probe");
    let cfg = ConnectConfig {
        server,
        identity_path: dir.join("identidade.json"),
        name: "Sonda".into(),
        cfg: Default::default(),
        enter: Enter::Create { world, bots: 3 },
    };
    let mut h = match sky_networking::connect(cfg).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("falhou: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "entrou: sala {} mundo {} como {} ({} atores)",
        h.joined.code,
        h.joined.style,
        h.joined.id,
        h.joined.players.len()
    );
    let end = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    let (mut states, mut texts) = (0, 0);
    while let Ok(Some(ev)) = tokio::time::timeout_at(end, h.events.recv()).await {
        match ev {
            NetEvent::State(s) => states += s.len(),
            NetEvent::Server(m) => {
                texts += 1;
                println!("  {m:?}");
            }
            NetEvent::Voice(_) => {}
            NetEvent::Closed(r) => {
                println!("fechou: {r}");
                break;
            }
        }
    }
    println!("posições recebidas: {states}, mensagens de texto: {texts}");
}
