# Grammar of Duka

## Lexical Grammar

### Identifiers

Identifiers can be composed of any available Unicode identifier characters, CJK is supported as well.

### Numbers

There are mainly two types of numbers:

- integers: `123`; `0x123`, `0b10101`, `0o123` for octal, binary, hexadecimal support
- floats: `1.23`, `-2.3e-10`

### Strings

Terminators for single-line string can be `"` or `'`  
For multi-line string, there must be `[(=)+[` and `](=)+]` at the end with the same count of `=` (**NOTICE:** `[[` is not supported anymore, see [here](README.md#array-type))

### Comments

- `--` for single-line comment
- `--[(=)*[` with `](=)]` for multi-line comment

### Boolean

- `true`
- `false`

### Nil Value

`nil`

See [here](type.md#nilable--nonnilable)

### Table

Three types of fields are supported:

- name-value `{name = value}`
- array-like value `{value}`
- key-value `{[key] = value}`

### Array

Begin with `[`, end with `]`. Nested array is supported

### Object

```lua
object <name> (extends <base>)?
    <property> (= <expr>)?

    function :<method>(...) ... end
    function <static_function>(...) ... end
    function __<metamethod>(...) ... end
end
```

### Variables & Assignment

```lua
(global|local) <name-type> (,<name-type>)* (= <expr> (,<expr>)*)?
<name> = <expr>
```

### Function

```lua
(global|local)? function <name>(<param-name-type>*)(: <return-type>)?
    ...
end
```

```lua
fn(<param-name-type>*)(: <return-type>)? <expr>

function(<param-name-type>*)(: <return-type>)?
    ...
end
```

### Type Annotation

See [here](./type.md)

```lua
: <basic-type>
: (function|func|fn)(<type>,*)(-> <type>,*)
```
