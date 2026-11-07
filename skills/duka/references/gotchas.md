# Duka gotchas

Every point here was observed on the real runtime. When a website page disagrees
with this file, the runtime wins.

## Syntax that trips up Lua habits

| Wrong (Lua / other)              | Right (duka)                                            |
| :------------------------------- | :------------------------------------------------------ |
| `else if`                        | `elseif`                                                |
| `!x`                             | `not x`                                                 |
| `!=`                             | `~=`                                                    |
| `tostring(x)`                    | `to_string(x)`                                          |
| `[[...]]` long strings           | `[=[ ... ]=]` (the `[[...]]` form was removed)          |
| `table.insert` etc. at top level | `array.push`, `table.remove` — everything is namespaced |
| `x = 1` makes a global           | `local x = 1` is local; `global x = 1` is global        |
| arrays start at 1                | arrays start at **0**                                   |

`fn(x) expr` takes exactly one expression. A multi-statement lambda needs an
explicit block:

```duka
local f = fn(x) x * 2          -- ok: one expression
local g = fn(x) do             -- ok: statement body
    return x * 2
end
```

## Control flow

- `if` closes its condition with `then`; `while`/`for` close theirs with `do`;
  every block ends with `end`.
- `break` leaves the nearest loop or `do` block; `continue` jumps to the next
  iteration.
- `match` is not a drop-in `switch`. A clause that completes exits the
  enclosing function/chunk instead of continuing after the `match`, and
  `else` is a plain block (no `->`). Prefer `if`/`elseif`/`else` in generated
  code unless you have verified the exact match form you need.

## Objects

- `function init(...)` is **static**: it is called on the class and `self` is
  available, but `super` is not. `super:method()` works only in instance
  methods written as `function :name()`.
- `A.new(...)` allocates, then calls `init(...)`.
- A `:` call passes the receiver; calling `obj.method()` skips `self` and the
  method's arity check will usually catch it.

## Types

- Annotations are compile-time only; they never validate runtime values.
- `default_nonnilable = true`: `int` doesn't accept `nil`. Use `int?` for nilable
  and `int?` for nilable.
- Native functions may return nilable types: `to_number` returns `float?`, so
  `local n: float = to_number(s)` fails to type-check. Handle the nil case or
  annotate as `float?`.
- Narrowing after `if x == nil` is not reliable in expressions; prefer
  `expect(x, "message")` or a result tuple when a value must be non-nil.

## Standard library traps

- `is_error(ok, err)` takes both slots of a result. `is_error(err)` is always
  false. `try` returns `(true, value...)` or `(false, message)`.
- `unwrap(ok, value)` raises on failure; `expect(v, msg)` raises on nil.
- Regex functions take the **pattern first**: `regex.replace(pattern, text,
replacement)`. The `CompiledRegex` methods keep the text in the receiver
  slot: `re:replace(text, replacement)`.
- `regex.search` skips forward to the first match, same as `is_match`.
- `select("#", ...)` counts varargs; `select(i, ...)` picks one by index.
- Nil-safe access guards exactly one step: `a?.b`, `a?[i]`, `a?:m()`, `a?()`.
  `a?.b()` still fails when `a` is nil, because the call is not the guarded
  step.

## Modules

- `require("./x")` is relative to the requiring file; `require("pkg")` resolves
  in the kao's `modules/` directory (or `DUKA_PATH` outside a project).
- `export` compiles to a `_EXPORTS` table and a trailing `return`; a module can
  also just `return { ... }`.
- `@declare export` adds a signature without a body: the importer is
  type-checked against it, but the module itself does not define the name.
- A package's root module can be `init.duka` inside the package directory.

## Tooling

- `dukao install --frozen` reproduces `kao.lock.toml` exactly; `--strict` is not
  a flag.
- `dukao test` scans `tests/` by default; test files are plain `.duka` scripts
  that `assert`.
- `dukao run <file.duka>` still treats the path as a project directory. To run a
  single file, put it in a kao or use the CLI/REPL.
