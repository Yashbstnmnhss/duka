---
title: Match
description: Pattern matching as a statement or an expression.
---

```duka
match <target> then
    1 -> print "true";-- also nil
    {1, ..., [a] = 1} -> not false;
    [1, ..., 3, _ * 2, 5] -> error("NO");
    true if false -> print "never";
    local b: int | string -> print("type match: ", b);
    2 or 3 or not 4 -> 2;
    |> check() and |> check2("s") -> do
        local a = 1
        a = 2
        return a
    end
    > 1 or < 2 -> do print "not 1 or 2" end
else
    <exhausted>
end
```

`<exhausted>` is required when `match` is an expression, same as `if`
expression.

## Pattern terms

- Constant(val) — `literal value`
- Bind(to, type?) — `local to: type`, binds to a local, checks the type if it
  has one
- Guard(term, expr) — `<term> if ...`
- Compare(op, expr) — `> value`
- Compound(term, term, op) — `<term> and/or <term>`
- MethodCall(func, params, op) — `|> function(...)`
- Not(expr) — `not <term>`
- List-Table(array, map)
- Array(array)

`and` / `or` / `xor` / `not` combine patterns the way they combine booleans.

## Ignoring items

For List-Table and Array, `...`, `_` and `_ * n` ignore one, several or a count
of items (`n` is a number, index based). The count of `...` must be less than
one.

```duka
{ first, ..., last }
{ _, second, ... }
{ _ * 3, fourth }   -- len = 4
```

## In type context

A `match` in [type context](/language/type-context/#match) has one more form,
**TypePattern**, which infers a type:

```duka
type function InferItem(who)
    return match who then
        array(local U) -> U;
    else
        return never
    end
end
```
