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

### `!` Block

Now, an identifier along with a `!` mark will be processed specially

#### `logic!` (in plan now)

You can use logic programming in duka now

```lua
logic! {
    fact ...
    rule ...
}
```

and query it in expression

```lua
solution = logic! { query  }
```

### _"Static ~~Reflection~~ Replacement"_

Inspired by the spectacular feature "Static Reflection" in C++26, (in particular its beautiful symbols as well as syntax) I made this essential decision that **Static Replacement is bound to be introduced in duka**

Back to the point, so now you can get tokens and store them by operator `^#` and keyword `define` (`enifed`) and `undef` (remove a defined macro)

This will be processed in lexer time, which means its replacement works with tokens as basic unit

To apply a macro, just use the **splicer** `[: ... :]`

```cpp
^#define PI -> 3.1415926;
...
a = [:PI:]
```

The `->` will just capture tokens until `;` in default
To capture more tokens, remove arrow and use `^#enifed` to mark the end

Replacement variables are supported in macro, it will automatically match identifier starting with `$` symbol

```cpp
^#define MAX(a, b)
if $a >= $b then $a else $b end
^#enifed
```

The `[:MAX(1, 2):]` will be replaced by `if 1 >= 2 then 1 else 2 end`

Also, there exists some meta macro in splicer, which ends with `!`:

-   `nameof!` will return the name of the first token parameters input
    ```lua
    [:nameof!(a):] -- "<identifier>"
    ```
-   `stringify!` stringify input tokens
-   `concat!` will concat **only identifier** (other will be ignored)
    input tokens to one identifier token
-   `when!`
-   `nonempty!`

Moreover, you can use `...` in macro to declare a vararg, it must be the last one in parameters;

To expand it, use `$...` to expand them into sequence separated by `,` as default
Need to custom it, add `[<token>]` after that, then the separator will be the single token in `[` or `(` and `)` or `]`:

```cpp
^#define A1(...) -> {$...[;)};
^#define A2(...) -> {$...[;]};
^#define A3(...) -> {$...(;)};

[:A1(1,2,3):] -- {;1;2;3}
[:A2(1,2,3):] -- {;1;2;3;}
[:A3(1,2,3):] -- {1;2;3}
```

To expand recursive macro, use `[:~macro(params):]` for lazy expanding

Up to now, the limitation of depth of expanding is `64`

Attention, only valid tokens are supported instead of raw text replacement

For instance, **string** must be a complete "" instead of a single quote `"`, which is an invalid token, but things like `[` `]` `(` `)` etc. can appear separately cause they are independent tokens respectively

Cycled recursion is forbidden, but nested macro will be dealt rightly

It's ~~useless~~ **cool**, isn't it?

## Extended Grammar (in plan)

### Better `local` and `global` (done)

In the original lua, all variables are global defined without `local` keyword

it is for sure a very confusing design

Now, any variables are local defined implicitly

Meanwhile, a explicit keyword `global` has been introduced in, which is the **only** way now to declare a global variable

### Extended `attr` (done)

Now you can use attr for function

```lua
function<abc> abc()
...
```

and multiple attributes are supported

```lua
local a <abc, ccb> = 1
```

### Module System (in plan)

I dont know how to do

the only progress i made is `module` had been created for preserved keyword

### Modern OOP (in plan)

Since lua has been convinced that "less is more", it only provides meta table to _simulate_ a class or a object, but to some extent, it is hard to use

Given that, i introduced `object` keyword in duka, which function like a pair of syntactic sugars that will be compiled to the same thing written before in original lua

```lua
object A
    property; -- nil as default
    property2 = 2;
    function func() end
end
```

### `match` Grammar (in plan)

Shall i introduce new keyword in?

also shall i implement a _powerful_ pattern matching?

```lua
match <target> then
    1 -> print "true";-- also nil
    {1, ..., [a] = 1} -> not false;
    true if false -> print "never";
    2 or 3 or not 4 -> 2;
    |> check() and |> check2("s") then
        local a = 1
        a = 2
        return a
    end
    > 1 or < 2 -> do print "not 1 or 2" end
else
    <exhausted>
end
```

Basic pattern term:

-   Constant(val)
-   Guard(term, expr)
-   Compound(term, term, op)
-   MethodCall(func, params, op)
-   Logic(op, expr)
-   List-Table(array, map)

### Pipeline Grammar (done)

```lua
param |> func
```

#### already supported

In expression, it behaves normally `a |> f`

but when in statment, where didnt allow expression directly,
you need to wrap it with `()` in order to make a **expression statment**
`(a |> f |> f2)`

i admit that it is kind of weird looking, but the very reason i can give is that **it hard and complicated to parse a statment with expression or argument list in head without anything else to recognize directly**

moreover, this only support one parameter in left

because im lazy to implement the tuple one

also `<|` is supported as well

```lua
(0 |> f(7, 2) <| 1)
```

this will be like:

```lua
f(7, 2, 1, 0)
```

### ~~`...` Grammar~~ (passed)

this may be passed

because i dont want to

```lua
[...array, 1] => flat the array
{ ...obj, a = 1 } => "with" grammar
```
