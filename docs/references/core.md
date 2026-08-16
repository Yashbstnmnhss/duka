# core

[Index](index.md)

## Contents

- [require](#require)
- [print](#print)
- [type](#type)
- [to_string](#to_string)
- [to_number](#to_number)
- [assert](#assert)
- [error](#error)
- [is_error](#is_error)
- [unwrap](#unwrap)
- [expect](#expect)
- [get_metatable](#get_metatable)
- [set_metatable](#set_metatable)
- [instanceof](#instanceof)
- [pairs](#pairs)
- [ipairs](#ipairs)
- [costatus](#costatus)
- [try](#try)

## Members

<a id="require"></a>
### `require(pattern: string)`

<blockquote>
Import module by pattern
</blockquote>

## Flags
@returns(module)

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `pattern` | string | *false* | *false* | *required* | - |

<a id="print"></a>
### `print(...args: any)`

<blockquote>
Prints to standard output
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `...args` | any | *true* | *false* | - | - |

<a id="type"></a>
### `type(val: any)`

<blockquote>
Get type name of value
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | any | *false* | *false* | *required* | - |

<a id="to_string"></a>
### `to_string(val: any)`

<blockquote>
Convert to string
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | any | *false* | *false* | *required* | - |

<a id="to_number"></a>
### `to_number(val: any)`

<blockquote>
Convert to number
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | any | *false* | *false* | *required* | - |

<a id="assert"></a>
### `assert(cond: any, msg: string = "assertion failed".to_owned())`

<blockquote>
Assertion
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `cond` | any | *false* | *false* | *required* | - |
| `msg` | string | *false* | *true* | `"assertion failed".to_owned()` | - |

<a id="error"></a>
### `error(msg: string = "error".to_owned())`

<blockquote>
Raise an error
</blockquote>

## Flags
@returns(exit)

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `msg` | string | *false* | *true* | `"error".to_owned()` | - |

<a id="is_error"></a>
### `is_error(...val: any)`

<blockquote>
Check if it is an error
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `...val` | any | *true* | *false* | - | - |

<a id="unwrap"></a>
### `unwrap(...val: any) -> ...`

<blockquote>
Unwrap a result
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `...val` | any | *true* | *false* | - | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="expect"></a>
### `expect(val: any, msg: string = "Got nil value".to_owned()) -> any`

<blockquote>
Expect a non-nil value
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | any | *false* | *false* | *required* | - |
| `msg` | string | *false* | *true* | `"Got nil value".to_owned()` | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |

<a id="get_metatable"></a>
### `get_metatable(val: table)`

<blockquote>
Get metatable
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | table | *false* | *false* | *required* | - |

<a id="set_metatable"></a>
### `set_metatable(val: table, metatable: table | nil) -> table`

<blockquote>
Set metatable
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | table | *false* | *false* | *required* | - |
| `metatable` | table \| nil | *false* | *false* | *required* | - |

## Returns

`table`



| Index | Type |
| :--- | :---: |
| 0 | table |

<a id="instanceof"></a>
### `instanceof(value: any, target: any)`

<blockquote>
Check if the value is an instance of target
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `value` | any | *false* | *false* | *required* | - |
| `target` | any | *false* | *false* | *required* | - |

<a id="pairs"></a>
### `pairs(tab: table)`

<blockquote>
Return key-value iterator for table
</blockquote>

## Flags
@returns(iterator)

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `tab` | table | *false* | *false* | *required* | - |

<a id="ipairs"></a>
### `ipairs(tab: table)`

<blockquote>
Return index-value iterator for table
</blockquote>

## Flags
@returns(iterator)

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `tab` | table | *false* | *false* | *required* | - |

<a id="costatus"></a>
### `costatus(coroutine: any)`

<blockquote>
Get coroutine's status
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `coroutine` | any | *false* | *false* | *required* | - |

<a id="try"></a>
### `try(func: function | table, ...params: any)`

<blockquote>
Run a function in protected mode, results follow Result Protocol
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `func` | function \| table | *false* | *false* | *required* | - |
| `...params` | any | *true* | *false* | - | - |
