---
title: Functions
description: function, fn, vararg and ensure.
---

## Defining

```duka
function name(a, b)
    ...
end

function global g() end
function local f() end
```

```duka
(global|local)? function <name>(<param-name-type>*)(: <return-type>)?
    ...
end
```

## Lambda

`fn(params) expression` takes a single expression as its body:

```duka
local b = c |> map(fn(x) x * 2)
```

With `do end` it reads like a `function` too:

```duka
local b = c |> map(fn(x) do return x * 2 end)
```

Anonymous form: `function() ... end`.

`function`, `fn` are preserved keywords, `func` is not — all three are used in
type annotation, see [function type](/language/types/#function-type).

## Parameters & returns

```duka
function B<T, V>(a: T, b: array<T!>!, ...): (int, C, ...)
    where
        T: [V],
        Concept({}),
        type [C, _] = [V, T, true]
    return 1, false
end
```

- parameter and return types are annotations, `...` is vararg
- `(int, C, ...)` is a tuple return, plain `: int` is a single one
- `...` on its own in return position returns dynamic, `-> ...: name` names the
  collected values
- `where` is the [constraint list](/language/types/#where)

```duka
function f(b: ...): (int, int, ...)
    local cc = [...]   -- collect varargs into an array
    return 0, 0
end
```

## Generics

```duka
function abc<T>() end
function abc2<T>(...) end          -- bound with the `...` vararg
function ACCC<...g>() end          -- vararg type parameter
function TESTTEST<...Ts>(...: Ts): ...Ts
function CONST<const T>(v: T!): T  -- const type parameter
```

A generic call in an expression needs the `.` prefix: `abc.<int>()`.

## `ensure`

A signature can carry a concept after its returns — a condition on the call
rather than a body of code:

```duka
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
```

Concepts live in [type context](/language/type-context/).

## Closures

A local captured by an inner function is an **upvalue**. It starts open (still
in its place) and is closed when the closure takes it.

```duka
local function counter()
    local n = 0
    return fn() n = n + 1 return n end
end
```

Index starts from 0, unlike lua.
