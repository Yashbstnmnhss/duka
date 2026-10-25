---
title: Errors
description: The result protocol and friends.
---

Functions that may fail return a **result**: `(success: bool, ...)`.

- on success: `(true, ...)` — `...` is whatever the `return` gave
- on failure: `(false, error_msg: string)`

```duka
local ok, value = do_something()
if ok then
    print(value)
else
    print(value)  -- the message
end
```

## Functions

- `assert` — do an assertion
- `error` — raise an error with a custom message
- `unwrap` — raise an error when it gets a failure
- `is_error` — check whether it is a failure

## As a type

```duka
type function Result(...) = [true, ...] | [false, string]
```

is a builtin type function, so a result is a tuple either way.

## Telling the checker

`@returns` says what the return slots stand for:

- `result` — the return values follow the result protocol
- `exit` — nothing comes back, so there is nothing to ask about

```duka
@returns(result)
function might_fail()
```

See [Attributes](/language/attributes/#returns).

## Postconditions

A signature may carry a concept with `ensure`:

```duka
function not_nil<T>(t: T): ensure NonNil(T)
    return t ~= nil
end
```
