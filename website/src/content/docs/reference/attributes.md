---
title: "Attributes"
description: "Attributes standard library reference" 
---
- [doc-value](#doc-value)
- [keywordish](#keywordish)
- [const](#const)
- [close](#close)
- [inline-always](#inline-always)
- [data-frozen](#data-frozen)
- [declare](#declare)
- [returns](#returns)

## @doc(value)

<a id="doc-value"></a>

Documentation attached to a declaration. `---` lines are sugar for this attribute

## @keywordish

<a id="keywordish"></a>

The declaration reads as a **keyword** rather than as a name

## @const

<a id="const"></a>

Marks a variable to be a constant. This variable will be immutable

## @close

<a id="close"></a>

Marks a variable to be closed automatically

## @inline(always?)

<a id="inline-always"></a>

Hints the generator to make this function **inline** if possible

## @data(frozen?)

<a id="data-frozen"></a>

Automatically generate `init()`, `__eq`, `__tostring` based on properties defined

## @declare

<a id="declare"></a>

Says what exists rather than what to build. 
The signature is written down and nothing is generated, so this is a note the type checker and every reader of the code get to see; the thing itself is expected to be there already, put there by the host, or by another module. 
A declared function takes no body and a declared value takes no value, because there would be nothing to do with either. 
A declaration with no annotation says nothing rather than saying `nil`, and one with no return annotation returns nothing

- `@declare export` is how a module hands a promise on: the importer gets the signature and calls it as a function, while the module itself writes no `export` for it and the name is `nil` there. 
- `RequireType(\"m\").f` asks a declaration for the shape it promised, the way it asks a type alias for a type -- and reads through as far as you like, each dot checked on its own


## @returns

<a id="returns"></a>

Says what the return slots stand for. 
- `result`: the return values follow the **Result Protocol**, the first is whether the call succeeded and the rest are its own values. 
- `exit`: nothing comes back and nothing after the call means anything either, so there is no question of what it returned.


