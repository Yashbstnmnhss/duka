# type-context


<a id="type-context"></a>

> Builtins for type-context

Flags: `@marker(type-context)`

## Contents

[Error](#error)

[Assert](#assert)

[Stringify](#stringify)

[Union](#union)

[Unpack](#unpack)

[IsSubType](#issubtype)

[HasVarArg](#hasvararg)

[HasRetVarArg](#hasretvararg)

[Subtract](#subtract)

[EndsWith](#endswith)

[StartsWith](#startswith)

[Slice](#slice)

[Split](#split)

[Uppercase](#uppercase)

[Lowercase](#lowercase)

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

<a id="issubtype"></a>

### type function `IsSubType`(type, type)

> Whether B is a sub type of A

<a id="hasvararg"></a>

### type function `HasVarArg`(type)

> Whether the function type has var-arg parameter

<a id="hasretvararg"></a>

### type function `HasRetVarArg`(type)

> Whether the function type has var-arg returns

<a id="subtract"></a>

### type function `Subtract`(type, type)

> Remove B in union or type array A

<a id="endswith"></a>

### type function `EndsWith`(type, type)

> Return true if literal string type A ends with B

<a id="startswith"></a>

### type function `StartsWith`(type, type)

> Return true if literal string type A starts with B

<a id="slice"></a>

### type function `Slice`(type, type)

> Slice string literal type A with start index B and optional end index C

<a id="split"></a>

### type function `Split`(type, type)

> Split string literal type A by separator string literal type B

<a id="uppercase"></a>

### type function `Uppercase`(type)

> Convert a string literal type into uppercase

<a id="lowercase"></a>

### type function `Lowercase`(type)

> Convert a string literal type into lowercase

