---
title: Type context
description: The second context, where types are values.
---

## Two contexts

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

:::note
`type function` and `function type` are two different things, the former is a callable type in type context, while the latter is an exact type representing a function signature.
:::

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

## Concepts

Concepts are type function whose result will be interpreted as boolean value to get `true` or `false`. A normal type function can be used asa a concept, depend on its return value's [truthiness](../expressions#truthiness).

### `where`

Concept type function can be applied in `where` clause. It will try to call this type function and pass the type checking if all concepts returned `true` in truthiness. See [here](../functions#parameters--returns)

### `provide`

The pseudo function `provide(...)` can attach a constraint to a type parameter in current type function, similar to `ensure ...` return annotation in value-context function.

```duka
type function Abc(t)
    provide(NonNil(t))
end
```

By providing concepts as constraints to certain type parameter, the type checker will memorize them and provide a better checking for type correctness.

## See also

- [types](./types.md) - which are seen as literals in type context
- [type-context](../reference/type-context/) — the standard library side for type context
