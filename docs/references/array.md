# array


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

## Members

<a id="pack"></a>

### `pack(...vals: any) -> array`

> Pack all arguments into an array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...vals` | `any` | *true* | *false* | - | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

<a id="unpack"></a>

### `unpack(arr: array) -> ...`

> Unpack array into a tuple(as results)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |

#### Returns

`...`<br/>

| Index | Type |
| :--- | :--- |
| - | `...` |

<a id="has"></a>

### `has(arr: array, who: any) -> bool`

> Whether given value is in target array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |
| `who` | `any` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="push"></a>

### `push(arr: array, val: any)`

> Push a value into array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |
| `val` | `any` | *false* | *false* | *required* | - |

<a id="pop"></a>

### `pop(arr: array) -> any`

> Pop a value from array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="insert"></a>

### `insert(arr: array, index: int, value: any) -> array`

> Insert value at given index in array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |
| `index` | `int` | *false* | *false* | *required* | - |
| `value` | `any` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

<a id="remove"></a>

### `remove(arr: array, index: int) -> array`

> Remove target value at given index in array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |
| `index` | `int` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

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

### `concat(arr: array, other: array) -> array`

> Concat two arrays

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `arr` | `array` | *false* | *false* | *required* | - |
| `other` | `array` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

