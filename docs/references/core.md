# core


<a id="core"></a>

## Contents

[require](#require)

[print](#print)

[typeof](#typeof)

[to_string](#to_string)

[to_number](#to_number)

[assert](#assert)

[error](#error)

[is_error](#is_error)

[unwrap](#unwrap)

[expect](#expect)

[get_metatable](#get_metatable)

[set_metatable](#set_metatable)

[instanceof](#instanceof)

[pairs](#pairs)

[ipairs](#ipairs)

[costatus](#costatus)

[try](#try)

## Members

<a id="require"></a>

### `require(pattern: string)`

> Import module by pattern

Flags: `@returns(module)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |

<a id="print"></a>

### `print(...args: any)`

> Prints to standard output

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...args` | `any` | *true* | *false* | - | - |

<a id="typeof"></a>

### `typeof(val: any)`

> Get type name of value

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |

<a id="to_string"></a>

### `to_string(val: any)`

> Convert to string

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |

<a id="to_number"></a>

### `to_number(val: any)`

> Convert to number

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |

<a id="assert"></a>

### `assert(cond: any, msg: string = "assertion failed".to_owned())`

> Assertion

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `cond` | `any` | *false* | *false* | *required* | - |
| `msg` | `string` | *false* | *true* | `"assertion failed".to_owned()` | - |

<a id="error"></a>

### `error(msg: string = "error".to_owned())`

> Raise an error

Flags: `@returns(exit)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `msg` | `string` | *false* | *true* | `"error".to_owned()` | - |

<a id="is_error"></a>

### `is_error(...val: any)`

> Check if it is an error

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...val` | `any` | *true* | *false* | - | - |

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

### `expect(val: any, msg: string = "Got nil value".to_owned()) -> any`

> Expect a non-nil value

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |
| `msg` | `string` | *false* | *true* | `"Got nil value".to_owned()` | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="get_metatable"></a>

### `get_metatable(val: table)`

> Get metatable

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `table` | *false* | *false* | *required* | - |

<a id="set_metatable"></a>

### `set_metatable(val: table, metatable: table | nil) -> table`

> Set metatable

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `table` | *false* | *false* | *required* | - |
| `metatable` | `table | nil` | *false* | *false* | *required* | - |

#### Returns

`table`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `table` |

<a id="instanceof"></a>

### `instanceof(value: any, target: any)`

> Check if the value is an instance of target

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `value` | `any` | *false* | *false* | *required* | - |
| `target` | `any` | *false* | *false* | *required* | - |

<a id="pairs"></a>

### `pairs(tab: table)`

> Return key-value iterator for table

Flags: `@returns(iterator)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |

<a id="ipairs"></a>

### `ipairs(tab: table)`

> Return index-value iterator for table

Flags: `@returns(iterator)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |

<a id="costatus"></a>

### `costatus(coroutine: any)`

> Get coroutine's status

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coroutine` | `any` | *false* | *false* | *required* | - |

<a id="try"></a>

### `try(func: function | table, ...params: any)`

> Run a function in protected mode, results follow Result Protocol

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `func` | `function | table` | *false* | *false* | *required* | - |
| `...params` | `any` | *true* | *false* | - | - |

