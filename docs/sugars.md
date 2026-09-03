# Sugars in Duka

## Pipeline

There are two types of pipeline:

1. left-to-right pipeline `|>`
2. right-to-left pipeline `<|`

The default behaviour of two pipelines is to insert the left(right) expression into the nearest position in arguments for the pipeline.

The target can be either an call or an expression else.
For an already call expression, pipeline will insert a new expression into its arguments list, otherwise, it will make a call expression based on given target.

```lua
a |> f == f(a)
a |> f(1) == f(a, 1)
f(2) <| a == f(2, a)
```

### Positioned Pipeline

Only available for left-to-right pipeline `|>`
You can give a certain position where to insert the expression to:

- Default(Zero): `|>` `|0>`, insert at the head of list
- Numeric: `|n>`, insert at `n`, the gap will be filled with `nil` (if has). `a |2> f` => `f(nil, nil, a)`
- Tail: `|$>`, append(insert at the end). `a |$> f(1, 2)` => `f(1, 2, a)`

## Context Computation

Now, you can cover a batch of operations into a `do` block with a given context.

```lua
local a = Context! do
    ...
end
```

To use features that current context provides, you need to add a `!` marker after keywords, supported keywords (and its method in context) are:

|      Keyword      | Method in Context |
| :---------------: | :---------------: |
|     `local!`      |     `__bind`      |
|       `do!`       |     `__bind`      |
| `return` (no `!`) |    `__return`     |
|     `return!`     | No method needed  |
|    `for! in`\*    |     `__forin`     |
|    `while!`\*     |     `__while`     |
| _required by_ \*  |    `__combine`    |

## Match

## Export

## Object

## Linq
