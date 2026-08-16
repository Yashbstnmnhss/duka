# Project & Modules

## `require`

`require(pattern)` accepts two kinds of pattern:

- relevant path: `./a` `../b`, based on current file's path
- module path: `name`, find it in `modules/` (if in kao) or `DUKA_PATH`

See [here](../lib/src/module.rs)

## Project

Every kao needs an entry file (in default `src/main.duka`),
you can change it in `kao.toml`

See [here](../dukao/README.md)
