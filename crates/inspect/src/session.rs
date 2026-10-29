//! What the UI has chosen, and the IR it shows.
//!
//! - The dialects folder: a cargo workspace whose crates define pliron
//!   dialects. pliron-lsp runs on it and builds a dialect engine from those
//!   crates (instrumenting the project's pliron), so the project's own
//!   parsers and verifiers read the IR, with no code in the project.
//! - The traces folder: where the compiler wrote its traces.
//! - IR documents: the snapshots the UI shows, by id. They are opened in
//!   pliron-lsp as virtual files under the dialects folder (never written
//!   to disk), so the dialect engine answers for them: the model the CFG
//!   and tree views are drawn from, hover, definition, references and
//!   diagnostics.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use pliron_inspect_protocol::types::{Diagnostic, DiagnosticSeverity, RenderDocument};
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};

use crate::lsp::{self, Lsp};
use crate::render::{self, Ir, ModelResponse};

/// IR documents kept for the UI (and open in pliron-lsp).
const MAX_DOCS: usize = 16;
/// Models kept for drawing other views of the same document.
const MAX_MODELS: usize = 2;
/// Rust sources larger than this are not sent to the UI.
const MAX_SOURCE_BYTES: u64 = 4 << 20;

pub struct Session {
    dialects: RwLock<Slot>,
    traces: RwLock<Option<PathBuf>>,
    docs: Mutex<Docs>,
}

enum Slot {
    None,
    Opening { root: PathBuf },
    Ready(Arc<Dialects>),
    Error { root: PathBuf, message: String },
}

/// The dialects folder, with pliron-lsp running on it.
pub struct Dialects {
    root: PathBuf,
    lsp: Lsp,
    models: Mutex<VecDeque<(String, Model)>>,
}

/// The engine's model of a document, and what it could not read.
#[derive(Clone)]
struct Model {
    ir: Arc<Ir>,
    /// Set when the model misses parts that did not parse.
    partial: Option<String>,
}

#[derive(Default)]
struct Docs {
    texts: HashMap<String, Arc<String>>,
    /// Ids, least recently added first.
    order: VecDeque<String>,
}

/// Why a document has no model (yet).
enum NoModel {
    /// The engine is building or still analyzing: try again shortly.
    Pending(String),
    Failed(String),
}

impl Session {
    pub fn new() -> Arc<Session> {
        Arc::new(Session {
            dialects: RwLock::new(Slot::None),
            traces: RwLock::new(None),
            docs: Mutex::new(Docs::default()),
        })
    }

    /// Start pliron-lsp on `root` in the background; `state` reports it.
    /// The folder already open (or opening) stays as it is.
    pub async fn open_dialects(self: &Arc<Self>, root: PathBuf) {
        {
            let mut slot = self.dialects.write().await;
            match &*slot {
                Slot::Opening { root: r } if *r == root => return,
                Slot::Ready(d) if d.root == root => return,
                _ => {}
            }
            *slot = Slot::Opening { root: root.clone() };
        }
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let slot = match Dialects::start(&root).await {
                Ok(d) => Slot::Ready(Arc::new(d)),
                Err(e) => Slot::Error {
                    root: root.clone(),
                    message: format!("{e:#}"),
                },
            };
            let mut current = this.dialects.write().await;
            // Another folder may have been chosen meanwhile.
            if matches!(&*current, Slot::Opening { root: r } if *r == root) {
                *current = slot;
            }
        });
    }

    pub async fn close_dialects(&self) {
        *self.dialects.write().await = Slot::None;
    }

    pub async fn set_traces(&self, dir: PathBuf) {
        *self.traces.write().await = Some(dir);
    }

    pub async fn traces(&self) -> Option<PathBuf> {
        self.traces.read().await.clone()
    }

    async fn ready(&self) -> Option<Arc<Dialects>> {
        match &*self.dialects.read().await {
            Slot::Ready(d) => Some(Arc::clone(d)),
            _ => None,
        }
    }

    /// The chosen folders and the dialect engine's state, for the UI.
    pub async fn state(&self) -> Value {
        let dialects = match &*self.dialects.read().await {
            Slot::None => Value::Null,
            Slot::Opening { root } => json!({ "root": root, "state": "starting" }),
            Slot::Error { root, message } => {
                json!({ "root": root, "state": "error", "message": message })
            }
            Slot::Ready(d) => {
                let status = d.lsp.status().await;
                json!({
                    "root": d.root,
                    "state": status.state,
                    "message": status.message,
                    "lsp": d.lsp.path,
                })
            }
        };
        json!({ "dialects": dialects, "traces": self.traces().await })
    }

    /// Keep `text` as an IR document; returns its id.
    pub async fn add_doc(&self, text: String) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        let id = format!("{:016x}", hasher.finish());
        let evicted = {
            let mut docs = self.docs.lock().await;
            if docs.texts.contains_key(&id) {
                return id;
            }
            docs.texts.insert(id.clone(), Arc::new(text));
            docs.order.push_back(id.clone());
            let mut evicted = Vec::new();
            while docs.order.len() > MAX_DOCS {
                let old = docs.order.pop_front().expect("non-empty");
                docs.texts.remove(&old);
                evicted.push(old);
            }
            evicted
        };
        if let Some(d) = self.ready().await {
            for old in evicted {
                d.forget(&old).await;
            }
        }
        id
    }

    async fn doc(&self, id: &str) -> Option<Arc<String>> {
        self.docs.lock().await.texts.get(id).cloned()
    }

    /// The `view` (`cfg` or `tree`) of document `id`.
    pub async fn render(&self, id: &str, view: &str, root: Option<&str>) -> RenderDocument {
        let title = match view {
            "cfg" => "CFG",
            "tree" => "Tree",
            _ => view,
        };
        let note = |code: &str, message: String| {
            render::diagnostic_document(view, id, title, code, message)
        };
        if view != "cfg" && view != "tree" {
            return note("unknown-view", format!("Unknown view: {view}"));
        }
        let Some(text) = self.doc(id).await else {
            return note(
                "unknown-document",
                "This snapshot is no longer kept by the server; open the trace again.".into(),
            );
        };
        let dialects = match &*self.dialects.read().await {
            Slot::Ready(d) => Arc::clone(d),
            Slot::None => {
                return note(
                    "no-dialects",
                    "Choose a dialects folder: its dialect engine reads the IR for this view."
                        .into(),
                );
            }
            Slot::Opening { .. } => {
                return note("engine-pending", "Starting pliron-lsp…".into());
            }
            Slot::Error { message, .. } => return note("dialects-error", message.clone()),
        };
        match dialects.model(id, &text).await {
            Ok(model) => {
                let mut doc = match view {
                    "cfg" => render::cfg::cfg_document(&model.ir, id, root),
                    _ => render::tree::tree_document(&model.ir, id),
                };
                if let Some(partial) = model.partial {
                    doc.diagnostics.push(Diagnostic {
                        severity: DiagnosticSeverity::Warning,
                        code: "partial-model".into(),
                        message: partial,
                        entity_id: None,
                        attrs: Default::default(),
                    });
                }
                doc
            }
            Err(NoModel::Pending(message)) => note("engine-pending", message),
            Err(NoModel::Failed(message)) => note("no-model", message),
        }
    }

    /// A language query on document `id`: `hover`, `definition`,
    /// `references` (at `position`) or `diagnostics`.
    pub async fn language(
        &self,
        query: &str,
        id: &str,
        position: Option<&Value>,
    ) -> Result<Value, String> {
        let text = self
            .doc(id)
            .await
            .ok_or("this snapshot is no longer kept by the server; open the trace again")?;
        let dialects = self.ready().await.ok_or("no dialects folder is open")?;
        dialects
            .language(query, id, &text, position)
            .await
            .map_err(|e| format!("{e:#}"))
    }
}

impl Dialects {
    async fn start(root: &Path) -> anyhow::Result<Dialects> {
        let exe = lsp::find_pliron_lsp().ok_or_else(|| {
            anyhow::anyhow!(
                "pliron-lsp not found (install it from github.com/montaglue/pliron-lsp, or set PLIRON_LSP)"
            )
        })?;
        let lsp = Lsp::spawn(&exe, root).await?;
        let d = Dialects {
            root: root.to_path_buf(),
            lsp,
            models: Mutex::new(VecDeque::new()),
        };
        // Opening any document under the folder makes pliron-lsp prepare
        // the dialect engine now rather than at the first view.
        d.lsp.open(&d.document_path("warmup"), "").await?;
        Ok(d)
    }

    /// The virtual file of document `id`: under the folder, so pliron-lsp
    /// routes it to the folder's dialect engine.
    fn document_path(&self, id: &str) -> PathBuf {
        self.root
            .join("target")
            .join("pliron-inspect")
            .join(format!("{id}.plir"))
    }

    async fn forget(&self, id: &str) {
        let _ = self.lsp.close(&self.document_path(id)).await;
        self.models.lock().await.retain(|(m, _)| m != id);
    }

    /// The parse errors among the document's diagnostics, as
    /// `line N: message`.
    async fn parse_errors(&self, uri: &str) -> Vec<String> {
        self.lsp
            .diagnostics(uri)
            .await
            .iter()
            .filter(|d| matches!(d["code"].as_str(), Some("parse" | "panic")))
            .map(|d| {
                format!(
                    "line {}: {}",
                    d["range"]["start"]["line"].as_u64().unwrap_or(0) + 1,
                    d["message"].as_str().unwrap_or_default()
                )
            })
            .collect()
    }

    /// Open document `id` and wait for the engine's analysis of it. Returns
    /// its URI and whether the engine has analyzed it.
    async fn analyzed(
        &self,
        id: &str,
        text: &str,
        wait_ready: Duration,
    ) -> anyhow::Result<(String, bool)> {
        let uri = self.lsp.open(&self.document_path(id), text).await?;
        // With the engine ready, its analysis of a fresh document arrives
        // within moments (seconds for a large one); while it builds, the
        // syntax layer answers.
        let wait = if self.lsp.status().await.state == "ready" {
            wait_ready
        } else {
            Duration::from_millis(300)
        };
        let exact = self.lsp.wait_analysis(&uri, wait).await;
        Ok((uri, exact))
    }

    /// The engine's model of document `id`.
    async fn model(&self, id: &str, text: &Arc<String>) -> Result<Model, NoModel> {
        if let Some((_, model)) = self.models.lock().await.iter().find(|(m, _)| m == id) {
            return Ok(model.clone());
        }
        let failed = |e: anyhow::Error| NoModel::Failed(format!("{e:#}"));
        let (uri, exact) = self
            .analyzed(id, text, Duration::from_secs(120))
            .await
            .map_err(failed)?;
        let status = self.lsp.status().await;
        if !exact {
            return Err(match status.state.as_str() {
                "building" | "starting" => NoModel::Pending(format!(
                    "The dialect engine is building{}; the view appears when it is ready.",
                    status
                        .message
                        .map(|m| format!(" ({m})"))
                        .unwrap_or_default()
                )),
                "ready" => NoModel::Pending("The dialect engine is still reading this IR…".into()),
                state => NoModel::Failed(format!(
                    "No dialect engine reads this IR ({state}{}).",
                    status.message.map(|m| format!(": {m}")).unwrap_or_default()
                )),
            });
        }
        let response = self
            .lsp
            .request("pliron/model", json!({ "textDocument": { "uri": uri } }))
            .await
            .map_err(|e| {
                if e.to_string().contains("unsupported request") {
                    NoModel::Failed(format!(
                        "{} is too old to draw IR (it has no pliron/model); update pliron-lsp",
                        self.lsp.path.display()
                    ))
                } else {
                    failed(e)
                }
            })?;
        let response: ModelResponse =
            serde_json::from_value(response).map_err(|e| failed(e.into()))?;
        let parse_errors = self.parse_errors(&uri).await;
        let Some(model) = response.model else {
            return Err(NoModel::Failed(
                match (parse_errors.first(), response.note) {
                    (Some(e), _) => format!("The dialect engine could not read this IR: {e}"),
                    (None, Some(note)) => note,
                    (None, None) => "The dialect engine has no model of this IR.".into(),
                },
            ));
        };
        // The engine recovers from parse errors; the model then misses what
        // did not parse (e.g. ops of a dialect the folder does not define).
        let partial = parse_errors.first().map(|first| {
            format!(
                "The dialect engine could not read all of this IR ({} parse error{}; first, {first}). \
                 This view shows what it could read.",
                parse_errors.len(),
                if parse_errors.len() == 1 { "" } else { "s" }
            )
        });
        let model = Model {
            ir: Arc::new(Ir::new(Arc::clone(text), model, response.op_ranges)),
            partial,
        };
        let mut models = self.models.lock().await;
        models.push_back((id.to_string(), model.clone()));
        while models.len() > MAX_MODELS {
            models.pop_front();
        }
        Ok(model)
    }

    async fn language(
        &self,
        query: &str,
        id: &str,
        text: &str,
        position: Option<&Value>,
    ) -> anyhow::Result<Value> {
        let (uri, exact) = self.analyzed(id, text, Duration::from_secs(20)).await?;
        let lsp = &self.lsp;
        let text_document = json!({ "uri": uri });
        Ok(match query {
            "hover" => {
                let h = lsp
                    .request(
                        "textDocument/hover",
                        json!({ "textDocument": text_document, "position": position }),
                    )
                    .await?;
                let contents = match &h["contents"] {
                    Value::String(s) => Some(s.clone()),
                    Value::Object(o) => o.get("value").and_then(Value::as_str).map(String::from),
                    Value::Array(a) => Some(
                        a.iter()
                            .filter_map(|c| match c {
                                Value::String(s) => Some(s.clone()),
                                Value::Object(o) => {
                                    o.get("value").and_then(Value::as_str).map(String::from)
                                }
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n\n"),
                    ),
                    _ => None,
                };
                json!({ "contents": contents, "range": h.get("range"), "exact": exact })
            }
            "definition" => {
                let d = lsp
                    .request(
                        "textDocument/definition",
                        json!({ "textDocument": text_document, "position": position }),
                    )
                    .await?;
                let first = match &d {
                    Value::Array(a) => a.first().cloned(),
                    Value::Null => None,
                    v => Some(v.clone()),
                };
                match first {
                    None => json!({ "exact": exact }),
                    Some(loc) => {
                        let (target_uri, range) = match (loc.get("targetUri"), loc.get("uri")) {
                            (Some(u), _) => (
                                u.as_str().unwrap_or_default().to_string(),
                                loc.get("targetSelectionRange")
                                    .or(loc.get("targetRange"))
                                    .cloned(),
                            ),
                            (_, Some(u)) => (
                                u.as_str().unwrap_or_default().to_string(),
                                loc.get("range").cloned(),
                            ),
                            _ => (String::new(), None),
                        };
                        if target_uri == uri {
                            json!({ "range": range, "exact": exact })
                        } else {
                            json!({ "source": self.source(&target_uri, range), "exact": exact })
                        }
                    }
                }
            }
            "references" => {
                let r = lsp
                    .request(
                        "textDocument/references",
                        json!({
                            "textDocument": text_document,
                            "position": position,
                            "context": { "includeDeclaration": true },
                        }),
                    )
                    .await?;
                let ranges: Vec<Value> = r
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter(|l| l["uri"].as_str() == Some(&uri))
                            .filter_map(|l| l.get("range").cloned())
                            .collect()
                    })
                    .unwrap_or_default();
                json!({ "ranges": ranges, "exact": exact })
            }
            "diagnostics" => {
                let diagnostics: Vec<Value> = lsp
                    .diagnostics(&uri)
                    .await
                    .iter()
                    .map(|d| {
                        let severity = match d["severity"].as_u64() {
                            Some(2) => "warning",
                            Some(3) => "info",
                            Some(4) => "hint",
                            _ => "error",
                        };
                        json!({
                            "range": d["range"],
                            "severity": severity,
                            "source": d.get("source"),
                            // parse, verify, lint, ...
                            "phase": d.get("code"),
                            "message": d["message"],
                        })
                    })
                    .collect();
                json!({ "diagnostics": diagnostics, "exact": exact })
            }
            other => anyhow::bail!("unknown query {other}"),
        })
    }

    /// A definition outside the IR (the dialect's Rust source): its path and
    /// position, and its text when it is one of the dialects' sources (in
    /// the dialects folder or cargo's dependency sources).
    fn source(&self, uri: &str, range: Option<Value>) -> Value {
        let path = PathBuf::from(
            uri.strip_prefix("file://")
                .unwrap_or(uri)
                .replace("%20", " "),
        );
        let start = range
            .as_ref()
            .map(|r| r["start"].clone())
            .unwrap_or(json!({ "line": 0, "character": 0 }));
        let text = std::fs::canonicalize(&path)
            .ok()
            .filter(|p| self.readable_source(p))
            .and_then(|p| std::fs::read_to_string(p).ok());
        json!({ "path": path, "position": start, "range": range, "text": text })
    }

    fn readable_source(&self, path: &Path) -> bool {
        let cargo_home = std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")));
        let roots = [Some(self.root.clone()), cargo_home]
            .into_iter()
            .flatten()
            .filter_map(|r| std::fs::canonicalize(r).ok());
        path.extension().is_some_and(|e| e == "rs")
            && path.metadata().is_ok_and(|m| m.len() <= MAX_SOURCE_BYTES)
            && roots.into_iter().any(|r| path.starts_with(r))
    }
}
