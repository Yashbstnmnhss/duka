---
title: Compile & check
description: Source to proto or bytes, diagnostics without running anything.
---

## Compile

```rust
use duka_lib::duka_shared::types::SourceName;
use duka_lib::module::{compile_file, compile_to_bytes, from_source, load_proto, proto_to_bytes};

let proto = from_source("return 7", SourceName::Unnamed, Default::default())?;
let bytes = proto_to_bytes(&proto)?;
```

- `from_source(source, name, config)` — the frontend entry point. `name` decides
  where a relative `require` resolves from: `SourceName::Unnamed` for a chunk
  that has no file behind it, `SourceName::File(name, path)` otherwise
- `compile_file(path, config)` — read the file, then `from_source`
- `load_proto(path)` — `.dukac` is read as bytecode, everything else is compiled
  on the spot
- `proto_to_bytes` / `compile_to_bytes` — `DukaBinary` dump, 12 bytes of header
  at the very least

All five return `Result<_, Box<dyn Error + Send + Sync>>`.

## Config

`DukaConfig` is `lexer` + `parser` + `analyzer` + `adapter`, see
[Config](/language/config/). The defaults are what a script gets unless
`kao.toml` asks for something else:

```rust
use duka_lib::duka_shared::config::DukaConfig;
use duka_lib::kao::find_kao;
use std::path::Path;

let kao = find_kao(Path::new("."))?;
let config: DukaConfig = kao
    .manifest()
    .and_then(|m| m.build.config.clone())
    .unwrap_or_default();

let _entry = kao.entry(); // <root>/src/main.duka by default
let _src = kao.src_dir();
```

`find_kao` walks up until it finds `kao.toml`. `kao.modules_dir()` is what
`search_paths` uses, `kao.out_dir()` where the built artefacts go.

IR options are a separate `DukaIRConfig`, and both `from_source` and `harness`
set `var_default_local: false` for you.

## Check

Analysis without codegen — for an editor, a lint, or a build that wants the
diagnostics first:

```rust
use duka_lib::duka_frontend::analyzer::modules::ModuleBuildCache;
use duka_lib::duka_frontend::parser::ast::DukaChunk;
use duka_lib::duka_shared::config::DukaConfig;
use duka_lib::module::{Check, FileModuleSourceProvider};

let mut cache = ModuleBuildCache::default();
let mut chunk: DukaChunk = todo!();

let provider = FileModuleSourceProvider::for_entry(None);
let (data, errors) = Check::new(&provider)
    .config(&DukaConfig::default())
    .build_cache(&mut cache) // an editor re-checking the same files
    .run(&mut chunk);
```

- `build_cache(&mut ModuleBuildCache)` carries module types across checks, so a
  file that has not changed is not read and re-analyzed on every keystroke
- `already_adapted(adapted)` takes an adapter step's own output instead of
  adapting again — the CLI's pipeline, where the adapt is a step of its own
- `analyze_and_check(&mut chunk, &config, &provider, None, None)` is the same
  pipeline with the choices positional

Both return `(AnalyzerData, Vec<DukaDiagnose>)`, errors **and** analysis even
when there were errors: the editor reports them and still needs the types, a
build stops at the first.

`FileModuleSourceProvider::for_entry` resolves `require` against the entry file's
directory, the only path a chunk records.

## Diagnostics

- `diagnose.severity()` sorts them into `Error` / `Warning` / `Info`, only
  errors stop a build
- `Setup::chunk(src)` runs lexer, parser and check in one call and returns
  `Err` on the first error — see [Run scripts](run/#steps)
- `harness::scope_symbols(src)` stops after scope analysis, which is where the
  symbol table gets filled and what `hover` and `goto` resolve against; the
  prelude is injected, so a name a builtin owns does not look undeclared
- `prelude::inject(&mut analysis)` does that injection for a caller that walks
  the analysis itself
