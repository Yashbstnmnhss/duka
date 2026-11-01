# ffi


<a id="ffi"></a>

> FFI (Foreign Function Interface)

```lua
ffi.cdef("int abs(int n);")
ffi.load("libc.so.6")
local ok, n = ffi.call("abs", -1)
```

Flags: `@feature(ffi)`

## Contents

[cdef](#cdef)

[refresh](#refresh)

[load](#load)

[close](#close)

[call](#call)

[get](#get)

[set](#set)

## Members

<a id="cdef"></a>

### `cdef(code: string)`

> Declares the C types and functions a call reads, written the way a header writes them

Flags: `@returns(result)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `code` | `string` | *false* | *false* | *required* | - |

<a id="refresh"></a>

### `refresh()`

> Looks the declared names up again, after a library was opened

Flags: `@returns(result)`

<a id="load"></a>

### `load(path: string)`

> Opens a shared library and returns the id a call can name it by

Flags: `@returns(result)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `path` | `string` | *false* | *false* | *required* | - |

<a id="close"></a>

### `close(id: int)`

> Closes the library an id names, answering whether it was open

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `id` | `int` | *false* | *false* | *required* | - |

<a id="call"></a>

### `call(name: string, ...args: any)`

> Calls a declared C function with the given arguments

Flags: `@returns(result)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `name` | `string` | *false* | *false* | *required* | - |
| `...args` | `any` | *true* | *false* | - | - |

<a id="get"></a>

### `get(name: string)`

> Reads a declared C variable

Flags: `@returns(result)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `name` | `string` | *false* | *false* | *required* | - |

<a id="set"></a>

### `set(name: string, value: any)`

> Writes a declared C variable

Flags: `@returns(result)`

#### Params

| Name | Type | VarArg? | Optional? | Default | Doc |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `name` | `string` | *false* | *false* | *required* | - |
| `value` | `any` | *false* | *false* | *required* | - |

