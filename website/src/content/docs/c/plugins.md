---
title: C plugins
description: Write a Duka module in C and load it with require.
---

A C plugin is a native module that a script loads with `require`. The
`duka-capi` crate supplies the C ABI and a Rust-side loader; it does **not**
create or own a VM. The host builds the VM with the Rust library and installs
the loader on its VM state.

:::note
This is different from the [`ffi`](/reference/ffi/) standard library. `ffi`
calls C functions from a running script; a C plugin *is* a module a script
requires.
:::

## The plugin

A plugin is a shared library exporting one entry point, `dukaopen_<name>`,
where `<name>` is the module name the host registers. Inside it, functions are
added to the module table with `duka_export`.

```c
#include "duka.h"

static int tri_add(DukaCall* call) {
    DukaValue a = duka_arg(call, 0);
    DukaValue b = duka_arg(call, 1);
    duka_ret_int(call, a.as.integer + b.as.integer);
    return 1; /* number of values returned */
}

int dukaopen_fixture(DukaModule* m) {
    duka_export(m, "tri_add", tri_add);
    return 0; /* 0 = success */
}
```

`dukaopen_<name>` runs once, when the module is first required. `DukaModule*`
is only valid during that call. `DukaCall*` is only valid during one call of an
exported function.

## Exported functions

An exported function has the signature `int fn(DukaCall* call)`. Arguments are
read with `duka_arg` / `duka_arg_count`, results are pushed with the `duka_ret*`
family, and the function returns how many results it produced.

| C call                                  | meaning                          |
| --------------------------------------- | -------------------------------- |
| `int duka_arg_count(DukaCall*)`         | number of arguments              |
| `DukaValue duka_arg(DukaCall*, int)`    | argument at index (0-based)      |
| `void duka_ret(DukaCall*, DukaValue)`   | push one result                  |
| `void duka_ret_int(...)` / `_number` / `_bool` / `_string(...)` | typed shortcuts |
| `void duka_error(DukaCall*, const char*)` | fail the call with a message    |
| `int duka_export(DukaModule*, const char*, DukaCFunc)` | add a function to the module |

Values crossing the boundary:

| `DukaType`   | C field        | Duka value        |
| ------------ | -------------- | ----------------- |
| `DUKA_NIL`   | —              | `nil`             |
| `DUKA_BOOL`  | `as.boolean`   | `bool`            |
| `DUKA_INT`   | `as.integer`   | `int`             |
| `DUKA_NUMBER`| `as.number`    | `float`           |
| `DUKA_STRING`| `as.string`    | `string`          |

Arguments currently expose scalars only; tables, arrays and functions read as
`nil`. Returned strings are copied into the VM heap, so the pointer passed to
`duka_value_string` only needs to live for the duration of the call.

If the function returns a negative number, or calls `duka_error`, the call
fails with that error. `duka_error` takes precedence over the return count. A
panic inside a plugin is caught and turned into a runtime error rather than
unwinding into the VM.

## The host

Create the VM state with a `CPluginLoader`, then hand the state to `Setup` (or
`VM::with_vm_globals`). No plugin is loaded until `require` asks for it.

```rust
use duka_capi::CPluginLoader;
use duka_lib::builtin::require::new_vm_globals;
use duka_lib::harness::Setup;
use duka_shared::types::SourceName;

let loader = CPluginLoader::new().add("fixture", "plugins/libfixture.so");
let state = new_vm_globals(loader.loader());

let value = Setup::new()
    .vm_state(state)
    .run_last("return require(\"fixture\").tri_add(10, 32)", SourceName::Unnamed)
    .unwrap();
```

- `add(name, path)` loads `dukaopen_<name>` from that shared library on first
  use. The library stays loaded as long as the loader is alive.
- `add_function(name, fn)` registers an in-process `extern "C"` entry point
  instead, useful for a plugin linked directly into the host.
- `loader()` returns a `Send + Sync + 'static` function suitable for
  `new_vm_globals` / `set_loader`. It composes with the file and memory
  loaders: a name that is not a registered plugin is reported as an error, so
  register a wrapping loader if both kinds of module are needed.

Keep the same `VmGlobalsHandle` between runs so `require` caches the module
table and the plugin is only initialised once.

## The script

```duka
local m = require("fixture")
return m.tri_add(10, 32)
```

`require("fixture")` evaluates to a table of the exported functions. Like any
other module it is cached, so later `require` calls return the same table.

## Building

The `duka-capi` crate builds as `cdylib`, `staticlib` and `rlib` under the
library name `duka`. The C header lives at `capi/include/duka.h`. A plugin
written in Rust links the crate as an `rlib` and uses the same types:

```toml
[lib]
name = "my_plugin"
crate-type = ["cdylib", "rlib"]

[dependencies]
duka-capi = { path = "../../capi" }
```

```rust
use duka::{duka_arg, duka_export, duka_ret_int};

extern "C" fn add(call: *mut duka::DukaCall) -> std::ffi::c_int {
    let a = duka_arg(call, 0);
    let b = duka_arg(call, 1);
    let sum = unsafe { a.as_.integer + b.as_.integer };
    duka_ret_int(call, sum);
    1
}

#[unsafe(no_mangle)]
pub extern "C" fn dukaopen_my_plugin(m: *mut duka::DukaModule) -> std::ffi::c_int {
    duka_export(m, c"add".as_ptr(), Some(add));
    0
}
```

See `capi/tests/plugin_fixture` in the repository for a complete example.
