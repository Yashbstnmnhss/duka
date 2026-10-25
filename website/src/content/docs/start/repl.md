---
title: REPL & CLI
description: What duka-cli does.
---

`duka-cli` has three commands:

- `repl` (default) — the REPL mode
- `pipeline` — run the compilation pipeline, `--type` picks the output & input
- `doc-gen` — generate built-in documents in markdown

```sh
duka-cli
duka-cli pipeline
duka-cli doc-gen
```

The REPL keeps what you write across lines, so a bare `function f() end` comes
out global there. In a file the same line is local — see
[var_default_local](/language/config/).
