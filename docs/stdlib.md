# Standard Library for Duka

See [Builtin](./builtin/README.md)

## Concepts

- Duka is a 0-based language, which means indexes start from 0 instead of 1 (like Lua)
- In most functions, the range parameter is a **left-closed and right-open** interval(`[start, end)`)

### **Generator & Iterator Protocol**:

Iterator is used in `for` loop, and to get a iterator, you need generator.

1. Generator is a function with 3 returns: `(iterator, state, control)`
    - Iterator function (See below)
    - The value to be handled
    - A control value

    Relevant functions are: `pairs`, `ipairs`

2. Iterator is a function with 2 params and dynamic returns: `(state, control) -> (bool, ...)`
    1. Inputs:
    - The target value
    - State value
    2. Returns: `(bool, ...)` values. When it ends, results are `(false, ...?)`, otherwise `(true, ...)`

In default, `ipairs` and `pairs` won't walk deeply to get values from `__index`

### **Result Protocol**:

Results from a function that may raise errors are composed of `(success: bool, ...)`. When it succeed, results are `(true, ...)` [success] (... represents results from `return`), otherwise they will be `(false, error_msg: string)`[failure].

Relevant functions are:

- `assert`: do assertion
- `error`: raise an error with custom message
- `unwrap`: raise an error when get failure
- `is_error`: check if it is a failure

## String

- Strings are arrays of bytes. The unit of index of functions (like `substr`) is based on bytes instead of unicode characters
- Negative index is supported. For example: `-1` represents `len - 1`
- `substr` accepts `(str, start, len)`, while `slice` accepts `(str, from, end)`
