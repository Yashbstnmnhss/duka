# math

[Index](index.md)

## Contents

- [clamp](#clamp)
- [modf](#modf)
- [factors](#factors)
- [randf_range](#randf_range)
- [max](#max)
- [min](#min)
- [sum](#sum)
- [abs](#abs)
- [round](#round)
- [ceil](#ceil)
- [floor](#floor)
- [sin](#sin)
- [cos](#cos)
- [tan](#tan)
- [arcsin](#arcsin)
- [arccos](#arccos)
- [arctan](#arctan)
- [arctan2](#arctan2)
- [sqrt](#sqrt)
- [deg_to_rad](#deg_to_rad)
- [rad_to_deg](#rad_to_deg)
- [randf](#randf)
- [randi](#randi)
- [set_seed](#set_seed)
- [log](#log)
- [ln](#ln)
- [log2](#log2)
- [log10](#log10)
- [sign](#sign)
- [PI](#pi)
- [E](#e)
- [FLOAT_MAX](#float_max)
- [INT_MAX](#int_max)
- [INF](#inf)
- [NAN](#nan)

## Members

<a id="clamp"></a>
### `clamp(x: float, lo: float, hi: float) -> float`

<blockquote>
Clamp a number into [lo, hi]
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `x` | float | *false* | *false* | *required* | - |
| `lo` | float | *false* | *false* | *required* | - |
| `hi` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="modf"></a>
### `modf(x: float) -> int, float`

<blockquote>
Split x into integer and fractional parts
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `x` | float | *false* | *false* | *required* | - |

## Returns

`int, float`



| Index | Type |
| :--- | :---: |
| 0 | int |
| 1 | float |

<a id="factors"></a>
### `factors(n: int) -> ...`

<blockquote>
Return all factors of n
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `n` | int | *false* | *false* | *required* | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="randf_range"></a>
### `randf_range(lo: float, hi: float) -> float`

<blockquote>
Random float in [lo, hi)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `lo` | float | *false* | *false* | *required* | - |
| `hi` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="max"></a>
### `max(...vals: any) -> any`

<blockquote>
Calculate the maximum value in given values (or table)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `...vals` | any | *true* | *false* | - | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |

<a id="min"></a>
### `min(...vals: any) -> any`

<blockquote>
Calculate the minimum value in given values (or table)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `...vals` | any | *true* | *false* | - | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |

<a id="sum"></a>
### `sum(...vals: any) -> any`

<blockquote>
Calculate sum for given values (or table)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `...vals` | any | *true* | *false* | - | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |

<a id="abs"></a>
### `abs(val: float) -> any`

<blockquote>
Computes the absolute value of input
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |

<a id="round"></a>
### `round(val: float) -> int`

<blockquote>
Returns the nearest integer to self. If a value is half-way between two integers, round away from 0.0
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`int`



| Index | Type |
| :--- | :---: |
| 0 | int |

<a id="ceil"></a>
### `ceil(val: float) -> int`

<blockquote>
Returns the smallest integer that is greater than or equal to self
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`int`



| Index | Type |
| :--- | :---: |
| 0 | int |

<a id="floor"></a>
### `floor(val: float) -> int`

<blockquote>
Returns the largest integer that is less than or equal to self
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`int`



| Index | Type |
| :--- | :---: |
| 0 | int |

<a id="sin"></a>
### `sin(val: float) -> float`

<blockquote>
Computes the sine of a number (in radians)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="cos"></a>
### `cos(val: float) -> float`

<blockquote>
Computes the cosine of a number (in radians)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="tan"></a>
### `tan(val: float) -> float`

<blockquote>
Computes the tangent of a number (in radians)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="arcsin"></a>
### `arcsin(val: float) -> float`

<blockquote>
Computes the arcsine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="arccos"></a>
### `arccos(val: float) -> float`

<blockquote>
Computes the arccosine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="arctan"></a>
### `arctan(val: float) -> float`

<blockquote>
Computes the arctangent of a number. Return value is in radians in the range -pi/2, pi/2
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="arctan2"></a>
### `arctan2(val: float, val2: float) -> float`

<blockquote>
Computes the four quadrant arctangent of val and val2 in radians
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |
| `val2` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="sqrt"></a>
### `sqrt(val: float) -> float`

<blockquote>
Returns the square root of a number. Returns NaN if self is a negative number other than -0.0
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="deg_to_rad"></a>
### `deg_to_rad(val: float) -> float`

<blockquote>
Converts degrees to radians
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="rad_to_deg"></a>
### `rad_to_deg(val: float) -> float`

<blockquote>
Converts radians to degrees
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="randf"></a>
### `randf() -> float`

<blockquote>
Generate random float, from 0 to 1 (exclusive)
</blockquote>

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="randi"></a>
### `randi() -> int`

<blockquote>
Generate random integer, from 0 to MAX
</blockquote>

## Returns

`int`



| Index | Type |
| :--- | :---: |
| 0 | int |

<a id="set_seed"></a>
### `set_seed(seed: float)`

<blockquote>
Set seed for random generation (only accepts integer)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `seed` | float | *false* | *false* | *required* | - |

<a id="log"></a>
### `log(x: float, y: float) -> float`

<blockquote>
Returns the base y logarithm of the x number.
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `x` | float | *false* | *false* | *required* | - |
| `y` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="ln"></a>
### `ln(val: float) -> float`

<blockquote>
Returns the base natural logarithm of the number.
This returns NaN when the number is negative, and negative infinity when number is zero.
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="log2"></a>
### `log2(val: float) -> float`

<blockquote>
Returns the base 2 logarithm of the number.
This returns NaN when the number is negative, and negative infinity when number is zero.
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="log10"></a>
### `log10(val: float) -> float`

<blockquote>
Returns the base 10 logarithm of the number.
This returns NaN when the number is negative, and negative infinity when number is zero.
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`float`



| Index | Type |
| :--- | :---: |
| 0 | float |

<a id="sign"></a>
### `sign(val: float) -> int`

<blockquote>
Returns a number that represents the sign of it
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `val` | float | *false* | *false* | *required* | - |

## Returns

`int`



| Index | Type |
| :--- | :---: |
| 0 | int |

<a id="pi"></a>
### Constant `PI: float`

<blockquote>
Archimedes' constant (π)
</blockquote>

- Type: float
- Value: `3.14159265358979323846264338327950288`

<a id="e"></a>
### Constant `E: float`

<blockquote>
Euler's number (e)
</blockquote>

- Type: float
- Value: `2.71828182845904523536028747135266250`

<a id="float_max"></a>
### Constant `FLOAT_MAX: float`

<blockquote>
Largest finite float value
</blockquote>

- Type: float
- Value: `1.7976931348623157e+308`

<a id="int_max"></a>
### Constant `INT_MAX: int`

<blockquote>
Largest finite int value
</blockquote>

- Type: int
- Value: `9223372036854775807`

<a id="inf"></a>
### Constant `INF: float`

<blockquote>
Infinity
</blockquote>

- Type: float
- Value: `INFINITY`

<a id="nan"></a>
### Constant `NAN: float`

<blockquote>
Not a number
</blockquote>

- Type: float
- Value: `NAN`
