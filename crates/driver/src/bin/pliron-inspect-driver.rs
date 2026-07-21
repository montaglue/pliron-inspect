//! Default pliron-inspect driver: understands whatever dialects are linked
//! into it (the pliron builtin dialect only, in this default build).
//! Projects with their own dialects build their own driver binary from
//! [pliron_inspect_driver::run_stdio_driver].

use clap::Parser;
use std::path::PathBuf;

use pliron_inspect_driver::{DriverHooks, run_stdio_driver};

#[derive(Parser)]
#[command(name = "pliron-inspect-driver")]
#[command(about = "Default pliron IR driver for pliron-inspect")]
struct Args {
    /// Input IR file
    input: Option<PathBuf>,
}

struct DefaultHooks;
impl DriverHooks for DefaultHooks {}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    run_stdio_driver(&DefaultHooks, args.input.as_deref())
}
