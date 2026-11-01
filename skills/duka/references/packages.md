# Duka packages (kao)

A duka project is a **kao**. Dependencies are git repositories (or local
directories); there is no central registry. The installer copies a package into
`modules/`, so `require("<name>")` resolves it.

## Manifest

`kao.toml`:

```toml
[kao]
name = "myapp"
version = "0.1.0"

[build]
entry = "main.duka"       # relative to src_dir
out_dir = "build"
src_dir = "src"
modules_dir = "modules"

[dependencies]
# git, pinned to a tag
sol = { git = "https://host/sol.git", tag = "v1.2.0" }
# git, following a branch
dev = { git = "https://host/dev.git", branch = "main" }
# git, resolved via semver tags
lib = { git = "https://host/lib.git", version = "^1.2" }
# local directory, relative to the manifest
util = { path = "../shared/util" }
```

Rules:

- `version` requires `git` and cannot be combined with `tag` or `branch`.
- `tag`/`branch` cannot be combined with each other or with `version`.
- A `path` dependency is resolved relative to the manifest that declares it.

## Commands

```sh
dukao init mylib --name mylib --version 0.1.0
dukao add https://host/lib.git --version "^1.2" --as lib
dukao add ../shared/util --path --as util
dukao install                # install what the lock records, resolving what it misses
dukao install --frozen       # the lock must cover the manifest; no new resolution
dukao install --offline      # cached clones only, never the network
dukao update                 # re-resolve every requirement and rewrite the lock
dukao remove lib
```

`add` rewrites the `[dependencies]` section of the manifest and installs
immediately. `remove` deletes the entry, the installed copy, and the lock
entry.

## Version requirements

Requirements are resolved against the dependency repository's **tags**; the
highest matching tag wins, and the lock records both the tag and its commit.

| Requirement | Means |
| :-- | :-- |
| `1.2.3` | exactly `1.2.3` (npm semantics, not `^`) |
| `1.2` / `1.2.x` | any `1.2.*` |
| `^1.2.3` | `>=1.2.3, <2.0.0` |
| `^0.2.1` | `>=0.2.1, <0.3.0` (0.x locks the minor) |
| `^0.0.3` | exactly `0.0.3` (0.0.x locks the patch) |
| `~1.2.3` | `>=1.2.3, <1.3.0` |
| `~1.2` / `~1` | `>=1.2.0, <1.3.0` / `>=1.0.0, <2.0.0` |
| `>=1.0, <2.0` | intersection; comma or space separates parts |
| `*` | any tag |

Pre-releases only satisfy a requirement that names one: `^1.0.0` never selects
`1.5.0-rc.1`, but `^1.0.0-rc.1` can.

If no tag satisfies the requirement, `install` fails with exit code 2.

## Lock file

`kao.lock.toml` records, per package: source (`git`/`path`), url, tag, branch,
resolved `rev`, `version` requirement, and path. Commit it.

- `dukao install` reuses the recorded `rev`; a requirement or branch only moves
  when you run `dukao update`, which re-resolves everything and rewrites the lock.
- `dukao install --frozen` installs the recorded `rev` and refuses a lock that
  does not cover the manifest (`kao.lock.toml is out of date for ...`).
- Path dependencies are recorded relative to the project, so a lock survives a
  clone on another machine.
- Packages that are no longer in the manifest are pruned from `modules/`; with
  no dependencies left, the lock file and the installed copies are removed.

## Writing a package

```text
mylib/
├── kao.toml          # [kao] name = "mylib", version = "1.2.0"
├── src/
│   └── main.duka     # default entry: export the public API
└── tests/
    └── api.duka
```

```duka
-- src/main.duka
export function greet(name: string): string
    return "hello, " .. name
end
```

Consumers call `require("mylib")`: package-name resolution reads the package's
own `kao.toml` and loads its declared `[build] entry` (default
`src/main.duka`). An `init.duka` at the package root works too. A nested file
is reachable by path-style name: `require("mylib.util")` maps to
`modules/mylib/util.duka`.

Release = `git tag v1.2.0` + push. The tag is what `version = "^1.2"` resolves
against.

## Installing is recursive

A dependency's own `[dependencies]` are installed too: the tree is walked and
each package lands in the top-level `modules/` directory, so relative package
imports still work from any dependency.

## Limits

- No registry and no `dukao publish`; distribution is git only.
- The lock pins commits, not file contents: a force-pushed tag can change what
  `dukao update` fetches.
- `version` applies only to git dependencies (there are no registry versions).
