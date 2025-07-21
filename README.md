# DUKA‘s Interpreter

Based on lua's grammar, Duka is a _lightweight_ programming language.

## Not Done Yet

See [memo](./memo.md)

## Timeline

| Status | Parts        | Date      |
| :----: | :----------- | --------- |
|   完   | Lexer        | 2025.7.13 |
|   正   | Parser       |           |
|        | Codegen      |           |
|        | VM           |           |
|   完   | Instructions | 2025.7.11 |
|        | Cli          |           |
|        | Std Lib      |           |
|   完   | Macros       | 2025.7.14 |

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

### `....` Grammar

```lua
[...array, 1] => flat the array
{ ...obj, a = 1 } => "with" grammar
```
