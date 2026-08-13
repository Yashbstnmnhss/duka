

# string.`find()`
<blockquote>
Finds a substring in string (from given start index), returns its start index or nil when not found
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |
| `sub` | string | *false* | *false* | ***required*** | - |
| `from` | int | *false* | *true* | **`0`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | int | nil |







# string.`reverse()`
<blockquote>
Reverses string
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`lower()`
<blockquote>
Return a string with all ASCII characters in lowercase
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`upper()`
<blockquote>
Return a string with all ASCII characters in uppercase
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`repeat()`
<blockquote>
Repeat s n times, separated by sep
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |
| `n` | int | *false* | *false* | ***required*** | - |
| `sep` | string | *false* | *true* | **`Vec :: new()`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`trim()`
<blockquote>
Removes whitespace from both ends of this string and returns a new string, without modifying the original string
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`trim_start()`
<blockquote>
Removes whitespace from start of this string and returns a new string, without modifying the original string
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`trim_end()`
<blockquote>
Removes whitespace from end of this string and returns a new string, without modifying the original string
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`len()`
<blockquote>
Get length of string, same as #
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`substr()`
<blockquote>
Returns a portion of this string, starting at the specified index and extending for a given number of characters afterwards
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |
| `start` | int | *false* | *false* | ***required*** | - |
| `count` | int | *false* | *true* | **`- 1`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`slice()`
<blockquote>
Extracts a section [start, end) of this string and returns it as a new string, without modifying the original string
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |
| `start` | int | *false* | *false* | ***required*** | - |
| `end` | int | *false* | *true* | **`s.len() as DukaInt`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | string |







# string.`split()`
<blockquote>
Splits string s by sep
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `s` | string | *false* | *false* | ***required*** | - |
| `sep` | string | *false* | *true* | **`vec! [b' ']`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | table |




