use std::{
    process::Command,
    time::{Instant, SystemTime},
};

use duka_gc::{GcCell, Heap};
use duka_macros::{duka_builtin, duka_builtin_def};
use duka_shared::value::DukaInt;

use crate::{
    builtin::arg::{err, ok},
    errors::DukaRuntimeError,
    value::{RuntimeDukaTable, RuntimeValue},
    vm::coroutine::NativeApi,
};

duka_builtin_def! {
    mod os
    doc "Provide some functions interacting with OS"
    flags(@feature(platform))
    fn {
        meta:
            impl_execute,
            impl_exit,
            impl_remove,
            impl_rename,
            impl_clock co,
            impl_time,
            impl_date
    }
    const {}
}

#[duka_builtin(
    doc = "Get seconds from this program's start time, returns nil if not available",
    returns(float | nil)
)]
fn impl_clock(api: &mut NativeApi) -> Result<RuntimeValue, DukaRuntimeError> {
    if let Some(start) = api.start_time {
        Ok(RuntimeValue::Float((Instant::now() - start).as_secs_f64()))
    } else {
        Ok(RuntimeValue::Nil)
    }
}
#[duka_builtin(
    doc = "Get current timestamp from the UNIX epoch, throws if system time is before epoch",
    returns(int)
)]
fn impl_time() -> Result<RuntimeValue, DukaRuntimeError> {
    let since = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| DukaRuntimeError::Custom("System time before epoch".into()))?;
    Ok(RuntimeValue::Int(since.as_secs() as DukaInt))
}
#[duka_builtin(
    doc = "Get formatted current date string, throws if system time is before epoch",
    returns(table)
)]
fn impl_date(heap: &mut Heap) -> Result<RuntimeValue, DukaRuntimeError> {
    const DAYS_PER_400_YEARS: i32 = 146097;
    const DAYS_PEER_YEAR: i32 = 365;

    let since = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|_| DukaRuntimeError::Custom("System time before epoch".into()))?;
    let secs = since.as_secs();

    // Days
    let days = (secs / 86400) as i32;
    let secs_of_day = (secs % 86400) as u32;

    // Year, Month, Day
    let z = days + 719468;
    let era = if z >= 0 {
        z / DAYS_PER_400_YEARS
    } else {
        (z - DAYS_PER_400_YEARS + 1) / DAYS_PER_400_YEARS
    };
    let day_of_era = z - era * DAYS_PER_400_YEARS;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096)
        / DAYS_PEER_YEAR;

    let year = year_of_era + era * 400;
    let day_of_year =
        day_of_era - (year_of_era * DAYS_PEER_YEAR + year_of_era / 4 - year_of_era / 100);
    let month = (day_of_era * 5 + 2) / 153;
    let day = day_of_year - (month * 153 + 2) / 5 + 1;
    let month = if month < 10 { month + 3 } else { month - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    // Hours, Minutes, Seconds
    let hour = secs_of_day / 3600;
    let minute = (secs_of_day / 60) % 60;
    let second = secs_of_day % 60;

    let mut table = RuntimeDukaTable::new(6);
    table.set_by_key(heap, "year".to_owned(), RuntimeValue::Int(year as DukaInt));
    table.set_by_key(
        heap,
        "month".to_owned(),
        RuntimeValue::Int(month as DukaInt),
    );
    table.set_by_key(heap, "day".to_owned(), RuntimeValue::Int(day as DukaInt));
    table.set_by_key(heap, "hour".to_owned(), RuntimeValue::Int(hour as DukaInt));
    table.set_by_key(
        heap,
        "minute".to_owned(),
        RuntimeValue::Int(minute as DukaInt),
    );
    table.set_by_key(
        heap,
        "second".to_owned(),
        RuntimeValue::Int(second as DukaInt),
    );

    Ok(RuntimeValue::Table(heap.alloc(GcCell::new(table))))
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
    flags(@returns(result)),
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
    flags(@returns(result)),
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
    flags(@returns(result)),
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
    flags(@returns(exit)),
    doc = "Terminates program with exit code (default = 0)",
    params(code: int = 0)
)]
fn impl_exit(code: DukaInt) -> Result<(), DukaRuntimeError> {
    std::process::exit(code as i32)
}
