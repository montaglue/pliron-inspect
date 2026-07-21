use std::process::Stdio;

use tokio::{io::AsyncWriteExt, process::Command};

use pliron_inspect_protocol::types::{RenderDocument, RenderRequest};

pub async fn render_document(
    driver_binary: &str,
    req: &RenderRequest,
) -> anyhow::Result<RenderDocument> {
    let mut child = Command::new(driver_binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    let command = serde_json::json!({
        "cmd": "render",
        "view": &req.view,
        "snapshotId": &req.snapshot_id,
        "rootId": &req.root_id,
        "options": &req.options,
        "text": &req.text,
    });

    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("driver stdin was not available"))?;
        stdin.write_all(command.to_string().as_bytes()).await?;
        stdin.write_all(b"\n").await?;
    }
    drop(child.stdin.take());

    let output = child.wait_with_output().await?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let Some(line) = stdout.lines().find(|line| !line.trim().is_empty()) else {
        if output.status.success() {
            anyhow::bail!("driver returned no render response");
        }
        anyhow::bail!("driver failed: {}", stderr.trim());
    };

    serde_json::from_str::<RenderDocument>(line)
        .map_err(|error| anyhow::anyhow!("driver returned invalid render document: {error}"))
}
