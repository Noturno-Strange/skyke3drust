//! Gravação de JSON (`lib/persist.js`): grava num `.tmp` com permissão 0600 e renomeia (atômico).

use serde_json::Value;
use std::io::Write;
use std::path::Path;

/// Lê um JSON; `Ok(None)` quando o arquivo não existe, `Err` quando existe mas não é JSON.
pub fn read_json_file(file: &Path) -> std::io::Result<Option<Value>> {
    match std::fs::read(file) {
        Ok(b) => serde_json::from_slice(&b).map(Some).map_err(std::io::Error::other),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

pub fn write_json_atomic(file: &Path, obj: &Value, durable: bool) -> std::io::Result<()> {
    if let Some(d) = file.parent()
        && !d.as_os_str().is_empty()
    {
        std::fs::create_dir_all(d)?;
    }
    let mut tmp = file.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let data = crate::jsjson::to_string(obj).into_bytes();
    {
        let mut o = std::fs::OpenOptions::new();
        o.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            o.mode(0o600);
        }
        let mut f = o.open(&tmp)?;
        f.write_all(&data)?;
        if durable {
            let _ = f.sync_all();
        }
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, file)?;
    if durable
        && let Some(d) = file.parent()
        && let Ok(f) = std::fs::File::open(d)
    {
        let _ = f.sync_all();
    }
    Ok(())
}
