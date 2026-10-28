---
title: os
description: Functions interacting with the OS.
---

<a id="os"></a>

> Provide some functions interacting with OS

Flags: `@feature(platform)`

## Contents

[execute](#execute)

[exit](#exit)

[remove](#remove)

[rename](#rename)

[clock](#clock)

[time](#time)

[date](#date)

## Members

<a id="execute"></a>

### `execute(cmd: string)`

> Run a process with command, depends on platform

Flags: `@returns(result)`

#### Params

| Name  | Type     | VarArg? | Optional? | Default    | Doc |
| :---- | :------- | :------ | :-------- | :--------- | :-- |
| `cmd` | `string` | _false_ | _false_   | _required_ | -   |

<a id="exit"></a>

### `exit(code: int = 0)`

> Terminates program with exit code (default = 0)

Flags: `@returns(exit)`

#### Params

| Name   | Type  | VarArg? | Optional? | Default | Doc |
| :----- | :---- | :------ | :-------- | :------ | :-- |
| `code` | `int` | _false_ | _true_    | `0`     | -   |

<a id="remove"></a>

### `remove(path: string) -> ...`

> Removes a file or an **empty** directory from the filesystem

Flags: `@returns(result)`

#### Params

| Name   | Type     | VarArg? | Optional? | Default    | Doc |
| :----- | :------- | :------ | :-------- | :--------- | :-- |
| `path` | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="rename"></a>

### `rename(path: string, name: string) -> ...`

> Renames a file or directory to a new name, replacing the original file if `name` already exists

Flags: `@returns(result)`

#### Params

| Name   | Type     | VarArg? | Optional? | Default    | Doc |
| :----- | :------- | :------ | :-------- | :--------- | :-- |
| `path` | `string` | _false_ | _false_   | _required_ | -   |
| `name` | `string` | _false_ | _false_   | _required_ | -   |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="clock"></a>

### `clock() -> float | nil`

> Get seconds from this program's start time, returns nil if not available

#### Returns

`float | nil`<br/>

| Index | Type   |
| :---- | :----- | ---- |
| 0     | `float | nil` |

<a id="time"></a>

### `time() -> int`

> Get current timestamp from the UNIX epoch, throws if system time is before epoch

#### Returns

`int`<br/>

| Index | Type  |
| :---- | :---- |
| 0     | `int` |

<a id="date"></a>

### `date() -> table`

> Get formatted current date string, throws if system time is before epoch

#### Returns

`table`<br/>

| Index | Type    |
| :---- | :------ |
| 0     | `table` |
