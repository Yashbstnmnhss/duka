

# `require()`
<blockquote>
Import module by pattern
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `pattern` | string | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `print()`
<blockquote>
Prints to standard output
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `args` | - | *true* | *false* | **-** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `type()`
<blockquote>
Get type name of value
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | any | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `to_string()`
<blockquote>
Convert to string
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | any | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `to_number()`
<blockquote>
Convert to number
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | any | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `assert()`
<blockquote>
Assertion
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `cond` | any | *false* | *false* | ***required*** | - |
| `msg` | string | *false* | *true* | **`"assertion failed".to_owned()`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `error()`
<blockquote>
Raise an error
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `msg` | string | *false* | *true* | **`"error".to_owned()`** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `is_error()`
<blockquote>
Check if it is an error
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | - | *true* | *false* | **-** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `unwrap()`
<blockquote>
Unwrap a result
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | - | *true* | *false* | **-** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `get_metatable()`
<blockquote>
Get metatable
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | table | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `set_metatable()`
<blockquote>
Set metatable
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | table | *false* | *false* | ***required*** | - |
| `metatable` | table | nil | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | table |







# `instanceof()`
<blockquote>
Check if the value is an instance of target
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `value` | any | *false* | *false* | ***required*** | - |
| `target` | any | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `pairs()`
<blockquote>
Return (iter, table, nil) tuple for table
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `tab` | table | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `ipairs()`
<blockquote>
Return (iter_index, table, nil) tuple for table
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `tab` | table | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `costatus()`
<blockquote>
Get coroutine's status
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `coroutine` | any | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# `try()`
<blockquote>
Run a function in protected mode, results follow Result Protocol
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `func` | function | table | *false* | *false* | ***required*** | - |
| `params` | - | *true* | *false* | **-** | - |

## Returns

| Index | Type | 
| :--- | :---: | 




