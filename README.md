# DUKA‘s Interpreter

Based on lua's grammar, Duka is a _lightweight_ programming language.

## Not Done Yet

See [memo](./memo.md)

## Extended Grammar (in plan)

### `match` Grammar

```lua
match ...
case ... then ... break
end
```

### Pipeline Grammar

```lua
param |> func
```

### `...` Grammar

```lua
...params => params[]
[...array, 1] => flat the array
{ ...obj, a = 1 } => "with" grammar
```
