use crate::builtin::BuiltinFn;
use std::process::Command;

use duka_gc::Heap;
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::value::DukaInt;

use crate::{
    builtin::arg::{err, ok},
    errors::DukaRuntimeError,
    value::RuntimeValue,
    vm::coroutine::CoState,
};

duka_builtin_def! {
    mod os
    fn {
        meta:
            impl_execute,
            impl_exit,
            impl_remove,
            impl_rename
    }
    const {}
}

#[duka_builtin()]
fn impl_clock(_sv: &mut CoState) -> Result<RuntimeValue, DukaRuntimeError> {
    todo!()
}
#[duka_builtin()]
fn impl_time(_sv: &mut CoState) -> Result<RuntimeValue, DukaRuntimeError> {
    todo!()
}
#[duka_builtin()]
fn impl_date(_sv: &mut CoState) -> Result<RuntimeValue, DukaRuntimeError> {
    todo!()
}

#[duka_builtin(
    
    doc = "Fetches the environment variable `name` from the current process",
    params(name: string),
    returns(any)
)]
fn impl_get_env(h: &mut Heap, name: String) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(std::env::var(name)
        .map(|i| RuntimeValue::from_string(h, i))
        .unwrap_or_default())
}

macro_rules! ret_err {
    ($a: expr, $h: expr) => {
        match $a {
            Ok(v) => v,
            Err(e) => return Ok(err($h, e)),
        }
    };
}

#[duka_builtin(
    
    doc = "Removes a file or an **empty** directory from the filesystem",
    params(path: string),
    returns(vararg)
)]
fn impl_remove(h: &mut Heap, path: String) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let md = ret_err!(std::fs::metadata(&path), h);
    if md.is_dir() {
        ret_err!(std::fs::remove_dir(path), h);
    } else if md.is_file() {
        ret_err!(std::fs::remove_file(path), h);
    } else {
        return Ok(vec![
            RuntimeValue::Bool(false),
            RuntimeValue::from_string(h, "Unsupported type".to_owned()),
        ]);
    }
    Ok(ok(RuntimeValue::Nil))
}

#[duka_builtin(
    
    doc = "Renames a file or directory to a new name, replacing the original file if `name` already exists",
    params(path: string, name: string),
    returns(vararg)
)]
fn impl_rename(
    h: &mut Heap,
    path: String,
    name: String,
) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    ret_err!(std::fs::rename(path, name), h);
    Ok(ok(RuntimeValue::Nil))
}

#[duka_builtin(
    
    doc = "Run a process with command, depends on platform",
    params(cmd: string)
)]
fn impl_execute(h: &mut Heap, cmd: String) -> Result<Vec<RuntimeValue>, DukaRuntimeError> {
    let output = ret_err!(
        if cfg!(unix) {
            Command::new("sh").arg("-c").arg(cmd).output()
        } else if cfg!(windows) {
            Command::new("cmd").arg("/C").arg(cmd).output()
        } else {
            unreachable!()
        },
        h
    );
    let so = ret_err!(String::from_utf8(output.stdout), h);
    let se = ret_err!(String::from_utf8(output.stderr), h);
    Ok(vec![
        RuntimeValue::Bool(output.status.success()),
        output
            .status
            .code()
            .map(|i| RuntimeValue::Int(i.into()))
            .unwrap_or_default(),
        RuntimeValue::from_string(h, so),
        RuntimeValue::from_string(h, se),
    ])
}

#[duka_builtin(
    
    doc = "Terminates program with exit code (default = 0)",
    params(code: int = 0)
)]
fn impl_exit(code: DukaInt) -> Result<(), DukaRuntimeError> {
    std::process::exit(code as i32)
}
