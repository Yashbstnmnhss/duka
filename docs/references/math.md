# math


<a id="math"></a>

## Contents

[clamp](#clamp)

[modf](#modf)

[factors](#factors)

[randf_range](#randf_range)

[max](#max)

[min](#min)

[sum](#sum)

[abs](#abs)

[round](#round)

[ceil](#ceil)

[floor](#floor)

[sin](#sin)

[cos](#cos)

[tan](#tan)

[arcsin](#arcsin)

[arccos](#arccos)

[arctan](#arctan)

[arctan2](#arctan2)

[sqrt](#sqrt)

[deg_to_rad](#deg_to_rad)

[rad_to_deg](#rad_to_deg)

[randf](#randf)

[randi](#randi)

[set_seed](#set_seed)

[log](#log)

[ln](#ln)

[log2](#log2)

[log10](#log10)

[sign](#sign)

[PI](#pi)

[E](#e)

[FLOAT_MAX](#float_max)

[INT_MAX](#int_max)

[INF](#inf)

[NAN](#nan)

## Members

<a id="clamp"></a>

### `clamp(x: float, lo: float, hi: float) -> float`

> Clamp a number into [lo, hi]

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `x` | `float` | *false* | *false* | *required* | - |
| `lo` | `float` | *false* | *false* | *required* | - |
| `hi` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="modf"></a>

### `modf(x: float) -> int, float`

> Split x into integer and fractional parts

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `x` | `float` | *false* | *false* | *required* | - |

#### Returns

`int, float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |
| 1 | `float` |

<a id="factors"></a>

### `factors(n: int) -> ...`

> Return all factors of n

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `n` | `int` | *false* | *false* | *required* | - |

#### Returns

`...`<br/>

| Index | Type |
| :--- | :--- |
| - | `...` |

<a id="randf_range"></a>

### `randf_range(lo: float, hi: float) -> float`

> Random float in [lo, hi)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `lo` | `float` | *false* | *false* | *required* | - |
| `hi` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="max"></a>

### `max(...vals: any) -> any`

> Calculate the maximum value in given values (or table)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...vals` | `any` | *true* | *false* | - | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="min"></a>

### `min(...vals: any) -> any`

> Calculate the minimum value in given values (or table)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...vals` | `any` | *true* | *false* | - | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="sum"></a>

### `sum(...vals: any) -> any`

> Calculate sum for given values (or table)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `...vals` | `any` | *true* | *false* | - | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="abs"></a>

### `abs(val: float) -> any`

> Computes the absolute value of input

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="round"></a>

### `round(val: float) -> int`

> Returns the nearest integer to self. If a value is half-way between two integers, round away from 0.0

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

<a id="ceil"></a>

### `ceil(val: float) -> int`

> Returns the smallest integer that is greater than or equal to self

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

<a id="floor"></a>

### `floor(val: float) -> int`

> Returns the largest integer that is less than or equal to self

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

<a id="sin"></a>

### `sin(val: float) -> float`

> Computes the sine of a number (in radians)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="cos"></a>

### `cos(val: float) -> float`

> Computes the cosine of a number (in radians)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="tan"></a>

### `tan(val: float) -> float`

> Computes the tangent of a number (in radians)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="arcsin"></a>

### `arcsin(val: float) -> float`

> Computes the arcsine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="arccos"></a>

### `arccos(val: float) -> float`

> Computes the arccosine of a number. Return value is in radians in the range -pi/2, pi/2 or NaN if the number is outside the range -1, 1

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="arctan"></a>

### `arctan(val: float) -> float`

> Computes the arctangent of a number. Return value is in radians in the range -pi/2, pi/2

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="arctan2"></a>

### `arctan2(val: float, val2: float) -> float`

> Computes the four quadrant arctangent of val and val2 in radians

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |
| `val2` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="sqrt"></a>

### `sqrt(val: float) -> float`

> Returns the square root of a number. Returns NaN if self is a negative number other than -0.0

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="deg_to_rad"></a>

### `deg_to_rad(val: float) -> float`

> Converts degrees to radians

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="rad_to_deg"></a>

### `rad_to_deg(val: float) -> float`

> Converts radians to degrees

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="randf"></a>

### `randf() -> float`

> Generate random float, from 0 to 1 (exclusive)

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="randi"></a>

### `randi() -> int`

> Generate random integer, from 0 to MAX

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

<a id="set_seed"></a>

### `set_seed(seed: float)`

> Set seed for random generation (only accepts integer)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `seed` | `float` | *false* | *false* | *required* | - |

<a id="log"></a>

### `log(x: float, y: float) -> float`

> Returns the base y logarithm of the x number.

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `x` | `float` | *false* | *false* | *required* | - |
| `y` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="ln"></a>

### `ln(val: float) -> float`

> Returns the base natural logarithm of the number.
> This returns NaN when the number is negative, and negative infinity when number is zero.

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="log2"></a>

### `log2(val: float) -> float`

> Returns the base 2 logarithm of the number.
> This returns NaN when the number is negative, and negative infinity when number is zero.

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="log10"></a>

### `log10(val: float) -> float`

> Returns the base 10 logarithm of the number.
> This returns NaN when the number is negative, and negative infinity when number is zero.

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`float`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `float` |

<a id="sign"></a>

### `sign(val: float) -> int`

> Returns a number that represents the sign of it

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `float` | *false* | *false* | *required* | - |

#### Returns

`int`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `int` |

<a id="pi"></a>

### Constant `PI`: float

> Archimedes' constant (π)

- Type: `float` 
- `Value: 3.14159265358979323846264338327950288` 

<a id="e"></a>

### Constant `E`: float

> Euler's number (e)

- Type: `float` 
- `Value: 2.71828182845904523536028747135266250` 

<a id="float_max"></a>

### Constant `FLOAT_MAX`: float

> Largest finite float value

- Type: `float` 
- `Value: 1.7976931348623157e+308` 

<a id="int_max"></a>

### Constant `INT_MAX`: int

> Largest finite int value

- Type: `int` 
- `Value: 9223372036854775807` 

<a id="inf"></a>

### Constant `INF`: float

> Infinity

- Type: `float` 
- `Value: INFINITY` 

<a id="nan"></a>

### Constant `NAN`: float

> Not a number

- Type: `float` 
- `Value: NAN` 

