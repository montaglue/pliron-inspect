//! Wire types for the resident-server extension of the driver protocol
//! (line-delimited JSON, same transport as the per-file driver commands).
//!
//! Command names: `server_health`, `list_targets`, `load_module`,
//! `start_run`, `run_status`, `cancel_run`, `run_ir`, `run_artifact`.
//! Progress is polled via `run_status` (the stdio transport is strict
//! request/response; there are no unsolicited event lines).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerHealthResponse {
    pub ok: bool,
    pub workers: usize,
    pub modules: usize,
    pub runs: usize,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListTargetsResponse {
    pub targets: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadModuleRequest {
    pub name: String,
    /// Printed pliron IR (a `builtin.module`).
    pub text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadModuleResponse {
    pub module_id: u64,
    pub name: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunRequest {
    pub module_id: u64,
    /// A target name from `list_targets` (e.g. `aarch64-linux`, or the
    /// driver-defined kernel target).
    pub target: String,
    /// Config axes as the environment-variable names/values the batch
    /// tooling already uses (e.g. `CRABBIT_REGALLOC`); the driver decides
    /// which keys it honors.
    #[serde(default)]
    pub config: BTreeMap<String, String>,
    /// Capture the IR after every pass (memory-heavy); without it,
    /// `run_ir` replays deterministically from the stored module text.
    #[serde(default)]
    pub inspect: bool,
    /// Stop after this many passes (replay/debug aid).
    #[serde(default)]
    pub upto: Option<usize>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRunResponse {
    pub run_id: u64,
    pub passes: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    Queued,
    Running,
    Done,
    Failed,
    Cancelled,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PassTiming {
    pub name: String,
    pub micros: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStatusResponse {
    pub run_id: u64,
    pub status: RunStatus,
    /// Passes completed so far (also the per-pass wall times).
    pub passes: Vec<PassTiming>,
    pub total_passes: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunIrRequest {
    pub run_id: u64,
    /// IR after this pass index (0-based); `None` = after the last pass.
    #[serde(default)]
    pub pass: Option<usize>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunIrResponse {
    pub run_id: u64,
    pub pass: usize,
    pub ir: String,
    /// `captured` or `replay`.
    pub source: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunArtifactResponse {
    pub run_id: u64,
    /// The primary artifact (object bytes / PTX text), base64.
    pub artifact_base64: String,
    pub sidecars: Vec<SidecarArtifact>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SidecarArtifact {
    pub name: String,
    pub base64: String,
}

/// Standard base64 (RFC 4648, with padding) — kept here so driver and
/// clients agree without an extra dependency.
/// A zero-based line/character position in a rendered IR document (UTF-16
/// code units for `character`, matching LSP).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocRange {
    pub start: DocPosition,
    pub end: DocPosition,
}

/// Shared request shape of the `ir_hover` / `ir_definition` /
/// `ir_references` commands: the IR document is either a run's rendered
/// IR (`runId` + optional `pass`, defaulting to the latest completed
/// pass) or a loaded module's stored text (`moduleId`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrQueryRequest {
    #[serde(default)]
    pub run_id: Option<u64>,
    #[serde(default)]
    pub module_id: Option<u64>,
    #[serde(default)]
    pub pass: Option<usize>,
    pub position: DocPosition,
    /// `ir_references` only: include the definition itself (default true).
    #[serde(default = "default_true")]
    pub include_declaration: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrHoverResponse {
    /// Markdown, absent when there is nothing at the position.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<DocRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrDefinitionResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<DocRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrReferencesResponse {
    pub ranges: Vec<DocRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrDiagnosticsRequest {
    #[serde(default)]
    pub run_id: Option<u64>,
    #[serde(default)]
    pub module_id: Option<u64>,
    #[serde(default)]
    pub pass: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrDiagnostic {
    pub range: DocRange,
    /// "error" | "warning" | "info" | "hint"
    pub severity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrDiagnosticsResponse {
    /// Whether the document parsed into a module (analysis is lexical-only
    /// when false).
    pub parsed: bool,
    pub diagnostics: Vec<IrDiagnostic>,
}

pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = u32::from_be_bytes([0, b[0], b[1], b[2]]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { ALPHABET[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { ALPHABET[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(s: &str) -> Result<Vec<u8>, String> {
    fn val(c: u8) -> Result<u32, String> {
        match c {
            b'A'..=b'Z' => Ok((c - b'A') as u32),
            b'a'..=b'z' => Ok((c - b'a' + 26) as u32),
            b'0'..=b'9' => Ok((c - b'0' + 52) as u32),
            b'+' => Ok(62),
            b'/' => Ok(63),
            _ => Err(format!("invalid base64 byte {c}")),
        }
    }
    let s = s.trim_end_matches('=');
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let mut acc = 0u32;
    let mut bits = 0u32;
    for &c in s.as_bytes() {
        acc = (acc << 6) | val(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        for input in [b"".as_slice(), b"f", b"fo", b"foo", b"foob", b"\x00\xff\x7f\x80"] {
            assert_eq!(base64_decode(&base64_encode(input)).unwrap(), input);
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }
}

// ---- run_costs: the backward cost lift (crabbit PROFILE-FEEDBACK-BACKWARD) ----

/// `run_costs`: join measured per-op costs (a `profile_ingest.py`
/// `op_costs.json` payload) with the ops at an attribution boundary of the
/// run's pipeline, so any pass view can be heat-mapped. `level` is
/// `"source"` (default: the pre-mid-end numbering, key `source` of the
/// payload, falling back to `lifted` when the mid-end table was absent) or
/// `"ra"` (the RA-boundary numbering, key `lifted`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCostsRequest {
    pub run_id: u64,
    /// The parsed contents of an `op_costs.json`.
    pub op_costs: serde_json::Value,
    #[serde(default)]
    pub level: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostEntry {
    /// Attribution op id at this level (stringified integer).
    pub id: String,
    pub cost: f64,
    /// 0-based line of the op in the boundary IR text (null when the op
    /// could not be located).
    #[serde(default)]
    pub line: Option<u32>,
    /// The op's own printed text (truncated).
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionCosts {
    pub symbol: String,
    pub entries: Vec<CostEntry>,
    /// Semantic accounting: cost attributed to synthetic roots
    /// (`isel:abi`, `frame`, `regalloc`, `placement`, `midend`,
    /// `unattributed`) — code created by the compiler itself.
    pub roots: std::collections::BTreeMap<String, f64>,
    /// Ids in the costs payload with no op at this boundary (deleted ops
    /// or stale profile) — reported, never silently dropped.
    pub unmatched: std::collections::BTreeMap<String, f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunCostsResponse {
    pub run_id: u64,
    pub level: String,
    /// The boundary IR text the `line` fields refer to.
    pub ir: String,
    pub functions: Vec<FunctionCosts>,
}
