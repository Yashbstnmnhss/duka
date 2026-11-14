# Duka Language extension

Language support for Duka, backed by the `duka-lsp` server bundled with the
workspace.

## Features

- Diagnostics as you type, including errors in required modules.
- Hover, go to definition, references, rename, document highlights.
- Completion with documentation for project symbols and the standard library.
- Signature help, inlay hints (inferred types, parameter names, `goto` targets),
  folding, selection ranges, semantic highlighting and document links for
  `require` strings.
- Workspace symbol search and an outline with object members nested under their
  class.
- Files changed outside the editor are picked up automatically, and a document
  under a `kao.toml` is analyzed with the project's own configuration.

## Settings

- `duka.lsp.path` — path to the `duka-lsp` executable. Empty means discover it
  in `target/debug`, `target/release`, the extension directory, or `PATH`.
- `duka.modulePaths` — extra module search templates for `require`, using `?`
  for the module name (for example `C:/duka/libs/?.duka`).

## Development

Build the server first:

```sh
cargo build -p duka-lsp
```

Then run the `启动扩展` launch configuration from the repository root.
