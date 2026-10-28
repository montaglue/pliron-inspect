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

This opens a browser at `http://127.0.0.1:3000` by default; the analysis and project
panels are at `http://127.0.0.1:3000/analysis`. Useful flags:

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


## Quickstart: open a project (crabbit)

```sh
# The language server, from its own repository (once).
cargo install --locked --git https://github.com/montaglue/pliron-lsp pliron-lsp pliron-lsp-engine-ref
# The UI.
cargo build -p pliron-inspect
target/debug/pliron-inspect            # opens the browser
```

Open `http://127.0.0.1:3000/analysis` and enter the project root (for crabbit,
its checkout) under **Project → Open project**. From the root the server
derives everything:

- **Binaries.** `[workspace.metadata.pliron-inspect]` in the project's
  `Cargo.toml` names its render driver and resident analysis server
  (`crabbit-inspect-driver`, `crabbit-analysisd`); they are taken from the
  project's `target/`, built there with `cargo build --bin` when missing,
  and the analysis server is started on stdio.
- **Traces, modules, profiles.** `<root>/.crabbit-traces` (or the `traces`
  dirs of the metadata) is the trace library; `.plir` modules found there
  are offered under **emitted modules**, `op_costs.json` profiles under
  **profiles** (the **costs** button of a run lifts one onto the run's IR).
  crabbit writes there with `CRABBIT_TRACE=1` and `CRABBIT_EMIT_IR=1`.
- **Dialect sources.** pliron-lsp is started on the root; it builds a
  dialect engine from the project's own dialect crates (instrumenting the
  project's pliron) and rebuilds it when their sources change. Hover,
  definition, references and diagnostics in the IR view come from it:
  hover shows the dialects' Rust docs, **d** on an op or type name shows
  its Rust definition. While the engine builds, the syntax layer answers
  and the panel says so.

Then: load a module (from the project, a file, or pasted text) → **Load
module** → **Show module IR**, or choose target/config axes → **Start run**
→ **inspect** → pick a pass → **Show IR**. In the IR view, click a token
for hover info, then press **d** (definition) or **r** (references);
**Diagnostics** shows parse/verify state; **artifact** downloads the
object plus sidecars. `--frontend-dir DIR` serves a built frontend from
DIR instead of the checkout default.

Without a project, `--server 127.0.0.1:PORT` connects the panels to a
hand-started analysis server (`crabbit-analysisd --http 127.0.0.1:PORT`),
with the resident server's own (lexical) language queries.

The end-to-end test for all of the above:
`python3 tools/e2e_analysis_ui.py <crabbit-analysisd> <pliron-inspect> <module.plir>`.
