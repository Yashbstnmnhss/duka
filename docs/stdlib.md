# Standard Library for Duka

## Concepts

- Duka is a 0-based language, which means indexes start from 0 instead of 1 (like Lua)
- In most functions, the range parameter is a **left-closed and right-open** interval(`[start, end)`)

## Core

### Require

## Table

## String

- Strings are arrays of bytes. The unit of index of functions (like `substr`) is based on bytes instead of unicode characters
- Negative index is supported. For example: `-1` represents `len - 1`
- `substr` accepts `(str, start, len)`, while `slice` accepts `(str, from, end)`

## Math
