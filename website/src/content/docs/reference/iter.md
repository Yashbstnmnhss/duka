---
title: "iter"
description: "iterstandard library reference" 
---
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

### `range(from: int, to: int, step: int = 1) -> Yield(false, int)`

> Create an iterator over a range [from, to)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `from` | `int` | *false* | *false* | *required* | - |
| `to` | `int` | *false* | *false* | *required* | - |
| `step` | `int` | *false* | *true* | `1` | - |

#### Returns

`Yield(false, int)`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Yield(false, int)` |

<a id="repeat"></a>

### `repeat(who: T, times: int = infinity) -> Yield(false, T)`

> Create an iterator repeats who for times (or infinity)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `who` | `T` | *false* | *false* | *required* | - |
| `times` | `int` | *false* | *true* | `infinity` | - |

#### Returns

`Yield(false, T)`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Yield(false, T)` |

<a id="map"></a>

### `map(coll: any, f: fn(...) -> U) -> Yield(false, U)`

> Map each element of an iterable through a function, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `any` | *false* | *false* | *required* | - |
| `f` | `fn(...) -> U` | *false* | *false* | *required* | - |

#### Returns

`Yield(false, U)`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Yield(false, U)` |

<a id="filter"></a>

### `filter(coll: T, pred: fn(...) -> bool) -> Yield(false, Elem(T))`

> Keep elements for which pred returns truthy, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `T` | *false* | *false* | *required* | - |
| `pred` | `fn(...) -> bool` | *false* | *false* | *required* | - |

#### Returns

`Yield(false, Elem(T))`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Yield(false, Elem(T))` |

<a id="take"></a>

### `take(coll: T, n: int) -> Yield(false, Elem(T))`

> Take at most n elements from an iterable, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `T` | *false* | *false* | *required* | - |
| `n` | `int` | *false* | *false* | *required* | - |

#### Returns

`Yield(false, Elem(T))`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Yield(false, Elem(T))` |

<a id="skip"></a>

### `skip(coll: T, n: int) -> Yield(false, Elem(T))`

> Skip at most n elements from an iterable, lazily

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `T` | *false* | *false* | *required* | - |
| `n` | `int` | *false* | *false* | *required* | - |

#### Returns

`Yield(false, Elem(T))`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Yield(false, Elem(T))` |

<a id="to_array"></a>

### `to_array(coll: T) -> array<Elem(T)>`

> Collect all elements of an iterable into an array

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `T` | *false* | *false* | *required* | - |

#### Returns

`array<Elem(T)>`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array<Elem(T)>` |

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

### `partition(coll: T, pred: fn(...) -> bool) -> array<Elem(T)>, array<Elem(T)>`

> Make partition of source by predication

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `coll` | `T` | *false* | *false* | *required* | - |
| `pred` | `fn(...) -> bool` | *false* | *false* | *required* | - |

#### Returns

`array<Elem(T)>, array<Elem(T)>`<br/>First is `true`, second is `false`, both array

| Index | Type |
| :--- | :--- |
| 0 | `array<Elem(T)>` |
| 1 | `array<Elem(T)>` |

