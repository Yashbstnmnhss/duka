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
    let lexer = Lexer::new(Cursor::new(src), None);
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
    // 类属性 + :init 构造 + 实例方法,两个实例互不共享 self.value
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
    // 无 :init、无 base:自动生成空 init 兜底,A.new() 可用
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
    // `function .static`(无冒号)= 静态方法,不注入 self
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
    // 类级属性可直接改,后续读新值
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
    // B extends A:B 不写 :init 时沿 __index 链找到 A.init;B 覆盖 :method
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
    // B extends A:B 不写该属性时沿 metatable __index 找到 A 的类属性
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
    // 子类覆盖 :init 后手动调 A.init(self, ...) 串联祖先构造
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
    // 多级继承 C extends B extends A:init 沿链回退到 A,方法就近解析
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
    // 用户自定义静态 `new` 时跳过自动工厂,new 体内手动 setmetatable + init
    let r = run_last(
        r#"
object A
    function :init(v)
        self.value = v
    end
    function new(v)
        local s = setmetatable({}, A)
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
    // `[expr] = val` 键表达式属性
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
    // A:method() 直接调类上实例方法,self = A
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
    // 两次 new 的实例互不影响(曾 bug:共享同一实例表)
    let r = run_last(
        r#"
object A
    function :init()
        self.n = 0
    end
    function :bump()
        self.n = self.n + 1
        return self.n
    end
end
local a = A.new()
local b = A.new()
local r1 = a:bump()
local r2 = b:bump()
return r1 * 10 + r2
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(11));
}
