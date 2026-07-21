//! pliron-inspect driver library: IR render-document builders and the
//! stdio JSON protocol harness. Projects embed this in a small binary that
//! links their dialects and passes, and point the pliron-inspect server at
//! it with `--driver`.

pub mod cfg_render;
pub mod function_like;
pub mod harness;
pub mod structural;
pub mod tree_render;

pub use function_like::FunctionLikeInterface;
pub use harness::{DriverHooks, run_stdio_driver};

// ---- compatibility re-exports over the pliron core (cleanup pending) ----
pub use pliron::{
    attribute, basic_block, builtin, common_traits, context, debug_info, dialect,
    graph, identifier, irbuild, irfmt, linked_list, location, op, operation, opts,
    parsable, printable, region, storage_uniquer, symbol_table, r#type,
    uniqued_any, utils, value,
};
pub mod result {
    pub use pliron::result::*;
    pub type STAIRResult<T> = pliron::result::Result<T>;
}
pub mod ir {
    pub use pliron::{
        attribute, basic_block, dialect, irfmt, location, op, operation, region, value,
    };
    pub use pliron::r#type;
}
pub mod dialects {
    pub use pliron::builtin;
}
pub use pliron::{
    arg_err, arg_err_noloc, arg_error, arg_error_noloc, create_err, create_error,
    dict_key, impl_verify_succ, indented_block, input_err, input_err_noloc,
    input_error, input_error_noloc, type_to_trait, verify_err, verify_err_noloc,
    verify_error, verify_error_noloc,
};
