# regex


<a id="regex"></a>

> RegEx for duka

## Contents

[search](#search)

[find_all](#find_all)

[compile](#compile)

[escape](#escape)

[replace](#replace)

[replace_all](#replace_all)

[is_match](#is_match)

[CompiledRegex](#compiledregex)

## Members

<a id="search"></a>

### `search(pattern: string, text: string, from: int = 0) -> ...`

> Search a substring by given pattern in text (search once)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |
| `text` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`...`<br/>

| Index | Type |
| :--- | :--- |
| - | `...` |

<a id="find_all"></a>

### `find_all(pattern: string, text: string) -> array, array`

> Find all strings by given pattern (global mode)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |
| `text` | `string` | *false* | *false* | *required* | - |

#### Returns

`array, array`<br/>Nested array, `[[captures1...], [captures2...]]`

| Index | Type |
| :--- | :--- |
| 0 | `array` |
| 1 | `array` |

<a id="compile"></a>

### `compile(pattern: string) -> any`

> Compile a pattern into CompiledRegex

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |

#### Returns

`any`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `any` |

<a id="escape"></a>

### `escape(pattern: string) -> string`

> Escape a regex string

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="replace"></a>

### `replace(pattern: string, text: string, replacement: string, from: int = 0, times: int = 1) -> string`

> Replace given string by given pattern in text to replacement (replace **once** by default)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |
| `text` | `string` | *false* | *false* | *required* | - |
| `replacement` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |
| `times` | `int` | *false* | *true* | `1` | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="replace_all"></a>

### `replace_all(pattern: string, text: string, replacement: string, from: int = 0) -> string`

> Replace given string by given pattern in text to replacement (replace **all**)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |
| `text` | `string` | *false* | *false* | *required* | - |
| `replacement` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="is_match"></a>

### `is_match(pattern: string, text: string, from: int = 0) -> bool`

> Whether given text matches given pattern

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |
| `text` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="compiledregex"></a>

### UserData `CompiledRegex:CompiledRegex`

> Compiled regex object

#### Methods

<a id="replace_all"></a>

#### `replace_all(self: any, text: string, replacement: string, from: int = 0) -> string`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |
| `replacement` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="replacen"></a>

#### `replacen(self: any, text: string, replacement: string, from: int = 0, times: int = 1) -> string`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |
| `replacement` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |
| `times` | `int` | *false* | *true* | `1` | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="replace"></a>

#### `replace(self: any, text: string, replacement: string, from: int = 0) -> string`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |
| `replacement` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`string`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `string` |

<a id="is_match"></a>

#### `is_match(self: any, text: string, from: int = 0) -> bool`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`bool`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |

<a id="search"></a>

#### `search(self: any, text: string, from: int = 0) -> bool, ...`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`bool, ...`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `bool` |
| - | `...` |

<a id="find_all"></a>

#### `find_all(self: any, text: string) -> array, array`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |

#### Returns

`array, array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |
| 1 | `array` |

