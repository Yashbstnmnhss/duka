# iter


<a id="iter"></a>

Flags: `@returns(iterator)`

## Contents

[range](#range)

[repeat](#repeat)

[map](#map)

[filter](#filter)

[take](#take)

[skip](#skip)

[to_array](#to_array)

[all](#all)

[any](#any)

[chain](#chain)

[count](#count)

[enumerate](#enumerate)

[for_each](#for_each)

[partition](#partition)

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

<a id="repeat"></a>

### `repeat(who: any, times: int = infinity) -> any`

> Create an iterator repeats who for times (or infinity)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `who` | `any` | *false* | *false* | *required* | - |
| `times` | `int` | *false* | *true* | `infinity` | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="map"></a>

### `map(coll: any, f: fn(...) -> ...) -> any`

> Map each element of an iterable through a function, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `f` | `fn(...) -> ...` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="filter"></a>

### `filter(coll: any, pred: fn(...) -> bool) -> any`

> Keep elements for which pred returns truthy, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `pred` | `fn(...) -> bool` | *false* | *false* | *required* | - |

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

<a id="skip"></a>

### `skip(coll: any, n: int) -> any`

> Skip at most n elements from an iterable, lazily

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

<a id="all"></a>

### `all(coll: any, pred: fn(...) -> bool) -> bool`

> Collect all elements of an iterable, check whether all of them fit predication

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `pred` | `fn(...) -> bool` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="any"></a>

### `any(coll: any, pred: fn(...) -> bool) -> bool`

> Collect all elements of an iterable, check whether any of them fits predication

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `pred` | `fn(...) -> bool` | *false* | *false* | *required* | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="chain"></a>

### `chain(coll: any, other: any) -> any`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `other` | `any` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="count"></a>

### `count(coll: any) -> int`

> Return the count of all elements of an iterable into an array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

<a id="enumerate"></a>

### `enumerate(coll: any) -> any`

> Creates an iterator which gives the current iteration count as well as the next value

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="for_each"></a>

### `for_each(coll: any, f: fn(...))`

> `foreach item in ...`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `f` | `fn(...)` | *false* | *false* | *required* | - |

<a id="partition"></a>

### `partition(coll: any, pred: fn(...) -> bool) -> array, array`

> Make partition of source by predication

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `pred` | `fn(...) -> bool` | *false* | *false* | *required* | - |

#### Returns

`array, array`<br/>First is `true`, second is `false`, both array

| Index | Type |
| :--- | :--- |
| 0 | `array` |
| 1 | `array` |

