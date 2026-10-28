---
title: io
description: File and stream I/O.
---

<a id="io"></a>

> File and stream I/O

```lua
local ok, f = io.open("a.txt", "w")
if ok then
    f:write("hello")
end
```

Flags: `@feature(platform)`

## Contents

[open](#open)

[tmpfile](#tmpfile)

[type](#type)

[IOOut](#ioout)

[IOIn](#ioin)

[File](#file)

[stdout](#stdout)

[stderr](#stderr)

[stdin](#stdin)

## Members

<a id="open"></a>

### `open(path: string, mode: string = "r".to_owned()) -> ...`

> Opens the file `path` with `mode` ("r", "w", "a", "r+", "w+", "a+", optionally with a "b" suffix)

Flags: `@returns(result)`

#### Params

| Name   | Type     | VarArg? | Optional? | Default          | Doc |
| :----- | :------- | :------ | :-------- | :--------------- | :-- |
| `path` | `string` | _false_ | _false_   | _required_       | -   |
| `mode` | `string` | _false_ | _true_    | `"r".to_owned()` | -   |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="tmpfile"></a>

### `tmpfile() -> ...`

> Creates and opens a unique temporary file for reading and writing

Flags: `@returns(result)`

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="type"></a>

### `type(v: any) -> any`

> Returns "file" if `v` is an open file handle, "closed file" if it is closed, otherwise nil

#### Params

| Name | Type  | VarArg? | Optional? | Default    | Doc |
| :--- | :---- | :------ | :-------- | :--------- | :-- |
| `v`  | `any` | _false_ | _false_   | _required_ | -   |

#### Returns

`any`<br/>

| Index | Type  |
| :---- | :---- |
| 0     | `any` |

<a id="ioout"></a>

### UserData `IOOut:IOOut`

#### Methods

<a id="write"></a>

#### `write(self: any, ...vals: any) -> ...`

> Write content to this stream; nil is written as "nil". Returns [true, count] on success, [false, msg] on error

Flags: `@returns(result)`

#### Params

| Name      | Type  | VarArg? | Optional? | Default    | Doc   |
| :-------- | :---- | :------ | :-------- | :--------- | :---- |
| `self`    | `any` | _false_ | _false_   | _required_ | IOOut |
| `...vals` | `any` | _true_  | _false_   | -          | -     |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="flush"></a>

#### `flush(self: any) -> ...`

> Flush content to this stream

Flags: `@returns(result)`

#### Params

| Name   | Type  | VarArg? | Optional? | Default    | Doc   |
| :----- | :---- | :------ | :-------- | :--------- | :---- |
| `self` | `any` | _false_ | _false_   | _required_ | IOOut |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="ioin"></a>

### UserData `IOIn:IOIn`

#### Methods

<a id="read"></a>

#### `read(self: any, ...what: any) -> ...`

> Reads from standard input. With no argument reads one line; with an integer reads that many bytes; with a string uses a format: "a" reads all, "l"/"L" reads a line, "n" reads a number. Returns [true, data] on success, [true, nil] at end of input, [false, msg] on error

Flags: `@returns(result)`

#### Params

| Name      | Type  | VarArg? | Optional? | Default    | Doc  |
| :-------- | :---- | :------ | :-------- | :--------- | :--- |
| `self`    | `any` | _false_ | _false_   | _required_ | IOIn |
| `...what` | `any` | _true_  | _false_   | -          | -    |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="lines"></a>

#### `lines(self: any) -> ...`

> Returns an iterator that yields one line from standard input per iteration

Flags: `@returns(result)`

#### Params

| Name   | Type  | VarArg? | Optional? | Default    | Doc  |
| :----- | :---- | :------ | :-------- | :--------- | :--- |
| `self` | `any` | _false_ | _false_   | _required_ | IOIn |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="file"></a>

### UserData `File:File`

> An open file handle

#### Methods

<a id="close"></a>

#### `close(self: any) -> ...`

> Closes the file

Flags: `@returns(result)`

#### Params

| Name   | Type  | VarArg? | Optional? | Default    | Doc      |
| :----- | :---- | :------ | :-------- | :--------- | :------- |
| `self` | `any` | _false_ | _false_   | _required_ | FileData |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="is_open"></a>

#### `is_open(self: any) -> bool`

> Returns whether the file handle is still open

#### Params

| Name   | Type  | VarArg? | Optional? | Default    | Doc      |
| :----- | :---- | :------ | :-------- | :--------- | :------- |
| `self` | `any` | _false_ | _false_   | _required_ | FileData |

#### Returns

`bool`<br/>

| Index | Type   |
| :---- | :----- |
| 0     | `bool` |

<a id="read"></a>

#### `read(self: any, ...what: any) -> ...`

> Reads from the file. With no argument reads one line; with an integer reads that many bytes; with a string uses a format: "a" reads all, "l"/"L" reads a line, "n" reads a number. Returns [true, data] on success, [true, nil] at end of file, [false, msg] on error

Flags: `@returns(result)`

#### Params

| Name      | Type  | VarArg? | Optional? | Default    | Doc      |
| :-------- | :---- | :------ | :-------- | :--------- | :------- |
| `self`    | `any` | _false_ | _false_   | _required_ | FileData |
| `...what` | `any` | _true_  | _false_   | -          | -        |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="write"></a>

#### `write(self: any, ...data: any) -> ...`

> Writes each argument as a string to the file; nil is written as "nil". Returns [true, count] on success, [false, msg] on error

Flags: `@returns(result)`

#### Params

| Name      | Type  | VarArg? | Optional? | Default    | Doc      |
| :-------- | :---- | :------ | :-------- | :--------- | :------- |
| `self`    | `any` | _false_ | _false_   | _required_ | FileData |
| `...data` | `any` | _true_  | _false_   | -          | -        |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="seek"></a>

#### `seek(self: any, whence: string = "cur", offset: int = 0) -> ...`

> Sets and gets the file position; `whence` is "set", "cur" or "end". Returns [true, pos] on success, [false, msg] on error

Flags: `@returns(result)`

#### Params

| Name     | Type     | VarArg? | Optional? | Default    | Doc      |
| :------- | :------- | :------ | :-------- | :--------- | :------- |
| `self`   | `any`    | _false_ | _false_   | _required_ | FileData |
| `whence` | `string` | _false_ | _true_    | `"cur"`    | -        |
| `offset` | `int`    | _false_ | _true_    | `0`        | -        |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="flush"></a>

#### `flush(self: any) -> ...`

> Flushes any buffered data to the file

Flags: `@returns(result)`

#### Params

| Name   | Type  | VarArg? | Optional? | Default    | Doc      |
| :----- | :---- | :------ | :-------- | :--------- | :------- |
| `self` | `any` | _false_ | _false_   | _required_ | FileData |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

<a id="lines"></a>

#### `lines(self: any) -> ...`

> Returns an iterator that yields one line per iteration

Flags: `@returns(iterator)`

#### Params

| Name   | Type  | VarArg? | Optional? | Default    | Doc      |
| :----- | :---- | :------ | :-------- | :--------- | :------- |
| `self` | `any` | _false_ | _false_   | _required_ | FileData |

#### Returns

`...`<br/>

| Index | Type  |
| :---- | :---- |
| -     | `...` |

#### Example

```lua
local f = io.open("a.txt")
```

<a id="stdout"></a>

### Static `stdout`(IOOut)

> Standard stream for output

[See here](#ioout)

<a id="stderr"></a>

### Static `stderr`(IOOut)

> Standard stream for error output

[See here](#ioout)

<a id="stdin"></a>

### Static `stdin`(IOIn)

> Standard stream for input

[See here](#ioin)
