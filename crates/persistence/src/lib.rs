//! Gravação de dados locais.
//!
//! * [`write_atomic`] grava num arquivo temporário na mesma pasta e renomeia, para nunca deixar um arquivo pela metade
//!   (o mesmo cuidado de `lib/persist.js` no servidor original).
//! * Em sistemas Unix, arquivos marcados como privados ficam com permissão `0600`.
//! * [`load_json`] limita o tamanho lido e trata o conteúdo como não confiável (o `serde` valida a estrutura).
//!
//! Perfis separados: os dados do modo offline ficam numa pasta própria ([`Profile::Offline`]) e nunca são importados
//! pelo multiplayer oficial, que guarda progresso só no servidor.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PersistError {
    #[error("{path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("{path}: arquivo grande demais ({size} bytes)")]
    TooLarge { path: PathBuf, size: u64 },
    #[error("{path}: JSON inválido: {source}")]
    Json { path: PathBuf, source: serde_json::Error },
}

/// Perfil de dados locais.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Conexão com servidores online: só identidade e preferências; o progresso fica no servidor.
    Online,
    /// Servidor local integrado: progresso próprio, nunca enviado a servidores online.
    Offline,
}

impl Profile {
    pub fn dir_name(self) -> &'static str {
        match self {
            Profile::Online => "online",
            Profile::Offline => "offline",
        }
    }
}

/// Grava `bytes` em `path` de forma atômica. `private` = só o dono lê (Unix).
pub fn write_atomic(path: &Path, bytes: &[u8], private: bool) -> Result<(), PersistError> {
    let io = |source| PersistError::Io { path: path.to_owned(), source };
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).map_err(io)?;
    let tmp = dir.join(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|n| n.to_str()).unwrap_or("dados"),
        std::process::id()
    ));
    {
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        if private {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        #[cfg(not(unix))]
        let _ = private;
        let mut f = opts.open(&tmp).map_err(io)?;
        f.write_all(bytes).map_err(io)?;
        f.sync_all().map_err(io)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        io(e)
    })
}

pub fn save_json<T: Serialize>(path: &Path, value: &T, private: bool) -> Result<(), PersistError> {
    let bytes =
        serde_json::to_vec_pretty(value).map_err(|source| PersistError::Json { path: path.to_owned(), source })?;
    write_atomic(path, &bytes, private)
}

/// Lê JSON de até `max_bytes`. `Ok(None)` se o arquivo não existe.
pub fn load_json<T: DeserializeOwned>(path: &Path, max_bytes: u64) -> Result<Option<T>, PersistError> {
    let io = |source| PersistError::Io { path: path.to_owned(), source };
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(e)),
    };
    if meta.len() > max_bytes {
        return Err(PersistError::TooLarge { path: path.to_owned(), size: meta.len() });
    }
    let bytes = std::fs::read(path).map_err(io)?;
    serde_json::from_slice(&bytes).map(Some).map_err(|source| PersistError::Json { path: path.to_owned(), source })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn roundtrip_and_limits() {
        let dir = std::env::temp_dir().join(format!("sky-persist-{}", std::process::id()));
        let f = dir.join("a/b.json");
        let mut v = BTreeMap::new();
        v.insert("x".to_string(), 1);
        save_json(&f, &v, true).unwrap();
        let back: BTreeMap<String, i32> = load_json(&f, 1024).unwrap().unwrap();
        assert_eq!(back, v);
        assert!(matches!(load_json::<BTreeMap<String, i32>>(&f, 2), Err(PersistError::TooLarge { .. })));
        assert!(load_json::<BTreeMap<String, i32>>(&dir.join("nada.json"), 10).unwrap().is_none());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
