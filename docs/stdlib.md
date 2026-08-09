# Standard Library for Duka

See [Builtin](./builtin/README.md)

## Concepts

- Duka is a 0-based language, which means indexes start from 0 instead of 1 (like Lua)
- In most functions, the range parameter is a **left-closed and right-open** interval(`[start, end)`)

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
