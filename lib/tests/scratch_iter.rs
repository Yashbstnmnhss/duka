use duka_backend::value::RuntimeValue;
use duka_lib::harness::run_results;

#[test]
fn scratch_two_var() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    if i >= 3 then return false end
    local k = i
    local v = arr[i]
    i = i + 1
    return true, k, v
end
local n = 0
local total = 0
for k, v in it do
    n = n + 1
    total = total + v
end
return n, total
"#,
    )
    .unwrap();
    println!("results: {:?}", r);
    assert_eq!(*r, [RuntimeValue::Int(3), RuntimeValue::Int(60)]);
}

#[test]
fn scratch_single_var() {
    let r = run_results(
        r#"
local i = 0
local it = function()
    i = i + 1
    if i > 3 then return false end
    return true, i
end
local count = 0
local sum = 0
for k in it do
    count = count + 1
    sum = sum + k
end
return count, sum
"#,
    )
    .unwrap();
    println!("results2: {:?}", r);
    assert_eq!(*r, [RuntimeValue::Int(3), RuntimeValue::Int(6)]);
}

#[test]
fn scratch_userfunc_multireturn() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    if i >= 3 then return false end
    local k = i
    local v = arr[i]
    i = i + 1
    return true, k, v
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("results3: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Bool(true),
            RuntimeValue::Int(0),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_closure_multireturn() {
    let r = run_results(
        r#"
local it = function()
    return true, 0, 10
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("results5: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Bool(true),
            RuntimeValue::Int(0),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_closure_multireturn_2() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local it = function()
    local v = arr[0]
    return true, 0, v
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("results6: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Bool(true),
            RuntimeValue::Int(0),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_upvalue_index() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    local v = arr[i]
    i = i + 1
    return true, 0, v
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("results7: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Bool(true),
            RuntimeValue::Int(0),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_bare_multireturn_3() {
    let r = run_results(
        r#"
local function f()
    local v = {10}[0]
    return true, 0, v
end
local a, b, c = f()
return a, b, c
"#,
    )
    .unwrap();
    println!("results8: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Bool(true),
            RuntimeValue::Int(0),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_upvalue_read_write() {
    let r = run_results(
        r#"
local i = 0
local it = function()
    local k = i
    i = i + 1
    return true, k, 10
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("results9: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Bool(true),
            RuntimeValue::Int(0),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_upvalue_table_index() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local it = function()
    local v = arr[0]
    return false, 9, v
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("resultsA: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Bool(false),
            RuntimeValue::Int(9),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_upvalue_var_index_read_only() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    local v = arr[i]
    return i, 9, v
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("resultsB: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Int(0),
            RuntimeValue::Int(9),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_upvalue_var_index_write_later() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    local v = arr[i]
    i = i + 1
    return 5, 9, v
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("resultsC: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Int(5),
            RuntimeValue::Int(9),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_param_index() {
    let r = run_results(
        r#"
local function it(arr, i)
    local v = arr[i]
    return i, 9, v
end
local a, b, c = it({10, 20, 30}, 0)
return a, b, c
"#,
    )
    .unwrap();
    println!("resultsD: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Int(0),
            RuntimeValue::Int(9),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_local_index() {
    let r = run_results(
        r#"
local function it()
    local arr = {10, 20, 30}
    local i = 0
    local v = arr[i]
    return i, 9, v
end
local a, b, c = it()
return a, b, c
"#,
    )
    .unwrap();
    println!("resultsE: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Int(0),
            RuntimeValue::Int(9),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_upvalue_single_ret() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    local v = arr[i]
    return v
end
return it()
"#,
    )
    .unwrap();
    println!("resultsF: {:?}", r);
    assert_eq!(*r, [RuntimeValue::Int(10)]);
}

#[test]
fn scratch_upvalue_ret_vararg() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    local v = arr[i]
    return 1, 2, v
end
return it()
"#,
    )
    .unwrap();
    println!("resultsG: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_dump_ir() {
    let ir = duka_lib::harness::to_ir(
        r#"
local arr = {10, 20, 30}
local i = 0
local it = function()
    local v = arr[i]
    return 1, 2, v
end
return it()
"#,
    )
    .unwrap();
    for proto in ir.nesteds.into_iter() {
        println!(
            "=== nested proto: {} ===",
            proto.debug_info.debug_name.as_deref().unwrap_or("...")
        );
        println!("{}", proto);
    }
}

#[test]
fn scratch_upvalue_array_index() {
    let r = run_results(
        r#"
local arr = [10, 20, 30]
local i = 0
local it = function()
    local v = arr[i]
    return 1, 2, v
end
return it()
"#,
    )
    .unwrap();
    println!("resultsArr: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_upvalue_table_localidx() {
    let r = run_results(
        r#"
local arr = {10, 20, 30}
local it = function()
    local i = 0
    local v = arr[i]
    return 1, 2, v
end
return it()
"#,
    )
    .unwrap();
    println!("resultsT: {:?}", r);
    assert_eq!(
        *r,
        [
            RuntimeValue::Int(1),
            RuntimeValue::Int(2),
            RuntimeValue::Int(10)
        ]
    );
}

#[test]
fn scratch_plain_multireturn() {
    let r = run_results(
        r#"local acc = {}
for v in iter.range(0, 3) do
    table.insert(acc, v, 1)
end
return #acc
"#,
    )
    .unwrap();
    println!("results4: {:?}", r);
    assert_eq!(*r, [RuntimeValue::Int(3)]);
}
