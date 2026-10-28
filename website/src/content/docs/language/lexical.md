---
title: Lexical
description: Identifiers, numbers, strings and comments.
---

## Identifiers

Identifiers can be composed of any available Unicode identifier characters, CJK
is supported as well.

For values in attributes, identifiers will turn into its literal strings, see [here](../attributes).

Most of keywords cannot be used as an identifier, see [here](../keywords).

## Numbers

There are mainly two types of numbers:

- integers: `123`; `0x123`, `0b10101`, `0o123` for hexadecimal, binary, octal
- floats: `1.23`, `-2.3e-10`, `1f` (with suffix)

## Strings

Terminators for single-line string can be `"` or `'`.

For multi-line string there must be `[(=)+[` and `](=)+]` with the same count of
`=`.

:::caution
The `[[...]]` long-string syntax is **not** supported anymore, use
`[=[...]=]` (or any higher level like `[==[ ]==]`). The lexer can't tell whether
`[[1, 2], [3, 4]]` is a string or a nested array.
:::

## Comments

- `--` for single-line comment
- `--[(=)*[` with `](=)]` for multi-line comment
- `---` for single-line **document** comment

## Boolean & nil

- `true`, `false`
- `nil`

See [Types](../types).

## Lexical macros

Like preprocessor in C/C++, duka supports following macros at lexical time:

- `^#define`:
    - `^#define Name -> Token;` for single token replacement
    - `^#define Name(params) ... ^#enifed` for multiple tokens replacement
- `^#undef`: to undefine defined replacement
- `^#ifdef ... ... ^#fedfi`: check if target has been defined, if not, code wrapped in won't be included in this file.

See [Macros](../macros).
