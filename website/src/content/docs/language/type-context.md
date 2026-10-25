---
title: Type context
description: The second context, where types are values.
---

## Concepts

- **type-context**: statements & expressions that start with the `type` keyword
- **value-context**: normal code

In type-context types are treated as values (at compile time). Everything is
**erased** before compiling into bytecode.

```duka
type Value = int

type function Nullable(who)
    return who | nil
end

type ValueWithNil = Nullable(Value) -- int|nil
```

A type function is like TypeScript's util type, one argument, returns a type.

## Interaction between the two contexts

- type-context -> value-context: by [annotation](/language/types/#annotations)
- value-context -> type-context: by `type(expr)`

```duka
local a = 0
type B = type(a) -- int
```

## Expressions

Almost the same as value-context, with a few specials. An anonymous type
function is `type function() ... end` or `type fn() ...`, which is what enables
**HKT** (higher kinded types).

## Statements

Only `for` (numeric & generic), `while`, `if`, `match` and variable definitions
& assignments are allowed here.

### Loops

A loop runs at most 1000 times.

### Match

`match` in type-context has one more pattern — **TypePattern**, which infers a
type from a shape:

```duka
type A = array<int>

type function InferItem(who)
    return match who then
        array(local U) -> U;
    else
        return never
    end
end

type B = InferItem(A) -- int
```

That works for user-defined type functions too:

```duka
type function C(a)
    if a == int then
        return string
    else
        return a
    end
end

type function Infer(who)
    return match who then
        C(local U) -> U;
    else
        return never
    end
end

type D = C(int)  -- string
type A = Infer(D) -- int
type B = Infer(string) -- never
```

For basic types the infer system just takes the generic arguments. For
user-defined ones it **looks up** a cache table by the given type's hidden tag
and the pattern name.

Other supported patterns: constant, compound, guard, not, bind.

## See also

- [type-context](/reference/type-context/) — the standard library side
