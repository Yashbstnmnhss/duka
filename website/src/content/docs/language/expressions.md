---
title: Expressions
description: Literals, pipeline, nil check and operators.
---

## Literals

```duka
local n = 1
local s = "hi"
local t = { a = 1, 2, [3] = 4 }
local a = [1, 2, 3]
local e = []
```

Three field forms in a table: name-value `{name = value}`, array-like
`{value}`, key-value `{[key] = value}`. Array literal is 0-based, both `,` and
`;` separate elements, trailing separator is allowed.

An array literal creates a fresh table on every execution, same as `{}`.

### Truthiness

Values can be interpreted as boolean value implicitly if needed.
Values like `nil`, `false` turn into `false`, otherwise `true`.
In [type context](./type-context), `never` and `nil` (literal type) turn into `false`.

## Calls & paths

```duka
f(1, 2)
a.b.c
a:b(1)      -- method call, `self` is passed
a[1]
```

`a.b.c` reads one dot at a time, so `RequireType("m").Outer.inner.deep` is
checked step by step.

## Nil check

`?` in front of `[` `.` `:` `(` chains a nil check: the access happens only when
the left side isn't nil, otherwise the chain stops and gives `nil`.

```duka
local v = a?.b
local w = a?[1]
local x = a?:method()
```

The same marker sits in a type position, where it means `T | nil` — see
[nilable](/language/types/#nilable--nonnilable).

## Pipeline

```duka
a |> f == f(a)
a |> f(1) == f(a, 1)
f(2) <| a == f(2, a)
```

The default behaviour of both pipelines is to insert the left (right) expression
into the nearest position in the argument list. The target can be a call or any
other expression — a non-call becomes one.

### Positioned pipeline

Only for `|>`, say where the argument goes:

- Default (zero): `|>`, `|0>` insert at the head
- Numeric: `|n>` insert at `n`, the gaps are filled with `nil`.
  `a |2> f` => `f(nil, nil, a)`
- Tail: `|$>` append. `a |$> f(1, 2)` => `f(1, 2, a)`

Only one expression on the left (or right), there is no tuple form.

:::Warning

In a statement position a bare pipeline can start a line, but make sure there won't create an **ambiguity** caused by function calling without `()` in pipeline.

```duka del={1,2,5,6}
a |> f |> f2
(1+1) -- turns into "a |> f |> f2(1+1)"

-- Wrap the bare expression like this
(a |> f |> f2)
(1+1) -- turns into f2(f(a))(1+1)

-- Use ";" to separate them
a |> f2;
(1+1) -- Ok, this is a wrapped expression in statement position
```

:::

## Operators

From loosest to tightest binding:

| level | operators                   |
| :---: | --------------------------- |
|   1   | `or`                        |
|   2   | `and`                       |
|   3   | `xor`                       |
|   4   | `==` `~=` `<` `<=` `>` `>=` |
|   5   | ` >` `< `                   |
|   6   | `\|` `~` `&`                |
|   7   | `<<` `>>`                   |
|   8   | `..` (right)                |
|   9   | `+` `-`                     |
|  10   | `*` `/` `//` `%`            |
|  11   | `^` (right)                 |

Unary: `not`, `-`, `~`, `#`.

Comparison only returns `bool`, `and` / `or` / `xor` are the logical ones.

## Statement as expression

In duka, these following statement can be used at expression position:

- `match` with `else`
- `if` with `else`
- `do end` (for temp scope with `break ...` values)

All of them return their expression values by `break ...`, named `do` block is supported by `break ::block_label:: ...`. To perform a return for current function, just use `return ...`. To break/continue a certain loop, use `break/continue ::loop_label::` without values.
