# core


<a id="core"></a>

## Contents

[as](#as)

[as_const](#as_const)

[select](#select)

[require](#require)

[print](#print)

[typeof](#typeof)

[instanceof](#instanceof)

[to_string](#to_string)

[to_number](#to_number)

[assert](#assert)

[error](#error)

[is_error](#is_error)

[unwrap](#unwrap)

[expect](#expect)

[get_metatable](#get_metatable)

[set_metatable](#set_metatable)

[pairs](#pairs)

[ipairs](#ipairs)

[costatus](#costatus)

[try](#try)

[clone](#clone)

[curry](#curry)

[Result](#result)

## Members

<a id="as"></a>

### `as(...who: T) -> T, ...`

> Forces the type of a value. The type argument says what it is, and nothing checks that it is right

Flags: `@inline(fake), @keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...who` | `T` | *true* | *false* | - | - |

#### Returns

`T, ...`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `T` |
| - | `...` |

#### Example

```lua
as.<any>(1)
```

<a id="as_const"></a>

### `as_const(who: T) -> T`

> Keeps a constant at its most precise type: `as_const(1)` is `1`, not `int`. An argument that is not a constant answers whatever it already was

Flags: `@inline(fake), @keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `who` | `T` | *false* | *false* | *required* | - |

#### Returns

`T`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `T` |

#### Example

```lua
local n: 1 = as_const(1)
```

<a id="select"></a>

### `select(pat: P, ...vals: T) -> Select, ...`

> Select element(s) or length from var args

Flags: `@keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pat` | `P` | *false* | *false* | *required* | - |
| `...vals` | `T` | *true* | *false* | - | - |

#### Returns

`Select, ...`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Select` |
| - | `...` |

#### Example

```lua
... |$> select(2)   -- [...][2]
```

<a id="require"></a>

### `require(pattern: string) -> any`

> Import module by pattern

Flags: `@returns(module), @keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="print"></a>

### `print(...args: any)`

> Prints to standard output

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...args` | `any` | *true* | *false* | - | - |

<a id="typeof"></a>

### `typeof(val: any) -> string`

> Get type name of value

Flags: `@keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="instanceof"></a>

### `instanceof(value: any, target: any) -> bool`

> Check if the value is an instance of target

Flags: `@keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `value` | `any` | *false* | *false* | *required* | - |
| `target` | `any` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="to_string"></a>

### `to_string(val: any) -> string`

> Convert to string

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="to_number"></a>

### `to_number(val: any) -> float | nil`

> Convert to number

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |

#### Returns

`float | nil`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float | nil` |

<a id="assert"></a>

### `assert(cond: any, msg: string = Assertion failed)`

> Assertion

Flags: `@keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `cond` | `any` | *false* | *false* | *required* | - |
| `msg` | `string` | *false* | *true* | `Assertion failed` | - |

<a id="error"></a>

### `error(msg: string = Error)`

> Raise an error

Flags: `@returns(exit)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `msg` | `string` | *false* | *true* | `Error` | - |

<a id="is_error"></a>

### `is_error(...val: any) -> bool`

> Check if it is an error

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...val` | `any` | *true* | *false* | - | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="unwrap"></a>

### `unwrap(...val: any) -> ...`

> Unwrap a result

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...val` | `any` | *true* | *false* | - | - |

#### Returns

`...`<br/>

| Index | Type |
| :--- | :--- |
| - | `...` |

<a id="expect"></a>

### `expect(val: T, msg: string = Got nil value) -> T`

> Expect a non-nil value

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `T` | *false* | *false* | *required* | - |
| `msg` | `string` | *false* | *true* | `Got nil value` | - |

#### Returns

`T`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `T` |

<a id="get_metatable"></a>

### `get_metatable(val: table) -> table | nil`

> Get metatable

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `table` | *false* | *false* | *required* | - |

#### Returns

`table | nil`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `table | nil` |

<a id="set_metatable"></a>

### `set_metatable(val: T, metatable: V) -> WithMetatable`

> Set metatable

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `T` | *false* | *false* | *required* | - |
| `metatable` | `V` | *false* | *false* | *required* | - |

#### Returns

`WithMetatable`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `WithMetatable` |

<a id="pairs"></a>

### `pairs(tab: table<K, V>) -> Iterator`

> Return key-value iterator for table

Flags: `@returns(iterator)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |

#### Returns

`Iterator`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Iterator` |

<a id="ipairs"></a>

### `ipairs(tab: array<V> | table<any, V>) -> Iterator`

> Return index-value iterator for table or array

Flags: `@returns(iterator)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `array<V> | table<any, V>` | *false* | *false* | *required* | - |

#### Returns

`Iterator`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Iterator` |

<a id="costatus"></a>

### `costatus(coroutine: coroutine) -> CoStatus`

> Get coroutine's status

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coroutine` | `coroutine` | *false* | *false* | *required* | - |

#### Returns

`CoStatus`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `CoStatus` |

<a id="try"></a>

### `try(func: function | table, ...params: any)`

> Run a function in protected mode, results follow Result Protocol

Flags: `@returns(result), @keywordish()`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `func` | `function | table` | *false* | *false* | *required* | - |
| `...params` | `any` | *true* | *false* | - | - |

<a id="clone"></a>

### `clone(val: T) -> T`

> Clone a value (shallowly)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `T` | *false* | *false* | *required* | - |

#### Returns

`T`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `T` |

<a id="curry"></a>

### `curry(f: function, ...args: any) -> any`

> Bind arguments to a function partially

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `f` | `function` | *false* | *false* | *required* | - |
| `...args` | `any` | *true* | *false* | - | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="result"></a>

### Static `Result`(DukaResult)

> Context for `result` protocol

[See here](#dukaresult)

