# array


<a id="array"></a>

## Contents

[pack](#pack)

[unpack](#unpack)

[has](#has)

[push](#push)

[pop](#pop)

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

