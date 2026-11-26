//! Configuração no estilo de um servidor de Minecraft: `server.properties` na pasta do servidor, criado com os valores
//! padrão e comentários na primeira execução. Cada opção também pode vir da variável de ambiente com o nome do servidor
//! Node.js (`PORT`, `MAXROOMS`, `STATUS_TOKEN`…), que tem prioridade: assim as implantações e os testes do original
//! continuam funcionando sem mudança. A chave no arquivo é o nome da variável em minúsculas, com `-` no lugar de `_`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Uma opção: nome da variável, valor padrão e explicação (vai como comentário no arquivo).
pub struct Opt {
    pub var: &'static str,
    pub def: &'static str,
    pub help: &'static str,
}

/// Opções escritas no `server.properties` novo (as demais variáveis do original também são aceitas no arquivo).
pub const OPTS: &[Opt] = &[
    Opt { var: "PORT", def: "3000", help: "Porta HTTP e WebSocket" },
    Opt { var: "BIND_HOST", def: "0.0.0.0", help: "Endereço de escuta (0.0.0.0 = todas as interfaces)" },
    Opt {
        var: "PUBLIC_URL",
        def: "",
        help: "Endereço público em https (produção); vazio = desenvolvimento, aberto para a rede local",
    },
    Opt { var: "MAXROOMS", def: "60", help: "Limite de salas (1 a 150)" },
    Opt { var: "MAX_PER_ROOM", def: "16", help: "Pessoas por sala (2 a 40); bots não contam" },
    Opt { var: "MAXBOTS", def: "200", help: "Bots no servidor inteiro" },
    Opt {
        var: "STATUS_TOKEN",
        def: "",
        help: "Token do painel /status (vazio = painel desligado; use 24+ caracteres)",
    },
    Opt {
        var: "TICKET_MODE",
        def: "",
        help: "Ticket de admissão: off, warn ou enforce (vazio = enforce em produção, warn em desenvolvimento)",
    },
    Opt { var: "VOICE_TRANSPORT", def: "ws", help: "Voz: ws (pelo próprio servidor) ou webrtc (precisa de TURN)" },
    Opt { var: "RECONNECT_MS", def: "30000", help: "Tempo para reconectar sem perder o lugar na sala (ms)" },
    Opt { var: "DATA_DIR", def: "data", help: "Pasta dos dados (identidades, nomes, fazendas, culinária…)" },
    Opt { var: "WEB_ROOT", def: "web", help: "Pasta do cliente do navegador (index.html, assets/, vendor/, pwa/)" },
    Opt { var: "BAN", def: "1", help: "Banimento progressivo por abuso (1 liga, 0 desliga)" },
    Opt { var: "PASSKEYS", def: "1", help: "Passkeys (WebAuthn) como alternativa ao código de recuperação" },
    Opt { var: "PRIVACY_CONTROLLER", def: "", help: "Responsável pelos dados (aparece em /privacidade)" },
    Opt { var: "PRIVACY_EMAIL", def: "", help: "Contato de privacidade (aparece em /privacidade)" },
    Opt { var: "MOTD", def: "Um servidor de Skyke3D", help: "Mensagem do dia mostrada no console ao abrir" },
];

#[derive(Debug, Clone, Default)]
pub struct Config {
    props: BTreeMap<String, String>,
    pub dir: PathBuf,
    pub file: Option<PathBuf>,
}

pub fn prop_key(var: &str) -> String {
    var.to_ascii_lowercase().replace('_', "-")
}

impl Config {
    /// Lê (ou cria) `server.properties` em `dir`.
    pub fn load(dir: &Path, create: bool) -> Self {
        let file = dir.join("server.properties");
        let mut props = BTreeMap::new();
        match std::fs::read_to_string(&file) {
            Ok(text) => {
                for line in text.lines() {
                    let l = line.trim();
                    if l.is_empty() || l.starts_with('#') || l.starts_with('!') {
                        continue;
                    }
                    if let Some((k, v)) = l.split_once('=') {
                        props.insert(k.trim().to_ascii_lowercase(), unescape(v.trim()));
                    }
                }
            }
            Err(_) if create => {
                let mut out = String::from(
                    "# Skyke3D — propriedades do servidor\n# Variáveis de ambiente com o mesmo nome (PORT, MAXROOMS...) têm prioridade.\n\n",
                );
                for o in OPTS {
                    out += &format!("# {}\n{}={}\n\n", o.help, prop_key(o.var), o.def);
                }
                if std::fs::write(&file, out).is_ok() {
                    tracing::info!("criado {}", file.display());
                }
                for o in OPTS {
                    props.insert(prop_key(o.var), o.def.to_owned());
                }
            }
            Err(_) => {}
        }
        Self { props, dir: dir.to_owned(), file: Some(file) }
    }

    /// Sem arquivo: só o ambiente (testes).
    pub fn env_only() -> Self {
        Self { props: BTreeMap::new(), dir: PathBuf::from("."), file: None }
    }

    /// Valor cru: ambiente, depois o arquivo. `None` = não definido (vale o padrão do servidor).
    pub fn raw(&self, var: &str) -> Option<String> {
        if let Ok(v) = std::env::var(var) {
            return Some(v);
        }
        self.props.get(&prop_key(var)).cloned()
    }

    pub fn str(&self, var: &str, def: &str) -> String {
        self.raw(var).unwrap_or_else(|| def.to_owned())
    }

    /// Inteiro (`envInt`): vazio = padrão; fora da faixa ou inválido para o servidor com a explicação.
    pub fn int(&self, var: &str, def: i64, min: i64, max: i64) -> Result<i64, String> {
        let Some(raw) = self.raw(var).filter(|r| !r.is_empty()) else { return Ok(def) };
        let bad = || format!("{var} inválido: use um inteiro entre {min} e {max}.");
        if !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        let n: i64 = raw.parse().map_err(|_| bad())?;
        if n < min || n > max {
            return Err(bad());
        }
        Ok(n)
    }

    /// Liga e desliga (`0`, `false`, `no` e `off` desligam).
    pub fn on(&self, var: &str, def: bool) -> bool {
        match self.raw(var) {
            None => def,
            Some(v) => !matches!(v.to_ascii_lowercase().as_str(), "0" | "false" | "no" | "off"),
        }
    }

    /// Só liga com `1`, `true`, `yes` ou `on`.
    pub fn yes(&self, var: &str) -> bool {
        self.raw(var).is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
    }

    /// Arquivo de dados: padrão `<DATA_DIR>/<nome>`; `0` desliga (só memória).
    pub fn data_file(&self, var: &str, name: &str) -> String {
        match self.raw(var) {
            Some(v) if v == "0" => String::new(),
            Some(v) if !v.is_empty() => v,
            _ => self.data_dir().join(name).to_string_lossy().into_owned(),
        }
    }

    pub fn data_dir(&self) -> PathBuf {
        let d = self.str("DATA_DIR", "data");
        let p = PathBuf::from(&d);
        if p.is_absolute() { p } else { self.dir.join(p) }
    }

    /// Grava um valor no `server.properties` (comandos do console), mantendo o resto do arquivo.
    pub fn set(&mut self, var: &str, value: &str) -> std::io::Result<()> {
        let key = prop_key(var);
        self.props.insert(key.clone(), value.to_owned());
        let Some(file) = &self.file else { return Ok(()) };
        let text = std::fs::read_to_string(file).unwrap_or_default();
        let mut found = false;
        let mut out: Vec<String> = text
            .lines()
            .map(|l| {
                if l.split_once('=').is_some_and(|(k, _)| k.trim().eq_ignore_ascii_case(&key))
                    && !l.trim_start().starts_with('#')
                {
                    found = true;
                    format!("{key}={value}")
                } else {
                    l.to_owned()
                }
            })
            .collect();
        if !found {
            out.push(format!("{key}={value}"));
        }
        std::fs::write(file, out.join("\n") + "\n")
    }
}

fn unescape(v: &str) -> String {
    v.replace("\\:", ":").replace("\\=", "=").replace("\\\\", "\\")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn properties_and_env() {
        let dir = std::env::temp_dir().join(format!("sky-cfg-{}", crate::hex(&crate::random_bytes::<6>())));
        std::fs::create_dir_all(&dir).unwrap();
        let mut c = Config::load(&dir, true);
        assert!(dir.join("server.properties").exists());
        assert_eq!(c.int("MAX_PER_ROOM", 16, 2, 40), Ok(16));
        c.set("MAX_PER_ROOM", "20").unwrap();
        let c2 = Config::load(&dir, true);
        assert_eq!(c2.int("MAX_PER_ROOM", 16, 2, 40), Ok(20));
        assert!(c2.int("MAX_PER_ROOM", 16, 2, 10).is_err());
        assert_eq!(c2.data_file("SKY_TESTE_ARQUIVO", "x.json"), dir.join("data/x.json").to_string_lossy());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
