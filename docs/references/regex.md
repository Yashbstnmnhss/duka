# regex


<a id="regex"></a>

> Regex for duka

## Contents

[search](#search)

[find_all](#find_all)

[compile](#compile)

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

### `find_all(pattern: string, text: string) -> array`

> Find all strings by given pattern (global mode)

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `pattern` | `string` | *false* | *false* | *required* | - |
| `text` | `string` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>Nested array, `[[captures1...], [captures2...]]`

| Index | Type |
| :--- | :--- |
| 0 | `array` |

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

<a id="compiledregex"></a>

### UserData `CompiledRegex:CompiledRegex`

> Compiled regex object

#### Methods

<a id="search"></a>

#### `search(self: any, text: string, from: int = 0) -> ...`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |
| `from` | `int` | *false* | *true* | `0` | - |

#### Returns

`...`<br/>

| Index | Type |
| :--- | :--- |
| - | `...` |

<a id="find_all"></a>

#### `find_all(self: any, text: string) -> array`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `self` | `any` | *false* | *false* | *required* | CompiledRegex |
| `text` | `string` | *false* | *false* | *required* | - |

#### Returns

`array`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `array` |

