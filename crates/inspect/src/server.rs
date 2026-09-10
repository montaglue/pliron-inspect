use std::{
    collections::BTreeMap,
    path::{Component, Path, PathBuf},
};

use axum::{
    Json, Router,
    extract::{Path as AxumPath, State},
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use pliron_inspect_protocol::trace::{self, TRACE_EXTENSION};

use crate::driver;
use pliron_inspect_protocol::types::*;

const PROTOCOL_VERSION: u32 = 1;

#[derive(Clone)]
pub struct AppState {
    pub driver_binary: Option<String>,
    pub trace_library_dirs: Vec<PathBuf>,
    pub temp_trace_dirs: Vec<PathBuf>,
    /// Address of a running `crabbit-analysisd` HTTP shim (loopback), if
    /// the analysis panels are enabled.
    pub analysis_server: Option<String>,
    /// Directory holding the built frontend (index.html + assets);
    /// defaults to `<manifest dir>/frontend/dist`.
    pub frontend_dir: Option<PathBuf>,
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/api/health", get(health))
        .route("/api/capabilities", get(capabilities))
        .route("/api/traces", get(list_traces))
        .route("/api/traces/import", post(import_trace))
        .route("/api/traces/open", post(open_trace))
        .route("/api/render", post(render_document))
        .route("/api/analysis", post(analysis_proxy))
        .route("/api/analysis/health", get(analysis_health))
        .route("/{*path}", get(serve_dist_asset))
        .with_state(state)
}

async fn list_traces(State(state): State<AppState>) -> Response {
    let library_projects = match discover_trace_projects(&state.trace_library_dirs) {
        Ok(projects) => projects,
        Err(error) => return error_response(error),
    };
    let temp_projects = match discover_trace_projects(&state.temp_trace_dirs) {
        Ok(projects) => projects,
        Err(error) => return error_response(error),
    };

    Json(TraceListResponse {
        library_projects,
        temp_projects,
        library_dirs: display_dirs(&state.trace_library_dirs),
        temp_dirs: display_dirs(&state.temp_trace_dirs),
    })
    .into_response()
}

async fn open_trace(State(state): State<AppState>, Json(req): Json<OpenTraceRequest>) -> Response {
    let mut path = match std::fs::canonicalize(&req.filepath) {
        Ok(path) => path,
        Err(error) => {
            return error_response(anyhow::anyhow!(
                "cannot resolve trace path '{}': {}",
                req.filepath,
                error
            ));
        }
    };

    if !is_trace_path(&path) {
        return error_response(anyhow::anyhow!(
            "expected a .{} trace file: {}",
            TRACE_EXTENSION,
            path.display()
        ));
    }

    let imported_from = if is_inside_any_dir(&path, &state.temp_trace_dirs) {
        let source = path.clone();
        match import_temp_trace(&state, &source) {
            Ok(imported) => {
                path = imported;
                Some(source.to_string_lossy().to_string())
            }
            Err(error) => return error_response(error),
        }
    } else {
        None
    };

    let trace = match trace::StairTraceFile::read(&path) {
        Ok(trace) => trace,
        Err(error) => return error_response(error),
    };

    trace_response(path, imported_from, trace)
}

async fn import_trace(
    State(state): State<AppState>,
    Json(req): Json<ImportTraceRequest>,
) -> Response {
    let filename = Path::new(&req.filename)
        .file_name()
        .and_then(|filename| filename.to_str())
        .unwrap_or("trace.stx");
    let filename = if filename.ends_with(&format!(".{TRACE_EXTENSION}")) {
        filename.to_string()
    } else {
        format!("{filename}.{TRACE_EXTENSION}")
    };

    let trace = match trace::StairTraceFile::from_stx_str(&req.contents) {
        Ok(trace) => trace,
        Err(error) => {
            return error_response(anyhow::anyhow!(
                "failed to parse imported trace '{}': {}",
                req.filename,
                error
            ));
        }
    };

    let Some(trace_library_dir) = state.trace_library_dirs.first() else {
        return error_response(anyhow::anyhow!(
            "no trace library directories are configured"
        ));
    };

    let stem = Path::new(&filename)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    let project_dir = trace_library_dir.join(trace::derive_project_from_stem(&stem));
    if let Err(error) = std::fs::create_dir_all(&project_dir) {
        return error_response(error.into());
    }

    let mut path = project_dir.join(&filename);
    if path.exists() {
        path = unique_trace_destination(&project_dir, std::ffi::OsStr::new(&filename));
    }

    if let Err(error) = trace.write(&path) {
        return error_response(error);
    }

    let path = match std::fs::canonicalize(&path) {
        Ok(path) => path,
        Err(error) => return error_response(error.into()),
    };

    trace_response(path, None, trace)
}

fn trace_response(
    path: PathBuf,
    imported_from: Option<String>,
    trace: trace::StairTraceFile,
) -> Response {
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
        imported_from,
        meta: trace.meta,
        snapshots,
        pipeline,
    })
    .into_response()
}

async fn health(State(state): State<AppState>) -> impl IntoResponse {
    Json(HealthResponse {
        ok: true,
        driver_binary: state.driver_binary,
    })
}

async fn capabilities() -> impl IntoResponse {
    Json(DisplayCapabilities {
        protocol_version: PROTOCOL_VERSION,
        views: vec![
            ViewCapability::new("text", "Text", "Plain Monaco IR snapshots"),
            ViewCapability::new("ir-diff", "IR Diff", "Monaco snapshot diff"),
            ViewCapability::new("cfg", "CFG", "React Flow + Flowblocks geometry"),
            ViewCapability::new("tree", "Tree", "D3 flextree operation hierarchy"),
            ViewCapability::new("egraph", "EGraph", "Cytoscape equivalence graph"),
            ViewCapability::new("versions", "Versions", "Trace versions of the selected project"),
        ],
        cli_commands: vec![
            "list_capabilities".to_string(),
            "parse_ir".to_string(),
            "run_pipeline".to_string(),
            "render".to_string(),
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

async fn render_document(
    State(state): State<AppState>,
    Json(mut req): Json<RenderRequest>,
) -> impl IntoResponse {
    let snapshot_id = req
        .snapshot_id
        .clone()
        .unwrap_or_else(|| "ad-hoc".to_string());
    let view = req.view.clone();
    let title = req
        .root_id
        .as_ref()
        .map(|root| format!("{view} render for {root}"))
        .unwrap_or_else(|| format!("{view} render"));

    let document = match view.as_str() {
        "text" | "ir-diff" => RenderDocument {
            version: PROTOCOL_VERSION,
            view: view.clone(),
            title: Some(title),
            snapshot_id: snapshot_id.clone(),
            entities: vec![],
            graph: None,
            tree: None,
            text: Some(TextDocument {
                language: req
                    .options
                    .get("language")
                    .and_then(|value| value.as_str())
                    .unwrap_or("stair-ir")
                    .to_string(),
                text: req.text.unwrap_or_default(),
                spans: vec![],
            }),
            diagnostics: vec![],
        },
        "cfg" | "tree" => match state.driver_binary.as_deref() {
            Some(driver_binary) => {
                req.snapshot_id = Some(snapshot_id.clone());
                match driver::render_document(driver_binary, &req).await {
                    Ok(document) => document,
                    Err(error) => diagnostic_document(
                        view,
                        snapshot_id,
                        title,
                        "driver-error",
                        format!("CLI render failed: {error}"),
                    ),
                }
            }
            None => diagnostic_document(
                view,
                snapshot_id,
                title,
                "driver-missing",
                "No pliron-inspect-driver driver binary is configured.",
            ),
        },
        "egraph" => diagnostic_document(
            view,
            snapshot_id,
            title,
            "renderer-not-wired",
            "This view is registered, but CLI-backed semantic rendering is not wired yet.",
        ),
        unknown => diagnostic_document(
            unknown.to_string(),
            snapshot_id,
            title,
            "unknown-view",
            format!("Unknown render view: {unknown}"),
        ),
    };

    Json(document)
}

fn diagnostic_document(
    view: String,
    snapshot_id: String,
    title: String,
    code: impl Into<String>,
    message: impl Into<String>,
) -> RenderDocument {
    RenderDocument {
        version: PROTOCOL_VERSION,
        view,
        title: Some(title),
        snapshot_id,
        entities: vec![],
        graph: None,
        tree: None,
        text: None,
        diagnostics: vec![Diagnostic {
            severity: DiagnosticSeverity::Info,
            code: code.into(),
            message: message.into(),
            entity_id: None,
            attrs: BTreeMap::new(),
        }],
    }
}

/// Forward one server-protocol command object to the analysis server's
/// HTTP shim and return its JSON verbatim. The whole analysis API is this
/// single generic route: the frontend speaks the same wire commands as
/// stdio/curl clients.
async fn analysis_proxy(
    State(state): State<AppState>,
    Json(cmd): Json<serde_json::Value>,
) -> Response {
    let Some(addr) = state.analysis_server.clone() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": "no analysis server configured; start crabbit-analysisd --http 127.0.0.1:PORT and pass --server 127.0.0.1:PORT"})),
        )
            .into_response();
    };
    match forward_analysis(&addr, &cmd).await {
        Ok(value) => Json(value).into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({"error": format!("analysis server unreachable at {addr}: {e}")})),
        )
            .into_response(),
    }
}

async fn analysis_health(State(state): State<AppState>) -> Response {
    analysis_proxy(State(state), Json(serde_json::json!({"cmd": "server_health"}))).await
}

/// Minimal HTTP/1.1 POST to the analysis shim (std TcpStream on a blocking
/// thread; the shim closes the connection after each response).
async fn forward_analysis(
    addr: &str,
    cmd: &serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let addr = addr.to_string();
    let body = cmd.to_string();
    tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(&addr)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(600)))?;
        write!(
            stream,
            "POST / HTTP/1.1\r\nHost: {addr}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        let mut response = Vec::new();
        stream.read_to_end(&mut response)?;
        let text = String::from_utf8_lossy(&response);
        let payload = text
            .split_once("\r\n\r\n")
            .map(|(_, b)| b)
            .unwrap_or(&text);
        Ok(serde_json::from_str(payload)?)
    })
    .await?
}

fn frontend_dir(state: &AppState) -> PathBuf {
    state
        .frontend_dir
        .clone()
        .unwrap_or_else(|| manifest_dir().join("frontend").join("dist"))
}

async fn serve_index(State(state): State<AppState>) -> Response {
    let dist_index = frontend_dir(&state).join("index.html");
    match std::fs::read_to_string(&dist_index) {
        Ok(contents) => Html(contents).into_response(),
        Err(_) => Html(include_str!("static/index.html").to_string()).into_response(),
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

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn content_type_for(path: &Path) -> &'static str {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("css") => "text/css; charset=utf-8",
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

fn is_trace_path(path: &Path) -> bool {
    path.extension().and_then(|ext| ext.to_str()) == Some(TRACE_EXTENSION)
}

/// Project a trace belongs to: the name of its project subfolder when it is
/// nested inside one of the trace roots, otherwise (legacy flat layout)
/// derived from the filename stem.
fn project_for_source(source: &Path, roots: &[PathBuf]) -> String {
    let in_project_folder = source.parent().is_some_and(|parent| {
        roots
            .iter()
            .filter_map(|root| std::fs::canonicalize(root).ok())
            .any(|root| parent != root && parent.starts_with(root))
    });
    if in_project_folder {
        if let Some(folder) = source.parent().and_then(|parent| parent.file_name()) {
            return folder.to_string_lossy().to_string();
        }
    }
    let stem = source
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();
    trace::derive_project_from_stem(&stem).to_string()
}

fn is_inside_any_dir(path: &Path, dirs: &[PathBuf]) -> bool {
    dirs.iter().any(|dir| is_inside_dir(path, dir))
}

fn is_inside_dir(path: &Path, dir: &Path) -> bool {
    let Ok(dir) = std::fs::canonicalize(dir) else {
        return false;
    };
    path.starts_with(dir)
}

fn import_temp_trace(state: &AppState, source: &Path) -> anyhow::Result<PathBuf> {
    let trace_library_dir = state
        .trace_library_dirs
        .first()
        .ok_or_else(|| anyhow::anyhow!("no trace library directories are configured"))?;
    let project_dir = trace_library_dir.join(project_for_source(source, &state.temp_trace_dirs));
    std::fs::create_dir_all(&project_dir)?;
    let filename = source
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("trace path has no filename: {}", source.display()))?;
    let mut destination = project_dir.join(filename);
    if destination.exists() {
        destination = unique_trace_destination(&project_dir, filename);
    }

    std::fs::copy(source, &destination).map_err(|error| {
        anyhow::anyhow!(
            "failed to import immutable temp trace {} into {}: {}",
            source.display(),
            destination.display(),
            error
        )
    })?;
    std::fs::canonicalize(&destination).map_err(|error| {
        anyhow::anyhow!(
            "failed to resolve imported trace {}: {}",
            destination.display(),
            error
        )
    })
}

fn discover_trace_projects(
    dirs: &[PathBuf],
) -> anyhow::Result<Vec<pliron_inspect_protocol::trace::StairTraceProjectInfo>> {
    let mut merged: BTreeMap<String, Vec<pliron_inspect_protocol::trace::StairTraceFileInfo>> = BTreeMap::new();
    for dir in dirs {
        for project in trace::discover_trace_projects(dir)? {
            merged
                .entry(project.name)
                .or_default()
                .extend(project.versions.into_iter().map(canonical_trace_info));
        }
    }
    Ok(merged
        .into_iter()
        .map(|(name, mut versions)| {
            versions.sort_by(|left, right| left.filepath.cmp(&right.filepath));
            versions.dedup_by(|left, right| left.filepath == right.filepath);
            trace::sort_versions_newest_first(&mut versions);
            pliron_inspect_protocol::trace::StairTraceProjectInfo { name, versions }
        })
        .collect())
}

fn canonical_trace_info(
    trace: pliron_inspect_protocol::trace::StairTraceFileInfo,
) -> pliron_inspect_protocol::trace::StairTraceFileInfo {
    let filepath = std::fs::canonicalize(&trace.filepath)
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or(trace.filepath);
    pliron_inspect_protocol::trace::StairTraceFileInfo { filepath, ..trace }
}

fn display_dirs(dirs: &[PathBuf]) -> Vec<String> {
    dirs.iter()
        .map(|dir| dir.to_string_lossy().to_string())
        .collect()
}

fn unique_trace_destination(dir: &Path, filename: &std::ffi::OsStr) -> PathBuf {
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("trace");
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or(TRACE_EXTENSION);
    for index in 1.. {
        let candidate = dir.join(format!("{stem}-{index}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn error_response(error: anyhow::Error) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        headers,
        serde_json::to_string(&ErrorResponse {
            error: error.to_string(),
        })
        .unwrap(),
    )
        .into_response()
}
