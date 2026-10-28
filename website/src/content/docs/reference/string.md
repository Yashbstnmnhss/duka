---
title: string
description: String builtins.
---

<a id="string"></a>

## Contents

[find](#find)

[reverse](#reverse)

[lower](#lower)

[upper](#upper)

[repeat](#repeat)

[trim](#trim)

[trim_start](#trim_start)

[trim_end](#trim_end)

[len](#len)

[substr](#substr)

[slice](#slice)

[split](#split)

[concat](#concat)

## Members

<a id="find"></a>

### `find(s: string, sub: string, from: int = 0) -> int | nil`

> Finds a substring in string (from given start index), returns its start index or nil when not found

#### Params

| Name   | Type     | VarArg? | Optional? | Default    | Doc |
| :----- | :------- | :------ | :-------- | :--------- | :-- |
| `s`    | `string` | _false_ | _false_   | _required_ | -   |
| `sub`  | `string` | _false_ | _false_   | _required_ | -   |
| `from` | `int`    | _false_ | _true_    | `0`        | -   |

#### Returns

`int | nil`<br/>

| Index | Type |
| :---- | :--- | ---- |
| 0     | `int | nil` |

<a id="reverse"></a>

### `reverse(s: string) -> string`

> Reverses string

#### Params

| Name | Type     | VarArg? | Optional? | Default    | Doc |
| :--- | :------- | :------ | :-------- | :--------- | :-- |
| `s`  | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="lower"></a>

### `lower(s: string) -> string`

> Return a string with all ASCII characters in lowercase

#### Params

| Name | Type     | VarArg? | Optional? | Default    | Doc |
| :--- | :------- | :------ | :-------- | :--------- | :-- |
| `s`  | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="upper"></a>

### `upper(s: string) -> string`

> Return a string with all ASCII characters in uppercase

#### Params

| Name | Type     | VarArg? | Optional? | Default    | Doc |
| :--- | :------- | :------ | :-------- | :--------- | :-- |
| `s`  | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="repeat"></a>

### `repeat(s: string, n: int, sep: string = "") -> string`

> Repeat s n times, separated by sep

#### Params

| Name  | Type     | VarArg? | Optional? | Default    | Doc |
| :---- | :------- | :------ | :-------- | :--------- | :-- |
| `s`   | `string` | _false_ | _false_   | _required_ | -   |
| `n`   | `int`    | _false_ | _false_   | _required_ | -   |
| `sep` | `string` | _false_ | _true_    | `""`       | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="trim"></a>

### `trim(s: string) -> string`

> Removes whitespace from both ends of this string and returns a new string, without modifying the original string

#### Params

| Name | Type     | VarArg? | Optional? | Default    | Doc |
| :--- | :------- | :------ | :-------- | :--------- | :-- |
| `s`  | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="trim_start"></a>

### `trim_start(s: string) -> string`

> Removes whitespace from start of this string and returns a new string, without modifying the original string

#### Params

| Name | Type     | VarArg? | Optional? | Default    | Doc |
| :--- | :------- | :------ | :-------- | :--------- | :-- |
| `s`  | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="trim_end"></a>

### `trim_end(s: string) -> string`

> Removes whitespace from end of this string and returns a new string, without modifying the original string

#### Params

| Name | Type     | VarArg? | Optional? | Default    | Doc |
| :--- | :------- | :------ | :-------- | :--------- | :-- |
| `s`  | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="len"></a>

### `len(s: string) -> string`

> Get length of string based on characters instead of bytes

#### Params

| Name | Type     | VarArg? | Optional? | Default    | Doc |
| :--- | :------- | :------ | :-------- | :--------- | :-- |
| `s`  | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="substr"></a>

### `substr(s: string, start: int, count: int = - 1) -> string`

> Returns a portion of this string, starting at the specified index and extending for a given number of characters afterwards

#### Params

| Name    | Type     | VarArg? | Optional? | Default    | Doc |
| :------ | :------- | :------ | :-------- | :--------- | :-- |
| `s`     | `string` | _false_ | _false_   | _required_ | -   |
| `start` | `int`    | _false_ | _false_   | _required_ | -   |
| `count` | `int`    | _false_ | _true_    | `- 1`      | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="slice"></a>

### `slice(s: string, start: int, end: int = #s) -> string`

> Extracts a section [start, end) of this string and returns it as a new string, without modifying the original string

#### Params

| Name    | Type     | VarArg? | Optional? | Default    | Doc |
| :------ | :------- | :------ | :-------- | :--------- | :-- |
| `s`     | `string` | _false_ | _false_   | _required_ | -   |
| `start` | `int`    | _false_ | _false_   | _required_ | -   |
| `end`   | `int`    | _false_ | _true_    | `#s`       | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |

<a id="split"></a>

### `split(s: string, sep: string = " ") -> table`

> Splits string s by sep

#### Params

| Name  | Type     | VarArg? | Optional? | Default    | Doc |
| :---- | :------- | :------ | :-------- | :--------- | :-- |
| `s`   | `string` | _false_ | _false_   | _required_ | -   |
| `sep` | `string` | _false_ | _true_    | `" "`      | -   |

#### Returns

`table`<br/>

| Index | Type    |
| :---- | :------ |
| 0     | `table` |

<a id="concat"></a>

### `concat(...vals: any) -> string`

> Concat all arguments into one string

#### Params

| Name      | Type  | VarArg? | Optional? | Default | Doc |
| :-------- | :---- | :------ | :-------- | :------ | :-- |
| `...vals` | `any` | _true_  | _false_   | -       | -   |

#### Returns

`string`<br/>

| Index | Type     |
| :---- | :------- |
| 0     | `string` |
