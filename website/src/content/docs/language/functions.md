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
type annotation, see [function type](../types/#function-type).

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
- `where` is the [constraint list](../types/#where)

```duka
function f(b: ...): (int, int, ...)
    local cc = [...]   -- collect varargs into an array
    return 0, 0
end
```

### Generics

```duka
function abc<T>() end
function abc2<T>(...) end
function ACCC<...g>() end          -- vararg type parameter
function TESTTEST<...Ts>(...: Ts): ...Ts -- bound vararg with vararg type parameter
function CONST<const T>(v: T!): T  -- const type parameter
```

A generic call in an expression needs the `.` prefix: `abc.<int>()`.

There are two kinds of prefixes:

- `const`:
  Prefer to infer this generic as a literal type instead of a normal type.

    ```duka {3}
    function test<const T>(t: T) end
    test(1) -- T = 1, not int
    ```

- `...`:
  Mark this type generic as a var-arg type parameter like `...` in value-context, and it turns into a type tuple. When it is combined with `...` in parameters, it will infer/check all types in `...` respectively.

    ```duka del={6}
    -- infer
    function test<...Ts>(...: Ts) end
    test(1, true, "abc") -- Ts = [int, bool, string]
    -- bound
    function test2<...Ts: [int, int, bool]>(...: Ts) end
    test(1, true, false) -- Type error, the second one should be int
    ```

    In other positions, `...Ts` acts like a expander.

    ```duka
    function test<...Ts>(...: Ts): (bool, ...Ts)
        -- ...
    end
    local a, b, c, d = test(1, true, "abc") -- returns a: bool, b: int, c: bool, d: string
    ```

    When a `...` appears alone in type tuple, it means ignoring check after this item.

    ```duka del={2}
    local d: [bool, int, ...] = [true, 2, "string", 1.2] -- Ok
    local d2: [bool, int, ...] = [2, true, "string"] -- Error
    ```

### `ensure`

A signature can carry a concept after its returns — a condition on the call
rather than a body of code:

```duka
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
```

Concepts live in [type context](../type-context/).

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
