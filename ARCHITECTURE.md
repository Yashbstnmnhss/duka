# DUKA Architecture

## Overview

DUKA is a Lua-based programming language implemented in Rust. It extends Lua's grammar with modern features while keeping the core VM-based execution model.

The project is organized as a Cargo workspace with 14 crates, each handling a specific part of the compilation and execution pipeline.

## Crate Layout

```
duka/
├── shared/          # Core types, IR definitions, error handling
├── gc/              # Tri-color marking garbage collector
├── frontend/        # Lexer, parser, analyzers, IR generation
├── backend/         # Bytecode generation, VM, scheduler
├── macros/          # Procedural macros for AST visitors and builtins
├── pipeline/        # Generic compiler pipeline utilities
├── lib/             # Facade crate, module system, project manifest
├── cli/             # CLI tool (init/build/run/test/add/remove/install)
├── ffi/             # FFI bridge
├── lsp/             # Language server (hover, go-to, completions)
├── backend-wasm/    # WASM target for web deployment
├── dukao/           # Package manager and test runner
├── app/             # Binary target
└── tests/           # Integration tests
```

## Compilation Pipeline

The compiler transforms source code through several stages:

```
Source (.duka)
    ↓
Lexer → Token stream
    ↓
Parser → AST (DukaChunk)
    ↓
BangExpander → Macro expansion (! syntax)
    ↓
ScopeAnalyzer → Symbol table, upvalue tracking
    ↓
BasicAnalyzer → Basic type info
    ↓
TypeEval → Type evaluation
    ↓
TypeChecker → Type validation
    ↓
IRGenerator → DukaIR (intermediate representation)
    ↓
DefaultGenerator → DukaProto (bytecode)
    ↓
VM execution
```

### Lexer

Tokenizes source into ~80+ token types. Handles:
- Standard Lua tokens
- Extended keywords (`global`, `object`, `yield`, `spawn`, `go`, `continue`)
- Macro syntax (`^#define`, `[:...:]` splicers)
- Bang blocks (`linq!`, `logic!`)

### Parser

Recursive descent parser producing AST nodes (Stmt, Expr, Block). Supports:
- Destructuring assignments
- Pipeline operators (`|>`, `<|`)
- Pattern matching (`match`)
- Array literals (`[1, 2, 3]`)
- Object definitions
- Attributes (`@inline`, `@const`, `@data`)

### Analyzers

Multiple analysis passes run after parsing:
- **ScopeAnalyzer**: Resolves variable scopes, tracks upvalues
- **BasicAnalyzer**: Infers basic type information
- **TypeEval**: Evaluates type expressions
- **TypeChecker**: Validates type constraints with module support

### IR Generation

Converts AST to DukaIR, an intermediate representation that:
- Maps variables to registers
- Tracks scope nesting
- Preserves debug information

### Code Generation

DefaultGenerator compiles DukaIR to DukaProto (bytecode functions). Each proto contains:
- Instruction sequence
- Constant pool
- Debug info (line numbers, variable names)

## Runtime

### VM Architecture

The VM is a register-based interpreter with coroutine support.

**Core components:**
- **Scheduler**: Manages coroutine lifecycle and context switching
- **CoState**: Per-coroutine state (stack, frames, upvalues, RNG, module paths)
- **Instruction dispatch**: Main loop executing bytecodes

**Coroutine states:**
- Ready → Running → Suspended → Dead
- Actions: Return, Yield, Go (call another coroutine), Spawn (create new)

### Instruction Set

Instructions use ABC/ABKb/ABN/ASn addressing modes:

| Category | Operations |
|----------|------------|
| Arithmetic | add, sub, mul, div, idiv, mod, pow |
| Bitwise | band, bor, bxor, shl, shr, bnot |
| Comparison | eq, ne, lt, le, gt, ge |
| Logic | Unify, Bind, Call, Proceed (WAM-based) |
| Table | getfield, setfield, gettable, settable |
| Coroutine | yield, spawn, go, resume |
| Control | jump, jumpif, jumpifnot |

### Runtime Values

`RuntimeValue` enum represents all runtime data:
- Primitives: Nil, Int, Float, Bool
- Strings: Short (≤14B inline), Medium (≤47B GC), Long (heap)
- Containers: Table, Array
- Functions: UserFunc, NativeFunc
- OOP: UserData, Coroutine

Tables support metamethods (`__index`, `__newindex`, `__call`, `__gc`, `__add`, etc.).

## Garbage Collector

Tri-color mark-and-sweep collector with page-based allocation.

**Object states:**
- White (unused) → Gray (reachable, unscanned) → Black (reachable, scanned)
- Dead state for objects with finalizers

**Memory layout:**
- 8 size classes (16B to 512B) for small objects
- Large objects (>512B) allocated directly
- GcHeader stores TypeId, destructor, trace function, color/age

**Generational hints:**
- New → Survival → Old (age tracking for generational collection)

## Module System

### Loading

Two loaders:
- **FileLoader**: Resolves modules via search paths (`modules/?.duka`, `?/init.duka`)
- **MemoryLoader**: In-memory module table for WASM/embedded use

### Kao Manifest

Project configuration via `kao.toml`:
- Entry point
- Build configuration
- Dependencies

### Export Syntax

```lua
export local a = 1
export function b() end
```

Compiles to module table population and return.

## Macro System

### Lexical Replacement

`^#define` / `^#enifed` blocks for token-level replacement:
```cpp
^#define PI -> 3.1415926;
[:PI:]  -- replaced with 3.1415926
```

Supports parameters, varargs, and recursive expansion (via `~`).

### Built-in Macros

Meta macros in splicers:
- `nameof!(x)` - token name
- `stringify!(x)` - token string
- `concat!(...)` - concat identifiers
- `when!(a, b, c)` - conditional insertion
- `nonempty!(x)` - token presence check

### Bang Blocks

Custom `!` syntax for domain-specific extensions:
- `linq!` - Language Integrated Query
- `logic!` - Logic programming (WIP)

## Targets

### Native (backend)

Default target. Compiles to bytecode executed by the Rust VM.

### WASM (backend-wasm)

Web deployment target:
- Pre-compiled bytecode injection
- Memory-based module loading
- Web builtins for DOM manipulation (`__push_patch`, `__inject_css`, `__inject_html`)
- JSON serialization for runtime values
- Persistent VM state across calls

## LSP Features

- **Hover**: Type info, documentation, method signatures
- **Go-to-definition**: Symbol resolution, method calls
- **Semantic tokens**: Keywords, types, functions, variables, properties
- **Completions**: Symbol-based with kind classification

## CLI

`duka-cli` provides:
- `init` - Create new project
- `build` - Compile to bytecode
- `run` - Execute program
- `test` - Run tests
- `add`/`remove`/`install` - Package management

## References

- [CraftingInterpreters](https://craftinginterpreters.com/)
- [BuildLuaInRust](https://wubingzheng.github.io/build-lua-in-rust/zh)
- [Lua5.4Manual](https://www.lua.org/manual/5.4/manual.html)
