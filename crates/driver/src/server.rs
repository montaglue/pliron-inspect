//! Resident-server extension of the driver protocol: modules are loaded
//! once, pipeline runs execute on a bounded worker pool with per-pass
//! progress and cancellation, per-pass IR is served from capture or by
//! deterministic replay, and artifacts are fetched when a run completes.
//!
//! Same line-delimited JSON transport as [crate::harness::run_stdio_driver]
//! (commands are defined in `pliron_inspect_protocol::server`); an optional
//! localhost HTTP listener accepts the identical command objects as POST
//! bodies and answers with the identical JSON — a thin shim over the same
//! handler.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use pliron_inspect_lsp::DocumentAnalysis;
use pliron_inspect_protocol::server as wire;
use serde_json::{Value, json};

use crate::harness::{AttributionOps, DriverHooks, PipelineEvent, parse_ir};

/// Builds a fresh [DriverHooks] per worker/request thread (the hooks
/// themselves need not be `Send`).
pub type HooksFactory = Arc<dyn Fn() -> Box<dyn DriverHooks> + Send + Sync>;

struct ModuleRecord {
    /// Kept for symmetry with the wire request; nothing reads it back yet.
    #[allow(dead_code)]
    name: String,
    text: String,
}

struct RunRecord {
    module_id: u64,
    target: String,
    config: BTreeMap<String, String>,
    inspect: bool,
    upto: Option<usize>,
    status: wire::RunStatus,
    passes: Vec<wire::PassTiming>,
    total_passes: usize,
    captured: Vec<String>,
    artifact: Option<Vec<u8>>,
    sidecars: Vec<(String, Vec<u8>)>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

struct State {
    modules: HashMap<u64, ModuleRecord>,
    runs: HashMap<u64, RunRecord>,
    next_id: u64,
}

pub struct Server {
    state: Mutex<State>,
    factory: HooksFactory,
    attribution_cache: Mutex<HashMap<(u64, String), Arc<AttributionOps>>>,
    queue: Mutex<mpsc::Sender<u64>>,
    workers: usize,
    /// Language-feature analyses keyed by IR document identity
    /// (`module:<id>` / `run:<id>:<pass>`); both underlying texts are
    /// immutable, so entries never go stale. Bounded (cleared when full).
    analyses: Mutex<HashMap<String, Arc<DocumentAnalysis>>>,
}

impl Server {
    pub fn new(factory: HooksFactory, workers: usize) -> Arc<Self> {
        let (tx, rx) = mpsc::channel::<u64>();
        let server = Arc::new(Server {
            state: Mutex::new(State {
                modules: HashMap::new(),
                runs: HashMap::new(),
                next_id: 1,
            }),
            factory,
            attribution_cache: Mutex::new(HashMap::new()),
            queue: Mutex::new(tx),
            workers: workers.max(1),
            analyses: Mutex::new(HashMap::new()),
        });
        let rx = Arc::new(Mutex::new(rx));
        for _ in 0..server.workers {
            let server = Arc::clone(&server);
            let rx = Arc::clone(&rx);
            std::thread::spawn(move || {
                loop {
                    let run_id = {
                        let rx = rx.lock().unwrap_or_else(|e| e.into_inner());
                        match rx.recv() {
                            Ok(id) => id,
                            Err(_) => return,
                        }
                    };
                    server.execute_run(run_id);
                }
            });
        }
        server
    }

    fn fresh_id(state: &mut State) -> u64 {
        let id = state.next_id;
        state.next_id += 1;
        id
    }

    /// One protocol command → one JSON response. Shared by the stdio loop
    /// and the HTTP shim.
    pub fn handle(&self, cmd: &Value) -> Value {
        match cmd["cmd"].as_str() {
            Some("server_health") => {
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                to_value(wire::ServerHealthResponse {
                    ok: true,
                    workers: self.workers,
                    modules: state.modules.len(),
                    runs: state.runs.len(),
                })
            }
            Some("list_targets") => {
                let hooks = (self.factory)();
                to_value(wire::ListTargetsResponse {
                    targets: hooks.list_targets(),
                })
            }
            Some("load_module") => match from_value::<wire::LoadModuleRequest>(cmd) {
                Ok(req) => self.load_module(req),
                Err(e) => error(&e),
            },
            Some("start_run") => match from_value::<wire::StartRunRequest>(cmd) {
                Ok(req) => self.start_run(req),
                Err(e) => error(&e),
            },
            Some("run_status") => match cmd["runId"].as_u64() {
                Some(id) => self.run_status(id),
                None => error("run_status requires runId"),
            },
            Some("cancel_run") => match cmd["runId"].as_u64() {
                Some(id) => self.cancel_run(id),
                None => error("cancel_run requires runId"),
            },
            Some("run_costs") => match from_value::<wire::RunCostsRequest>(cmd) {
                Ok(req) => self.run_costs(req),
                Err(e) => error(&format!("bad run_costs request: {e}")),
            },
            Some("run_ir") => match from_value::<wire::RunIrRequest>(cmd) {
                Ok(req) => self.run_ir(req),
                Err(e) => error(&e),
            },
            Some("run_artifact") => match cmd["runId"].as_u64() {
                Some(id) => self.run_artifact(id),
                None => error("run_artifact requires runId"),
            },
            Some(cmd_name @ ("ir_hover" | "ir_definition" | "ir_references")) => {
                match from_value::<wire::IrQueryRequest>(cmd) {
                    Ok(req) => self.ir_query(cmd_name, req),
                    Err(e) => error(&e),
                }
            }
            Some("ir_diagnostics") => match from_value::<wire::IrDiagnosticsRequest>(cmd) {
                Ok(req) => self.ir_diagnostics(req),
                Err(e) => error(&e),
            },
            Some(other) => error(&format!("unknown server command: {other}")),
            None => error("missing cmd"),
        }
    }

    fn load_module(&self, req: wire::LoadModuleRequest) -> Value {
        // Parse once to verify; the stored text is what runs re-parse.
        let hooks = (self.factory)();
        let mut ctx = hooks.create_context();
        if let Err(e) = parse_ir(&req.text, &mut ctx) {
            return error(&format!("module does not parse: {e}"));
        }
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = Self::fresh_id(&mut state);
        state.modules.insert(
            id,
            ModuleRecord {
                name: req.name.clone(),
                text: req.text,
            },
        );
        to_value(wire::LoadModuleResponse {
            module_id: id,
            name: req.name,
        })
    }

    fn start_run(&self, req: wire::StartRunRequest) -> Value {
        let hooks = (self.factory)();
        let passes = match hooks.pipeline_pass_names(&req.target, &req.config) {
            Ok(p) => p,
            Err(e) => return error(&e),
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.modules.contains_key(&req.module_id) {
            return error(&format!("no module {}", req.module_id));
        }
        let id = Self::fresh_id(&mut state);
        state.runs.insert(
            id,
            RunRecord {
                module_id: req.module_id,
                target: req.target,
                config: req.config,
                inspect: req.inspect,
                upto: req.upto,
                status: wire::RunStatus::Queued,
                passes: Vec::new(),
                total_passes: req.upto.map_or(passes.len(), |u| u.min(passes.len())),
                captured: Vec::new(),
                artifact: None,
                sidecars: Vec::new(),
                error: None,
                cancel: Arc::new(AtomicBool::new(false)),
            },
        );
        drop(state);
        let _ = self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .send(id);
        to_value(wire::StartRunResponse { run_id: id, passes })
    }

    fn execute_run(self: &Arc<Self>, run_id: u64) {
        let (text, target, config, inspect, upto, cancel) = {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(run) = state.runs.get_mut(&run_id) else {
                return;
            };
            if run.cancel.load(Ordering::SeqCst) {
                run.status = wire::RunStatus::Cancelled;
                return;
            }
            run.status = wire::RunStatus::Running;
            let module_id = run.module_id;
            let payload = (
                String::new(),
                run.target.clone(),
                run.config.clone(),
                run.inspect,
                run.upto,
                Arc::clone(&run.cancel),
            );
            let text = state.modules[&module_id].text.clone();
            (text, payload.1, payload.2, payload.3, payload.4, payload.5)
        };

        let hooks = (self.factory)();
        let mut ctx = hooks.create_context();
        let root = match parse_ir(&text, &mut ctx) {
            Ok(root) => root,
            Err(e) => return self.finish_run(run_id, Err(format!("parse failed: {e}"))),
        };
        let server = Arc::clone(self);
        let result = hooks.run_pipeline(
            &mut ctx,
            root,
            &target,
            &config,
            upto,
            inspect,
            &mut |event: PipelineEvent| {
                let mut state = server.state.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(run) = state.runs.get_mut(&run_id) {
                    run.passes.push(wire::PassTiming {
                        name: event.name,
                        micros: event.micros,
                    });
                    if let Some(ir) = event.ir {
                        run.captured.push(ir);
                    }
                }
                !cancel.load(Ordering::SeqCst)
            },
        );
        if let Err(e) = result {
            return self.finish_run(run_id, Err(e));
        }
        if cancel.load(Ordering::SeqCst) {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(run) = state.runs.get_mut(&run_id) {
                run.status = wire::RunStatus::Cancelled;
            }
            return;
        }
        // Artifacts only for complete (non-upto) runs.
        let artifact = if upto.is_none() {
            match hooks.write_artifact(&mut ctx, root, &target) {
                Ok(bytes) => {
                    Some((bytes, hooks.artifact_sidecars(&mut ctx, root, &target, &config)))
                }
                Err(e) => return self.finish_run(run_id, Err(e)),
            }
        } else {
            None
        };
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(run) = state.runs.get_mut(&run_id) {
            if let Some((bytes, sidecars)) = artifact {
                run.artifact = Some(bytes);
                run.sidecars = sidecars;
            }
            run.status = wire::RunStatus::Done;
        }
    }

    fn finish_run(&self, run_id: u64, result: Result<(), String>) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(run) = state.runs.get_mut(&run_id) {
            match result {
                Ok(()) => run.status = wire::RunStatus::Done,
                Err(e) => {
                    run.status = wire::RunStatus::Failed;
                    run.error = Some(e);
                }
            }
        }
    }

    fn run_status(&self, run_id: u64) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(run) = state.runs.get(&run_id) else {
            return error(&format!("no run {run_id}"));
        };
        to_value(wire::RunStatusResponse {
            run_id,
            status: run.status,
            passes: run.passes.clone(),
            total_passes: run.total_passes,
            error: run.error.clone(),
        })
    }

    fn cancel_run(&self, run_id: u64) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(run) = state.runs.get(&run_id) else {
            return error(&format!("no run {run_id}"));
        };
        run.cancel.store(true, Ordering::SeqCst);
        json!({"ok": true})
    }

    fn run_ir(&self, req: wire::RunIrRequest) -> Value {
        let (text, target, config, captured, done_passes) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(run) = state.runs.get(&req.run_id) else {
                return error(&format!("no run {}", req.run_id));
            };
            let text = state.modules[&run.module_id].text.clone();
            (
                text,
                run.target.clone(),
                run.config.clone(),
                if run.inspect {
                    Some(run.captured.clone())
                } else {
                    None
                },
                run.passes.len(),
            )
        };
        if done_passes == 0 {
            return error("run has not completed any pass yet");
        }
        let pass = req.pass.unwrap_or(done_passes - 1);
        if pass >= done_passes {
            return error(&format!("pass {pass} not completed yet ({done_passes} done)"));
        }
        if let Some(captured) = captured
            && let Some(ir) = captured.get(pass)
        {
            return to_value(wire::RunIrResponse {
                run_id: req.run_id,
                pass,
                ir: ir.clone(),
                source: "captured".to_string(),
            });
        }
        match self.replay_ir(&text, &target, &config, pass) {
            Ok(ir) => to_value(wire::RunIrResponse {
                run_id: req.run_id,
                pass,
                ir,
                source: "replay".to_string(),
            }),
            Err(e) => error(&e),
        }
    }

    /// The backward cost lift (crabbit docs/PROFILE-FEEDBACK-BACKWARD.md):
    /// join a `profile_ingest.py` op_costs payload with the attribution
    /// ops at the requested boundary of this run's pipeline. `source`
    /// (default) is the pre-mid-end numbering (payload key `source`,
    /// falling back to `lifted` for profiles taken without mid-end
    /// stamping); `ra` is the RA-boundary numbering (payload key
    /// `lifted`). The boundary op table is cached per (run, level) — the
    /// replay is deterministic.
    fn run_costs(&self, req: wire::RunCostsRequest) -> Value {
        let level = req.level.unwrap_or_else(|| "source".to_string());
        let (boundary_pass, primary_key, fallback_key) = match level.as_str() {
            "source" => ("llvm-op-ids", "source", Some("lifted")),
            "ra" => ("aarch64-op-ids", "lifted", None),
            other => return error(&format!("unknown level `{other}` (source, ra)")),
        };
        let (text, target, config) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(run) = state.runs.get(&req.run_id) else {
                return error(&format!("no run {}", req.run_id));
            };
            let Some(module) = state.modules.get(&run.module_id) else {
                return error("run's module is gone");
            };
            (module.text.clone(), run.target.clone(), run.config.clone())
        };
        let cache_key = (req.run_id, level.clone());
        let ops = {
            let cached = self.attribution_cache.lock().unwrap().get(&cache_key).cloned();
            match cached {
                Some(ops) => ops,
                None => {
                    let hooks = (self.factory)();
                    match hooks.attribution_ops(&text, &target, &config, boundary_pass) {
                        Ok(ops) => {
                            let ops = Arc::new(ops);
                            self.attribution_cache
                                .lock()
                                .unwrap()
                                .insert(cache_key, ops.clone());
                            ops
                        }
                        Err(e) => return error(&format!("attribution replay failed: {e}")),
                    }
                }
            }
        };
        let mut functions = Vec::new();
        for (symbol, table) in &ops.functions {
            let payload = &req.op_costs[symbol.as_str()];
            let costs = payload
                .get(primary_key)
                .or_else(|| fallback_key.and_then(|k| payload.get(k)))
                .and_then(|v| v.as_object());
            let Some(costs) = costs else {
                continue;
            };
            let mut entries = Vec::new();
            let mut roots = std::collections::BTreeMap::new();
            let mut unmatched = std::collections::BTreeMap::new();
            let by_id: HashMap<u32, (Option<u32>, &String)> = table
                .iter()
                .map(|(id, line, snippet)| (*id, (*line, snippet)))
                .collect();
            for (key, value) in costs {
                let cost = value.as_f64().unwrap_or(0.0);
                if cost <= 0.0 {
                    continue;
                }
                match key.parse::<u32>() {
                    Ok(id) => match by_id.get(&id) {
                        Some((line, snippet)) => entries.push(wire::CostEntry {
                            id: key.clone(),
                            cost,
                            line: *line,
                            snippet: (*snippet).clone(),
                        }),
                        None => {
                            unmatched.insert(key.clone(), cost);
                        }
                    },
                    Err(_) => {
                        roots.insert(key.clone(), cost);
                    }
                }
            }
            entries.sort_by(|a, b| b.cost.total_cmp(&a.cost));
            functions.push(wire::FunctionCosts {
                symbol: symbol.clone(),
                entries,
                roots,
                unmatched,
            });
        }
        to_value(wire::RunCostsResponse {
            run_id: req.run_id,
            level,
            ir: ops.ir.clone(),
            functions,
        })
    }

    /// Deterministic replay of the first `pass + 1` passes from the stored
    /// module text; returns the IR text after that pass.
    fn replay_ir(
        &self,
        text: &str,
        target: &str,
        config: &BTreeMap<String, String>,
        pass: usize,
    ) -> Result<String, String> {
        let hooks = (self.factory)();
        let mut ctx = hooks.create_context();
        let root = parse_ir(text, &mut ctx).map_err(|e| format!("replay parse failed: {e}"))?;
        let mut last_ir = None;
        hooks
            .run_pipeline(
                &mut ctx,
                root,
                target,
                config,
                Some(pass + 1),
                true,
                &mut |event: PipelineEvent| {
                    last_ir = event.ir;
                    true
                },
            )
            .map_err(|e| format!("replay failed: {e}"))?;
        last_ir.ok_or_else(|| "replay produced no IR".to_string())
    }

    /// The analyzed IR document for a language-feature query: a loaded
    /// module's stored text (`module_id`), or a run's rendered IR at
    /// `pass` (default: latest completed pass). Cached — both sources are
    /// immutable.
    fn analysis_for(
        &self,
        run_id: Option<u64>,
        module_id: Option<u64>,
        pass: Option<usize>,
    ) -> Result<Arc<DocumentAnalysis>, String> {
        let (key, text) = match (run_id, module_id) {
            (Some(run_id), _) => {
                let (text, target, config, captured, done_passes) = {
                    let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    let run = state
                        .runs
                        .get(&run_id)
                        .ok_or_else(|| format!("no run {run_id}"))?;
                    (
                        state.modules[&run.module_id].text.clone(),
                        run.target.clone(),
                        run.config.clone(),
                        if run.inspect {
                            Some(run.captured.clone())
                        } else {
                            None
                        },
                        run.passes.len(),
                    )
                };
                if done_passes == 0 {
                    return Err("run has not completed any pass yet".to_string());
                }
                let pass = pass.unwrap_or(done_passes - 1);
                if pass >= done_passes {
                    return Err(format!(
                        "pass {pass} not completed yet ({done_passes} done)"
                    ));
                }
                let key = format!("run:{run_id}:{pass}");
                if let Some(hit) = self
                    .analyses
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&key)
                {
                    return Ok(Arc::clone(hit));
                }
                let ir = match captured.and_then(|c| c.get(pass).cloned()) {
                    Some(ir) => ir,
                    None => self.replay_ir(&text, &target, &config, pass)?,
                };
                (key, ir)
            }
            (None, Some(module_id)) => {
                let key = format!("module:{module_id}");
                if let Some(hit) = self
                    .analyses
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(&key)
                {
                    return Ok(Arc::clone(hit));
                }
                let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let module = state
                    .modules
                    .get(&module_id)
                    .ok_or_else(|| format!("no module {module_id}"))?;
                (key, module.text.clone())
            }
            (None, None) => return Err("ir query requires runId or moduleId".to_string()),
        };
        let analysis = Arc::new(DocumentAnalysis::new(text));
        let mut cache = self.analyses.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= 16 {
            cache.clear();
        }
        cache.insert(key, Arc::clone(&analysis));
        Ok(analysis)
    }

    fn ir_query(&self, cmd: &str, req: wire::IrQueryRequest) -> Value {
        let analysis = match self.analysis_for(req.run_id, req.module_id, req.pass) {
            Ok(a) => a,
            Err(e) => return error(&e),
        };
        let pos = lsp_types::Position {
            line: req.position.line,
            character: req.position.character,
        };
        match cmd {
            "ir_hover" => {
                let (range, contents) = match analysis.hover(pos) {
                    Some((range, value)) => (Some(to_doc_range(range)), Some(value)),
                    None => (None, None),
                };
                to_value(wire::IrHoverResponse { contents, range })
            }
            "ir_definition" => to_value(wire::IrDefinitionResponse {
                range: analysis.definition(pos).map(to_doc_range),
            }),
            "ir_references" => to_value(wire::IrReferencesResponse {
                ranges: analysis
                    .references(pos, req.include_declaration)
                    .into_iter()
                    .map(to_doc_range)
                    .collect(),
            }),
            _ => error("unreachable ir query"),
        }
    }

    fn ir_diagnostics(&self, req: wire::IrDiagnosticsRequest) -> Value {
        let analysis = match self.analysis_for(req.run_id, req.module_id, req.pass) {
            Ok(a) => a,
            Err(e) => return error(&e),
        };
        to_value(wire::IrDiagnosticsResponse {
            parsed: analysis.parsed,
            diagnostics: analysis
                .diagnostics
                .iter()
                .map(|d| wire::IrDiagnostic {
                    range: to_doc_range(d.range),
                    severity: match d.severity {
                        Some(lsp_types::DiagnosticSeverity::WARNING) => "warning",
                        Some(lsp_types::DiagnosticSeverity::INFORMATION) => "info",
                        Some(lsp_types::DiagnosticSeverity::HINT) => "hint",
                        _ => "error",
                    }
                    .to_string(),
                    source: d.source.clone(),
                    message: d.message.clone(),
                })
                .collect(),
        })
    }

    fn run_artifact(&self, run_id: u64) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(run) = state.runs.get(&run_id) else {
            return error(&format!("no run {run_id}"));
        };
        let Some(artifact) = &run.artifact else {
            return error(&format!(
                "run {run_id} has no artifact (status {:?})",
                run.status
            ));
        };
        to_value(wire::RunArtifactResponse {
            run_id,
            artifact_base64: wire::base64_encode(artifact),
            sidecars: run
                .sidecars
                .iter()
                .map(|(name, bytes)| wire::SidecarArtifact {
                    name: name.clone(),
                    base64: wire::base64_encode(bytes),
                })
                .collect(),
        })
    }
}

fn to_doc_range(range: lsp_types::Range) -> wire::DocRange {
    wire::DocRange {
        start: wire::DocPosition {
            line: range.start.line,
            character: range.start.character,
        },
        end: wire::DocPosition {
            line: range.end.line,
            character: range.end.character,
        },
    }
}

fn to_value<T: serde::Serialize>(value: T) -> Value {
    serde_json::to_value(value).unwrap_or_else(|e| json!({"error": format!("{e}")}))
}

fn from_value<T: serde::de::DeserializeOwned>(cmd: &Value) -> Result<T, String> {
    serde_json::from_value(cmd.clone()).map_err(|e| format!("bad request: {e}"))
}

fn error(msg: &str) -> Value {
    json!({"error": msg})
}

/// Serve the resident protocol on stdin/stdout (one JSON command per line,
/// one JSON response per line), optionally also on a localhost HTTP
/// listener (POST any path with the command object as the body).
pub fn run_server_stdio(
    factory: HooksFactory,
    workers: usize,
    http: Option<std::net::SocketAddr>,
) -> anyhow::Result<()> {
    let server = Server::new(factory, workers);
    if let Some(addr) = http {
        anyhow::ensure!(
            addr.ip().is_loopback(),
            "the HTTP shim binds loopback only (got {addr})"
        );
        let server = Arc::clone(&server);
        std::thread::spawn(move || serve_http(server, addr));
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(cmd) => server.handle(&cmd),
            Err(e) => error(&format!("invalid JSON: {e}")),
        };
        writeln!(stdout.lock(), "{response}")?;
    }
    Ok(())
}

/// Minimal HTTP/1.1 shim: each POST body is one protocol command.
fn serve_http(server: Arc<Server>, addr: std::net::SocketAddr) {
    let listener = match std::net::TcpListener::bind(addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("http shim failed to bind {addr}: {e}");
            return;
        }
    };
    eprintln!("http shim listening on {addr}");
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let server = Arc::clone(&server);
        std::thread::spawn(move || {
            let _ = handle_http(&server, stream);
        });
    }
}

fn handle_http(server: &Server, mut stream: std::net::TcpStream) -> std::io::Result<()> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Ok(());
        }
        buf.push(byte[0]);
        if buf.len() > 64 * 1024 {
            return Ok(());
        }
    }
    let headers = String::from_utf8_lossy(&buf);
    let content_length = headers
        .lines()
        .find_map(|l| {
            let (name, value) = l.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    let mut body = vec![0u8; content_length];
    stream.read_exact(&mut body)?;
    let response = match serde_json::from_slice::<Value>(&body) {
        Ok(cmd) => server.handle(&cmd),
        Err(e) => error(&format!("invalid JSON: {e}")),
    };
    let payload = response.to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    )
}
