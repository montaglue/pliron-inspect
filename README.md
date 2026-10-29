# pliron-inspect

An inspector for [pliron](https://github.com/pliron-org/pliron) compiler traces: a web UI that
shows what each pass of a compilation did to the IR — as text, as a diff, as a control-flow graph
and as an operation tree — with hover, definitions, references and diagnostics from your own
dialects.

## How it fits together

- **Two folders**, chosen on the start page. The *dialects folder* is the cargo workspace whose
  crates define your pliron dialects. The *traces folder* is where your compiler writes its traces.
- **[pliron-lsp](https://github.com/montaglue/pliron-lsp)** runs on the dialects folder and builds
  a dialect engine from its crates, with no code in your project. The engine parses each IR
  snapshot with your dialects' own parsers and verifiers. pliron-inspect draws the CFG and tree
  views from the engine's model of the IR, and passes its hover, definition, references and
  diagnostics through to the text view (definitions open the dialect's Rust source).
- **`pliron-inspect`** — the server: serves the UI, lists and opens traces, and talks to
  pliron-lsp. It does not link pliron itself, so it works with any dialects pliron-lsp supports.
- **`pliron-inspect-protocol`** — the trace file format and the documents the server sends the UI.
- **`frontend/`** — the React + TypeScript UI (Vite, Monaco for text, React Flow + Flowblocks for
  CFGs, D3 + d3-flextree for trees).

## Traces

A trace (`.crt`) records one compilation: a JSON header (`name`, `kind`, `pipeline`, `target`, …)
followed by the IR before the first pass and after each pass, each part starting with a
`--- crabbit <label> ---` line. The traces folder holds one subfolder per project, with one trace
per compilation in it. The UI can also import a `.crt` file into the traces folder.

## Views

| View     | What it shows |
|----------|---------------|
| Text     | An IR snapshot, with hover, go to definition (F12), references (⇧F12) and the engine's parse and verifier diagnostics |
| IR Diff  | One snapshot against the previous one |
| CFG      | Control-flow graph of a function, nested regions drawn inside their blocks |
| Tree     | Operation / region / block hierarchy |
| Versions | The traces of the selected project |

The pipeline list in the sidebar jumps to the IR after a pass.

## Building from source

Requires Rust and Node.js (for the frontend).

```sh
# The UI (the server serves it from frontend/dist).
cd frontend && npm ci && npm run build && cd ..

cargo build -p pliron-inspect

# The language server, from its own repository (once).
cargo install --locked --git https://github.com/montaglue/pliron-lsp pliron-lsp pliron-lsp-engine-ref
```

## Running

```sh
target/debug/pliron-inspect
```

This opens a browser at `http://127.0.0.1:3000`. Choose the dialects folder and the traces folder
on the start page (the browser remembers them), then **Inspect traces**. The folder browser lists
folders under your home directory on the machine the server runs on.

| Flag | Description |
|------|-------------|
| `--port <PORT>` | Port to serve on (default `3000`) |
| `--no-open` | Don't open a browser |
| `--listen-all` | Listen on all interfaces instead of 127.0.0.1 only. There is no authentication. |
| `--frontend-dir <DIR>` | Serve a frontend built elsewhere (default: the checkout's `frontend/dist`) |

pliron-lsp is found through `$PLIRON_LSP`, next to the `pliron-inspect` binary, in
`~/.cargo/bin`, or on the `PATH`.

## License

Apache-2.0, see [LICENSE](LICENSE). Built on top of [pliron](https://github.com/pliron-org/pliron),
also Apache-2.0.
