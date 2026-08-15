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
