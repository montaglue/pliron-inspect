//! An embeddable LSP server for pliron textual IR (`.plir`).
//!
//! # Dialect bundles, not dynamic dialects
//!
//! pliron dialects are Rust code that registers itself at link time:
//! everything linked into a binary self-registers when
//! [`Context::new`](pliron::context::Context::new) runs. Unlike MLIR's
//! generic LSP, this server therefore "receives" its dialects by being
//! *compiled* with them. This crate is a library; each project builds its
//! own tiny LSP binary that links its dialect crates and calls
//! [`run_stdio_server`]:
//!
//! ```ignore
//! // my-project-lsp/src/main.rs
//! use my_dialects as _; // link-time dialect registration
//! use pliron_llvm as _;
//!
//! fn main() -> anyhow::Result<()> {
//!     pliron_inspect_lsp::run_stdio_server()
//! }
//! ```
//!
//! The `pliron-inspect-lsp-server` crate in this workspace is the
//! reference bundle (pliron builtin + `pliron-llvm`).
//!
//! # One module per document
//!
//! Every open document is parsed into its own fresh pliron `Context` as a
//! single module. All analysis — diagnostics, definitions, references,
//! hover, document symbols — is strictly intra-module. There is no
//! workspace indexing, no cross-file resolution and no cross-module
//! symbol tables; this deliberately preserves pliron's one-module
//! compilation philosophy.
//!
//! # Features
//!
//! * **Diagnostics**: parse errors (with the parser's source position)
//!   and verifier errors (best-effort: the location of the offending
//!   entity) are published on `didOpen`/`didChange`.
//! * **Document symbols**: symbol-defining ops (`@name`) as a tree, with
//!   labeled blocks as children.
//! * **Go-to-definition / find-references** for SSA value names, block
//!   labels (`^bb`) and symbols (`@sym`), via a token-level scan of the
//!   document keyed off the parsed module (pliron's IR does not retain
//!   source spans for value uses); see [`navigation`] for the exact
//!   rules.
//! * **Hover**: op name, result types and attributes for ops; type and
//!   defining entity for values; signatures for blocks.

pub mod analysis;
pub mod lex;
pub mod lineindex;
pub mod navigation;
pub mod server;

pub use analysis::DocumentAnalysis;
pub use server::run_stdio_server;
