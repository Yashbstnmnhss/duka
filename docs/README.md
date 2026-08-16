<div align=center>
<img src="logo.svg" width="100" height="100" alt="logo">

# DUKA

</div>

`duka` is a programming language with similar grammar of lua

## Documents

- See [grammar](grammar.md) for duka's grammar
- See [stdlib](stdlib.md) for standard library
- See [types](type.md) for type system
- See [modules](modules.md) for module system
- See [user_data](user_data.md) for UserData type

## WIP

- See [explanation](misc/explanation.md)

- See [memo](misc/memo.md)

- See [improvement](misc/improvement.md)

## ~~Implementing~~ ~~Just Use A Garbage Collector~~ No, Just Implement it

~~let's use `gc` crate~~

See [gc](misc/gc_thing.md) and `gc` crate here

## Weird Features

### `!` Block

See [here](../frontend/src/parser/bang.rs)

Now, an identifier along with a `!` mark will be processed specially

#### `linq!`

You can use linq in duka, by wrapping it between `linq!(` and `)`

It is an expression, instead of a statement;

```csharp
global list = linq!(
    from x in array1
    where x > 0
    from y in array2
    select x * y
)
```

will be transformed into:

```lua
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

Up to now, only `where` and `from` clauses are supported;

A valid linq expression must start with at least one `from ... in ...` and end with single `select`

#### `logic!` (WIP)

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

- `nameof!(x)` will return the name of the first token parameters(**x**) input
    ```lua
    [:nameof!(a):] -- "<identifier>"
    ```
- `stringify!(x)` stringify input tokens, only **x**
- `concat!(...)` will concat **only identifier in ...** (other will be ignored)
  input tokens to one identifier token
- `when!(a, b, c)` when parameter **a** is exactly a `true` token, **b** will be inserted, otherwise **c**
- `nonempty!(x)` when **x** contains at lease one token, it will be `true` token, otherwise `false`

Moreover, you can use `...` in macro to declare a vararg, it must be the last one in parameters;

To expand it, use `$...` to expand them into sequence separated by `,` as default
Needing to custom it, add `[<token>]` after that, then the separator will be the single token in `[` or `(` and `)` or `]`:

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

Cycled recursion is forbidden (using `~` instead), but nested macro will be dealt rightly

It's ~~useless~~ **cool**, isn't it?

## Extended Grammar

### Better `local` and `global`

In the original lua, all variables are global defined without `local` keyword

it is for sure a very confusing design

Now, any variables are local defined implicitly

Meanwhile, an explicit keyword `global` has been introduced in, which is the **only** way now to declare a global variable

### Destructuring

```lua
local { a, b, c } = expr
```

is equivalent to

```lua
local a, b, c = do
    local <name> = expr
    return <name>.a, <name>.b, <name>.c
end
```

This will be useful when you are importing:

```lua
local { a1, a2 } = require("A")
```

### `function` & `fn`

To define a function, you need to use `function` keyword.
Also, for lambda expression (or anonymous function), you can use `function() end`.

To simplify the syntax of lambda expression, now you can use `fn(params) expression` to define a lambda expression. It accepts single expression as its return

```lua
local b = c |> map(fn(x) x * 2)
```

But combined with `do end` expression, you can use it like a `function` too:

```lua
local b = c |> map(fn(x) do return x * 2 end)
```

Both `function` `fn` are preserved keywords, they can be used in type annotation (also `func`, but not preserved keyword) to declare a function type

### Extended `@attribute`

Now you can use attr for function

```lua
@abc(key = val)
function abc()
...
```

and multiple attributes are supported

For variables, **prefix** applies to all while **suffix** applies to single variable before it

```lua
@for_all
local a @only_a, b = 1, 2
```

is equivalent to

```lua
@for_all
@only_a
local a = 1
@for_all
local b = 2
```

Supported attributes:

- `@inline`: **function**, hints inline behaviour
- `@const`: **variable**, make it immutable
- `@data(frozen: bool)`: **object**, generate `:init` `__eq` `__tostring` automatically based on its properties

### Module System

See [require()](../docs/builtin/index.md#require)

`export` keyword:

```lua
export local a = 1
export function b() end
```

is equivalent to:

```lua
local _EXPORTS = {}

local a = 1
function b() end

_EXPORTS.a = 1
_EXPORTS.b = b

return _EXPORTS
```

### Modern OOP

Since lua has been convinced that "less is more", it only provides meta table to _simulate_ a class or an object, but to some extent, it is hard to use

Given that, I introduced `object` keyword in duka, which function like a pair of syntactic sugars that will be compiled to the same thing written before in original lua

```lua
object A
    property -- nil as default
    property2 = 2 -- property with default value

    function func() end -- static function
    function :method() -- method on instance
        print(self)
    end

    function init(args...) end -- When every instance init, args from `new(...)`
    function new(args...) end -- Custom new function (return instance) for object A

    function __tostring() -- Metamethod supports
        return "A"
    end
end
```

```lua
A.func()
local a = A.new() -- invoke new(...) then init(...)
a:method() -- "A"
```

### Array Type

Lua's table is a mix of dictionary and array, it is very confusing.
Since, I added **array** `[]` type in duka. Meanwhile, `{}` **table** also supports the original `list` syntax like `{ 1, 2, 3 }`, but it is translated into `{ [0] = 1, [1] = 2, [2] = 3}`

```lua
local a = [1, 2, 3]      -- array literal, 0-based index
local empty = []         -- empty array
local nested = [[1, 2], [3, 4]]   -- nested arrays
local mixed = [1, "a", true]      -- mixed element types
```

An array literal creates a fresh table on every execution (same as `{}`); `[]` elements can be any expression. Both `,` and `;` can separate elements, and a trailing separator is allowed(same for table)

**Notice**: the `[[...]]` long-string syntax is **not** supported anymore; Use the `[=[...]=]` form (or any higher level like `[==[ ]==]`) for multi-line strings

_(case like `[[1, 2], [3, 4]]`, lexer doesnt know whether it is a string or nested array)_

### `match` Grammar

Shall I introduce new keyword in?

also shall I implement a _powerful_ pattern matching?

```lua
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

The `<exhausted>` pattern is required when `match` is an expression, same for `if` expression;

Basic pattern term:

- Constant(val) `literal value`
- Bind(to, type?) `local to: type` (_bind to a local value, with type check (if has)_)
- Guard(term, expr) `<term> if ...`
- Compare(op, expr) `> value`
- Compound(term, term, op) `<term> and/or <term>`
- MethodCall(func, params, op) `|> function(...)`
- Not(expr) `not <term>`
- List-Table(array, map)
- Array(array)

For List-Table and Array, you can use `...` `_` `_ * n` to ignore single or many or what count you want items of array(using numbers for index), notice that count of `...` must be less than one;

```lua
{ first, ..., last }
{ _, second, ... }
{ _ * 3, fourth } -- also len = 4
```

### Pipeline Grammar

```lua
param |> func
```

The `func` can be either a path/identifier or a calling like `a |> f(1)`
In expression, it behaves normally `a |> f`, a will be inserted into the first

but when in statement, where didn't allow expression directly,
you need to wrap it with `()` in order to make an **expression statement**
`(a |> f |> f2)`

I admit that it is kind of weird looking, but the very reason I can give is that **it hard and complicated to parse a statement with expression or argument list in head without anything else to recognize directly**

moreover, this only support one parameter in left(or right)

because im lazy to implement the tuple one

also `<|` is supported as well, but inserts it into last

```lua
(0 |> f(7, 2) <| 1)
```

this will be like:

```lua
f(0, 7, 2, 1)
```

See `iter` stdlib

## References

- [CraftingInterpreters](https://craftinginterpreters.com/)
- [BuildLuaInRust](https://wubingzheng.github.io/build-lua-in-rust/zh)
- [Lua5.4Manual](https://www.lua.org/manual/5.4/manual.html)

## Roadmap

| Status  | Parts        | Date      |
| :-----: | :----------- | --------- |
|  Done   | Lexer        | 2025.7.13 |
|  Done   | Parser       | 2025.7.23 |
|  Done   | Codegen      | 2026.2.12 |
|  Done   | VM           | 2026.1.21 |
|  Done   | **GC**       | 2026.1.13 |
|  Done   | Instructions | 2025.7.11 |
|  Done   | Cli          | 2026.1.19 |
| Working | Std Lib      |           |
|  Done   | Macros       | 2025.7.14 |
