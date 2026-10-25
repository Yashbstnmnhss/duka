---
title: Config
description: The switches that change what the language accepts.
---

Duka has a handful of switches. They live in `DukaConfig`
(`shared/src/config.rs`) and a kao hands them over as `build.config` in
`kao.toml`. No section there means the defaults below.

## lexer

- `keep_comment` — keep comments in the token stream, `false`

## parser

- `use_bang_expr` — `keyword!` expressions, `true`
- `use_bang_stmt` — `keyword!` statements, `true`
- `use_stmt_expr` — expression statements, `true`
- `type_annotations` — `: type`, `true`
- `default_nonnilable` — see [nilable](/language/types/#nilable--nonnilable),
  `false`
- `var_default_local` — a bare name is local, `true`

## analyzer

The same three again, checked while types are validated: `var_default_local`,
`type_annotations`, `default_nonnilable`.

## adapter

- `do_inline_adapt` — inline adaptation, `true`

## The REPL

The REPL turns `var_default_local` off, so a bare `function f() end` stays
global across lines. In a file the same line is local.
