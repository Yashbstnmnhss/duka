---
title: Your first kao
description: Project layout and the dukao commands.
---

A duka project is called a **kao**.

```
- project_name/
    - kao.toml
    - src/
        - *.duka
    - build/
        - *.dukac
    - modules/
        - ...
    - test/
        - *.test.duka
```

`kao.toml` is the manifest, `src/main.duka` is the entry. See
[modules](/language/modules/).

## Init

```sh
dukao init <path> --name --version --force
```

- `<path>` optional, `./` in default
- `--name`, uses directory name in default
- `--version`, `0.1.0` in default
- `--force`, replace an existing kao

## Run

```sh
dukao run <path> --entry --no-color -- <script args>
```

- `<path>` optional, nearest `kao.toml` upward, else `./`
- `--entry` sets the entry file explicitly
- everything after `--` goes to `...` of the script

## Build

```sh
dukao build <path> --list
```

- `--list` only lists the files that would be built

## Test

```sh
dukao test <path> --list --filter <name> --no-color
```

Tests are `.duka` files under `test/`. `tests/` in the repo has examples, run
them with `dukao test ./tests`.

## Dependencies

```sh
dukao add <git url> --tag --branch --as <name>
dukao remove <name>
dukao install --strict
```

`add` writes into `[dependencies]` of `kao.toml`, `install` resolves them into
`kao.lock.toml`, `--strict` follows the lock without updating.

## Serve

```sh
dukao serve <path> --port
```

Serves `build/` on `3000` in default, which is where the wasm target lands.
