

# `require()`
<blockquote>
Import module by pattern
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `pattern` | string | *false* | *false* | ***required*** |  |

## Returns
[1] 






# `print()`
<blockquote>
Prints to standard output
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `args` | any | *true* | *false* | ***empty*** |  |

## Returns
[None] 






# `type()`
<blockquote>
Get type name of value
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | any | *false* | *false* | ***required*** |  |

## Returns
[1] 






# `to_string()`
<blockquote>
Convert to string
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | any | *false* | *false* | ***required*** |  |

## Returns
[1] 






# `to_number()`
<blockquote>
Convert to number
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | any | *false* | *false* | ***required*** |  |

## Returns
[1] 






# `assert()`
<blockquote>
Assertion
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `cond` | any | *false* | *false* | ***required*** |  |
| `msg` | string | *false* | *true* | **`"assertion failed".to_owned()`** |  |

## Returns
[1] 






# `error()`
<blockquote>
Raise an error
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `msg` | string | *false* | *true* | **`"error".to_owned()`** |  |

## Returns
[None] 






# `get_metatable()`
<blockquote>
Get metatable
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | table | *false* | *false* | ***required*** |  |

## Returns
[1] 






# `set_metatable()`
<blockquote>
Set metatable
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | table | *false* | *false* | ***required*** |  |
| `metatable` | any | *false* | *false* | ***required*** |  |

## Returns
[1] 






# `instanceof()`
<blockquote>
Check if the value is an instance of target
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `value` | any | *false* | *false* | ***required*** |  |
| `target` | any | *false* | *false* | ***required*** |  |

## Returns
[1] 






# `pairs()`
<blockquote>
Return (iter, table, nil) tuple for table
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `tab` | table | *false* | *false* | ***required*** |  |

## Returns
[3] 






# `ipairs()`
<blockquote>
Return (iter_index, table, nil) tuple for table
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `tab` | table | *false* | *false* | ***required*** |  |

## Returns
[3] 



