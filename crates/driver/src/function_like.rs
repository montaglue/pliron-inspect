//! Op interface for pliron-inspect's default root-selection heuristic
//! (`cfg_render::discover_function_roots`).
//!
//! Not part of pliron core (proposed upstream, not yet merged — see
//! MIGRATION-TO-PLIRON.md item #6 in the stair repo). Defined here, the one
//! place every driver already depends on, so any dialect crate wanting its
//! ops picked up as CFG roots implements this trait directly rather than
//! pliron-inspect depending on a specific dialect.

use pliron::{
    context::{Context, Ptr},
    op::Op,
    derive::op_interface,
    builtin::op_interfaces::{OneRegionInterface, SymbolOpInterface},
    region::Region,
    result::Result,
};

/// An [Op] that owns an executable function-like body, analogous to MLIR's
/// FunctionOpInterface.
#[op_interface]
pub trait FunctionLikeInterface: OneRegionInterface + SymbolOpInterface {
    /// Return the function body region. Declarations should return `None`.
    fn body_region(&self, ctx: &Context) -> Option<Ptr<Region>> {
        Some(self.get_region(ctx))
    }

    /// User-facing name, e.g. for tooling to label roots.
    fn display_name(&self, ctx: &Context) -> String {
        format!("@{}", self.get_symbol_name(ctx))
    }

    fn verify(_op: &dyn Op, _ctx: &Context) -> Result<()>
    where
        Self: Sized,
    {
        Ok(())
    }
}
