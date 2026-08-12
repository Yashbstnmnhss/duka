use duka_lib::harness::run;

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
