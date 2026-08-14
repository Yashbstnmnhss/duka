# os

[Index](index.md)

## Contents

- [execute](#execute)
- [exit](#exit)
- [remove](#remove)
- [rename](#rename)

## Members

<a id="execute"></a>
### execute(cmd: string)

<blockquote>
Run a process with command, depends on platform
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `cmd` | string | *false* | *false* | *required* | - |

<a id="exit"></a>
### exit(code: int = 0)

<blockquote>
Terminates program with exit code (default = 0)
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `code` | int | *false* | *true* | `0` | - |

<a id="remove"></a>
### remove(path: string) -> ...

<blockquote>
Removes a file or an **empty** directory from the filesystem
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `path` | string | *false* | *false* | *required* | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |

<a id="rename"></a>
### rename(path: string, name: string) -> ...

<blockquote>
Renames a file or directory to a new name, replacing the original file if `name` already exists
</blockquote>

## Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :---: | :---: | :---: | :---: | :--- |
| `path` | string | *false* | *false* | *required* | - |
| `name` | string | *false* | *false* | *required* | - |

## Returns

`...`



| Index | Type |
| :--- | :---: |
| - | `...` |
