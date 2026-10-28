---
title: json
description: Read & write JSON data format.
---

<a id="json"></a>

> Read & write JSON data format

Flags: `@feature(json)`

## Contents

[parse](#parse)

[stringify](#stringify)

## Members

<a id="parse"></a>

### `parse(json: string) -> ...`

> Parse a JSON string

Flags: `@returns(result)`

#### Params

| Name   | Type     | VarArg? | Optional? | Default    | Doc |
| :----- | :------- | :------ | :-------- | :--------- | :-- |
| `json` | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="stringify"></a>

### `stringify(val: any) -> ...`

> Convert a value into JSON string

Flags: `@returns(result)`

#### Params

| Name  | Type  | VarArg? | Optional? | Default    | Doc |
| :---- | :---- | :------ | :-------- | :--------- | :-- |
| `val` | `any` | _false_ | _false_   | _required_ | -   |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |
