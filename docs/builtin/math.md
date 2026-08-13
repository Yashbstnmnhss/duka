

# math.`clamp()`
<blockquote>
Clamp a number into [lo, hi]
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `x` | float | *false* | *false* | ***required*** | - |
| `lo` | float | *false* | *false* | ***required*** | - |
| `hi` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`modf()`
<blockquote>
Split x into integer and fractional parts
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `x` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | int |
| 1 | float |







# math.`factors()`
<blockquote>
Return all factors of n
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `n` | int | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| - | `...` |






# math.`randf_range()`
<blockquote>
Random float in [lo, hi)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `lo` | float | *false* | *false* | ***required*** | - |
| `hi` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`max()`
<blockquote>
Calculate the maximum value in given values (or table)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `vals` | - | *true* | *false* | **-** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# math.`min()`
<blockquote>
Calculate the minimum value in given values (or table)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `vals` | - | *true* | *false* | **-** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# math.`sum()`
<blockquote>
Calculate sum for given values (or table)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `vals` | - | *true* | *false* | **-** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# math.`abs()`
<blockquote>
Computes the absolute value of input
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | any |







# math.`round()`
<blockquote>
Returns the nearest integer to self. If a value is half-way between two integers, round away from 0.0
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | int |







# math.`ceil()`
<blockquote>
Returns the smallest integer that is greater than or equal to self
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | int |







# math.`floor()`
<blockquote>
Returns the largest integer that is less than or equal to self
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | int |







# math.`sin()`
<blockquote>
Computes the sine of a number (in radians)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`cos()`
<blockquote>
Computes the cosine of a number (in radians)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`tan()`
<blockquote>
Computes the tangent of a number (in radians)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`arcsin()`
<blockquote>
Computes the arcsine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`arccos()`
<blockquote>
Computes the arccosine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`arctan()`
<blockquote>
Computes the arctangent of a number. Return value is in radians in the range -pi/2, pi/2
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`arctan2()`
<blockquote>
Computes the four quadrant arctangent of val and val2 in radians
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |
| `val2` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`sqrt()`
<blockquote>
Returns the square root of a number. Returns NaN if self is a negative number other than -0.0
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`deg_to_rad()`
<blockquote>
Converts degrees to radians
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`rad_to_deg()`
<blockquote>
Converts radians to degrees
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`randf()`
<blockquote>
Generate random float, from 0 to 1 (exclusive)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`randi()`
<blockquote>
Generate random integer, from 0 to MAX
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | int |







# math.`set_seed()`
<blockquote>
Set seed for random generation (only accepts integer)
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `seed` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 







# math.`log()`
<blockquote>
Returns the base y logarithm of the x number.
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `x` | float | *false* | *false* | ***required*** | - |
| `y` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`ln()`
<blockquote>
Returns the base natural logarithm of the number.
This returns NaN when the number is negative, and negative infinity when number is zero.
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`log2()`
<blockquote>
Returns the base 2 logarithm of the number.
This returns NaN when the number is negative, and negative infinity when number is zero.
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`log10()`
<blockquote>
Returns the base 10 logarithm of the number.
This returns NaN when the number is negative, and negative infinity when number is zero.
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | float |







# math.`sign()`
<blockquote>
Returns a number that represents the sign of it
</blockquote>


## Params
| Name | Type | VarArg? | Optional? | Default | Doc | 
 | :--- | :---: | :---: | :---: | :---: | :--- | 
 | `val` | float | *false* | *false* | ***required*** | - |

## Returns

| Index | Type | 
| :--- | :---: | 
| 0 | int |







# math.`PI`
<blockquote>
Archimedes' constant (π)
</blockquote>


- Type: any
- Value: 3.14159265358979323846264338327950288






# math.`E`
<blockquote>
Euler's number (e)
</blockquote>


- Type: any
- Value: 2.71828182845904523536028747135266250






# math.`FLOAT_MAX`
<blockquote>
Largest finite float value
</blockquote>


- Type: any
- Value: 1.7976931348623157e+308






# math.`INT_MAX`
<blockquote>
Largest finite int value
</blockquote>


- Type: any
- Value: 9223372036854775807






# math.`INF`
<blockquote>
Infinity
</blockquote>


- Type: any
- Value: INFINITY






# math.`NAN`
<blockquote>
Not a number
</blockquote>


- Type: any
- Value: NAN



