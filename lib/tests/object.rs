//! End-to-end object sugar tests: `object`/`extends`/`:init`/`.new` -> parse -> compile -> run.

use std::io::Cursor;

use duka_backend::codegen::DefaultGenerator;
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_shared::config::DukaIRConfig;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

fn run(src: &str) -> Result<Box<[RuntimeValue]>, String> {
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

fn run_last(src: &str) -> Result<RuntimeValue, String> {
    Ok(run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil))
}

#[test]
fn object_property_and_instance_method() {
    let r = run_last(
        r#"
object A
    property = 1
    function :init(a)
        self.value = a
    end
    function :method(a)
        return self.value + a
    end
end
local a = A.new(10)
local b = A.new(100)
return A.property + a:method(2) + b:method(1)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(114));
}

#[test]
fn object_empty_generates_fallback_init() {
    let r = run_last(
        r#"
object A
end
local a = A.new()
return 42
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn object_static_method() {
    let r = run_last(
        r#"
object A
    function double(x)
        return x * 2
    end
end
return A.double(21)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(42));
}

#[test]
fn object_static_property_reassignment() {
    let r = run_last(
        r#"
object A
    count = 1
end
A.count = A.count + 10
return A.count
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}

#[test]
fn object_inherit_method_resolution() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
    function :method()
        return self.value + 1
    end
end
object B extends A
    function :method()
        return self.value + 10
    end
end
local b = B.new(5)
return b:method()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(15));
}

#[test]
fn object_inherit_class_level_property() {
    let r = run_last(
        r#"
object A
    shared = 7
end
object B extends A
end
return B.shared
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(7));
}

#[test]
fn object_inherit_manual_super_init() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
end
object B extends A
    function :init(a)
        A.init(self, a + 1)
        self.extra = a * 10
    end
end
local b = B.new(5)
return b.value + b.extra
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(56));
}

#[test]
fn object_inherit_multilevel() {
    let r = run_last(
        r#"
object A
    function :init(a)
        self.value = a
    end
    function :method()
        return self.value + 1
    end
end
object B extends A
    function :method()
        return self.value + 10
    end
end
object C extends B
    function :method()
        return self.value + 100
    end
end
local c = C.new(3)
return c:method()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(103));
}

#[test]
fn object_custom_new() {
    let r = run_last(
        r#"
object A
    function :init(v)
        self.value = v
    end
    function new(v)
        local s = set_metatable({}, A)
        s:init(v)
        return s
    end
end
local a = A.new(99)
return a.value
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(99));
}

#[test]
fn object_computed_key_property() {
    let r = run_last(
        r#"
object A
    ["k" .. "ey"] = 5
end
return A.key
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(5));
}

#[test]
fn object_method_on_class_itself() {
    let r = run_last(
        r#"
object A
    function :method(a)
        return a + 1
    end
end
return A:method(10)
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}

#[test]
fn object_new_returns_distinct_instances() {
    let r = run_last(
        r#"
object A
    function :init()
        self.n = 0
    end
    function __tostring()
        return "12312"
    end
    function :bump()
        self.n = self.n + 1
        return self.n
    end
end
local a = A.new()
local b = A.new()
print(a)
local r1 = a:bump()
local r2 = b:bump()
return r1 * 10 + r2
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}

#[test]
fn data_object_auto_init() {
    let r = run_last(
        r#"
@data object A
    x
    y
end
local a = A.new(3, 5)
return a.x + a.y
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(8));
}

#[test]
fn data_object_auto_tostring() {
    let r = run_last(
        r#"
@data object A
    x = 3
    y = 5
end
local a = A.new(3, 5)
return to_string(a)
"#,
    )
    .unwrap();
    assert_eq!(
        r.eval_to_string(),
        format!("{}{}", RuntimeValue::Int(3).eval_to_string(), RuntimeValue::Int(5).eval_to_string())
    );
}
