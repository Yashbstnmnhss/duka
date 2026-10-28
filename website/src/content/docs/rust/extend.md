---
title: Extend the runtime
description: Add functions and constants a script can call.
---

## `#[duka_builtin]`

Turns a rust function into a builtin and writes the signature down for the
compiler and the doc generator. `name` defaults to the identifier with the
`impl_` prefix stripped.

```rust
#[duka_builtin(
    name = "double",
    doc = "Multiply an integer by two",
    params(n: int),
    returns(int)
)]
fn impl_double(n: DukaInt) -> Result<RuntimeValue, DukaRuntimeError> {
    Ok(RuntimeValue::Int(n * 2))
}
```

Accepted keys: `name`, `doc`, `return_doc`, `example`, `params`, `type_params`,
`returns`, `where`, `constraint`, `flags`.

`DukaInt` is `duka_lib::duka_shared::value::DukaInt`, `RuntimeValue` and
`DukaRuntimeError` come from `duka_lib::value` and `duka_lib::errors`.

What the signature may look like:

| you write | rust side |
| --- | --- |
| `params(s: string, start: int)` | `s: String, start: DukaInt` |
| `params(vals: vararg)` | `vals: Vec<RuntimeValue>` |
| `params(arr: array)` | `arr: RuntimeValue` |
| `h: &mut Heap` first | allocate while the call runs |
| `cv: &mut CoState` + `api: &mut NativeApi` | call back into duka, the `co` form |

Return shapes: `Result<(), _>` gives no value, `Result<RuntimeValue, _>` one,
`Result<Vec<RuntimeValue>, _>` backs `returns(vararg)`.

Constants take their own set of keys — `type`, `name`, `doc`, `example`,
`value`, `flags`:

```rust
#[duka_builtin(type = "int", name = "MAX_DEPTH", doc = "Search depth", value = "64")]
const DUKA_MAX_DEPTH: RuntimeValue = RuntimeValue::Int(64);
```

## `duka_builtin_def!`

Groups the functions and constants into a module:

```rust
duka_builtin_def! {
    mod mine
    doc "Host builtins"
    fn {
        plain:
            impl_double,
        meta:
            impl_greet co,
    }
    init {
        GREET: make_greet(heap) meta __DUKA_GREET_META doc("Greeter"),
    }
}
```

- `plain:` registers the function, `meta:` registers it **and** its
  `__DUKA_*_META`, which is what doc-gen reads
- `co` marks a function that takes `NativeApi` (`BuiltinFn::Co`), everything
  else is `BuiltinFn::Plain`
- `init { Name: expr meta IDENT }` evaluates `expr` while the table is built —
  a value that needs the heap goes there
- other sections: `const { … }`, `mod { … }` for nested modules, `userdata { … }`
- the block generates `MODULE_NAME`, `registry()`, `consts_registry()`,
  `init_registry(heap)` and `MODULE_META`

## Registration

`VM::new` and `VM::with_vm_globals` call `builtin::register_all(&mut heap,
&mut vm_ctx)`, which puts every module table into globals. A builtin is there
for every script the moment a `VM` exists — there is no per-VM hook for plain
functions; the VM state (`VmGlobalsHandle`) is where `require` and its loader
live.

The builtins the language ships live in `backend/src/builtin/*.rs`, the macros
in `macros/src/`.

## Function values

At runtime a rust function is a `RustClosure`, and anything holding one is a
`RuntimeValue`:

```rust
use duka_lib::duka_gc::{GcCell, Heap};
use duka_lib::duka_shared::types::ValueCount;
use duka_lib::value::{RustClosure, RuntimeValue};
use duka_lib::vm::VM;
use std::cell::RefCell;
use std::rc::Rc;

let ticks = Rc::new(RefCell::new(0u32));
let seen = ticks.clone();

let mut vm = VM::new(Heap::new());
let tick = RuntimeValue::NativeFunc(vm.heap.alloc(GcCell::new(RustClosure::returns(
    move |_co, _h, _api| {
        *seen.borrow_mut() += 1;
        Ok(ValueCount::Exact(0))
    },
    Some("host_tick".into()),
))));
vm.set_global("host_tick", tick);
```

```duka
host_tick()
```

- `returns(f, name)` — `f` is `FnMut(&mut CoState, &mut Heap, &mut NativeApi)`
  returning `Result<ValueCount, DukaRuntimeError>`
- `Exact(0)` is a function that hands back no value, `Exact(n)` hands back `n`
- `RustClosure::define::<P, R, f>` is the typed form: `[RuntimeValue; P]` in,
  `[RuntimeValue; R]` out

## Next

- a struct with methods is [user data](/guides/userdata-in-rust/)
- telling the checker about a function you provide is
  [`@declare`](/language/attributes/#declare)
