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
