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

## Parameter kinds

A parameter of a type function is a type, and it may say which **kinds** it takes. Written with no kind, it takes any type.

```duka
type function First(t: type array): type array
    return t
end

type function OnlyInt(t: int)        -- a subtype of int
    return int
end

type function Literal(n: const int)  -- a literal int
    return n
end
```

- `type` takes any type, the way `any` takes any value.
- A bare type (`int`, `MyAlias`, `array<int>`) is a **subset bound**: the argument must be a subtype of it.
- `const` asks for a literal type, `const int` for a literal of that base.
- `type <shape>` asks for a shape: `object`, `array` (or `list`), `table`, `record`, `tuple`, `union` or `pattern`, with optional slot kinds like `type array<type fn>`.
- `type fn(...)` asks for a type function.
- `K1 | K2` takes either kind.

A parameter whose argument is not decided yet -- `any`, `unknown`, a type variable -- passes every kind; only a decided argument that does not fit is refused.

The return annotation takes the same spellings, and then the result is checked against it: `: type array` refuses a function that answers `int`, while `: type` accepts anything. A plain type written there (`: int`) is only a signature, shown by the language server but not enforced.

See [kinds](./types.md#kinds) for the whole list and the examples.

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

:::caution[Knowing Issues]
`break` and `continue` with **label** are not supported yet in type-context.
:::

:::note
Unlike functions in value-context, functions in type-context cannot return multiple values directly like `type fn() -> (A, B, C)`.

However, you can still use `TypeTuple` (like `[int, string]`) to simulate it, that's how `return` and `break` with multiple expressions work in type-context.
:::

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
