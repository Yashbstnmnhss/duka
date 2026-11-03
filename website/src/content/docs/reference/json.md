---
title: "json"
description: "json standard library reference" 
---
<a id="json"></a>

> Read & write JSON data format

Flags: `@feature(json)`

## Contents

[parse](#parse)

[stringify](#stringify)

## Members

<a id="parse"></a>

### `parse(json: string) -> Result2(any), ...`

> Parse a JSON string

Flags: `@returns(result)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `json` | `string` | *false* | *false* | *required* | - |

#### Returns

`Result2(any), ...`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Result2(any)` |
| - | `...` |

<a id="stringify"></a>

### `stringify(val: any) -> Result2(string), ...`

> Convert a value into JSON string

Flags: `@returns(result)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `val` | `any` | *false* | *false* | *required* | - |

#### Returns

`Result2(string), ...`<br/>

| Index | Type |
| :--- | :--- |
| 0 | `Result2(string)` |
| - | `...` |

