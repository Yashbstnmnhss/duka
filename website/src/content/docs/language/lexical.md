---
title: Lexical
description: Identifiers, numbers, strings and comments.
---

## Identifiers

Identifiers can be composed of any available Unicode identifier characters, CJK
is supported as well.

## Numbers

There are mainly two types of numbers:

- integers: `123`; `0x123`, `0b10101`, `0o123` for hexadecimal, binary, octal
- floats: `1.23`, `-2.3e-10`

## Strings

Terminators for single-line string can be `"` or `'`.

For multi-line string there must be `[(=)+[` and `](=)+]` with the same count of
`=`.

**Notice**: the `[[...]]` long-string syntax is **not** supported anymore, use
`[=[...]=]` (or any higher level like `[==[ ]==]`). The lexer can't tell whether
`[[1, 2], [3, 4]]` is a string or a nested array.

## Comments

- `--` for single-line comment
- `--[(=)*[` with `](=)]` for multi-line comment

## Boolean & nil

- `true`, `false`
- `nil`

See [Types](/language/types/).
