//! Default module loader helpers for `require()`.

use std::fs::File;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use duka_backend::codegen::DefaultGenerator;
use duka_backend::codegen::binary::{DukaBinary, Load};
use duka_backend::value::{DukaProto, RuntimeValue};
use duka_backend::vm::VM;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer, TypeChecker};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::LexerWithMacro;
use duka_frontend::parser::Parser;
use duka_shared::config::DukaIRConfig;
use duka_shared::constants::{COMPILED_SUFFIX, SOURCE_SUFFIX};
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

pub fn compile_file(path: &Path) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    let source = std::fs::read_to_string(path)?;
    from_source(&source, path.to_str().map(|s| s.to_owned()))
}

pub fn from_source(
    source: &str,
    name: Option<String>,
) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    let lexer = LexerWithMacro::new(Cursor::new(source), name, Default::default());
    let stream = lexer.tokenize()?;
    let chunk = Parser::parse(stream, Default::default())?;

    let errors: Vec<_> = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .chain(TypeChecker)
        .analyze(&chunk, Default::default())
        .1
        .collect();
    if let Some(err) = errors.into_iter().next() {
        return Err(Box::new(err));
    }
    let mut chunk = chunk;
    Adapter.adapt(&mut chunk);

    let ir = IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )?;
    let proto = DefaultGenerator::generate(ir, ())?;
    Ok(proto)
}

/// Load an executable `DukaProto` from a file, dispatching on its suffix.
///
/// `{COMPILED_SUFFIX}` files hold pre-compiled bytecode and are read back
/// directly (skipping compilation); anything else is treated as `{SOURCE_SUFFIX}`
/// source and compiled on the fly.
pub fn load_proto(path: &Path) -> Result<DukaProto, Box<dyn std::error::Error + Send + Sync>> {
    if path.to_string_lossy().ends_with(COMPILED_SUFFIX) {
        let mut file = File::open(path)?;
        let binary = DukaBinary::load(&mut file)?;
        Ok(binary.into_proto())
    } else {
        compile_file(path)
    }
}

/// Convert a `require` module name to a relative path: dots become `/`.
///
/// `require("foo.bar")` searches for `foo/bar`, like Lua/Node package names.
fn normalize_name(name: &str) -> String {
    name.replace('.', "/")
}

/// Resolve the search-path templates used by `file_loader`.
///
/// Uses the `<base_dir>/modules`
/// with the `?.duka`, `?.dukac`, `?/init.duka` and `?/init.dukac` templates, and
/// `DUKA_PATH` environment variable (`;`-separated templates, `?` is the module-name placeholder) if finds
pub fn search_paths(base_dir: &Path) -> Vec<String> {
    let mut res = vec![];
    let modules = base_dir.join("modules");
    res.push(format!("{}/?{SOURCE_SUFFIX}", modules.display()));
    res.push(format!("{}/?{COMPILED_SUFFIX}", modules.display()));
    res.push(format!("{}/?/init{SOURCE_SUFFIX}", modules.display()));
    res.push(format!("{}/?/init{COMPILED_SUFFIX}", modules.display()));
    if let Ok(env) = std::env::var("DUKA_PATH") {
        res.extend(env.split(';').map(|s| s.to_owned()));
    }
    res
}

/// Build a filesystem-backed loader that resolves each module name against a list of
/// search-path templates.
///
/// For every template the `?` placeholder is replaced with the normalized name
/// (`foo.bar` -> `foo/bar`); the first existing candidate is loaded (source files
/// are compiled, `{COMPILED_SUFFIX}` bytecode is read directly) and run in a
/// scratch VM, and its last returned value is used as the module value.
///
/// Pass the result to `duka_backend::builtin::require::set_loader`.
pub fn file_loader(
    templates: impl IntoIterator<Item = String>,
) -> impl Fn(&str) -> Result<RuntimeValue, String> + Send + Sync + 'static {
    let templates: Vec<String> = templates.into_iter().collect();
    move |name| {
        let n = normalize_name(name);
        let mut tried = Vec::with_capacity(templates.len());
        for template in &templates {
            let candidate = template.replace('?', &n);
            let path = PathBuf::from(&candidate);
            if path.exists() {
                let proto =
                    load_proto(&path).map_err(|e| format!("module '{name}' load error: {e}"))?;
                let results =
                    VM::run(&proto).map_err(|e| format!("module '{name}' runtime error: {e}"))?;
                return Ok(results.last().cloned().unwrap_or(RuntimeValue::Nil));
            }
            tried.push(candidate);
        }
        Err(format!(
            "module '{name}' not found (tried: {})",
            tried.join(", ")
        ))
    }
}
