---
title: Types
description: Annotations, nilable, unions, generics, labels, units and patterns.
---

Duka has a type system based on annotations, a **compile-time** system instead
of a checker in runtime.

:::caution[Knowing Behaviour]
Because of the design of lua language, duka's type system is not such strict as typescript's checker does, it still allows you to access, assign something might not defined in current scope. In other words, this system prefers to check exactly known things than other possible dynamic operations.
:::

## Annotations

```duka
local a: int = 1
function b(c: string): bool
    return false
end
```

You use it by adding a `:`.

## Nilable & nonnilable

- With `default_nonnilable = true`, all values are nonnilable in default. Write
  `type?` to mark one nilable, same as `type | nil`.
- With `default_nonnilable = false`, everything accepts `nil` in default. Write
  `type!` to mark one nonnilable.

`default_nonnilable` is **false** by default.

```duka
-- default_nonnilable = false
local a: int = nil -- Ok
local b: int! = nil -- TypeMismatchErr
```

```duka
-- default_nonnilable = true
local a: int = nil -- TypeMismatchErr
local b: int? = nil -- Ok
```

## Union type

Make one with `|`. `any` cannot sit in a union, the whole thing collapses into
a single `any`.

```duka
local a: int | string = 0
```

## Intersection type

Almost used with table-like types, it takes intersection of two given types.

```duka {3}
type A = { b: bool }
type B = { a: int }
type C = A & B -- { a: int, b: bool }
```

## Basic types

Types have many aliases:

| type     | aliases          | note              |
| -------- | ---------------- | ----------------- |
| `int`    | `integer`        |                   |
| `float`  | `num`, `number`  | accepts `int`     |
| `string` | `str`            |                   |
| `bool`   | `boolean`        |                   |
| `table`  |                  | accepts `object`  |
| `array`  | `list`           | `[]`              |
| `func`   | `function`, `fn` |                   |
| `nil`    |                  | only `nil` itself |
| `any`    |                  | accepts all, and is accepted by all |
| `unknown`|                  | accepts all, gives back nothing |
| `never`  |                  | accepts nothing   |

## `any` and `unknown`

`any` is dynamic: it stands in both directions, so a value of type `any` can be
used as anything.

`unknown` is the top type: every value is one, but nothing comes out of it
without naming it first.

```duka {4,7}
local a: any = 1
local b: int = a -- Ok

local c: unknown = 1
local d: int = c -- TypeMismatchErr
local e: int = as.<int>(c) -- Ok
```

## Special types

There are two kinds of special types both used in type-context and value-context.

- `TypeTaple`: `{ a: bool }`
  A table-like type.
    - In value-context: a table with its fields' type exactly known
    - In type-context: acts as a table **value** like what in value-context
- `TypeTuple`: `[string, int, ...]`
    - In value-context: an array with its items' type exactly known.
      And when the last item is `...`, it will skip checking elements after that.
    - In Type-context: acts as an array **value** like what in value-context

## With some marks

Types support to carry some kinds of marks.

### With `Metatable`

`object`, `table` and `TypeTable` (table-like type) supports to carry on its metatable's type.
Two built-in type functions `WithMetatable` `GetMetatable` provide ability to fetch or modify metatable's type for given table-like type. And built-in functions like `set_metatable` use them as return value's type.

```duka {5}
local b = {
    __len = function() return true end
}
local a = set_metatable({}, b)
-- Now a is "{} + metatable { __len: function() -> bool }"
local c: bool = #a
```

### With `Brand`

A brand is a name a type carries. `WithBrand(T, "name")` puts one on any type.

```duka del={3}
type UserId = WithBrand(int, "user")
local a: UserId = as.<UserId>(1) -- Ok
local b: UserId = 1              -- TypeMismatchErr
```

The rendered type writes the name after the type, `int ::user::`, which is what
the language server shows and what a diagnostic prints.

- A labelled value **is** its base: `local n: int = a` is fine.
- A plain value is **not** labelled: `local u: UserId = n` is refused.
- A literal is not read as a label either. `as` is the only way to make one.

A label has no algebra: `a + a` is another `UserId`, but `a * a` is a type
error, because an id is not a quantity. If you want multiplication, you want a
unit.

### With `Unit`

`Unit(T, "m/s^2")` is a special label with algebra. Only `int` and `float` can wear one.

```duka {1,2}
type Speed = Unit(float, "m/s")
type Accel = Unit(float, "m/s^2")
```

Its arithmetics are the ones dimensional analysis gives:

| operation  | rule                                                      |
| ---------- | --------------------------------------------------------- |
| `+ - % //` | both sides share the unit, the result keeps it            |
| `*` `/`    | units combine: `m * s` is `m·s`, `m / m` is dimensionless |
| `^ n`      | `n` is a constant whole number, the exponents multiply    |
| `sqrt`     | the exponents halve, an odd one is refused                |

A dimensionless result is the same type as carrying no label at all, so
`(m/s) / (m/s)` is a plain `float` and so is `speed ** 0`.

A **literal** is dimensionless, so it can initialize a unit: `1.0` written where
`m` is wanted is `1.0 m`. A variable cannot: `local n = as.<float>(1.0)` stays
`float`, and passing it to a `Speed` parameter needs `as.<Speed>`.

```duka
local v: Speed = 1.0         -- Ok, a literal
local w = as.<Speed>(2.0) ^ 2 -- float ::m^2/s^2::
```

An exponent that is not a constant is refused **when the base carries a unit**:
the result's unit would be `u^n`, and `n` is not known. Take the label off with
`as.<float>` first if that is what you meant. An exponent that carries a unit is
refused too — `m/s` to the power of `m/s` is not a quantity.

:::caution[Whole exponents only]
The unit group has integer exponents, so `m ** 0.5` has no spelling. Use `sqrt`
for the half power; it is the same operation with a written-down rule.
:::

`abs`, `floor`, `ceil`, `round`, `clamp`, `min`, `max` and `sum` keep the unit
they were handed. String, json and io builtins do not know about units: they
read a labelled value as its base.

### With `Pattern`

A pattern is a value constraint carried by a type, written `int*`. It is the
value-level counterpart of a `where` clause: a `where` is a condition on types
and is resolved at once, a pattern travels with the type and is only read when a
value meets it.

There are only two kinds:

| family | constructors                                                                                                          |
| ------ | --------------------------------------------------------------------------------------------------------------------- |
| number | `GtPat(int, 5)`, `GePat`, `LtPat`, `LePat`, `EqPat`, `NePat`                                                          |
| text   | `HasPat(string, "@")`, `StartsPat`, `EndsPat`, `RegexPat`, `LenGtPat`, `LenGePat`, `LenLtPat`, `LenLePat`, `LenEqPat` |
| both   | `AndPat(a, b)`, `OrPat(a, b)`, `NotPat(a)`                                                                            |

```duka {1,2} del={8}
type Port = AndPat(GtPat(int, 0), LePat(int, 65535))
type Mail = HasPat(string, "@")

function open(p: Port): int
    return 1
end

open(70000) -- TypeMismatchErr: Expected int*, but got int
```

A pattern is checked when the value is known at compile time:

- a literal that does not satisfy it is an error;
- a value nobody wrote down — a variable, a call — is let through;
- nothing happens at runtime.

A pattern is transparent: `int*` and `int` stand in for each other in both
directions, so an annotation can be loosened to the base and a base value can be
passed where a pattern is wanted.

:::caution[Knowing limitations]
It is a promise that the checker keeps where
it can, not a tag on the value. It also does not enter containers: an element of
an `array<Port>` is not checked one by one.

```duka del={1} add={2}
local p: Port = 70000 -- TypeMismatchErr
local a: array<Port> = [70000] -- Ok, elements are not checked
```

:::

A label and a pattern compose, and the label is outside:

```duka
type Port = WithBrand(LePat(int, 65535), "port") -- int* ::port::
```

## Function type

```duka
function(bool)                      -- no return
function() -> int                   -- one return value
function(...) -> ...                -- dynamic return
function(int, string, ...) -> (int, bool)  -- tuple return
```

The validation is not strict. VarArg isn't checked, and the parameter list may
be longer or shorter — only the **common part** is validated.

```duka
func(int, ...) <=> func(int, bool, string) -- OK
fn(int) <=> fn(int, bool, ...)             -- OK
```

## Object type

```duka
object A extends B
    ...
end
```

```duka
local instance: A = A.new() -- type is object
local parent: B = instance  -- upcast
```

See [objects](../objects).

## As

- [`as`](../reference/core)
  Convert into another type without any checking
    ```duka
    local d: int = c |> as.<int>
    ```
    Multiple values are supported.
- [`as_const`](../reference/core)
  Prefer to convert into literal type if possible
    ```duka
    local d = 2 |> as_const -- d: 2
    ```

## Generic

```duka
function abc<T>() end
function abc2<T>(...) end -- automatically bound with the `...` vararg

local a: array<int> = [1,2,3]
local b: table<string, bool> = { c = true }
```

Both `<` and `>` are valid tokens in an expression, so a generic call in an
expression needs a `.` prefix:

```duka
abc.<int>()
A.<string>.static()
```

### `where`

A generic list may carry constraints:

```duka
function f<T>(x: T) where T: Point, Sized(T), type V = T
```

- `U: Point` — a subtype bound, and the body is checked against it
- `Sized(T)` — a concept, a type-level expression read for its truth. `nil` and
  `false` fail, everything else answers
- `type V = T` — a type-level binding, `type {A, B} = T` binds more than one

The whole list is optional, each clause is checked on its own.

## See also

- [Type context](/language/type-context/) — `type`, type functions, HKT
