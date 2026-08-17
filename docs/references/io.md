# io

[Index](index.md)

<blockquote>
File and stream I/O
</blockquote>

## Example

```lua
local ok, f = io.open("a.txt", "w")
if ok then 
    f:write("hello") 
end
```

## Flags
@feature(platform)

## Contents

- [open](#open)
- [tmpfile](#tmpfile)
- [type](#type)
- [IOOut](#ioout)
- [IOIn](#ioin)
- [File](#file)
- [stdout](#stdout)
- [stderr](#stderr)
- [stdin](#stdin)

## Members

<a id="open"></a>
### `open(path: string, mode: string = "r".to_owned()) -> ...`

<blockquote>
Opens the file `path` with `mode` ("r", "w", "a", "r+", "w+", "a+", optionally with a "b" suffix)
</blockquote>

## Flags
@returns(result)

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `path` | string | *false* | *false* | *required* | - |
| `mode` | string | *false* | *true* | `"r".to_owned()` | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="tmpfile"></a>
### `tmpfile() -> ...`

<blockquote>
Creates and opens a unique temporary file for reading and writing
</blockquote>

## Flags
@returns(result)

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="type"></a>
### `type(v: any) -> any`

<blockquote>
Returns "file" if `v` is an open file handle, "closed file" if it is closed, otherwise nil
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `v` | any | *false* | *false* | *required* | - |

## Returns

`any`



| Index | Type |
| :--- | :---: |
| 0 | any |

<a id="ioout"></a>
### UserData `IOOut`

## Methods

<a id="ioout-write"></a>
#### `write(self: any, ...vals: any) -> ...`

<blockquote>
Write content to this stream; nil is written as "nil". Returns [true, count] on success, [false, msg] on error
</blockquote>

## Flags
@returns(result)

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | IOOut |
| `...vals` | any | *true* | *false* | - | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="ioout-flush"></a>
#### `flush(self: any) -> ...`

<blockquote>
Flush content to this stream
</blockquote>

## Flags
@returns(result)

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | IOOut |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="ioin"></a>
### UserData `IOIn`

## Methods

<a id="ioin-read"></a>
#### `read(self: any, ...what: any) -> ...`

<blockquote>
Reads from standard input. With no argument reads one line; with an integer reads that many bytes; with a string uses a format: "a" reads all, "l"/"L" reads a line, "n" reads a number. Returns [true, data] on success, [true, nil] at end of input, [false, msg] on error
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | IOIn |
| `...what` | any | *true* | *false* | - | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="ioin-lines"></a>
#### `lines(self: any) -> ...`

<blockquote>
Returns an iterator that yields one line from standard input per iteration
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | IOIn |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="file"></a>
### UserData `File`

<blockquote>
An open file handle
</blockquote>

## Methods

<a id="file-close"></a>
#### `close(self: any) -> ...`

<blockquote>
Closes the file
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | FileData |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="file-is_open"></a>
#### `is_open(self: any) -> bool`

<blockquote>
Returns whether the file handle is still open
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | FileData |

## Returns

`bool`



| Index | Type |
| :--- | :---: |
| 0 | bool |

<a id="file-read"></a>
#### `read(self: any, ...what: any) -> ...`

<blockquote>
Reads from the file. With no argument reads one line; with an integer reads that many bytes; with a string uses a format: "a" reads all, "l"/"L" reads a line, "n" reads a number. Returns [true, data] on success, [true, nil] at end of file, [false, msg] on error
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | FileData |
| `...what` | any | *true* | *false* | - | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="file-write"></a>
#### `write(self: any, ...data: any) -> ...`

<blockquote>
Writes each argument as a string to the file; nil is written as "nil". Returns [true, count] on success, [false, msg] on error
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | FileData |
| `...data` | any | *true* | *false* | - | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="file-seek"></a>
#### `seek(self: any, whence: string = "cur".to_owned(), offset: int = 0) -> ...`

<blockquote>
Sets and gets the file position; `whence` is "set", "cur" or "end". Returns [true, pos] on success, [false, msg] on error
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | FileData |
| `whence` | string | *false* | *true* | `"cur".to_owned()` | - |
| `offset` | int | *false* | *true* | `0` | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="file-flush"></a>
#### `flush(self: any) -> ...`

<blockquote>
Flushes any buffered data to the file
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | FileData |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="file-lines"></a>
#### `lines(self: any) -> ...`

<blockquote>
Returns an iterator that yields one line per iteration
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `self` | any | *false* | *false* | *required* | FileData |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

## Example

```lua
local f = io.open("a.txt")
```

<a id="stdout"></a>
### Static `stdout`(IOOut)

<blockquote>
Standard stream for output
</blockquote>

See [here](#ioout)

<a id="stderr"></a>
### Static `stderr`(IOOut)

<blockquote>
Standard stream for error output
</blockquote>

See [here](#ioout)

<a id="stdin"></a>
### Static `stdin`(IOIn)

<blockquote>
Standard stream for input
</blockquote>

See [here](#ioin)
