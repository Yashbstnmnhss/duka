# table


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

### `raw_get(tab: table, key: any) -> any`

> Get property in table by given key without calling metamethod

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `key` | `any` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="raw_set"></a>

### `raw_set(tab: table, key: any, val: any)`

> Set property in table by given key and value without calling metamethod

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `key` | `any` | *false* | *false* | *required* | - |
| `val` | `any` | *false* | *false* | *required* | - |

<a id="keys"></a>

### `keys(tab: table) -> array`

> Get an array with keys in table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

<a id="values"></a>

### `values(tab: table) -> array`

> Get an array with values in table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

<a id="has"></a>

### `has(tab: table, key: any) -> bool`

> Whether given key is in target table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `key` | `any` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="has_value"></a>

### `has_value(tab: table, val: any) -> bool`

> Whether given value is in target table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `val` | `any` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="raw_get_set"></a>

### `raw_get_set(tab: table, key: any, val: any = nil) -> any`

> Get property in tab by given key without calling metamethod. If not exist, insert with val and return it

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `key` | `any` | *false* | *false* | *required* | - |
| `val` | `any` | *false* | *true* | `nil` | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="merge"></a>

### `merge(tab: table, other: table, keep: bool = false)`

> Merge another table to this table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `other` | `table` | *false* | *false* | *required* | - |
| `keep` | `bool` | *false* | *true* | `false` | - |

<a id="remove"></a>

### `remove(tab: table, key: any) -> any`

> Remove property in table by given key

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |
| `key` | `any` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="clear"></a>

### `clear(tab: table) -> table`

> Clear table

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `tab` | `table` | *false* | *false* | *required* | - |

#### Returns

`table`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `table` |

<a id="capacity"></a>

### `capacity(cap: int) -> table`

> Create new table with given capacity

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `cap` | `int` | *false* | *false* | *required* | - |

#### Returns

`table`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `table` |

