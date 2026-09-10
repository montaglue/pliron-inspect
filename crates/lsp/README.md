# pliron-inspect-lsp

An embeddable LSP server for pliron textual IR (`.plir`).

## Dialect bundles, not dynamic dialects

MLIR's LSP is generic: it can be told about one dialect grammar at a time.
pliron dialects are different — they are Rust code that registers itself
**at link time** (every dialect, op, type and attribute linked into a
binary self-registers via `CONTEXT_REGISTRATIONS` when `Context::new()`
runs). The LSP server therefore "receives" its dialects by being
*compiled* with them.

This crate is a **library**. The binary that links it decides the dialect
bundle by which dialect crates it links. Each project builds its own tiny
LSP binary. The `pliron-inspect-lsp-server` crate in this workspace is the
reference bundle: pliron builtin + `pliron-llvm`.

### Wiring a project-specific bundle

A project like `crabbit` creates its own LSP binary in ~10 lines:

```toml
# crates/crabbit-lsp/Cargo.toml
[package]
name = "crabbit-lsp"
edition = "2024"

[dependencies]
pliron-inspect-lsp = "0.1"
pliron = "0.17"
my-dialects = { path = "../my-dialects" }   # your dialect crates
pliron-llvm = { version = "0.17", default-features = false }
anyhow = "1"
```

```rust
// crates/crabbit-lsp/src/main.rs
// Link the dialects; they self-register on Context::new().
use my_dialects as _;
use pliron_llvm as _;

fn main() -> anyhow::Result<()> {
    pliron_inspect_lsp::run_stdio_server()
}
```

Build it (`cargo build -p crabbit-lsp`) and point the VS Code extension's
`pliron.serverPath` setting at the produced binary
(`/path/to/crabbit/target/debug/crabbit-lsp`). That's it — the editor now
understands exactly the dialects your project links.

## One module per document

Every open document is parsed into its **own fresh pliron `Context`** as a
**single module**. All analysis — diagnostics, go-to-definition,
find-references, hover, document symbols — is strictly intra-module.
There is no workspace indexing, no cross-file resolution, and no
cross-module symbol tables. This deliberately preserves pliron's
one-module compilation philosophy.

## Features

| Feature | Notes |
|---|---|
| Diagnostics | Parse errors with the parser's source position; verifier errors with best-effort ranges (the offending entity's start location). Published on `didOpen`/`didChange`. Parser/verifier panics are caught and reported as diagnostics. |
| Document symbols | Symbol-defining ops (`@name`, e.g. funcs/globals/the module) as a tree; labeled blocks (`^bb`) as children. |
| Go-to-definition | SSA value names, block labels (`^bb`), symbols (`@sym`), within the module. |
| Find references | Same name classes, within the module. |
| Hover | Ops: op id, result names/types, attributes. Values: type + defining op/block and line. Blocks: signature. |

### How navigation spans are obtained

pliron's parsed IR retains a start `Location` per operation and per block,
but **no source spans for individual value uses**. Navigation therefore
uses a token-level lexical index over the document text, *keyed off the
successfully parsed module*:

* a scope tree is derived from `{`/`}` region nesting;
* definitions are found lexically (`x = ...` results, `^bb(a: T):` block
  headers/arguments) and filtered against the set of value names the
  parsed module actually defines — which discards look-alikes such as
  `variadic = false` inside type syntax;
* references resolve to the nearest preceding definition in the innermost
  enclosing scope (a lexical approximation of SSA dominance); block labels
  resolve order-independently within their region; symbols module-wide.

When a document does not parse, diagnostics are still published and
navigation degrades gracefully to the unfiltered lexical index.

## LSP stack

Transport is the synchronous `lsp-server` + `lsp-types` stack
(rust-analyzer's), chosen over `tower-lsp`/`tokio` for minimal
dependencies — a per-document, single-module server has no need for an
async runtime.
