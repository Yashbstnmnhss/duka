# DUKA‘s Interpreter

Based on lua's grammar, Duka is a _lightweight_ programming language.

## Not Done Yet

See [memo](./memo.md)

## Timeline

| Status | Parts        | Date      |
| :----: | :----------- | --------- |
|  Done  | Lexer        | 2025.7.13 |
|  Done  | Parser       | 2025.7.23 |
| v.ing  | Codegen      |           |
| v.ing  | VM           |           |
|  Done  | Instructions | 2025.7.11 |
|        | Cli          |           |
|        | Std Lib      |           |
|  Done  | Macros       | 2025.7.14 |

## Something Weird

### `logic` (in plan now)

you can use logic programming in duka now

```lua
logic! {
    fact
    rule
}
```

and query it in expression

```lua
solution = logic! { query  }
```

## Extended Grammar (in plan)

### `match` Grammar

shall i introduce new keyword in?

also shall i implement a _powerful_ pattern matching?

```lua
match ...
case ... then ... break
end
```

### Pipeline Grammar (Done)

```lua
param |> func
```

#### already supported

in expression, it behaves normally `a |> f`

but when in statment, where didnt allow expression directly,
you need to wrap it with `()` in order to make a **expression statment**
`(a |> f |> f2)`

i admit that it is kind of weird looking, but the very reason i can give is that **it hard and complicated to parse a statment with expression or argument list in head without anything else to recognize directly**

moreover, this only support one parameter in left

because im lazy to implement the tuple one

### ~~`...` Grammar~~ (Passed)

this may be passed

because i dont want to

```lua
[...array, 1] => flat the array
{ ...obj, a = 1 } => "with" grammar
```
