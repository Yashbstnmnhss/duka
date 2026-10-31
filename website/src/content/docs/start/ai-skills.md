---
title: AI skills
description: Give coding agents a verified duka reference.
---

`skills/duka/` in the repository is an [Agent Skill](https://agentskills.io)
for writing duka code: syntax, project layout, the standard library, and the
pitfalls that trip models up. Every example in it was executed against the real
runtime, and the wording follows the implementation where old prose lagged
behind it.

## Install

Copy the skill into the directory your agent reads:

```sh
# Claude Code / OpenCode / compatible agents
cp -r skills/duka ~/.claude/skills/duka          # user-wide
cp -r skills/duka .claude/skills/duka            # per project
```

The source of truth lives in the repository:

- [`skills/duka/SKILL.md`](https://github.com/Yashbstnmnhss/duka/blob/master/skills/duka/SKILL.md)
- [standard library reference](https://github.com/Yashbstnmnhss/duka/blob/master/skills/duka/references/stdlib.md)
- [packages and the `dukao` installer](https://github.com/Yashbstnmnhss/duka/blob/master/skills/duka/references/packages.md)
- [gotchas](https://github.com/Yashbstnmnhss/duka/blob/master/skills/duka/references/gotchas.md)

Raw files, if your agent fetches URLs:

```text
https://raw.githubusercontent.com/Yashbstnmnhss/duka/master/skills/duka/SKILL.md
```

## What it covers

- kao project layout, `kao.toml`, and every `dukao` command
- variables, control flow, functions and closures, tables and arrays
- type annotations, `object`, modules with `require`/`export`
- the `(ok, value)` error protocol and `try`/`unwrap`/`is_error`
- namespaced standard library calls and versioned git dependencies

It is deliberately short: details live in `references/` and are loaded only
when needed, and the authoritative version of each language topic is linked back
to this site.
