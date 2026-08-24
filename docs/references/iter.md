# iter


<a id="iter"></a>

Flags: `@returns(iterator)`

## Contents

[range](#range)

[map](#map)

[filter](#filter)

[take](#take)

[to_array](#to_array)

## Members

<a id="range"></a>

### `range(from: int, to: int, step: int = 1) -> any`

> Create an iterator over a range [from, to)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `from` | `int` | *false* | *false* | *required* | - |
| `to` | `int` | *false* | *false* | *required* | - |
| `step` | `int` | *false* | *true* | `1` | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="map"></a>

### `map(coll: any, f: function) -> any`

> Map each element of an iterable through a function, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `f` | `function` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="filter"></a>

### `filter(coll: any, pred: function) -> any`

> Keep elements for which pred returns truthy, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `pred` | `function` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="take"></a>

### `take(coll: any, n: int) -> any`

> Take at most n elements from an iterable, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `n` | `int` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="to_array"></a>

### `to_array(coll: any) -> array`

> Collect all elements of an iterable into an array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

