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

All of them will be **erased** before compiling into bytecode

Duka treats every type as a value. To process this "value", you can use or define a **type function** (like Typescript's util type)

```lua
type Value = int
type function Nullable(who)
    return who | nil
end
type ValueWithNil = Nullable(Value) -- int|nil
```
