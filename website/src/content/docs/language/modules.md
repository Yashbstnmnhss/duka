---
title: Modules
description: require, export and RequireType.
---

## `require`

`require(pattern)` accepts two kinds of pattern:

- relative path: `./a`, `../b`, based on the current file's path
- module path: `name`, looked up in `modules/` (in a kao) or `DUKA_PATH`

```duka
local m = require("./a")
local { a, b } = require("./a")
```

## `export`

```duka
export local a = 1
export function b() end
```

is the same as

```duka
local _EXPORTS = {}

local a = 1
function b() end

_EXPORTS.a = 1
_EXPORTS.b = b

return _EXPORTS
```

## `@declare export`

A module can hand a promise on instead of a value:

```duka
-- m.duka
@declare export function strlen(s: string): int

-- elsewhere
local m = require("m")
local n: int = m.strlen("s")   -- checked against the signature
local bad = m.strlen()         -- an error: it takes one argument
```

The module itself writes no `export` for it, so the name is `nil` there. The
importer gets the signature and calls it like any function.

## `RequireType`

```duka
local g: RequireType("m").strlen = some_function
local b: RequireType("m").Outer.inner.deep = true
```

Each dot is checked on its own. A name that resolves to a plain `int` is not a
reference and says nothing.

## `export object`

A class crosses a `require` as itself rather than as `any` — see
[Objects](/language/objects/#across-require).

## Project

Every kao needs an entry file (`src/main.duka` in default), change it in
`kao.toml`. See [Your first kao](/start/first-project/).
