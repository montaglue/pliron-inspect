use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
    pub ok: bool,
    pub driver_binary: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceListResponse {
    pub library_projects: Vec<crate::trace::StairTraceProjectInfo>,
    pub temp_projects: Vec<crate::trace::StairTraceProjectInfo>,
    pub library_dirs: Vec<String>,
    pub temp_dirs: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTraceRequest {
    pub filepath: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportTraceRequest {
    pub filename: String,
    pub contents: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OpenTraceResponse {
    pub filepath: String,
    pub filename: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<String>,
    pub meta: crate::trace::StairTraceMeta,
    pub snapshots: Vec<TraceSnapshot>,
    pub pipeline: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceSnapshot {
    pub label: String,
    pub ir: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayCapabilities {
    pub protocol_version: u32,
    pub views: Vec<ViewCapability>,
    pub cli_commands: Vec<String>,
    pub frontend: FrontendStack,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewCapability {
    pub id: String,
    pub label: String,
    pub description: String,
}

impl ViewCapability {
    pub fn new(id: &str, label: &str, description: &str) -> Self {
        Self {
            id: id.to_string(),
            label: label.to_string(),
            description: description.to_string(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendStack {
    pub framework: String,
    pub bundler: String,
    pub language: String,
    pub graph: String,
    pub tree: String,
    pub text: String,
    pub egraph: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRequest {
    pub view: String,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub root_id: Option<String>,
    #[serde(default)]
    pub options: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub text: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RenderDocument {
    pub version: u32,
    pub view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub snapshot_id: String,
    pub entities: Vec<Entity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph: Option<GraphDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tree: Option<TreeDocument>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<TextDocument>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<Diagnostic>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Entity {
    pub id: String,
    pub kind: EntityKind,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub interfaces: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_span: Option<SourceSpan>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum EntityKind {
    Operation,
    Region,
    Block,
    Value,
    Type,
    Attribute,
    Interface,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceSpan {
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphDocument {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<GraphLayout>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    pub id: String,
    pub entity_id: String,
    pub kind: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    pub id: String,
    pub from: String,
    pub to: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphLayout {
    pub engine: String,
    pub width: f32,
    pub height: f32,
    pub nodes: Vec<GraphNodeLayout>,
    pub edges: Vec<GraphEdgeLayout>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphNodeLayout {
    pub node_id: String,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub rank: usize,
    pub column: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdgeLayout {
    pub edge_id: String,
    pub points: Vec<GraphPoint>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
#[serde(rename_all = "camelCase")]
pub struct GraphPoint {
    pub x: f32,
    pub y: f32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeDocument {
    pub root: TreeNode,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    pub id: String,
    pub entity_id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TextDocument {
    pub language: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spans: Vec<TextSpan>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TextSpan {
    pub entity_id: String,
    pub start: usize,
    pub end: usize,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Diagnostic {
    pub severity: DiagnosticSeverity,
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}
