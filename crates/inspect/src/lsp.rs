//! pliron-lsp as a child process: the inspector is an LSP client.
//!
//! The IR snapshots the UI shows are opened in pliron-lsp as virtual files
//! under the dialects folder, so it routes them to that folder's dialect
//! engine, which it generates and builds from the folder's own dialect
//! crates (instrumenting their pliron) and rebuilds when their sources
//! change. The IR model, hover, definition, references and diagnostics
//! then come from the real parsers and verifiers, with the dialects' Rust
//! docs and source locations, for any project, with no code in the project.
//!
//! Only what the UI needs is implemented: JSON-RPC framing, requests with
//! responses, the notifications pliron-lsp sends (`pliron/status`,
//! `textDocument/publishDiagnostics`, `pliron/analysisUpdated`), and
//! null answers to the server's own requests.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{Mutex, Notify, oneshot};

/// pliron-lsp's `pliron/status`: the engine serving the project.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    /// Project root, "reference", or absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// "starting" (ours) | "syntax-only" | "building" | "ready" | "error"
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl Default for Status {
    fn default() -> Self {
        Status {
            engine: None,
            state: "starting".into(),
            message: None,
        }
    }
}

struct Inner {
    pending: Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>,
    status: Mutex<Status>,
    /// Latest published diagnostics per document URI.
    diagnostics: Mutex<HashMap<String, Vec<Value>>>,
    /// Engine analyses reported per document URI (`pliron/analysisUpdated`).
    analyses: Mutex<HashMap<String, u64>>,
    changed: Notify,
}

pub struct Lsp {
    pub path: PathBuf,
    writer: Arc<Mutex<ChildStdin>>,
    next_id: AtomicI64,
    inner: Arc<Inner>,
    opened: Mutex<HashSet<String>>,
    _child: Child,
}

impl Lsp {
    /// Start `exe` on `root` (its single workspace folder) and initialize.
    pub async fn spawn(exe: &Path, root: &Path) -> anyhow::Result<Lsp> {
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("cannot start {}: {e}", exe.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = child.stdout.take().expect("piped");
        let inner = Arc::new(Inner {
            pending: Mutex::new(HashMap::new()),
            status: Mutex::new(Status::default()),
            diagnostics: Mutex::new(HashMap::new()),
            analyses: Mutex::new(HashMap::new()),
            changed: Notify::new(),
        });
        let writer = Arc::new(Mutex::new(stdin));
        tokio::spawn(read_loop(stdout, Arc::clone(&inner), Arc::clone(&writer)));
        let lsp = Lsp {
            path: exe.to_path_buf(),
            writer,
            next_id: AtomicI64::new(1),
            inner,
            opened: Mutex::new(HashSet::new()),
            _child: child,
        };
        let root_uri = file_uri(root);
        lsp.request(
            "initialize",
            json!({
                "processId": std::process::id(),
                "rootUri": root_uri,
                "workspaceFolders": [{
                    "uri": root_uri,
                    "name": root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                }],
                "capabilities": {
                    "general": { "positionEncodings": ["utf-16"] },
                    "textDocument": { "hover": { "contentFormat": ["markdown", "plaintext"] } },
                },
                // The round trip (print + reparse) is for dialect authors.
                "initializationOptions": { "roundTrip": false },
            }),
        )
        .await?;
        lsp.notify("initialized", json!({})).await?;
        Ok(lsp)
    }

    pub async fn status(&self) -> Status {
        self.inner.status.lock().await.clone()
    }

    /// Open `text` as the virtual file `path` (once); returns its URI.
    pub async fn open(&self, path: &Path, text: &str) -> anyhow::Result<String> {
        let uri = file_uri(path);
        let fresh = self.opened.lock().await.insert(uri.clone());
        if fresh {
            self.notify(
                "textDocument/didOpen",
                json!({ "textDocument": { "uri": uri, "languageId": "pliron", "version": 1, "text": text } }),
            )
            .await?;
        }
        Ok(uri)
    }

    /// Close the virtual file `path`, if it is open.
    pub async fn close(&self, path: &Path) -> anyhow::Result<()> {
        let uri = file_uri(path);
        if !self.opened.lock().await.remove(&uri) {
            return Ok(());
        }
        self.inner.analyses.lock().await.remove(&uri);
        self.inner.diagnostics.lock().await.remove(&uri);
        self.notify(
            "textDocument/didClose",
            json!({ "textDocument": { "uri": uri } }),
        )
        .await
    }

    /// Wait until the engine has analyzed `uri` (or `timeout` passes).
    /// Returns whether an engine analysis exists.
    pub async fn wait_analysis(&self, uri: &str, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self
                .inner
                .analyses
                .lock()
                .await
                .get(uri)
                .copied()
                .unwrap_or(0)
                > 0
            {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep(left) => {}
            }
        }
    }

    pub async fn diagnostics(&self, uri: &str) -> Vec<Value> {
        self.inner
            .diagnostics
            .lock()
            .await
            .get(uri)
            .cloned()
            .unwrap_or_default()
    }

    pub async fn request(&self, method: &str, params: Value) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.inner.pending.lock().await.insert(id, tx);
        write_message(
            &self.writer,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )
        .await?;
        match rx.await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => anyhow::bail!("{method}: {e}"),
            Err(_) => anyhow::bail!("pliron-lsp exited"),
        }
    }

    pub async fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        write_message(
            &self.writer,
            &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
        .await
    }
}

/// The pliron-lsp binary: `$PLIRON_LSP`, next to this executable, in
/// `~/.cargo/bin`, or on the `PATH`.
pub fn find_pliron_lsp() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("PLIRON_LSP") {
        return Some(PathBuf::from(p));
    }
    let name = format!("pliron-lsp{}", std::env::consts::EXE_SUFFIX);
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        candidates.push(dir.join(&name));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".cargo").join("bin").join(&name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            candidates.push(dir.join(&name));
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

pub fn file_uri(path: &Path) -> String {
    // Paths here are absolute; percent-encoding beyond spaces is not needed
    // for cargo project layouts.
    format!("file://{}", path.display().to_string().replace(' ', "%20"))
}

async fn write_message(writer: &Mutex<ChildStdin>, msg: &Value) -> anyhow::Result<()> {
    let body = msg.to_string();
    let mut w = writer.lock().await;
    w.write_all(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes())
        .await?;
    w.write_all(body.as_bytes()).await?;
    w.flush().await?;
    Ok(())
}

async fn read_loop(stdout: ChildStdout, inner: Arc<Inner>, writer: Arc<Mutex<ChildStdin>>) {
    let mut r = BufReader::new(stdout);
    loop {
        let mut len: Option<usize> = None;
        loop {
            let mut line = String::new();
            if r.read_line(&mut line).await.unwrap_or(0) == 0 {
                fail_pending(&inner).await;
                return;
            }
            let t = line.trim_end();
            if t.is_empty() {
                break;
            }
            if let Some(v) = t.strip_prefix("Content-Length:") {
                len = v.trim().parse().ok();
            }
        }
        let Some(len) = len else { continue };
        let mut buf = vec![0u8; len];
        if r.read_exact(&mut buf).await.is_err() {
            fail_pending(&inner).await;
            return;
        }
        let Ok(msg) = serde_json::from_slice::<Value>(&buf) else {
            continue;
        };
        handle(msg, &inner, &writer).await;
    }
}

async fn fail_pending(inner: &Inner) {
    for (_, tx) in inner.pending.lock().await.drain() {
        let _ = tx.send(Err("pliron-lsp exited".into()));
    }
    inner.status.lock().await.state = "exited".into();
    inner.changed.notify_waiters();
}

async fn handle(msg: Value, inner: &Inner, writer: &Mutex<ChildStdin>) {
    match msg.get("method").and_then(Value::as_str) {
        Some(method) => {
            if let Some(id) = msg.get("id") {
                // A request from the server (progress tokens, capability
                // registration): we support none of them.
                let _ = write_message(
                    writer,
                    &json!({ "jsonrpc": "2.0", "id": id, "result": Value::Null }),
                )
                .await;
                return;
            }
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            match method {
                "pliron/status" => {
                    if let Ok(s) = serde_json::from_value::<Status>(params) {
                        *inner.status.lock().await = s;
                        inner.changed.notify_waiters();
                    }
                }
                "textDocument/publishDiagnostics" => {
                    if let Some(uri) = params["uri"].as_str() {
                        let diags = params["diagnostics"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default();
                        inner
                            .diagnostics
                            .lock()
                            .await
                            .insert(uri.to_string(), diags);
                    }
                }
                "pliron/analysisUpdated" => {
                    if let Some(uri) = params["uri"].as_str() {
                        *inner
                            .analyses
                            .lock()
                            .await
                            .entry(uri.to_string())
                            .or_insert(0) += 1;
                        inner.changed.notify_waiters();
                    }
                }
                "window/logMessage" | "window/showMessage" => {
                    if let Some(m) = params["message"].as_str() {
                        eprintln!("[pliron-lsp] {m}");
                    }
                }
                _ => {}
            }
        }
        None => {
            let Some(id) = msg.get("id").and_then(Value::as_i64) else {
                return;
            };
            if let Some(tx) = inner.pending.lock().await.remove(&id) {
                let result = match msg.get("error") {
                    Some(e) => Err(e["message"].as_str().unwrap_or("error").to_string()),
                    None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                };
                let _ = tx.send(result);
            }
        }
    }
}
