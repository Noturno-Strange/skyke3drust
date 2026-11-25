//! Rhai integrado em sandbox.
//!
//! Regras (ver `docs/rhai.md`):
//! * O motor é criado com [`rhai::Engine::new_raw`] mais os pacotes de linguagem pura: **sem** `print`/`debug` para o
//!   terminal, sem `eval`, sem módulos de arquivo e sem nenhuma função de sistema operacional, rede ou processo.
//! * Limites de operações, profundidade de chamada e de expressão, tamanho de strings, arrays e mapas.
//! * Scripts são compilados uma vez e guardados em cache (por nome + hash do código). Em desenvolvimento,
//!   [`ScriptHost::reload_if_changed`] recompila só o que mudou no disco.
//! * Scripts rodam por evento (ex.: "planta cresceu", "receita pronta"), nunca a cada quadro.
//! * A API exposta aos scripts é versionada ([`API_VERSION`]); cada crate de gameplay registra as funções que permite.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use rhai::packages::{Package, StandardPackage};
use rhai::{AST, Engine, EvalAltResult, ParseError, Scope};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

/// Versão da API de scripts. Scripts declaram `const API = 1;` e são recusados se forem de outra versão principal.
pub const API_VERSION: i64 = 1;

/// Limites de execução de um script.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_operations: u64,
    pub max_call_depth: usize,
    pub max_expr_depth: usize,
    pub max_string_size: usize,
    pub max_array_size: usize,
    pub max_map_size: usize,
    /// Tamanho máximo do código-fonte, em bytes.
    pub max_source: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_operations: 50_000,
            max_call_depth: 16,
            max_expr_depth: 48,
            max_string_size: 4_096,
            max_array_size: 1_024,
            max_map_size: 256,
            max_source: 64 * 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScriptError {
    #[error("script {0} não encontrado")]
    NotFound(String),
    #[error("script {name} grande demais ({size} bytes)")]
    TooLarge { name: String, size: usize },
    #[error("script {name}: erro de sintaxe: {source}")]
    Parse { name: String, source: ParseError },
    #[error("script {name}: API {found} incompatível (esperada {API_VERSION})")]
    Api { name: String, found: i64 },
    #[error("script {name}: {source}")]
    Runtime { name: String, source: Box<EvalAltResult> },
    #[error("leitura de {path}: {source}")]
    Io { path: PathBuf, source: std::io::Error },
    #[error("nome de script inválido: {0}")]
    BadName(String),
}

/// Cria um motor Rhai sem acesso ao sistema e com os limites dados.
pub fn sandboxed_engine(limits: &Limits) -> Engine {
    let mut e = Engine::new_raw();
    e.register_global_module(StandardPackage::new().as_shared_module());
    e.set_max_operations(limits.max_operations)
        .set_max_call_levels(limits.max_call_depth)
        .set_max_expr_depths(limits.max_expr_depth, limits.max_expr_depth / 2)
        .set_max_string_size(limits.max_string_size)
        .set_max_array_size(limits.max_array_size)
        .set_max_map_size(limits.max_map_size)
        .set_max_modules(0)
        .set_max_functions(256)
        .set_max_variables(256);
    // `eval` executaria código montado em tempo de execução, fora do cache e da validação.
    e.disable_symbol("eval");
    // Saída de print/debug vai para o log, nunca direto para o terminal.
    e.on_print(|s| tracing::debug!(target: "rhai", "{s}"));
    e.on_debug(|s, src, pos| tracing::debug!(target: "rhai", "{src:?} {pos}: {s}"));
    e
}

#[derive(Debug)]
struct Compiled {
    ast: Arc<AST>,
    hash: u64,
    path: Option<PathBuf>,
    mtime: Option<SystemTime>,
}

/// Hospeda scripts compilados e chama funções neles.
pub struct ScriptHost {
    engine: Engine,
    limits: Limits,
    scripts: HashMap<String, Compiled>,
    root: Option<PathBuf>,
}

impl std::fmt::Debug for ScriptHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScriptHost").field("scripts", &self.scripts.keys().collect::<Vec<_>>()).finish()
    }
}

fn hash_src(s: &str) -> u64 {
    let mut h = DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Nomes de script: `pasta/nome`, só letras minúsculas, números, `_`, `-` e `/` (sem `..` nem caminho absoluto).
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 96
        && !name.starts_with('/')
        && !name.split('/').any(|p| p.is_empty() || p == "." || p == "..")
        && name.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-' | b'/'))
}

impl ScriptHost {
    pub fn new(limits: Limits) -> Self {
        Self { engine: sandboxed_engine(&limits), limits, scripts: HashMap::new(), root: None }
    }

    /// Scripts lidos de `root` (a pasta `scripts/` do projeto). Só arquivos `.rhai` dentro dela.
    pub fn with_root(limits: Limits, root: impl Into<PathBuf>) -> Self {
        Self { root: Some(root.into()), ..Self::new(limits) }
    }

    /// Acesso ao motor para registrar a API de um sistema (ex.: `register_fn("crop_stage", ...)`).
    pub fn engine_mut(&mut self) -> &mut Engine {
        &mut self.engine
    }

    /// Compila um script a partir do texto (ou reaproveita o cache se o texto não mudou).
    pub fn load_source(&mut self, name: &str, src: &str) -> Result<(), ScriptError> {
        self.load_inner(name, src, None, None)
    }

    fn load_inner(
        &mut self,
        name: &str,
        src: &str,
        path: Option<PathBuf>,
        mtime: Option<SystemTime>,
    ) -> Result<(), ScriptError> {
        if !valid_name(name) {
            return Err(ScriptError::BadName(name.to_owned()));
        }
        if src.len() > self.limits.max_source {
            return Err(ScriptError::TooLarge { name: name.to_owned(), size: src.len() });
        }
        let hash = hash_src(src);
        if let Some(c) = self.scripts.get_mut(name)
            && c.hash == hash
        {
            c.mtime = mtime;
            return Ok(());
        }
        let ast = self.engine.compile(src).map_err(|source| ScriptError::Parse { name: name.to_owned(), source })?;
        self.check_api(name, &ast)?;
        self.scripts.insert(name.to_owned(), Compiled { ast: Arc::new(ast), hash, path, mtime });
        Ok(())
    }

    fn check_api(&self, name: &str, ast: &AST) -> Result<(), ScriptError> {
        // `const API = n;` no topo do script; sem ela, assume a versão atual. Os comandos de nível de topo rodam aqui uma
        // vez, sob os mesmos limites (e sem API de efeitos colaterais, não há o que fazerem além de definir constantes).
        let mut scope = Scope::new();
        self.engine
            .run_ast_with_scope(&mut scope, &ast.clone_statements_only())
            .map_err(|source| ScriptError::Runtime { name: name.to_owned(), source })?;
        let found = scope.get_value::<i64>("API").unwrap_or(API_VERSION);
        if found != API_VERSION {
            return Err(ScriptError::Api { name: name.to_owned(), found });
        }
        Ok(())
    }

    /// Carrega `scripts/<name>.rhai` da pasta raiz.
    pub fn load_file(&mut self, name: &str) -> Result<(), ScriptError> {
        if !valid_name(name) {
            return Err(ScriptError::BadName(name.to_owned()));
        }
        let root = self.root.clone().ok_or_else(|| ScriptError::NotFound(name.to_owned()))?;
        let path = root.join(format!("{name}.rhai"));
        let (src, mtime) = read_limited(&path, self.limits.max_source)?;
        self.load_inner(name, &src, Some(path), mtime)
    }

    /// Recompila os scripts cujo arquivo mudou. Devolve os nomes recarregados; erros mantêm a versão anterior.
    pub fn reload_if_changed(&mut self) -> Vec<Result<String, ScriptError>> {
        let changed: Vec<(String, PathBuf)> = self
            .scripts
            .iter()
            .filter_map(|(n, c)| {
                let p = c.path.as_ref()?;
                let now = std::fs::metadata(p).and_then(|m| m.modified()).ok();
                (now != c.mtime).then(|| (n.clone(), p.clone()))
            })
            .collect();
        changed
            .into_iter()
            .map(|(name, path)| {
                let (src, mtime) = read_limited(&path, self.limits.max_source)?;
                self.load_inner(&name, &src, Some(path), mtime).map(|()| name)
            })
            .collect()
    }

    pub fn has(&self, name: &str) -> bool {
        self.scripts.contains_key(name)
    }

    /// Chama `func` no script `name`. Cada chamada tem escopo novo: scripts não guardam estado entre eventos
    /// (o estado do jogo fica no Rust e é passado nos argumentos).
    pub fn call<T: Clone + Send + Sync + 'static>(
        &self,
        name: &str,
        func: &str,
        args: impl rhai::FuncArgs,
    ) -> Result<T, ScriptError> {
        let c = self.scripts.get(name).ok_or_else(|| ScriptError::NotFound(name.to_owned()))?;
        let mut scope = Scope::new();
        self.engine
            .call_fn::<T>(&mut scope, &c.ast, func, args)
            .map_err(|source| ScriptError::Runtime { name: name.to_owned(), source })
    }

    /// O script define a função `func` com `arity` parâmetros?
    pub fn defines(&self, name: &str, func: &str, arity: usize) -> bool {
        self.scripts
            .get(name)
            .is_some_and(|c| c.ast.iter_functions().any(|f| f.name == func && f.params.len() == arity))
    }
}

fn read_limited(path: &Path, max: usize) -> Result<(String, Option<SystemTime>), ScriptError> {
    let io = |source| ScriptError::Io { path: path.to_owned(), source };
    let meta = std::fs::metadata(path).map_err(io)?;
    if meta.len() > max as u64 {
        return Err(ScriptError::TooLarge { name: path.display().to_string(), size: meta.len() as usize });
    }
    let src = std::fs::read_to_string(path).map_err(io)?;
    Ok((src, meta.modified().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> ScriptHost {
        ScriptHost::new(Limits::default())
    }

    #[test]
    fn calls_functions() {
        let mut h = host();
        h.load_source("t/soma", "fn soma(a, b) { a + b }").unwrap();
        assert_eq!(h.call::<i64>("t/soma", "soma", (2_i64, 3_i64)).unwrap(), 5);
        assert!(h.defines("t/soma", "soma", 2));
    }

    #[test]
    fn infinite_loop_is_stopped() {
        let mut h = host();
        h.load_source("t/loop", "fn f() { loop { } }").unwrap();
        let e = h.call::<()>("t/loop", "f", ()).unwrap_err();
        assert!(e.to_string().contains("operations") || e.to_string().contains("operações"), "{e}");
    }

    #[test]
    fn deep_recursion_is_stopped() {
        let mut h = host();
        h.load_source("t/rec", "fn r(n) { r(n + 1) }").unwrap();
        assert!(h.call::<i64>("t/rec", "r", (0_i64,)).is_err());
    }

    #[test]
    fn huge_strings_are_stopped() {
        let mut h = host();
        h.load_source("t/str", r#"fn f() { let s = "x"; loop { s += s; } }"#).unwrap();
        assert!(h.call::<()>("t/str", "f", ()).is_err());
    }

    #[test]
    fn no_eval_and_no_modules() {
        let mut h = host();
        assert!(h.load_source("t/eval", r#"fn f() { eval("1") }"#).is_err());
        h.load_source("t/imp", r#"fn f() { import "x" as y; 1 }"#).ok();
        if h.has("t/imp") {
            assert!(h.call::<i64>("t/imp", "f", ()).is_err());
        }
    }

    #[test]
    fn api_version_checked() {
        let mut h = host();
        assert!(h.load_source("t/api", "const API = 1; fn f() { 1 }").is_ok());
        assert!(matches!(h.load_source("t/api2", "const API = 2; fn f() { 1 }"), Err(ScriptError::Api { .. })));
    }

    #[test]
    fn rejects_bad_names_and_big_sources() {
        let mut h = host();
        assert!(matches!(h.load_source("../x", "1"), Err(ScriptError::BadName(_))));
        assert!(matches!(h.load_source("/abs", "1"), Err(ScriptError::BadName(_))));
        let big = "1;".repeat(40_000);
        assert!(matches!(h.load_source("t/big", &big), Err(ScriptError::TooLarge { .. })));
    }

    #[test]
    fn hot_reload_from_disk() {
        let dir = std::env::temp_dir().join(format!("sky-rhai-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("t")).unwrap();
        let file = dir.join("t/v.rhai");
        std::fs::write(&file, "fn v() { 1 }").unwrap();
        let mut h = ScriptHost::with_root(Limits::default(), &dir);
        h.load_file("t/v").unwrap();
        assert_eq!(h.call::<i64>("t/v", "v", ()).unwrap(), 1);
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&file, "fn v() { 2 }").unwrap();
        // garante mtime diferente mesmo em sistemas de arquivos com resolução grossa
        let later = SystemTime::now() + std::time::Duration::from_secs(2);
        std::fs::File::options().write(true).open(&file).unwrap().set_modified(later).unwrap();
        let r = h.reload_if_changed();
        assert_eq!(r.len(), 1);
        assert_eq!(h.call::<i64>("t/v", "v", ()).unwrap(), 2);
        // erro de sintaxe mantém a versão anterior
        std::fs::write(&file, "fn v( {").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_modified(later + std::time::Duration::from_secs(2))
            .unwrap();
        assert!(h.reload_if_changed()[0].is_err());
        assert_eq!(h.call::<i64>("t/v", "v", ()).unwrap(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}
