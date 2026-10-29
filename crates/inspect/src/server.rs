use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path as AxumPath, Query, State},
    http::{StatusCode, header::CONTENT_TYPE},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use pliron_inspect_protocol::trace::{self, TRACE_EXTENSION};
use pliron_inspect_protocol::types::*;
use serde_json::json;

use crate::history::Histories;
use crate::session::Session;

const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone)]
pub struct AppState {
    /// Directory holding the built frontend (index.html + assets);
    /// defaults to the checkout's `frontend/dist`.
    pub frontend_dir: Option<PathBuf>,
    pub session: Arc<Session>,
    /// Diff and blame of the traces opened, computed once per trace.
    pub histories: Arc<Histories>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/api/health", get(health))
        .route("/api/capabilities", get(capabilities))
        .route("/api/state", get(session_state))
        .route("/api/folders", get(list_folders))
        .route("/api/dialects", post(open_dialects))
        .route("/api/dialects/close", post(close_dialects))
        .route("/api/traces/folder", post(set_traces_folder))
        .route("/api/traces", get(list_traces))
        .route("/api/traces/open", post(open_trace))
        .route("/api/traces/import", post(import_trace))
        .route("/api/history/step", post(history_step))
        .route("/api/ir", post(add_ir))
        .route("/api/ir/render", post(render_ir))
        .route("/api/ir/{query}", post(ir_query))
        .route("/{*path}", get(serve_dist_asset))
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024 * 1024))
        .with_state(state)
}

fn bad_request(error: impl std::fmt::Display) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": error.to_string() })),
    )
        .into_response()
}

async fn health() -> impl IntoResponse {
    Json(HealthResponse { ok: true })
}

async fn capabilities() -> impl IntoResponse {
    Json(DisplayCapabilities {
        protocol_version: PROTOCOL_VERSION,
        views: vec![
            ViewCapability::new(
                "text",
                "Text",
                "IR snapshots, with the dialects' hover, definitions and diagnostics",
            ),
            ViewCapability::new("ir-diff", "IR Diff", "Snapshot diff"),
            ViewCapability::new("cfg", "CFG", "Control-flow graph of a function"),
            ViewCapability::new("tree", "Tree", "Operation hierarchy"),
            ViewCapability::new(
                "versions",
                "Versions",
                "Trace versions of the selected project",
            ),
        ],
        frontend: FrontendStack {
            framework: "React".to_string(),
            bundler: "Vite".to_string(),
            language: "TypeScript".to_string(),
            graph: "React Flow + Flowblocks geometry".to_string(),
            tree: "D3 + d3-flextree".to_string(),
            text: "Monaco".to_string(),
            egraph: "Cytoscape.js".to_string(),
        },
    })
}

async fn session_state(State(state): State<AppState>) -> Response {
    Json(state.session.state().await).into_response()
}

#[derive(serde::Deserialize)]
struct FoldersQuery {
    path: Option<String>,
}

/// The folder picker's listing of `path` (default: the home directory).
async fn list_folders(Query(query): Query<FoldersQuery>) -> Response {
    match crate::folders::list(query.path.as_deref()) {
        Ok(listing) => Json(listing).into_response(),
        Err(error) => bad_request(error),
    }
}

#[derive(serde::Deserialize)]
struct FolderRequest {
    path: String,
}

/// Use `path` (a folder under the home directory) as the dialects folder:
/// pliron-lsp is started on it. Asynchronous: `/api/state` reports it.
async fn open_dialects(State(state): State<AppState>, Json(req): Json<FolderRequest>) -> Response {
    match crate::folders::resolve(&req.path) {
        Ok(root) => {
            state.session.open_dialects(root).await;
            Json(state.session.state().await).into_response()
        }
        Err(error) => bad_request(error),
    }
}

async fn close_dialects(State(state): State<AppState>) -> Response {
    state.session.close_dialects().await;
    Json(state.session.state().await).into_response()
}

/// Read traces from `path` (a folder under the home directory).
async fn set_traces_folder(
    State(state): State<AppState>,
    Json(req): Json<FolderRequest>,
) -> Response {
    match crate::folders::resolve(&req.path) {
        Ok(dir) => {
            state.session.set_traces(dir).await;
            Json(state.session.state().await).into_response()
        }
        Err(error) => bad_request(error),
    }
}

async fn list_traces(State(state): State<AppState>) -> Response {
    let dir = state.session.traces().await;
    let projects = match &dir {
        Some(dir) => match trace::discover_trace_projects(dir) {
            Ok(projects) => projects,
            Err(error) => return bad_request(format!("{error:#}")),
        },
        None => Vec::new(),
    };
    Json(TraceListResponse {
        dir: dir.map(|d| d.to_string_lossy().into_owned()),
        projects,
    })
    .into_response()
}

/// `filepath` as a trace of the traces folder (canonical), or why not.
async fn trace_in_folder(state: &AppState, filepath: &str) -> Result<PathBuf, String> {
    let Some(dir) = state.session.traces().await else {
        return Err("choose a traces folder first".to_string());
    };
    let path = std::fs::canonicalize(filepath).map_err(|error| format!("{filepath}: {error}"))?;
    if !path.starts_with(&dir) {
        return Err(format!("{} is not in the traces folder", path.display()));
    }
    if path.extension().and_then(|ext| ext.to_str()) != Some(TRACE_EXTENSION) {
        return Err(format!(
            "expected a .{TRACE_EXTENSION} trace file: {}",
            path.display()
        ));
    }
    Ok(path)
}

async fn open_trace(State(state): State<AppState>, Json(req): Json<OpenTraceRequest>) -> Response {
    let path = match trace_in_folder(&state, &req.filepath).await {
        Ok(path) => path,
        Err(error) => return bad_request(error),
    };
    match trace::CrabbitTraceFile::read(&path) {
        Ok(trace) => trace_response(path, trace),
        Err(error) => bad_request(format!("{error:#}")),
    }
}

#[derive(serde::Deserialize)]
struct HistoryStepRequest {
    filepath: String,
    step: usize,
}

/// One step of a trace's history: the diff of snapshot `step` against the
/// one before, and the blame of its lines.
async fn history_step(
    State(state): State<AppState>,
    Json(req): Json<HistoryStepRequest>,
) -> Response {
    let path = match trace_in_folder(&state, &req.filepath).await {
        Ok(path) => path,
        Err(error) => return bad_request(error),
    };
    match state.histories.steps(&path).await {
        Ok(steps) => match steps.get(req.step) {
            Some(step) => Json(step).into_response(),
            None => bad_request(format!("the trace has no step {}", req.step)),
        },
        Err(error) => bad_request(format!("{error:#}")),
    }
}

/// Save an uploaded trace into the traces folder (in its project's
/// subfolder) and open it.
async fn import_trace(
    State(state): State<AppState>,
    Json(req): Json<ImportTraceRequest>,
) -> Response {
    let Some(dir) = state.session.traces().await else {
        return bad_request("choose a traces folder first");
    };
    let filename = Path::new(&req.filename)
        .file_name()
        .and_then(|filename| filename.to_str())
        .unwrap_or("trace.crt");
    let filename = if filename.ends_with(&format!(".{TRACE_EXTENSION}")) {
        filename.to_string()
    } else {
        format!("{filename}.{TRACE_EXTENSION}")
    };

    let trace = match trace::CrabbitTraceFile::from_crt_str(&req.contents) {
        Ok(trace) => trace,
        Err(error) => {
            return bad_request(format!("{}: not a trace: {error:#}", req.filename));
        }
    };

    let stem = Path::new(&filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let project_dir = dir.join(trace::derive_project_from_stem(&stem));
    if let Err(error) = std::fs::create_dir_all(&project_dir) {
        return bad_request(format!("{}: {error}", project_dir.display()));
    }
    let mut path = project_dir.join(&filename);
    if path.exists() {
        path = unique_trace_destination(&project_dir, &filename);
    }
    if let Err(error) = trace.write(&path) {
        return bad_request(format!("{error:#}"));
    }
    trace_response(path, trace)
}

fn trace_response(path: PathBuf, trace: trace::CrabbitTraceFile) -> Response {
    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let pipeline = trace.meta.pipeline.clone();
    let snapshots = trace
        .ir_dumps
        .iter()
        .map(|dump| TraceSnapshot {
            label: dump.label.clone(),
            ir: dump.ir.clone(),
        })
        .collect();

    Json(OpenTraceResponse {
        filepath: path.to_string_lossy().to_string(),
        filename,
        meta: trace.meta,
        snapshots,
        pipeline,
    })
    .into_response()
}

fn unique_trace_destination(dir: &Path, filename: &str) -> PathBuf {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("trace");
    for index in 1.. {
        let candidate = dir.join(format!("{stem}-{index}.{TRACE_EXTENSION}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

#[derive(serde::Deserialize)]
struct AddIrRequest {
    text: String,
}

/// Keep an IR snapshot the UI shows; the views and language queries refer
/// to it by the returned id.
async fn add_ir(State(state): State<AppState>, Json(req): Json<AddIrRequest>) -> Response {
    let id = state.session.add_doc(req.text).await;
    Json(json!({ "id": id })).into_response()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RenderIrRequest {
    id: String,
    view: String,
    #[serde(default)]
    root_id: Option<String>,
}

async fn render_ir(State(state): State<AppState>, Json(req): Json<RenderIrRequest>) -> Response {
    Json(
        state
            .session
            .render(&req.id, &req.view, req.root_id.as_deref())
            .await,
    )
    .into_response()
}

#[derive(serde::Deserialize)]
struct IrQueryRequest {
    id: String,
    #[serde(default)]
    position: Option<serde_json::Value>,
}

/// `hover`, `definition`, `references` (at an LSP position) or
/// `diagnostics` of an IR snapshot, from its dialect engine.
async fn ir_query(
    State(state): State<AppState>,
    AxumPath(query): AxumPath<String>,
    Json(req): Json<IrQueryRequest>,
) -> Response {
    match state
        .session
        .language(&query, &req.id, req.position.as_ref())
        .await
    {
        Ok(value) => Json(value).into_response(),
        Err(error) => bad_request(error),
    }
}

fn frontend_dir(state: &AppState) -> PathBuf {
    state.frontend_dir.clone().unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("frontend")
            .join("dist")
    })
}

/// The built frontend, or a page that says how to build it.
async fn serve_index(State(state): State<AppState>) -> Response {
    let dist = frontend_dir(&state);
    match std::fs::read_to_string(dist.join("index.html")) {
        Ok(contents) => Html(contents).into_response(),
        Err(_) => {
            Html(include_str!("static/index.html").replace("{{dist}}", &dist.display().to_string()))
                .into_response()
        }
    }
}

async fn serve_dist_asset(
    State(state): State<AppState>,
    AxumPath(path): AxumPath<String>,
) -> Response {
    let dist = frontend_dir(&state);
    let Some(asset_path) = safe_dist_path(&dist, &path) else {
        return (StatusCode::BAD_REQUEST, "invalid asset path").into_response();
    };

    match std::fs::read(&asset_path) {
        Ok(bytes) => {
            let content_type = content_type_for(&asset_path);
            ([(CONTENT_TYPE, content_type)], bytes).into_response()
        }
        Err(_) => serve_index(State(state)).await,
    }
}

fn safe_dist_path(dist: &Path, path: &str) -> Option<PathBuf> {
    let mut out = dist.to_path_buf();
    for component in Path::new(path).components() {
        match component {
            Component::Normal(part) => out.push(part),
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("wasm") => "application/wasm",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}
