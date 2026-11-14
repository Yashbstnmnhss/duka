# LSP for Duka

`duka-lsp` speaks the Language Server Protocol over stdio. It is the server the
[VS Code extension](../extension/README.md) starts, and any other LSP client can
speak to it directly.

## Features

- Diagnostics, with a short debounce while typing and errors mapped to the
  module file they came from. `@deprecated` uses and discarded `@must_use`
  results are reported as warnings with quick fixes.
- Hover: signatures, types, documentation, generic bindings, class members,
  labels and `goto` targets.
- Go to definition and go to type definition, including members reached through
  `require` and the generated declaration files of the standard library.
- References and rename across every file of the project the document belongs
  to, with open buffers winning over the copy on disk.
- Workspace symbols (`Ctrl+T` / `Cmd+T`), document symbols with object members
  nested under their class.
- Completion for values, types, members, `require` module names and the
  standard library, resolved lazily so the list stays small.
- Signature help, inlay hints, document highlights, folding ranges, selection
  ranges, document links for `require` strings, semantic tokens (full and range).
- Project awareness: a document under a `kao.toml` inherits the manifest's
  edition, features and strictness, and resolves modules from the project's
  modules directory. A document with no manifest is edited on its own, with
  cross-file features limited to what its `require`s reach.

## Generated declaration files

A builtin is written in Rust, so there is no file for the editor to open. The
server generates one per module and serves it under the `duka-virtual:` scheme;
the VS Code extension registers a content provider that asks
`duka/declaredFile` for the text. Go to definition on `os.exit` or `print`
lands in that file, and hover, completion and goto keep working inside it.

## Project layout

- `src/backend.rs` — the protocol surface: request handlers, diagnostics
  publication, completion, navigation, cross-file searches.
- `src/workspace.rs` — open documents, analysis snapshots and the kao project
  model; analysis runs outside the workspace lock and an edit supersedes a run
  that is still in flight.
- `src/compile.rs` — the pipeline exactly as the compiler runs it, including
  the module provider and the project configuration.
- `src/convert.rs` — compiler types to LSP types.
- `src/docs.rs`, `src/hover.rs` — hover cards and standard library docs.
- `src/declared.rs` — the generated declaration files.
- `src/roles.rs` — syntactic roles used by semantic highlighting.

## Development

```sh
cargo build -p duka-lsp
cargo test -p duka-lsp
```

The extension looks for `duka-lsp` in `target/debug`, `target/release` above
each workspace folder, then in `PATH`. `duka.lsp.path` overrides the search.
