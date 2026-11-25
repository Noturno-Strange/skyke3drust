//! Argumentos de linha de comando (sem dependências extras).

use bevy::prelude::Resource;
use sky_shared::World;
use std::path::PathBuf;

pub const USAGE: &str = "uso: skyke3d [--world MUNDO] [--server URL --name NOME [--join CODIGO | --bots N]] \
[--shot jogo|alto --out ARQUIVO.png] [--assets PASTA]\n\
mundos: amazora, cerron, atlanthia, caatara, panthara, pamphora";

#[derive(Resource, Debug, Clone, Default)]
pub struct Args {
    pub world: World,
    pub server: Option<String>,
    pub name: String,
    pub join: Option<String>,
    pub bots: u32,
    pub shot: Option<String>,
    pub out: PathBuf,
    pub assets: Option<PathBuf>,
}

impl Args {
    pub fn parse(mut it: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut a = Args { name: "Visitante".into(), out: "captura.png".into(), ..Default::default() };
        while let Some(k) = it.next() {
            let mut val = || it.next().ok_or_else(|| format!("{k} precisa de um valor"));
            match k.as_str() {
                "--world" => a.world = val()?.parse().map_err(|e| format!("{e}"))?,
                "--server" => a.server = Some(val()?),
                "--name" => a.name = val()?,
                "--join" => a.join = Some(val()?),
                "--bots" => a.bots = val()?.parse().map_err(|_| "--bots precisa de um número")?,
                "--shot" => a.shot = Some(val()?),
                "--out" => a.out = val()?.into(),
                "--assets" => a.assets = Some(val()?.into()),
                "-h" | "--help" => return Err(String::new()),
                _ => return Err(format!("argumento desconhecido: {k}")),
            }
        }
        Ok(a)
    }

    /// Pasta de assets: `--assets`, `SKYKE3D_ASSETS`, `assets/` ao lado do executável ou a do repositório.
    pub fn assets_dir(&self) -> PathBuf {
        if let Some(p) = &self.assets {
            return p.clone();
        }
        if let Some(p) = std::env::var_os("SKYKE3D_ASSETS") {
            return p.into();
        }
        if let Some(p) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("assets")))
            && p.join("worlds").is_dir()
        {
            return p;
        }
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets")
    }
}
