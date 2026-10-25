---
title: Coroutines
description: yield, spawn, go.
---

Coroutines are first-class: `yield`, `spawn`, `go` and `resume` are instructions
of the VM, not library calls.

```duka
function b(...): (int, int, ...)
    local d = spawn()
    go(spawn())
    yield(123, 213, 213, 2131)
    return 0, 0
end
```

- `spawn()` creates a coroutine and returns it
- `go(c)` runs another coroutine from here
- `yield(...)` hands control back, `resume` picks it up again

## States

```duka
type CoStatus =
    "ready" |
    "running" |
    "suspended" |
    "dead" |
    "unknown"
```

Ready -> Running -> Suspended -> Dead. A dead coroutine is done, a suspended one
is waiting for a resume.

The scheduler owns the lifecycle: each coroutine has its own stack, frames,
upvalues and module path. Type one with `local c: coroutine`.
