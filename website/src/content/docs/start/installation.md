---
title: Installation
description: Build the duka toolchain from source.
---

Duka is rust, the repo is a cargo workspace.

```sh
git clone https://github.com/Yashbstnmnhss/duka
cd duka
cargo build
```

Two tools come out of it:

- `duka-cli` — compiler & runner, has `repl`, `pipeline`, `doc-gen`
- `dukao` — package manager, builds and tests a kao project

## Resources

`dukao build` needs resources to build a target, they live in `dukao/res`:

- wasm target: `duka-backend-wasm.wasm` & `duka-glue.js`
- executable target: `duka-app.exe`

After building crates `backend-wasm` / `app`, run `scripts/update-shell.cmd` to
refresh them.
