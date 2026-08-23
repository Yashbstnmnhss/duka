# Type System of Duka

Duka adopted a type system based on annotations, this is a **compile-time** system instead of a checker in runtime.

## Annotations

```lua
local a: int = 1
function b(c: string): bool
    return false
end
```

You can use it by adding a `:`

## Nilable & Nonnilable

- When `default_nonnilable = true`, all values are nonnilable in default.
  You can use `type?` to mark a type nilable, this is equivalent to `type | nil`

- When `default_nonnilable = false`, all values can be assigned with `nil` (Equivalent to `type | nil` as default).
  By using `type!`, you can mark a type nonnilable, which is equivalent to `type` (`| nil` removed)

`default_nonnilable` is **false** in default

```lua
-- default_nonnilable = false
local a: int = nil --Ok
local b: int! = nil --TypeMismatchErr
```

```lua
-- default_nonnilable = true
local a: int = nil --TypeMismatchErr
local b: int? = nil --Ok
```

## Union Type

You can make **union type** by `|` operator.
`any` cannot be in union type, otherwise the whole type turns into a single `any`.

```lua
local a: int | string = 0
```

## Object Type

Duka has a special syntax `object`

```lua
object A extends B
    ...
end
```

```lua
local instance: A = A.new() -- type is object
local parent: B = instance -- upcast
```

## Function Type

```lua
function(bool)  -- No return
function() -> int   -- With one return value
function(...) -> ...    -- Dynamic return
function(int, string, ...) -> (int, bool)   -- Tuple return
```

The validation for function types is not strict. VarArg won't be checked.Parameters can be less or more than the other function type without errors, it will only validate **the common part** of arguments.

```lua
func(int, ...) <=> func(int, bool, string) -- OK
fn(int) <=> fn(int, bool, ...) -- OK
```

## Basic Types

Types have many aliases.

- int, integer
- float, num, number (accepts int implicitly)
- string, str
- bool, boolean
- table (accepts object)
- array, list
- func, function, fn
- nil (only accepts nil itself)
- any (accepts all)
- never (accepts nothing)

## Generic Type

Now you can use generic in **array** **table** **function** or other custom types

```lua
function abc<T>() end
function abc2<T>(...) end -- Notice: this gerneic type is automatically bond with ... vararg

local a: array<int> = [1,2,3]
local b: table<string, bool> = { c = true }
```

Given that both `<` and `>` are valid token in expression respectively, you must add a `.` prefix when using generic type in an expression in order to avoid parsing errors

```lua
abc.<int>() -- calling a function
A.<string>.static() -- calling a static function in object A
```

## Type Context

### Concepts

- **type-context**: statements & expressions start with `type` keyword.
- **value-context**: the normal codes

In type-context, types are treated as values (in compile-time)

All of them will be **erased** before compiling into bytecode
To process those type "value", you can use or define a **type function** (like TypeScript's util type) (See below)

```lua
type Value = int -- define a type (like a global variable in value-context)
type function Nullable(who) -- define a type function with one argument and return a type
    return who | nil
end
type ValueWithNil = Nullable(Value) -- int|nil
```

### Interaction Between Two Contexts

- `type-context` -> `value-context`: by [type annotation](#annotations)
- `value-context` -> `type-context`: by `type(expr)`

```lua
local a = 0
type B = type(a) -- int
```

### Expressions

Almost same as expressions in value-context. There have some speical expressions for type-context.

You can define an anonymous type function (closure) by `type function() ... end` and `type fn() ..`, which enables you to use **HKT**(Higher Kinded Type)

### Statements

Only `for`(numeric & generic), `while`, `if`, `match` and variable definitions & assignments are supported in type context

#### Loops

There has a limitation for loops (only 1000 times allow)

#### Match

`match` statement in type-context can have a special pattern: **TypePattern**, which allows you to _infer_ a type by some certain patterns:

```lua
type A = array<int>

type function InferItem(who)
    return match who then
        array(local U) -> U;
    else
        return never
    end
end

type B = InferItem(A)   -- is `int`
```

That also works for user-defined type functions

```lua
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

type D = C(int) -- string
type A = Infer(D) -- int
type B = Infer(string) -- never
```

For basic-types, _infer_ system just takes its generic types.
But for user-defined type funcitons, _infer_ system will **lookup** a cache table based on given type's tag (hidden) and pattern name to fetch the arguments.

Other supported patterns are:

- Constant pattern
- Compound pattern
- Guard pattern
- Not pattern
- Bind pattern
