---
title: Statements
description: Declarations, blocks and control flow.
---

## Local & global

In lua every variable is global without `local`, which is a confusing design.

Duka goes the other way: any variable is local implicitly, and `global` is the
**only** way to declare a global one.

```duka
global a = 1
local b = 2
```

```duka
(global|local) <name-type> (,<name-type>)* (= <expr> (,<expr>)*)?
<name> = <expr>
```

## Destructuring

```duka
local { a, b, c } = expr
```

is equivalent to

```duka
local a, b, c = do
    local <name> = expr
    return <name>.a, <name>.b, <name>.c
end
```

Which is what makes importing read well:

```duka
local { a1, a2 } = require("A")
```

## Blocks

`do ... end` is a block and an expression at once, so an IIFE is just

```duka
local f = do
    ...
end
```

`if`, `while`, `for`, `match` each open a block, with `then` closes the if-like condition, `do` closes the loop condition and `end` closes every block.

:::note
`if`, `do` block can be placed in expression position, with its result returned by `break` instead of `return`

```duka {3, 6}
function f()
    local a = do
        break 1, 2 -- these to a
    end
    local b = do
        return 1, 2 -- these break f()
    end
end
```

:::

### Block with context

See [here](../macros).

## Control flow

- `if` / `elseif` / `else`
- `while`, `for` (numeric and generic with `in`)
- `break` leaves the nearest `do` block or loop, `break 1, 2` leaves with
  values — only a `do` block has somewhere to put them
- `continue` skips to the next iteration
- `goto` jumps to a visible label `::A::`
- `return` may be written anywhere in a block, whatever follows it is
  unreachable

Each keyword has its own entry in [Keywords](../keywords/).

## Named blocks

These blocks can have a name:

- `do` block - with `break values`
    ```duka
    do ::name::
        break ::name:: values
    end
    ```
- `for`, `while` block - with `break` `continue`
    ```duka
    for ::name:: ... do end
    while ::name:: ... do end
    ```

## `match`

`match` is a statement or an expression, see [Match](../match/).
