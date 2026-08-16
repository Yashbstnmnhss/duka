# string

[Index](index.md)

## Contents

- [find](#find)
- [reverse](#reverse)
- [lower](#lower)
- [upper](#upper)
- [repeat](#repeat)
- [trim](#trim)
- [trim_start](#trim_start)
- [trim_end](#trim_end)
- [len](#len)
- [substr](#substr)
- [slice](#slice)
- [split](#split)

## Members

<a id="find"></a>
### `find(s: string, sub: string, from: int = 0) -> int | nil`

<blockquote>
Finds a substring in string (from given start index), returns its start index or nil when not found
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |
| `sub` | string | *false* | *false* | *required* | - |
| `from` | int | *false* | *true* | `0` | - |

## Returns

`int | nil`



| Index | Type |
| :--- | :---: |
| 0 | int | nil |

<a id="reverse"></a>
### `reverse(s: string) -> string`

<blockquote>
Reverses string
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="lower"></a>
### `lower(s: string) -> string`

<blockquote>
Return a string with all ASCII characters in lowercase
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="upper"></a>
### `upper(s: string) -> string`

<blockquote>
Return a string with all ASCII characters in uppercase
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="repeat"></a>
### `repeat(s: string, n: int, sep: string = Vec :: new()) -> string`

<blockquote>
Repeat s n times, separated by sep
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |
| `n` | int | *false* | *false* | *required* | - |
| `sep` | string | *false* | *true* | `Vec :: new()` | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="trim"></a>
### `trim(s: string) -> string`

<blockquote>
Removes whitespace from both ends of this string and returns a new string, without modifying the original string
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="trim_start"></a>
### `trim_start(s: string) -> string`

<blockquote>
Removes whitespace from start of this string and returns a new string, without modifying the original string
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="trim_end"></a>
### `trim_end(s: string) -> string`

<blockquote>
Removes whitespace from end of this string and returns a new string, without modifying the original string
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="len"></a>
### `len(s: string) -> string`

<blockquote>
Get length of string, same as #
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="substr"></a>
### `substr(s: string, start: int, count: int = - 1) -> string`

<blockquote>
Returns a portion of this string, starting at the specified index and extending for a given number of characters afterwards
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |
| `start` | int | *false* | *false* | *required* | - |
| `count` | int | *false* | *true* | `- 1` | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="slice"></a>
### `slice(s: string, start: int, end: int = s.len() as DukaInt) -> string`

<blockquote>
Extracts a section [start, end) of this string and returns it as a new string, without modifying the original string
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |
| `start` | int | *false* | *false* | *required* | - |
| `end` | int | *false* | *true* | `s.len() as DukaInt` | - |

## Returns

`string`



| Index | Type |
| :--- | :---: |
| 0 | string |

<a id="split"></a>
### `split(s: string, sep: string = vec! [b' ']) -> table`

<blockquote>
Splits string s by sep
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `s` | string | *false* | *false* | *required* | - |
| `sep` | string | *false* | *true* | `vec! [b' ']` | - |

## Returns

`table`



| Index | Type |
| :--- | :---: |
| 0 | table |
