---
title: Types
description: Annotations, nilable, unions, generics and where clauses.
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
| `any`    |                  | accepts all       |
| `never`  |                  | accepts nothing   |

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

## Metatable

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
