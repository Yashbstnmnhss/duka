---
title: Language Reference
description: What is in this section.
---

Duka has lua's grammar, an annotation type system and a register VM. Everything
is 0-based.

- [Lexical](/language/lexical/) — tokens, literals, comments
- [Statements](/language/statements/) — declarations, blocks, control flow
- [Expressions](/language/expressions/) — literals, pipeline, operators
- [Functions](/language/functions/) — `function`, `fn`, vararg
- [Types](/language/types/) — annotations, nilable, generic, `where`
- [Type context](/language/type-context/) — `type`, type functions, HKT
- [Objects](/language/objects/) — `object`, `extends`, metamethods
- [Match](/language/match/) — pattern matching
- [Macros & bang](/language/macros/) — `^#define`, splicer, `linq!`, `logic!`
- [Modules](/language/modules/) — `require`, `export`, `RequireType`
- [Coroutines](/language/coroutines/) — `yield`, `spawn`, `go`
- [Errors](/language/errors/) — the result protocol
- [Keywords](/language/keywords/) — every keyword, one entry each
- [Attributes](/language/attributes/) — `@declare`, `@const`, ...
- [Config](/language/config/) — the switches behind `kao.toml`

Standard library lives in [Standard library](/reference/), guides in
[Guides](/guides/migrate-from-lua/).
