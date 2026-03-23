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
For multi-line string, there must be `[(=)*[` and `](=)*]` at the end with the same count of `=`

### Comments

- `--` for single-line comment
- `--[(=)*[` with `](=)]` for multi-line comment

### Boolean
- `true`
- `false`

### Nil Value
`nil`

### Table

Three types of fields are supported:
- name-value `{name = value}`
- array-like value `{value}`
- key-value `{[key] = value}`