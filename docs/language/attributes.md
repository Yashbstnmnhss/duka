# Attributes

- [declare](#declare)
- [inline](#inline)
- [const](#const)
- [close](#close)
- [data-frozen--bool---false](#data-frozen--bool---false)
- [returns](#returns)
- [keywordish](#keywordish)

## @declare

<a id="declare"></a>

Available for: function, variable, object 

Says what exists rather than what to build. The signature is written down and nothing is generated, so this is a note the type checker and every reader of the code get to see; the thing itself is expected to be there already, put there by the host, by `ffi`, or by another module. A declared function takes no body and a declared value takes no value, because there would be nothing to do with either. A declaration with no annotation says nothing rather than saying `nil`, and one with no return annotation returns nothing

`@declare export` is how a module hands a promise on: the importer gets the signature and calls it as a function, while the module itself writes no `export` for it and the name is `nil` there. `RequireType("m").f` asks a declaration for the shape it promised, the way it asks a type alias for a type -- and reads through as far as you like, each dot checked on its own

### Example

```lua
@declare function strlen(s: string): int

@declare global HOSTNAME: string
@declare @const MAX_PATH: int

@declare object Point
    x: int
    y: int
end

-- in `m.duka`
@declare export function strlen(s: string): int

-- elsewhere
local m = require("m")
local n: int = m.strlen("s")
local g: RequireType("m").strlen = some_function
```

## @inline

<a id="inline"></a>

Available for: function 

Hints the generator to make this function **inline** if possible

## @const

<a id="const"></a>

Available for: variable 

Marks a variable to be a constant. This variable will be immutable

## @close

<a id="close"></a>

Available for: variable 

Marks a variable to be closed automatically

## @data(frozen: bool = false)

<a id="data-frozen--bool---false"></a>

Available for: object 

Automatically generate `init()`, `__eq`, `__tostring` based on properties defined

## @returns(...)

<a id="returns"></a>

Available for: function 

Says what the return slots stand for. `result`: the return values follow the **Result Protocol**, the first is whether the call succeeded and the rest are its own values. `exit`: nothing comes back and nothing after the call means anything either, so there is no question of what it returned

## @keywordish

<a id="keywordish"></a>

Available for: function 

The declaration reads as a **keyword** rather than as a name

