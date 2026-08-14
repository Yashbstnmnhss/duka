# table

[Index](index.md)

## Contents

- [raw_get](#raw_get)
- [raw_set](#raw_set)
- [keys](#keys)
- [values](#values)
- [has](#has)

## Members

<a id="raw_get"></a>
### raw_get(tab: table, key: any) -> any

<blockquote>
Get property in table by given key without calling metamethod
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `tab` | table | *false* | *false* | *required* | - |
| `key` | any | *false* | *false* | *required* | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |

<a id="raw_set"></a>
### raw_set(tab: table, key: any, val: any)

<blockquote>
Set property in table by given key and value without calling metamethod
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `tab` | table | *false* | *false* | *required* | - |
| `key` | any | *false* | *false* | *required* | - |
| `val` | any | *false* | *false* | *required* | - |

<a id="keys"></a>
### keys(tab: table) -> array

<blockquote>
Get an array with keys in table
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `tab` | table | *false* | *false* | *required* | - |

## Returns

`array`



| Index | Type |
| :--- | :---: |
| 0 | array |

<a id="values"></a>
### values(tab: table) -> array

<blockquote>
Get an array with values in table
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `tab` | table | *false* | *false* | *required* | - |

## Returns

`array`



| Index | Type |
| :--- | :---: |
| 0 | array |

<a id="has"></a>
### has(tab: table, key: any) -> bool

<blockquote>
Whether given key is in target table
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `tab` | table | *false* | *false* | *required* | - |
| `key` | any | *false* | *false* | *required* | - |

## Returns

`bool`



| Index | Type |
| :--- | :---: |
| 0 | bool |
