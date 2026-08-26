<div align=center>
<img src="./logo.svg" width="100" height="100" alt="logo">

# DUKAO

</div>

See [duka-cli](../cli)

## The `kao`

For duka and dukao, a project is called `kao`.
Normally, a kao project looks like:

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

## Init

To create new kao, you can use `init`

```sh
dukao init <path> --name --version --force
```

- `<path>` optional, `./` in default
- `--name`, uses directory name in default
- `--version`, `0.1.0` in default
- `--force`, if kao already exists, set `--force` to replace it with new

## Run

Run current kao project (from entry file)

```sh
dukao run <path> --entry --no_color --(script_args)
```

- `<path>` optional, current directory in default, used to find `kao.toml`
- `--entry` sets entry file explicitly
- `--no_color` disables the colored output
- `--(script_args)` passes args to `...` var arg in main

## Build

Dukao can build a kao project

```sh
dukao build <path> --list
```

- `<path>` optional
- `--list` will list all source files without building

## Test

Dukao can run tests (per file) under a given directory

```sh
dukao test <path> --no-color --list --filter
```

- `<path>` optional
- `--list` will only list available tests without running them
- `--filter` will skip testing files in this list
- `--no-color` disables the colored output

## Resources

`build` command needs some resources to build, you can find them in `./res` folder. See [here](./res/README.md)

- wasm target: needs `duka-backend-wasm.wasm` & `duka-glue.js`
- executable binary target: needs `duka-app.exe`

To update them after you build crate `backend-wasm`, `app` and so on, you can use `update-shell.cmd`
