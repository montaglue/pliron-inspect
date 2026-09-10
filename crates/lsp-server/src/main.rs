//! Reference LSP server binary for pliron textual IR: the dialect bundle
//! is pliron's builtin dialect plus the LLVM dialect from `pliron-llvm`.
//!
//! Projects with their own dialects should build their own binary; see
//! the `pliron-inspect-lsp` crate docs.

// Link the LLVM dialect so its ops/types/attributes self-register on
// `Context::new()`.
use pliron_llvm as _;

fn main() -> anyhow::Result<()> {
    pliron_inspect_lsp::run_stdio_server()
}
