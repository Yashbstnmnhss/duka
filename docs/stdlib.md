# Standard Library for Duka

See [References](./references/index.md)

## Concepts

- Duka is a 0-based language, which means indexes start from 0 instead of 1 (like Lua)
- In most functions, the range parameter is a **left-closed and right-open** interval(`[start, end)`)

### **Generator & Iterator Protocol**:

Iterator is used in `for` loop & `iter` lib.
To get an iterator, you need a generator to generate one.

1. Generator is a function with one return: `iterator`
    - It accepts custom inputs
    - And returns an iterator function (See below)

    Relevant functions are: `pairs`, `ipairs`, `iter.range`

2. Iterator is a function with 0 params and dynamic returns: `() -> (bool, ...)`
   it returns `(bool, ...)` values. When it ends, results are `(false, ...?)`, otherwise `(true, ...)`

In default, `ipairs` and `pairs` won't walk deeply to get values from `__index`

### **Result Protocol**:

Results from a function that may raise errors are composed of `(success: bool, ...)`. When it succeeds, results are `(true, ...)` [success] (... represents results from `return`), otherwise they will be `(false, error_msg: string)`[failure].

Relevant functions are:

- `assert`: do assertion
- `error`: raise an error with custom message
- `unwrap`: raise an error when get failure
- `is_error`: check if it is a failure

## String

- Strings are arrays of bytes. The unit of index of functions (like `substr`) is based on Unicode characters
- Negative index is supported. For example: `-1` represents `len - 1`
- `substr` accepts `(str, start, len)`, while `slice` accepts `(str, from, end)`

## Iter

```lua
local { map, filter, to_array } = iter
local nums = [1, 2, 3, 4, 5, 6]
local out = nums
    |> map(fn(x) x * 10)
    |> filter(fn(x) x > 30)
    |> to_array()
return out[0] + out[1] + out[2]
```
