---
title: Coming from lua
description: What changes when you move a lua script over.
---

- Index starts at **0**, not 1. Ranges are `[start, end)`.
- A variable is **local** by default, `global` is the only way to declare a
  global one.
- `[]` is an array, `{}` is a table. `{1, 2, 3}` still works and becomes
  `{[0] = 1, [1] = 2, [2] = 3}`.
- `[[...]]` is gone, use `[=[...]=]` for multi-line strings — the lexer can't
  tell `[[1, 2], [3, 4]]` from a string.
- There is no `repeat` / `until`.
- `continue` exists, `xor` exists.
- `function` still works, `fn(x) expr` is the lambda form.
- `object A ... end` replaces the metatable dance.
- `match` replaces long `if/elseif` chains.
- `local { a, b } = require("./m")` destructures on import.
- `|>` pipes into the first argument, `<|` into the last.
- Types are annotations, they are erased: `local a: int = 1`.

More in [Language](/language/).
