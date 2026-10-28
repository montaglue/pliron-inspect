//! The opened project: its analysis server, its pliron-lsp, and the IR
//! documents the UI has seen. Opened from the browser (`/api/project/open`
//! with the project root); the discovery and the processes live here.
//!
//! Language queries (`ir_hover`, `ir_definition`, `ir_references`,
//! `ir_diagnostics`) are answered by pliron-lsp when it runs: the queried
//! document (a loaded module's text, a run's IR after a pass) is opened in
//! it as a virtual file under the project, so the project's own dialect
//! engine, with the dialects' Rust docs and source locations, answers.
//! Everything else goes to the project's analysis server over stdio.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};

use crate::analysisd::Analysisd;
use crate::lsp::{self, Lsp};
use crate::project::{self, Project};

pub struct Opened {
    pub project: Project,
    pub analysisd: Option<Analysisd>,
    pub lsp: Option<Lsp>,
    /// Why there is no pliron-lsp, if so.
    pub lsp_error: Option<String>,
    /// Texts of the IR documents served so far, by key (`module:<id>`,
    /// `run:<id>:<pass>`); both are immutable.
    docs: Mutex<HashMap<String, String>>,
}

pub enum Slot {
    None,
    Opening { root: PathBuf },
    Ready(Arc<Opened>),
    Error { root: PathBuf, message: String },
}

pub struct Projects {
    slot: RwLock<Slot>,
    /// Worker threads for the project's analysis server.
    workers: usize,
}

impl Projects {
    pub fn new(workers: usize) -> Arc<Projects> {
        Arc::new(Projects {
            slot: RwLock::new(Slot::None),
            workers,
        })
    }

    pub async fn opened(&self) -> Option<Arc<Opened>> {
        match &*self.slot.read().await {
            Slot::Ready(o) => Some(Arc::clone(o)),
            _ => None,
        }
    }

    /// Start opening `root` in the background; `info` reports progress.
    pub async fn open(self: &Arc<Self>, root: PathBuf) {
        *self.slot.write().await = Slot::Opening { root: root.clone() };
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let result = this.open_now(&root).await;
            let mut slot = this.slot.write().await;
            *slot = match result {
                Ok(opened) => Slot::Ready(Arc::new(opened)),
                Err(e) => Slot::Error {
                    root,
                    message: format!("{e:#}"),
                },
            };
        });
    }

    async fn open_now(&self, root: &Path) -> anyhow::Result<Opened> {
        let root = root.to_path_buf();
        // Discovery may build the project's binaries: off the async runtime.
        let project = tokio::task::spawn_blocking(move || Project::discover(&root, true)).await??;
        let analysisd = match &project.analysisd {
            Some(path) => Some(Analysisd::spawn(path, self.workers)?),
            None => None,
        };
        let (lsp, lsp_error) = match project::find_pliron_lsp() {
            Some(exe) => match Lsp::spawn(&exe, &project.root).await {
                Ok(lsp) => {
                    // Opening any document under the project makes
                    // pliron-lsp prepare the project's dialect engine now.
                    let _ = lsp.open(&project.document_path("warmup"), "").await;
                    (Some(lsp), None)
                }
                Err(e) => (None, Some(format!("{e:#}"))),
            },
            None => (
                None,
                Some(
                    "pliron-lsp not found (install it from github.com/montaglue/pliron-lsp, or set PLIRON_LSP)"
                        .into(),
                ),
            ),
        };
        Ok(Opened {
            project,
            analysisd,
            lsp,
            lsp_error,
            docs: Mutex::new(HashMap::new()),
        })
    }

    pub async fn close(&self) {
        *self.slot.write().await = Slot::None;
    }

    /// The project's state, for the UI.
    pub async fn info(&self) -> Value {
        match &*self.slot.read().await {
            Slot::None => json!({ "state": "none" }),
            Slot::Opening { root } => json!({ "state": "opening", "root": root }),
            Slot::Error { root, message } => {
                json!({ "state": "error", "root": root, "message": message })
            }
            Slot::Ready(o) => {
                let p = &o.project;
                let lsp = match &o.lsp {
                    Some(l) => json!({ "path": l.path, "status": l.status().await }),
                    None => json!({ "error": o.lsp_error }),
                };
                json!({
                    "state": "ready",
                    "root": p.root,
                    "metadata": p.metadata,
                    "traceDirs": p.trace_dirs,
                    "driver": p.driver,
                    "analysisd": o.analysisd.as_ref().map(|a| &a.path),
                    "lsp": lsp,
                    "modules": p.modules(),
                    "profiles": p.profiles(),
                })
            }
        }
    }

    /// Answer an analysis command for the opened project, or `None` when
    /// no project is open (the configured `--server` applies then).
    pub async fn handle(&self, cmd: &Value) -> Option<Value> {
        let opened = self.opened().await?;
        Some(opened.handle(cmd).await)
    }
}

/// The IR document a language query is about.
struct Doc {
    key: String,
    text: String,
}

impl Opened {
    async fn handle(&self, cmd: &Value) -> Value {
        match cmd["cmd"].as_str() {
            Some("project_module") => self.project_module(cmd),
            Some("run_costs") if cmd.get("opCostsPath").is_some() => {
                let Some(path) = cmd["opCostsPath"].as_str() else {
                    return error("opCostsPath must be a path");
                };
                let payload = match self.read_owned(Path::new(path)) {
                    Ok(text) => match serde_json::from_str::<Value>(&text) {
                        Ok(v) => v,
                        Err(e) => return error(&format!("{path}: not JSON: {e}")),
                    },
                    Err(e) => return error(&e),
                };
                let mut cmd = cmd.clone();
                cmd["opCosts"] = payload;
                cmd.as_object_mut().unwrap().remove("opCostsPath");
                self.forward(&cmd).await
            }
            Some("ir_hover" | "ir_definition" | "ir_references" | "ir_diagnostics")
                if self.lsp.is_some() =>
            {
                match self.doc_for(cmd).await {
                    Ok(doc) => match self.language_query(cmd, &doc).await {
                        Ok(v) => v,
                        Err(e) => error(&format!("{e:#}")),
                    },
                    Err(e) => error(&e),
                }
            }
            _ => {
                let resp = self.forward(cmd).await;
                self.remember(cmd, &resp).await;
                resp
            }
        }
    }

    async fn forward(&self, cmd: &Value) -> Value {
        match &self.analysisd {
            Some(a) => match a.call(cmd).await {
                Ok(v) => v,
                Err(e) => error(&format!("analysis server: {e:#}")),
            },
            None => error(
                "this project has no analysis server (set `analysisd` in [workspace.metadata.pliron-inspect])",
            ),
        }
    }

    /// Keep the texts of documents the UI will query: a loaded module's
    /// text, a run's IR after a pass.
    async fn remember(&self, cmd: &Value, resp: &Value) {
        match cmd["cmd"].as_str() {
            Some("load_module") => {
                if let (Some(id), Some(text)) = (resp["moduleId"].as_u64(), cmd["text"].as_str()) {
                    self.docs
                        .lock()
                        .await
                        .insert(format!("module:{id}"), text.to_string());
                }
            }
            Some("run_ir") => {
                if let (Some(run), Some(pass), Some(ir)) = (
                    resp["runId"].as_u64(),
                    resp["pass"].as_u64(),
                    resp["ir"].as_str(),
                ) {
                    self.docs
                        .lock()
                        .await
                        .insert(format!("run:{run}:{pass}"), ir.to_string());
                }
            }
            _ => {}
        }
    }

    fn read_owned(&self, path: &Path) -> Result<String, String> {
        if !self.project.owns(path) {
            return Err(format!(
                "{} is not under the project's trace directories",
                path.display()
            ));
        }
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    fn project_module(&self, cmd: &Value) -> Value {
        let Some(path) = cmd["path"].as_str() else {
            return error("project_module requires path");
        };
        match self.read_owned(Path::new(path)) {
            Ok(text) => json!({
                "name": Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()),
                "path": path,
                "text": text,
            }),
            Err(e) => error(&e),
        }
    }

    /// The document of an `ir_*` query: a run's IR at a pass (fetched from
    /// the analysis server when not seen yet) or a loaded module's text.
    async fn doc_for(&self, cmd: &Value) -> Result<Doc, String> {
        if let Some(run) = cmd["runId"].as_u64() {
            if let Some(pass) = cmd["pass"].as_u64() {
                let key = format!("run:{run}:{pass}");
                if let Some(text) = self.docs.lock().await.get(&key).cloned() {
                    return Ok(Doc { key, text });
                }
            }
            let resp = self
                .forward(&json!({ "cmd": "run_ir", "runId": run, "pass": cmd["pass"] }))
                .await;
            if let Some(e) = resp["error"].as_str() {
                return Err(e.to_string());
            }
            self.remember(&json!({ "cmd": "run_ir" }), &resp).await;
            let pass = resp["pass"]
                .as_u64()
                .ok_or("run_ir answered without a pass")?;
            let text = resp["ir"].as_str().ok_or("run_ir answered without IR")?;
            return Ok(Doc {
                key: format!("run:{run}:{pass}"),
                text: text.to_string(),
            });
        }
        if let Some(id) = cmd["moduleId"].as_u64() {
            let key = format!("module:{id}");
            return match self.docs.lock().await.get(&key).cloned() {
                Some(text) => Ok(Doc { key, text }),
                None => Err(format!(
                    "module {id} was not loaded through this UI server, so its text is unknown here"
                )),
            };
        }
        Err("ir query requires runId or moduleId".into())
    }

    async fn language_query(&self, cmd: &Value, doc: &Doc) -> anyhow::Result<Value> {
        let lsp = self.lsp.as_ref().expect("checked");
        let path = self.project.document_path(&doc.key);
        let uri = lsp.open(&path, &doc.text).await?;
        // With the engine ready, its analysis of a fresh document arrives
        // within moments; while it builds, the syntax layer answers.
        let status = lsp.status().await;
        let wait = if status.state == "ready" {
            Duration::from_secs(20)
        } else {
            Duration::from_millis(300)
        };
        let exact = lsp.wait_analysis(&uri, wait).await;
        let text_document = json!({ "uri": uri });
        Ok(match cmd["cmd"].as_str().unwrap_or_default() {
            "ir_hover" => {
                let h = lsp
                    .request(
                        "textDocument/hover",
                        json!({ "textDocument": text_document, "position": cmd["position"] }),
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
            "ir_definition" => {
                let d = lsp
                    .request(
                        "textDocument/definition",
                        json!({ "textDocument": text_document, "position": cmd["position"] }),
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
                            json!({ "location": source_location(&target_uri, range.as_ref()), "exact": exact })
                        }
                    }
                }
            }
            "ir_references" => {
                let include = cmd["includeDeclaration"].as_bool().unwrap_or(true);
                let r = lsp
                    .request(
                        "textDocument/references",
                        json!({
                            "textDocument": text_document,
                            "position": cmd["position"],
                            "context": { "includeDeclaration": include },
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
            _ => {
                let diags = lsp.diagnostics(&uri).await;
                let mut parsed = true;
                let out: Vec<Value> = diags
                    .iter()
                    .map(|d| {
                        let severity = match d["severity"].as_u64() {
                            Some(2) => "warning",
                            Some(3) => "info",
                            Some(4) => "hint",
                            _ => "error",
                        };
                        if severity == "error" {
                            parsed = false;
                        }
                        json!({
                            "range": d["range"],
                            "severity": severity,
                            "source": d.get("source"),
                            "message": d["message"],
                        })
                    })
                    .collect();
                json!({
                    "parsed": parsed,
                    "diagnostics": out,
                    "engine": status.state,
                    "exact": exact,
                })
            }
        })
    }
}

/// A definition outside the IR document (the dialect's Rust source): its
/// path, position and a few lines around it.
fn source_location(uri: &str, range: Option<&Value>) -> Value {
    let path = uri
        .strip_prefix("file://")
        .unwrap_or(uri)
        .replace("%20", " ");
    let line = range.and_then(|r| r["start"]["line"].as_u64()).unwrap_or(0) as usize;
    let character = range
        .and_then(|r| r["start"]["character"].as_u64())
        .unwrap_or(0);
    let snippet = std::fs::read_to_string(&path).ok().map(|text| {
        let lines: Vec<&str> = text.lines().collect();
        let from = line.saturating_sub(3);
        let to = (line + 5).min(lines.len());
        lines[from..to]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:>5}  {l}", from + i + 1))
            .collect::<Vec<_>>()
            .join("\n")
    });
    json!({ "path": path, "line": line, "character": character, "snippet": snippet })
}

fn error(msg: &str) -> Value {
    json!({ "error": msg })
}

/// Shared by the server for the document URI of a key (tests).
#[allow(dead_code)]
pub fn document_uri(project: &Project, key: &str) -> String {
    lsp::file_uri(&project.document_path(key))
}
