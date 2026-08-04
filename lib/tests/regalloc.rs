//! reg 分配回归:alloc_temp 计数与匿名回收

use std::io::Cursor;

use duka_backend::codegen::DefaultGenerator;
use duka_backend::value::RuntimeValue;
use duka_backend::vm::VM;
use duka_frontend::analyzer::{Adapter, BasicAnalyzer, ScopeAnalyzer};
use duka_frontend::ir::IRGenerator;
use duka_frontend::lexer::Lexer;
use duka_frontend::parser::Parser;
use duka_shared::config::DukaIRConfig;
use duka_shared::ir::DukaIR;
use duka_shared::types::{DukaAdapter, DukaAnalyzer, DukaGenerator, DukaLexer, DukaParser};

fn to_ir(src: &str) -> Result<DukaIR, String> {
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
    IRGenerator::generate(
        chunk,
        DukaIRConfig {
            var_default_local: false,
            ..DukaIRConfig::default()
        },
    )
    .map_err(|e| format!("{e}"))
}

fn run(src: &str) -> Result<Box<[RuntimeValue]>, String> {
    let ir = to_ir(src)?;
    let proto = DefaultGenerator::generate(ir, ()).map_err(|e| format!("{e}"))?;
    VM::run(&proto).map_err(|e| format!("{e}"))
}

fn run_last(src: &str) -> Result<RuntimeValue, String> {
    Ok(run(src)?.last().cloned().unwrap_or(RuntimeValue::Nil))
}

/// 从指令 Display 提取最大寄存器号
fn max_reg(ir: &DukaIR) -> usize {
    ir.instructions
        .iter()
        .map(|i| {
            let s = format!("{i}");
            let bytes = s.as_bytes();
            let mut max = 0usize;
            let mut i = 0usize;
            while i + 2 <= bytes.len() {
                if bytes[i] == b'R' && bytes[i + 1] == b'[' {
                    let mut n = 0usize;
                    let mut j = i + 2;
                    while j < bytes.len() && bytes[j].is_ascii_digit() {
                        n = n * 10 + (bytes[j] - b'0') as usize;
                        j += 1;
                    }
                    max = max.max(n);
                    i = j;
                } else {
                    i += 1;
                }
            }
            max
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn float_rhs_is_counted() {
    // 右操作数是 float 字面量,alloc_temp 分配必须计入帧大小
    let r = run_last("local x = 3\nreturn x + 2.5").unwrap();
    assert_eq!(r, RuntimeValue::Float(5.5));
}

#[test]
fn nested_float_rhs() {
    // 右操作数是嵌套二元表达式
    let r = run_last("local x = 3\nlocal y = 4\nreturn x + (y + 0.5)").unwrap();
    assert_eq!(r, RuntimeValue::Float(7.5));
}

#[test]
fn int_rhs_immediate() {
    // int 字面量走立即数路径,对照
    let r = run_last("local x = 3\nreturn x + 1").unwrap();
    assert_eq!(r, RuntimeValue::Int(4));
}

#[test]
fn multi_call_rhs() {
    // 每个调用返回值都是右操作数,修复后应回收不再堆积
    let r = run_last(
        r#"
global function f() return 1 end
global function g() return 2 end
global function h() return 3 end
local a = f() + g() + h()
return a
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(6));
}

#[test]
fn upvalue_on_rhs() {
    // upvalue 作为右操作数触发 without_up_val materialize
    let r = run_last(
        r#"
local x = 1
local function b() return 2.5 + x end
return b()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Float(3.5));
}

#[test]
fn float_rhs_inside_closure() {
    // 闭包内 float 右操作数
    let r = run_last(
        r#"
local x = 1
local function a() return x + 2.5 end
return a()
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Float(3.5));
}

#[test]
fn chained_calls_no_exhaust() {
    // 链式调用不耗尽寄存器且结果正确
    let r = run_last(
        r#"
global function f(i) return i end
local r = 0
r = r + f(1) + f(2) + f(3) + f(4) + f(5) + f(6) + f(7) + f(8)
return r
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Int(36));
}

/// 递归断言每个函数(主 + 嵌套)count 都覆盖自身指令引用的最大寄存器
fn assert_frames_cover(ir: &DukaIR) {
    for n in ir.nesteds.iter() {
        assert_frames_cover(n);
    }
    let need = max_reg(ir) + 1;
    assert!(
        need <= ir.reg_lifetime.count,
        "frame {} 需覆盖 {need}",
        ir.reg_lifetime.count
    );
}

#[test]
fn int_lhs_float_rhs_then_alloc() {
    // 纯 int 左 + float 右:常量折叠,不应崩且结果正确
    let r = run_last(
        r#"
local a = 1 + 2.5
local b = 99
return a
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Float(3.5));
}

#[test]
fn int_imm_lhs_nested_rhs_reuses_target_reg() {
    // 左 int 立即数 + 右嵌套二元:右物化到目标 reg,right==reg 不可被 free
    let r = run_last(
        r#"
local x = 1
local a = 1 + (x + 2.5)
return a
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Float(4.5));
}

#[test]
fn int_imm_lhs_call_rhs() {
    // 左 int 立即数 + 右函数调用:right 是 call 返回值(temp),emit 后才 free
    let r = run_last(
        r#"
local function f()
    return 2.5
end
local a = 1 + f()
return a
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Float(3.5));
}

#[test]
fn int_imm_lhs_call_rhs_then_more_alloc() {
    // 上述场景后继续分配,确保 free 的 call 返回值槽可安全复用
    let r = run_last(
        r#"
local function f()
    return 2.5
end
local a = 1 + f()
local b = 7
return a + b
"#,
    )
    .unwrap();
    assert_eq!(r, RuntimeValue::Float(10.5));
}

#[test]
fn frame_covers_all_registers() {
    // 帧大小必须覆盖所有指令引用的寄存器
    let ir = to_ir("local x = 3\nreturn x + 2.5").unwrap();
    assert!(max_reg(&ir) + 1 <= ir.reg_lifetime.count);
}

#[test]
fn all_frames_cover_own_instructions() {
    // lifetime 数据自洽:含嵌套函数,每层 count 都要够
    let ir = to_ir(
        r#"
global function outer(n)
    local function inner(x)
        return x + 2.5
    end
    local acc = 0
    for i = 1, n do
        acc = acc + inner(i) + 1.5
    end
    return acc
end
return outer(3)
"#,
    )
    .unwrap();
    assert_frames_cover(&ir);
}
