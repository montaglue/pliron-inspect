# pliron-inspect commit-series partition plan

Base: HEAD `9d639d7` ("pin pliron to diffirent commit"). Working tree: 10
modified files + 6 untracked paths (the LSP crates and the vscode extension
were never committed — they predate this session but exist only on disk).
Standard: one topic per commit, each lane's staged state builds and passes
its own tests; user commits with the drafted message; Claude stages.

**Cross-repo constraint (state it, don't fight it):** crabbit's committed
HEAD (`fa499c3`) path-deps on `pliron-inspect-{protocol,driver}` and calls
the full server API (`AttributionOps`, `run_costs` wire types,
`run_server_stdio`). crabbit builds only from L3 onward. L1–L2 verify at
this repo's own scope — the same caveat crabbit's plan recorded for its
C1–C2 against this repo's siblings.

Lockfile rule: regenerate per lane (`cargo build --workspace`) and stage
the result; don't hunk-split Cargo.lock.

---

### L1 — deps: move to crates.io pliron 0.17

Files: `Cargo.toml` (the two pliron pin lines ONLY — the
`pliron-inspect-lsp` workspace-dep line belongs to L2; hunk-split),
`Cargo.lock` (regenerated).
Verify: `cargo build --workspace && cargo test --workspace` (the tracked
crates only — lsp crates are still untracked at this point and
`members = ["crates/*"]` picks up untracked dirs, so run verification in an
exported-index worktree, which contains only tracked+staged files).

Message:
```
deps: crates.io pliron 0.17.0

Off the git pin: the 0.17 release carries everything the previous
rev was pinned for, and every sibling workspace (crabbit, the
research repos) resolves the same crates.io version — one pliron
per build graph.
```

### L2 — LSP: embeddable server library, reference binary, vscode client

Files (untracked, whole): `crates/lsp/` (lib: lex, lineindex, analysis,
navigation, server, README), `crates/lsp-server/` (reference bundle
binary), `editors/vscode/` — SOURCE ONLY: add `editors/vscode/.gitignore`
with `node_modules/` and `out/` before staging (build products must not
land; mirrors crabbit's C1). Plus the `pliron-inspect-lsp` workspace-dep
line in root `Cargo.toml` (the L1 leftover hunk), `Cargo.lock`.
Verify: `cargo build -p pliron-inspect-lsp -p pliron-inspect-lsp-server`.

Message:
```
lsp: embeddable .plir language server + reference bundle + vscode client

Dialect bundles, not dynamic dialects: pliron dialects register at
link time, so the LSP is a library each project compiles with its
own dialect crates; the reference binary bundles builtin + llvm.
Lexer-based fallback plus parse-backed analysis (diagnostics,
hover, definition, references) over in-memory documents — the
same DocumentAnalysis the resident-server work reuses. Known
gaps, found by a verified session audit: references from a def
position return empty; op-name hover needs a parsed module.
```

### L3 — resident server: protocol extension, driver engine, language + cost commands

Files: `crates/protocol/src/server.rs` (untracked, whole: 8 base commands +
`ir_hover`/`ir_definition`/`ir_references`/`ir_diagnostics` +
`run_costs`/`CostEntry`/`FunctionCosts` wire types),
`crates/protocol/src/lib.rs` (the `pub mod server;` line),
`crates/driver/src/server.rs` (untracked, whole: Server engine, worker
pool, HTTP shim, DocumentAnalysis cache, run_costs handler),
`crates/driver/src/harness.rs` (whole diff: `PipelineEvent`,
`AttributionOps`, five default-implemented DriverHooks methods,
`parse_ir` made pub), `crates/driver/src/lib.rs` (whole diff: module +
re-exports), `crates/driver/Cargo.toml` (deps: `pliron-inspect-lsp`,
`lsp-types`), `Cargo.lock`.
One lane, not three: protocol/engine/language/costs were built as one
piece, live in two cohesive new files, and crabbit's committed HEAD needs
the whole surface — fabricating per-feature intermediates of untracked
files buys review granularity crabbit's own C6 (which consumed all of it
at once) does not benefit from.
Verify: `cargo build --workspace && cargo test --workspace` in the
exported tree, then cross-repo: `cd ~/projects/montaglue/crabbit && cargo
build -p crabbit-inspect-driver` (first lane where this must succeed).

Message:
```
server: resident driver protocol — runs, replayed IR, language, costs

DriverHooks grows default-implemented server methods (targets,
pipeline names, observed runs, artifacts, attribution ops) so
existing drivers are untouched; run_server_stdio adds a worker
pool and a loopback HTTP shim speaking the same line-JSON
protocol. Commands: module load (parse-verified), runs with
per-pass progress and cancellation, IR at any pass by replay or
capture, artifact + sidecar fetch, position→op hover/definition/
references/diagnostics answered from the live analysis, and
run_costs — the backward cost lift joining measured per-op costs
(profile_ingest/ncu_ingest shape) to source-boundary ops with
per-root semantic accounting.
```

### L4 — UI: connect to a running analysis server

Files: `crates/inspect/src/main.rs` (whole diff: `--server`,
`--frontend-dir`), `crates/inspect/src/server.rs` (whole diff: AppState
fields, `/api/analysis` generic proxy + health, frontend-dir resolution),
`crates/inspect/src/static/index.html` (whole diff: module upload, run
launcher with config axes, live progress, per-pass IR with click-hover /
d-definition / r-references, artifact download — vanilla JS on the
compiled-in page; there is no npm frontend project).
Verify: `cargo build -p pliron-inspect` + `cargo test --workspace`.

Message:
```
ui: analysis-server panels — runs, per-pass IR, language features

--server connects the UI to a running analysisd; the whole
analysis API is one generic proxy route (the frontend speaks the
same wire commands as curl). Panels: module upload, run launcher
over the engine/config axes, polled live progress, per-pass IR
views with position→op hover, definition and references, artifact
download. --frontend-dir unhardcodes the asset path (compiled-in
page remains the fallback).
```

### L5 — e2e test + quickstart docs

Files: `tools/e2e_analysis_ui.py` (untracked), `README.md` (whole diff:
QUICKSTART section).
Verify: `python3 -m py_compile tools/e2e_analysis_ui.py`; the full e2e
needs a built crabbit-analysisd — run it once manually
(`python3 tools/e2e_analysis_ui.py`, 24 checks) and say so in the commit.

Message:
```
e2e: drive analysisd + UI end to end; README quickstart

Scripted session against a live crabbit-analysisd: health,
module load, runs under both engines, progress, captured and
replayed IR, typed hover/definition/references, ELF-validity of
artifacts, then the same flow through the UI's own routes.
README gains the two-process quickstart.
```

---

**Push order:** this repo pushes BEFORE crabbit (crabbit's relocated git
deps will pin a rev of this repo that must contain L3; push all five lanes
together). After push, delete this COMMIT-PLAN.md.
**Pre-push checks:** exported-tree `cargo build --workspace && cargo test
--workspace` green at L5; crabbit `cargo build -p crabbit-inspect-driver`
green against this HEAD; `git status` clean; no `node_modules`/`out` in
`git ls-files editors/`.
