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
| `--trace-dir <DIR>` | Mutable directory of imported trace files (`$CRABBIT_DISPLAY_TRACE_DIR`, then `~/.crabbit/traces`, then `.crabbit-traces`) |
| `--temp-trace-dir <DIR>` | Immutable directory where compiler runs write trace files |

Building your own driver: implement `pliron_inspect_driver::DriverHooks` for your dialects/passes
and call `run_stdio_driver`, then point `pliron-inspect --driver` at the resulting binary.

## License

Apache-2.0, see [LICENSE](LICENSE). Built on top of [pliron](https://github.com/pliron-org/pliron),
also Apache-2.0.


## Quickstart: analysis server + UI (crabbit)

```sh
# 1. Build (crabbit repo): the analysis server; (this repo): the UI.
cd ~/projects/montaglue/crabbit && cargo build -p crabbit-inspect-driver
cd ~/projects/montaglue/pliron-inspect && cargo build -p pliron-inspect

# 2. Get IR: any crabbit compile with CRABBIT_EMIT_IR=<dir> writes
#    <crate>-crabbit_rust.plir next to nothing else you need.

# 3. Start the analysis server (resident compiler):
~/projects/montaglue/crabbit/target/debug/crabbit-analysisd     --workers 4 --http 127.0.0.1:8177

# 4. Start the UI pointed at it (opens the browser):
target/debug/pliron-inspect --port 3000 --server 127.0.0.1:8177
```

In the browser: paste or file-pick the `.plir` → **Load module** → choose
target/config axes → **Start run** → watch per-pass progress → **inspect**
on the run row → pick a pass → **Show IR**. In the IR view, click a token
for hover info, then press **d** (go to definition) or **r** (highlight
references); **Diagnostics** shows parse/verify state; **artifact**
downloads the object plus sidecars. `--frontend-dir DIR` serves a built
frontend from DIR instead of the checkout default.

The end-to-end test for all of the above:
`python3 tools/e2e_analysis_ui.py <crabbit-analysisd> <pliron-inspect> <module.plir>`.
