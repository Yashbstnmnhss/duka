# array

[Index](index.md)

## Contents

- [pack](#pack)
- [unpack](#unpack)
- [has](#has)
- [push](#push)
- [pop](#pop)

## Members

<a id="pack"></a>
### pack(...vals: any) -> array

<blockquote>
Pack all arguments into an array
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `...vals` | any | *true* | *false* | - | - |

## Returns

`array`



| Index | Type |
| :--- | :---: |
| 0 | array |

<a id="unpack"></a>
### unpack(arr: array) -> ...

<blockquote>
Unpack array into a tuple(as results)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `arr` | array | *false* | *false* | *required* | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="has"></a>
### has(arr: array, who: any) -> bool

<blockquote>
Whether given value is in target array
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `arr` | array | *false* | *false* | *required* | - |
| `who` | any | *false* | *false* | *required* | - |

## Returns

`bool`



| Index | Type |
| :--- | :---: |
| 0 | bool |

<a id="push"></a>
### push(arr: array, val: any)

<blockquote>
Push a value into array
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `arr` | array | *false* | *false* | *required* | - |
| `val` | any | *false* | *false* | *required* | - |

<a id="pop"></a>
### pop(arr: array) -> any

<blockquote>
Pop a value from array
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `arr` | array | *false* | *false* | *required* | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |
