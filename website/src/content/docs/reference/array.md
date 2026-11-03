---
title: "array"
description: "array standard library reference" 
---
<a id="array"></a>

## Contents

[pack](#pack)

[unpack](#unpack)

[has](#has)

[push](#push)

[pop](#pop)

[insert](#insert)

[remove](#remove)

[len](#len)

[concat](#concat)

[sort](#sort)

[index_of](#index_of)

## Members

<a id="pack"></a>

### `pack(...vals: T) -> T`

> Pack all arguments into an array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...vals` | `T` | *true* | *false* | - | - |

#### Returns

`T`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `T` |

<a id="unpack"></a>

### `unpack(arr: T) -> UnpackRet(T), ...`

> Unpack array into a tuple(as results)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `T` | *false* | *false* | *required* | - |

#### Returns

`UnpackRet(T), ...`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `UnpackRet(T)` |
| - | `...` |

<a id="has"></a>

### `has(arr: array<T>, who: T) -> bool`

> Whether given value is in target array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |
| `who` | `T` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="push"></a>

### `push(arr: array<T>, val: T)`

> Push a value into array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |
| `val` | `T` | *false* | *false* | *required* | - |

<a id="pop"></a>

### `pop(arr: array<T>) -> T`

> Pop a value from array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |

#### Returns

`T`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `T` |

<a id="insert"></a>

### `insert(arr: array<T>, index: int, value: T) -> array<T>`

> Insert value at given index in array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |
| `index` | `int` | *false* | *false* | *required* | - |
| `value` | `T` | *false* | *false* | *required* | - |

#### Returns

`array<T>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array<T>` |

<a id="remove"></a>

### `remove(arr: array<T>, index: int) -> array<T>`

> Remove target value at given index in array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |
| `index` | `int` | *false* | *false* | *required* | - |

#### Returns

`array<T>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array<T>` |

<a id="len"></a>

### `len(arr: array) -> int`

> Get length of array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

<a id="concat"></a>

### `concat(arr: array<T>, other: array<E>) -> array<T | E>`

> Concat two arrays

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |
| `other` | `array<E>` | *false* | *false* | *required* | - |

#### Returns

`array<T | E>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array<T | E>` |

<a id="sort"></a>

### `sort(arr: array<T>, cmp: fn(T, T) -> int | nil = nil) -> array<T>`

> Sort array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |
| `cmp` | `fn(T, T) -> int | nil` | *false* | *true* | `nil` | This function should return an integer, for `< 0` represents less, `> 0` represents greater and `= 0` represents equal (and other situations) |

#### Returns

`array<T>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array<T>` |

<a id="index_of"></a>

### `index_of(arr: array<T>, who: T | fn(T) -> bool) -> int`

> Find an item in array and return its index

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array<T>` | *false* | *false* | *required* | - |
| `who` | `T | fn(T) -> bool` | *false* | *false* | *required* | - |

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

