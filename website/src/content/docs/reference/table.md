---
title: "table"
description: "tablestandard library reference" 
---
<a id="table"></a>

## Contents

[raw_get](#raw_get)

[raw_set](#raw_set)

[keys](#keys)

[values](#values)

[has](#has)

[has_value](#has_value)

[raw_get_set](#raw_get_set)

[merge](#merge)

[remove](#remove)

[clear](#clear)

[capacity](#capacity)

## Members

<a id="raw_get"></a>

### `raw_get(tab: table, key: K) -> V`

> Get property in table by given key without calling metamethod

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `key` | `K` | *false* | *false* | *required* | - |

#### Returns

`V`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `V` |

<a id="raw_set"></a>

### `raw_set(tab: table<K, V>, key: K, val: V)`

> Set property in table by given key and value without calling metamethod

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |
| `key` | `K` | *false* | *false* | *required* | - |
| `val` | `V` | *false* | *false* | *required* | - |

<a id="keys"></a>

### `keys(tab: table<K, V>) -> array<K>`

> Get an array with keys in table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |

#### Returns

`array<K>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array<K>` |

<a id="values"></a>

### `values(tab: table<K, V>) -> array<V>`

> Get an array with values in table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |

#### Returns

`array<V>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array<V>` |

<a id="has"></a>

### `has(tab: table<K, V>, key: K) -> bool`

> Whether given key is in target table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |
| `key` | `K` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="has_value"></a>

### `has_value(tab: table<K, V>, val: V) -> bool`

> Whether given value is in target table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |
| `val` | `V` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="raw_get_set"></a>

### `raw_get_set(tab: table<K, V>, key: K, val: V | nil = nil) -> V | nil`

> Get property in tab by given key without calling metamethod. If not exist, insert with val and return it

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |
| `key` | `K` | *false* | *false* | *required* | - |
| `val` | `V | nil` | *false* | *true* | `nil` | - |

#### Returns

`V | nil`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `V | nil` |

<a id="merge"></a>

### `merge(tab: table<K, V>, other: table<K2, V2>, keep: bool = false)`

> Merge another table to this table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |
| `other` | `table<K2, V2>` | *false* | *false* | *required* | - |
| `keep` | `bool` | *false* | *true* | `false` | - |

<a id="remove"></a>

### `remove(tab: table<K, V>, key: K) -> V | nil`

> Remove property in table by given key

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |
| `key` | `K` | *false* | *false* | *required* | - |

#### Returns

`V | nil`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `V | nil` |

<a id="clear"></a>

### `clear(tab: table<K, V>) -> table<K, V>`

> Clear table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table<K, V>` | *false* | *false* | *required* | - |

#### Returns

`table<K, V>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `table<K, V>` |

<a id="capacity"></a>

### `capacity(cap: int) -> table<K, V>`

> Create new table with given capacity

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `cap` | `int` | *false* | *false* | *required* | - |

#### Returns

`table<K, V>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `table<K, V>` |

