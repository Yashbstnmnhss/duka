---
title: Macros & bang
description: \`!\` blocks, linq!, logic! and static replacement.
---

An identifier followed by `!` is processed specially — a **bang**. The handler
is chosen by the keyword in front of it.

```duka
linq!(...)
logic! { ... }
```

## `linq!`

An expression, not a statement:

```duka
global list = linq!(
    from x in array1
    where x > 0
    from y in array2
    select x * y
)
```

turns into

```duka
global list = do
    _s_list = {}
    _s_index = 0
    for x in array1 do
        if x > 0 then
            for y in array2 do
                _s_list[_s_index] = x * y
                _s_index = _s_index + 1
            end
        end
    end
    return _s_list
end
```

Up to now only `where` and `from` clauses are supported. A valid expression
starts with at least one `from ... in ...` and ends with a single `select`.

## `logic!` (WIP)

```duka
logic! {
    fact ...
    rule ...
}
```

and query it in an expression:

```duka
solution = logic! { query }
```

## Static replacement

Tokens, not text. Everything happens at lexer time, so a replacement works with
tokens as the basic unit.

```cpp
^#define PI -> 3.1415926;
...
a = [:PI:]
```

`->` captures tokens until `;` by default. To capture more, drop the arrow and
mark the end with `^#enifed`. `undef` removes a defined macro.

Replacement variables match identifiers starting with `$`:

```cpp
^#define MAX(a, b)
if $a >= $b then $a else $b end
^#enifed
```

`[:MAX(1, 2):]` becomes `if 1 >= 2 then 1 else 2 end`.

### Meta macros

Inside the splicer, names ending with `!`:

- `nameof!(x)` — the name of the first token parameter
- `stringify!(x)` — stringify input tokens, only **x**
- `concat!(...)` — concat **only identifiers** into one identifier token
- `when!(a, b, c)` — `a` is exactly a `true` token, then `b`, else `c`
- `nonempty!(x)` — `true` token when **x** holds at least one token

### Vararg

`...` in a macro declares a vararg, it must be the last parameter. `$...`
expands it separated by `,` by default, a token in `[` `(` gives the separator:

```cpp
^#define A1(...) -> {$...[;)};
^#define A2(...) -> {$...[;]};
^#define A3(...) -> {$...(;)};

[:A1(1,2,3):] -- {;1;2;3}
[:A2(1,2,3):] -- {;1;2;3;}
[:A3(1,2,3):] -- {1;2;3}
```

### Recursion

`[:~macro(params):]` expands lazily, up to a depth of 64. Cycles are forbidden,
nesting is fine.

Attention: only valid tokens are supported instead of raw text. A string must be
a complete `""`, a lone `"` is not a token — while `[` `]` `(` `)` are
independent tokens and can appear separately.
