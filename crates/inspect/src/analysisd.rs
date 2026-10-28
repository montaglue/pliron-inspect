//! The project's resident analysis server as a child process, spoken to
//! over its stdio protocol (one JSON command per line, one JSON response
//! per line; see `pliron_inspect_protocol::server`).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

pub struct Analysisd {
    pub path: PathBuf,
    io: Mutex<(ChildStdin, BufReader<ChildStdout>)>,
    _child: Child,
}

impl Analysisd {
    pub fn spawn(path: &Path, workers: usize) -> anyhow::Result<Analysisd> {
        let mut child = Command::new(path)
            .args(["--workers", &workers.to_string()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| anyhow::anyhow!("cannot start {}: {e}", path.display()))?;
        let stdin = child.stdin.take().expect("piped");
        let stdout = BufReader::new(child.stdout.take().expect("piped"));
        Ok(Analysisd {
            path: path.to_path_buf(),
            io: Mutex::new((stdin, stdout)),
            _child: child,
        })
    }

    /// One command, one response. The protocol is strict request/response,
    /// so calls are serialized; long commands (`start_run`) return at once
    /// and are polled with `run_status`.
    pub async fn call(&self, cmd: &Value) -> anyhow::Result<Value> {
        let mut io = self.io.lock().await;
        let (stdin, stdout) = &mut *io;
        stdin.write_all(cmd.to_string().as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;
        let mut line = String::new();
        let n = stdout.read_line(&mut line).await?;
        anyhow::ensure!(n > 0, "the analysis server {} exited", self.path.display());
        Ok(serde_json::from_str(&line)?)
    }
}
