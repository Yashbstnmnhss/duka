<div align=center>
<img src="./docs/logo.svg" width="100" height="100" alt="logo">

# DUKA

</div>

**Still WIP**

Based on lua's grammar, Duka is a project planning to implement a ~~lightweight~~ programming language.

## Docs (WIP)

See [/docs](./docs/README.md)

## Features

### Require & Module

Almost same as lua's, see `duka/`

Accept `.duka` & `.dukac` files as input (for cli and `require()`)

### GC

GC is a headache to me...

### Macro

- lexical replacement macro with `^#define` and so on
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

## Examples

See `examples` for a kao project with examples

## Tests & Benches

See `tests` for some tests

See `frontend`, `lib` crates for benches

## Crates

### Wrapper

See `duka-lib` for wrapper for rust

### Tools

- `duka-cli` Compiler-Runner & DocGen tool for duka
- `dukao` Test runner & Package manager(NOT DONE YET) for duka

### Core

- `duka-shared` Shared types and structs for duka
- `duka-frontend` Lexer, Parser, Checker, Transformer and IR Generator

---

- `duka-backend` Default backend for duka, including instruction generator, virtual machine (Runtime)

### Targets

- `duka-wasm` WASM target
- `app` Executable binary target

### Other

- `duka-macros` Macros for duka crates, see `duka_builtin` `duka_userdata`
- `duka-gc` Garbage collection implement for duka
- `duka-pipeline` Compiler pipeline utilities for duka
- `duka-printer` Renderer for structural documents
