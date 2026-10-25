---
title: Objects
description: object, extends, methods and metamethods.
---

Lua only gives you a metatable to simulate a class, which is hard to use. Duka
has `object` — syntactic sugar that compiles to the same thing.

```duka
object A
    property       -- nil as default
    property2 = 2  -- property with default value

    function func() end          -- static function
    function :method()           -- method on instance
        print(self)
    end

    function init(args...) end   -- runs on every new(...)
    function new(args...) end    -- custom new (returns the instance)

    function __tostring()        -- metamethods
        return "A"
    end
end
```

```duka
A.func()
local a = A.new()   -- new(...) then init(...)
a:method()
```

## Grammar

```duka
object <name> (extends <base>)? (where ...)? (extends ... where ... end)?
    <property> (= <expr>)?

    function :<method>(...) ... end
    function <static_function>(...) ... end
    function __<metamethod>(...) ... end
end
```

`where` after the header takes the same clauses as a generic function:

```duka
object ABCC<T> extends A where T: int end
```

## Extends

`extends` declares the parent object.

Notice: `extends` only reaches a class the **same file** declares. A module
cannot extend a class it imported, and an instance of one module's `Point` is
never an instance of another's — each file has its own table of classes and
every table is numbered from zero.

## Across `require`

A class crosses a `require` as itself rather than as `any`:

```duka
-- m.duka
export object Point
    x: int
    function :len() return self.x end
end

-- elsewhere
local m = require("m")
local p = m.Point.new(7)                        -- a `Point`
local n: int = p.x                              -- checked against the field
local q: RequireType("m").Point = m.Point.new(1)
local bad: string = p.x                         -- an error
```

## `@data`

```duka
@Data(frozen = true)
object Point
    x: int
    y: int
end
```

generates `init`, `__eq` and `__tostring` from the properties. See
[Attributes](/language/attributes/).
