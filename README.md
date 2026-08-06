<div align=center>
<img src="./docs/logo.svg" width="100" height="100" alt="logo">

# DUKA

</div>

**Still WIP**

Based on lua's grammar, Duka is a project planning to implement a ~~lightweight~~ programming language.

## Features

### Require & Module

Same as lua's, see `examples/`

Accept `.duka` & `.dukac` files as input (for cli and `require()`)

### GC

GC is a headache to me...

### Macro

- lexical replacement macro with `#^define` and so on
- built-in macros like `[:stringify!(...):]`

### Extended Syntax

- `global` explicit keyword and `local` default behavior
- `object` keyword for easier OOP table
- array
- `yield` `spawn` `go` for coroutine support
- `continue` in loop flow control

### Bang

- support custom `!` syntax for expression and statement
- `linq!` for Language Integrated Query
- `logic!` for logic programming in duka

## Docs (WIP)

See [/docs](./docs/README.md)

## Examples & Benches

See `examples` for some simple examples

See `frontend`, `lib` crates for benches

## Crates

### Core

- `duka-shared` Shared types and structs for duka
- `duka-frontend` Lexer, Parser, Checker, Transformer and IR Generator

---

- `duka-backend` Default backend for duka, including instruction generator, virtual machine
- `duka-backend-wasm` WASM target for duka's compiling, (NOT DONE YET)

### Optional

- `duka-cli` Commandline tool for duka
- `duka-gc` Garbage collection implement for duka
- `duka-pipeline` Compiler pipeline utilities for duka
