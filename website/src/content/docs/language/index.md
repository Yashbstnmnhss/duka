---
title: Language Reference
description: What is in this section.
---

Duka has lua's grammar, an annotation type system and a register VM. Everything
is 0-based.

- [Lexical](lexical/) — tokens, literals, comments
- [Statements](statements/) — declarations, blocks, control flow
- [Expressions](expressions/) — literals, pipeline, operators
- [Functions](functions/) — `function`, `fn`, vararg
- [Types](types/) — annotations, nilable, generic, `where`
- [Type context](type-context/) — `type`, type functions, HKT
- [Objects](objects/) — `object`, `extends`, metamethods
- [Match](match/) — pattern matching
- [Macros & bang](macros/) — `^#define`, splicer, `linq!`, `logic!`
- [Modules](modules/) — `require`, `export`, `RequireType`
- [Coroutines](coroutines/) — `yield`, `spawn`, `go`
- [Errors](errors/) — the result protocol
- [Keywords](keywords/) — every keyword, one entry each
- [Attributes](attributes/) — `@declare`, `@const`, ...
- [Config](config/) — the switches behind `kao.toml`

Standard library lives in [Standard library](../reference/), guides in
[Guides](../guides/migrate-from-lua/).
