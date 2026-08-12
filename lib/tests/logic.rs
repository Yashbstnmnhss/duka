//! End-to-end logic engine tests: `logic!` -> parse -> compile -> run.

use duka_backend::value::{RuntimeDukaTable, RuntimeValue};
use duka_lib::harness::run;

fn run_single(src: &str) -> Result<RuntimeValue, String> {
    Ok(run(src)?
        .last()
        .cloned()
        .unwrap_or(RuntimeValue::Nil))
}

fn as_table(v: &RuntimeValue) -> impl std::ops::Deref<Target = RuntimeDukaTable> {
    let RuntimeValue::Table(g) = v else {
        panic!("expected table, got {v:?}");
    };
    g.borrow()
}

fn table_values(t: &impl std::ops::Deref<Target = RuntimeDukaTable>) -> Vec<String> {
    let mut vals = Vec::new();
    for i in 1..=t.len() {
        match t.array_get(i) {
            Some(v) => vals.push(v.eval_to_string().into_owned()),
            None => vals.push(format!("<nil@{i}>")),
        }
    }
    vals
}

#[test]
fn logic_single_clause_take_one() {
    let r = run_single(
        r#"
logic! {
    fact color(red)
}
local x = logic!(color(X))
return x
        "#,
    )
    .unwrap();
    let table = as_table(&r);
    assert_eq!(table.len(), 1);
    let vals = table_values(&table);
    assert!(vals.contains(&"red".into()), "vals = {vals:?}");
}

#[test]
fn logic_multi_clause_take_two() {
    let r = run_single(
        r#"
logic! {
    fact color(red)
    fact color(blue)
}
local a, b = logic!(color(X))
return b
        "#,
    )
    .unwrap();
    let table = as_table(&r);
    let vals = table_values(&table);
    assert!(vals.contains(&"blue".into()), "vals = {vals:?}");
}

#[test]
fn logic_rule_chain() {
    let r = run_single(
        r#"
logic! {
    fact father(john, bob)
    rule ancestor(X, Y) = father(X, Y)
}
local a = logic!(ancestor(X, Y))
return a
        "#,
    )
    .unwrap();
    let table = as_table(&r);
    let vals = table_values(&table);
    assert!(vals.contains(&"john".into()) && vals.contains(&"bob".into()), "vals = {vals:?}");
}