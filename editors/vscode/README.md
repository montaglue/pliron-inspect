# pliron IR for VS Code

Language support for pliron textual IR (`.plir`, language id `plir`):
syntax highlighting plus an LSP client providing diagnostics, document
symbols, go-to-definition, find-references and hover.

## The server binary IS the dialect bundle

pliron dialects register at link time, so there is no generic server:
every project compiles its own LSP binary that links its dialect crates
(see the `pliron-inspect-lsp` crate README). This extension just launches
whatever binary you point it at:

```jsonc
// settings.json
{
  // Absolute path; ${workspaceFolder} is substituted.
  "pliron.serverPath": "/path/to/myproject/target/debug/myproject-lsp"
}
```

When `pliron.serverPath` is empty, the extension falls back to
`${workspaceFolder}/target/{debug,release}/pliron-inspect-lsp-server`
(the reference builtin+llvm bundle built by this repo) and then to
`pliron-inspect-lsp-server` on `PATH`.

Use the command palette entry **"pliron: Restart LSP Server"** after
rebuilding your server binary; the client also restarts automatically
when `pliron.serverPath` changes.

## Building the extension

Requires node/npm:

```sh
cd editors/vscode
npm install
npm run compile            # tsc -> out/extension.js
```

To try it without packaging: open `editors/vscode` in VS Code and press
F5 ("Run Extension"), or symlink the folder into `~/.vscode/extensions/`.

To package a `.vsix`:

```sh
npx --yes @vscode/vsce package
code --install-extension pliron-plir-0.1.0.vsix
```

## Demo

Build the reference server and open the sample:

```sh
cargo build -p pliron-inspect-lsp-server   # from the repo root
code editors/vscode/sample/demo.plir
```

(with `pliron.serverPath` pointed at
`<repo>/target/debug/pliron-inspect-lsp-server` if the file is opened
outside this workspace).
