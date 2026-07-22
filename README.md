# pliron-inspect

Interactive IR inspector for [pliron](https://github.com/pliron-org/pliron): a web UI for browsing
IR snapshots, control-flow graphs, operation trees, and e-graphs, driven by a small CLI process
that links your compiler's dialects.

## How it fits together

- **`pliron-inspect`** — the server/CLI. Serves the web UI and proxies render requests to a
  driver binary over stdio.
- **`pliron-inspect-driver`** — a library for building that driver binary. Projects link their
  own dialects and passes against it and expose them through `pliron_inspect_driver::run_stdio_driver`.
  The crate also ships a default binary (`pliron-inspect-driver`) that only understands pliron's
  builtin dialect, useful for trying the UI without writing one.
- **`pliron-inspect-protocol`** — the wire types shared between the server, drivers, and trace
  files, so custom drivers and the server always agree on the render-document format.
- **`frontend/`** — the React + TypeScript UI (Vite, Monaco for text, React Flow + Flowblocks for
  CFGs, D3 + d3-flextree for trees, Cytoscape.js for e-graphs).

## Views

| View     | What it shows                          |
|----------|-----------------------------------------|
| Text     | Plain Monaco IR snapshot                |
| IR Diff  | Monaco snapshot diff                    |
| CFG      | Control-flow graph (React Flow + Flowblocks geometry) |
| Tree     | Operation hierarchy (D3 flextree)       |
| EGraph   | Equivalence graph (Cytoscape.js)        |
| Versions | Trace versions for the selected project |

## Building from source

Requires Rust and Node.js (for the frontend).

```sh
# Build the frontend bundle the server falls back to serving from disk in dev.
cd frontend && npm install && npm run build && cd ..

# Build the Rust workspace.
cargo build --workspace
```

## Running

```sh
cargo run -p pliron-inspect
```

This opens a browser at `http://127.0.0.1:3000` by default. Useful flags:

| Flag | Description |
|------|-------------|
| `--port <PORT>` | Port to serve on (default `3000`) |
| `--no-open` | Don't open a browser automatically |
| `--driver <PATH>` | Path to a driver binary for CFG/Tree rendering. Defaults to a `pliron-inspect-driver` binary next to the current executable, if present. |
| `--trace-dir <DIR>` | Mutable directory of imported trace files (`$STAIR_DISPLAY_TRACE_DIR`, then `~/.stair/traces`, then `.stair-traces`) |
| `--temp-trace-dir <DIR>` | Immutable directory where compiler runs write trace files |

Building your own driver: implement `pliron_inspect_driver::DriverHooks` for your dialects/passes
and call `run_stdio_driver`, then point `pliron-inspect --driver` at the resulting binary.

## License

Apache-2.0, see [LICENSE](LICENSE). Built on top of [pliron](https://github.com/pliron-org/pliron),
also Apache-2.0.
