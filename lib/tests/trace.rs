use std::io::Cursor;

use duka_backend::codegen::DefaultGenerator;
use duka_backend::vm::VM;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_shared::config::DukaIRConfig;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

fn run(src: &str) -> Result<Box<[duka_backend::value::RuntimeValue]>, String> {
    let lexer = Lexer::new(Cursor::new(src), None, Default::default());
    let stream = lexer.tokenize().map_err(|e| format!("{e}"))?;
    let chunk = Parser::parse(stream, Default::default()).map_err(|e| format!("{e}"))?;
    let errors: Vec<_> = ScopeAnalyzer
        .chain(BasicAnalyzer)
        .analyze(&chunk, Default::default())
        .1
        .collect();
    if let Some(err) = errors.into_iter().next() {
        return Err(format!("{err}"));
    }
    let mut chunk = chunk;
    Adapter.adapt(&mut chunk);
    let ir = IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )
    .map_err(|e| format!("{e}"))?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    VM::run(&proto).map_err(|e| format!("{e}"))
}

#[test]
fn trace_depth() {
    let src = r#"local function level3(x)
    return x + "oops"
end
local function level2(x)
    local y = level3(x)
    return y
end
local function level1(x)
    local y = level2(x)
    return y
end
return level1(42)
"#;
    let e = run(src).unwrap_err();
    let lines: Vec<_> = e.lines().collect();
    println!("=== full error ===\n{e}\n=== lines ===\n{lines:#?}");
    assert!(e.contains("level3"), "trace should mention level3: {e}");
    assert!(e.contains("level2"), "trace should mention level2: {e}");
    assert!(e.contains("level1"), "trace should mention level1: {e}");
}

#[test]
fn trace_metamethod_child() {
    // 元方法 __add 是 user function，出错时应在 trace 中体现。
    let src = r#"local function boom(a, b)
    return a + b
end

local mt = {}
mt.__add = function(a)
    return a.field + 1
end
local tab = set_metatable({}, mt)

boom(1, 2)
local _ = tab + 5
"#;
    let e = run(src).unwrap_err();
    let lines: Vec<_> = e.lines().collect();
    println!("=== full error ===\n{e}\n=== lines ===\n{lines:#?}");
    assert!(
        e.contains("__add") || e.contains("stack traceback"),
        "trace should include metamethod frames: {e}"
    );
}
