//! The driver side of the pliron-inspect protocol: line-delimited JSON
//! commands on stdin, one JSON response per line on stdout.

use std::fs;
use std::io::{BufRead, Write};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use pliron::combine::Parser;
use pliron::context::{Context, Ptr};
use pliron::location::Source;
use pliron::operation::{Operation, OperationParserConfig};
use pliron::parsable::{Parsable, State, state_stream_from_iterator};
use pliron::printable::Printable;

use crate::cfg_render::{build_cfg_render_document, diagnostic_document};
use crate::structural::build_ir_graph;
use crate::tree_render::{
    build_tree_render_document, diagnostic_document as tree_diagnostic_document,
};

/// One pipeline step's completion, as reported to [DriverHooks::run_pipeline]
/// observers: index and name of the pass, wall time, and the IR after it
/// when capture was requested.
pub struct PipelineEvent {
    pub index: usize,
    pub name: String,
    pub micros: u64,
    pub ir: Option<String>,
}

/// What the embedding binary provides: a context with its dialects
/// registered, and its pass pipeline.
///
/// The `run_stdio_driver` per-file commands use [Self::pass_names] /
/// [Self::run_pass]; the resident server ([crate::server]) additionally
/// uses the default-implemented pipeline/artifact methods below — drivers
/// that don't override them simply don't support serving.
/// See [DriverHooks::attribution_ops].
#[derive(Debug, Clone, Default)]
pub struct AttributionOps {
    /// The printed IR at the boundary (lines referenced by `line`).
    pub ir: String,
    /// symbol → [(op id, 0-based line or None, snippet)]
    pub functions: Vec<(String, Vec<(u32, Option<u32>, String)>)>,
}

pub trait DriverHooks {
    /// A fresh context. With derive-macro-defined entities this is usually
    /// just `Context::new()` (everything linked self-registers).
    fn create_context(&self) -> Context {
        Context::new()
    }

    /// Pass names offered to the UI, in pipeline order.
    fn pass_names(&self) -> Vec<String> {
        vec![]
    }

    /// Run `name` on `root`, returning the (possibly new) root operation.
    fn run_pass(
        &self,
        name: &str,
        _root: Ptr<Operation>,
        _ctx: &mut Context,
    ) -> Result<Ptr<Operation>, String> {
        Err(format!("unknown pass: {name}"))
    }

    /// Targets the full pipeline can compile for (resident server only).
    fn list_targets(&self) -> Vec<String> {
        vec![]
    }

    /// The pass names of the full pipeline for `target` under `config`
    /// (resident server only).
    fn pipeline_pass_names(
        &self,
        _target: &str,
        _config: &std::collections::BTreeMap<String, String>,
    ) -> Result<Vec<String>, String> {
        Err("pipeline runs are not supported by this driver".to_string())
    }

    /// Run the full pipeline for `target` under `config` on `root`,
    /// calling `events` after every pass (with the printed IR when
    /// `capture`). Stops cleanly after `upto` passes when set, or when
    /// `events` returns false (cancellation).
    #[allow(clippy::too_many_arguments)]
    fn run_pipeline(
        &self,
        _ctx: &mut Context,
        _root: Ptr<Operation>,
        _target: &str,
        _config: &std::collections::BTreeMap<String, String>,
        _upto: Option<usize>,
        _capture: bool,
        _events: &mut dyn FnMut(PipelineEvent) -> bool,
    ) -> Result<(), String> {
        Err("pipeline runs are not supported by this driver".to_string())
    }

    /// The attribution op table at a named boundary pass of `target`'s
    /// pipeline (crabbit docs/PROFILE-FEEDBACK-BACKWARD.md): replays the
    /// pipeline over `module_text` up to and including `boundary_pass`,
    /// with attribution stamping forced on, and returns the boundary IR
    /// text plus, per function, the ops carrying an attribution id.
    /// Default: unsupported.
    fn attribution_ops(
        &self,
        _module_text: &str,
        _target: &str,
        _config: &std::collections::BTreeMap<String, String>,
        _boundary_pass: &str,
    ) -> Result<AttributionOps, String> {
        Err("attribution is not supported by this driver".to_string())
    }

    /// The primary artifact for a module fully lowered by
    /// [Self::run_pipeline] (object bytes, PTX text, …).
    fn write_artifact(
        &self,
        _ctx: &mut Context,
        _root: Ptr<Operation>,
        _target: &str,
    ) -> Result<Vec<u8>, String> {
        Err("artifacts are not supported by this driver".to_string())
    }

    /// Named secondary artifacts (e.g. a block map) for a lowered module.
    fn artifact_sidecars(
        &self,
        _ctx: &mut Context,
        _root: Ptr<Operation>,
        _target: &str,
        _config: &std::collections::BTreeMap<String, String>,
    ) -> Vec<(String, Vec<u8>)> {
        vec![]
    }
}

fn print_plain(ctx: &Context, op: Ptr<Operation>) -> String {
    let state = pliron::printable::State::default();
    op.print(ctx, &state).to_string()
}

pub fn parse_ir(content: &str, ctx: &mut Context) -> anyhow::Result<Ptr<Operation>> {
    let state = State::new(ctx, Source::InMemory);
    let stream = state_stream_from_iterator(content.chars(), state);
    let config = OperationParserConfig {
        look_for_outlined_attrs: false,
    };
    let (op, _) = <Operation as Parsable>::parser(config)
        .parse(stream)
        .map_err(|e| anyhow::anyhow!("failed to parse IR: {}", e))?;
    Ok(op)
}

fn parse_file(
    hooks: &dyn DriverHooks,
    path: &Path,
) -> anyhow::Result<(Context, Ptr<Operation>, String)> {
    let content = fs::read_to_string(path)?;
    let mut ctx = hooks.create_context();
    let op = parse_ir(&content, &mut ctx)?;
    Ok((ctx, op, content))
}

/// Run the stdio protocol loop until stdin closes.
pub fn run_stdio_driver(hooks: &dyn DriverHooks, file_path: Option<&Path>) -> anyhow::Result<()> {
    let mut ctx;
    let mut op_ptr: Option<Ptr<Operation>>;
    let mut source_content;
    let mut parse_error: Option<String>;

    match file_path {
        Some(file_path) => match parse_file(hooks, file_path) {
            Ok((c, ptr, content)) => {
                ctx = c;
                op_ptr = Some(ptr);
                source_content = content;
                parse_error = None;
            }
            Err(e) => {
                ctx = hooks.create_context();
                op_ptr = None;
                source_content = fs::read_to_string(file_path).unwrap_or_default();
                parse_error = Some(format!("{}", e));
            }
        },
        None => {
            ctx = hooks.create_context();
            op_ptr = None;
            source_content = String::new();
            parse_error = Some("no input file was provided".to_string());
        }
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let cmd: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = serde_json::json!({"error": format!("invalid JSON: {}", e)});
                writeln!(stdout.lock(), "{}", resp)?;
                continue;
            }
        };

        let response = match cmd["cmd"].as_str() {
            Some("list_passes") => {
                serde_json::json!({"passes": hooks.pass_names()})
            }
            Some("get_ir") => match op_ptr {
                Some(ptr) => {
                    let ir = print_plain(&ctx, ptr);
                    let graph = build_ir_graph(&ctx, ptr);
                    serde_json::json!({"ir": ir, "graph": graph})
                }
                None => {
                    serde_json::json!({"error": parse_error.as_deref().unwrap_or("parse failed")})
                }
            },
            Some("graph_ir") => match cmd["ir"].as_str() {
                Some(ir) => {
                    let mut graph_ctx = hooks.create_context();
                    match parse_ir(ir, &mut graph_ctx) {
                        Ok(ptr) => {
                            let graph = build_ir_graph(&graph_ctx, ptr);
                            serde_json::json!({"graph": graph})
                        }
                        Err(e) => {
                            serde_json::json!({"error": format!("failed to graph supplied IR: {}", e)})
                        }
                    }
                }
                None => serde_json::json!({"error": "graph_ir command requires an 'ir' string"}),
            },
            Some("render") => {
                let view = cmd["view"].as_str().unwrap_or("cfg");
                let snapshot_id = cmd["snapshotId"]
                    .as_str()
                    .or_else(|| cmd["snapshot_id"].as_str())
                    .unwrap_or("ad-hoc")
                    .to_string();
                let root_id = cmd["rootId"].as_str().or_else(|| cmd["root_id"].as_str());

                match view {
                    "cfg" => {
                        if let Some(ir) = cmd["ir"].as_str().or_else(|| cmd["text"].as_str()) {
                            let mut render_ctx = hooks.create_context();
                            match parse_ir(ir, &mut render_ctx) {
                                Ok(ptr) => serde_json::to_value(build_cfg_render_document(
                                    &render_ctx,
                                    ptr,
                                    snapshot_id,
                                    root_id,
                                ))
                                .unwrap(),
                                Err(e) => serde_json::to_value(diagnostic_document(
                                    "cfg",
                                    snapshot_id,
                                    "CFG render",
                                    "parse-error",
                                    format!("failed to parse IR: {e}"),
                                ))
                                .unwrap(),
                            }
                        } else {
                            match op_ptr {
                                Some(ptr) => serde_json::to_value(build_cfg_render_document(
                                    &ctx,
                                    ptr,
                                    snapshot_id,
                                    root_id,
                                ))
                                .unwrap(),
                                None => serde_json::to_value(diagnostic_document(
                                    "cfg",
                                    snapshot_id,
                                    "CFG render",
                                    "parse-error",
                                    parse_error
                                        .as_deref()
                                        .unwrap_or("no IR was available for rendering"),
                                ))
                                .unwrap(),
                            }
                        }
                    }
                    "tree" => {
                        if let Some(ir) = cmd["ir"].as_str().or_else(|| cmd["text"].as_str()) {
                            let mut render_ctx = hooks.create_context();
                            match parse_ir(ir, &mut render_ctx) {
                                Ok(ptr) => serde_json::to_value(build_tree_render_document(
                                    &render_ctx,
                                    ptr,
                                    snapshot_id,
                                ))
                                .unwrap(),
                                Err(e) => serde_json::to_value(tree_diagnostic_document(
                                    "tree",
                                    snapshot_id,
                                    "Tree render",
                                    "parse-error",
                                    format!("failed to parse IR: {e}"),
                                ))
                                .unwrap(),
                            }
                        } else {
                            match op_ptr {
                                Some(ptr) => serde_json::to_value(build_tree_render_document(
                                    &ctx,
                                    ptr,
                                    snapshot_id,
                                ))
                                .unwrap(),
                                None => serde_json::to_value(tree_diagnostic_document(
                                    "tree",
                                    snapshot_id,
                                    "Tree render",
                                    "parse-error",
                                    parse_error
                                        .as_deref()
                                        .unwrap_or("no IR was available for rendering"),
                                ))
                                .unwrap(),
                            }
                        }
                    }
                    unknown => serde_json::to_value(diagnostic_document(
                        unknown,
                        snapshot_id,
                        format!("{unknown} render"),
                        "unknown-view",
                        format!("unknown render view: {unknown}"),
                    ))
                    .unwrap(),
                }
            }
            Some("run_pass") => match op_ptr {
                Some(ptr) => {
                    let name = cmd["name"].as_str().unwrap_or("");
                    match catch_unwind(AssertUnwindSafe(|| hooks.run_pass(name, ptr, &mut ctx))) {
                        Ok(Ok(new_ptr)) => {
                            op_ptr = Some(new_ptr);
                            let ir = print_plain(&ctx, new_ptr);
                            let graph = build_ir_graph(&ctx, new_ptr);
                            serde_json::json!({"ir": ir, "graph": graph})
                        }
                        Ok(Err(e)) => serde_json::json!({"error": e}),
                        Err(panic) => {
                            let msg = panic
                                .downcast_ref::<&str>()
                                .map(|s| s.to_string())
                                .or_else(|| panic.downcast_ref::<String>().cloned())
                                .unwrap_or_else(|| "unknown panic".to_string());
                            op_ptr = None;
                            parse_error = Some(format!("pass '{}' panicked: {}", name, msg));
                            serde_json::json!({"error": parse_error.as_deref().unwrap()})
                        }
                    }
                }
                None => {
                    serde_json::json!({"error": parse_error.as_deref().unwrap_or("no IR: parse failed")})
                }
            },
            Some("reset") | Some("reload") => match file_path {
                Some(file_path) => match parse_file(hooks, file_path) {
                    Ok((new_ctx, new_ptr, new_source)) => {
                        ctx = new_ctx;
                        op_ptr = Some(new_ptr);
                        source_content = new_source.clone();
                        parse_error = None;
                        let ir = print_plain(&ctx, new_ptr);
                        let graph = build_ir_graph(&ctx, new_ptr);
                        if cmd["cmd"].as_str() == Some("reload") {
                            serde_json::json!({"ir": ir, "source": new_source, "graph": graph})
                        } else {
                            serde_json::json!({"ir": ir, "graph": graph})
                        }
                    }
                    Err(e) => {
                        ctx = hooks.create_context();
                        op_ptr = None;
                        let new_source = fs::read_to_string(file_path).unwrap_or_default();
                        source_content = new_source.clone();
                        let msg = format!("{} failed: {}", cmd["cmd"].as_str().unwrap(), e);
                        parse_error = Some(msg.clone());
                        serde_json::json!({"error": msg, "source": new_source})
                    }
                },
                None => serde_json::json!({"error": "reset/reload requires an input file"}),
            },
            Some("get_source") => {
                serde_json::json!({"source": source_content})
            }
            Some(unknown) => {
                serde_json::json!({"error": format!("unknown command: {}", unknown)})
            }
            None => {
                serde_json::json!({"error": "missing 'cmd' field"})
            }
        };

        writeln!(stdout.lock(), "{}", response)?;
        stdout.lock().flush()?;
    }

    Ok(())
}
