---
name: duka
description: Write and maintain Duka (.duka) code and kao projects — syntax, functions, types, objects, modules, error handling, standard library, and the dukao CLI. Use when the task involves .duka files, kao.toml manifests, or the dukao tool.
---

# Duka language

Duka is a statically annotated, Lua-like scripting language with its own
project tool, `dukao`. This skill is the fast path: verified syntax patterns,
the project layout, the commands, and the pitfalls that language models get
wrong. Every example below was executed against the real runtime.

Authoritative but human-oriented details live on the website; link there
instead of guessing:

- Language: https://yashbstnmnhss.github.io/duka/language/
- Standard library: https://yashbstnmnhss.github.io/duka/reference/

## Project layout (a "kao")

```text
myapp/
├── kao.toml          # manifest
├── src/
│   └── main.duka     # default entry
├── build/            # compiled output
├── modules/          # installed packages
└── tests/            # dukao test scans here by default
```

`kao.toml`:

```toml
[kao]
name = "myapp"
version = "0.1.0"

[build]
entry = "main.duka"      # relative to src_dir
src_dir = "src"
modules_dir = "modules"

[dependencies]
mylib = { git = "https://host/mylib.git", version = "^1.2" }
```

Only `[kao] name` is normally needed; every other field has a default.

## Commands

```sh
dukao init myapp --name myapp --version 0.1.0
dukao run            # run the entry; `--entry file.duka` overrides it
dukao run -- arg1    # everything after `--` becomes the script's `...`
dukao build          # compile entry + modules into build/
dukao test           # run tests/*.duka (--filter <substr>)
dukao install        # install what the lock records, resolving what it misses
dukao install --frozen   # the lock must cover the manifest; no new resolution
dukao install --offline  # cached clones only, never the network
dukao update         # re-resolve every requirement and rewrite the lock
dukao add <git-url> [--tag v1.0] [--branch main] [--version "^1.2"] [--as name]
dukao add <dir> --path [--as name]
dukao remove <name>
dukao serve [--port 3000]
```

`dukao run <path>` also accepts a project directory; bare `dukao run` walks up
to the nearest `kao.toml`.

## Program shape

```duka
-- a line comment; `---` is a doc comment
--- Doc comments can precede declarations.

local name = "duka"       -- local is the default in files
global counter = 0        -- `global` is the only way to make a global

print("hello from", name)
```

Files run top to bottom. A module returns its exports with `return` (or
`export`, see [Modules](#modules)).

## Variables, scope, destructuring

```duka
local a = 1
local b: int = 2
local c, d = 1, 2

global shared = {}

local { x, y } = { x = 1, y = 2 }   -- table field destructuring
assert(x == 1 and y == 2)
```

In the REPL declarations are global by default; in files they are local.

## Control flow

`then` closes an `if` condition, `do` closes a loop header, every block ends
with `end`:

```duka
local n = 7
if n % 2 == 0 then
    print("even")
elseif n > 5 then          -- one word, not `else if`
    print("big odd")
else
    print("small")
end

local i = 0
while i < 3 do
    i = i + 1
end

for k = 1, 5 do            -- inclusive bounds
    print(k)
end

for k = 5, 1, -1 do        -- optional step
    print(k)
end

for key, value in pairs({ red = 1 }) do
    print(key, value)
end

for v in iter.range(0, 3) do
    print(v)
end
```

`break` leaves the nearest loop (or `do` block); `continue` skips to the next
iteration:

```duka
local odds = 0
for i = 0, 9 do
    if i % 2 == 0 then
        continue
    end
    odds = odds + 1
end
assert(odds == 5)
```

## Functions

```duka
function add(x, y)          -- local in a file
    return x + y
end

local function factorial(n)
    if n <= 1 then
        return 1
    end
    return n * factorial(n - 1)
end

local double = fn(x) x * 2             -- `fn` takes ONE expression
local bump = fn(x) do                  -- multi-statement body needs do..end
    return x + 1
end
local anon = function(msg)             -- `function` block body
    print(msg)
end

assert(double(21) == 42)
assert(bump(1) == 2)
anon("hi")
```

Closures capture their enclosing locals:

```duka
local function make_counter()
    local c = 0
    return fn() do
        c = c + 1
        return c
    end
end

local tick = make_counter()
assert(tick() == 1 and tick() == 2)
```

Parameter and return types are optional annotations:

```duka
function inc(x: int): int
    return x + 1
end
```

`...` is vararg; forward it unchanged or index it with `select`:

```duka
function log_all(...)
    print(...)
end
```

## Tables and arrays

Arrays are **0-based**; `#` is the length:

```duka
local arr = [10, 20, 30]
assert(arr[0] == 10)
assert(#arr == 3)

local t = { name = "kit", 99, [100] = 0 }
assert(t.name == "kit")
assert(t[0] == 99)          -- array part starts at 0
```

Three field forms: `{ key = value }`, `{ value }` (array part), `{ [expr] = value }`.

## Operators

```duka
assert(1 ~= 2)              -- `~=` is not-equal (`!=` is invalid)
assert(2 ^ 10 == 1024)      -- power
assert(7 // 2 == 3)         -- floor division
assert(7 % 3 == 1)
assert("a" .. "-" .. "b" == "a-b")   -- concatenation
assert(false or true)       -- and / or / not, truthiness: only nil and false are falsy
```

Comparison: `== ~= < <= > >=`. String concatenation `..` coerces numbers.

Nil-safe access: `?` before `.`, `[`, `:` or `(` guards that one step. A nil left
side stops the chain and gives `nil`; the call's arguments are not evaluated:

```duka
local cfg = { deep = { val = 7 } }
assert(cfg?.deep?.val == 7)
local missing = nil
assert(missing?.deep == nil)   -- short-circuits instead of erroring
assert(missing?[0] == nil)
assert(missing?() == nil)      -- nil-safe call
assert(missing?:m() == nil)    -- nil-safe method call
```

`a?.b()` guards the field read only: the `.b` step gives nil, then the call is
a call on nil and fails. Guard the call too with `a?.b?()`.

Pipeline operator threads a value as the first argument:

```duka
assert(5 |> add(3) == 8)
```

## Strings

```duka
local s = "hi"
local single = 'also fine'
local multi = [=[first line
second line]=]              -- `[[...]]` is NOT supported

assert(string.upper(s) == "HI")
assert(string.split("a,b,c", ",")[1] == "b")
assert(to_string(42) == "42")     -- not `tostring`
```

## Types

Types are annotations checked at compile time, not runtime. They are optional
and mostly used at function boundaries and public module surfaces.

```duka
local count: int = 0
local label: string = "x"
local maybe: int | nil = nil              -- union
local nums: array<int> = [1, 2, 3]
local pick: function(int, int) -> int = add
```

- `nil` is accepted by default (`default_nonnilable = true`); `int!` marks a
  non-nilable value, `int?` a nilable one.
- `any` swallows unions.
- Generics, `where` clauses and type functions exist; prefer simple annotations
  unless the task requires advanced typing.

## Objects

`object` is sugar over tables and metatables:

```duka
object Shape
    name = "shape"          -- field with default

    function init(tag)      -- called by new(); static, `self` is available
        self.name = tag
    end

    function :area()        -- instance method, `self` is the receiver
        return 0
    end

    function :describe()
        return self.name .. ":" .. to_string(self:area())
    end
end

object Square extends Shape
    function init(side)
        self.name = "square"    -- `super` is NOT available in static init
        self.side = side
    end

    function :area()
        return self.side * self.side
    end

    function :base_area()
        return super:area()     -- `super:method()` works in instance methods
    end
end

local s = Square.new(3)         -- new(...) then init(...)
assert(s:area() == 9)
assert(s:describe() == "square:9")
```

- `function name()` is static, called as `Obj.name()`.
- `function :name()` is an instance method, called as `obj:name()`.
- `super:method()` only inside instance methods.
- `__tostring`, `__index`, etc. can be declared as metamethods in the body.

## Modules

`require` accepts a relative path or a package name (looked up in the kao's
`modules/`, or `DUKA_PATH`):

```duka
local m = require("./lib/mathx")        -- relative to this file
local { add } = require("./lib/mathx")  -- destructuring import
local pkg = require("mylib")            -- installed package
local sub = require("mylib.sub")        -- nested module
```

Export from `lib/mathx.duka`:

```duka
export local version = "1.0"

export function add(a, b)
    return a + b
end

export object Point
    function init(x, y)
        self.x = x
        self.y = y
    end

    function :len()
        return math.sqrt(self.x * self.x + self.y * self.y)
    end
end
```

`export` compiles to a `_EXPORTS` table plus a trailing `return`, so a module
can also just `return { ... }` manually. A package's own `kao.toml` decides
which file is its root module (default `src/main.duka`); `init.duka` at the
package root also works.

`@declare export` gives the importer a typed signature while the module itself
defines it elsewhere (or not at all):

```duka
-- m.duka
@declare export function strlen(s: string): int
```

## Errors: the result protocol

Fallible functions return `(true, ...)` or `(false, message)`.
`is_error` checks **both slots**, it is not a one-argument predicate:

```duka
local ok, err = try(fn() error("boom"))
assert(ok == false)
assert(is_error(ok, err))
assert(err == "boom")

local ok2, val = try(fn() 42)
assert(ok2 == true)
assert(unwrap(ok2, val) == 42)
```

Writing a fallible function:

```duka
function parse_positive(n)
    if n > 0 then
        return true, n
    end
    return false, "not positive"
end

local good, got = parse_positive(5)
assert(good and got == 5)
```

- `error(msg)` raises.
- `assert(cond, msg)` raises when falsy.
- `unwrap(ok, value)` returns the value or raises the message.
- `expect(value, msg)` raises when the value is `nil`.
- `@returns(result)` / `@returns(exit)` tell the checker what a native function's
  return slots mean.

## Standard library

Everything is namespaced except core globals. Call it as `math.floor(x)`, not
`floor(x)`. Full signatures: [references/stdlib.md](references/stdlib.md).

Globals: `print`, `assert`, `error`, `require`, `pairs`, `ipairs`, `typeof`,
`instanceof`, `to_string`, `to_number`, `is_error`, `unwrap`, `expect`, `try`,
`select`, `clone`, `curry`, `get_metatable`, `set_metatable`.

Namespaces: `math`, `string`, `array`, `table`, `iter`, `regex`, `os`, `io`,
`json`.

## Pitfalls

See [references/gotchas.md](references/gotchas.md) for the full list. The
ones that break code most often:

1. `elseif` is one word; `else if` is a syntax error.
2. A multi-statement `fn` body must be `fn(...) do ... end`; `fn(x)` alone takes
   one expression.
3. `object` `init` is static: `self` works, `super:init(...)` does not. Use
   `super:method()` only in instance methods.
4. `is_error(ok, err)` takes both result slots; `is_error(err)` is always false.
5. Arrays are 0-based; `#arr` works, `arr.length` does not.
6. Inequality is `~=`, not `!=`.
7. `tostring` is `to_string`.
8. Multi-line strings use `[=[ ... ]=]`; `[[ ... ]]` is gone.
9. `match` clauses do not fall through to the next statement: a completed
   clause exits the enclosing function/chunk. Prefer `if`/`elseif` unless you
   are certain of the match semantics you need.
10. Types are compile-time only. A wrong annotation is a compile error, never a
    runtime check; do not rely on them for validation.
