---
title: Run scripts
description: Hand the harness source, take RuntimeValues back.
---

## One shot

`harness` runs a string through the whole pipeline — lexer, parser, analyzer,
IR, codegen, VM — and gives the results back.

```rust
use duka_lib::harness::{run, run_last, run_results, run_with_input};
use duka_lib::value::RuntimeValue;

let one: RuntimeValue = run_last("return 1 + 1").unwrap(); // Int(2)
let all = run("return 1, 2").unwrap(); // Box<[RuntimeValue]>
let list = run_results("return 1").unwrap(); // Vec<RuntimeValue>
```

- `run_last` keeps the value the last `return` produced, a chunk that returns
  nothing reads as `nil`
- `run` / `run_results` keep every value the chunk returned
- `run_with_input(src, b"..")` feeds bytes to stdin
- `run_with(src, state)` runs against a `VmGlobalsHandle` you already have, so
  `require` keeps its module cache between calls

Every one of them returns `Result<_, DukaError>`:

| variant | the stage that failed |
| --- | --- |
| `Spanned` | lexer / parser |
| `Analysis(Vec<DukaDiagnose>)` | analyzer & type checker |
| `Generator` | IR |
| `Codegen` | bytecode |
| `RuntimeTrace` | VM, with a stack trace |
| `Runtime` | VM, without a stack trace |

`DukaError` implements `Display`, so `eprintln!("{e}")` is enough for a host.

## Steps

`Setup` is the same thing with the choices named. It holds the source provider
(`require` asks it), the VM state and nothing else.

```rust
use duka_lib::harness::Setup;
use duka_lib::module::FileModuleSourceProvider;
use std::path::Path;

let provider = FileModuleSourceProvider::for_entry(Some(Path::new("src/main.duka")));
let setup = Setup::new().provider(&provider);

let ir = setup.ir("return 1")?; // lex + parse + check + IR
let chunk = setup.chunk("return 1")?; // stops at the first error
let last = setup.run_last("return 1")?;
```

`chunk` runs the same pipeline the compiler runs — a harness that checked a
different way would pass tests the compiler would not.

## Own VM

When one run is not enough, take the VM apart: globals survive between
`execute` calls, stdin/stdout are sinks you install.

```rust
use duka_lib::duka_gc::Heap;
use duka_lib::duka_shared::types::SourceName;
use duka_lib::module::from_source;
use duka_lib::value::RuntimeValue;
use duka_lib::vm::VM;
use duka_lib::DukaVM;

let proto = from_source("return limit", SourceName::Unnamed, Default::default())?;

let mut vm = VM::new(Heap::new());
vm.set_global("limit", RuntimeValue::Int(10));

let count = vm.execute(&proto)?;
let mut main = vm.main_coroutine_mut();
let mut state = std::mem::take(&mut main.inner);
let values: Vec<RuntimeValue> = state.take_stack_many(0, count)?.into();
```

- `execute` runs the proto and returns how many values are on the main stack,
  `take_stack_many` takes them
- `VM::run_from(&proto, entry, state)` does that dance for you in one call and
  returns `Box<[RuntimeValue]>`, `entry` is the path relative `require`
  resolves from
- `set_stdout` / `set_stderr` install an `Arc<Mutex<Vec<u8>>>` sink that
  `print` writes into, `take_stdout` hands it back so you can read the bytes;
  `set_input` / `take_input` do the same for stdin
- `collect_gc` runs a collection by hand

## Values from rust

Anything that is a `RuntimeValue` goes in with `set_global`:

```rust
vm.set_global("config", RuntimeValue::Int(10));
```

A struct with methods is [user data](/guides/userdata-in-rust/):

```rust
let counter = Counter::new(0).into_value(&mut vm.heap);
vm.set_global("counter", counter);
```

A plain rust function is a `RustClosure`, see
[Extend the runtime](extend/#function-values).

## require

`require` reads through a loader installed on the VM state. Two are provided:

```rust
use duka_lib::builtin::require::{default_vm_globals, set_loader};
use duka_lib::duka_shared::config::DukaConfig;
use duka_lib::module::{file_loader, memory_loader, search_paths};
use std::path::Path;

let state = default_vm_globals();
let templates = search_paths(Path::new("."), "modules");
set_loader(&state, file_loader(templates, DukaConfig::default()));
```

- `file_loader` resolves against `modules/?.duka`, `?.dukac`, `?/init.*` and
  every template in `DUKA_PATH` (`;`-separated, `?` is the module name)
- `memory_loader` is the same resolution over a `HashMap<String, Vec<u8>>` —
  no filesystem, for a wasm build or an archive in memory
- keep the state with `Setup::new().vm_state(state.clone())` or
  `VM::with_vm_globals(heap, state)` so modules are loaded once
