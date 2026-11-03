---
title: "type-context"
description: "type-context standard library reference" 
---
<a id="type-context"></a>

> Builtins for type-context

Flags: `@feature(type-context)`

## Contents

[Error](#error)

[Assert](#assert)

[Stringify](#stringify)

[Union](#union)

[Unpack](#unpack)

[Elem](#elem)

[IsIterable](#isiterable)

[IsSubType](#issubtype)

[NonNil](#nonnil)

[Super](#super)

[In](#in)

[HasVarArg](#hasvararg)

[HasRetVarArg](#hasretvararg)

[BuildFunction](#buildfunction)

[Exclude](#exclude)

[EndsWith](#endswith)

[StartsWith](#startswith)

[Remove](#remove)

[AppendHead](#appendhead)

[Append](#append)

[Slice](#slice)

[Split](#split)

[Uppercase](#uppercase)

[Capitalize](#capitalize)

[Uncapitalize](#uncapitalize)

[Lowercase](#lowercase)

[Regex](#regex)

[NotIn](#notin)

[Has](#has)

[Any](#any)

[All](#all)

[IsTuple](#istuple)

[provide](#provide)

[Not](#not)

[IsLoosely](#isloosely)

[WithBrand](#withbrand)

[GetMetatable](#getmetatable)

[Unit](#unit)

[GtPat](#gtpat)

[GePat](#gepat)

[LtPat](#ltpat)

[LePat](#lepat)

[EqPat](#eqpat)

[NePat](#nepat)

[HasPat](#haspat)

[StartsPat](#startspat)

[EndsPat](#endspat)

[RegexPat](#regexpat)

[LenGePat](#lengepat)

[LenLePat](#lenlepat)

[LenGtPat](#lengtpat)

[LenLtPat](#lenltpat)

[LenEqPat](#leneqpat)

[AndPat](#andpat)

[OrPat](#orpat)

[NotPat](#notpat)

[WithMetatable](#withmetatable)

## Members

<a id="error"></a>

### type function `Error`(type)

> Throw an error with a message

<a id="assert"></a>

### type function `Assert`(type, type)

> If A isn't true, throws an error with message B

<a id="stringify"></a>

### type function `Stringify`(type)

> Stringify a type

<a id="union"></a>

### type function `Union`(type)

> Pack types from type array into a union type

<a id="unpack"></a>

### type function `Unpack`(type)

> Unpack a union type

<a id="elem"></a>

### type function `Elem`(type)

> The element type of an array, table or tuple type

<a id="isiterable"></a>

### type function `IsIterable`(type)

> Whether A is an iterable type value

<a id="issubtype"></a>

### type function `IsSubType`(type, type)

> Whether B is a sub type of A

<a id="nonnil"></a>

### type function `NonNil`(type)

> A type with nil taken out of it

<a id="super"></a>

### type function `Super`(type)

> Get the super type of a type value in type context

<a id="in"></a>

### type function `In`(type, type)

> Whether A is in B

<a id="hasvararg"></a>

### type function `HasVarArg`(type)

> Whether the function type has var-arg parameter

<a id="hasretvararg"></a>

### type function `HasRetVarArg`(type)

> Whether the function type has var-arg returns

<a id="buildfunction"></a>

### type function `BuildFunction`(type, type, type, type)

> Build a function type

<a id="exclude"></a>

### type function `Exclude`(type, type)

> Remove B in union or type array A

<a id="endswith"></a>

### type function `EndsWith`(type, type)

> Return true if literal string type A ends with B

<a id="startswith"></a>

### type function `StartsWith`(type, type)

> Return true if literal string type A starts with B

<a id="remove"></a>

### type function `Remove`(type, type)

> Remove an element in type tuple or type table by its index or key

<a id="appendhead"></a>

### type function `AppendHead`(type, type)

> Insert new element to the head of a type tuple or type table

<a id="append"></a>

### type function `Append`(type, type)

> Append new element to type tuple or type table

<a id="slice"></a>

### type function `Slice`(type, type)

> Slice string literal type A with start index B and optional end index C

<a id="split"></a>

### type function `Split`(type, type)

> Split string literal type A by separator string literal type B

<a id="uppercase"></a>

### type function `Uppercase`(type)

> Convert a string literal type into uppercase

<a id="capitalize"></a>

### type function `Capitalize`(type)

> Capitalize a string literal type

<a id="uncapitalize"></a>

### type function `Uncapitalize`(type)

> Uncapitalize a string literal type

<a id="lowercase"></a>

### type function `Lowercase`(type)

> Convert a string literal type into lowercase

<a id="regex"></a>

### type function `Regex`(type, type)

> Search a RegEx pattern in a string literal type

<a id="notin"></a>

### type function `NotIn`(type, type)

> Whether A is a member of union, tuple or record B

<a id="has"></a>

### type function `Has`(type, type)

> Narrow A when A has a member called B

<a id="any"></a>

### type function `Any`(type, type)

> A if it answers, otherwise B -- one of the two, whichever answers

<a id="all"></a>

### type function `All`(type, type)

> Narrow A when both answer -- either side may be the one that narrowed

<a id="istuple"></a>

### type function `IsTuple`(type)

> Whether given type is a type tuple

<a id="provide"></a>

### type function `provide`(type)

> Hand a type back while keeping the constraints its evaluation contributed

<a id="not"></a>

### type function `Not`(type, type)

> Narrow A when C does not answer -- impossible once C does

<a id="isloosely"></a>

### type function `IsLoosely`(type)

> Whether the given type tuple is checking types loosely (etc [bool, string, ...] returns true)

<a id="withbrand"></a>

### type function `WithBrand`(type, type)

> Give T the nominal label NAME, `int ::len::`

<a id="getmetatable"></a>

### type function `GetMetatable`(type)

> Get metatable type

<a id="unit"></a>

### type function `Unit`(type, type)

> Give T the unit label UNITS, `m/s^2`

<a id="gtpat"></a>

### type function `GtPat`(type, type)

> Numbers greater than N

<a id="gepat"></a>

### type function `GePat`(type, type)

> Numbers greater than or equal to N

<a id="ltpat"></a>

### type function `LtPat`(type, type)

> Numbers less than N

<a id="lepat"></a>

### type function `LePat`(type, type)

> Numbers less than or equal to N

<a id="eqpat"></a>

### type function `EqPat`(type, type)

> Numbers equal to N

<a id="nepat"></a>

### type function `NePat`(type, type)

> Numbers other than N

<a id="haspat"></a>

### type function `HasPat`(type, type)

> Strings containing PART

<a id="startspat"></a>

### type function `StartsPat`(type, type)

> Strings starting with PART

<a id="endspat"></a>

### type function `EndsPat`(type, type)

> Strings ending with PART

<a id="regexpat"></a>

### type function `RegexPat`(type, type)

> Strings matching REGEX in full

<a id="lengepat"></a>

### type function `LenGePat`(type, type)

> Strings at least N characters long

<a id="lenlepat"></a>

### type function `LenLePat`(type, type)

> Strings at most N characters long

<a id="lengtpat"></a>

### type function `LenGtPat`(type, type)

> Strings longer than N characters

<a id="lenltpat"></a>

### type function `LenLtPat`(type, type)

> Strings shorter than N characters

<a id="leneqpat"></a>

### type function `LenEqPat`(type, type)

> Strings exactly N characters long

<a id="andpat"></a>

### type function `AndPat`(type, type)

> Both patterns hold

<a id="orpat"></a>

### type function `OrPat`(type, type)

> Either pattern holds

<a id="notpat"></a>

### type function `NotPat`(type)

> The pattern does not hold

<a id="withmetatable"></a>

### type function `WithMetatable`(type, type)

> Attach metatable MT to T, or take it off when MT is nil

